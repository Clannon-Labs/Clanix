const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const appSource = fs.readFileSync(path.join(__dirname, "../static/app.js"), "utf8");

function createElement(textContent = "") {
  const listeners = new Map();
  return {
    classList: { add() {}, remove() {}, toggle() {} },
    dataset: {},
    disabled: false,
    hidden: false,
    innerHTML: "",
    scrollHeight: 24,
    scrollTop: 0,
    selectionStart: 0,
    style: {},
    textContent,
    value: "",
    addEventListener(type, listener) {
      const handlers = listeners.get(type) ?? [];
      handlers.push(listener);
      listeners.set(type, handlers);
    },
    async dispatch(type, event = {}) {
      const results = (listeners.get(type) ?? []).map((listener) => listener(event));
      await Promise.all(results);
    },
    focus() {},
  };
}

async function openTerminal(storedTheme = null, storageFails = false, snapshotOverride = null, options = {}) {
  const elements = new Map();
  const element = (selector, text = "") => {
    if (!elements.has(selector)) elements.set(selector, createElement(text));
    return elements.get(selector);
  };

  const form = element("#terminal-form");
  const command = element("#command");
  const output = element("#terminal-output", "Create an environment to open the shell.");
  const run = element("#terminal-form button[type='submit']");
  const storedValues = new Map();
  const fetches = [];
  const decoders = [];
  if (storedTheme !== null) storedValues.set("clannon-color-theme", storedTheme);
  form.requestSubmit = () => {
    void form.dispatch("submit", { preventDefault() {} });
  };

  const sockets = [];
  class MockWebSocket {
    static CONNECTING = 0;
    static OPEN = 1;
    static CLOSED = 3;

    constructor(url, protocols) {
      this.url = url;
      this.requestedProtocols = protocols;
      this.protocol = options.negotiatedProtocol ?? protocols;
      this.binaryType = "blob";
      this.listeners = new Map();
      this.readyState = MockWebSocket.CONNECTING;
      this.sent = [];
      this.closeCalls = [];
      this.throwOnSend = false;
      sockets.push(this);
    }

    addEventListener(type, listener) {
      this.listeners.set(type, listener);
    }

    open() {
      this.readyState = MockWebSocket.OPEN;
      this.listeners.get("open")?.();
    }

    send(value) {
      if (this.throwOnSend) throw new Error("send failed");
      this.sent.push(value);
    }

    close(code = 1000, reason = "") {
      if (this.readyState === MockWebSocket.CLOSED) return;
      this.closeCalls.push({ code, reason });
      this.readyState = MockWebSocket.CLOSED;
      this.listeners.get("close")?.({ code, reason });
    }

    message(data) {
      this.listeners.get("message")?.({ data });
    }
  }

  class TrackingTextDecoder {
    constructor() {
      this.decoder = new TextDecoder();
      this.flushes = 0;
      this.streamingDecodes = 0;
      decoders.push(this);
    }

    decode(bytes, settings) {
      if (bytes === undefined) this.flushes += 1;
      if (settings?.stream) this.streamingDecodes += 1;
      return this.decoder.decode(bytes, settings);
    }
  }

  const emptySnapshot = snapshotOverride ?? {
    captured_at_ms: 0,
    files: [],
    network: [],
    processes: [],
    transcript: [],
    warnings: [],
  };
  const response = (status, body) => ({
    ok: status >= 200 && status < 300,
    status,
    async json() { return body; },
  });

  const context = {
    document: {
      documentElement: { dataset: {} },
      querySelector(selector) {
        return selector === "#terminal-form button[type='submit']" ? run : element(selector);
      },
    },
    fetch: async (url, requestOptions = {}) => {
      fetches.push({ url, options: requestOptions });
      if (url === "/api/environments" && requestOptions.method === "POST") {
        return response(200, { id: "env-test" });
      }
      return response(200, emptySnapshot);
    },
    location: {
      host: "localhost",
      protocol: "http:",
    },
    localStorage: {
      getItem(key) {
        if (storageFails) throw new Error("storage unavailable");
        return storedValues.get(key) ?? null;
      },
      setItem(key, value) {
        if (storageFails) throw new Error("storage unavailable");
        storedValues.set(key, value);
      },
    },
    ArrayBuffer,
    TextDecoder: TrackingTextDecoder,
    TextEncoder,
    WebSocket: MockWebSocket,
    window: {
      addEventListener() {},
      requestAnimationFrame(callback) { callback(); },
      setTimeout() {},
    },
  };

  vm.runInNewContext(appSource, context);
  await element("#create").dispatch("click");
  assert.equal(sockets.length, 1);
  sockets[0].open();
  if (options.autoReady !== false && sockets[0].readyState === MockWebSocket.OPEN) {
    sockets[0].message(JSON.stringify({ type: "ready", version: 1, resumed: false, resize: false }));
  }
  return {
    announcement: element("#terminal-announcement"),
    colorTheme: element("#color-theme"),
    command,
    decoders,
    documentElement: context.document.documentElement,
    destroy: element("#destroy"),
    fetches,
    form,
    output,
    prompt: element("#command-prompt"),
    reconnect: element("#reconnect"),
    run,
    socket: sockets[0],
    sockets,
    status: element("#terminal-status"),
    storedValues,
    transcript: element("#transcript"),
    transcriptCount: element("#transcript-count"),
  };
}

function sentInput(socket) {
  return socket.sent.slice(1).map((value) => JSON.parse(value).data);
}

function bytes(text) {
  return new TextEncoder().encode(text).buffer;
}

function enterEvent(overrides = {}) {
  return {
    isComposing: false,
    key: "Enter",
    shiftKey: false,
    prevented: false,
    preventDefault() { this.prevented = true; },
    ...overrides,
  };
}

test("negotiates v1 and waits for ready before enabling Run", async () => {
  const terminal = await openTerminal(null, false, null, { autoReady: false });

  assert.equal(terminal.socket.requestedProtocols, "clannon.terminal.v1");
  assert.equal(terminal.socket.binaryType, "arraybuffer");
  assert.deepEqual(JSON.parse(terminal.socket.sent[0]), {
    type: "open",
    version: 1,
    columns: 80,
    rows: 24,
  });
  assert.equal(terminal.status.textContent, "Connecting");
  assert.equal(terminal.command.disabled, false);
  assert.equal(terminal.run.disabled, true);

  terminal.socket.message(JSON.stringify({ type: "ready", version: 1, resumed: false, resize: false }));

  assert.equal(terminal.status.textContent, "Ready");
  assert.equal(terminal.run.disabled, false);
  assert.equal(terminal.output.textContent, "Clannon environment ready.\n");
  assert.match(terminal.announcement.textContent, /fresh shell/i);
});

test("enforces the 64 KiB UTF-8 input limit without losing the draft", async () => {
  const accepted = await openTerminal();
  accepted.command.value = `${"é".repeat(32_767)}x`;
  await accepted.form.dispatch("submit", { preventDefault() {} });

  assert.equal(new TextEncoder().encode(sentInput(accepted.socket)[0]).byteLength, 64 * 1024);
  assert.equal(accepted.command.value, "");

  const rejected = await openTerminal();
  rejected.command.value = "é".repeat(32_768);
  await rejected.form.dispatch("submit", { preventDefault() {} });

  assert.equal(rejected.command.value, "é".repeat(32_768));
  assert.deepEqual(sentInput(rejected.socket), []);
  assert.doesNotMatch(rejected.output.textContent, /\$ é/);
  assert.match(rejected.output.textContent, /65537 bytes; the terminal limit is 65536 bytes/);
  assert.match(rejected.announcement.textContent, /64 KiB terminal input limit.*draft was preserved/i);
});

test("keeps draft and local evidence unchanged when input send throws", async () => {
  const terminal = await openTerminal();
  terminal.command.value = "echo should-stay";
  terminal.socket.throwOnSend = true;

  await terminal.form.dispatch("submit", { preventDefault() {} });

  assert.equal(terminal.command.value, "echo should-stay");
  assert.deepEqual(sentInput(terminal.socket), []);
  assert.doesNotMatch(terminal.output.textContent, /\$ echo should-stay/);
  assert.match(terminal.output.textContent, /send failed/);
});

test("streams split UTF-8 and flushes its socket decoder exactly once", async () => {
  const terminal = await openTerminal();
  const encoded = new TextEncoder().encode("€ tail");

  terminal.socket.message(encoded.slice(0, 2).buffer);
  assert.equal(terminal.output.textContent, "Clannon environment ready.\n");
  terminal.socket.message(encoded.slice(2).buffer);
  assert.equal(terminal.output.textContent, "Clannon environment ready.\n€ tail");
  assert.equal(terminal.decoders.length, 1);
  assert.equal(terminal.decoders[0].streamingDecodes, 2);

  terminal.socket.message(JSON.stringify({ type: "exit", code: 0 }));
  assert.equal(terminal.decoders[0].flushes, 1);
  terminal.socket.close(1000, "terminal exited");
  assert.equal(terminal.decoders[0].flushes, 1);
});

test("flushes the socket decoder once when the environment is destroyed", async () => {
  const terminal = await openTerminal();
  const partial = new TextEncoder().encode("€").slice(0, 2).buffer;
  terminal.socket.message(partial);

  await terminal.destroy.dispatch("click");

  assert.equal(terminal.decoders[0].flushes, 1);
  assert.equal(terminal.socket.closeCalls.length, 1);
  assert.equal(terminal.output.textContent, "Environment destroyed. Its container and writable data are gone.");
});

test("treats text output, Blob output, malformed controls, and wrong versions as protocol errors", async () => {
  const cases = [
    (terminal) => terminal.socket.message("plain terminal output"),
    (terminal) => terminal.socket.message(new Blob(["binary blob"])),
    (terminal) => terminal.socket.message("{"),
    (terminal) => terminal.socket.message(JSON.stringify({ type: "ready", version: 2, resumed: false, resize: false })),
    (terminal) => {
      terminal.socket.message(JSON.stringify({
        type: "error",
        code: "protocol_error",
        message: "unsupported terminal control",
      }));
      terminal.socket.close(1002, "protocol error");
    },
  ];

  for (const violate of cases) {
    const terminal = await openTerminal(null, false, null, { autoReady: false });
    violate(terminal);
    assert.equal(terminal.status.textContent, "Protocol error");
    assert.equal(terminal.socket.closeCalls.at(-1).code, 1002);
    assert.equal(terminal.reconnect.hidden, true);
    assert.match(terminal.output.textContent, /Destroy this environment before reloading the page/);
    assert.match(terminal.announcement.textContent, /Destroy this environment before reloading the page/);
  }
});

test("keeps output gaps in a recoverable terminal-error state after close", async () => {
  const terminal = await openTerminal();
  terminal.command.value = "draft survives";

  terminal.socket.message(JSON.stringify({
    type: "error",
    code: "output_gap",
    message: "missed 3 output events",
  }));
  terminal.socket.close(1011, "output gap");

  assert.equal(terminal.status.textContent, "Terminal error");
  assert.equal(terminal.command.value, "draft survives");
  assert.equal(terminal.run.disabled, true);
  assert.equal(terminal.reconnect.hidden, false);
  assert.equal(terminal.reconnect.textContent, "Reconnect");
  assert.doesNotMatch(terminal.output.textContent, /terminal disconnected/);
  assert.match(terminal.announcement.textContent, /Reconnect to continue.*Transcript may contain/i);
});

test("renders a structured runtime failure as an ended shell", async () => {
  const terminal = await openTerminal();

  terminal.socket.message(JSON.stringify({
    type: "error",
    code: "runtime_error",
    message: "terminal proxy stopped",
  }));
  terminal.socket.close(1011, "terminal proxy stopped");

  assert.equal(terminal.status.textContent, "Ended");
  assert.match(terminal.output.textContent, /runtime_error: terminal proxy stopped/);
  assert.equal(terminal.reconnect.textContent, "Open new shell");
});

test("preserves draft and output across disconnect then resumes and refreshes Transcript", async () => {
  const terminal = await openTerminal();
  terminal.socket.message(bytes("visible output"));
  terminal.command.value = "draft command";
  const observationsBefore = terminal.fetches.filter(({ url }) => url.endsWith("/observations")).length;

  terminal.socket.close(1006, "network lost");

  assert.equal(terminal.command.value, "draft command");
  assert.equal(terminal.command.disabled, false);
  assert.equal(terminal.run.disabled, true);
  assert.match(terminal.output.textContent, /visible output/);
  assert.equal(terminal.reconnect.textContent, "Reconnect");

  await terminal.reconnect.dispatch("click");
  assert.equal(terminal.sockets.length, 2);
  assert.equal(terminal.status.textContent, "Reconnecting");
  assert.match(terminal.announcement.textContent, /Reconnecting to the environment shell/i);
  const resumed = terminal.sockets[1];
  resumed.open();
  assert.equal(terminal.status.textContent, "Reconnecting");
  assert.equal(terminal.run.disabled, true);
  assert.equal(resumed.sent.length, 1, "reconnect only sends the open control");
  resumed.message(JSON.stringify({ type: "ready", version: 1, resumed: true, resize: false }));

  assert.equal(terminal.command.value, "draft command");
  assert.match(terminal.output.textContent, /shell preserved/i);
  assert.match(terminal.output.textContent, /Transcript/);
  const observationsAfter = terminal.fetches.filter(({ url }) => url.endsWith("/observations")).length;
  assert.equal(observationsAfter, observationsBefore + 1);
  assert.match(terminal.announcement.textContent, /existing shell was preserved/i);
});

test("labels shell exit and explicitly opens a fresh shell", async () => {
  const terminal = await openTerminal();

  terminal.socket.message(JSON.stringify({ type: "exit", code: 7 }));
  terminal.socket.close(1000, "terminal exited");

  assert.match(terminal.output.textContent, /shell exited with code 7/);
  assert.equal(terminal.status.textContent, "Ended");
  assert.equal(terminal.reconnect.textContent, "Open new shell");

  await terminal.reconnect.dispatch("click");
  const fresh = terminal.sockets[1];
  fresh.open();
  fresh.message(JSON.stringify({ type: "ready", version: 1, resumed: false, resize: false }));

  assert.match(terminal.output.textContent, /fresh shell opened after the previous shell ended/i);
  assert.equal(terminal.status.textContent, "Ready");
});

test("renders a prompt for every submitted command", async () => {
  const terminal = await openTerminal();

  for (const command of ["echo one", "echo two"]) {
    terminal.command.value = command;
    terminal.command.selectionStart = command.length;
    const event = enterEvent();
    await terminal.command.dispatch("keydown", event);
    assert.equal(event.prevented, true);
  }

  assert.deepEqual(sentInput(terminal.socket), ["echo one\n", "echo two\n"]);
  assert.equal(terminal.output.textContent, "Clannon environment ready.\n$ echo one\n$ echo two\n");
});

test("waits for and renders a backslash continuation", async () => {
  const terminal = await openTerminal();
  terminal.command.value = "echo hey \\";
  terminal.command.selectionStart = terminal.command.value.length;

  const continuation = enterEvent();
  await terminal.command.dispatch("keydown", continuation);
  assert.equal(continuation.prevented, false);
  assert.deepEqual(sentInput(terminal.socket), []);

  terminal.command.value += "\n";
  terminal.command.selectionStart = terminal.command.value.length;
  await terminal.command.dispatch("input");
  assert.equal(terminal.prompt.textContent, ">");

  terminal.command.value += "echo something";
  terminal.command.selectionStart = terminal.command.value.length;
  const submit = enterEvent();
  await terminal.command.dispatch("keydown", submit);

  assert.equal(submit.prevented, true);
  assert.deepEqual(sentInput(terminal.socket), ["echo hey \\\necho something\n"]);
  assert.equal(terminal.output.textContent, "Clannon environment ready.\n$ echo hey \\\n> echo something\n");
  assert.equal(terminal.command.value, "");
  assert.equal(terminal.prompt.textContent, "$");
});

test("Run opens an unfinished backslash continuation", async () => {
  const terminal = await openTerminal();
  terminal.command.value = "echo hey \\";
  terminal.command.selectionStart = terminal.command.value.length;

  await terminal.form.dispatch("submit", { preventDefault() {} });

  assert.deepEqual(sentInput(terminal.socket), []);
  assert.equal(terminal.command.value, "echo hey \\\n");
  assert.equal(terminal.prompt.textContent, ">");
});

test("renders Shift+Enter command lines as independent prompts", async () => {
  const terminal = await openTerminal();
  terminal.command.value = "echo one";
  terminal.command.selectionStart = terminal.command.value.length;

  const newline = enterEvent({ shiftKey: true });
  await terminal.command.dispatch("keydown", newline);
  assert.equal(newline.prevented, false);

  terminal.command.value += "\n";
  terminal.command.selectionStart = terminal.command.value.length;
  await terminal.command.dispatch("input");
  assert.equal(terminal.prompt.textContent, "$");

  terminal.command.value += "echo two";
  terminal.command.selectionStart = terminal.command.value.length;
  await terminal.command.dispatch("keydown", enterEvent());

  assert.deepEqual(sentInput(terminal.socket), ["echo one\necho two\n"]);
  assert.equal(terminal.output.textContent, "Clannon environment ready.\n$ echo one\n$ echo two\n");
});

test("applies and persists validated color themes", async () => {
  const terminal = await openTerminal("dark");
  assert.equal(terminal.colorTheme.value, "dark");
  assert.equal(terminal.documentElement.dataset.theme, "dark");

  terminal.colorTheme.value = "light";
  await terminal.colorTheme.dispatch("change");
  assert.equal(terminal.documentElement.dataset.theme, "light");
  assert.equal(terminal.storedValues.get("clannon-color-theme"), "light");

  terminal.colorTheme.value = "system";
  await terminal.colorTheme.dispatch("change");
  assert.equal(terminal.documentElement.dataset.theme, undefined);
  assert.equal(terminal.storedValues.get("clannon-color-theme"), "system");

  const invalid = await openTerminal("sepia");
  assert.equal(invalid.colorTheme.value, "system");
  assert.equal(invalid.documentElement.dataset.theme, undefined);

  const unavailable = await openTerminal(null, true);
  unavailable.colorTheme.value = "dark";
  await unavailable.colorTheme.dispatch("change");
  assert.equal(unavailable.documentElement.dataset.theme, "dark");
});

test("boots a stored explicit theme before the stylesheet", () => {
  const html = fs.readFileSync(path.join(__dirname, "../static/index.html"), "utf8");
  const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1];
  assert.ok(script);
  const bootstrap = html.indexOf(script);
  const stylesheet = html.indexOf("/styles.css");
  assert.ok(bootstrap < stylesheet);

  const document = { documentElement: { dataset: {} } };
  vm.runInNewContext(script, {
    document,
    localStorage: { getItem() { return "dark"; } },
  });
  assert.equal(document.documentElement.dataset.theme, "dark");
});

test("keeps the system and native theme-control contracts", () => {
  const html = fs.readFileSync(path.join(__dirname, "../static/index.html"), "utf8");
  const styles = fs.readFileSync(path.join(__dirname, "../static/styles.css"), "utf8");

  assert.match(html, /<label for="color-theme"[^>]*>Color theme<\/label>/);
  for (const theme of ["system", "light", "dark"]) {
    assert.match(html, new RegExp(`<option value="${theme}">`, "i"));
  }
  assert.match(styles, /@media \(prefers-color-scheme: dark\) {[\s\S]*:root:not\(\[data-theme\]\)/);
});

test("renders timestamped transcript evidence without trusting its HTML", async () => {
  const transcript = Array.from({ length: 102 }, (_, index) => ({
    timestamp_ms: 1_700_000_000_000 + index,
    direction: index === 101 ? "input" : "output",
    data: index === 101 ? "printf '<script>&\\n'\n" : `output ${index}\n`,
  }));
  const terminal = await openTerminal(null, false, {
    captured_at_ms: 1_700_000_001_000,
    files: [],
    network: [],
    processes: [],
    transcript,
    warnings: [],
  });

  assert.equal(terminal.transcriptCount.textContent, 102);
  assert.match(terminal.transcript.innerHTML, /Showing the newest 100 of 102 events/);
  assert.equal((terminal.transcript.innerHTML.match(/class="evidence-row transcript-row"/g) ?? []).length, 100);
  assert.match(terminal.transcript.innerHTML, /<time datetime="2023-/);
  assert.match(terminal.transcript.innerHTML, /Command<\/span>/);
  assert.match(terminal.transcript.innerHTML, /&lt;script&gt;&amp;\\n/);
  assert.ok(terminal.transcript.innerHTML.includes("printf &#039;&lt;script&gt;&amp;\\n&#039;\n</code>"));
  assert.doesNotMatch(terminal.transcript.innerHTML, /<script>/);
});

test("bounds live terminal output while preserving its newest tail", async () => {
  const terminal = await openTerminal();
  terminal.socket.message(bytes(`${"old ".repeat(60_000)}\nnewest-tail`));
  terminal.socket.message(bytes("-still-visible"));

  assert.match(terminal.output.textContent, /^\[earlier terminal output omitted\]\n/);
  assert.match(terminal.output.textContent, /newest-tail-still-visible$/);
  assert.equal((terminal.output.textContent.match(/earlier terminal output omitted/g) ?? []).length, 1);
  assert.ok(terminal.output.textContent.length < 201 * 1024);
});
