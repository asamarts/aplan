# aplan

**A plan that cannot quietly lie to you.**

`aplan` validates a plan the way a linter validates code. Tasks are markdown
files with YAML frontmatter, committed next to the work. `aplan check` refuses a
plan whose dependency graph has a cycle, whose tasks reference blockers that do
not exist, or (the point of the whole exercise) whose tasks claim `status:
done` without a shred of evidence.

```console
$ aplan check
error[APLAN004] P1-002: dependency cycle: P1-002 → P1-004 → P1-002
  --> aplan/tasks/P1-002.md
error[APLAN005] P0-003: `status: done` with no `evidence`: a status is a claim, not a proof
  --> aplan/tasks/P0-003.md
failed: 2 error(s), 0 warning(s) across 34 tasks

$ echo $?
1
```

## Why it exists

Every tracker lets you type `done`. None of them ask *how you know*.

Drift is not a storage problem, so it is not solved by moving your plan into git,
into Jira, or into markdown. A committed history records **what was claimed**,
not proof the claim was true. The only mechanism that closes the gap is to make
status a **falsifiable assertion**, and then run a machine over it.

So `aplan` enforces a set of always-on invariants (`aplan facts` prints the authoritative
list, generated from the code, this README does not hand-count them, which is the drift the
tool exists to catch). Most are errors that fail CI; `APLAN009` and `APLAN021` are warnings:

| Code | Invariant |
|------|-----------|
| `APLAN001` | Frontmatter satisfies the schema derived from the Rust types |
| `APLAN002` | Task ids are unique |
| `APLAN003` | `blocked_by` references a task that exists |
| `APLAN004` | The dependency graph is acyclic |
| `APLAN005` | **`done` carries at least one piece of evidence** |
| `APLAN006` | No task blocks itself |
| `APLAN007` | A `done` task does not depend on an unfinished one |
| `APLAN008` | `traces` anchors resolve to real headings in real documents |
| `APLAN009` | Active tasks carry an effort estimate *(warning)* |
| `APLAN010` | A `note` used as evidence carries a date |
| `APLAN020` | A live task is not waiting on a `dropped` one |
| `APLAN021` | A `milestone` task has members and carries no effort or paths *(warning)* |

`APLAN020` catches a plan that is *deadlocked*: a task still `todo` whose blocker
was abandoned can never legitimately start, and every tracker will happily let it
sit there forever. `APLAN021` keeps the `milestone` tag honest (milestones group a
release cut; see [self-hosting](#self-hosting)). (`APLAN011`-`APLAN015` are the
evidence-ladder rules below, run by `aplan verify`, not `check`; `APLAN017`,
`APLAN018`, and `APLAN022` are the opt-in rules below; `APLAN016` (url reachability)
and `APLAN019` (`paths` globs match a tracked file) are planned. A rule code is a
contract, so it is not renumbered for tidiness.)

Three more `check` rules are **opt-in**, off unless `.aplan.yml` turns them on.
`APLAN017` requires a `done` task's evidence to meet a configured minimum strength,
so a real project can decree that a bare `note` does not close a task the way a
`commit` or a `test` does. `APLAN018` detects *stale-`done` drift*: `aplan close`
stamps a content fingerprint of each traced section, and `check` warns (advisory)
when a closed task's traced section changes afterward, cleared by re-reviewing and
`aplan close --restamp`. `APLAN022` requires every task to be a member of exactly one
release cut (or tagged `cross-cutting`), so a milestone's `done` proves the cut is
exhaustively complete. See [Configuration](#configuration).

`APLAN008` is the one people underestimate. A task may declare the passage of a
document it is supposed to resolve:

```yaml
traces:
  - INITIAL_RESEARCH.md#delaware-icis-access
```

The anchor must exist. Closing the task obliges you to go edit that section. It
is the link that stops a research document and a plan from silently diverging.
Anchors resolve against the current directory by default; `--no-traces` turns the
rule off, but nothing turns it off *for* you.

## What counts as evidence

Not "I finished it."

| Kind | Meaning |
|------|---------|
| `commit` | A git SHA in this repository |
| `url` | An external URL, a filing receipt, a regulator page, a vendor quote |
| `file` | A repo-relative path to a committed artefact |
| `test` | A named test that must pass |
| `note` | A dated human note. The weakest kind; requires `captured:` |

An undated note is a rumour, so `APLAN010` rejects it. `commit`, `file`, and
`test` are the kinds `aplan verify` resolves against the repository (below); the
rest are checked structurally.

## The verification ladder

`check` asks whether evidence is *present*, a real field, well-formed. It does
not ask whether the evidence is *true*, because that means opening the
repository, and `check` is deliberately hermetic (no git, no network, no
subprocess) so it can run on every keystroke and in a pre-commit hook. That
property is tested, not hoped for.

`aplan verify` opens the repository and resolves the evidence:

| Rung | Question | Command |
|------|----------|---------|
| **L0** Asserted | Is the evidence present and well-formed? | `aplan check` |
| **L1** Resolved | Does the referent *exist*? | `aplan verify` |
| **L2** Relevant | Does it connect to *this* task? | `aplan verify` |
| **L3** Executed | Does the cited test *pass*, now? | `aplan verify --execute` |

A `commit` SHA must resolve to a real commit (`APLAN011`), and its diff must
touch at least one of the task's declared `paths` (`APLAN012`); a `file` must
exist at the cited rev, or at `HEAD` (`APLAN015`); a `test` must be one the
configured runner lists (`APLAN013`), and under `--execute` it must pass
(`APLAN014`).

`APLAN012` is the one that matters. A careless or dishonest agent cites a real
commit that has nothing to do with the task, so a tool that only checks the
field is non-empty learns nothing. `paths` turns the citation into a
*falsifiable* claim:

```yaml
paths: [crates/aplan-core/**]
evidence:
  - kind: commit
    ref: 6d5f410
```

Now closing the task with that commit is checkable: `verify` fails if the commit
touches nothing under `crates/aplan-core/`.

Running a `test` (rung L3) is non-deterministic, so it is opt-in (`--execute`,
off by default). It is also the one place `verify` runs a command, so it is
security-critical: a task file names only a *test*, never a command. The name is
substituted, as a whole argument, into a runner template that lives only in the
trusted top-level `.aplan.yml`, and it is spawned with no shell, so a task file
arriving in a fork pull request can never inject a command:

```yaml
# .aplan.yml
test:
  list: ["cargo", "test", "--", "--list"]
  run:  ["cargo", "test", "--exact", "{test}"]
```

CI must not pass `--execute` for fork pull requests (see [`SECURITY.md`](SECURITY.md)).

## A task

```markdown
---
id: P0-003
title: Confirm Delaware ICIS XML filing access
phase: 0
status: in_progress
owner: kaminsod
effort_days: 3
blocked_by: [P0-001]
traces:
  - INITIAL_RESEARCH.md#21-delaware-the-filing-channel
tags: [delaware, blocking]
---

## Why

The ICIS entity-filing channel rests on a 2007 RFP and a vendor integration doc.
It is the load-bearing assumption of the entire US build.

## Acceptance criteria

- [ ] Written confirmation of the access model from the Division of Corporations
- [ ] Agent Number application process documented
```

## Commands

```sh
aplan check                    # validate structure, hermetically; exit 1 on any error
aplan check --format agent     # every diagnostic carries an agent_instruction
aplan check --format github    # inline PR annotations
aplan check --no-traces        # skip APLAN008 (opt out; never silent)
aplan verify                   # resolve evidence against the git repo (L1/L2)
aplan verify --execute         # also run cited tests (L3; off by default)
aplan ready                    # workable tasks whose blockers are all done
aplan why P1-003               # transitive explanation of what blocks a task
aplan summary                  # tasks, effort, and ready count rolled up by phase
aplan summary --format json    # the same, machine-readable
aplan critical-path            # the longest effort-weighted chain of dependent tasks
aplan roadmap                  # the plan grouped by cut: rollups + per-cut critical path
aplan roadmap --format markdown # the committed ROADMAP.status.md companion (ADR-0009)
aplan close P1-003             # set done + attach the HEAD commit, in place
aplan close P1-003 --evidence commit:d213c9a --evidence test:mod::name
aplan import kep.yaml          # read a Kubernetes KEP into a task (lossy, and says so)
aplan import kep.yaml --dry-run --cut M-v0.6   # preview, and place it in a release cut
aplan init                     # scaffold a starter .aplan.yml (never overwrites)
aplan graph                    # generated Mermaid dependency graph
aplan schema                   # the JSON Schema, derived from the Rust types
aplan facts                    # the tool's rules/formats/commands as JSON (see facts.json)
```

`close`, `init`, and `import` are the only commands that write, and all land in your
working tree to stage yourself, never an auto-commit. `close` changes one task file in place,
the `status:` line and a new `evidence:` block, and leaves the rest byte-for-byte
untouched; it refuses to author a false record: an unknown or still-blocked task, one
already `done` or already carrying evidence, or a `note` (which needs a `captured` date
it cannot invent). `init` refuses to overwrite an existing `.aplan.yml`, so it never
clobbers a policy you have already committed to.

`import` reads someone else's plan, and is **lossy by design, and says so** (ADR-0012):
every field the task model cannot hold is reported, never dropped in silence, so you
see exactly what did not survive. It refuses to overwrite a task id that already
exists, and `--dry-run` shows the whole result before anything is written. The rule
that matters is that an importer never asserts a `done` it cannot prove: a source
claiming "implemented" arrives as a `todo` tagged with its own vocabulary (for a KEP,
`kep:implemented`), because a claim is not proof. `--implemented-as done` opts into the
weaker reading, needs an explicit `--as-of` date (aplan does not invent provenance),
and records only note-level evidence. `--cut <milestone>` places the imported tasks in
a release cut, refusing one that is already `done`.

Exit codes are a contract (ADR-0007): `0` clean, `1` at least one error, `2` a
usage or configuration error (your input), `3` an internal error (a bug in
aplan). The four codes let a consumer's CI tell your fault from aplan's. The
commands, output formats, and exit codes are pinned by a black-box corpus in
[`crates/aplan/tests/cmd/`](crates/aplan/tests/cmd/), so the contract is tested,
not merely written down; the flags not yet in that corpus (`--traces-root`,
`--no-require-effort`, `--execute`, `--repo`) are exercised by unit tests.

`ready` lists only *workable* tasks: a task parked at `status: blocked` has no
unmet blockers in the graph, but the plan says do not work on it, so it is not
offered. Every command that reports on the plan (`ready`, `why`, `graph`, `summary`,
`critical-path`, `roadmap`) refuses to answer at all if a task file failed to load: a partial
answer that looks whole is the failure this tool exists to prevent.

## Three consumers, one report

- **Humans** get coloured terminal output with file paths.
- **CI** gets `--format github` annotations, or a `1` exit code and nothing else.
- **Agents** get `--format agent`, where every diagnostic carries an
  `agent_instruction` string that can be acted on without reading the source.

## Spec-driven

The Rust types are the source of truth. `schemas/v1/task.json` is *derived* from
`aplan_core::task::Task` via `schemars`:

```sh
cargo xtask gen-schema           # regenerate
cargo xtask gen-schema --check   # CI: fail if types and schema disagree
```

Prose is never a regenerable source of truth. Design decisions live in
[`docs/adr/`](docs/adr/) as point-in-time records, not as specifications.

## Configuration

A repo declares its policy once in `.aplan.yml` at the root, rather than threading
flags through CI:

```yaml
# .aplan.yml (every key optional)
require_effort: true            # warn on active tasks with no effort_days (APLAN009)
traces_root: "."                # resolve `traces` anchors against this root (APLAN008)
no_traces: false                # or switch trace checking off entirely
test:                           # the runner for `aplan verify --execute`
  list: ["cargo", "test", "--", "--list"]
  run:  ["cargo", "test", "--exact", "{test}"]
evidence:                       # minimum evidence strength for `done` (APLAN017)
  require_min_strength: [commit, test, file]  # a note alone is not enough...
  exempt_tags: [phone]          # ...except on tasks tagged thus, where it is
```

Three properties make the file trustworthy:

- **Schema-derived, like the task schema.** `schemas/v1/config.json` is *derived*
  from the `Config` Rust type and gated by `cargo xtask gen-schema --check`, so the
  documented shape cannot drift from what the tool parses.
- **No key is silently ignored.** An unknown key is a reported error, not a no-op,
  because a config the tool ignores is drift on day one, the exact failure this tool
  exists to catch.
- **Flags override the file, the file overrides the defaults.** So CI can pin a
  policy in `.aplan.yml` and a one-off run can still say `--no-require-effort`.

The file is optional: with no `.aplan.yml`, the built-in defaults apply. Defaults
fail *closed*, every invariant runs unless you switch it off by name, and a `--root`
that is not a directory is a usage error (exit 2), not an empty plan that passes.

## Install

Not yet published. Build from source:

```sh
cargo build --release        # binary at target/release/aplan
```

`cargo install aplan` will work once the crate is released.

## Self-hosting

`aplan` plans, verifies, and renders `aplan`. Its own plan lives in
[`aplan/tasks/`](aplan/tasks/), and `aplan check` and `aplan verify` run clean on it on every
push (the "aplan lints its own plan" CI job), so it is a plan the tool itself vouches for,
evidence resolved against git, not merely asserted.

The roadmap self-hosts too. Work is grouped into **cuts** (release slices), each a
**milestone** meta-task `blocked_by` the tasks in its cut (`APLAN021`/`APLAN022` keep that
model honest); `aplan roadmap` renders the plan grouped by cut, with per-cut rollups and the
critical path. From that, [`ROADMAP.status.md`](ROADMAP.status.md) is *generated* (`cargo
xtask gen-roadmap`) and gated whole-file (`gen-roadmap --check`), exactly as the JSON Schema
is: where the committed file and the plan disagree, CI fails and the plan wins, so the
roadmap's status can never quietly lie about what is done (ADR-0009). [`ROADMAP.md`](ROADMAP.md)
stays the hand-authored *rationale*; the live status is its generated companion.

No task or rule count is quoted here on purpose: a hand-written number is the drift this tool
exists to catch (it has drifted twice already). Run `aplan facts` for the machine-readable
list of every rule, format, and command the tool enforces (committed as `facts.json`, gated
in CI).

## Documentation

- [`ROADMAP.md`](ROADMAP.md), the release sequence and validation plan.
- [`HLD.md`](HLD.md), the technical design and evidence model.
- [`docs/adr/`](docs/adr/), architecture decision records.
- [`CONTRIBUTING.md`](CONTRIBUTING.md), build and test guidance.

## License

Apache-2.0 OR MIT.
