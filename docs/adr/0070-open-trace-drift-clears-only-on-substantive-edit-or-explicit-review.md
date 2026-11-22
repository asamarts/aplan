# ADR-0070: Open-task trace drift clears only on a substantive edit or explicit review

Date: 2026-11-22

Status: Accepted

Tasks: `A2-052`, `A2-053`

Extends `APLAN045` (`A1-301`), `ADR-0044` (write-surface admission bar), and `ADR-0069`
(kind-tagged trace stamps). Supersedes nothing.

## Context

`APLAN045` reports when a section traced by a live (`todo`, `in_progress`, or `blocked`) task has
changed since the task was last touched. Its fix text tells the reader that editing the task
records that somebody looked and clears the finding.

Issue #31 demonstrated that the current baseline -- the last Git commit that modified any byte of
the task file, plus skipping whenever the working-tree file differs by any byte from `HEAD` --
treats routine mechanical edits as proof of review:

- `aplan tag add ABC-001 unrelated` clears `APLAN045` (both in the working tree and once committed).
- `aplan start ABC-001` or `aplan stop ABC-001` clears `APLAN045`.
- `aplan mv`, `aplan dep`, `owner` updates, `effort_days` edits, `blocked_by` edits, or bulk
  formatting sweeps clear `APLAN045` across every touched task.

A drift signal that disappears as a side effect of unrelated workflow commands says nothing about
whether anybody re-read the changed specification.

Two complementary mechanisms were proposed in #31:

1. Restrict implicit clearing to commits (and working-tree edits) that modify the task's
   substantive content (`body`, `paths`, `traces`, and `title`).
2. Add an explicit review command (`aplan review <ID>`), the open-task counterpart of
   `aplan close --restamp`, for the legitimate case where somebody re-reads the changed section and
   confirms the open task still holds without needing prose changes.

## Decision

**Adopt both mechanisms composed under one deterministic, tamper-resistant rule: `APLAN045` clears
only on a substantive edit to the task's definition OR on an explicit `aplan review <ID>` stamp
that matches the current traced content.**

### 1. Substantive fields vs non-substantive metadata

Define a task's **substantive content** as exactly four semantic parts:

- `title`
- `body` (trimmed only for leading/trailing blank-line normalization around the frontmatter fence,
  preserving real content edits)
- `paths` (as a normalized list of strings)
- `traces` (as a normalized list of strings)

All other fields are **non-substantive** for `APLAN045`: `status`, `tags`, `owner`, `blocked_by`,
`prefer_after`, `effort_days`, `phase`, `revisit_at`, `version_impact`, `version_impact_reason`,
`ratified_by`, `contains`, `evidence`, `evidence_fingerprints`, as well as frontmatter key
reordering, YAML quoting/spacing churn, and frontmatter comments.

A commit (or uncommitted working-tree edit) moves the task's **substantive baseline** only when at
least one of `title`, `body`, `paths`, or `traces` changed compared to its previous revision.
Tagging, starting, stopping, moving cuts, editing dependencies, changing owner or effort, or
reformatting frontmatter never moves the baseline and never clears `APLAN045` -- whether committed
or sitting uncommitted in the working tree.

### 2. Explicit review command (`aplan review <ID>` and MCP `aplan_review`)

When a traced section changes in a way that leaves an open task still valid verbatim (for example a
clarification or typo fix in the spec), requiring a substantive edit alone would force authors to
make fake prose edits just to clear the warning.

Therefore, introduce **`aplan review <ID>`** (with its MCP peer **`aplan_review`**):

- **Passes `ADR-0044`'s write-surface bar on both clauses:** computing and writing exact content
  and element-kind fingerprints (`ADR-0069`) is an invariant hand-editing cannot maintain (the same
  justification as `aplan close --restamp`), and MCP agents have no text editor.
- **Scope:** accepts only live (`todo`, `in_progress`, `blocked`) tasks that declare at least one
  resolvable anchored trace. Refuses terminal tasks (`done` uses `aplan close --restamp`,
  `dropped` is cancelled) and tasks with no resolvable anchored traces.
- **Storage (no schema break):** records the current resolved kind-tagged fingerprints
  (`heading:<16-hex>`, `row:<16-hex>`, `node:<16-hex>` per `ADR-0069`) in the existing
  `trace_fingerprints` frontmatter field via the surgical YAML writer (`aplan_core::close`),
  guarded by compare-and-swap and the standard post-state validation gate.
  - `APLAN018` and `APLAN028` remain scoped strictly to `status: done`, so live-task
    `trace_fingerprints` creates zero friction in `aplan check`.
  - `aplan close` already replaces any pre-existing live `trace_fingerprints` with close-time stamps
    (`close.replaces_live_trace_fingerprints`).
  - `aplan review <ID>` writes to the working tree and never creates a commit, matching `close`,
    `amend`, `start`, and `stop`.

### 3. Unified clearing predicate for `APLAN045`

For each anchored trace `tr` on a live task during `aplan verify`:

1. **Explicit review stamp check:** if `task.trace_fingerprints.get(tr)` is present, check it
   against the current document at `HEAD` using kind-aware `holds_in` (`ADR-0069`):
   - If the element kind retargeted, emit the blocking retargeting error (`ADR-0069`).
   - If the stamp holds (same kind and matching content hash), `APLAN045` is **cleared** for `tr`.
   - If the stamp does not match (because the traced section moved again after `aplan review`, or
     because someone hand-wrote an invalid value), do **not** clear via the stamp; fall through to
     the substantive-edit check below (and if the baseline also differs, fire `APLAN045`).
2. **Substantive-edit baseline check:** walk the task file's Git history to find the most recent
   commit `rev` in which `title`, `body`, `paths`, or `traces` changed (stopping at the task's
   creation commit if never substantively edited since creation).
   - If the working-tree task file has uncommitted changes to `title`, `body`, `paths`, or `traces`
     relative to `HEAD`, treat the working tree as the current substantive edit (clearing
     `APLAN045` while the author edits the task before committing, preserving `A1-301`'s dogfood
     invariant without letting an uncommitted `aplan start` or `aplan tag add` clear it!).
   - Otherwise, compare the traced section at `rev` against `HEAD`:
     - Kind changed -> blocking retargeting error (`ADR-0069`).
     - Content hash changed -> emit `APLAN045`.
     - Equal -> silent.
3. **Pure function of the repository:** both the substantive-commit walk and `trace_fingerprints`
   verification read only Git history, task files, and traced documents. No clock, no external
   state, and no commit-message convention is consulted.

### 4. Updated fix text

`APLAN045`'s agent instruction and human fix text must state both legitimate remedies clearly:
re-read the traced section against the task; if the task needs changes, update its body, `paths`,
or `traces`; if it still holds as written, run `aplan review <ID>` (never claiming that any touch
to the file clears the warning).

## Consequences

- `aplan tag add/rm/rename`, `aplan start`, `aplan stop`, `aplan mv`, `aplan dep`, `owner` or
  `effort_days` edits, and bulk metadata sweeps never silence `APLAN045`, whether committed or
  uncommitted in the working tree.
- Authors who confirm an open task still holds after a spec edit have an honest, auditable,
  one-command path (`aplan review <ID>` / `aplan_review`) that automatically re-arms `APLAN045`
  the next time the traced section changes.
- Because `APLAN045`'s semantics narrow what counts as clearing an existing rule, `A2-053` must
  register a named `Behaviour` in `crates/aplan-core/src/behaviour.rs` with executable proof tests,
  add `aplan_review` to MCP parity (`crates/aplan/tests/mcp_parity.rs`), update `docs/rule-catalogue.md`
  and `docs/upgrading.md`, and close #31.
- History lookup for `APLAN045` inspects only commits that touched each live traced task file,
  stopping at the first substantive diff; performance stays bounded by the small number of commits
  per live traced task file rather than the whole repository history.

## Alternatives considered

| Alternative | Disposition |
|---|---|
| Substantive edits only, no `aplan review` command | Rejected: forces fake prose or whitespace edits whenever a spec change leaves an open task still valid, training authors to game the gate. |
| Explicit `aplan review` only, ignoring substantive edits | Rejected: forces redundant ceremony when an author genuinely updated the task body, `paths`, or `traces` to reflect the spec change. |
| Record review in a Git commit trailer (e.g. `Reviewed-Trace:`) | Rejected: breaks under squash/rebase, cannot clear in the working tree before commit, and puts plan state outside `aplan/` (violating `ADR-0009`). |
| Add a separate `reviewed_traces` frontmatter field | Rejected: duplicates `trace_fingerprints` with the exact same key/value domain and would require migrating or clearing two maps at `close`. |

## References

- Issue #31, `A2-052`, `A2-053`.
- `ADR-0009` (plan as source of truth), `ADR-0044` (write-surface bar), `ADR-0069` (kind-tagged
  trace stamps).
