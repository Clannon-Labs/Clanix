#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
bind_address=${CLANNON_SMOKE_BIND:-127.0.0.1:39081}
base_url=""
authority=""
origin=""
access_token=""
server_log=$(mktemp)
snapshot_file=$(mktemp)
environment_id=""
container_name=""
server_pid=""

cleanup() {
  if [[ -n "$environment_id" ]]; then
    curl --silent --max-time 5 --request DELETE \
      --header "Host: $authority" \
      --header "Origin: $origin" \
      --header "Authorization: Bearer $access_token" \
      "$base_url/api/environments/$environment_id" >/dev/null 2>&1 || true
  fi
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" >/dev/null 2>&1 || true
    for _ in $(seq 1 40); do
      state=$(ps -o stat= -p "$server_pid" 2>/dev/null | tr -d ' ' || true)
      if [[ -z "$state" || "$state" == Z* ]]; then
        break
      fi
      sleep 0.25
    done
    state=$(ps -o stat= -p "$server_pid" 2>/dev/null | tr -d ' ' || true)
    if [[ -n "$state" && "$state" != Z* ]]; then
      kill -KILL "$server_pid" >/dev/null 2>&1 || true
    fi
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
  private_url=$(sed -n 's/^Clannon is ready at //p' "$server_log" | tail -n 1)
  if [[ -n "$private_url" ]]; then
    break
  fi
  if ! kill -0 "$server_pid" 2>/dev/null; then
    cat "$server_log" >&2
    exit 1
  fi
  sleep 0.25
done
if [[ -z "${private_url:-}" ]]; then
  cat "$server_log" >&2
  exit 1
fi

base_url=${private_url%%/#*}
origin=$base_url
authority=${origin#http://}
access_token=${private_url##*#}
if [[ ! "$access_token" =~ ^[0-9a-f]{64}$ ]]; then
  printf 'invalid private URL in server output: %s\n' "$private_url" >&2
  exit 1
fi

curl --silent --fail --header "Host: $authority" "$base_url/" >/dev/null

status=$(curl --silent --output /dev/null --write-out '%{http_code}' \
  --request POST \
  --header 'Host: attacker.example' \
  --header "Origin: $origin" \
  --header "Authorization: Bearer $access_token" \
  "$base_url/api/environments")
[[ "$status" == "421" ]]

status=$(curl --silent --output /dev/null --write-out '%{http_code}' \
  --request POST \
  --header "Host: $authority" \
  --header "Origin: $origin" \
  "$base_url/api/environments")
[[ "$status" == "401" ]]

status=$(curl --silent --output /dev/null --write-out '%{http_code}' \
  --request POST \
  --header "Host: $authority" \
  --header 'Origin: http://evil.example' \
  --header "Authorization: Bearer $access_token" \
  "$base_url/api/environments")
[[ "$status" == "403" ]]

if ! create_response=$(curl --silent --show-error --fail-with-body \
  --request POST \
  --header "Host: $authority" \
  --header "Origin: $origin" \
  --header "Authorization: Bearer $access_token" \
  "$base_url/api/environments"); then
  printf '%s\n' "$create_response" >&2
  cat "$server_log" >&2
  exit 1
fi
environment_id=$(node -e 'const d=JSON.parse(process.argv[1]); if(!d.id) process.exit(1); process.stdout.write(d.id)' "$create_response")

mapfile -t environment_containers < <(
  podman ps --format '{{.Names}}' | awk -v prefix="clannon-${server_pid}-" 'index($0, prefix) == 1'
)
if [[ "${#environment_containers[@]}" -ne 1 ]]; then
  printf 'expected one server-owned container, found %s\n' "${#environment_containers[@]}" >&2
  podman ps --format '{{.Names}}' >&2
  exit 1
fi
container_name=${environment_containers[0]}

BASE_URL="$base_url" ACCESS_TOKEN="$access_token" ENVIRONMENT_ID="$environment_id" node <<'NODE'
const wsUrl = process.env.BASE_URL.replace(/^http/, "ws") +
  `/api/environments/${process.env.ENVIRONMENT_ID}/terminal?access_token=${encodeURIComponent(process.env.ACCESS_TOKEN)}`;
const protocol = "clannon.terminal.v1";
const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

async function waitUntil(predicate, label, milliseconds = 10_000) {
  const deadline = Date.now() + milliseconds;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await delay(25);
  }
  throw new Error(`timed out waiting for ${label}`);
}

async function openTerminal(expectedResumed, columns, rows) {
  const socket = new WebSocket(wsUrl, protocol);
  socket.binaryType = "arraybuffer";
  const decoder = new TextDecoder();
  const connection = {
    socket,
    output: "",
    outputBytes: [],
    controls: [],
    close: null,
    decoderFlushed: false,
  };
  const ready = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("terminal ready timed out")), 10_000);
    socket.addEventListener("open", () => {
      if (socket.protocol !== protocol) {
        reject(new Error(`unexpected subprotocol ${socket.protocol}`));
        return;
      }
      socket.send(JSON.stringify({ type: "open", version: 1, columns, rows }));
    });
    socket.addEventListener("message", (event) => {
      if (typeof event.data === "string") {
        const control = JSON.parse(event.data);
        connection.controls.push(control);
        if (control.type === "error") {
          reject(new Error(`${control.code}: ${control.message}`));
        } else if (control.type === "ready") {
          if (control.version !== 1 || control.resize !== true || control.resumed !== expectedResumed) {
            reject(new Error(`unexpected ready control ${event.data}`));
            return;
          }
          clearTimeout(timer);
          resolve(connection);
        }
        return;
      }
      if (!(event.data instanceof ArrayBuffer)) {
        reject(new Error("terminal output was not binary ArrayBuffer data"));
        return;
      }
      const bytes = new Uint8Array(event.data);
      connection.outputBytes.push(...bytes);
      connection.output += decoder.decode(bytes, { stream: true });
    });
    socket.addEventListener("error", () => reject(new Error("terminal websocket failed")));
  });
  socket.addEventListener("close", (event) => {
    if (!connection.decoderFlushed) {
      connection.output += decoder.decode();
      connection.decoderFlushed = true;
    }
    connection.close = { code: event.code, reason: event.reason };
  });
  return ready;
}

(async () => {
  const first = await openTerminal(false, 91, 33);
  first.socket.send(JSON.stringify({
    type: "input",
    data: "for fd in 0 1; do test -t \"$fd\" || exit 91; done\nprintf 'tty-proof:%s\\n' \"$(tty)\"\nprintf 'initial-size:'; stty size\ncd /tmp\nexport CLANNON_SMOKE_VAR=preserved\nprintf 'smoke-proof\\n' > /workspace/proof.txt\nnohup sleep 20 >/dev/null 2>&1 &\nnohup nc -l -s 0.0.0.0 -p 23456 >/dev/null 2>&1 &\nfor delay in 1 2 3 4 5 6 7 8 9 10; do grep -q ':5BA0 ' /proc/net/tcp && break; sleep 0.1; done\n(sleep 1; printf '%s%s\\n' detached -proof) &\nCLANNON_ATTACHED=attached\nCLANNON_ATTACHED=\"${CLANNON_ATTACHED}-proof\"\nprintf '%s\\n' \"$CLANNON_ATTACHED\"\n",
  }));
  await waitUntil(
    () => /tty-proof:\/dev\/pts\/[0-9]+/.test(first.output),
    "terminal stdin and stdout TTY proof",
  );
  await waitUntil(
    () => first.output.includes("initial-size:33 91"),
    "initial PTY dimensions",
  );
  await waitUntil(() => first.output.includes("attached-proof"), "initial terminal output");

  first.socket.send(JSON.stringify({ type: "resize", columns: 117, rows: 41 }));
  first.socket.send(JSON.stringify({
    type: "input",
    data: "printf 'resized-size:'; stty size\n",
  }));
  await waitUntil(
    () => first.output.includes("resized-size:41 117"),
    "later PTY resize",
  );

  first.socket.send(new TextEncoder().encode("sleep 30\n"));
  await waitUntil(() => first.output.includes("sleep 30"), "foreground sleep start");
  await delay(250);
  first.socket.send(Uint8Array.of(3));
  first.socket.send(JSON.stringify({
    type: "input",
    data: "printf 'after-etx:%s:%s\\n' \"$PWD\" \"$CLANNON_SMOKE_VAR\"\n",
  }));
  await waitUntil(
    () => first.output.includes("after-etx:/tmp:preserved"),
    "foreground sleep interruption and surviving shell",
    5_000,
  );
  if (first.controls.some((control) => control.type === "exit")) {
    throw new Error("raw ETX exited the shell instead of interrupting its foreground sleep");
  }

  first.socket.close();
  await waitUntil(() => first.close !== null, "initial disconnect");

  await delay(1_500);
  const resumed = await openTerminal(true, 73, 29);
  if (resumed.output.includes("detached-proof")) {
    throw new Error("detached output was replayed on the live terminal");
  }
  resumed.socket.send(JSON.stringify({
    type: "input",
    data: "printf 'reconnect-size:'; stty size\nprintf 'resume-proof:%s:%s\\n' \"$PWD\" \"$CLANNON_SMOKE_VAR\"\n",
  }));
  await waitUntil(
    () => resumed.output.includes("reconnect-size:29 73"),
    "reconnect dimensions applied before ready",
  );
  await waitUntil(
    () => resumed.output.includes("resume-proof:/tmp:preserved"),
    "preserved working directory and shell variable",
  );
  resumed.socket.send(JSON.stringify({ type: "input", data: "exit 7\n" }));
  await waitUntil(
    () => resumed.controls.some((control) => control.type === "exit" && control.code === 7),
    "structured shell exit",
  );
  await waitUntil(() => resumed.close !== null, "normal shell close");
  if (resumed.close.code !== 1000) {
    throw new Error(`shell exit closed with ${resumed.close.code}`);
  }

  const fresh = await openTerminal(false, 88, 27);
  fresh.socket.send(JSON.stringify({
    type: "input",
    data: "printf 'fresh-proof:%s:%s\\n' \"$PWD\" \"${CLANNON_SMOKE_VAR-unset}\"\n",
  }));
  await waitUntil(
    () => fresh.output.includes("fresh-proof:/workspace:unset"),
    "fresh shell after exit",
  );
  fresh.socket.close();
  await waitUntil(() => fresh.close !== null, "fresh shell disconnect");
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
NODE

curl --silent --fail \
  --header "Host: $authority" \
  --header "Origin: $origin" \
  --header "Authorization: Bearer $access_token" \
  "$base_url/api/environments/$environment_id/observations" >"$snapshot_file"
node - "$snapshot_file" <<'NODE'
const fs = require("node:fs");
const snapshot = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const inputTranscript = snapshot.transcript
  .filter((entry) => entry.direction === "input")
  .map((entry) => entry.data)
  .join("");
const outputTranscript = snapshot.transcript
  .filter((entry) => entry.direction === "output")
  .map((entry) => entry.data)
  .join("");
const hasProofFile = snapshot.files.some((file) => file.path === "/workspace/proof.txt");
const hasCommand = inputTranscript.includes("proof.txt");
const hasDetachedOutput = outputTranscript.includes("detached-proof");
const hasTtyProof = /tty-proof:\/dev\/pts\/[0-9]+/.test(outputTranscript);
const hasInitialSize = outputTranscript.includes("initial-size:33 91");
const hasResizeProof = outputTranscript.includes("resized-size:41 117");
const hasRawEtx = inputTranscript.includes("\u0003");
const hasAfterEtxProof = outputTranscript.includes("after-etx:/tmp:preserved");
const hasReconnectSize = outputTranscript.includes("reconnect-size:29 73");
const hasResumeProof = outputTranscript.includes("resume-proof:/tmp:preserved");
const hasFreshProof = outputTranscript.includes("fresh-proof:/workspace:unset");
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
if (!hasProofFile || !hasCommand || !hasDetachedOutput || !hasTtyProof || !hasInitialSize || !hasResizeProof || !hasRawEtx || !hasAfterEtxProof || !hasReconnectSize || !hasResumeProof || !hasFreshProof || !timestampsAreValid || !timestampsAreOrdered || !hasSleep || !hasTcpListener) {
  console.error(JSON.stringify({
    hasProofFile,
    hasCommand,
    hasDetachedOutput,
    hasTtyProof,
    hasInitialSize,
    hasResizeProof,
    hasRawEtx,
    hasAfterEtxProof,
    hasReconnectSize,
    hasResumeProof,
    hasFreshProof,
    timestampsAreValid,
    timestampsAreOrdered,
    hasSleep,
    hasTcpListener,
    snapshot,
  }, null, 2));
  process.exit(1);
}
NODE

mapfile -t proxy_pids < <(
  ps --ppid "$server_pid" -o pid=,args= | awk '$0 ~ /podman exec/ { print $1 }'
)
if [[ "${#proxy_pids[@]}" -ne 1 ]]; then
  printf 'expected one detached terminal proxy before destroy, found %s\n' "${#proxy_pids[@]}" >&2
  ps --ppid "$server_pid" -o pid=,args= >&2 || true
  exit 1
fi

curl --silent --fail --max-time 5 --request DELETE \
  --header "Host: $authority" \
  --header "Origin: $origin" \
  --header "Authorization: Bearer $access_token" \
  "$base_url/api/environments/$environment_id" >/dev/null
status=$(curl --silent --output /dev/null --write-out '%{http_code}' \
  --header "Host: $authority" \
  --header "Origin: $origin" \
  --header "Authorization: Bearer $access_token" \
  "$base_url/api/environments/$environment_id/observations")
[[ "$status" == "404" ]]

cleanup_complete=false
for _ in $(seq 1 50); do
  container_alive=false
  if podman container exists "$container_name" >/dev/null 2>&1; then
    container_alive=true
  fi

  proxy_alive=false
  for proxy_pid in "${proxy_pids[@]}"; do
    if kill -0 "$proxy_pid" >/dev/null 2>&1; then
      proxy_alive=true
      break
    fi
  done

  mapfile -t remaining_proxies < <(
    ps --ppid "$server_pid" -o pid=,args= | awk '$0 ~ /podman exec/ { print $1 }'
  )
  if [[ "$container_alive" == false && "$proxy_alive" == false && "${#remaining_proxies[@]}" -eq 0 ]]; then
    cleanup_complete=true
    break
  fi
  sleep 0.1
done
if [[ "$cleanup_complete" != true ]]; then
  printf 'destroy leaked container %s or its terminal proxy\n' "$container_name" >&2
  podman ps --all --format '{{.Names}}' >&2 || true
  ps --ppid "$server_pid" -o pid=,args= >&2 || true
  exit 1
fi
environment_id=""

echo "Clannon smoke test passed"
