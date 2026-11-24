# ADR-0073: Task closure reconciles same-actor local claims

Date: 2026-11-23
Status: Accepted
Tasks: `A2-152`
Amends: `ADR-0051` (terminal-transition lifecycle and coordination presentation only; live-work
`local_ephemeral_until_explicit_release` and no-timeout guarantees remain in force)

## Context

`ADR-0051` separated local ephemeral claims (`<git-common-dir>/aplan/claims-v1.json`) from durable
task ownership (`owner`) and intentionally gave live claims `local_ephemeral_until_explicit_release`
durability with no silent timeout. That guarantee protects live work from clock-driven expiry.

In practice, however, closing or dropping a task is not an uncertain clock event: the same
operation has just recorded that work ended. Leaving the closer's own local claim attached to a
terminal task (`done` or `dropped`) caused every linked worktree to display finished work as active
until somebody manually ran `aplan release` for every closed task.

Adopting repositories also surfaced three adjacent coordination defects:

1. **Unclaimed `in_progress` tasks read like fresh ready work.** A live task left `in_progress`
   with no local claim disagrees between its durable status and local coordination, yet `show` and
   `ready` presented it without naming the disagreement or its safe recovery.
2. **Takeover refusals read like store corruption.** Refusing to claim a task held by another
   identity returned `ClaimError::Invalid`, prefixing the message with
   `invalid local claim state at .../claims-v1.json` as though the store were damaged.
3. **Human claim times printed raw Unix epoch integers.** Operators and agents had to convert Unix
   seconds externally to tell whether a claim was seconds or days old.

## Decision

### 1. Post-publication terminal claim reconciliation

Task publication (`status: done` or `status: dropped`) and local claim cleanup operate on two
separate stores (the task Markdown file and `<git-common-dir>/aplan/claims-v1.json`) and cannot be
one atomic filesystem write. Plan publication always commits first and is **never** rolled back if
coordination cleanup fails.

Immediately after a task transitions to `done` (`close`, `close --execute`, `aplan_close`, TUI
close) or `dropped` (`drop`), every delivery surface runs one shared application reconciliation
helper (`reconcile_terminal_claim`) with the resolved acting identity (if available):

- **No claim present**: no-op (`none`).
- **Same actor holds the claim** (`record.holder == actor`): compare-and-swap releases that exact
  `ClaimRecord` and reports `released`.
- **Another actor holds the claim** (`record.holder != actor`): never silently removed. Reports
  `retained_other_holder` naming the holder and safe manual recovery (`aplan release <id>`).
- **Actor identity unknown**: never guessed or silently removed. Reports `retained_unknown_actor`
  with safe manual recovery (`aplan release <id>`).
- **Concurrent takeover or release race**: if the claim store moves between inspection and CAS
  write, reconciliation re-reads the current record. A concurrent release resolves cleanly to
  `none`; a concurrent takeover by another actor resolves to `retained_other_holder`.
- **Unavailable store (non-Git repository)**: reports `unavailable`; plan closure succeeds.
- **Unsafe, corrupt, or unwritable store**: plan closure remains published and truthful. Reports
  `cleanup_failed` with the exact store error and recovery guidance so an idempotent `aplan release
  <id>` can complete cleanup once the store is healthy.

Live tasks keep `ADR-0051`'s `local_ephemeral_until_explicit_release` durability with no timeout.

### 2. Explicit coordination lifecycle states on read surfaces

Read projections (`show`, `ready`, `tasks`, `stale`, MCP, and TUI) classify each task's
status-versus-claim relationship into one shared typed state (`ClaimCoordinationState`):

- `active`: live task held by an active local claim.
- `stale_terminal`: terminal task (`done` or `dropped`) still carrying a local claim (from an older
  build, another actor, or failed cleanup). Retains the complete unauthenticated `ClaimRecord` and
  names `aplan release <id>` for recovery.
- `in_progress_unclaimed`: live `in_progress` task with no active local claim when coordination is
  available. Surfaces never present it as fresh ready work; human, JSON, MCP, and TUI surfaces name
  the status/coordination disagreement and safe recovery (`aplan claim <id>` or `aplan stop <id>`).
- `unclaimed`: ordinary live or terminal task with no claim.
- `unavailable`: no shared local coordination scope is available.

### 3. Ordinary typed takeover refusals

A valid claim held by another identity (or a mismatched `--from` takeover) returns a dedicated
typed refusal (`ClaimError::Conflict`) instead of `ClaimError::Invalid`. Its message names the
current holder and the exact `--from <holder>` takeover or `aplan release <id>` recovery without
claiming the store file is invalid or corrupt.

### 4. Human UTC timestamps and stable age

Human-facing claim output prints both an absolute UTC ISO-8601 instant (`YYYY-MM-DDTHH:MM:SSZ`)
and a human-readable relative age (`just now`, `17s ago`, `5m ago`, `3h 12m ago`, `2d 4h ago`, or
`in the future by <duration> (clock skew)` when `claimed_at > observed_at`). Machine-readable JSON
and MCP payloads retain the exact integer `claimed_at_epoch_seconds`,
`claim_observed_at_epoch_seconds`, and `claim_age_seconds` fields.

## Consequences

- Linked worktrees immediately see finished work as released when the claimant closes or drops it,
  while another actor's claim is never silently destroyed.
- Partial local cleanup after a committed plan write is explicit and recoverable rather than silent
  or misrepresented as a failed closure.
- Unclaimed `in_progress` tasks and stale terminal claims are both surfaced clearly across CLI,
  MCP, and TUI.
- Takeover refusals are distinguishable from store corruption both programmatically (`ClaimError::Conflict` vs `ClaimError::Invalid`) and in human diagnostics.
