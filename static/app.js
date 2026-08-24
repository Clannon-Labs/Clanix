const elements = {
  create: document.querySelector("#create"),
  destroy: document.querySelector("#destroy"),
  refresh: document.querySelector("#refresh"),
  form: document.querySelector("#terminal-form"),
  command: document.querySelector("#command"),
  run: document.querySelector("#terminal-form button[type='submit']"),
  reconnect: document.querySelector("#reconnect"),
  output: document.querySelector("#terminal-output"),
  environmentId: document.querySelector("#environment-id"),
  terminalStatus: document.querySelector("#terminal-status"),
  stateDot: document.querySelector("#state-dot"),
  stateLabel: document.querySelector("#state-label"),
  warnings: document.querySelector("#warnings"),
  snapshotTime: document.querySelector("#snapshot-time"),
};

let environmentId = null;
let socket = null;
let terminalState = "disconnected";
let destroying = false;

elements.create.addEventListener("click", createEnvironment);
elements.destroy.addEventListener("click", destroyEnvironment);
elements.refresh.addEventListener("click", refreshObservations);
elements.form.addEventListener("submit", runCommand);
elements.reconnect.addEventListener("click", () => connectTerminal(false));
window.addEventListener("beforeunload", () => socket?.close());

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
  if (socket?.readyState !== WebSocket.OPEN) {
    showTerminalError("Terminal is not ready. Reconnect before running commands.");
    updateTerminalControls();
    return;
  }
  appendOutput(`${command}\n`);
  socket.send(`${command}\n`);
  elements.command.value = "";
  window.setTimeout(refreshObservations, 350);
}

async function refreshObservations() {
  if (!environmentId) return;
  elements.refresh.disabled = true;
  try {
    const response = await fetch(`/api/environments/${environmentId}/observations`);
    const snapshot = await readResponse(response);
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
  elements.environmentId.textContent = "waiting for environment";
  elements.stateLabel.textContent = "No environment";
  elements.stateDot.classList.remove("running");
  updateEnvironmentControls();
  setTerminalState("disconnected");
  ["processes", "files", "network"].forEach((id) => {
    const container = document.querySelector(`#${id}`);
    container.classList.add("empty");
    container.textContent = "No snapshot yet.";
  });
  ["process-count", "file-count", "network-count"].forEach((id) => {
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
  elements.output.textContent += text;
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
