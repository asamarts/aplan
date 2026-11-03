# ADR-0051: An active claim is local ephemeral coordination, not task ownership

Date: 2026-10-19
Status: Accepted
Tasks: `A1-531`
Supersedes ADR-0044 only where it stores a claim in task `owner` and describes one task-file
compare-and-swap as repository-wide exclusion. Its write-surface argument and its decision about
`in_progress` remain in force.

## Context

ADR-0044 used `owner` for two different facts: durable responsibility for a task and the actor
working on it now. Real plans use the first meaning. In this repository, and in adopters, owners
are people, roles and teams that remain useful after work ends. Claiming such a task replaced that
fact, `ready --unclaimed` mistook every assigned task for active work, and a takeover destroyed the
assignment it was meant to coordinate.

The exclusion boundary was also smaller than the command promised. A task file is copied into each
linked Git worktree. Two actors in different worktrees compare and swap different files, so both
can be told they won before Git later exposes a merge conflict. P000000000018 measured fifty-five
linked worktrees, making this the ordinary agent topology rather than an edge case.

Finally, the task file supplied neither a claim time nor a truthful activity clock. Writing an
owner refreshed the file whose history `stale` measures, making an abandoned claim initially look
like progress. A holder label inherited from Git can also be a placeholder such as `t` or `root`;
it is attribution, but the output presented it like an authenticated colleague.

## Decision

The terms are distinct:

- **Task `owner`** is durable plan data: a person, role or team responsible for the task. Claim and
  release never edit it. `--owner` and `policy.when.owner` keep this meaning.
- **Claim holder** is the unauthenticated label of the actor working on a live task now. It is local
  ephemeral coordination state, not plan authorship and not evidence that work began.
- **`in_progress`** is the durable plan claim that work began. `start` and `stop` still edit it, and
  `APLAN053` still checks it. Starting does not implicitly claim; claiming does not implicitly
  start.
- **Identity label** is attribution only. Resolution remains deterministic: `--as`, `APLAN_OWNER`,
  per-user `me`, Git `user.name`, then the operating-system user. The record says which source won.
  A common machine or test placeholder produces an actionable warning. None of these authenticates
  a person.
- **Coordination scope** is one canonical Git common directory. Every linked worktree of that local
  repository observes and contends on the same record. The scope is reported by a digest of its
  canonical path; the path itself is not a portable repository identity.
- **Durability** is `local_ephemeral_until_explicit_release`. There is no timeout. A claim survives
  process exit and a worktree's removal until release or an explicit takeover.

The record lives at `<git-common-dir>/aplan/claims-v1.json`. Each claim carries the holder, identity
source, Unix time, host label when available and canonical worktree path when available. The file
is a deterministic bounded JSON document, replaced through the same locked compare-and-swap and
atomic-rename protocol used for plan writes. A changed whole-store snapshot is re-read and a
nonconflicting claim or release is recomputed, so concurrent operations on different task ids
compose while contenders for one id still get one winner. The directory is created atomically at
mode 0700 and a newly created file is mode 0600 on Unix. Existing group- or other-accessible paths,
symlinks, non-regular files and multiply linked files are refused rather than repaired or followed.
Repository content cannot choose this path or executable code.

Every read command that reports or selects claim state uses one snapshot. `ready`, `show`, `stale`
and task JSON expose the scope and complete record; MCP uses the same projections. `--unclaimed`
means no claim in an available snapshot and is independent of `owner`. Corrupt or unsafe state,
or a record naming a task absent from the current worktree, is explicitly unavailable or an error
and never an empty set.
`release <id>` is the manual recovery path and works even when that task is absent or the rest of
the plan does not load. `rekey` refuses a claimed id until release.

A plan outside Git remains readable. Its machine output states that coordination is unavailable;
claim and release refuse because they have no shared local scope, and `--unclaimed` refuses because
unobserved work is not free work. Different hosts do not share this guarantee unless they literally
share the same Git common-directory filesystem with its atomic file semantics. This design is not a
distributed lock, lease, scheduler or identity system.

## Consequences

Linked worktrees get real local mutual exclusion without dirtying a worktree or creating commits.
Claiming cannot refresh a task's staleness clock, and stable ownership remains useful to policy and
reporting while another actor temporarily holds the work.

There is deliberately no silent expiry. Crash recovery is an explicit release or a takeover naming
the displaced holder, so a suspended or long-running actor never loses work because a clock guessed
wrong. The timestamp and origin help a person make that decision but prove neither liveness nor
identity.

Switching to a branch that cannot see a claimed task fails claim-reading operations until the actor
returns to a branch that can see it or releases that id. This is noisier than treating the record as
empty and is the safe direction: duplicate work costs more than an explicit recovery.

The pre-release `owner`-as-claim representation has no automatic migration. Existing owner values
may be durable roles or temporary holders, and guessing would destroy one class to recover the
other. They remain owners. Teams review them under their own vocabulary and explicitly reclaim any
work that is active.
