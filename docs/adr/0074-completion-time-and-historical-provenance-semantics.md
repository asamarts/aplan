# ADR-0074: Completion-time and historical-provenance semantics

Date: 2026-11-24
Status: Accepted
Tasks: `A2-108`
Related: `A2-063`, `A2-109`, `A2-110`, `A2-111`, `A2-112`, `A2-119`

## Context

Adopters frequently ask "what were the last N tasks we completed?" or want a chronological slice of
finished work by cut, phase, owner, or tag. Today a task records `status: done` and its `evidence`,
but carries no completion instant.

A timestamp can mean at least four different things:

1. When the underlying real-world engineering work occurred or finished.
2. When a person or agent claimed it was done.
3. When `aplan` accepted the closure claim and wrote `status: done`.
4. When Git first observed a committed `status: done` transition in reachable history.

Collapsing those meanings would make a precise-looking recent-completions report less trustworthy
than returning `unknown`. Existing repositories also hold hundreds of `done` tasks with no recorded
timestamp, and Git history in the wild includes shallow clones, merges, reverts, repeated
transitions, skewed committer clocks, renames, rekeys, and non-Git directories.

This ADR locks the durable data model, provenance taxonomy, Git history inference contract,
deterministic report ordering, and shared CLI/MCP/TUI query semantics before `A2-109`, `A2-110`,
`A2-111`, `A2-112`, and `A2-119` implement them.

## Decision

### 1. Meaning of `closed_at`

`closed_at` is an optional task frontmatter field meaning:

> **The instant this task's transition to `done` was accepted and recorded by `aplan`.**

It is a lifecycle claim in the plan (like `status`), **not** evidence of when real-world work
happened or finished. Evidence remains the proof of work.

`closed_at` is strictly separate from measured focus time, calendar elapsed duration, and external
wait time (`A2-063`). Neither can be derived honestly from the other, and this decision does not
settle `A2-063`.

### 2. Storage encoding, precision, normalization, and provenance

Two optional frontmatter fields carry stored completion-time state:

```yaml
status: done
closed_at: 2026-10-11T19:24:08Z
# Optional; omitted (default) for local `aplan close`, required `imported` when preserved on import:
closed_at_source: imported
```

- **Canonical encoding**: RFC 3339 UTC instant at whole-second precision (`YYYY-MM-DDTHH:MM:SSZ`).
  Sub-second fractions are truncated to whole seconds when sampled on write.
- **Validation**: Stored `closed_at` values must be valid canonical RFC 3339 UTC second-precision
  calendar instants in the range `1970-01-01T00:00:00Z` through `9999-12-31T23:59:59Z`. Impossible
  calendar dates (such as February 30, month 13, or fabricated leap-second `:60`), pre-epoch
  instants, non-UTC offsets in persisted task files, and unparseable strings are rejected by a
  validation rule.
- **No backdating on `close`**: Ordinary `aplan close` (CLI, MCP, and TUI) never accepts a
  caller-supplied timestamp or backdating flag. `--at` remains exclusively the historical revision
  selector on read/report commands and never becomes a close-time flag.
- **Durable local vs. imported provenance**:
  - Local `aplan close` writes `closed_at: <UTC>` and omits `closed_at_source` (or reads absent
    `closed_at_source` alongside `closed_at` as `local`).
  - An explicit import path that preserves a source timestamp must validate the input (normalizing
    valid numeric offsets and sub-second precision to canonical UTC whole seconds), reject invalid
    or out-of-range instants, and durably write `closed_at_source: imported`. An imported instant
    can never become an indistinguishable local record on subsequent reads.
  - If an importer cannot validate a source timestamp or none exists, it must omit both fields
    (leaving the task `unknown`) rather than inventing an instant, unless the import explicitly
    executes a local closure transition (which records the local clock as `local`).
- **Invalid field combinations rejected by validation**:
  - `closed_at` present on any task whose `status` is not `done` (`todo`, `in_progress`, `dropped`).
  - `closed_at_source` present when `closed_at` is absent.
  - Any `closed_at_source` value other than `local` or `imported` (`history_first_seen` and
    `unknown` are never persisted in task frontmatter).

### 3. Lifecycle transition dispositions

Every code path that constructs, mutates, or imports a task has an explicit disposition:

- **Successful `close` (ordinary task and milestone -> `done`)**: Samples the clock (real or
  injected in tests) once inside the guarded mutation immediately before atomic replacement, after
  all validation, evidence, ratification, and precondition gates pass. Writes `status: done`,
  `evidence`, trace/file stamps, any release `version`, and `closed_at` in one atomic replacement.
- **Refused `close`**: Writes nothing; no timestamp is sampled into storage.
- **Snapshot-contention retry on `close`**: On retry after concurrent snapshot contention, the
  retry attempt re-samples the clock when building the winning replacement so only the successfully
  published transition introduces `closed_at`.
- **`drop` (`status: dropped`)**: Dropping work is not completing it. `drop` never populates
  `closed_at` (and clears any invalid `closed_at` if a rejected state is ever repaired).
- **`amend` and `restamp` on a `done` task**: Preserve the existing `closed_at` and
  `closed_at_source` verbatim. If a legacy `done` task has no `closed_at`, `amend` and `restamp`
  leave it absent and never invent one.
- **Supported transition out of `done` (reopen)**: Clears `closed_at` and `closed_at_source`
  atomically because those fields are valid only on `done`.
- **Legacy and manually edited `done` tasks**: Existing `done` tasks without `closed_at` remain
  100% valid with `unknown` provenance (no bulk migration or invented timestamps). A valid manual
  edit to a `done` task is validated by the same schema and rules as any other frontmatter claim.

### 4. Milestones vs. completed work

Milestones (`tags: [milestone]`) may record `closed_at` when closed to `done`, because knowing when
a cut or release checkpoint closed is useful metadata on the milestone itself.

However, milestones are release/cut checkpoints, not completed work units, and `dropped` tasks are
abandoned work. The default completed-task view (`aplan completed`, MCP completed resource, and TUI
completed list) **excludes milestones and excludes `dropped` tasks** so release checkpoints and
dropped work are never silently mixed into completed work.

### 5. Typed provenance taxonomy

Every typed completion-history item exposes three non-overlapping provenance discriminators that
cannot be confused in storage or output:

| `source` | `recorded_origin` | `commit` | `instant` | Meaning |
| --- | --- | --- | --- | --- |
| `recorded` | `local` \| `imported` | `null` | RFC 3339 UTC | Exact persisted `closed_at` claim, durably distinguishing local close from preserved import |
| `history_first_seen` | `null` | full commit OID | RFC 3339 UTC | Best-effort selected committed observation of a transition to `done`; never persisted as `closed_at` |
| `unknown` | `null` | `null` | `null` | No defensible instant is available |

Recorded values always take precedence over Git history inference.

### 6. Git history inference semantics

When a legacy `done` task lacks `closed_at` and history inference is permitted:

- **Single bounded walk**: Traverse commits reachable from the named `basis_revision` touching the
  plan root in one pass (`O(commits)`), never once per task (`O(tasks * commits)`). Historical task
  blobs are parsed as untrusted repository data without executing configuration or following paths
  outside the repository.
- **Transition candidate definition**: A commit `C` is a transition candidate for task `T` iff `T`
  has `status: done` in `C` and either `C` has no parents (root commit) or `T` was absent / had
  `status != done` in at least one parent `P` of `C` -- subject to the merge rule below.
- **Merge commits**: If `T` already had `status: done` in **any** parent of a merge commit `M`,
  then `M` is **not** a transition candidate (the transition occurred on an ancestor branch). `M` is
  a candidate only when `T` was not `done` in **all** parents of `M` (the merge itself introduced
  `status: done`).
- **Reverts and repeated transitions**: Walk backwards from `basis_revision` along the contiguous
  reachable subgraph where `T` remains `done`. The transition candidates for `T`'s **current**
  completion episode are the minimal (earliest) commits in that contiguous `done` component whose
  parent(s) were non-`done` or absent. An earlier `done` episode separated by a revert or reopen
  does not hijack the current episode.
- **Commit-time choice**: Report the selected commit's **committer timestamp** normalized to UTC
  whole seconds (not author timestamp), because committer time reflects when the commit entered the
  repository's history. Clock skew is never rewritten or fabricated against parent timestamps; the
  selected commit OID is always returned alongside the timestamp for inspection.
- **Renames and rekeys**: Track tasks by parsed frontmatter `id` across paths under the plan root
  so directory moves and file renames preserve identity. If a task was rekeyed (`OLD -> NEW`), follow
  the predecessor only when the repository supplies unambiguous rename/rekey evidence; otherwise
  treat the `NEW` appearance commit as the candidate rather than guessing cross-ID lineage.
- **Incomparable candidates and deterministic ties**: When multiple minimal transition candidates
  exist on incomparable DAG branches (for example, parallel branches independently transitioning
  `T` to `done` before merging), select deterministically by:
  1. earliest committer timestamp (`min(committer_time)`),
  2. lexicographically smallest full commit OID (`min(commit_oid)`).
  Record `incomparable_candidates` metadata on the item so consumers see that the DAG had multiple
  incomparable candidates rather than one linear origin.
- **Uncommitted, deleted, and unparseable cases**:
  - A `done` task present only in the working tree (untracked or still non-`done` at `HEAD`) has no
    committed transition and stays `unknown`.
  - Deleted paths do not reappear unless present in the basis snapshot.
  - Historical task files that fail to parse (unsupported older shape) are reported as an
    incompleteness reason (`unparseable_historical_files`) rather than silently omitted.
- **No-repository / empty-repository / shallow clones**: Non-Git directories and empty repositories
  return `unknown` for legacy tasks with explicit `unavailable` status. Shallow clones report
  `incomplete_shallow` whenever a legacy task's contiguous `done` walk reaches a shallow graft
  boundary without finding a non-`done` parent.

### 7. History policy and scoped completeness

Every completion-history query carries an explicit `HistoryPolicy`:

- `auto` (default): Consult Git history only if at least one in-scope `done` task lacks `closed_at`.
  If all in-scope tasks have `recorded` values, skip Git traversal and report `complete`. If Git
  history is shallow, bounded, unobservable, or unavailable, return best-effort results with
  explicit non-complete status and limitation reasons.
- `never`: Never consult Git history. Fast, hermetic, repository-independent; legacy tasks without
  `closed_at` remain `unknown` with completeness state `history_skipped`.
- `required`: Require complete reachable history from `basis_revision` whenever any in-scope task
  lacks `closed_at`. If Git is unavailable, shallow, capped by traversal budgets, or encounters
  unparseable historical task shapes, refuse with a typed error rather than returning a partial
  historical answer.

Every response names its `basis_revision` (the resolved commit OID, `working_tree@<oid>`, or
`unversioned`). Completeness is **always scoped to history reachable from `basis_revision`** and
never claims that rebased, force-pushed, or unreachable history never existed:

- `complete`: All in-scope tasks resolved from recorded claims and/or exhaustive reachable history
  from `basis_revision`.
- `history_skipped`: `HistoryPolicy::Never` skipped Git inference while legacy tasks were present.
- `incomplete_shallow`: Traversal hit a shallow-clone boundary before resolving all legacy tasks.
- `incomplete_bounded`: Traversal hit explicit commit, work, or memory budgets before resolving all
  legacy tasks. Early stopping is legal only when the requested filters, order, and limit make the
  returned slice provably complete.
- `incomplete_unobservable`: Unparseable historical task files or missing objects prevented full
  observation.
- `unavailable`: No Git repository or no commits exist at `basis_revision`.

### 8. Deterministic report ordering (non-causal)

Completed tasks are ordered deterministically by `(known_before_unknown, instant, task_id)`:

- **Default (`newest-first`)**: known instants first, `instant DESC`, then `task_id ASC`.
- **Reversed (`--oldest`)**: known instants first, `instant ASC`, then `task_id ASC`.
- **Unknown instants**: Always sort **after** all known instants in both newest-first and
  oldest-first views, ordered by `task_id ASC`, so `--oldest` never presents an unknown timestamp as
  falsely oldest.

This `(instant, task_id)` order is a **deterministic report order, not an exact causal sequence**
across same-second closes, skewed clocks, or concurrent Git branches. Exact causal or
multi-transition event history belongs to a future event-log design and is explicitly not smuggled
into `closed_at`.

### 9. Shared query, time-bound, limit, and pagination contract (CLI, MCP, TUI)

CLI (`aplan completed`), MCP bounded completion resources, and TUI completion views share one typed
application request/response contract:

- **Filter-before-limit**: Selector filters (`cut`, `phase`, `owner`, `tag`) and time bounds
  (`since`, `until`) apply before `limit` and pagination.
- **Time-bound inclusivity**: Both bounds are closed/inclusive (`since <= instant <= until`).
  Date-only inputs `YYYY-MM-DD` normalize to `YYYY-MM-DDT00:00:00Z` for `since` and
  `YYYY-MM-DDT23:59:59Z` for `until`; full RFC 3339 inputs normalize to UTC whole seconds.
- **Unknowns and time bounds**: Tasks with `unknown` instant never satisfy a `since` or `until`
  predicate.
- **Invalid inputs refused**: `limit == 0`, reversed bounds (`since > until`), and unparseable
  instants are refused with a clear typed error.
- **Revision selection (`--at`)**: Existing report `--at <rev>` reads task data, input identity,
  and Git history basis from that same revision; it never mixes current working-tree data with
  historical commits or acts as a close timestamp.
- **Snapshot-consistent pagination**: Continuation cursors are opaque, strictly parsed,
  size-bounded, and bound to `(input_identity, basis_revision, query_fingerprint, last_sort_key)`.
  If `input_identity` or `basis_revision` changes between pages, continuation is refused as stale
  rather than duplicating or omitting rows across equal or unknown timestamps.

### 10. Compatibility and migration

- Existing plans and `done` tasks without `closed_at` remain completely valid; no schema migration
  or bulk file rewrite is required.
- Legacy `done` tasks report `unknown` under `HistoryPolicy::Never` and best-effort
  `history_first_seen` under `HistoryPolicy::Auto` / `Required`.
- Adding `closed_at` on `aplan close` changes what an existing command writes and therefore requires
  a named `Behaviour` and executable proof test when implemented in `A2-109`.

## Consequences

- `A2-109`, `A2-110`, `A2-111`, `A2-112`, and `A2-119` share one unambiguous storage, provenance,
  history-walk, ordering, and pagination contract across CLI, MCP, and TUI.
- Legacy repositories upgrade with zero churn and zero fabricated timestamps.
- Completion chronology remains honest about what `aplan` actually observed and stays cleanly
  decoupled from measured work duration (`A2-063`) and any future multi-event audit log.
