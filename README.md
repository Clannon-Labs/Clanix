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
- Session snapshots retain an immutable, bounded copy of `/workspace` and can
  fork that copy into a fresh disposable environment after the source is gone.
- Guest outbound networking is disabled by default; loopback listeners inside
  the disposable container continue to work and appear in evidence.
- State is deliberately in memory; graceful shutdown attempts to clean up every
  container created by that server process.

## Release status

There is no published binary release yet. The first candidate is `v0.1.0`, a
local alpha for Linux x86-64. Publication is blocked until approved MIT and
Apache-2.0 license texts and attribution are checked in, third-party distribution
obligations are reviewed, and GitHub private vulnerability reporting is
verified. `main` remains development state until those gates and the acceptance
checklist in [`RELEASING.md`](RELEASING.md) pass.

The planned asset is
`clannon-v0.1.0-x86_64-unknown-linux-musl.tar.gz` with a companion
`SHA256SUMS`. It contains one `clannon` executable with the browser assets
embedded, plus the README, product and security contracts, approved license
texts, build metadata, and a locked dependency inventory. The inventory is
not a standards-compliant SBOM or a bundle of third-party license texts.

## Supported local-alpha host

- x86-64 Linux
- Rootless Podman 5.x; the candidate is currently exercised with Podman 5.8.4
- A current Chromium-based desktop browser for the supported UI path
- Internet access on first environment creation to pull Clannon's immutable
  Alpine 3.24.1 image, unless that image is already cached
- `curl`, `sha256sum`, `tar`, and `install`, or equivalent tools, for the
  documented archive installation

Clannon does not require `sudo`, a system service, or Rust when using the
release archive. Other architectures, browsers, container engines, package
managers, and host operating systems are untested and unsupported in the first
alpha.

Confirm that Podman reports `true`:

```sh
podman info --format '{{.Host.Security.Rootless}}'
```

## Install a published alpha

These commands describe the planned `v0.1.0` release and will work only after it
is published. They intentionally download an exact version rather than a mutable
“latest” URL:

```sh
release_version=0.1.0
release_root="clannon-v${release_version}-x86_64-unknown-linux-musl"
release_url="https://github.com/Clannon-Labs/Clannon/releases/download/v${release_version}"

curl --fail --location --remote-name "${release_url}/${release_root}.tar.gz"
curl --fail --location --remote-name "${release_url}/SHA256SUMS"
sha256sum --check SHA256SUMS
tar --extract --gzip --file "${release_root}.tar.gz"
"./${release_root}/clannon" --version
"./${release_root}/clannon" doctor
```

Run directly from the extracted directory, or optionally copy the executable to
a user-owned directory already on `PATH`:

```sh
mkdir --parents "$HOME/.local/bin"
install --mode 0755 "./${release_root}/clannon" "$HOME/.local/bin/clannon"
clannon --version
clannon doctor
```

`clannon doctor` checks that the host is Linux, the configured loopback address
can bind, and Podman is working rootlessly. It does not pull the guest image,
enforce the supported Podman major version, open a browser, or prove the full
create-to-destroy path.

## Run

```sh
clannon
# Equivalent explicit command:
clannon serve
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
current snapshots.

Use **Save snapshot** to retain the current `/workspace` for this server session.
The browser deliberately keeps one active workbench: save, explicitly
**Destroy** the active environment, then **Fork** a retained snapshot into that
workbench. A snapshot is immutable, survives destruction of its source, and may
be forked repeatedly until it is deleted. A fork is a normal fresh environment
with a new ID, shell, transcript, Activity log, and observation baselines. It
does not inherit source processes, shell variables or working directory, open
sockets, terminal history, sampled evidence, or changes outside `/workspace`.

One server owns at most four environments that are creating, live, or awaiting
cleanup. Separately, it retains at most four workspace snapshots, at most 64 MiB
per serialized archive and 128 MiB in aggregate. Only one snapshot capture runs
at a time. The archive byte count includes serialization overhead and is not the
logical size reported by tools such as `du`. Snapshots live only in server
memory and disappear on restart.

Snapshot capture copies a live `/workspace`; it is not atomic or
application-consistent, so files being changed during the copy may represent
different moments. Ordinary files, directories, permission modes, symbolic
links, and hard links are restored. The configured base image and Clannon's
normal network, process, CPU, memory, capability, and privilege restrictions
are applied anew to every fork.

Transcript evidence is the newest retained tail, bounded to 500 whole
entries and 1 MiB of UTF-8 entry data. Activity separately keeps at most the
newest 500 whole events and 1 MiB of estimated owned event data, and reports how
many earlier events were omitted by either bound. Event sequence defines order;
timestamps may repeat or move with the host clock. Runtime-known facts are
timestamped when recorded; sampled-change events are timestamped when their
capture completed. Both tails are lost on destroy or restart. The opaque
environment ID is an identifier, not a credential; the private URL capability
is what authorizes local API and terminal access.

The CLI surface is intentionally small:

```text
clannon [serve|doctor|--version|--help]
```

`CLANNON_BIND` changes the numeric loopback listen address for `serve` and the
bind check used by `doctor`. `CLANNON_IMAGE` changes only the guest image used by
`serve`:

```sh
CLANNON_BIND=127.0.0.1:4000 clannon
CLANNON_IMAGE=docker.io/library/alpine:3.24 clannon
```

A custom guest image remains an expert option, not a compatibility promise. It
must provide Clannon's existing POSIX shell and observation tools and `tar` for
workspace snapshot capture and restore. A missing required guest tool fails the
corresponding operation; Clannon never falls back to a host command.

Only numeric IPv4 or IPv6 loopback bind addresses are supported. Port `0` is
allowed; Clannon prints the actual selected port in its private URL. API clients
must use an allowed `Host`, an optional matching HTTP `Origin`, and
`Authorization: Bearer <capability>`. The terminal WebSocket carries the same
capability in its `access_token` query parameter.

## Upgrade

Clannon has no automatic updater or persistent environment migration. Stop the
running process with Ctrl-C and wait for it to exit before replacing the binary;
all runtime ownership, workspace snapshots, and in-memory Activity/transcript
evidence are disposable and do not survive the restart. Graceful shutdown
attempts to remove the containers first. Download the newer exact-version
archive and `SHA256SUMS`, verify them as above, overwrite only the installed
executable, and rerun `clannon --version` and `clannon doctor`.

Before 1.0, a new minor version may break the CLI, terminal protocol,
observation JSON, or disposable runtime behavior. Read that release's notes
before upgrading. Only the newest tagged local alpha is supported.

## Remove

Stop Clannon with Ctrl-C and wait for graceful cleanup, then remove the exact
binary you installed:

```sh
rm -- "$HOME/.local/bin/clannon"
```

If you ran from an extracted archive, remove that extracted directory yourself
after confirming its path. Clannon stores no database or system-wide
configuration. Closing its browser tab clears the tab-scoped capability. Podman
images remain cached because they may be shared with other tools. After an
abrupt process or host termination, inspect `podman ps --all` for a confirmed
Clannon-named container and remove only that exact container explicitly.

## Build from source

Until `v0.1.0` is published, this is the supported developer path. It requires
Rust 1.85 or newer (edition 2024), the repository checkout, and rootless Podman:

```sh
cargo run -- doctor
cargo run
```

Source builds are development builds, not substitutes for exercising the
packaged release candidate.

## Verify

```sh
cargo test --workspace
./tests/smoke.sh
```

Unit tests do not require Podman. The smoke test requires working rootless user
namespaces and exercises create, PTY sizing and resize, foreground interruption,
reconnection, runtime-known and refresh-sampled Activity semantics, observations,
workspace save/fork fidelity and fresh-state boundaries, destroy, access-gate
rejection, and rejection of destroyed environment and deleted snapshot IDs.

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
