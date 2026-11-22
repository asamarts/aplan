# ADR-0071: Live unmatched paths report only provably stranded targets and bare directories

Date: 2026-11-22

Status: Accepted

Tasks: `A2-056`, `A2-101`

Coordinates with `APLAN019` (`A1-138`, `A2-091`), `APLAN053` (`A1-359`), `A2-099`, and `A2-101`.
Supersedes nothing.

## Context

An adopting repository authored a live task with `paths: [tests/test_runner.py]`, mistyping a file
that lived elsewhere. `show`, `check`, and `verify` all passed; the typo surfaced only during
delivery. Because `paths` defines both task ownership and `APLAN012` commit-evidence locality, a
typo in `paths` weakens or breaks later evidence checks.

A separate adopter hit the historical variant (`A2-101`): renaming one source file stranded six live
tasks (and seven terminal tasks that must retain the historical path their closed evidence resolves
against). Because the six live tasks had no evidence yet, `check` and `verify` stayed green until
delivery tried to close them.

At the same time, prior adoption measurement (`A1-138`, pre-public adoption evidence item 24)
established a hard boundary: a `todo` task legitimately declares files and directories it has not
created yet (for example `crates/new_crate/**` or `src/new_module.rs`). Reporting every unmatched
`paths` entry on live tasks produced 140 speculative warnings across 71 `todo` tasks under
`deny_warnings: true` and caused an adopter to decline upgrading. `A2-091` explicitly preserves that
silence for tasks without evidence.

The question for `A2-056` is whether -- and when -- an exact (non-glob) `paths` entry on a live
task that matches no currently tracked file should be reported, consistently with `APLAN053`,
`A2-091`, `A2-099`, and `A2-101`.

## Decision

**Do not warn in `check` or `verify` on a prospective literal path that has never been tracked on a
`todo` or `blocked` task with no evidence. Instead, report early in `verify` only the two live-path
shapes that are provably broken without guessing author intent: (1) a literal path that existed in
Git history and was renamed or removed, and (2) a literal path naming an existing tracked
directory.**

### 1. Why prospective unmatched literal paths remain silent in `check` and `verify`

Given only a `todo` or `blocked` task with `paths: [src/foo.rs]` where `src/foo.rs` has never
existed in the repository, no deterministic rule can distinguish:

- a typo for `src/bar.rs`, from
- a new file `src/foo.rs` that the task is scheduled to create.

Warning on it under `deny_warnings: true` turns forward planning into a build failure, recreating
the exact 140-warning regression `A1-138` removed. Even a single-edit near-miss warning (for
example flagging `src/v2.rs` because `src/v1.rs` exists, or `docs/adr/0069-...` beside
`docs/adr/0068-...`) produces routine false positives whenever a task creates a sibling file.

Prospective literal paths therefore stay silent in `check` and `verify` until either work starts
(`APLAN053`) or evidence is attached (`APLAN019`).

### 2. The two provably broken live-path shapes reported early in `verify` (`A2-101`)

Unlike a never-tracked prospective file, two live-path failures are **facts of the repository**,
not guesses about intent:

1. **Stranded historical literal path.** A literal (non-glob) `paths` entry on a live (`todo`,
   `in_progress`, or `blocked`) task matches no currently tracked file, AND that exact path WAS
   tracked in Git history at or before the task's declaration (for example at the commit that added
   that path to the task, or renamed/deleted in reachable history from that point to `HEAD`). Such a
   path is provably no longer where the task claims it lives -- not a future file waiting to be
   created. `A2-101` reports it in `verify`, surfaces deterministic bounded rename candidates when
   unambiguous (or reports ambiguity rather than guessing), and pairs it with an atomic live-only
   retarget command that refuses to alter terminal tasks.
2. **Bare tracked directory.** A literal (non-glob) `paths` entry on a live task matches no tracked
   file because it names a directory that already contains tracked files (for example `paths: [src]`
   when `src/lib.rs` is tracked). A regular file cannot be created at an existing tracked directory
   path, so the entry is provably missing `/**`. Reporting it early on `todo` and `blocked` tasks
   using the shared `looks_like_a_tracked_directory` predicate (already used by `APLAN019` and
   `APLAN053`) catches the most common `paths` authoring mistake on day one with zero false
   positives.

### 3. Coherent lifecycle across all four path rules

This decision establishes one non-overlapping ladder across the four path rules:

| Lifecycle stage | Command | Rule / task | What it reports | What stays silent |
|---|---|---|---|---|
| Author time (any live non-milestone task) | `check` (hermetic) | Universal catch-all (`A2-099`) | Universal globs (`**`, `*`, `**/*`) that can never bind `APLAN012` commit evidence | Broad-but-non-universal globs and prospective paths |
| Live repository hygiene (`todo`, `in_progress`, `blocked`) | `verify` (Git) | Stranded literal path & bare tracked directory (`A2-101`) | Literal paths whose tracked target was renamed/removed in history, and bare tracked directories (`src` -> `src/**`) | Never-tracked prospective files and prospective globs; terminal historical tasks |
| Work claimed under way (`in_progress`) | `verify` (Git index) | `APLAN053` | `in_progress` task whose entire `paths` set matches nothing tracked | `todo` tasks, or `in_progress` tasks with at least one tracked match |
| Evidence attached (live or `done`) | `verify` (Git) | `APLAN019` (widened per entry by `A2-091`) | Every individual dead `paths` entry on an evidence-bearing task, even beside a live sibling (with historical-commit lookup for `done` tasks) | No-evidence tasks |

### 4. Non-blocking visibility in read-only inspection

Read-only inspection surfaces (`aplan show`, `aplan paths`) may annotate currently unmatched
prospective paths informatively (never as a diagnostic or warning), giving human reviewers a
chance to spot never-tracked typos like `tests/test_runner.py` during task review without turning
prospective planning into a gate failure.

## Consequences

- Planning tasks for files that do not exist yet remains clean under `deny_warnings: true`,
  consistent with `A1-138` and `A2-091`.
- Renaming or deleting a tracked file no longer silently strands live tasks (`A2-101`), while
  terminal tasks keep their historical paths untouched.
- Writing a bare tracked directory (`paths: [src]`) on a `todo` task is caught at `verify` time
  instead of waiting until `aplan start` (`APLAN053`) or `aplan close` (`APLAN019`).
- `A2-101` is the single implementation task for this decision; its acceptance criteria are updated
  to cover both stranded historical literal paths (with live-only retargeting) and bare tracked
  directories on live tasks. No separate duplicate implementation task is needed.

## Alternatives considered

| Alternative | Disposition |
|---|---|
| Warn on every unmatched exact (non-glob) path on `todo` tasks | Rejected: indistinguishable from planned new files; breaks `deny_warnings: true` repositories that plan new files ahead of implementation (the 140-warning failure `A1-138` fixed). |
| Warn on exact paths within edit distance 1-2 of a tracked file | Rejected: flags legitimate new sibling files (`v2.rs` beside `v1.rs`, `0069-...` beside `0068-...`) as typos. |
| Require an explicit `creates:` field separate from `paths:` | Rejected: splits ownership declarations into two lists that drift as soon as a file is created (creating a migration chore on every commit that lands a new file) and adds schema surface for a distinction Git history already answers for renames/deletions. |
| Report nothing at all on `todo` tasks (rely solely on `APLAN053` and `APLAN019`) | Rejected: leaves both file-rename stranding (`A2-101`) and provably wrong bare-directory entries (`paths: [src]`) silent until implementation or closure. |

## References

- `A2-056` (this decision), `A2-091`, `A2-099`, `A2-101`.
- `A1-138` (`APLAN019` evidence-bearing scope), `A1-359` (`APLAN053`).
