# ADR-0002: The Rust types are the schema

- Status: Accepted
- Date: 2026-07-09
- Deciders: kaminsod

## Context

`aplan` validates task frontmatter against a JSON Schema. That schema is also the
tool's user-facing documentation of the task format, and it is what editors read
for completion.

There are two ways to hold a schema:

1. **Hand-write it.** It drifts from the code that actually parses configs, and
   the drift is silent until a user files a bug.
2. **Derive it from the types**, and fail CI when the committed artefact and the
   types disagree.

alint hit exactly this problem: 2,180 lines of hand-written JSON Schema that
could silently disagree with the Rust types. Its ADR-0001 resolved it by
deriving the schema from the types via `schemars` and gating with a
regenerate-and-diff check.

## Decision

`aplan_core::task::Task` is the single source of truth. `schemas/v1/task.json` is
generated from it:

```sh
cargo xtask gen-schema           # write
cargo xtask gen-schema --check   # CI gate: non-zero on any diff
```

The loader validates YAML frontmatter against the *derived* schema before
attempting typed deserialisation, so a bad field yields a schema diagnostic
pointing at the JSON path (`/status`) rather than an opaque serde error.

## Consequences

**Good.** The schema cannot lie about the types. Documentation, editor
completion, and validation share one origin. A field added to `Task` fails CI
until the schema is regenerated and committed, the change is forced into the
diff where a reviewer sees it.

**Bad.** `schemars` output is verbose and its shape is not hand-tuned. The
committed schema is generated, so it must never be hand-edited; a contributor who
does so will see CI fail with a diff, which is the intended behaviour but reads
as hostile the first time.

**Note.** This is the tool applying its own thesis to itself: a generated artefact
that is checked, not trusted. The same relationship `aplan` demands between a
task's `status` and its `evidence`.
