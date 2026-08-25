const elements = {
  create: document.querySelector("#create"),
  saveSnapshot: document.querySelector("#save-snapshot"),
  colorTheme: document.querySelector("#color-theme"),
  destroy: document.querySelector("#destroy"),
  refresh: document.querySelector("#refresh"),
  form: document.querySelector("#terminal-form"),
  command: document.querySelector("#command"),
  run: document.querySelector("#terminal-form button[type='submit']"),
  reconnect: document.querySelector("#reconnect"),
  interrupt: document.querySelector("#interrupt"),
  output: document.querySelector("#terminal-output"),
  commandPrompt: document.querySelector("#command-prompt"),
  environmentId: document.querySelector("#environment-id"),
  terminalStatus: document.querySelector("#terminal-status"),
  terminalAnnouncement: document.querySelector("#terminal-announcement"),
  stateDot: document.querySelector("#state-dot"),
  stateLabel: document.querySelector("#state-label"),
  warnings: document.querySelector("#warnings"),
  snapshotTime: document.querySelector("#snapshot-time"),
  snapshotSection: document.querySelector("#snapshot-section"),
  snapshotTitle: document.querySelector("#snapshot-title"),
  snapshotCount: document.querySelector("#snapshot-count"),
  snapshotForkHelp: document.querySelector("#snapshot-fork-help"),
  snapshotList: document.querySelector("#snapshot-list"),
  snapshotAnnouncement: document.querySelector("#snapshot-announcement"),
};

const MAX_TERMINAL_OUTPUT_CHARACTERS = 200 * 1024;
const MAX_VISIBLE_EXECUTION_EVENTS = 100;
const TERMINAL_OMISSION_MARKER = "[earlier terminal output omitted]\n";
const TERMINAL_PROTOCOL = "clannon.terminal.v1";
const TERMINAL_VERSION = 1;
const MAX_TERMINAL_INPUT_BYTES = 64 * 1024;
const MIN_TERMINAL_DIMENSION = 1;
const MAX_TERMINAL_DIMENSION = 1000;
const TERMINAL_RESIZE_DEBOUNCE_MS = 120;
const SCREEN_OUTPUT_NOTICE = "[terminal control sequences omitted — plain-text view]\n";
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
let screenOutputNoticeShown = false;
let snapshots = [];
let snapshotsLoaded = false;
let snapshotsLoading = false;
let snapshotLoadGeneration = 0;
let snapshotOperation = null;
let snapshotError = "";
let forkWelcome = "";
let creating = false;

setThemePreference(readThemePreference());
elements.create.addEventListener("click", createEnvironment);
elements.saveSnapshot.addEventListener("click", saveEnvironmentSnapshot);
elements.colorTheme.addEventListener("change", () => setThemePreference(elements.colorTheme.value, true));
elements.destroy.addEventListener("click", destroyEnvironment);
elements.refresh.addEventListener("click", refreshObservations);
elements.form.addEventListener("submit", runCommand);
elements.command.addEventListener("keydown", handleCommandKeydown);
elements.command.addEventListener("input", updateCommandComposer);
elements.reconnect.addEventListener("click", handleReconnect);
elements.interrupt.addEventListener("click", sendInterrupt);
elements.snapshotList.addEventListener("click", handleSnapshotAction);
window.addEventListener("beforeunload", () => socket?.close());
window.addEventListener("hashchange", handleAccessFragment);
updateEnvironmentControls();
if (!accessToken) {
  lockForAccess();
} else if (accessFragmentCleanupFailed) {
  elements.output.textContent = "Private URL remains in the address bar because browser storage is unavailable. Close this tab when finished.";
  announceTerminal(elements.output.textContent);
}
if (accessToken) void refreshSnapshots();

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
  clearSnapshots();
  resetEnvironment();
  elements.output.textContent = "Private access restored. Create an environment when ready.";
  void refreshSnapshots();
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
  elements.saveSnapshot.disabled = true;
  elements.destroy.disabled = true;
  elements.refresh.disabled = true;
  elements.command.disabled = true;
  elements.run.disabled = true;
  elements.interrupt.disabled = true;
  elements.reconnect.hidden = true;
  elements.reconnect.disabled = true;
  elements.stateDot.classList.remove("running");
  elements.stateLabel.textContent = "Private access required";
  elements.terminalStatus.dataset.state = "disconnected";
  elements.terminalStatus.textContent = "Locked";
  elements.output.textContent = ACCESS_INSTRUCTION;
  clearSnapshots();
  renderSnapshots();
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

async function refreshSnapshots() {
  if (!accessToken || snapshotsLoading || snapshotOperation) return;
  const loadGeneration = ++snapshotLoadGeneration;
  snapshotsLoading = true;
  snapshotError = "";
  updateEnvironmentControls();
  renderSnapshots();
  try {
    const response = await apiFetch("/api/snapshots", { cache: "no-store" });
    const data = await readResponse(response);
    if (!Array.isArray(data)) {
      throw new Error("Clannon returned an invalid snapshot list.");
    }
    snapshots = data.map(normalizeSnapshotSummary);
    snapshotsLoaded = true;
  } catch (error) {
    if (error instanceof StaleAccessRequest) return;
    if (!accessToken) {
      lockForAccess();
      return;
    }
    snapshotError = `Session snapshots could not be loaded. ${errorMessage(error)}`;
  } finally {
    if (loadGeneration === snapshotLoadGeneration) {
      snapshotsLoading = false;
      updateEnvironmentControls();
      renderSnapshots();
    }
  }
}

async function saveEnvironmentSnapshot() {
  if (!environmentId || !accessToken || snapshotsLoading || snapshotOperation) return;
  const sourceEnvironmentId = environmentId;
  const operation = { kind: "save", id: sourceEnvironmentId };
  snapshotOperation = operation;
  snapshotError = "";
  updateEnvironmentControls();
  renderSnapshots();
  announceSnapshot("Saving a session snapshot of /workspace. Background writes may race.");
  try {
    const response = await apiFetch(`/api/environments/${encodeURIComponent(sourceEnvironmentId)}/snapshots`, {
      method: "POST",
    });
    const status = response.status;
    let summary;
    try {
      summary = normalizeSnapshotSummary(await readResponse(response));
    } catch (error) {
      if (!(error instanceof StaleAccessRequest)) error.status = status;
      throw error;
    }
    snapshots = [...snapshots.filter((snapshot) => snapshot.id !== summary.id), summary];
    snapshotsLoaded = true;
    announceSnapshot("Snapshot saved for this Clannon session.");
  } catch (error) {
    if (error instanceof StaleAccessRequest) return;
    if (!accessToken) {
      lockForAccess();
      return;
    }
    snapshotError = snapshotFailureMessage("save", error);
    announceSnapshot(snapshotError);
  } finally {
    if (snapshotOperation === operation) {
      snapshotOperation = null;
      updateEnvironmentControls();
      renderSnapshots();
    }
  }
}

async function handleSnapshotAction(event) {
  const action = event.target?.dataset?.snapshotAction;
  const snapshotIndex = Number(event.target?.dataset?.snapshotIndex);
  if (!action || !Number.isSafeInteger(snapshotIndex) || snapshotIndex < 0
    || snapshotIndex >= snapshots.length || snapshotsLoading || snapshotOperation || creating || !accessToken) return;
  const snapshotId = snapshots[snapshotIndex].id;
  if (action === "fork") {
    await forkSnapshot(snapshotId);
  } else if (action === "delete") {
    await deleteSnapshot(snapshotId);
  }
}

async function forkSnapshot(snapshotId) {
  if (environmentId || snapshotOperation) return;
  const operation = { kind: "fork", id: snapshotId };
  snapshotOperation = operation;
  snapshotError = "";
  updateEnvironmentControls();
  renderSnapshots();
  announceSnapshot(`Forking snapshot ${makeControlsVisible(snapshotId)} into a fresh environment.`);
  try {
    const response = await apiFetch(`/api/snapshots/${encodeURIComponent(snapshotId)}/forks`, {
      method: "POST",
    });
    const status = response.status;
    let data;
    try {
      data = await readResponse(response);
      if (!data || typeof data.id !== "string" || !data.id) {
        throw new Error("Clannon returned an invalid forked environment.");
      }
    } catch (error) {
      if (!(error instanceof StaleAccessRequest)) error.status = status;
      throw error;
    }
    resetEnvironment();
    environmentId = data.id;
    forkWelcome = `Forked from snapshot ${makeControlsVisible(snapshotId)}. /workspace was copied; this shell and its evidence are new.\n`;
    elements.output.textContent = `${forkWelcome}Connecting a fresh shell…\n`;
    elements.environmentId.textContent = makeControlsVisible(environmentId);
    elements.stateLabel.textContent = "Environment running";
    elements.stateDot.classList.add("running");
    snapshotOperation = null;
    updateEnvironmentControls();
    renderSnapshots();
    announceSnapshot(`Forked snapshot ${makeControlsVisible(snapshotId)} into a fresh environment.`);
    connectTerminal(true);
    await refreshObservations();
  } catch (error) {
    if (error instanceof StaleAccessRequest) return;
    if (!accessToken) {
      lockForAccess();
      return;
    }
    snapshotError = snapshotFailureMessage("fork", error);
    announceSnapshot(snapshotError);
  } finally {
    if (snapshotOperation === operation) {
      snapshotOperation = null;
      updateEnvironmentControls();
      renderSnapshots();
    }
  }
}

async function deleteSnapshot(snapshotId) {
  const visibleId = makeControlsVisible(snapshotId);
  if (!window.confirm(`Delete session snapshot ${visibleId}? Existing forked environments are unaffected.`)) return;
  const operation = { kind: "delete", id: snapshotId };
  snapshotOperation = operation;
  snapshotError = "";
  updateEnvironmentControls();
  renderSnapshots();
  announceSnapshot(`Deleting snapshot ${visibleId}.`);
  try {
    const response = await apiFetch(`/api/snapshots/${encodeURIComponent(snapshotId)}`, { method: "DELETE" });
    const status = response.status;
    try {
      await readResponse(response);
    } catch (error) {
      if (!(error instanceof StaleAccessRequest)) error.status = status;
      throw error;
    }
    snapshots = snapshots.filter((snapshot) => snapshot.id !== snapshotId);
    announceSnapshot(`Snapshot ${visibleId} deleted. Existing forks are unaffected.`);
    elements.snapshotTitle.focus();
  } catch (error) {
    if (error instanceof StaleAccessRequest) return;
    if (!accessToken) {
      lockForAccess();
      return;
    }
    if (error.status === 404) {
      snapshots = snapshots.filter((snapshot) => snapshot.id !== snapshotId);
      snapshotError = "This snapshot is no longer available.";
      elements.snapshotTitle.focus();
    } else {
      snapshotError = `Snapshot could not be deleted. ${errorMessage(error)}`;
    }
    announceSnapshot(snapshotError);
  } finally {
    if (snapshotOperation === operation) {
      snapshotOperation = null;
      updateEnvironmentControls();
      renderSnapshots();
    }
  }
}

function normalizeSnapshotSummary(summary) {
  if (!summary || typeof summary !== "object"
    || typeof summary.id !== "string" || !summary.id
    || typeof summary.source_environment_id !== "string" || !summary.source_environment_id
    || !Number.isSafeInteger(summary.created_at_ms) || summary.created_at_ms < 0
    || !Number.isSafeInteger(summary.archive_bytes) || summary.archive_bytes < 0) {
    throw new Error("Clannon returned invalid snapshot details.");
  }
  return {
    id: summary.id,
    source_environment_id: summary.source_environment_id,
    created_at_ms: summary.created_at_ms,
    archive_bytes: summary.archive_bytes,
  };
}

function renderSnapshots() {
  const busy = snapshotsLoading || Boolean(snapshotOperation);
  elements.snapshotSection.setAttribute("aria-busy", String(busy));
  elements.snapshotCount.textContent = String(snapshots.length);
  elements.snapshotCount.setAttribute(
    "aria-label",
    `${snapshots.length} session ${snapshots.length === 1 ? "snapshot" : "snapshots"}`,
  );
  elements.snapshotForkHelp.hidden = !environmentId || snapshots.length === 0;

  if (!accessToken) {
    elements.snapshotList.innerHTML = "<p>Private access is required to view session snapshots.</p>";
    return;
  }

  const notice = snapshotError
    ? `<p class="snapshot-error" role="alert">${escapeHtml(makeControlsVisible(snapshotError))}</p>`
    : "";
  if (!snapshotsLoaded && snapshotsLoading) {
    elements.snapshotList.innerHTML = `${notice}<p>Loading session snapshots…</p>`;
    return;
  }
  if (!snapshotsLoaded && snapshotError) {
    elements.snapshotList.innerHTML = `${notice}<p>Snapshot availability is unknown.</p>`;
    return;
  }
  if (snapshots.length === 0) {
    const empty = environmentId
      ? "No snapshots yet. Save /workspace to fork it later."
      : "No snapshots in this Clannon session.";
    elements.snapshotList.innerHTML = `${notice}<p>${empty}</p>`;
    return;
  }

  const rows = snapshots.map((snapshot, snapshotIndex) => {
    const visibleId = makeControlsVisible(snapshot.id);
    const sourceId = makeControlsVisible(snapshot.source_environment_id);
    const timestamp = formatEvidenceTimestamp(snapshot.created_at_ms);
    const savedAt = timestamp.datetime
      ? `<time datetime="${escapeHtml(timestamp.datetime)}" title="${escapeHtml(timestamp.datetime)}">${escapeHtml(timestamp.label)}</time>`
      : "Unknown time";
    const forking = snapshotOperation?.kind === "fork" && snapshotOperation.id === snapshot.id;
    const deleting = snapshotOperation?.kind === "delete" && snapshotOperation.id === snapshot.id;
    const actionsDisabled = Boolean(snapshotOperation) || creating;
    const forkDisabled = actionsDisabled || Boolean(environmentId);
    return `<li class="snapshot-item">
      <div class="snapshot-item-copy">
        <code>${escapeHtml(visibleId)}</code>
        <small>From ${escapeHtml(sourceId)} · ${savedAt} · ${escapeHtml(formatBytes(snapshot.archive_bytes))}</small>
      </div>
      <div class="snapshot-item-actions">
        <button type="button" data-snapshot-action="fork" data-snapshot-index="${snapshotIndex}" aria-label="Fork snapshot ${escapeHtml(visibleId)}" aria-describedby="snapshot-scope${environmentId ? " snapshot-fork-help" : ""}"${forkDisabled ? " disabled" : ""}>${forking ? "Forking…" : "Fork"}</button>
        <button type="button" data-snapshot-action="delete" data-snapshot-index="${snapshotIndex}" aria-label="Delete snapshot ${escapeHtml(visibleId)}"${actionsDisabled ? " disabled" : ""}>${deleting ? "Deleting…" : "Delete"}</button>
      </div>
    </li>`;
  }).join("");
  elements.snapshotList.innerHTML = `${notice}<ul class="snapshot-items">${rows}</ul>`;
}

function clearSnapshots() {
  snapshotLoadGeneration += 1;
  snapshots = [];
  snapshotsLoaded = false;
  snapshotsLoading = false;
  snapshotOperation = null;
  snapshotError = "";
  elements.snapshotAnnouncement.textContent = "";
}

function announceSnapshot(message) {
  elements.snapshotAnnouncement.textContent = makeControlsVisible(String(message));
}

function snapshotFailureMessage(action, error) {
  if (error.status === 404) return "This snapshot is no longer available.";
  if (action === "fork" && error.status === 409) {
    return "Clannon already has 4 environments. Destroy one before forking.";
  }
  const lead = action === "save" ? "Snapshot could not be saved." : "Environment could not be forked.";
  return `${lead} ${errorMessage(error)}`;
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

async function createEnvironment() {
  if (!accessToken) {
    lockForAccess();
    return;
  }
  if (snapshotOperation || creating) return;
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
    renderSnapshots();
    connectTerminal(true);
    await refreshObservations();
  } catch (error) {
    if (error instanceof StaleAccessRequest) return;
    if (!accessToken) {
      lockForAccess();
      return;
    }
    resetEnvironment();
    showTerminalError(error);
    announceTerminal(`Environment creation failed. ${error instanceof Error ? error.message : String(error)}`);
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
  const projector = createPlainTextPtyProjector();
  let decoderFlushed = false;
  let resizeObserver = null;
  let resizeTimer = null;
  let lastDimensions = null;
  const reconnecting = !clearOutput;
  setTerminalState(
    reconnecting ? "reconnecting" : "connecting",
    `${reconnecting ? "Reconnecting" : "Connecting"} to the environment shell. Run remains unavailable until the shell is ready.`,
  );

  const flushDecoder = (appendTail = true) => {
    if (decoderFlushed) return;
    decoderFlushed = true;
    const tail = decoder.decode();
    if (appendTail) {
      displayProjection(projector.write(tail));
      displayProjection(projector.flush());
    } else {
      projector.discard();
    }
  };

  const stopResizeObserver = () => {
    const observer = resizeObserver;
    resizeObserver = null;
    if (resizeTimer !== null) window.clearTimeout(resizeTimer);
    resizeTimer = null;
    observer?.disconnect();
  };

  const sendMeasuredResize = () => {
    if (!resizeObserver || !isCurrentSocket(terminalSocket, id)
      || terminalState !== "ready" || terminalSocket.readyState !== WebSocket.OPEN) return;
    const dimensions = measureTerminalDimensions();
    if (sameDimensions(dimensions, lastDimensions)) return;
    try {
      terminalSocket.send(JSON.stringify({ type: "resize", ...dimensions }));
      lastDimensions = dimensions;
    } catch (error) {
      showTerminalError(error);
      announceTerminal("The terminal size could not be updated. Reconnect to continue.");
      terminalSocket.close();
    }
  };

  const startResizeObserver = () => {
    if (resizeObserver || typeof ResizeObserver !== "function") return;
    const observer = new ResizeObserver(() => {
      if (resizeObserver !== observer || !isCurrentSocket(terminalSocket, id)) return;
      if (resizeTimer !== null) window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(() => {
        resizeTimer = null;
        if (resizeObserver !== observer) return;
        sendMeasuredResize();
      }, TERMINAL_RESIZE_DEBOUNCE_MS);
    });
    resizeObserver = observer;
    observer.observe(elements.output);
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
      lastDimensions = measureTerminalDimensions();
      terminalSocket.send(JSON.stringify({
        type: "open",
        version: TERMINAL_VERSION,
        ...lastDimensions,
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
        startResizeObserver,
        stopResizeObserver,
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
      displayProjection(projector.write(text));
    } catch {
      protocolError("server output could not be decoded.");
    }
  });
  terminalSocket.addEventListener("close", () => {
    stopResizeObserver();
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
      || control.resize !== true) {
      connection.protocolError("received a malformed or out-of-sequence ready control.");
      return;
    }
    if (connection.clearOutput) {
      elements.output.textContent = forkWelcome || "Clannon environment ready.\n";
      forkWelcome = "";
      screenOutputNoticeShown = false;
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
    connection.startResizeObserver();
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
    connection.stopResizeObserver();
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
    connection.stopResizeObserver();
    const recovery = control.code === "protocol_error" ? ` ${PROTOCOL_ERROR_RECOVERY}` : "";
    showTerminalError(`${control.code}: ${control.message}${recovery}`);
    if (control.code === "protocol_error") {
      setTerminalState("protocol-error", `Terminal protocol error. ${control.message} ${PROTOCOL_ERROR_RECOVERY}`);
    } else if (control.code === "output_gap") {
      setTerminalState("terminal-error", `Some live terminal output was missed. ${control.message} Reconnect to continue; Transcript may contain the missed output.`);
    } else {
      setTerminalState("terminal-error", `The terminal reported a runtime error. ${control.message} Reconnect to continue; the existing shell may still be available.`);
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

async function refreshObservations() {
  if (!environmentId || !accessToken) return;
  elements.refresh.disabled = true;
  try {
    const response = await apiFetch(`/api/environments/${environmentId}/observations`);
    const snapshot = await readResponse(response);
    renderExecutionEvents(snapshot.execution_events, snapshot.execution_events_omitted);
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

function renderExecutionEvents(executionEvents, runtimeOmitted) {
  const container = document.querySelector("#execution-events");
  const visible = executionEvents.slice(-MAX_VISIBLE_EXECUTION_EVENTS);
  const browserOmitted = executionEvents.length - visible.length;
  const omittedByRuntime = Number.isSafeInteger(runtimeOmitted) && runtimeOmitted > 0
    ? runtimeOmitted
    : 0;
  document.querySelector("#execution-count").textContent = executionEvents.length;
  container.classList.toggle("empty", executionEvents.length === 0);

  const omissionNotices = [];
  if (omittedByRuntime > 0) {
    omissionNotices.push(
      `<p class="execution-omission">Runtime omitted ${omittedByRuntime} earlier ${omittedByRuntime === 1 ? "event" : "events"} from this retained tail.</p>`,
    );
  }
  if (browserOmitted > 0) {
    omissionNotices.push(
      `<p class="execution-omission">Showing the newest ${visible.length} of ${executionEvents.length} retained events.</p>`,
    );
  }

  if (executionEvents.length === 0) {
    container.innerHTML = `${omissionNotices.join("")}Nothing recorded in this snapshot.`;
    return;
  }

  const rows = visible.map((event) => {
    const presentation = executionEventPresentation(event);
    const detail = presentation.detail
      ? `<small>${escapeHtml(makeControlsVisible(presentation.detail))}</small>`
      : "";
    return `
      <li class="evidence-row execution-row" data-outcome="${presentation.outcome}">
        <span>${renderEvidenceTimestamp(event.timestamp_ms)} · ${escapeHtml(formatExecutionSequence(event.sequence))}</span>
        <div>
          <code>${escapeHtml(presentation.label)}</code>
          ${detail}
        </div>
      </li>`;
  }).join("");
  container.innerHTML = `${omissionNotices.join("")}<ol class="execution-list" aria-label="Execution events, oldest to newest">${rows}</ol>`;
}

function executionEventPresentation(event) {
  const generation = `generation ${executionValue(event.generation)}`;
  const dimensions = `${executionValue(event.columns)} × ${executionValue(event.rows)}`;
  switch (event.type) {
    case "environment_ready":
      return { label: "Environment ready", detail: "", outcome: "normal" };
    case "shell_started":
      return { label: "Shell started", detail: `${generation} · ${dimensions}`, outcome: "normal" };
    case "terminal_input": {
      const inputKind = executionValue(event.input_kind);
      const bytes = formatExecutionBytes(event.bytes);
      if (event.input_kind === "interrupt") {
        return { label: "Interrupt sent", detail: `${generation} · ${bytes}`, outcome: "normal" };
      }
      return { label: "Input accepted", detail: `${generation} · ${inputKind} · ${bytes}`, outcome: "normal" };
    }
    case "terminal_resized":
      return { label: "Terminal resized", detail: `${generation} · ${dimensions}`, outcome: "normal" };
    case "shell_exited": {
      const exit = event.code === null ? "exit code unavailable" : `code ${executionValue(event.code)}`;
      return { label: "Shell exited", detail: `${generation} · ${exit}`, outcome: "normal" };
    }
    case "shell_failed":
      return { label: "Shell failed", detail: generation, outcome: "failed" };
    case "process_added":
      return sampledExecutionPresentation(
        "Process appeared",
        event,
        processSubject(event.process),
        processFacts(event.process),
      );
    case "process_removed":
      return sampledExecutionPresentation(
        "Process disappeared",
        event,
        processSubject(event.process),
        processFacts(event.process),
      );
    case "process_changed":
      return sampledExecutionPresentation(
        "Process changed",
        event,
        `pid ${executionValue(event.current?.pid ?? event.previous?.pid)}`,
        changedObservationFacts(event.previous, event.current, [
          ["pid", "pid", executionValue],
          ["parent_pid", "parent", executionValue],
          ["state", "state", executionValue],
          ["command", "command", executionValue],
          ["arguments", "arguments", formatObservedArguments],
        ]),
      );
    case "file_added":
      return sampledExecutionPresentation(
        "File appeared",
        event,
        fileSubject(event.file),
        fileFacts(event.file),
      );
    case "file_removed":
      return sampledExecutionPresentation(
        "File disappeared",
        event,
        fileSubject(event.file),
        fileFacts(event.file),
      );
    case "file_changed":
      return sampledExecutionPresentation(
        "File changed",
        event,
        fileSubject(event.current ?? event.previous),
        changedObservationFacts(event.previous, event.current, [
          ["path", "path", formatObservedPath],
          ["size_bytes", "size", formatObservedBytes],
          ["modified_unix_seconds", "modified", formatObservedUnixSeconds],
          ["kind", "kind", executionValue],
        ]),
      );
    case "network_added":
      return sampledExecutionPresentation(
        "Socket appeared",
        event,
        socketSubject(event.network),
        socketFacts(event.network),
      );
    case "network_removed":
      return sampledExecutionPresentation(
        "Socket disappeared",
        event,
        socketSubject(event.network),
        socketFacts(event.network),
      );
    default:
      return {
        label: "Unknown event",
        detail: `type ${executionValue(event.type, "missing")}`,
        outcome: "unknown",
      };
  }
}

function sampledExecutionPresentation(label, event, subject, facts) {
  return {
    label,
    detail: `${formatCaptureSequence(event.capture_sequence)} · ${subject}${facts ? ` · ${facts}` : ""}`,
    outcome: "normal",
  };
}

function formatCaptureSequence(captureSequence) {
  return Number.isSafeInteger(captureSequence) && captureSequence >= 1
    ? `sample #${captureSequence}`
    : "sample #?";
}

function processSubject(process = {}) {
  return `pid ${executionValue(process.pid)} · ${executionValue(process.command)}`;
}

function processFacts(process = {}) {
  return `parent ${executionValue(process.parent_pid)} · state ${executionValue(process.state)} · arguments ${formatObservedArguments(process.arguments)}`;
}

function fileSubject(file = {}) {
  return formatObservedPath(file.path);
}

function fileFacts(file = {}) {
  return `${formatObservedBytes(file.size_bytes)} · ${executionValue(file.kind)} · modified ${formatObservedUnixSeconds(file.modified_unix_seconds)}`;
}

function socketSubject(socket = {}) {
  return `${executionValue(socket.protocol)} · ${executionValue(socket.local_address)}`;
}

function socketFacts(socket = {}) {
  return `remote ${executionValue(socket.remote_address)} · ${executionValue(socket.state)}`;
}

function changedObservationFacts(previous = {}, current = {}, fields) {
  const changes = fields.flatMap(([key, label, formatter]) => previous?.[key] === current?.[key]
    ? []
    : [`${label} ${formatter(previous?.[key])} → ${formatter(current?.[key])}`]);
  return changes.length ? changes.join(" · ") : "sampled fields unchanged";
}

function formatObservedArguments(argumentsValue) {
  return argumentsValue === "" ? "—" : executionValue(argumentsValue);
}

function formatObservedPath(path) {
  const value = executionValue(path);
  return value.startsWith("/workspace/") ? value.slice("/workspace/".length) : value;
}

function formatObservedBytes(bytes) {
  return Number.isSafeInteger(bytes) && bytes >= 0 ? formatBytes(bytes) : "unknown size";
}

function formatObservedUnixSeconds(seconds) {
  if (!Number.isSafeInteger(seconds) || seconds < 0) return "unknown time";
  const timestamp = new Date(seconds * 1000);
  return Number.isNaN(timestamp.getTime()) ? "unknown time" : timestamp.toISOString();
}

function executionValue(value, fallback = "unknown") {
  return makeControlsVisible(String(value ?? fallback));
}

function formatExecutionSequence(sequence) {
  return Number.isSafeInteger(sequence) && sequence >= 0 ? `#${sequence}` : "#?";
}

function formatExecutionBytes(bytes) {
  if (!Number.isSafeInteger(bytes) || bytes < 0) return "unknown byte count";
  return `${bytes} ${bytes === 1 ? "byte" : "bytes"}`;
}

function formatEvidenceTimestamp(timestampMs) {
  const recordedAt = new Date(Number(timestampMs));
  if (Number.isNaN(recordedAt.getTime())) {
    return { datetime: "", label: "Unknown time" };
  }
  return {
    datetime: recordedAt.toISOString(),
    label: recordedAt.toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      fractionalSecondDigits: 3,
    }),
  };
}

function renderEvidenceTimestamp(timestampMs) {
  const timestamp = formatEvidenceTimestamp(timestampMs);
  if (!timestamp.datetime) return timestamp.label;
  return `<time datetime="${escapeHtml(timestamp.datetime)}" title="${escapeHtml(timestamp.datetime)}">${escapeHtml(timestamp.label)}</time>`;
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
    const data = entry.data === "" ? "∅" : makeControlsVisible(String(entry.data));
    return `
      <div class="evidence-row transcript-row" data-direction="${input ? "input" : "output"}">
        <span>${renderEvidenceTimestamp(entry.timestamp_ms)} · ${input ? "Input" : "Output"}</span>
        <code>${escapeHtml(data)}</code>
      </div>`;
  }).join("");
}

async function destroyEnvironment() {
  if (!environmentId || !accessToken || snapshotOperation) return;
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
  forkWelcome = "";
  screenOutputNoticeShown = false;
  elements.command.value = "";
  updateCommandComposer();
  elements.environmentId.textContent = "waiting for environment";
  elements.stateLabel.textContent = accessToken ? "No environment" : "Private access required";
  elements.stateDot.classList.remove("running");
  updateEnvironmentControls();
  setTerminalState("disconnected", "No environment is active.");
  ["execution-events", "transcript", "processes", "files", "network"].forEach((id) => {
    const container = document.querySelector(`#${id}`);
    container.classList.add("empty");
    container.textContent = id === "execution-events" ? "No execution events yet." : "No snapshot yet.";
  });
  ["execution-count", "transcript-count", "process-count", "file-count", "network-count"].forEach((id) => {
    document.querySelector(`#${id}`).textContent = "0";
  });
  elements.snapshotTime.textContent = "Evidence appears after the environment starts.";
  elements.warnings.hidden = true;
  renderSnapshots();
}

function updateEnvironmentControls() {
  const hasAccess = Boolean(accessToken);
  const hasEnvironment = Boolean(environmentId);
  const snapshotMutating = Boolean(snapshotOperation);
  elements.destroy.disabled = !hasAccess || !hasEnvironment || destroying || snapshotMutating || creating;
  elements.saveSnapshot.disabled = !hasAccess || !hasEnvironment || destroying || snapshotsLoading || snapshotMutating || creating;
  elements.saveSnapshot.textContent = snapshotOperation?.kind === "save" ? "Saving snapshot…" : "Save snapshot";
  elements.refresh.disabled = !hasAccess || !hasEnvironment || destroying;
  elements.create.disabled = !hasAccess || hasEnvironment || snapshotMutating || creating;
  updateTerminalControls();
}

function updateTerminalControls() {
  const hasAccess = Boolean(accessToken);
  const hasEnvironment = hasAccess && Boolean(environmentId) && !destroying;
  const ready = hasEnvironment && terminalState === "ready" && socket?.readyState === WebSocket.OPEN;
  elements.command.disabled = !hasEnvironment;
  elements.run.disabled = !ready;
  elements.interrupt.disabled = !ready;
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
  creating = busy;
  updateEnvironmentControls();
  elements.create.textContent = busy ? "Creating…" : "Create environment";
  if (label) elements.stateLabel.textContent = label;
  renderSnapshots();
}

function appendOutput(text) {
  const previousScrollTop = elements.output.scrollTop;
  const wasAtBottom = elements.output.scrollHeight - previousScrollTop - elements.output.clientHeight <= 2;
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
  elements.output.scrollTop = wasAtBottom ? elements.output.scrollHeight : previousScrollTop;
}

function sendInterrupt() {
  if (terminalState !== "ready" || socket?.readyState !== WebSocket.OPEN) {
    updateTerminalControls();
    return;
  }
  try {
    socket.send(Uint8Array.of(0x03));
    announceTerminal("Ctrl-C sent.");
  } catch (error) {
    showTerminalError(error);
    announceTerminal("Ctrl-C was not sent. Your draft was preserved.");
  } finally {
    elements.command.focus();
  }
}

function measureTerminalDimensions() {
  const style = window.getComputedStyle(elements.output);
  const horizontalPadding = numericPixels(style.paddingLeft) + numericPixels(style.paddingRight);
  const verticalPadding = numericPixels(style.paddingTop) + numericPixels(style.paddingBottom);
  const contentWidth = Math.max(0, elements.output.clientWidth - horizontalPadding);
  const contentHeight = Math.max(0, elements.output.clientHeight - verticalPadding);
  const fontSize = numericPixels(style.fontSize) || 16;
  const lineHeight = numericPixels(style.lineHeight) || fontSize * 1.2;
  const canvas = document.createElement("canvas");
  const context = canvas.getContext("2d");
  if (context) context.font = style.font || `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
  const measuredWidth = context?.measureText("0000000000").width / 10;
  const cellWidth = Number.isFinite(measuredWidth) && measuredWidth > 0 ? measuredWidth : fontSize * 0.6;
  return {
    columns: clampTerminalDimension(Math.floor(contentWidth / cellWidth)),
    rows: clampTerminalDimension(Math.floor(contentHeight / lineHeight)),
  };
}

function numericPixels(value) {
  const parsed = Number.parseFloat(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function clampTerminalDimension(value) {
  return Math.min(MAX_TERMINAL_DIMENSION, Math.max(MIN_TERMINAL_DIMENSION, value));
}

function sameDimensions(left, right) {
  return Boolean(left && right && left.columns === right.columns && left.rows === right.rows);
}

function displayProjection(projection) {
  if (projection.text) appendOutput(projection.text);
  if (projection.screenOriented && !screenOutputNoticeShown) {
    const leadingNewline = elements.output.textContent && !elements.output.textContent.endsWith("\n") ? "\n" : "";
    appendOutput(`${leadingNewline}${SCREEN_OUTPUT_NOTICE}`);
    screenOutputNoticeShown = true;
  }
}

function createPlainTextPtyProjector() {
  let state = "text";

  const consume = (text, flushing = false) => {
    let output = "";
    let screenOriented = false;
    const markScreenControl = () => { screenOriented = true; };

    for (const character of text) {
      let reprocess = true;
      while (reprocess) {
        reprocess = false;
        const code = character.codePointAt(0);
        if (state === "carriage-return") {
          output += "\n";
          state = "text";
          if (character === "\n") break;
          reprocess = true;
        } else if (state === "escape") {
          if (character === "[") {
            state = "csi";
            markScreenControl();
          } else if (character === "]") {
            state = "osc";
            markScreenControl();
          } else if (code >= 0x20 && code <= 0x2f) {
            state = "escape-intermediate";
          } else if (code >= 0x30 && code <= 0x7e) {
            state = "text";
            markScreenControl();
          } else {
            output += "␛";
            state = "text";
            reprocess = true;
          }
        } else if (state === "escape-intermediate") {
          if (code >= 0x20 && code <= 0x2f) {
            continue;
          } else if (code >= 0x30 && code <= 0x7e) {
            state = "text";
            markScreenControl();
          } else {
            output += "␛[unsupported ESC control]";
            state = "text";
            reprocess = true;
          }
        } else if (state === "csi") {
          if (code >= 0x40 && code <= 0x7e) {
            state = "text";
          } else if (!(code >= 0x20 && code <= 0x3f)) {
            output += "[unsupported CSI control]";
            state = "text";
            reprocess = true;
          }
        } else if (state === "osc") {
          if (character === "\u0007" || character === "\u009c") {
            state = "text";
          } else if (character === "\u001b") {
            state = "osc-escape";
          }
        } else if (state === "osc-escape") {
          if (character === "\\") {
            state = "text";
          } else {
            state = character === "\u001b" ? "osc-escape" : "osc";
          }
        } else if (character === "\r") {
          state = "carriage-return";
        } else if (character === "\u001b") {
          state = "escape";
        } else if (character === "\u009b") {
          state = "csi";
          markScreenControl();
        } else if (character === "\u009d") {
          state = "osc";
          markScreenControl();
        } else if (isBidiFormattingControl(code)) {
          output += visibleControlCharacter(code);
        } else if (character === "\n" || character === "\t" || code >= 0x20 && code !== 0x7f && !(code >= 0x80 && code <= 0x9f)) {
          output += character;
        } else {
          output += visibleControlCharacter(code);
        }
      }
    }

    if (flushing) {
      if (state === "carriage-return") output += "\n";
      if (state === "escape" || state === "escape-intermediate") output += "␛";
      if (state === "csi") output += "[incomplete CSI control]";
      if (state === "osc" || state === "osc-escape") output += "[incomplete OSC control]";
      state = "text";
    }
    return { text: output, screenOriented };
  };

  return {
    write(text) { return consume(text); },
    flush() { return consume("", true); },
    discard() { state = "text"; },
  };
}

function makeControlsVisible(value) {
  let visible = "";
  for (const character of value) {
    const code = character.codePointAt(0);
    visible += character === "\n" ? character
      : code < 0x20 || code === 0x7f || code >= 0x80 && code <= 0x9f || isBidiFormattingControl(code)
        ? visibleControlCharacter(code)
        : character;
  }
  return visible;
}

function isBidiFormattingControl(code) {
  return code === 0x061c
    || code === 0x200e
    || code === 0x200f
    || code >= 0x202a && code <= 0x202e
    || code >= 0x2066 && code <= 0x2069;
}

function visibleControlCharacter(code) {
  if (code >= 0 && code <= 0x1f) return String.fromCodePoint(0x2400 + code);
  if (code === 0x7f) return "␡";
  return `[U+${code.toString(16).toUpperCase().padStart(4, "0")}]`;
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
