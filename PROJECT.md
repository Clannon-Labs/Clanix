# Clannon

Clannon is an interactive environment for understanding how software actually
behaves. Its core loop is:

1. run software in an isolated environment;
2. observe what really happened;
3. inspect execution across time and system boundaries;
4. eventually explain, fork, replay, and share it.

`Clannon.com` is the future hosted product. `Clannonic.org` is the name reserved
for a future open-source execution runtime. This repository is an intentionally
small local V0, not either site's production architecture.

## V0 product contract

The first vertical slice lets one local user:

- create a rootless, isolated Linux container;
- enter commands in a browser and see their real output;
- inspect a basic snapshot of running processes, files changed in the workspace,
  open TCP/UDP sockets, and the terminal transcript;
- explicitly destroy the environment and its data;
- trust that guest commands never fall back to host execution.

The observation shape should remain easy for a later AI layer to consume, but V0
does not call a model or generate explanations.

## V0 acceptance criteria

- `cargo run` starts one local Rust process and serves the UI.
- The server accepts loopback binds only and prints a fresh capability-bearing
  private URL derived from its actual listening address.
- Static assets require an exact local Host. Every API and terminal WebSocket
  request additionally requires an allowed Origin when present and the current
  server capability before runtime state is accessed.
- **Create environment** starts a rootless Podman container with a writable
  `/workspace` and returns an opaque environment ID.
- The browser terminal drives `/bin/sh` on a real container PTY, including
  initial sizing, later resize controls, and raw control bytes such as Ctrl-C.
- The observation endpoint returns JSON containing transcript, process, file,
  and network data gathered from that container.
- **Destroy environment** force-removes the container. Reusing its ID fails.
- Container names are scoped to Clannon, and shutdown attempts to remove every
  container created by this server process.
- Unit tests cover parsing and lifecycle-independent behavior; an opt-in smoke
  test exercises Podman when the host permits it.

The browser terminal negotiates `clannon.terminal.v1`, sends a validated `open`
control before the runtime attachment opens, and becomes ready only after the
server's structured `ready` control. The server streams terminal output as raw
binary frames and reserves its text frames for structured ready, exit, or error
controls. After ready, clients may send bounded structured text or raw binary
input or validated resize controls. A detached live shell may be reattached;
recently captured output from the detached interval remains available subject
to the transcript retention limit, but is not replayed into the reattached live
stream. The dependency-free browser renders a control-safe plain-text PTY log;
it is intentionally not a screen terminal emulator.

## Architecture principles

- **One process, one node, in memory.** The Rust server owns HTTP, WebSockets,
  lifecycle state, and observation collection.
- **The container is the security boundary.** Rootless Podman provides namespaces
  and disposable storage. Clannon does not execute guest commands directly.
- **Observe through ordinary Linux interfaces.** V0 uses `ps`, filesystem
  metadata, and `/proc/net`; eBPF and syscall tracing can wait.
- **Lose data on restart.** This is desirable in V0: no database, migrations,
  accounts, or recovery protocol.
- **Browser code stays dependency-free.** The terminal is deliberately modest;
  richer emulation can be added only when terminal behavior requires it.
- **Prefer truthful limitations.** A snapshot is not a timeline. Container
  isolation is not a hardened multi-tenant sandbox.

## Explicit non-goals

- Kubernetes, microservices, queues, service discovery, or distributed state.
- Multi-user hosting, authentication, billing, domains, DNS, or cloud purchase.
- A hardened hostile-code platform or a claim of perfect containment.
- Persistent environments, images built from user repositories, uploads, or IDE
  features.
- AI explanations, agent orchestration, vector databases, or model-provider
  abstraction.
- eBPF, syscall tracing, deterministic replay, sharing, or collaboration.
- A polished terminal emulator, desktop/mobile parity, or production deployment.

## Near-term sequence

Only after V0 is proven locally: improve terminal fidelity; add timestamped
execution events; then consider saved/forked environments. Hosted-product and
open-source-runtime separation should be designed from evidence, not in advance.
