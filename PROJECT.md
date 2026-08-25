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
- At most four environments may be creating, live, or awaiting cleanup at once;
  a fifth create fails without starting another container.
- Guest outbound networking is disabled. Loopback remains available inside the
  container so local listeners still appear in network evidence.
- The browser terminal drives `/bin/sh` on a real container PTY, including
  initial sizing, later resize controls, and raw control bytes such as Ctrl-C.
- The observation endpoint returns JSON containing a timestamped Activity tail,
  terminal transcript, and process, file, and network data gathered from that
  container.
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

The transcript is a retained newest tail, not an unlimited audit log. It keeps
at most 500 whole entries and 1 MiB of UTF-8 entry data; the counts shown by the
browser describe that retained tail.

Activity is a separate retained newest tail of at most 500 whole execution
events and 1 MiB of estimated owned event data. Its per-environment sequence is
the authoritative order; timestamps are the host wall-clock time when Clannon
recorded each fact and are not promised to be monotonic. Observation JSON
returns that tail as `execution_events` and the number of earlier events omitted
by either retention bound as `execution_events_omitted`. Runtime-known event
types are `environment_ready`; `shell_started` with generation and dimensions;
`terminal_input` with generation, `text`, `binary`, or `interrupt` input kind,
and byte count; `terminal_resized` with generation and dimensions;
`shell_exited` with generation and nullable shell code; and `shell_failed` with
generation. Accepted input is not proof that a command executed, and a shell
exit is not an individual command result. Reattaching a live shell does not
start a new shell.

Successful observation refreshes also compare the current system sample with
the last successful sample. `process_added`, `process_removed`, and
`process_changed` carry `capture_sequence` and either `process` or
`previous`/`current` process observations. The equivalent file variants carry
`file` or `previous`/`current` file observations. `network_added` and
`network_removed` carry a `network` observation. These events mean only that a
fact appeared, disappeared, or changed between successful refresh samples.
Their shared event timestamp is capture completion time, not the exact time the
underlying change occurred; short-lived facts between samples may be missed,
and no causal link to terminal input or output is inferred. The first successful
sample for each domain establishes that domain's baseline and emits no changes
for it. A failed process, file, or network domain does not fabricate removals for
that domain. If a file capture exceeds 200 entries, the snapshot warns, emits no
file changes, and preserves the previous successful file baseline for a later
comparison.

Activity and transcript evidence are lost when the environment is destroyed or
the server restarts. Environment IDs identify runtime objects, but only the
server capability authorizes access to them.

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
- **Prefer truthful limitations.** Activity combines runtime-known terminal and
  shell facts with changes detected between process, file, and network samples;
  the observation payload still contains the current point-in-time snapshots.
  Container isolation is not a hardened multi-tenant sandbox.

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

With terminal fidelity, runtime-known events, and truthfully labeled sampled
system changes proven locally, next consider saved/forked environments.
Hosted-product and open-source-runtime separation should be designed from
evidence, not in advance.
