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
  stateDot: document.querySelector("#state-dot"),
  stateLabel: document.querySelector("#state-label"),
  warnings: document.querySelector("#warnings"),
  snapshotTime: document.querySelector("#snapshot-time"),
};

const MAX_TERMINAL_OUTPUT_CHARACTERS = 200 * 1024;
const TERMINAL_OMISSION_MARKER = "[earlier terminal output omitted]\n";

let environmentId = null;
let socket = null;
let terminalState = "disconnected";
let destroying = false;

setThemePreference(readThemePreference());
elements.create.addEventListener("click", createEnvironment);
elements.colorTheme.addEventListener("change", () => setThemePreference(elements.colorTheme.value, true));
elements.destroy.addEventListener("click", destroyEnvironment);
elements.refresh.addEventListener("click", refreshObservations);
elements.form.addEventListener("submit", runCommand);
elements.command.addEventListener("keydown", handleCommandKeydown);
elements.command.addEventListener("input", updateCommandComposer);
elements.reconnect.addEventListener("click", () => connectTerminal(false));
window.addEventListener("beforeunload", () => socket?.close());

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
  setBusy(true, "Starting isolated Linux…");
  elements.output.textContent = "Creating a rootless container. The first run may pull a small image…\n";
  try {
    const response = await fetch("/api/environments", { method: "POST" });
    const data = await readResponse(response);
    environmentId = data.id;
    elements.environmentId.textContent = environmentId;
    elements.stateLabel.textContent = "Environment running";
    elements.stateDot.classList.add("running");
    updateEnvironmentControls();
    connectTerminal(true);
    await refreshObservations();
  } catch (error) {
    showTerminalError(error);
    resetEnvironment();
  } finally {
    setBusy(false);
  }
}

function connectTerminal(clearOutput) {
  if (!environmentId || socket?.readyState === WebSocket.CONNECTING || socket?.readyState === WebSocket.OPEN) return;

  const id = environmentId;
  const protocol = location.protocol === "https:" ? "wss" : "ws";
  const terminalSocket = new WebSocket(`${protocol}://${location.host}/api/environments/${id}/terminal`);
  socket = terminalSocket;
  setTerminalState("connecting");

  terminalSocket.addEventListener("open", () => {
    if (!isCurrentSocket(terminalSocket, id)) return;
    if (clearOutput) {
      elements.output.textContent = "";
    } else {
      appendOutput("\n[terminal reconnected]\n");
    }
    setTerminalState("ready");
    elements.command.focus();
  });
  terminalSocket.addEventListener("message", (event) => {
    if (!isCurrentSocket(terminalSocket, id)) return;
    appendOutput(event.data);
  });
  terminalSocket.addEventListener("close", () => {
    if (!isCurrentSocket(terminalSocket, id)) return;
    socket = null;
    if (environmentId) {
      appendOutput("\n[terminal disconnected — the environment may still be inspected]\n");
      setTerminalState("disconnected");
    }
  });
  terminalSocket.addEventListener("error", () => {
    if (!isCurrentSocket(terminalSocket, id)) return;
    showTerminalError("Could not connect the browser terminal.");
  });
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
  if (socket?.readyState !== WebSocket.OPEN) {
    showTerminalError("Terminal is not ready. Reconnect before running commands.");
    updateTerminalControls();
    return;
  }
  appendSubmittedCommand(command);
  socket.send(`${command}\n`);
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
  if (!environmentId) return;
  elements.refresh.disabled = true;
  try {
    const response = await fetch(`/api/environments/${environmentId}/observations`);
    const snapshot = await readResponse(response);
    renderTranscript(snapshot.transcript);
    renderProcesses(snapshot.processes);
    renderFiles(snapshot.files);
    renderNetwork(snapshot.network);
    renderWarnings(snapshot.warnings);
    elements.snapshotTime.textContent = `Captured ${new Date(Number(snapshot.captured_at_ms)).toLocaleTimeString()}`;
  } catch (error) {
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
  if (!environmentId) return;
  const id = environmentId;
  destroying = true;
  updateEnvironmentControls();
  elements.stateLabel.textContent = "Destroying environment…";
  try {
    const response = await fetch(`/api/environments/${id}`, { method: "DELETE" });
    await readResponse(response);
    const destroyedSocket = socket;
    resetEnvironment();
    destroyedSocket?.close();
    elements.output.textContent = "Environment destroyed. Its container and writable data are gone.";
  } catch (error) {
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
  elements.stateLabel.textContent = "No environment";
  elements.stateDot.classList.remove("running");
  updateEnvironmentControls();
  setTerminalState("disconnected");
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
  const hasEnvironment = Boolean(environmentId);
  elements.destroy.disabled = !hasEnvironment || destroying;
  elements.refresh.disabled = !hasEnvironment || destroying;
  elements.create.disabled = hasEnvironment;
  updateTerminalControls();
}

function updateTerminalControls() {
  const ready = Boolean(environmentId) && !destroying && socket?.readyState === WebSocket.OPEN;
  elements.command.disabled = !ready;
  elements.run.disabled = !ready;
  const canReconnect = Boolean(environmentId) && !destroying && terminalState === "disconnected";
  elements.reconnect.hidden = !canReconnect;
  elements.reconnect.disabled = !canReconnect;
}

function setTerminalState(state) {
  terminalState = state;
  elements.terminalStatus.dataset.state = state;
  elements.terminalStatus.textContent = state[0].toUpperCase() + state.slice(1);
  updateTerminalControls();
}

function isCurrentSocket(candidate, id) {
  return socket === candidate && environmentId === id;
}

function setBusy(busy, label = "") {
  elements.create.disabled = busy || Boolean(environmentId);
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
  if (response.status === 204) return null;
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.error || `Request failed (${response.status})`);
  return data;
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
