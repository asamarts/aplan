# ADR-0061: Focused L3 closure is an explicit preview-bound mutation

Date: 2026-10-29

Status: accepted

Answers `A1-705`. Supersedes ADR-0043 only for an explicitly requested focused test
execution. Its refusal of triggers, agent-initiated effects and implicit close-time work remains
unchanged.

## Context

ADR-0043 made ordinary `close` inert outside plan writes. It may ask `test.list` which identities
exist, but it does not run one. That is an honest L2 answer: the citation resolves and is relevant
to the task. It is not the L3 claim that the cited test currently passes.

P00002 exposed the missing authoring-time rung twice. `AM-384` closed on
`checks::verify::28-the-record-counts-what-is-there` while the check compared a recorded 402 with
403; `AM-364` closed in the same window shape at 376 against 380. Whole-plan `verify --execute`
would catch both, but it replays every cited test in the plan. That is too broad for the edit-close
loop and makes the safe operation expensive in proportion to historical evidence rather than the
claim being authored.

This is not permission for a task file to trigger work. The user must choose the execution at the
command or interactive boundary, trusted configuration still chooses what program can run, and a
failed or stale observation must publish no plan bytes.

## Decision

`aplan close ID --evidence test:NAME --execute` is the focused L3 closure operation.

1. **The boundary is explicit and off by default.** Without `--execute`, close retains its L2
   listing behavior. `--execute` conflicts with `--no-resolve` and `--restamp`, and requires at
   least one explicit `test` citation. The TUI asks separately whether to run cited tests and names
   `test.run`/`test.batch` before Preview starts them. MCP `aplan_close` remains L2-only and has no
   execution parameter.

2. **The proposal defines the execution set.** Resolution builds a one-task post-close plan from
   the proposed task bytes and only those proposed citations. Each unambiguous runner/listed-name
   pair executes once, even when the citation is repeated. Evidence already recorded by this or
   any other task is absent from the execution plan and cannot run.

3. **Only a typed unconditional pass closes.** Every proposed `test` citation must produce a
   `CitationResolution` whose strength is `Executed` and whose outcome is `Resolved`. Failed,
   skipped, missing, ambiguous, non-selecting, timed-out, resource-stopped, output-limited,
   framework-crashed, incomplete and unlaunchable work is not L3. Warning policy cannot upgrade
   one of those outcomes. Human and interactive output say L2 or L3 rather than using an
   unqualified "resolved" for both.

4. **Execution and publication share one stale boundary.** The application preview binds exact
   proposed plan bytes, repository HEAD plus changed worktree material, selected configuration
   bytes and semantics, and the selected providers' definitions, inherited environment,
   executable/interpreter/file-argument bytes and applicable toolchain state. The selected
   provider material is measured before execution, after execution and immediately before the
   guarded write. Movement returns a stale refusal; it never silently reruns or attaches old proof
   to new bytes.

5. **The ordinary mutation gates still decide publication.** Minimum evidence strength,
   exemptions, path relevance, trace and mutable-evidence stamps, post-state diagnostics,
   compare-and-swap file replacement and concurrent writers are unchanged. Execution produces a
   preview; it is not authority to bypass a denied post-state.

## Why this does not restore triggers

ADR-0043 refused `on_close.command`: repository content would decide that an effect occurs, MCP
could cause it, and the effect could outlive or exceed the plan mutation. Focused L3 has the
opposite properties. A person explicitly chooses it for one invocation; the only admitted effect
is observing the exact evidence claim being proposed; failure prevents the mutation; and no
agent-facing close tool can request it. It does not publish, release, notify or invoke an arbitrary
post-close action.

Shell composition remains the answer for downstream effects. A successful focused close can be
followed by another command with `&&`, where the caller, review and automation policy can see the
second effect.

## Consequences

The edit-close loop can prove the strongest test rung without replaying historical evidence.
Repositories with expensive suites can use focused L3 where the cited runner supports exact
selection or a complete typed batch result.

The operation may refuse after a passing test when source, configuration or provider material
moved. That extra run is intentionally discarded: attaching it to changed inputs would turn proof
into a race.

Configured tests are repository-trusted code. Review untrusted changes and select trusted config
before opting in. Ordinary CLI close, MCP close and non-executing TUI previews retain the inert
boundary.
