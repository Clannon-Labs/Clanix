# Expert proposals

Use proposals only when an expert needs a coordinator decision before it can
continue. Status and completed-work summaries belong in the final response or
`comms/`; detailed evidence belongs in `reports/`.

Store proposals at `proposals/to-coordinator/YYYY-MM-DD_short-slug.md`:

```markdown
From: <role>-expert
To: product-coordinator
Status: pending | accepted | rejected | done
Priority: low | normal | high
Summary: <one-line decision request>

## Context
## Decision required
## Options and consequences
## Recommendation
## What this unblocks
## Response
```

The coordinator appends `## Response`, updates status, and moves settled files
to `proposals/archive/`.
