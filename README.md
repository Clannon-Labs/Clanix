# Clannon

Clannon is a small local workbench for running commands inside a disposable
Linux container and inspecting what actually happened.

This repository is the first hosted-product slice, not a production sandbox and
not yet the future Clannonic open-source runtime. The product contract and the
decisions that keep the project small live in [`PROJECT.md`](PROJECT.md).

## What works

- A Rust server creates and destroys rootless Podman containers.
- A dependency-free browser UI provides a command terminal over WebSockets.
- Snapshot observations show the terminal transcript, processes, workspace files,
  and Linux TCP/UDP socket tables.
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

Refresh **Evidence** to see the resulting transcript, file, and process snapshot.
Use **Destroy** when finished.

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
namespaces and exercises create, WebSocket command execution, observations,
destroy, access-gate rejection, and rejection of the destroyed ID.

## Current boundaries

This is a truthful V0: the browser is a command console rather than a full PTY
emulator; observations are point-in-time snapshots rather than a timeline; and
rootless Podman is useful isolation but not a hardened hostile multi-tenant
security boundary. See `PROJECT.md` before widening the scope.
