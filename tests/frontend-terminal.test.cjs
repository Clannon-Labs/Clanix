const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const appSource = fs.readFileSync(path.join(__dirname, "../static/app.js"), "utf8");
const ACCESS_TOKEN = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

function createElement(textContent = "") {
  const listeners = new Map();
  return {
    classList: { add() {}, remove() {}, toggle() {} },
    dataset: {},
    disabled: false,
    hidden: false,
    innerHTML: "",
    clientHeight: 0,
    clientWidth: 0,
    focusCount: 0,
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
    focus() { this.focusCount += 1; },
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
  output.clientWidth = options.outputWidth ?? 816;
  output.clientHeight = options.outputHeight ?? 416;
  const run = element("#terminal-form button[type='submit']");
  const storedValues = new Map();
  const sessionValues = new Map();
  const fetches = [];
  const decoders = [];
  const historyCalls = [];
  const locationReplacements = [];
  const windowListeners = new Map();
  const animationFrames = [];
  const timers = new Map();
  const resizeObservers = [];
  let nextTimerId = 1;
  let resolveDeferredCreate;
  let resolveDeferredDestroy;
  const deferredCreate = options.deferCreate
    ? new Promise((resolve) => { resolveDeferredCreate = resolve; })
    : null;
  const deferredDestroy = options.deferDestroy
    ? new Promise((resolve) => { resolveDeferredDestroy = resolve; })
    : null;
  if (storedTheme !== null) storedValues.set("clannon-color-theme", storedTheme);
  if (options.storedToken !== undefined) {
    sessionValues.set("clannon-access-token", options.storedToken);
  }
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

  class MockResizeObserver {
    constructor(callback) {
      this.callback = callback;
      this.disconnected = false;
      this.observed = [];
      resizeObservers.push(this);
    }

    observe(target) {
      this.observed.push(target);
    }

    disconnect() {
      this.disconnected = true;
    }

    trigger() {
      this.callback([{ target: output }]);
    }
  }

  const emptySnapshot = snapshotOverride ?? {
    captured_at_ms: 0,
    execution_events: [],
    execution_events_omitted: 0,
    files: [],
    network: [],
    processes: [],
    transcript: [],
    warnings: [],
  };
  const observationResponses = [...(options.observationResponses ?? [])];
  const response = (status, body) => ({
    ok: status >= 200 && status < 300,
    status,
    async json() { return body; },
  });

  const context = {
    document: {
      documentElement: { dataset: {} },
      createElement(tagName) {
        assert.equal(tagName, "canvas");
        return {
          getContext(kind) {
            assert.equal(kind, "2d");
            return {
              font: "",
              measureText() { return { width: (options.cellWidth ?? 8) * 10 }; },
            };
          },
        };
      },
      querySelector(selector) {
        return selector === "#terminal-form button[type='submit']" ? run : element(selector);
      },
    },
    fetch: async (url, requestOptions = {}) => {
      fetches.push({ url, options: requestOptions });
      if (url === "/api/access") {
        return response(options.accessProbeStatus ?? 204, {});
      }
      if (url === "/api/environments" && requestOptions.method === "POST") {
        if (deferredCreate) return deferredCreate;
        const status = options.createStatus ?? 200;
        const body = options.createBody ?? (status === 401
          ? { error: "unauthorized" }
          : status >= 400
            ? { error: "environment capacity reached" }
            : { id: "env-test" });
        return response(status, body);
      }
      if (url.endsWith("/observations") && observationResponses.length) {
        const next = observationResponses.shift();
        return response(next.status, next.body);
      }
      if (requestOptions.method === "DELETE" && deferredDestroy) return deferredDestroy;
      return response(200, emptySnapshot);
    },
    location: {
      hash: options.hash ?? `#${ACCESS_TOKEN}`,
      host: "localhost:3000",
      pathname: "/",
      protocol: "http:",
      search: "",
      replace(url) {
        if (options.locationReplaceFails) throw new Error("location replacement unavailable");
        locationReplacements.push(url);
        this.hash = "";
      },
    },
    history: {
      state: { test: true },
      replaceState(state, title, url) {
        if (options.historyFails) throw new Error("history unavailable");
        historyCalls.push({ state, title, url });
        context.location.hash = "";
      },
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
    sessionStorage: {
      getItem(key) {
        if (options.sessionStorageFails) throw new Error("session storage unavailable");
        return sessionValues.get(key) ?? null;
      },
      setItem(key, value) {
        if (options.sessionStorageFails) throw new Error("session storage unavailable");
        sessionValues.set(key, value);
      },
      removeItem(key) {
        if (options.sessionStorageFails) throw new Error("session storage unavailable");
        sessionValues.delete(key);
      },
    },
    ArrayBuffer,
    TextDecoder: TrackingTextDecoder,
    TextEncoder,
    ResizeObserver: MockResizeObserver,
    WebSocket: MockWebSocket,
    window: {
      addEventListener(type, listener) {
        const handlers = windowListeners.get(type) ?? [];
        handlers.push(listener);
        windowListeners.set(type, handlers);
      },
      getComputedStyle() {
        return {
          font: "400 16px mock-mono",
          fontFamily: "mock-mono",
          fontSize: "16px",
          fontWeight: "400",
          lineHeight: `${options.lineHeight ?? 16}px`,
          paddingBottom: `${options.paddingBottom ?? 16}px`,
          paddingLeft: `${options.paddingLeft ?? 16}px`,
          paddingRight: `${options.paddingRight ?? 16}px`,
          paddingTop: `${options.paddingTop ?? 16}px`,
        };
      },
      requestAnimationFrame(callback) {
        animationFrames.push(callback);
        return animationFrames.length;
      },
      clearTimeout(id) {
        timers.delete(id);
      },
      setTimeout(callback) {
        const id = nextTimerId;
        nextTimerId += 1;
        timers.set(id, callback);
        return id;
      },
    },
  };

  vm.runInNewContext(appSource, context);
  if (options.create !== false) {
    await element("#create").dispatch("click");
    if ((options.createStatus ?? 200) < 400 && options.openSocket !== false) {
      assert.equal(sockets.length, 1);
      sockets[0].open();
    }
  }
  if (options.create !== false && (options.createStatus ?? 200) < 400
    && options.openSocket !== false && options.autoReady !== false
    && sockets[0].readyState === MockWebSocket.OPEN) {
    sockets[0].message(JSON.stringify({ type: "ready", version: 1, resumed: false, resize: true }));
  }
  return {
    announcement: element("#terminal-announcement"),
    colorTheme: element("#color-theme"),
    command,
    create: element("#create"),
    decoders,
    documentElement: context.document.documentElement,
    destroy: element("#destroy"),
    environmentId: element("#environment-id"),
    executionCount: element("#execution-count"),
    executionEvents: element("#execution-events"),
    fetches,
    form,
    historyCalls,
    interrupt: element("#interrupt"),
    location: context.location,
    locationReplacements,
    output,
    prompt: element("#command-prompt"),
    reconnect: element("#reconnect"),
    refresh: element("#refresh"),
    resizeObservers,
    run,
    resolveCreate(status, body = { error: "unauthorized" }) {
      resolveDeferredCreate?.(response(status, body));
    },
    resolveDestroy(status, body = { error: "destroy failed" }) {
      resolveDeferredDestroy?.(response(status, body));
    },
    socket: sockets[0],
    sockets,
    stateLabel: element("#state-label"),
    status: element("#terminal-status"),
    storedValues,
    sessionValues,
    transcript: element("#transcript"),
    transcriptCount: element("#transcript-count"),
    snapshotTime: element("#snapshot-time"),
    warnings: element("#warnings"),
    flushAnimationFrames() {
      while (animationFrames.length) animationFrames.shift()();
    },
    flushTimers() {
      while (timers.size) {
        const pending = [...timers.values()];
        timers.clear();
        for (const callback of pending) callback();
      }
    },
    setOutputSize(width, height) {
      output.clientWidth = width;
      output.clientHeight = height;
    },
    async dispatchWindow(type, event = {}) {
      const results = (windowListeners.get(type) ?? []).map((listener) => listener(event));
      await Promise.all(results);
    },
  };
}

function sentInput(socket) {
  return sentControls(socket).filter((control) => control.type === "input").map((control) => control.data);
}

function sentControls(socket) {
  return socket.sent.filter((value) => typeof value === "string").map((value) => JSON.parse(value));
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

test("moves a fragment capability into tab storage and authenticates every API and WebSocket request", async () => {
  const terminal = await openTerminal();

  assert.equal(terminal.sessionValues.get("clannon-access-token"), ACCESS_TOKEN);
  assert.deepEqual(terminal.historyCalls, [{ state: { test: true }, title: "", url: "/" }]);
  assert.equal(
    terminal.socket.url,
    `ws://localhost:3000/api/environments/env-test/terminal?access_token=${ACCESS_TOKEN}`,
  );

  await terminal.destroy.dispatch("click");
  assert.ok(terminal.fetches.length >= 3);
  for (const request of terminal.fetches) {
    assert.equal(request.options.headers.Authorization, `Bearer ${ACCESS_TOKEN}`);
  }
  assert.equal(terminal.storedValues.has("clannon-access-token"), false);
});

test("reuses a tab capability on reload and lets a fresh fragment replace a stale one", async () => {
  const reload = await openTerminal(null, false, null, {
    hash: "",
    storedToken: ACCESS_TOKEN,
  });
  assert.equal(reload.historyCalls.length, 0);
  assert.equal(reload.fetches[0].options.headers.Authorization, `Bearer ${ACCESS_TOKEN}`);

  const replacement = "f".repeat(64);
  const refreshed = await openTerminal(null, false, null, {
    hash: `#${replacement}`,
    storedToken: ACCESS_TOKEN,
  });
  assert.equal(refreshed.sessionValues.get("clannon-access-token"), replacement);
  assert.equal(refreshed.fetches[0].options.headers.Authorization, `Bearer ${replacement}`);
  assert.match(refreshed.socket.url, new RegExp(`access_token=${replacement}$`));
});

test("keeps the workbench readable and non-operational without a capability", async () => {
  const terminal = await openTerminal(null, false, null, { hash: "", create: false });

  assert.equal(terminal.sockets.length, 0);
  assert.equal(terminal.fetches.length, 0);
  assert.equal(terminal.create.disabled, true);
  assert.equal(terminal.destroy.disabled, true);
  assert.equal(terminal.refresh.disabled, true);
  assert.equal(terminal.run.disabled, true);
  assert.equal(terminal.command.disabled, true);
  assert.equal(terminal.stateLabel.textContent, "Private access required");
  assert.equal(terminal.output.textContent, "Open the private URL printed by Clannon.");
});

test("contains session-storage and history failures while preserving fragment access", async () => {
  const terminal = await openTerminal(null, false, null, {
    create: false,
    historyFails: true,
    sessionStorageFails: true,
  });

  assert.match(terminal.output.textContent, /Private URL remains in the address bar/);
  await terminal.create.dispatch("click");
  terminal.sockets[0].open();
  terminal.sockets[0].message(JSON.stringify({ type: "ready", version: 1, resumed: false, resize: true }));
  assert.equal(terminal.fetches[0].options.headers.Authorization, `Bearer ${ACCESS_TOKEN}`);
  assert.match(terminal.sockets[0].url, new RegExp(`access_token=${ACCESS_TOKEN}$`));
  assert.equal(terminal.status.textContent, "Ready");
});

test("falls back to a clean reload when history fails but tab storage works", async () => {
  const terminal = await openTerminal(null, false, null, {
    create: false,
    historyFails: true,
  });

  assert.deepEqual(terminal.locationReplacements, ["/"]);
  assert.equal(terminal.sessionValues.get("clannon-access-token"), ACCESS_TOKEN);
});

test("accepts a private URL pasted into an already-open locked tab", async () => {
  const terminal = await openTerminal(null, false, null, { hash: "", create: false });
  assert.equal(terminal.status.textContent, "Locked");

  terminal.location.hash = `#${ACCESS_TOKEN}`;
  await terminal.dispatchWindow("hashchange");

  assert.equal(terminal.sessionValues.get("clannon-access-token"), ACCESS_TOKEN);
  assert.equal(terminal.location.hash, "");
  assert.equal(terminal.status.textContent, "Disconnected");
  assert.equal(terminal.create.disabled, false);
  assert.match(terminal.output.textContent, /Private access restored/);
  const probe = terminal.fetches.find(({ url }) => url === "/api/access");
  assert.equal(probe.options.cache, "no-store");
  assert.equal(probe.options.headers.Authorization, `Bearer ${ACCESS_TOKEN}`);
});

test("rejects an unverified fragment without replacing the current access state", async () => {
  const terminal = await openTerminal(null, false, null, {
    accessProbeStatus: 401,
    hash: "",
    create: false,
  });

  terminal.location.hash = `#${ACCESS_TOKEN}`;
  await terminal.dispatchWindow("hashchange");

  assert.equal(terminal.sessionValues.has("clannon-access-token"), false);
  assert.equal(terminal.status.textContent, "Locked");
  assert.match(terminal.announcement.textContent, /private URL was rejected/i);
});

test("a late 401 from an old credential cannot clear newly verified access", async () => {
  const replacement = "f".repeat(64);
  const terminal = await openTerminal(null, false, null, {
    create: false,
    deferCreate: true,
  });

  const pendingCreate = terminal.create.dispatch("click");
  await new Promise((resolve) => setImmediate(resolve));
  terminal.location.hash = `#${replacement}`;
  await terminal.dispatchWindow("hashchange");
  terminal.resolveCreate(401);
  await pendingCreate;

  assert.equal(terminal.sessionValues.get("clannon-access-token"), replacement);
  assert.notEqual(terminal.status.textContent, "Locked");
  assert.equal(terminal.create.disabled, false);
});

test("a stale destroy failure cannot restore an environment after access recovery", async () => {
  const replacement = "e".repeat(64);
  const terminal = await openTerminal(null, false, null, { deferDestroy: true });

  const pendingDestroy = terminal.destroy.dispatch("click");
  await new Promise((resolve) => setImmediate(resolve));
  terminal.location.hash = `#${replacement}`;
  await terminal.dispatchWindow("hashchange");
  terminal.resolveDestroy(502);
  await pendingDestroy;

  assert.equal(terminal.sessionValues.get("clannon-access-token"), replacement);
  assert.equal(terminal.environmentId.textContent, "waiting for environment");
  assert.equal(terminal.stateLabel.textContent, "No environment");
  assert.equal(terminal.create.disabled, false);
  assert.equal(terminal.destroy.disabled, true);
  assert.doesNotMatch(terminal.output.textContent, /destroy failed/i);
});

test("a 401 clears tab access and leaves every operation safely disabled", async () => {
  const terminal = await openTerminal(null, false, null, { createStatus: 401 });

  assert.equal(terminal.sockets.length, 0);
  assert.equal(terminal.sessionValues.has("clannon-access-token"), false);
  assert.equal(terminal.create.disabled, true);
  assert.equal(terminal.destroy.disabled, true);
  assert.equal(terminal.refresh.disabled, true);
  assert.equal(terminal.run.disabled, true);
  assert.equal(terminal.command.disabled, true);
  assert.equal(terminal.status.textContent, "Locked");
  assert.equal(terminal.announcement.textContent, "Open the private URL printed by Clannon.");
  assert.match(terminal.output.textContent, /Open the private URL printed by Clannon\./);
  assert.doesNotMatch(terminal.output.textContent, new RegExp(ACCESS_TOKEN));
});

test("shows environment capacity conflicts without opening a terminal", async () => {
  const terminal = await openTerminal(null, false, null, {
    createStatus: 409,
    createBody: { error: "at most 4 environments may exist at once" },
  });

  assert.equal(terminal.sockets.length, 0);
  assert.equal(terminal.stateLabel.textContent, "No environment");
  assert.equal(terminal.create.disabled, false);
  assert.equal(terminal.destroy.disabled, true);
  assert.match(terminal.output.textContent, /at most 4 environments may exist at once/);
  assert.match(terminal.announcement.textContent, /at most 4 environments may exist at once/);
});

test("locks a stale tab when its terminal upgrade and authenticated access probe are rejected", async () => {
  const terminal = await openTerminal(null, false, null, {
    accessProbeStatus: 401,
    autoReady: false,
    openSocket: false,
  });

  terminal.socket.close(1006, "upgrade rejected");
  await new Promise((resolve) => setImmediate(resolve));

  const probe = terminal.fetches.find(({ url }) => url === "/api/access");
  assert.ok(probe);
  assert.equal(probe.options.cache, "no-store");
  assert.equal(probe.options.headers.Authorization, `Bearer ${ACCESS_TOKEN}`);
  assert.equal(terminal.sessionValues.has("clannon-access-token"), false);
  assert.equal(terminal.status.textContent, "Locked");
  assert.equal(terminal.reconnect.hidden, true);
  assert.equal(terminal.create.disabled, true);
});

test("measures the PTY for open and waits for strict resize readiness", async () => {
  const terminal = await openTerminal(null, false, null, { autoReady: false });

  assert.equal(terminal.socket.requestedProtocols, "clannon.terminal.v1");
  assert.equal(terminal.socket.binaryType, "arraybuffer");
  assert.deepEqual(JSON.parse(terminal.socket.sent[0]), {
    type: "open",
    version: 1,
    columns: 98,
    rows: 24,
  });
  assert.equal(terminal.status.textContent, "Connecting");
  assert.equal(terminal.command.disabled, false);
  assert.equal(terminal.run.disabled, true);
  assert.equal(terminal.interrupt.disabled, true);

  terminal.socket.message(JSON.stringify({ type: "ready", version: 1, resumed: false, resize: true }));

  assert.equal(terminal.status.textContent, "Ready");
  assert.equal(terminal.run.disabled, false);
  assert.equal(terminal.interrupt.disabled, false);
  assert.equal(terminal.output.textContent, "Clannon environment ready.\n");
  assert.match(terminal.announcement.textContent, /fresh shell/i);
  assert.equal(terminal.resizeObservers.length, 1);
  assert.deepEqual(terminal.resizeObservers[0].observed, [terminal.output]);
});

test("clamps measured PTY dimensions to the protocol bounds", async () => {
  const terminal = await openTerminal(null, false, null, {
    autoReady: false,
    cellWidth: 1,
    lineHeight: 16,
    outputHeight: 0,
    outputWidth: 20_000,
    paddingBottom: 0,
    paddingLeft: 0,
    paddingRight: 0,
    paddingTop: 0,
  });

  assert.deepEqual(JSON.parse(terminal.socket.sent[0]), {
    type: "open",
    version: 1,
    columns: 1000,
    rows: 1,
  });
});

test("trailing-debounces and deduplicates resize while rejecting stale observer work", async () => {
  const terminal = await openTerminal();
  const observer = terminal.resizeObservers[0];

  terminal.setOutputSize(976, 496);
  observer.trigger();
  observer.trigger();
  assert.equal(sentControls(terminal.socket).filter(({ type }) => type === "resize").length, 0);
  terminal.setOutputSize(992, 496);
  observer.trigger();
  terminal.flushTimers();
  assert.deepEqual(sentControls(terminal.socket).at(-1), {
    type: "resize",
    columns: 120,
    rows: 29,
  });

  observer.trigger();
  terminal.flushTimers();
  assert.equal(sentControls(terminal.socket).filter(({ type }) => type === "resize").length, 1);

  terminal.setOutputSize(1056, 496);
  observer.trigger();
  terminal.socket.close(1006, "network lost");
  assert.equal(observer.disconnected, true);
  terminal.flushTimers();
  assert.equal(sentControls(terminal.socket).filter(({ type }) => type === "resize").length, 1);

  await terminal.reconnect.dispatch("click");
  const resumed = terminal.sockets[1];
  resumed.open();
  assert.deepEqual(JSON.parse(resumed.sent[0]), {
    type: "open",
    version: 1,
    columns: 128,
    rows: 29,
  });

  observer.trigger();
  terminal.flushTimers();
  assert.equal(sentControls(terminal.socket).filter(({ type }) => type === "resize").length, 1);
  assert.equal(resumed.sent.length, 1);
});

test("drops a queued resize when destruction makes its connection stale", async () => {
  const terminal = await openTerminal();
  terminal.setOutputSize(900, 450);
  terminal.resizeObservers[0].trigger();

  await terminal.destroy.dispatch("click");
  terminal.flushTimers();

  assert.equal(sentControls(terminal.socket).filter(({ type }) => type === "resize").length, 0);
  assert.equal(terminal.resizeObservers[0].disconnected, true);
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

test("sends exact binary ETX only while ready without changing the draft or copy shortcuts", async () => {
  const terminal = await openTerminal(null, false, null, { autoReady: false });
  terminal.command.value = "draft survives";
  const outputBeforeReady = terminal.output.textContent;

  await terminal.interrupt.dispatch("click");
  assert.equal(terminal.socket.sent.length, 1);
  assert.equal(terminal.command.value, "draft survives");
  assert.equal(terminal.output.textContent, outputBeforeReady);

  terminal.socket.message(JSON.stringify({ type: "ready", version: 1, resumed: false, resize: true }));
  const readyOutput = terminal.output.textContent;
  const focusBeforeInterrupt = terminal.command.focusCount;
  await terminal.interrupt.dispatch("click");

  assert.deepEqual(Array.from(terminal.socket.sent.at(-1)), [0x03]);
  assert.equal(terminal.command.value, "draft survives");
  assert.equal(terminal.output.textContent, readyOutput);
  assert.equal(terminal.announcement.textContent, "Ctrl-C sent.");
  assert.equal(terminal.command.focusCount, focusBeforeInterrupt + 1);

  const copy = { key: "c", ctrlKey: true, prevented: false, preventDefault() { this.prevented = true; } };
  await terminal.command.dispatch("keydown", copy);
  assert.equal(copy.prevented, false);

  terminal.socket.close(1006, "network lost");
  assert.equal(terminal.interrupt.disabled, true);
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

test("projects fragmented CR, CSI, OSC, ESC, UTF-8, and controls as safe plain text", async () => {
  const terminal = await openTerminal();
  const euro = new TextEncoder().encode("€");

  terminal.socket.message(euro.slice(0, 2).buffer);
  terminal.socket.message(euro.slice(2).buffer);
  terminal.socket.message(bytes(" alpha\r"));
  terminal.socket.message(bytes("\nbeta\rgamma\u001b[3"));
  terminal.socket.message(bytes("1mred\u001b[0m\u001b]0;ti"));
  terminal.socket.message(bytes("tle\u001b"));
  terminal.socket.message(bytes("\\done\u001b7\u0001tail\r"));
  terminal.socket.message(JSON.stringify({ type: "exit", code: 0 }));

  assert.match(terminal.output.textContent, /€ alpha\nbeta\ngamma\n\[terminal control sequences omitted/);
  assert.match(terminal.output.textContent, /\nreddone␁tail\n/);
  assert.match(terminal.output.textContent, /done␁tail\n/);
  assert.doesNotMatch(terminal.output.textContent, /\u001b|\[31m|\[0m|title/);
  assert.equal((terminal.output.textContent.match(/terminal control sequences omitted/g) ?? []).length, 1);
  assert.equal(terminal.decoders[0].flushes, 1);
});

test("renders bidi formatting controls visibly in live output and transcript evidence", async () => {
  const terminal = await openTerminal(null, false, {
    captured_at_ms: 1_700_000_001_000,
    execution_events: [],
    execution_events_omitted: 0,
    files: [],
    network: [],
    processes: [],
    transcript: [{ timestamp_ms: 1_700_000_000_000, direction: "output", data: "safe\u2066spoof\u2069" }],
    warnings: [],
  });

  terminal.socket.message(bytes("safe\u202Espoof\u202C"));

  assert.match(terminal.output.textContent, /safe\[U\+202E\]spoof\[U\+202C\]/);
  assert.doesNotMatch(terminal.output.textContent, /\u202e|\u202c/);
  assert.match(terminal.transcript.innerHTML, /safe\[U\+2066\]spoof\[U\+2069\]/);
  assert.doesNotMatch(terminal.transcript.innerHTML, /\u2066|\u2069/);
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
    (terminal) => terminal.socket.message(JSON.stringify({ type: "ready", version: 2, resumed: false, resize: true })),
    (terminal) => terminal.socket.message(JSON.stringify({ type: "ready", version: 1, resumed: false, resize: false })),
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

test("keeps a generic runtime failure recoverable without claiming the shell ended", async () => {
  const terminal = await openTerminal();

  terminal.socket.message(JSON.stringify({
    type: "error",
    code: "runtime_error",
    message: "terminal proxy stopped",
  }));
  terminal.socket.close(1011, "terminal proxy stopped");

  assert.equal(terminal.status.textContent, "Terminal error");
  assert.match(terminal.output.textContent, /runtime_error: terminal proxy stopped/);
  assert.equal(terminal.reconnect.hidden, false);
  assert.equal(terminal.reconnect.textContent, "Reconnect");
  assert.match(terminal.announcement.textContent, /existing shell may still be available/i);
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
  resumed.message(JSON.stringify({ type: "ready", version: 1, resumed: true, resize: true }));

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
  fresh.message(JSON.stringify({ type: "ready", version: 1, resumed: false, resize: true }));

  assert.match(terminal.output.textContent, /fresh shell opened after the previous shell ended/i);
  assert.equal(terminal.status.textContent, "Ready");
});

test("relies on authoritative PTY echo instead of fabricating submitted commands", async () => {
  const terminal = await openTerminal();

  for (const command of ["echo one", "echo two"]) {
    terminal.command.value = command;
    terminal.command.selectionStart = command.length;
    const event = enterEvent();
    await terminal.command.dispatch("keydown", event);
    assert.equal(event.prevented, true);
  }

  assert.deepEqual(sentInput(terminal.socket), ["echo one\n", "echo two\n"]);
  assert.equal(terminal.output.textContent, "Clannon environment ready.\n");
  terminal.socket.message(bytes("/workspace # echo one\r\none\r\n/workspace # echo two\r\ntwo\r\n"));
  assert.equal((terminal.output.textContent.match(/echo one/g) ?? []).length, 1);
  assert.equal((terminal.output.textContent.match(/echo two/g) ?? []).length, 1);
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
  assert.equal(terminal.output.textContent, "Clannon environment ready.\n");
  terminal.socket.message(bytes("echo hey \\\r\n> echo something\r\n"));
  assert.equal((terminal.output.textContent.match(/echo hey/g) ?? []).length, 1);
  assert.equal((terminal.output.textContent.match(/echo something/g) ?? []).length, 1);
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
  assert.equal(terminal.output.textContent, "Clannon environment ready.\n");
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
  assert.match(html, /<button id="interrupt"[^>]*aria-label="Send Ctrl-C to the running shell"[^>]*disabled>Ctrl-C<\/button>/);
  assert.match(html, /Live \/ PTY · plain text/);
  assert.ok(html.indexOf('id="execution-events"') < html.indexOf('id="transcript"'));
  assert.match(html, /id="execution-events"[^>]*role="region"[^>]*aria-labelledby="execution-title"[^>]*tabindex="0"/);
  assert.match(styles, /\.observation-body:focus-visible/);
  assert.match(styles, /@media \(prefers-color-scheme: dark\) {[\s\S]*:root:not\(\[data-theme\]\)/);
});

test("renders authoritative execution events in retained order", async () => {
  const executionEvents = [
    { sequence: 20, timestamp_ms: 1_700_000_000_020, type: "environment_ready" },
    { sequence: 21, timestamp_ms: 1_700_000_000_010, type: "shell_started", generation: 2, columns: 91, rows: 33 },
    { sequence: 22, timestamp_ms: 1_700_000_000_022, type: "terminal_input", generation: 2, input_kind: "text", bytes: 12 },
    { sequence: 23, timestamp_ms: 1_700_000_000_023, type: "terminal_input", generation: 2, input_kind: "binary", bytes: 4 },
    { sequence: 24, timestamp_ms: 1_700_000_000_024, type: "terminal_input", generation: 2, input_kind: "interrupt", bytes: 1 },
    { sequence: 25, timestamp_ms: 1_700_000_000_025, type: "terminal_resized", generation: 2, columns: 117, rows: 41 },
    { sequence: 26, timestamp_ms: 1_700_000_000_026, type: "shell_exited", generation: 2, code: 7 },
    { sequence: 27, timestamp_ms: 1_700_000_000_027, type: "shell_exited", generation: 2, code: null },
    { sequence: 28, timestamp_ms: 1_700_000_000_028, type: "shell_failed", generation: 4 },
  ];
  const terminal = await openTerminal(null, false, {
    captured_at_ms: 1_700_000_001_000,
    execution_events: executionEvents,
    execution_events_omitted: 0,
    files: [],
    network: [],
    processes: [],
    transcript: [],
    warnings: [],
  });

  assert.equal(terminal.executionCount.textContent, executionEvents.length);
  assert.equal((terminal.executionEvents.innerHTML.match(/class="evidence-row execution-row"/g) ?? []).length, executionEvents.length);
  assert.match(terminal.executionEvents.innerHTML, /<time datetime="2023-/);
  assert.match(terminal.executionEvents.innerHTML, /generation 2 · 91 × 33/);
  assert.match(terminal.executionEvents.innerHTML, /Input accepted<\/code>[\s\S]*generation 2 · text · 12 bytes/);
  assert.match(terminal.executionEvents.innerHTML, /Input accepted<\/code>[\s\S]*generation 2 · binary · 4 bytes/);
  assert.match(terminal.executionEvents.innerHTML, /Interrupt sent<\/code>[\s\S]*generation 2 · 1 byte/);
  assert.match(terminal.executionEvents.innerHTML, /Terminal resized<\/code>[\s\S]*generation 2 · 117 × 41/);
  assert.match(terminal.executionEvents.innerHTML, /Shell exited<\/code>[\s\S]*generation 2 · code 7/);
  assert.match(terminal.executionEvents.innerHTML, /generation 2 · exit code unavailable/);
  assert.match(terminal.executionEvents.innerHTML, /data-outcome="failed"[\s\S]*Shell failed<\/code>[\s\S]*generation 4/);
  assert.ok(terminal.executionEvents.innerHTML.indexOf("Environment ready") < terminal.executionEvents.innerHTML.indexOf("Shell started"));
  assert.doesNotMatch(terminal.executionEvents.innerHTML, /command (ran|completed)/i);
});

test("shows runtime and browser omissions while safely preserving unknown events", async () => {
  const executionEvents = Array.from({ length: 102 }, (_, index) => ({
    sequence: index,
    timestamp_ms: 1_700_000_000_000 + index,
    type: index === 101 ? "<img src=x>\u202E" : "environment_ready",
  }));
  const terminal = await openTerminal(null, false, {
    captured_at_ms: 1_700_000_001_000,
    execution_events: executionEvents,
    execution_events_omitted: 7,
    files: [],
    network: [],
    processes: [],
    transcript: [],
    warnings: [],
  });

  assert.equal(terminal.executionCount.textContent, 102);
  assert.match(terminal.executionEvents.innerHTML, /Runtime omitted 7 earlier events from this retained tail/);
  assert.match(terminal.executionEvents.innerHTML, /Showing the newest 100 of 102 retained events/);
  assert.equal((terminal.executionEvents.innerHTML.match(/class="evidence-row execution-row"/g) ?? []).length, 100);
  assert.doesNotMatch(terminal.executionEvents.innerHTML, /· #0<\/span>|· #1<\/span>/);
  assert.match(terminal.executionEvents.innerHTML, /· #101<\/span>/);
  assert.match(terminal.executionEvents.innerHTML, /Unknown event/);
  assert.match(terminal.executionEvents.innerHTML, /type &lt;img src=x&gt;\[U\+202E\]/);
  assert.doesNotMatch(terminal.executionEvents.innerHTML, /<img|\u202e/i);
});

test("uses explicit execution empty and reset states", async () => {
  const terminal = await openTerminal();

  assert.equal(terminal.executionCount.textContent, 0);
  assert.equal(terminal.executionEvents.innerHTML, "Nothing recorded in this snapshot.");

  await terminal.destroy.dispatch("click");
  assert.equal(terminal.executionCount.textContent, "0");
  assert.equal(terminal.executionEvents.textContent, "No execution events yet.");
});

test("preserves execution evidence when a later refresh fails", async () => {
  const snapshot = {
    captured_at_ms: 1_700_000_001_000,
    execution_events: [{ sequence: 1, timestamp_ms: 1_700_000_000_000, type: "environment_ready" }],
    execution_events_omitted: 0,
    files: [],
    network: [],
    processes: [],
    transcript: [],
    warnings: [],
  };
  const terminal = await openTerminal(null, false, null, {
    observationResponses: [
      { status: 200, body: snapshot },
      { status: 502, body: { error: "observation unavailable" } },
    ],
  });
  const renderedEvents = terminal.executionEvents.innerHTML;
  const capturedAt = terminal.snapshotTime.textContent;

  await terminal.refresh.dispatch("click");

  assert.equal(terminal.executionEvents.innerHTML, renderedEvents);
  assert.equal(terminal.executionCount.textContent, 1);
  assert.equal(terminal.snapshotTime.textContent, capturedAt);
  assert.match(terminal.warnings.textContent, /observation unavailable/);
});

test("renders timestamped transcript evidence without trusting its HTML", async () => {
  const transcript = Array.from({ length: 102 }, (_, index) => ({
    timestamp_ms: 1_700_000_000_000 + index,
    direction: index === 101 ? "input" : "output",
    data: index === 101 ? "printf '<script>&\\n'\n\u0003\u001b" : `output ${index}\n`,
  }));
  const terminal = await openTerminal(null, false, {
    captured_at_ms: 1_700_000_001_000,
    execution_events: [],
    execution_events_omitted: 0,
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
  assert.match(terminal.transcript.innerHTML, /Input<\/span>/);
  assert.match(terminal.transcript.innerHTML, /&lt;script&gt;&amp;\\n/);
  assert.ok(terminal.transcript.innerHTML.includes("printf &#039;&lt;script&gt;&amp;\\n&#039;\n␃␛</code>"));
  assert.match(terminal.transcript.innerHTML, /␃␛/);
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

test("follows live output only while the user is already at the bottom", async () => {
  const terminal = await openTerminal();
  terminal.output.clientHeight = 400;
  terminal.output.scrollHeight = 1_000;
  terminal.output.scrollTop = 200;

  terminal.socket.message(bytes("while inspecting history"));
  assert.equal(terminal.output.scrollTop, 200);

  terminal.output.scrollTop = 600;
  terminal.socket.message(bytes("follow newest output"));
  assert.equal(terminal.output.scrollTop, 1_000);
});
