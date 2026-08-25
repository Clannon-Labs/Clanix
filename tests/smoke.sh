#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
bind_address=${CLANNON_SMOKE_BIND:-127.0.0.1:39081}
base_url="http://${bind_address}"
server_log=$(mktemp)
snapshot_file=$(mktemp)
environment_id=""
server_pid=""

cleanup() {
  if [[ -n "$environment_id" ]]; then
    curl --silent --request DELETE "$base_url/api/environments/$environment_id" >/dev/null 2>&1 || true
  fi
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" >/dev/null 2>&1 || true
    wait "$server_pid" >/dev/null 2>&1 || true
  fi
  rm -f "$server_log" "$snapshot_file"
}
trap cleanup EXIT

cd "$project_dir"
cargo build --quiet
CLANNON_BIND="$bind_address" target/debug/clannon >"$server_log" 2>&1 &
server_pid=$!

for _ in $(seq 1 120); do
  if curl --silent --fail "$base_url/" >/dev/null; then
    break
  fi
  if ! kill -0 "$server_pid" 2>/dev/null; then
    cat "$server_log" >&2
    exit 1
  fi
  sleep 0.25
done
curl --silent --fail "$base_url/" >/dev/null

if ! create_response=$(curl --silent --show-error --fail-with-body --request POST "$base_url/api/environments"); then
  printf '%s\n' "$create_response" >&2
  cat "$server_log" >&2
  exit 1
fi
environment_id=$(node -e 'const d=JSON.parse(process.argv[1]); if(!d.id) process.exit(1); process.stdout.write(d.id)' "$create_response")

BASE_URL="$base_url" ENVIRONMENT_ID="$environment_id" node <<'NODE'
const wsUrl = process.env.BASE_URL.replace(/^http/, "ws") +
  `/api/environments/${process.env.ENVIRONMENT_ID}/terminal`;
const socket = new WebSocket(wsUrl);
const timeout = setTimeout(() => {
  console.error("terminal command timed out");
  process.exit(1);
}, 10_000);
socket.addEventListener("open", () => {
  socket.send("printf 'smoke-proof\\n' > proof.txt; nohup sleep 20 >/dev/null 2>&1 & nohup nc -l -s 0.0.0.0 -p 23456 >/dev/null 2>&1 & for delay in 1 2 3 4 5 6 7 8 9 10; do grep -q ':5BA0 ' /proc/net/tcp && break; sleep 0.1; done; grep -q ':5BA0 ' /proc/net/tcp && echo command-finished\n");
});
socket.addEventListener("message", (event) => {
  if (String(event.data).includes("command-finished")) {
    clearTimeout(timeout);
    socket.close();
  }
});
socket.addEventListener("error", () => {
  console.error("terminal websocket failed");
  process.exit(1);
});
socket.addEventListener("close", () => {
  clearTimeout(timeout);
});
NODE

curl --silent --fail "$base_url/api/environments/$environment_id/observations" >"$snapshot_file"
node - "$snapshot_file" <<'NODE'
const fs = require("node:fs");
const snapshot = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const hasProofFile = snapshot.files.some((file) => file.path === "/workspace/proof.txt");
const hasCommand = snapshot.transcript.some((entry) => entry.direction === "input" && entry.data.includes("proof.txt"));
const hasOutput = snapshot.transcript.some((entry) => entry.direction === "output" && entry.data.includes("command-finished"));
const timestampsAreValid = snapshot.transcript.every((entry) =>
  Number.isSafeInteger(entry.timestamp_ms) && entry.timestamp_ms > 0
);
const timestampsAreOrdered = snapshot.transcript.every((entry, index, entries) =>
  index === 0 || entries[index - 1].timestamp_ms <= entry.timestamp_ms
);
const hasSleep = snapshot.processes.some((process) => process.command === "sleep");
const hasTcpListener = snapshot.network.some((socket) =>
  socket.protocol === "tcp" &&
  socket.local_address === "0.0.0.0:23456" &&
  socket.state === "listening"
);
if (!hasProofFile || !hasCommand || !hasOutput || !timestampsAreValid || !timestampsAreOrdered || !hasSleep || !hasTcpListener) {
  console.error(JSON.stringify({
    hasProofFile,
    hasCommand,
    hasOutput,
    timestampsAreValid,
    timestampsAreOrdered,
    hasSleep,
    hasTcpListener,
    snapshot,
  }, null, 2));
  process.exit(1);
}
NODE

curl --silent --fail --request DELETE "$base_url/api/environments/$environment_id" >/dev/null
status=$(curl --silent --output /dev/null --write-out '%{http_code}' "$base_url/api/environments/$environment_id/observations")
[[ "$status" == "404" ]]
environment_id=""

echo "Clannon smoke test passed"
