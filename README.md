# Clannon

Clannon is a small local workbench for running commands inside a disposable
Linux container and inspecting what actually happened.

This repository is the first hosted-product slice, not a production sandbox and
not yet the future Clannonic open-source runtime. The product contract and the
decisions that keep the project small live in [`PROJECT.md`](PROJECT.md).

## What works

- A Rust server creates and destroys rootless Podman containers.
- A dependency-free browser UI drives a real, resizable container PTY over
  WebSockets, including Ctrl-C and reconnectable shell sessions.
- Activity shows a timestamped, ordered tail of runtime-known environment and
  shell facts plus process, file, and network changes detected between refresh
  samples, without pretending input is a completed command or sampled changes
  are continuous tracing.
- Snapshot observations show the terminal transcript, processes, workspace files,
  and Linux TCP/UDP socket tables.
- Guest outbound networking is disabled by default; loopback listeners inside
  the disposable container continue to work and appear in evidence.
- State is deliberately in memory; stopping the server cleans up its containers.

## Requirements

- Rust 1.85 or newer (edition 2024)
- Rootless Podman
- Internet access on first launch to pull `alpine:3.20`, unless it is cached

Confirm that Podman reports `true`:

```sh
podman info --format '{{.Host.Security.Rootless}}'
```

## Run

```sh
cargo run
```

Clannon prints a fresh private URL such as
`http://127.0.0.1:3000/#<capability>`. Open that complete URL in the browser;
the fragment is moved into tab-scoped session storage before API or terminal
access is enabled. If the fragment is missing and the tab has no saved
capability, the workbench remains readable but non-operational.

Create an environment, then run commands such as:

```sh
printf 'hello from Clannon\n'
printf 'evidence\n' > note.txt
sleep 30 &
```

Refresh **Evidence** once to establish the system baseline. Later refreshes show
Activity events for process, workspace-file, and network facts that appeared,
disappeared, or changed between successful samples, alongside the transcript and
current snapshots. Use **Destroy** when finished.

One server owns at most four environments that are creating, live, or awaiting
cleanup. Transcript evidence is the newest retained tail, bounded to 500 whole
entries and 1 MiB of UTF-8 entry data. Activity separately keeps at most the
newest 500 whole events and 1 MiB of estimated owned event data, and reports how
many earlier events were omitted by either bound. Event sequence defines order;
timestamps may repeat or move with the host clock. Runtime-known facts are
timestamped when recorded; sampled-change events are timestamped when their
capture completed. Both tails are lost on destroy or restart. The opaque
environment ID is an identifier, not a credential; the private URL capability
is what authorizes local API and terminal access.

The optional settings are intentionally limited:

```sh
CLANNON_BIND=127.0.0.1:4000 cargo run
CLANNON_IMAGE=docker.io/library/alpine:3.20 cargo run
```

Only numeric IPv4 or IPv6 loopback bind addresses are supported. Port `0` is
allowed; Clannon prints the actual selected port in its private URL. API clients
must use an allowed `Host`, an optional matching HTTP `Origin`, and
`Authorization: Bearer <capability>`. The terminal WebSocket carries the same
capability in its `access_token` query parameter.

## Verify

```sh
cargo test --workspace
./tests/smoke.sh
```

Unit tests do not require Podman. The smoke test requires working rootless user
namespaces and exercises create, PTY sizing and resize, foreground interruption,
reconnection, runtime-known and refresh-sampled Activity semantics, observations,
destroy, access-gate rejection, and rejection of the destroyed ID.

## Current boundaries

This is a truthful V0: the shell runs on a real container PTY, while the browser
shows a control-safe plain-text log rather than a full screen-terminal emulator.
Activity records accepted input and shell/runtime facts; it does not parse
commands, infer per-command completion, or connect input causally to output.
Sampled system events mean only that a fact differed between successful refresh
samples: they do not reveal causality or exact occurrence time, and short-lived
facts may be missed. Each domain's first successful sample is only its baseline.
A failed observation domain does not fabricate removals; a workspace capture
over 200 files warns and preserves the prior file baseline. Process, file, and
network observations remain current point-in-time snapshots. Rootless Podman is
useful isolation but not a hardened hostile multi-tenant security boundary. See
`PROJECT.md` before widening the scope.
