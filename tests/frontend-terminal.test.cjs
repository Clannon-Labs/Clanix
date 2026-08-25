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

async function openTerminal(storedTheme = null, storageFails = false) {
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
  if (storedTheme !== null) storedValues.set("clannon-color-theme", storedTheme);
  form.requestSubmit = () => {
    void form.dispatch("submit", { preventDefault() {} });
  };

  const sockets = [];
  class MockWebSocket {
    static CONNECTING = 0;
    static OPEN = 1;

    constructor() {
      this.listeners = new Map();
      this.readyState = MockWebSocket.CONNECTING;
      this.sent = [];
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
      this.sent.push(value);
    }

    close() {
      this.readyState = 3;
      this.listeners.get("close")?.();
    }
  }

  const emptySnapshot = {
    captured_at_ms: 0,
    files: [],
    network: [],
    processes: [],
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
    fetch: async (url, options = {}) => {
      if (url === "/api/environments" && options.method === "POST") {
        return response(200, { id: "env-test" });
      }
      return response(200, emptySnapshot);
    },
    location: { host: "localhost", protocol: "http:" },
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
  return {
    colorTheme: element("#color-theme"),
    command,
    documentElement: context.document.documentElement,
    form,
    output,
    prompt: element("#command-prompt"),
    socket: sockets[0],
    storedValues,
  };
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

test("renders a prompt for every submitted command", async () => {
  const terminal = await openTerminal();

  for (const command of ["echo one", "echo two"]) {
    terminal.command.value = command;
    terminal.command.selectionStart = command.length;
    const event = enterEvent();
    await terminal.command.dispatch("keydown", event);
    assert.equal(event.prevented, true);
  }

  assert.deepEqual(terminal.socket.sent, ["echo one\n", "echo two\n"]);
  assert.equal(terminal.output.textContent, "$ echo one\n$ echo two\n");
});

test("waits for and renders a backslash continuation", async () => {
  const terminal = await openTerminal();
  terminal.command.value = "echo hey \\";
  terminal.command.selectionStart = terminal.command.value.length;

  const continuation = enterEvent();
  await terminal.command.dispatch("keydown", continuation);
  assert.equal(continuation.prevented, false);
  assert.deepEqual(terminal.socket.sent, []);

  terminal.command.value += "\n";
  terminal.command.selectionStart = terminal.command.value.length;
  await terminal.command.dispatch("input");
  assert.equal(terminal.prompt.textContent, ">");

  terminal.command.value += "echo something";
  terminal.command.selectionStart = terminal.command.value.length;
  const submit = enterEvent();
  await terminal.command.dispatch("keydown", submit);

  assert.equal(submit.prevented, true);
  assert.deepEqual(terminal.socket.sent, ["echo hey \\\necho something\n"]);
  assert.equal(terminal.output.textContent, "$ echo hey \\\n> echo something\n");
  assert.equal(terminal.command.value, "");
  assert.equal(terminal.prompt.textContent, "$");
});

test("Run opens an unfinished backslash continuation", async () => {
  const terminal = await openTerminal();
  terminal.command.value = "echo hey \\";
  terminal.command.selectionStart = terminal.command.value.length;

  await terminal.form.dispatch("submit", { preventDefault() {} });

  assert.deepEqual(terminal.socket.sent, []);
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

  assert.deepEqual(terminal.socket.sent, ["echo one\necho two\n"]);
  assert.equal(terminal.output.textContent, "$ echo one\n$ echo two\n");
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
