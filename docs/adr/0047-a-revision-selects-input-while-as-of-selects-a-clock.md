# ADR-0047: A revision selects input while as-of selects a clock

Date: 2026-10-10
Status: Accepted
Tasks: `A1-194`

## Context

Most reporting commands read task files and `.aplan.yml` from the working tree. Before this
decision, asking what a plan looked like at a release required checking out that release or making
a second worktree. `attest` was the only whole-plan answer that named a commit, file evidence could
name a revision for one citation, and `stale --as-of` could name a date. Those are three different
questions: which plan was read, which artifact a citation names, and which clock an age calculation
uses.

A revision-qualified report has two repository-owned inputs. The task files are the obvious one.
The discovered `.aplan.yml` is the less obvious one: it can change the plan root, throughput, and
summary grouping. Reading old tasks under today's discovered config would produce a hybrid state
that never existed. Conversely, `--config FILE` is a caller-selected trust boundary and
`--no-config` is an explicit refusal to read repository policy. A revision flag must not silently
replace either decision.

The first useful report is `summary`: it has stable human and JSON projections, exposes config
effects, and makes release-to-release trend collection possible without first defining a semantic
diff contract. This is also narrow enough to prove the repository and loader seams before every
reporting command acquires the option.

## Decision

**1. Revision selection is a report option, spelled `--at <REV>`.** The first surface is `aplan
summary --at <REV>`. Other read-only reporting commands may adopt the same spelling through the
shared snapshot seam. It is not a global flag: mutating commands and gates have different
questions about ambient state, and pretending they all support historical execution would make an
option appear broader than it is.

**2. A revision resolves once to a full commit id.** Task and discovered-config reads use that
immutable id, never the moving ref again. The repository is not checked out or mutated. The
resolved full id is written to stderr, leaving the report on stdout byte-compatible with the
ordinary command. On a clean checkout, `summary --at HEAD` and ordinary `summary` have identical
stdout.

**3. Discovery reads configuration from the selected tree.** It walks from the invocation's
repository-relative working directory toward that repository's root, using the nearest committed
`.aplan.yml`, then resolves its `root` relative to that file. A historical root need not exist in
the current worktree. If the caller supplied `--root`, that location wins as it does ordinarily.

**4. Explicit config selection is not revision selection.** `--config FILE` keeps reading exactly
that current caller-selected file, including its root and view policy. `--no-config` keeps reading
none. This makes mixed provenance visible and intentional: stderr identifies the plan commit,
while the invocation identifies an external policy. A future identified context may digest both;
this command does not imply an external file is part of the named commit.

**5. A revision and `--as-of` remain separate concepts.** `--at` selects immutable repository
input. `--as-of` selects an evaluation clock for a report whose result depends on time. A future
age report may accept both: for example, evaluate the plan stored at one commit as of a later date.
Folding them together would make a commit timestamp stand in for a caller's review date and would
make a date pretend to identify task/config bytes.

**6. Incomplete historical answers are refused.** An unknown revision, a selected tree without
the plan root, invalid config bytes, or any task file that fails the ordinary loader produces no
rollup. A plan loaded from repository bytes uses the same candidate, UTF-8, frontmatter, schema,
and diagnostic logic as a filesystem plan. An empty plan directory is distinct from a missing one.
Git path bytes are preserved where the host can represent them, and a task/config symlink follows
the link committed in the selected tree rather than the target in today's checkout. A discovered
config above the repository is neither current caller-selected policy nor content at the commit,
so `--at` refuses it until the caller says `--config` or `--no-config` explicitly.
A submodule beneath the selected plan root is also refused: its files are not blobs in the
superproject commit, so omitting a task it may contain would turn an incomplete rollup into a
complete-looking one.

## Consequences

- Release and trend tooling can collect stable summary documents from tags or commits without
  changing the checkout.
- The repository facade owns revision resolution and exact blob reads; `aplan-core` remains free
  of Git and loads an ordered set of path/byte pairs through the same task parser.
- Historical config errors name the committed file that caused them. Today's broken or missing
  worktree config cannot prevent a discovered historical summary from reading an older valid one.
- `summary --at` does not yet provide task-by-task comparison. The next experiment can extend an
  existing stable projection, then measure whether a semantic diff belongs in core.
- A repository outside the invocation's current repository cannot supply revision-discovered
  config. Callers reviewing another trust domain should select `--config` or `--no-config`
  explicitly, which was already the safe review posture.

## Alternatives

| option | why not |
|---|---|
| Check out the revision in place | Mutates the user's worktree, conflicts with dirty files, and makes a read-only report unsafe to automate |
| Create a temporary worktree | Pays filesystem and cleanup cost, runs discovery through ambient state, and duplicates Git lifecycle code for data already available as blobs |
| Read old tasks under current discovered config | Produces a state that may never have existed and makes grouping, throughput, and even the plan root time-inconsistent |
| Make `at` a wrapper command around another command | Requires a second command grammar or subprocess handoff and obscures which surfaces actually support historical input |
| Treat a revision as `as-of` | Confuses content identity with time; the two need to compose rather than alias each other |
| Add a semantic diff now | Commits to classification and output contracts before revision projections reveal which differences actually require domain knowledge |
