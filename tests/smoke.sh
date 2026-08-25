#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
bind_address=${CLANNON_SMOKE_BIND:-127.0.0.1:39081}
server_binary=${CLANNON_SMOKE_BINARY:-}
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
if [[ -z "$server_binary" ]]; then
  cargo build --quiet
  server_binary="$project_dir/target/debug/clannon"
elif [[ ! -x "$server_binary" ]]; then
  printf 'CLANNON_SMOKE_BINARY is not executable: %s\n' "$server_binary" >&2
  exit 1
fi
CLANNON_BIND="$bind_address" "$server_binary" >"$server_log" 2>&1 &
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
if [[ ! "$environment_id" =~ ^env-[0-9a-f]{32}$ ]]; then
  printf 'environment ID is not an opaque 128-bit identifier: %s\n' "$environment_id" >&2
  exit 1
fi

mapfile -t environment_containers < <(
  podman ps --format '{{.Names}}' | awk -v prefix="clannon-${server_pid}-" 'index($0, prefix) == 1'
)
if [[ "${#environment_containers[@]}" -ne 1 ]]; then
  printf 'expected one server-owned container, found %s\n' "${#environment_containers[@]}" >&2
  podman ps --format '{{.Names}}' >&2
  exit 1
fi
container_name=${environment_containers[0]}
network_mode=$(podman inspect --format '{{.HostConfig.NetworkMode}}' "$container_name")
if [[ "$network_mode" != "none" ]]; then
  printf 'expected guest network mode none, found %s\n' "$network_mode" >&2
  exit 1
fi

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
    data: "for fd in 0 1; do test -t \"$fd\" || exit 91; done\nprintf 'tty-proof:%s\\n' \"$(tty)\"\nprintf 'initial-size:'; stty size\ncd /tmp\nexport CLANNON_SMOKE_VAR=preserved\nprintf 'smoke-proof\\n' > /workspace/proof.txt\nrm -f /workspace/loopback-proof.txt\nnc -l -s 127.0.0.1 -p 23457 < /dev/null > /workspace/loopback-proof.txt &\nloopback_listener=$!\nfor attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do\n  if printf loopback-proof | nc -w 1 127.0.0.1 23457; then break; fi\n  sleep 0.05\ndone\nwait \"$loopback_listener\"\ntest \"$(cat /workspace/loopback-proof.txt)\" = loopback-proof || exit 92\nprintf 'loopback-proof-ok\\n'\nnohup sleep 20 >/dev/null 2>&1 &\nnohup nc -l -s 127.0.0.1 -p 23456 >/dev/null 2>&1 &\nfor delay in 1 2 3 4 5 6 7 8 9 10; do grep -q ':5BA0 ' /proc/net/tcp && break; sleep 0.1; done\n(sleep 1; printf '%s%s\\n' detached -proof) &\nCLANNON_ATTACHED=attached\nCLANNON_ATTACHED=\"${CLANNON_ATTACHED}-proof\"\nprintf '%s\\n' \"$CLANNON_ATTACHED\"\n",
  }));
  await waitUntil(
    () => /tty-proof:\/dev\/pts\/[0-9]+/.test(first.output),
    "terminal stdin and stdout TTY proof",
  );
  await waitUntil(
    () => first.output.includes("initial-size:33 91"),
    "initial PTY dimensions",
  );
  await waitUntil(
    () => first.output.includes("loopback-proof-ok"),
    "guest loopback connection under network none",
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
  try {
    await waitUntil(
      () => resumed.controls.some((control) => control.type === "exit" && control.code === 7),
      "structured shell exit",
    );
  } catch (error) {
    throw new Error(`${error.message}; output=${JSON.stringify(resumed.output)} controls=${JSON.stringify(resumed.controls)}`);
  }
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

capture_observations() {
  curl --silent --fail --max-time 10 \
    --header "Host: $authority" \
    --header "Origin: $origin" \
    --header "Authorization: Bearer $access_token" \
    "$base_url/api/environments/$environment_id/observations" >"$snapshot_file"
}

# The first successful capture is a baseline and must not invent historical
# system changes from the environment's current contents.
capture_observations
node - "$snapshot_file" <<'NODE'
const fs = require("node:fs");
const snapshot = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const sampledTypes = /^(process|file|network)_(added|removed|changed)$/;
if (!Array.isArray(snapshot.execution_events) ||
    snapshot.execution_events.some((event) => sampledTypes.test(event.type))) {
  console.error("first observation did not establish a clean sampled-change baseline");
  process.exit(1);
}
NODE

podman exec "$container_name" sh -c "printf x > /workspace/sampled-change.txt"
podman exec --detach "$container_name" sh -c \
  'echo $$ > /tmp/clannon-sampled.pid; exec sleep 97' >/dev/null
for _ in $(seq 1 40); do
  if podman exec "$container_name" sh -c \
    'test -s /tmp/clannon-sampled.pid && kill -0 "$(cat /tmp/clannon-sampled.pid)"' 2>/dev/null; then
    break
  fi
  sleep 0.05
done
if ! podman exec "$container_name" sh -c \
  'test -s /tmp/clannon-sampled.pid && kill -0 "$(cat /tmp/clannon-sampled.pid)"'; then
  printf 'sampled process did not become durable\n' >&2
  exit 1
fi
capture_observations

podman exec "$container_name" sh -c \
  "printf 'sampled-change-expanded' > /workspace/sampled-change.txt"
capture_observations

podman exec "$container_name" sh -c \
  'rm /workspace/sampled-change.txt; kill "$(cat /tmp/clannon-sampled.pid)"'
for _ in $(seq 1 40); do
  if ! podman exec "$container_name" sh -c \
    'kill -0 "$(cat /tmp/clannon-sampled.pid)"' 2>/dev/null; then
    break
  fi
  sleep 0.05
done
if podman exec "$container_name" sh -c \
  'kill -0 "$(cat /tmp/clannon-sampled.pid)"' 2>/dev/null; then
  printf 'sampled process did not stop\n' >&2
  exit 1
fi
podman exec "$container_name" rm /tmp/clannon-sampled.pid
capture_observations

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
const hasExecutionEvents = Array.isArray(snapshot.execution_events);
const executionEvents = hasExecutionEvents ? snapshot.execution_events : [];
const executionEventsOmitted = snapshot.execution_events_omitted;
const eventKeys = {
  environment_ready: ["sequence", "timestamp_ms", "type"],
  shell_started: ["columns", "generation", "rows", "sequence", "timestamp_ms", "type"],
  terminal_input: ["bytes", "generation", "input_kind", "sequence", "timestamp_ms", "type"],
  terminal_resized: ["columns", "generation", "rows", "sequence", "timestamp_ms", "type"],
  shell_exited: ["code", "generation", "sequence", "timestamp_ms", "type"],
  shell_failed: ["generation", "sequence", "timestamp_ms", "type"],
  process_added: ["capture_sequence", "process", "sequence", "timestamp_ms", "type"],
  process_removed: ["capture_sequence", "process", "sequence", "timestamp_ms", "type"],
  process_changed: ["capture_sequence", "current", "previous", "sequence", "timestamp_ms", "type"],
  file_added: ["capture_sequence", "file", "sequence", "timestamp_ms", "type"],
  file_removed: ["capture_sequence", "file", "sequence", "timestamp_ms", "type"],
  file_changed: ["capture_sequence", "current", "previous", "sequence", "timestamp_ms", "type"],
  network_added: ["capture_sequence", "network", "sequence", "timestamp_ms", "type"],
  network_removed: ["capture_sequence", "network", "sequence", "timestamp_ms", "type"],
};
const hasExactKeys = (value, expected) =>
  value && typeof value === "object" && !Array.isArray(value) &&
  JSON.stringify(Object.keys(value).sort()) === JSON.stringify(expected);
const processShapeIsValid = (process) => hasExactKeys(
  process,
  ["arguments", "command", "parent_pid", "pid", "state"],
) && Number.isSafeInteger(process.pid) && process.pid > 0 &&
  Number.isSafeInteger(process.parent_pid) && process.parent_pid >= 0 &&
  [process.arguments, process.command, process.state].every((value) => typeof value === "string");
const fileShapeIsValid = (file) => hasExactKeys(
  file,
  ["kind", "modified_unix_seconds", "path", "size_bytes"],
) && Number.isSafeInteger(file.size_bytes) && file.size_bytes >= 0 &&
  Number.isSafeInteger(file.modified_unix_seconds) && file.modified_unix_seconds >= 0 &&
  typeof file.path === "string" && typeof file.kind === "string";
const networkShapeIsValid = (network) => hasExactKeys(
  network,
  ["local_address", "protocol", "remote_address", "state"],
) && [network.local_address, network.protocol, network.remote_address, network.state]
  .every((value) => typeof value === "string");
const executionEventShapesAreValid = hasExecutionEvents && executionEvents.every((event) => {
  const expected = eventKeys[event?.type];
  if (!expected || !hasExactKeys(event, expected)) return false;
  if (!Number.isSafeInteger(event.sequence) || event.sequence < 1 ||
      !Number.isSafeInteger(event.timestamp_ms) || event.timestamp_ms < 1) return false;
  if (/^(process|file|network)_/.test(event.type)) {
    if (!Number.isSafeInteger(event.capture_sequence) || event.capture_sequence < 1) return false;
    if (event.type.startsWith("process_")) {
      return event.type === "process_changed"
        ? processShapeIsValid(event.previous) && processShapeIsValid(event.current)
        : processShapeIsValid(event.process);
    }
    if (event.type.startsWith("file_")) {
      return event.type === "file_changed"
        ? fileShapeIsValid(event.previous) && fileShapeIsValid(event.current)
        : fileShapeIsValid(event.file);
    }
    return networkShapeIsValid(event.network);
  }
  if (event.type !== "environment_ready" &&
      (!Number.isSafeInteger(event.generation) || event.generation < 1)) return false;
  if (event.type === "shell_started" || event.type === "terminal_resized") {
    return Number.isSafeInteger(event.columns) && event.columns >= 1 && event.columns <= 1000 &&
      Number.isSafeInteger(event.rows) && event.rows >= 1 && event.rows <= 1000;
  }
  if (event.type === "terminal_input") {
    return ["text", "binary", "interrupt"].includes(event.input_kind) &&
      Number.isSafeInteger(event.bytes) && event.bytes > 0;
  }
  return event.type !== "shell_exited" || event.code === null || Number.isSafeInteger(event.code);
});
const executionEventSequencesAreOrdered = hasExecutionEvents && executionEvents.every(
  (event, index, events) => index === 0 || events[index - 1].sequence < event.sequence,
);
const findEventAfter = (after, predicate) => {
  for (let index = after + 1; index < executionEvents.length; index += 1) {
    if (predicate(executionEvents[index])) return index;
  }
  return -1;
};
let activityCursor = findEventAfter(-1, (event) => event.type === "environment_ready");
const environmentReadyIndex = activityCursor;
activityCursor = findEventAfter(activityCursor, (event) =>
  event.type === "shell_started" && event.generation === 1 &&
  event.columns === 91 && event.rows === 33
);
const firstShellIndex = activityCursor;
activityCursor = findEventAfter(activityCursor, (event) =>
  event.type === "terminal_input" && event.generation === 1 && event.input_kind === "text"
);
const firstInputIndex = activityCursor;
activityCursor = findEventAfter(activityCursor, (event) =>
  event.type === "terminal_resized" && event.generation === 1 &&
  event.columns === 117 && event.rows === 41
);
const explicitResizeIndex = activityCursor;
activityCursor = findEventAfter(activityCursor, (event) =>
  event.type === "terminal_input" && event.generation === 1 && event.input_kind === "text" &&
  event.bytes === Buffer.byteLength("printf 'resized-size:'; stty size\n")
);
const resizedProofInputIndex = activityCursor;
activityCursor = findEventAfter(activityCursor, (event) =>
  event.type === "terminal_input" && event.generation === 1 && event.input_kind === "binary" &&
  event.bytes === Buffer.byteLength("sleep 30\n")
);
const binaryInputIndex = activityCursor;
activityCursor = findEventAfter(activityCursor, (event) =>
  event.type === "terminal_input" && event.generation === 1 &&
  event.input_kind === "interrupt" && event.bytes === 1
);
const interruptIndex = activityCursor;
activityCursor = findEventAfter(activityCursor, (event) =>
  event.type === "terminal_input" && event.generation === 1 && event.input_kind === "text" &&
  event.bytes === Buffer.byteLength("printf 'after-etx:%s:%s\\n' \"$PWD\" \"$CLANNON_SMOKE_VAR\"\n")
);
const afterInterruptInputIndex = activityCursor;
activityCursor = findEventAfter(activityCursor, (event) =>
  event.type === "terminal_resized" && event.generation === 1 &&
  event.columns === 73 && event.rows === 29
);
const reconnectResizeIndex = activityCursor;
activityCursor = findEventAfter(activityCursor, (event) =>
  event.type === "terminal_input" && event.generation === 1 && event.input_kind === "text" &&
  event.bytes === Buffer.byteLength("printf 'reconnect-size:'; stty size\nprintf 'resume-proof:%s:%s\\n' \"$PWD\" \"$CLANNON_SMOKE_VAR\"\n")
);
const resumedInputIndex = activityCursor;
activityCursor = findEventAfter(activityCursor, (event) =>
  event.type === "terminal_input" && event.generation === 1 && event.input_kind === "text" &&
  event.bytes === Buffer.byteLength("exit 7\n")
);
const exitInputIndex = activityCursor;
activityCursor = findEventAfter(activityCursor, (event) =>
  event.type === "shell_exited" && event.generation === 1 && event.code === 7
);
const firstShellExitIndex = activityCursor;
activityCursor = findEventAfter(activityCursor, (event) =>
  event.type === "shell_started" && event.generation === 2 &&
  event.columns === 88 && event.rows === 27
);
const secondShellIndex = activityCursor;
activityCursor = findEventAfter(activityCursor, (event) =>
  event.type === "terminal_input" && event.generation === 2 && event.input_kind === "text" &&
  event.bytes === Buffer.byteLength("printf 'fresh-proof:%s:%s\\n' \"$PWD\" \"${CLANNON_SMOKE_VAR-unset}\"\n")
);
const secondShellInputIndex = activityCursor;
const sampledFilePath = "/workspace/sampled-change.txt";
const sampledProcess = (process) => process.command === "sleep" &&
  /(^|\s)97($|\s)/.test(process.arguments);
const fileAddedIndex = executionEvents.findIndex((event) =>
  event.type === "file_added" && event.file.path === sampledFilePath &&
  event.file.size_bytes === Buffer.byteLength("x")
);
const fileChangedIndex = executionEvents.findIndex((event) =>
  event.type === "file_changed" && event.previous.path === sampledFilePath &&
  event.current.path === sampledFilePath &&
  event.previous.size_bytes === Buffer.byteLength("x") &&
  event.current.size_bytes === Buffer.byteLength("sampled-change-expanded")
);
const fileRemovedIndex = executionEvents.findIndex((event) =>
  event.type === "file_removed" && event.file.path === sampledFilePath &&
  event.file.size_bytes === Buffer.byteLength("sampled-change-expanded")
);
const processAddedIndex = executionEvents.findIndex((event) =>
  event.type === "process_added" && sampledProcess(event.process)
);
const processRemovedIndex = executionEvents.findIndex((event) =>
  event.type === "process_removed" && sampledProcess(event.process)
);
const fileAdded = executionEvents[fileAddedIndex];
const fileChanged = executionEvents[fileChangedIndex];
const fileRemoved = executionEvents[fileRemovedIndex];
const processAdded = executionEvents[processAddedIndex];
const processRemoved = executionEvents[processRemovedIndex];
const sampledActivitySemanticsAreValid = [
  fileAddedIndex,
  fileChangedIndex,
  fileRemovedIndex,
  processAddedIndex,
  processRemovedIndex,
].every((index) => index >= 0) &&
  fileAddedIndex < fileChangedIndex && fileChangedIndex < fileRemovedIndex &&
  processAddedIndex < processRemovedIndex &&
  fileAdded.capture_sequence === processAdded.capture_sequence &&
  fileAdded.timestamp_ms === processAdded.timestamp_ms &&
  fileAdded.capture_sequence < fileChanged.capture_sequence &&
  fileChanged.capture_sequence < fileRemoved.capture_sequence &&
  fileRemoved.capture_sequence === processRemoved.capture_sequence &&
  fileRemoved.timestamp_ms === processRemoved.timestamp_ms &&
  processAdded.process.pid === processRemoved.process.pid;
const activitySemanticsAreValid = executionEventShapesAreValid &&
  executionEventSequencesAreOrdered && executionEventsOmitted === 0 &&
  executionEvents[0]?.sequence === 1 && environmentReadyIndex === 0 &&
  secondShellInputIndex >= 0 && sampledActivitySemanticsAreValid &&
  executionEvents.filter((event) => event.type === "shell_started" && event.generation === 1).length === 1 &&
  !executionEvents.some((event) => event.type === "shell_exited" && event.generation === 2) &&
  !executionEvents.some((event) => event.type === "shell_failed");
const hasProofFile = snapshot.files.some((file) => file.path === "/workspace/proof.txt");
const hasCommand = inputTranscript.includes("proof.txt");
const hasDetachedOutput = outputTranscript.includes("detached-proof");
const hasTtyProof = /tty-proof:\/dev\/pts\/[0-9]+/.test(outputTranscript);
const hasInitialSize = outputTranscript.includes("initial-size:33 91");
const hasLoopbackProof = outputTranscript.includes("loopback-proof-ok");
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
  socket.local_address === "127.0.0.1:23456" &&
  socket.state === "listening"
);
const sampledFactsAreAbsent = !snapshot.files.some((file) => file.path === sampledFilePath) &&
  !snapshot.processes.some(sampledProcess);
if (!activitySemanticsAreValid || !sampledFactsAreAbsent || !hasProofFile || !hasCommand || !hasDetachedOutput || !hasTtyProof || !hasInitialSize || !hasLoopbackProof || !hasResizeProof || !hasRawEtx || !hasAfterEtxProof || !hasReconnectSize || !hasResumeProof || !hasFreshProof || !timestampsAreValid || !timestampsAreOrdered || !hasSleep || !hasTcpListener) {
  console.error(JSON.stringify({
    activitySemanticsAreValid,
    executionEventShapesAreValid,
    executionEventSequencesAreOrdered,
    executionEventsOmitted,
    sampledActivitySemanticsAreValid,
    sampledFactsAreAbsent,
    sampledIndexes: {
      fileAddedIndex,
      fileChangedIndex,
      fileRemovedIndex,
      processAddedIndex,
      processRemovedIndex,
    },
    activityIndexes: {
      environmentReadyIndex,
      firstShellIndex,
      firstInputIndex,
      explicitResizeIndex,
      resizedProofInputIndex,
      binaryInputIndex,
      interruptIndex,
      afterInterruptInputIndex,
      reconnectResizeIndex,
      resumedInputIndex,
      exitInputIndex,
      firstShellExitIndex,
      secondShellIndex,
      secondShellInputIndex,
    },
    hasProofFile,
    hasCommand,
    hasDetachedOutput,
    hasTtyProof,
    hasInitialSize,
    hasLoopbackProof,
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
