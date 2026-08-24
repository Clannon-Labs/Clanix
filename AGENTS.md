# Clannon agent guide

Read `PROJECT.md` before changing product behavior.

## Working agreement

- Build the smallest complete vertical slice that advances the current V0.
- Keep the Rust readable to a beginner/intermediate Rust programmer. Prefer plain
  data structures and explicit control flow over framework-heavy abstractions.
- Keep the runtime local and single-node until measured needs prove otherwise.
- Use rootless Podman as the isolation boundary. Never silently fall back to
  running guest commands on the host.
- Treat every environment as disposable. A successful destroy must remove its
  container, and server shutdown should attempt cleanup of containers it created.
- Record observations from real execution. Do not invent AI explanations or
  simulated process/file/network data.
- Add a dependency only when it materially shortens or secures the acceptance
  path. Do not add providers, plugin systems, queues, databases, or deployment
  machinery speculatively.
- Exercise the user-visible path and run `cargo test` before declaring work done.

## Repository map

- `src/main.rs`: HTTP/WebSocket application and container lifecycle.
- `static/`: dependency-free browser UI.
- `PROJECT.md`: product mission, V0 contract, and architectural boundaries.

## Commands

```sh
cargo run
cargo test
```

The server binds to `127.0.0.1:3000` by default. `CLANNON_BIND` and
`CLANNON_IMAGE` may override the bind address and container image.
