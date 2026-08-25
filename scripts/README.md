# Clannon expert crew

The primary Codex session owns product direction, integration, verification,
and commits. Headless Codex experts handle bounded work in separate contexts.

## Dispatch

Write a self-contained brief under `.agents/briefs/<role>/`, then run:

```sh
./scripts/crew.sh run runtime \
  --brief .agents/briefs/runtime/terminal-design.md \
  --dir crates/runtime \
  --dir tests/smoke.sh
```

Roles: `runtime`, `server`, `experience`, `verification`, `architecture`, and
`review`. Architecture and review tasks require explicit `--dir` scopes. Other
roles default to their owning directory; repeat `--dir` when source and tests
form one honest task boundary.

Workers are always fresh Codex executions. They cannot commit or push. The
launcher refuses dirty owned paths, permits one worker per role, detects new
out-of-scope edits, stores full logs under `.agents/runs/`, and appends concise
provenance to `comms/YYYY-MM-DD/coordinator.md`.

Assign parallel workers only disjoint path sets. The shared Git worktree is not
a merge queue; coordinator review remains the integration boundary.

## Coordination outputs

- `reports/<role>/`: deep local evidence and handoffs.
- `proposals/to-coordinator/`: decisions that block an expert.
- `comms/`: concise tracked provenance and durable status.

Templates and lifecycle rules live in the README inside each directory.
