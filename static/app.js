const elements = {
  create: document.querySelector("#create"),
  colorTheme: document.querySelector("#color-theme"),
  destroy: document.querySelector("#destroy"),
  refresh: document.querySelector("#refresh"),
  form: document.querySelector("#terminal-form"),
  command: document.querySelector("#command"),
  run: document.querySelector("#terminal-form button[type='submit']"),
  reconnect: document.querySelector("#reconnect"),
  output: document.querySelector("#terminal-output"),
  commandPrompt: document.querySelector("#command-prompt"),
  environmentId: document.querySelector("#environment-id"),
  terminalStatus: document.querySelector("#terminal-status"),
  terminalAnnouncement: document.querySelector("#terminal-announcement"),
  stateDot: document.querySelector("#state-dot"),
  stateLabel: document.querySelector("#state-label"),
  warnings: document.querySelector("#warnings"),
  snapshotTime: document.querySelector("#snapshot-time"),
};

const MAX_TERMINAL_OUTPUT_CHARACTERS = 200 * 1024;
const TERMINAL_OMISSION_MARKER = "[earlier terminal output omitted]\n";
const TERMINAL_PROTOCOL = "clannon.terminal.v1";
const TERMINAL_VERSION = 1;
const MAX_TERMINAL_INPUT_BYTES = 64 * 1024;
const PROTOCOL_ERROR_RECOVERY = "Destroy this environment before reloading the page.";
const ACCESS_TOKEN_STORAGE_KEY = "clannon-access-token";
const ACCESS_INSTRUCTION = "Open the private URL printed by Clannon.";
const responseAccessGenerations = new WeakMap();

let environmentId = null;
let socket = null;
let terminalState = "disconnected";
let destroying = false;
let accessFragmentCleanupFailed = false;
let accessToken = bootstrapAccessToken();
let accessGeneration = 0;
let accessCandidateGeneration = 0;

setThemePreference(readThemePreference());
elements.create.addEventListener("click", createEnvironment);
elements.colorTheme.addEventListener("change", () => setThemePreference(elements.colorTheme.value, true));
elements.destroy.addEventListener("click", destroyEnvironment);
elements.refresh.addEventListener("click", refreshObservations);
elements.form.addEventListener("submit", runCommand);
elements.command.addEventListener("keydown", handleCommandKeydown);
elements.command.addEventListener("input", updateCommandComposer);
elements.reconnect.addEventListener("click", handleReconnect);
window.addEventListener("beforeunload", () => socket?.close());
window.addEventListener("hashchange", handleAccessFragment);
updateEnvironmentControls();
if (!accessToken) {
  lockForAccess();
} else if (accessFragmentCleanupFailed) {
  elements.output.textContent = "Private URL remains in the address bar because browser storage is unavailable. Close this tab when finished.";
  announceTerminal(elements.output.textContent);
}

function bootstrapAccessToken() {
  let fragment = "";
  try {
    fragment = location.hash.slice(1);
  } catch {}
  if (isValidAccessToken(fragment)) {
    let stored = false;
    try {
      sessionStorage.setItem(ACCESS_TOKEN_STORAGE_KEY, fragment);
      stored = true;
    } catch {}
    cleanAccessFragment(stored);
    return fragment;
  }
  try {
    const stored = sessionStorage.getItem(ACCESS_TOKEN_STORAGE_KEY);
    return isValidAccessToken(stored) ? stored : null;
  } catch {
    return null;
  }
}

function cleanAccessFragment(stored) {
  const cleanUrl = `${location.pathname}${location.search}`;
  try {
    history.replaceState(history.state, "", cleanUrl);
    return;
  } catch {}
  if (stored) {
    try {
      location.replace(cleanUrl);
      return;
    } catch {}
  }
  accessFragmentCleanupFailed = true;
}

async function handleAccessFragment() {
  let fragment = "";
  try {
    fragment = location.hash.slice(1);
  } catch {}
  if (!isValidAccessToken(fragment)) return;
  const candidateGeneration = ++accessCandidateGeneration;
  if (fragment !== accessToken) {
    let response;
    try {
      response = await fetch("/api/access", {
        cache: "no-store",
        headers: { Authorization: `Bearer ${fragment}` },
      });
    } catch {
      if (candidateGeneration !== accessCandidateGeneration) return;
      announceTerminal("Could not verify the private URL. Try again when Clannon is reachable.");
      return;
    }
    if (candidateGeneration !== accessCandidateGeneration) return;
    if (response.status !== 204) {
      cleanAccessFragment(false);
      announceTerminal("The private URL was rejected. Open the current URL printed by Clannon.");
      return;
    }
  }
  let stored = false;
  try {
    sessionStorage.setItem(ACCESS_TOKEN_STORAGE_KEY, fragment);
    stored = true;
  } catch {}
  cleanAccessFragment(stored);
  if (fragment === accessToken) return;

  socket?.close();
  socket = null;
  accessToken = fragment;
  accessGeneration += 1;
  environmentId = null;
  destroying = false;
  resetEnvironment();
  elements.output.textContent = "Private access restored. Create an environment when ready.";
}

function isValidAccessToken(value) {
  return typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
}

async function apiFetch(url, options = {}) {
  if (!accessToken) {
    lockForAccess();
    throw new Error(ACCESS_INSTRUCTION);
  }
  const requestToken = accessToken;
  const requestGeneration = accessGeneration;
  let response;
  try {
    response = await fetch(url, {
      ...options,
      headers: {
        ...(options.headers ?? {}),
        Authorization: `Bearer ${requestToken}`,
      },
    });
  } catch (error) {
    if (accessGeneration !== requestGeneration) throw new StaleAccessRequest();
    throw error;
  }
  if (accessGeneration !== requestGeneration) throw new StaleAccessRequest();
  responseAccessGenerations.set(response, requestGeneration);
  if (response.status === 401 && accessToken === requestToken) {
    accessToken = null;
    accessGeneration += 1;
    try {
      sessionStorage.removeItem(ACCESS_TOKEN_STORAGE_KEY);
    } catch {}
    lockForAccess();
    throw new Error(ACCESS_INSTRUCTION);
  }
  return response;
}

function lockForAccess() {
  socket?.close();
  socket = null;
  terminalState = "disconnected";
  elements.create.disabled = true;
  elements.destroy.disabled = true;
  elements.refresh.disabled = true;
  elements.command.disabled = true;
  elements.run.disabled = true;
  elements.reconnect.hidden = true;
  elements.reconnect.disabled = true;
  elements.stateDot.classList.remove("running");
  elements.stateLabel.textContent = "Private access required";
  elements.terminalStatus.dataset.state = "disconnected";
  elements.terminalStatus.textContent = "Locked";
  elements.output.textContent = ACCESS_INSTRUCTION;
  announceTerminal(ACCESS_INSTRUCTION);
}

function readThemePreference() {
  try {
    const theme = localStorage.getItem("clannon-color-theme");
    if (["system", "light", "dark"].includes(theme)) return theme;
  } catch {}
  return "system";
}

function setThemePreference(theme, persist = false) {
  const preference = ["light", "dark"].includes(theme) ? theme : "system";
  elements.colorTheme.value = preference;
  if (preference === "system") {
    delete document.documentElement.dataset.theme;
  } else {
    document.documentElement.dataset.theme = preference;
  }
  if (persist) {
    try {
      localStorage.setItem("clannon-color-theme", preference);
    } catch {}
  }
}

async function createEnvironment() {
  if (!accessToken) {
    lockForAccess();
    return;
  }
  setBusy(true, "Starting isolated Linux…");
  elements.output.textContent = "Creating a rootless container. The first run may pull a small image…\n";
  try {
    const response = await apiFetch("/api/environments", { method: "POST" });
    const data = await readResponse(response);
    environmentId = data.id;
    elements.environmentId.textContent = environmentId;
    elements.stateLabel.textContent = "Environment running";
    elements.stateDot.classList.add("running");
    updateEnvironmentControls();
    connectTerminal(true);
    await refreshObservations();
  } catch (error) {
    if (error instanceof StaleAccessRequest) return;
    if (!accessToken) {
      lockForAccess();
      return;
    }
    showTerminalError(error);
    resetEnvironment();
  } finally {
    setBusy(false);
  }
}

function connectTerminal(clearOutput) {
  if (!accessToken) {
    lockForAccess();
    return;
  }
  if (!environmentId || socket?.readyState === WebSocket.CONNECTING || socket?.readyState === WebSocket.OPEN) return;

  const id = environmentId;
  const openingAfterEnd = terminalState === "ended";
  const protocol = location.protocol === "https:" ? "wss" : "ws";
  const terminalSocket = new WebSocket(
    `${protocol}://${location.host}/api/environments/${id}/terminal?access_token=${encodeURIComponent(accessToken)}`,
    TERMINAL_PROTOCOL,
  );
  terminalSocket.binaryType = "arraybuffer";
  socket = terminalSocket;
  const decoder = new TextDecoder();
  let decoderFlushed = false;
  const reconnecting = !clearOutput;
  setTerminalState(
    reconnecting ? "reconnecting" : "connecting",
    `${reconnecting ? "Reconnecting" : "Connecting"} to the environment shell. Run remains unavailable until the shell is ready.`,
  );

  const flushDecoder = (appendTail = true) => {
    if (decoderFlushed) return;
    decoderFlushed = true;
    const tail = decoder.decode();
    if (tail && appendTail) appendOutput(tail);
  };

  const protocolError = (message) => {
    if (!isCurrentSocket(terminalSocket, id)) return;
    flushDecoder();
    showTerminalError(`Terminal protocol error: ${message} ${PROTOCOL_ERROR_RECOVERY}`);
    setTerminalState("protocol-error", `Terminal protocol error. ${message} ${PROTOCOL_ERROR_RECOVERY}`);
    terminalSocket.close(1002, "terminal protocol error");
  };

  terminalSocket.addEventListener("open", () => {
    if (!isCurrentSocket(terminalSocket, id)) return;
    if (terminalSocket.protocol !== TERMINAL_PROTOCOL) {
      protocolError("the server did not negotiate clannon.terminal.v1.");
      return;
    }
    try {
      terminalSocket.send(JSON.stringify({
        type: "open",
        version: TERMINAL_VERSION,
        columns: 80,
        rows: 24,
      }));
    } catch (error) {
      showTerminalError(error);
      terminalSocket.close();
    }
  });
  terminalSocket.addEventListener("message", (event) => {
    if (!isCurrentSocket(terminalSocket, id)) return;
    if (typeof event.data === "string") {
      handleTerminalControl(event.data, {
        clearOutput,
        flushDecoder,
        openingAfterEnd,
        protocolError,
      });
      return;
    }
    if (!(event.data instanceof ArrayBuffer)) {
      protocolError("server output was not an ArrayBuffer.");
      return;
    }
    if (terminalState !== "ready" || decoderFlushed) {
      protocolError("binary output arrived outside a ready terminal session.");
      return;
    }
    try {
      const text = decoder.decode(event.data, { stream: true });
      if (text) appendOutput(text);
    } catch {
      protocolError("server output could not be decoded.");
    }
  });
  terminalSocket.addEventListener("close", () => {
    const current = isCurrentSocket(terminalSocket, id);
    flushDecoder(current);
    if (!current) return;
    const accessMayBeStale = ["connecting", "reconnecting"].includes(terminalState);
    socket = null;
    if (!environmentId) return;
    if (["ended", "terminal-error", "protocol-error"].includes(terminalState)) {
      updateTerminalControls();
    } else {
      appendOutput("\n[terminal disconnected — the environment may still be inspected]\n");
      setTerminalState("disconnected", "Terminal disconnected. Your draft and visible output were preserved. Reconnect when ready.");
      if (accessMayBeStale) void verifyAccessAfterSocketFailure();
    }
  });
  terminalSocket.addEventListener("error", () => {
    if (!isCurrentSocket(terminalSocket, id)) return;
    showTerminalError("Could not connect the browser terminal.");
    announceTerminal("The terminal connection reported an error. Your draft and visible output are preserved.");
  });
}

async function verifyAccessAfterSocketFailure() {
  try {
    await apiFetch("/api/access", { cache: "no-store" });
  } catch {
    if (!accessToken) lockForAccess();
  }
}

function handleTerminalControl(text, connection) {
  let control;
  try {
    control = JSON.parse(text);
  } catch {
    connection.protocolError("server text was not a valid control message.");
    return;
  }
  if (!control || Array.isArray(control) || typeof control !== "object" || typeof control.type !== "string") {
    connection.protocolError("server text was not a valid control message.");
    return;
  }

  if (control.type === "ready") {
    if (!["connecting", "reconnecting"].includes(terminalState)
      || !hasExactKeys(control, ["type", "version", "resumed", "resize"])
      || control.version !== TERMINAL_VERSION
      || typeof control.resumed !== "boolean"
      || control.resize !== false) {
      connection.protocolError("received a malformed or out-of-sequence ready control.");
      return;
    }
    if (connection.clearOutput) {
      elements.output.textContent = "Clannon environment ready.\n";
    } else if (control.resumed) {
      appendOutput("\n[shell preserved — recent output produced while detached may be in Transcript]\n");
      void refreshObservations();
    } else if (connection.openingAfterEnd) {
      appendOutput("\n[fresh shell opened after the previous shell ended]\n");
    } else {
      appendOutput("\n[previous shell was unavailable — fresh shell opened]\n");
    }
    const announcement = control.resumed
      ? "Terminal ready. The existing shell was preserved. Recent output produced while detached may be available in Transcript."
      : "Terminal ready with a fresh shell.";
    setTerminalState("ready", announcement);
    elements.command.focus();
    return;
  }

  if (control.type === "exit") {
    const validCode = control.code === null
      || (Number.isSafeInteger(control.code) && control.code >= -2_147_483_648 && control.code <= 2_147_483_647);
    if (terminalState !== "ready" || !hasExactKeys(control, ["type", "code"]) || !validCode) {
      connection.protocolError("received a malformed or out-of-sequence exit control.");
      return;
    }
    connection.flushDecoder();
    const code = control.code === null ? "unknown" : control.code;
    appendOutput(`\n[shell exited with code ${code}]\n`);
    setTerminalState("ended", `Shell exited with code ${code}. Open a new shell to continue in this environment.`);
    return;
  }

  if (control.type === "error") {
    if (!hasExactKeys(control, ["type", "code", "message"])
      || !["protocol_error", "runtime_error", "output_gap"].includes(control.code)
      || typeof control.message !== "string" || !control.message) {
      connection.protocolError("received a malformed error control.");
      return;
    }
    connection.flushDecoder();
    const recovery = control.code === "protocol_error" ? ` ${PROTOCOL_ERROR_RECOVERY}` : "";
    showTerminalError(`${control.code}: ${control.message}${recovery}`);
    if (control.code === "protocol_error") {
      setTerminalState("protocol-error", `Terminal protocol error. ${control.message} ${PROTOCOL_ERROR_RECOVERY}`);
    } else if (control.code === "output_gap") {
      setTerminalState("terminal-error", `Some live terminal output was missed. ${control.message} Reconnect to continue; Transcript may contain the missed output.`);
    } else {
      setTerminalState("ended", `The shell stopped because of a runtime error. ${control.message} Open a new shell to continue.`);
    }
    return;
  }

  connection.protocolError(`received unknown control type ${control.type}.`);
}

function hasExactKeys(value, expected) {
  const keys = Object.keys(value).sort();
  const expectedKeys = [...expected].sort();
  return keys.length === expectedKeys.length
    && keys.every((key, index) => key === expectedKeys[index]);
}

function handleReconnect() {
  connectTerminal(false);
}

function runCommand(event) {
  event.preventDefault();
  const command = elements.command.value;
  if (!command) return;
  if (lineEndsWithContinuation(command, command.length)) {
    elements.command.value += "\n";
    elements.command.selectionStart = elements.command.value.length;
    updateCommandComposer();
    elements.command.focus();
    return;
  }
  if (terminalState !== "ready" || socket?.readyState !== WebSocket.OPEN) {
    showTerminalError("Terminal is not ready. Reconnect before running commands.");
    updateTerminalControls();
    return;
  }
  const input = `${command}\n`;
  const inputBytes = new TextEncoder().encode(input).byteLength;
  if (inputBytes > MAX_TERMINAL_INPUT_BYTES) {
    showTerminalError(`Command is ${inputBytes} bytes; the terminal limit is ${MAX_TERMINAL_INPUT_BYTES} bytes.`);
    announceTerminal("The command was not sent because it exceeds the 64 KiB terminal input limit. Your draft was preserved.");
    return;
  }
  try {
    socket.send(JSON.stringify({ type: "input", data: input }));
  } catch (error) {
    showTerminalError(error);
    announceTerminal("The command was not sent. Your draft was preserved so you can try again.");
    return;
  }
  appendSubmittedCommand(command);
  elements.command.value = "";
  updateCommandComposer();
  window.setTimeout(refreshObservations, 350);
}

function handleCommandKeydown(event) {
  if (event.key !== "Enter" || event.isComposing) return;
  if (event.shiftKey || lineEndsWithContinuation(elements.command.value, elements.command.selectionStart)) {
    window.requestAnimationFrame(updateCommandComposer);
    return;
  }
  event.preventDefault();
  elements.form.requestSubmit();
}

function lineEndsWithContinuation(command, cursor) {
  const lineStart = command.lastIndexOf("\n", cursor - 1) + 1;
  const trailingBackslashes = command.slice(lineStart, cursor).match(/\\+$/)?.[0].length ?? 0;
  return trailingBackslashes % 2 === 1;
}

function updateCommandComposer() {
  const lines = elements.command.value.split("\n");
  const previousLine = lines.at(-2);
  elements.commandPrompt.textContent = previousLine !== undefined
    && lineEndsWithContinuation(previousLine, previousLine.length) ? ">" : "$";
  elements.command.style.height = "auto";
  elements.command.style.height = `${Math.min(elements.command.scrollHeight, 128)}px`;
}

function appendSubmittedCommand(command) {
  const leadingNewline = elements.output.textContent && !elements.output.textContent.endsWith("\n") ? "\n" : "";
  let continuing = false;
  const rendered = command.split("\n").map((line, index) => {
    const prompt = index > 0 && continuing ? ">" : "$";
    continuing = lineEndsWithContinuation(line, line.length);
    return `${prompt} ${line}`;
  }).join("\n");
  appendOutput(`${leadingNewline}${rendered}\n`);
}

async function refreshObservations() {
  if (!environmentId || !accessToken) return;
  elements.refresh.disabled = true;
  try {
    const response = await apiFetch(`/api/environments/${environmentId}/observations`);
    const snapshot = await readResponse(response);
    renderTranscript(snapshot.transcript);
    renderProcesses(snapshot.processes);
    renderFiles(snapshot.files);
    renderNetwork(snapshot.network);
    renderWarnings(snapshot.warnings);
    elements.snapshotTime.textContent = `Captured ${new Date(Number(snapshot.captured_at_ms)).toLocaleTimeString()}`;
  } catch (error) {
    if (error instanceof StaleAccessRequest) return;
    renderWarnings([String(error)]);
  } finally {
    updateEnvironmentControls();
  }
}

function renderTranscript(transcript) {
  const container = document.querySelector("#transcript");
  const visible = transcript.slice(-100);
  const omitted = transcript.length - visible.length;
  document.querySelector("#transcript-count").textContent = transcript.length;
  container.classList.toggle("empty", transcript.length === 0);
  if (transcript.length === 0) {
    container.textContent = "Nothing recorded in this snapshot.";
    return;
  }

  const omission = omitted > 0
    ? `<p class="transcript-omission">Showing the newest ${visible.length} of ${transcript.length} events.</p>`
    : "";
  container.innerHTML = omission + visible.map((entry) => {
    const input = entry.direction === "input";
    const recordedAt = new Date(Number(entry.timestamp_ms));
    const timestamp = Number.isNaN(recordedAt.getTime())
      ? "Unknown time"
      : recordedAt.toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        fractionalSecondDigits: 3,
      });
    const datetime = Number.isNaN(recordedAt.getTime()) ? "" : recordedAt.toISOString();
    const data = entry.data === "" ? "∅" : String(entry.data);
    return `
      <div class="evidence-row transcript-row" data-direction="${input ? "input" : "output"}">
        <span><time datetime="${datetime}" title="${datetime}">${escapeHtml(timestamp)}</time> · ${input ? "Command" : "Output"}</span>
        <code>${escapeHtml(data)}</code>
      </div>`;
  }).join("");
}

async function destroyEnvironment() {
  if (!environmentId || !accessToken) return;
  const id = environmentId;
  destroying = true;
  updateEnvironmentControls();
  elements.stateLabel.textContent = "Destroying environment…";
  try {
    const response = await apiFetch(`/api/environments/${id}`, { method: "DELETE" });
    await readResponse(response);
    const destroyedSocket = socket;
    resetEnvironment();
    destroyedSocket?.close();
    elements.output.textContent = "Environment destroyed. Its container and writable data are gone.";
  } catch (error) {
    if (error instanceof StaleAccessRequest) return;
    if (!accessToken) {
      lockForAccess();
      return;
    }
    environmentId = id;
    destroying = false;
    updateEnvironmentControls();
    elements.stateLabel.textContent = "Destroy failed";
    showTerminalError(error);
  }
}

function renderProcesses(processes) {
  renderList("processes", "process-count", processes, (process) => `
    <div class="evidence-row">
      <code>${escapeHtml(process.command)}</code>
      <span>pid ${process.pid} · parent ${process.parent_pid} · ${escapeHtml(process.state)}</span>
      <small>${escapeHtml(process.arguments || "—")}</small>
    </div>`);
}

function renderFiles(files) {
  renderList("files", "file-count", files, (file) => `
    <div class="evidence-row">
      <code>${escapeHtml(file.path.replace("/workspace/", ""))}</code>
      <span>${formatBytes(file.size_bytes)} · ${escapeHtml(file.kind)}</span>
    </div>`);
}

function renderNetwork(network) {
  renderList("network", "network-count", network, (socket) => `
    <div class="evidence-row horizontal">
      <code>${escapeHtml(socket.protocol)}</code>
      <span>${escapeHtml(socket.local_address)} · ${escapeHtml(socket.state)}</span>
    </div>`);
}

function renderList(containerId, countId, items, renderer) {
  const container = document.querySelector(`#${containerId}`);
  document.querySelector(`#${countId}`).textContent = items.length;
  container.classList.toggle("empty", items.length === 0);
  container.innerHTML = items.length ? items.map(renderer).join("") : "Nothing observed in this snapshot.";
}

function renderWarnings(warnings) {
  elements.warnings.hidden = warnings.length === 0;
  elements.warnings.textContent = warnings.join(" · ");
}

function resetEnvironment() {
  environmentId = null;
  socket = null;
  destroying = false;
  elements.command.value = "";
  updateCommandComposer();
  elements.environmentId.textContent = "waiting for environment";
  elements.stateLabel.textContent = accessToken ? "No environment" : "Private access required";
  elements.stateDot.classList.remove("running");
  updateEnvironmentControls();
  setTerminalState("disconnected", "No environment is active.");
  ["transcript", "processes", "files", "network"].forEach((id) => {
    const container = document.querySelector(`#${id}`);
    container.classList.add("empty");
    container.textContent = "No snapshot yet.";
  });
  ["transcript-count", "process-count", "file-count", "network-count"].forEach((id) => {
    document.querySelector(`#${id}`).textContent = "0";
  });
  elements.snapshotTime.textContent = "Evidence appears after the environment starts.";
  elements.warnings.hidden = true;
}

function updateEnvironmentControls() {
  const hasAccess = Boolean(accessToken);
  const hasEnvironment = Boolean(environmentId);
  elements.destroy.disabled = !hasAccess || !hasEnvironment || destroying;
  elements.refresh.disabled = !hasAccess || !hasEnvironment || destroying;
  elements.create.disabled = !hasAccess || hasEnvironment;
  updateTerminalControls();
}

function updateTerminalControls() {
  const hasAccess = Boolean(accessToken);
  const hasEnvironment = hasAccess && Boolean(environmentId) && !destroying;
  const ready = hasEnvironment && terminalState === "ready" && socket?.readyState === WebSocket.OPEN;
  elements.command.disabled = !hasEnvironment;
  elements.run.disabled = !ready;
  const canReconnect = hasEnvironment
    && !socket
    && ["disconnected", "ended", "terminal-error"].includes(terminalState);
  elements.reconnect.hidden = !canReconnect;
  elements.reconnect.disabled = !canReconnect;
  elements.reconnect.textContent = terminalState === "ended"
    ? "Open new shell"
    : "Reconnect";
}

function setTerminalState(state, announcement = "") {
  terminalState = state;
  elements.terminalStatus.dataset.state = state;
  const labels = {
    "protocol-error": "Protocol error",
    "terminal-error": "Terminal error",
  };
  elements.terminalStatus.textContent = labels[state] ?? state[0].toUpperCase() + state.slice(1);
  if (announcement) announceTerminal(announcement);
  updateTerminalControls();
}

function announceTerminal(message) {
  elements.terminalAnnouncement.textContent = message;
}

function isCurrentSocket(candidate, id) {
  return socket === candidate && environmentId === id;
}

function setBusy(busy, label = "") {
  elements.create.disabled = !accessToken || busy || Boolean(environmentId);
  elements.create.textContent = busy ? "Creating…" : "Create environment";
  if (label) elements.stateLabel.textContent = label;
}

function appendOutput(text) {
  const alreadyOmitted = elements.output.textContent.startsWith(TERMINAL_OMISSION_MARKER);
  const current = alreadyOmitted
    ? elements.output.textContent.slice(TERMINAL_OMISSION_MARKER.length)
    : elements.output.textContent;
  let next = current + text;
  let omitted = alreadyOmitted;
  if (next.length > MAX_TERMINAL_OUTPUT_CHARACTERS) {
    omitted = true;
    next = next.slice(-MAX_TERMINAL_OUTPUT_CHARACTERS);
    const firstNewline = next.indexOf("\n");
    if (firstNewline >= 0 && firstNewline < 4096) {
      next = next.slice(firstNewline + 1);
    }
  }
  elements.output.textContent = `${omitted ? TERMINAL_OMISSION_MARKER : ""}${next}`;
  elements.output.scrollTop = elements.output.scrollHeight;
}

function showTerminalError(error) {
  const message = error instanceof Error ? error.message : String(error);
  appendOutput(`\n[${message}]\n`);
}

async function readResponse(response) {
  assertCurrentAccessResponse(response);
  if (response.status === 204) return null;
  const data = await response.json().catch(() => ({}));
  assertCurrentAccessResponse(response);
  if (!response.ok) throw new Error(data.error || `Request failed (${response.status})`);
  return data;
}

class StaleAccessRequest extends Error {}

function assertCurrentAccessResponse(response) {
  if (responseAccessGenerations.get(response) !== accessGeneration) {
    throw new StaleAccessRequest();
  }
}

function formatBytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  return `${(bytes / 1024).toFixed(1)} KiB`;
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}
