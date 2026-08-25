
## dispatched experts
- `08:03` **architecture** via **codex** — crew-smoke.md — exit 0, 80s — `.agents/runs/20260825-080221-architecture.out`
- `08:06` **review** via **codex** — crew-hardening-smoke.md — exit 0, 32s — `.agents/runs/20260825-080543-review.out`
- `09:28` **runtime** via **codex** — reconnectable-terminal.md — exit 0, 979s — `.agents/runs/20260825-091201-runtime.out`
- `10:02` **server** via **codex** — structured-terminal-implementation.md — exit 0, 1045s — `.agents/runs/20260825-094512-server.out`
- `10:28` **architecture** via **codex** — pty-session-design.md — exit 5, 613s — `.agents/runs/20260825-101845-architecture.out`
- `10:36` **server** via **codex** — local-access-gate.md — exit 5, 636s — `.agents/runs/20260825-102615-server.out`
- `11:04` **experience** via **codex** — plaintext-pty.md — exit 5, 448s — `.agents/runs/20260825-105730-experience.out`
- `11:05` **server** via **codex** — pty-protocol-and-smoke.md — exit 5, 455s — `.agents/runs/20260825-105730-server.out`
- `11:05` **runtime** via **codex** — real-container-pty.md — exit 5, 467s — `.agents/runs/20260825-105730-runtime.out`

## v0.1.0 candidate evidence

- Rust 1.85.0 compiled and passed the locked workspace/runtime test path after
  four let-chain expressions were rewritten to honor the declared MSRV.
- `x86_64-unknown-linux-musl` built with Rust's `rust-lld`; `file` reported a
  static PIE executable, and that executable passed `--version` plus `doctor`
  against rootless Podman 5.8.4. The same stripped musl executable passed the
  complete real-Podman smoke through access control, PTY, Activity evidence,
  reconnect, destroy, and cleanup.
- RustSec `cargo-audit 0.22.2` found no vulnerabilities in 90 locked
  dependencies using advisory database commit
  `a7bfe16948bf6f3ee25bdee4822209f87da21b80` (1,226 advisories, updated
  2026-08-24T22:42:17-04:00).
- Publication remains stopped on approved project license attribution,
  third-party distribution review, and verified GitHub private vulnerability
  reporting. The candidate workflow is manual and read-only.
