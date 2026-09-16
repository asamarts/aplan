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
tool exists to catch). Most are errors that fail CI; the rest are marked *(warning)* in the
table, and a gate checks each marker against the code that declares it:

| Code | Invariant |
|------|-----------|
| `APLAN001` | Frontmatter satisfies the schema derived from the Rust types |
| `APLAN002` | Task ids are unique |
| `APLAN003` | `blocked_by` references a task that exists |
| `APLAN004` | The dependency graph is acyclic |
| `APLAN005` | **`done` carries at least one piece of evidence** |
| `APLAN006` | No task blocks itself |
| `APLAN007` | A `done` task does not depend on an unfinished one |
| `APLAN008` | `traces` anchors resolve to real headings, or table rows, in real documents |
| `APLAN009` | Active tasks carry an effort estimate *(warning)* |
| `APLAN010` | A `note` used as evidence carries a date |
| `APLAN020` | A live task is not waiting on a `dropped` one |
| `APLAN021` | A `milestone` task has members and carries no effort or paths *(warning)* |
| `APLAN023` | A `done` task's `commit`/`file` evidence can actually be path-checked *(warning)* |
| `APLAN024` | A `done` task's commit evidence is about that task, not mostly other work *(warning, `verify`)* |
| `APLAN025` | A cited test belongs to the part of the repository the task's `paths` cover *(warning, `verify`)* |
| `APLAN026` | A tag that looks like a member of a declared family is one *(warning, `check`)* |
| `APLAN027` | A cancelled cut's members are cancelled too *(warning, `check`)* |
| `APLAN029` | A declared tag family is carried by at least one task *(warning, `check`)* |
| `APLAN030` | A task carries the number of members a declared family requires *(error, `check`, opt-in)* |
| `APLAN031` | A cited `file` is inside the task's `paths`, so it proves this task's work and not that the file exists *(error, `verify`)* |
| `APLAN032` | Every `paths` entry is a glob this tool can compile, so one typo cannot leave every path-based rule silent about a task *(error, `check`)* |
| `APLAN033` | A `prefer_after` entry names a task that can actually be preferred: not unknown, not itself, not a dependency it would restate, and not a cycle *(error, `check`)* |
| `APLAN034` | A `traces` target that leaves this repository is declared in `traces_external`, so reading another repository's files is a stated dependency rather than a surprise on the machine that lacks it *(error, `check`)* |
| `APLAN035` | Cited `commit` evidence names a commit that did work rather than a merge, which combines whatever both sides touched and so satisfies almost any `paths` *(warning, `verify`)* |
| `APLAN036` | A configured test runner lists at least one test, so a `dir` or `list` that stopped matching is reported against `.aplan.yml` rather than as citations nothing resolves. Declare `expect_tests: false` for a component that genuinely has none *(warning, `verify`)* |
| `APLAN037` | A test reported as passing under `--execute` really ran: the runner is asked once what a *miss* looks like, and a pass that is the same bytes as that miss is reported rather than believed. Declare `run_selects: true` for a runner that says nothing either way *(warning, `verify --execute`)* |
| `APLAN038` | A `done` task has answered the acceptance criteria its own body states: the tool scaffolds the checkboxes and never read them, so a status could be `done` over its author's own unticked definition of done. Counted, never ticked for you; a box inside a code fence is quoted rather than stated *(warning, `check`)* |
| `APLAN039` | A `done` task has not ticked a prompt this tool wrote as though it were a criterion. `- [x] state what done means for this task` asserts the *instruction* was met, not the work; `import` writes a second prompt and both count. The one content this tool reads, because it is the one content this tool wrote *(warning, `check`)* |
| `APLAN040` | A task declares each `paths` entry once. A repeated glob matches the same files, so nothing is weakened -- it is a botched edit, and a bulk `paths` rewrite is when they happen. Two spellings of one path count as one entry *(warning, `check`)* |
| `APLAN042` | A `revisit_at` names a real instant, on a task that is parked. It is a hand-off to an external scheduler: reaching the instant means *look again*, never that the wait is over, so it changes no status and `check`, `ready` and `verify` never read a clock *(warning, `check`)* |
| `APLAN041` | The generated `AGENTS.md` block still says what this repository's policy says. It is what an agent reads before touching the plan, and it is a second copy of the policy, so it goes stale silently. Scoped by the markers: a file that carries none is one nobody asked to keep current *(warning, `check`)* |
| `APLAN043` | A live task does not carry the scaffold's own prompt beside the criteria its author wrote. There is no command that edits a body, so the real criteria go in by hand, and appending your own `## Acceptance criteria` to a scaffolded file leaves two of them with the prompt under the first *(warning, `check`)* |
| `APLAN044` | A closed task claims less of the released version than its level implies, and says nowhere why. A task's level is already a floor -- a unit of work a `fix`, a cut a `feature` -- so ordinary tasks declare nothing; claiming below the floor is overriding the plan's reading, which a reviewer should be told about. Off unless `version.require_impact` asks for it *(warning, `check`)* |

`APLAN023` is the one the first outside pilot produced. `paths` is optional, and `APLAN012`
binds a cited `commit` to a task by checking its diff against those globs, so a task with no
`paths` has nothing to compare against: the citation resolves and proves only that some commit
exists. That happened on seventeen of thirty-three tasks in a real repository while `check`
reported zero warnings, so the warning exists to make the gap visible rather than to forbid it.

`APLAN024` is what the second pilot produced, and it extends `APLAN012` rather than repeating
it. That rule asks whether a cited commit touches the task's `paths` *at all*, which a large
enough commit satisfies by accident: measured on a real migration, 31 of 64 citations pointed at
commits touching more than fifty files, 16 of them the same 124-file scaffold, and for those the
overlap test could not have rejected anything. It counts the paths a commit touches *outside*
the task's `paths`, so a large change wholly inside them is focused and a small one cannot swamp
anything, and it stays silent when the task also cites a test. `evidence.max_unrelated_files`
tunes it; `0` turns it off.

`APLAN025` closes the last gap in that argument. `test` is the strongest evidence kind here and
the one `require_min_strength` steers repositories toward, yet it was the only kind whose
*relevance* nothing checked: `APLAN012` binds a commit to the task's `paths` and `APLAN015` binds
a file, while a test only had to exist. The same audit found a task citing a test that lives in
another component and asserts something about that component, which the resolver accepted because
a runner really does list the name. A runner says what its tests cover with `covers`, or failing
that the `dir` it runs in, and a task whose `paths` never reach that code is the same mechanical
mismatch, reported as one. `covers` is the field that matters in practice: a runner selecting a
component with `--manifest-path` still runs at the repository root, so where a runner runs is not
what it covers. A runner declaring neither makes no claim about locality and is silent, which is
every single-runner configuration. Where one runner holds tests that cover different things,
`covers_by_test` names the exception: a repository-wide invariant usually lives in whichever crate
has a harness that can read the tree, and asserts a fact about a directory it is not part of.

A **release** is a milestone tagged `release`, and it closes cuts rather than holding work
(ADR-0022): `blocked_by` names the cuts, `ready` and `why` see it as they see any checkpoint,
`APLAN007` refuses to call it done while a cut it names is open, and `roadmap` rolls up the work of
those cuts. A task stays in exactly one cut, because a release claims none of them.

A cut is labelled by its own tag, `v0.3` where a project uses release versions and `s0` or
`phase-2` where it does not, and cuts without release versions order by a natural comparison so
`s2` precedes `s10`. A milestone listing another milestone in its `blocked_by` says its cut
follows that one, and the members of the later cut inherit the edge, so `ready` and
`critical-path` are right on a sequential plan without an edge on every task (ADR-0013). A plan
whose milestones do not block one another is unaffected.

`aplan` models **one** grouping level, the cut, and a second level is a declared tag family
rather than model surface (ADR-0014): a stage, a lifecycle, or a per-group state is a tag such as
`artifact:accepted`, because `Status` carries a verdict about work and a stage does not.

Where a project has that second level, `roadmap.group_by` names the family and each cut renders
under its headings, with an explicit `(no group)` section for tasks carrying none, since putting
them under a heading they do not have would be a lie. A plan that declares nothing renders exactly
as it did before the option existed.

`APLAN026` is opt-in and exists because a second grouping level always ends up in tags. ADR-0008
rejected tags for cut membership precisely because a typo is silent, and then a pilot put 22 epics
there anyway, with a build/measure/review taxonomy beside them: one validated grouping level and
one unvalidated one in the same frontmatter, indistinguishable by eye. Declaring the vocabulary in
`.aplan.yml` makes the claim checkable without making tags a model. A tag carrying a family's
prefix that the family does not list is reported, and so is one a single edit from a member, which
is what catches `epci-k` where a prefix rule sees nothing. Tags outside every declared family stay
legal and silent.

`APLAN029` is the same question one level up, and it exists because the pilot that asked for
`APLAN026` then mis-declared its own vocabulary: `epic: [a, b]` with tasks tagged `epic:a`, where
the documented shape is the whole tag. Every layer accepted it. The family is named and non-empty,
`group_by` names a declared family, and `APLAN026` judges only tags that already look like members,
of which there were none, so `check` reported nothing and the roadmap rendered every task under
`(no group)`. A family no task carries is now reported. It is a question about the plan rather than
about `.aplan.yml`, because a family may legitimately list members that do not carry its prefix and
a config-time check would refuse that working shape; and it is a warning, because a vocabulary
declared before its first use is as likely as a mis-declaration, and the two have different
remedies.

`APLAN030` is the other half of a declared vocabulary: how many of it a task carries. Declaring
members says what `priority` contains and nothing about whether a task may have two of them, so
`priority-p0` and `priority-p1` together, and no priority at all, were both clean. A family may now
say:

```yaml
tag_families:
  epic: [epic-a, epic-b]              # the list shorthand: no cardinality, unchanged
  priority:
    members: [priority-p0, priority-p1, priority-p2]
    require: exactly-one              # optional | at-most-one | at-least-one | exactly-one
    status: live                      # any | live | done | dropped | terminal
    milestones: exclude               # include | exclude | only
```

`status` and `milestones` are separate keys rather than a list of predicates, so a scope that can
never match cannot be written. An *error* rather than a warning, unlike the two rules above: they
report something that looks wrong, and this reports a requirement the repository wrote down being
unmet, which is what `APLAN022` and `APLAN017` already are. A repository adopting the requirement
before its plan satisfies it has `advisory:`.

Only declared members count. A task tagged `priority-p9` under `exactly-one` carries no priority
this repository declared, and `APLAN026` reports the tag; taking it as proof the requirement was met
would be the rule accepting a typo as evidence.

Every finding carries the specific thing to do about it, and `check` prints it once per rule: the
advice is the same for every finding of a code, so repeating it fifty times is what makes a large
report unreadable and withholding it is what makes a small one unactionable. `--format agent`
carries it on every diagnostic.

`APLAN020` catches a plan that is *deadlocked*: a task still `todo` whose blocker
was abandoned can never legitimately start, and every tracker will happily let it
sit there forever. `APLAN021` keeps the `milestone` tag honest (milestones group a
release cut; see [self-hosting](#self-hosting)). (`APLAN011`-`APLAN015` are the
evidence-ladder rules below, run by `aplan verify`, not `check`; `APLAN017`,
`APLAN018`, and `APLAN022` are the opt-in rules below; `APLAN016` (url reachability)
and `APLAN019` (`paths` globs match a tracked file, once the task cites evidence) run
under `verify`, the first
only where `.aplan.yml` sets `url.check`. A rule code is a
contract, so it is not renumbered for tidiness.)

Four more `check` rules are **opt-in**, off unless `.aplan.yml` turns them on.
`APLAN017` requires a `done` task's evidence to meet a configured minimum strength,
so a real project can decree that a bare `note` does not close a task the way a
`commit` or a `test` does. `APLAN018` detects *stale-`done` drift*: `aplan close`
stamps a content fingerprint of each traced section, and `check` warns (advisory)
when a closed task's traced section changes afterward, cleared by re-reviewing and
`aplan close --restamp`. `APLAN022` requires every task to be a member of exactly one
release cut (or tagged `cross-cutting`), so a milestone's `done` proves the cut is
exhaustively complete. `APLAN028` rides with `detect_drift`: `trace_fingerprints` is keyed
by the trace string, so *editing* a `traces` entry on a closed task leaves the new trace
unstamped and drift silently unchecked for it, and stamping some traces and not others is
an error rather than a judgement call. See [Configuration](#configuration).

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

**An anchor may also name a table row**, by its first cell (ADR-0015). A register
kept as one table under one heading would otherwise re-flag every task tracing it
whenever any row changed, so `constitution.md#invariant-7` fingerprints that row
alone and an edit to invariant 11 is not invariant 7's drift. Headings resolve
first, so nothing an existing plan traces changes meaning.

**A list item is not an anchor**, and that is a decision rather than an omission.
A table row has a first cell: a short name the author wrote, which survives the
prose around it being rewritten. A bullet has no such position, so its identity
would have to be its whole text -- and then rewording it breaks the reference
instead of reporting drift, which is the opposite of what row anchors bought.
Measured across nine repositories on 2026-08-22: of 16,632 list items in 1,091
markdown files, 71.5% are plain prose with no leading name at all, and only 10%
carry one short enough to be a name. If you keep a register you intend to trace item
by item, give each item a heading or a table row; the adopter who hit this
restructured four bullets into `### 4.1`..`### 4.4` and their document is more
addressable for it, not less.

## What counts as evidence

Not "I finished it."

| Kind | Meaning |
|------|---------|
| `commit` | A git SHA in this repository |
| `url` | An external URL, a filing receipt, a regulator page, a vendor quote |
| `file` | A repo-relative path to a committed artefact, optionally naming the section that proves it (`docs/report.md#result`) |
| `test` | A named test that must pass |
| `note` | A dated human note. The weakest kind; requires `captured:` |

An undated note is a rumour, so `APLAN010` rejects it. `commit`, `file`, and
`test` are the kinds `aplan verify` resolves against the repository (below); the
rest are checked structurally.

A `file` citation may name a section, and `verify` then checks that the section
is in that document -- the same heading-and-table-row reader `traces` uses, so an
anchor copied out of a rendered URL resolves. It is read at the cited rev
(`docs/report.md#result@abc123`) or at `HEAD`, never off disk, so an uncommitted
edit to the proof cannot settle a citation for its author and fail for everybody
else. It stays a `file` citation on the ladder; naming a section only narrows
what it claims.

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

**`verify` is a superset of `check`, and runs the L0 rules too.** The ladder is
cumulative: a plan that fails L0 has nothing worth resolving, so `verify` reports the
hermetic findings alongside the evidence ones and the two commands never disagree about
whether a plan passes. Run `check` alone when you want the hermetic answer without
touching git, which is what makes it fit a pre-commit hook; run `verify` when you want
everything.

That was not always true. `verify` used to skip the hermetic rules *and* print
`0 warning(s)`, which is a claim about the subset of rules it ran, worded as a claim
about the plan. An adopter who wired the deeper-sounding command into CI got a
permanently green build on a plan `check` rejected, and nothing in the output hinted
that a second command existed (`A1-058`, found in the `p00025` pilot).

A `commit` SHA must resolve to a real commit (`APLAN011`), and its diff must
touch at least one of the task's declared `paths` (`APLAN012`); a `file` must
exist at the cited rev, or at `HEAD` (`APLAN015`); a `test` must be one the
configured runner lists (`APLAN013`), and under `--execute` it must pass
(`APLAN014`).

A runner that *skips* the cited test exits zero too, and a zero exit is what
`--execute` reads as a pass. Declare `test.skipped` on the runner -- argv that
prints the tests it will not run -- and a task whose every test citation is one
of them is reported rather than believed (`APLAN014` again).

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
  run:  ["cargo", "test", "--", "--exact", "{test}"]
```

CI must not pass `--execute` for fork pull requests (see [`SECURITY.md`](SECURITY.md)).

### When the deliverable is a decision

Research, design, a recorded decision, a review round's findings: work whose product is a
document rather than a diff. This is the case adopters most often assume the tool cannot hold,
and the assumption is expensive, because the way out that presents itself is `exempt_tags` --
which switches the gate off for exactly the work whose claims are hardest to check.

**The same ladder applies, and it reaches further than it looks.** In order:

| What you cite | Strength | What binds it to this task |
|---|---|---|
| **A test that reads the document** | strongest | the document's own content, asserted against the code, failing when either drifts |
| **`file:<doc>#<section>`** | strong | the blob at the cited rev, *and* `APLAN031`: the file must lie inside the task's own `paths` |
| **`file:<doc>`** | honest when the deliverable is the whole document | the blob only |
| **`commit`** | weakest | path overlap alone (`APLAN012`), which is why `APLAN023` and `APLAN024` exist |
| **an exempt tag** | not a rung | nothing, and it *asserts that none of the above exists* |

That last row is the point. An exempt tag is not a shrug, it is a claim -- and one worth checking
before you make it. In the adoption that produced this table it would have been false for 25 tasks
at once, which is why that plan tightened its policy instead of reaching for the tag.

**The worked case.** `p00002` is a plan for a language; 25 of its 64 tasks have a prose
deliverable. Its `AM-005` reconciled a design document with the implementation and closed on a
*test*:

```yaml
evidence:
  - kind: test
    ref: checks::verify::22-the-design-record-describes-this
  - kind: file
    ref: docs/design/dsl-architecture.md
```

Rather than assert the document exists, it made the document **executable**: the check asserts
the four vocabularies the document publishes against the engine that implements them, and every
leading form the corpus uses against the grammar the document specifies. Rename a kind in the
code and the suite fails, naming the document.

It then *tightened* `require_min_strength` from `[commit, file, test]` to `[test, file]`, on the
reasoning that dropping `file` would be a regression dressed as a hardening: those 25 tasks would
each have had to claim `untestable`, "which asserts nothing stronger exists about a task where
something stronger does exist". **0 of its 72 tasks carry an exempt tag.**

**What none of this claims.** That the cited section is a *good* decision, or says what the task
needed it to say. `aplan` proves the artefact exists, sits inside the work this task owns, and --
with a `traces` anchor on the same section and `detect_drift: true` -- has not silently changed
since (`APLAN018`, opt-in). Whether the decision was right is a review, and no tool here pretends
otherwise.

Pin the anchored form with `aplan facts --require evidence.file_names_a_section`.

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

## Starting a plan

**A cut is created with its work, not before it.** That is not a quirk of the tool, it is what a
cut *is*: a `milestone` task whose `blocked_by` lists its members (ADR-0008), so an empty one is a
half-thing and `APLAN021` says so. Where `deny_warnings: true` makes that finding block -- the
policy this repository runs on itself -- neither half of a new plan can be added on its own. A
first task belongs to no cut (`APLAN022`); a first cut has no members (`APLAN021`).

So both halves go in one file. `aplan add --from` applies its entries as a single change, nothing
is written unless all of it can be, and an entry naming `cut:` joins a milestone the same file
declares:

```yaml
# plan.yml
- id: M-v0.1
  title: First cut
  tags: [milestone]
- id: T-001
  title: Decide what done looks like
  cut: M-v0.1
  effort: 1
- id: T-002
  title: Wire the gate into CI
  cut: M-v0.1
  effort: 2
```

```console
$ aplan init
wrote ./.aplan.yml
edit it to set your policy; every setting is optional and documented inline.
$ aplan add --from plan.yml
wrote M-v0.1  todo  First cut
wrote T-001  todo  Decide what done looks like
wrote T-002  todo  Wire the gate into CI
  `M-v0.1` holds 2 of them, in its own file
$ aplan check
ok: 3 tasks, 0 warning(s), DAG acyclic
```

Every later cut has the same shape, and there are three of them:

| the new cut holds | how |
|---|---|
| work that does not exist yet | another `--from` file, the milestone entry and its tasks together |
| work the plan has, in no cut | one command: `aplan add "Second cut" --id M-v0.2 --tag milestone --blocked-by T-003` |
| work that is in another cut | create the cut around its own new work first, then `aplan mv T-002 --cut M-v0.2` |

An entry takes the fields `aplan add` takes as flags, plus `body` for the task's prose, so a plan
authored from a document lands closeable rather than needing a second edit per task.

**Order is for reading, not for resolving.** An entry may wait on, or join, any id the file
declares, above it or below it: the milestone above lists work written under it, and it would have
worked equally with `blocked_by: [T-001, T-002]` on the milestone and no `cut:` on the tasks. An
id the file does not declare is minted from the convention the plan already uses, and one minted
for a later entry has no name to be referred to by, and a `cut:` the plan itself resolves is the
plan's.

## Commands

```sh
aplan check                    # validate structure, hermetically; exit 1 on any error
aplan check --format agent     # every diagnostic carries an agent_instruction
aplan check --format github    # inline PR annotations
aplan check --format sarif     # SARIF 2.1.0, for code-scanning dashboards
aplan check --no-traces        # skip APLAN008 (opt out; never silent)
aplan verify                   # resolve evidence against the git repo (L1/L2)
aplan verify --execute         # also run cited tests (L3; off by default)
aplan ready                    # workable tasks whose blockers are all done, best first
aplan ready --explain          # and why each one is where it is
aplan why P1-003               # what blocks a task: its own blockers, and any unfinished cut
aplan why P1-003 --all         # ... walking through those cuts into their membership too
aplan paths                    # how much of recent history each task's `paths` would match
aplan stale --older-than 30    # live tasks that have sat -- including parked ones, and how long the park has held
aplan due --as-of 2026-09-10T06:00:00Z   # parked tasks whose `revisit_at` has arrived: due for a look, not unblocked
aplan vacuity                  # per rule: what it checked, and what it could not have failed on
aplan deprecations             # runways: announced in one cut, removed in a later one
aplan attest                   # what this plan proved, bound to the commit it proved it at
aplan check-docs ROADMAP.md    # does this document name tasks that exist?
aplan export-roadmap-json roadmap.json  # regenerate a foreign status contract from the plan
aplan export-agents-md         # the agent contract your policy implies, fenced
aplan export-agents-md --check AGENTS.md   # ... and the gate that keeps it true
aplan summary                  # tasks, effort, and ready count rolled up by phase
aplan summary --format json    # the same, machine-readable
aplan tasks --format json      # the plan as data, so any other rollup is a `jq` line
aplan critical-path            # the longest effort-weighted chain of dependent tasks
aplan roadmap                  # the plan grouped by cut: rollups + per-cut critical path
aplan roadmap --format markdown # the committed ROADMAP.status.md companion (ADR-0009)
aplan roadmap --format markdown --regenerate-with 'make roadmap'  # when a wrapper generates it
aplan add "Second cut" --id M-v0.2 --tag milestone --blocked-by T-003  # a cut is created with its work
aplan add "Wire the sequencer" # scaffold the next task file, in this plan's own id convention
aplan add "Fix what the log found" --traces pre-public-adoption-evidence#24-a-finding   # and record what caused it
aplan add "Ship it" --cut M-v0.6 --effort 2 --paths 'src/**'   # and place it in a cut
aplan add --from plan.yml      # a written plan as one change: every task, its cut, and its body
aplan close P1-003             # set done + attach the HEAD commit, in place
aplan close P1-003 --evidence commit:d213c9a --evidence test:mod::name
aplan amend P1-003 --replace-evidence test:old_name test:new_name   # repair a closed task's citation
aplan supersede P1-003 "Re-fix the rail"  # the successor to a closed task whose outcome stopped holding
aplan dep add P1-004 --after P1-003     # P1-004 comes after P1-003; `dep rm` takes it back, and a cut's membership stays `mv`'s
aplan drop P1-003              # cancel a task, recording the decision rather than deleting it
aplan drop --cut M-v0.6        # cancel a whole cut: its members, then the milestone
aplan mv P1-003 P1-004 --cut M-v0.7   # move tasks between cuts, editing both milestones or neither
aplan import kep.yaml          # read a Kubernetes KEP into a task (lossy, and says so)
aplan import ROADMAP.md --format checklist     # a markdown `- [ ]` roadmap, the commonest shape
aplan import roadmap.json --format roadmap-json # a status contract, depends_on -> blocked_by
aplan import kep.yaml --dry-run --cut M-v0.6   # preview, and place it in a release cut
aplan export --format checklist --out ROADMAP.md  # write the plan back out as the checklist it came from
aplan export --format checklist --check ROADMAP.md  # ... and the gate that keeps it true
aplan mcp                      # MCP server over stdio: identity, ready, why, check, verify, add, show, dep, close
aplan init                     # scaffold a starter .aplan.yml (never overwrites)
aplan graph                    # generated Mermaid dependency graph; `--cut v0.8`/`--tag output` scope it
aplan tree A1-126              # one task's neighbourhood: what precedes it, what it releases (`--depth N`)
aplan show A1-126              # one task whole: fields, edges both ways, and the findings on it
aplan schema                   # the JSON Schema, derived from the Rust types
aplan facts                    # the tool's rules/formats/commands as JSON (see facts.json)
```

`add`, `close`, `drop`, `mv`, `init`, and `import` are the only commands that write, and all
land in your working tree to stage yourself, never an auto-commit. `close` changes one task file in place,
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
offered.

**`blocked` with an empty `blocked_by` is the idiom for "waiting on something
outside the plan"**, and it is deliberate that no field says what. Across eleven
repositories, 22 of the 38 tasks at `status: blocked` are parked that way, and
what each waits on is a judgement no tool can evaluate: "no named consumer
exists", "three models confirm the pattern", "no external authors have
surfaced". A field for it would be a claim nothing checks, which is the one
thing this format refuses. So the reason lives in the task body, `aplan why`
says a parked task is parked and points there, and unparking is a human
editing `status`. Every command that reports on the plan (`ready`, `why`, `graph`, `summary`,
`critical-path`, `roadmap`) refuses to answer at all if a task file failed to load: a partial
answer that looks whole is the failure this tool exists to prevent.

## Three consumers, one report

- **Humans** get coloured terminal output with file paths. Colour follows the
  stream -- a terminal gets it, a pipe gets plain bytes -- and honours `NO_COLOR`,
  `CLICOLOR_FORCE` and `CLICOLOR`. `--color=always` forces it into a pager or a
  screen capture; `--color=never` refuses it at a terminal. It carries emphasis and
  never information: stripping every escape leaves exactly what `--color=never`
  prints, and the machine formats below are byte-identical at every setting.
- **CI** gets `--format github` annotations, `--format sarif` for a code-scanning
  dashboard, or a `1` exit code and nothing else.
- **Agents** get `--format agent`, or `aplan mcp`, an MCP server over stdio with a
  deliberately small nine-tool surface (`identity`, `ready`, `why`, `check`, `verify`, `add`, `show`, `dep`, `close`). Its `close`
  goes through the same evidence gate as the CLI's, and takes no score, confidence, or
  completeness input: an agent supplies a *citation*, never an assessment of its own
  work. Refusing that is the point, and it is pinned by a test.

  `identity` is the one that answers about the *server* rather than about a plan, and it
  exists because **asking an installed CLI does not identify a configured MCP process**:
  the two can be different executables, and a pilot spent a pass diagnosing a defect its
  own checkout had already fixed. It returns the same document `aplan facts --identity`
  prints, and needs no plan, no repository and no clean tree -- the reason to ask is
  usually that something else did not work.
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
traces_external: ["../sibling"] # roots outside this repo a trace may name (APLAN034); default: none
no_traces: false                # or switch trace checking off entirely
test:                           # the runner for `aplan verify --execute`
  list: ["cargo", "test", "--", "--list"]
  run:  ["cargo", "test", "--", "--exact", "{test}"]
  skipped: ["cargo", "test", "--", "--list", "--ignored"]   # tests it will not run (APLAN014)
  timeout_seconds: 600          # how long `list` may take before the runner is reported
evidence:                       # minimum evidence strength for `done` (APLAN017)
  require_min_strength: [commit, test, file]  # a note alone is not enough...
  exempt_tags: [phone]          # ...except on tasks tagged thus, where it is
deny_warnings: false            # make a warning a finding, so every gate blocks
advisory: [APLAN022]            # hear these rules in full, without being stopped by them
id_strategy: sequential         # or `ulid`, for authors who mint ids offline in parallel
```

`evidence.require_min_strength` is a **progression, not a preference**, and the two ends of it
look like disagreement if nobody says so. **Start at the widest setting with an exempt tag you
review**: `[commit, test, file]` plus one tag, named for what claiming it asserts. A plan authored
before its implementation has commits at best and often nothing to cite, so demanding the strongest
rung at authoring time leaves every task uncloseable until code arrives, and the predictable answer
to that is a weaker citation dressed as a stronger one. The wide setting keeps a claim honest about
which rung it is standing on.

**Narrowing is earned, and what earns it is tests that exist to point at.** Once a component has a
runner `aplan verify` can list (`test:` above), `[test]` says a closed task names something a
machine reruns. This repository runs that narrow end, and it is the far end of the arc rather than
the recommended first step: its `.aplan.yml` sets `require_min_strength: [test]` with
`exempt_tags: [untestable, milestone]`, where the example above starts you at
`[commit, test, file]`. It earned the move by having a runner per crate before it asked for one. An exempt tag is a claim in either stage, and the one it makes is specific: *no rung
above this exists for this task*, which is why every task carrying one here says which case it is
in its own body.

**Every configured command is bounded.** `test.list`, `test.run` and `url.check` name programs
this tool spawns, and a command that never returns used to make `aplan verify` wait for it, which
in CI is a job that hangs rather than fails: nothing reports it and the runner is held until the
job's own timeout. `test.list` gives up after 600 seconds and `url.check` after 30, each
overridable (`timeout_seconds`, `0` for no limit) and each chosen for its command: `cargo test
--list` compiles, so minutes are legitimate, while a network check that has not answered in half a
minute has not answered. A timeout is reported as its own outcome -- *not checked*, never "did not
answer" -- because nothing was learned. `test.run` under `--execute` is deliberately unbounded: a
long test run is the point rather than a symptom, and that limit belongs to your harness.

`id_strategy` is the one key whose cost is paid by everyone who reads the plan. `aplan add`
normally continues the sequence a plan already uses (`A1-046` -> `A1-047`), which is what
makes it fit a repository rather than impose a format on one, and it has dstask's documented
failure: two people working offline both derive `A1-047` and find out at the merge.
`APLAN002` catches the collision there, so a plan cannot end up holding two tasks under one
id, but the repair is a manual renumber and every reference to the loser moves with it.

Setting `ulid` removes the collision instead of detecting it: the counter is replaced by a
ULID, 48 bits of timestamp and 80 random bits, so two authors offline cannot derive the same
id however long they stay offline. **The cost is that an id stops being memorable**:
`A1-047` becomes `A1-01K9ZBW7YQ8N4TQGX2H6RC3PVD`, in every `blocked_by` list, every `why`,
and every filename. The plan's own prefix is kept, because it is the readable part and
nothing about it collides. A repository with one author does not have this problem and
should not pay for it, which is why `sequential` is the default and why this is a choice
made once rather than a setting to try. Ids already written are never rewritten, and
`--id` still overrides both.

`deny_warnings` is for a repository whose policy is that every gate either passes or
blocks. Warnings are advisory by default and should be: `APLAN023` warns rather than
errors precisely because a citation may be perfectly honest and the repository is the
judge. Setting this says you would rather be the judge in the strict direction too. The
exit code does not change, and that is the point: `1` still means "at least one finding",
so nothing in a consumer's CI has to learn a new code. `--deny-warnings` and
`--no-deny-warnings` override the file for one run, in either direction.

`advisory` is the same knob turned the other way, and it exists for the case a rule is
most useful during exactly the migration it cannot yet pass. `APLAN022` says every task
belongs to a cut; a plan part-way through adopting cuts wants to see every task that does
not, and cannot afford a red build until the last one is placed. Listing a code here
reports its findings **in full and at their declared severity**, and excludes them from
the verdict. The severity stays the rule's own, in the output, in `aplan facts` and in the
catalogue, because a consumer asserting what a rule *says* must not be asserting something
a config can change. What a repository configures is the verdict.

Nothing is hidden by it, on any surface. The summary line names the count and the codes
(`ok: 41 tasks, 0 warning(s), 3 advisory (APLAN022), DAG acyclic`), `--format json`
carries an `advisory` count beside `errors` and `warnings`, a GitHub annotation says
`(advisory)` in its title, and a SARIF result carries a `suppressions` entry, which is
the word that format already has for a finding that is real and accepted. A code this
build does not know is refused rather than ignored, since a typo would downgrade nothing
while reading like a repository that had made a choice.

Two things it deliberately cannot do. It cannot make `close` and `verify` disagree: a
citation this policy tolerates is one `close` records, because `close` refuses exactly
what `verify` rejects and reads the same policy to decide. And it cannot make a plan that
did not *load* acceptable: a file the loader could not read leaves every other rule's
silence about that file meaningless, so `check`, `verify`, `ready` and `why` all refuse
it whatever `advisory:` says.

When one command cannot list every test, `test:` takes a sequence instead. An umbrella
repository of independent projects (no root workspace, so no single `cargo test` sees them
all), a polyglot tree, or a split between unit and e2e runs all hit that wall, and without
this most tasks cannot cite `test` evidence at all and quietly fall back to weaker kinds:

```yaml
test:
  - name: libs                  # `name` is only for diagnostics
    list: ["cargo", "test", "--manifest-path", "libs/Cargo.toml", "--", "--list"]
    run:  ["cargo", "test", "--manifest-path", "libs/Cargo.toml", "--", "--exact", "{test}"]
  - name: web
    dir:  web                   # runs here, so a non-Rust runner works at all
    list: ["pnpm", "test", "--list"]
    run:  ["pnpm", "test", "{test}"]
```

A citation resolves if exactly one runner lists it, and `--execute` runs it with that runner.
If two runners list the same name the citation does not identify a test, so `APLAN013` fails
and names both rather than picking by config order.

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

Build from source:

```sh
cargo build --release        # binary at target/release/aplan
```

Or take the released binary, a statically linked Linux x86_64 build published with
its `sha256` (`v0.1.0` onward), so it runs whatever glibc the host ships. `cargo
install aplan` will work once the crate is released.

A build from `main` reports a pre-release version (`0.3.0-dev`), never the version
of a published release, so a staged build can never be mistaken for one. Pin a
*capability* rather than a name where you can:

```sh
aplan facts --require test.covers --require APLAN024
```

which exits non-zero and names what is missing, so an image build asserts what it
depends on instead of matching `aplan facts` with `grep`.

Needing a feature that is not in a release yet means staging a binary you build,
and it has to be static or the runner image will not start it.
[`docs/upgrading.md`](docs/upgrading.md) gives the containerised build that works,
the two traps on the direct route, and what adopting a newer build will report on
a plan you have not touched.

### In CI

```yaml
- uses: actions/checkout@v7
- uses: asamarts/aplan@v0.2.0    # pinning the action pins the binary
```

That installs the release matching the tag you pinned, verifies it against the
published checksum, and runs `aplan check` with `--format github`, so a violation
becomes an inline annotation on the pull request. Policy comes from your
`.aplan.yml`, not from flags threaded through the workflow; `command: verify`
resolves evidence against git instead (check out with `fetch-depth: 0` for that).

**Automating the gate turns the warning tier off.** Both commands exit `0` with warnings
outstanding, by design -- and **21 of this tool's 44 rules are warnings**, among them
most of what it says about evidence: a task `done` over its own unticked acceptance
criteria (`APLAN038`), a traced section that moved after closure (`APLAN018`), and,
under `verify`, a commit so broad it would have overlapped almost any task's `paths`
(`APLAN024`) and a cited test from a runner that does not cover the files the task
claims (`APLAN025`). Every one of them reports, and not one changes the exit code.

Under `--format github` they do arrive as annotations on the pull request, which is
better than nothing and is still not a gate: the check is green, and a green check is
what a reviewer scrolls past and what a merge queue merges on. That is a defensible
policy while a person runs the gate and reads it. Switching `detect_drift` on and wiring
the gate into CI are two decisions made in two sittings, and this is where they meet:
the second one silences the first unless `deny_warnings: true` is in your `.aplan.yml`.
A pilot measured exactly that -- drift detection reporting two findings under a green
build. If some rule is one you cannot satisfy yet, `advisory:` names it and keeps the
rest blocking, which is a migration rather than a tier left switched off.

The action deliberately has no `latest`: an install that resolves at run time is
not pinned, and a gate whose version changes between runs is the drift this tool
exists to reject. Linux x86_64 only, which it says up front rather than failing
obscurely.

**It needs your workflow to be able to resolve `asamarts/aplan`, and today nothing
can.** This repository is private and its Actions access policy is `none`, so no
other repository may use the action -- not even another owned by the same account.
That much is a setting. What is not is the cross-account case: sharing a private
repository's action beyond its owner is an organisation feature, and this owner is
a personal account, so for a repository under a different account no setting and no
token makes the `uses:` line resolve. It fails before any of your plan is read, and
two pilots have hit exactly that. The arrangement
that works is a static binary staged into a self-hosted runner image and pinned by
version and digest; [`ci/README.md`](ci/README.md#running-aplan-in-someone-elses-ci)
has the procedure, and the alternatives it was chosen over. The action becomes the
simple path again when this repository goes public, which is settled as *not yet*:
the condition is v1.0 readiness followed by at least a month of continuously
planning real work with the tool, which is a measurement rather than a wait
(the pre-public repository-boundary decision, narrowed by the pre-public repository-boundary decision).

### Changing a plan without breaking it

`add`, `close` and `import` create and complete; the rest of a plan's life is
editing YAML, which is fine for an estimate and not fine where hand-editing can
break an invariant. Two mutations qualify:

```sh
aplan mv RD1-04 --cut M-s0     # edits both milestones, or neither
aplan drop RD3-01              # records the cancellation rather than deleting it
aplan drop M-s3 --cut          # the whole cut: its members, then the milestone
```

Both resolve everything before writing anything, so a refusal leaves the plan
exactly as it was, and both parse back what they wrote before saving it, on
`close`'s standard that a rewrite the tool cannot read again is a corrupted file.
`drop` refuses a task that is already `done` or `dropped`: those record a
decision, and rewriting one silently would launder it.

Everything else is still YAML on purpose. Changing an estimate or a `paths` glob
breaks nothing that `check` will not catch on the next run, and a command for it
would be write surface bought with no invariant to maintain.

### Announcing a removal, and keeping the promise

"Announce it in one release, remove it in a later one" is the most checkable
promise a plan makes, and it needs nothing new here: it is two ordinary tasks and
the edge between them.

```yaml
# aplan/tasks/T-40.md -- the announcement, a member of the v0.3 cut
id: T-40
title: Announce that `--legacy-format` is going away
status: done
tags: [deprecation]

# aplan/tasks/T-51.md -- the removal, a member of the v0.5 cut
id: T-51
title: Remove `--legacy-format`
status: todo
tags: [removal]
blocked_by: [T-40]
```

Each is a member of its cut the way any task is, so "announced in v0.3, removed
in v0.5" is already written down. `aplan deprecations` reads it back:

```
T-40  Announce that `--legacy-format` is going away
  announced in v0.3 (done)
  removed in v0.5 by T-51 (todo)
```

and names the ways the pair breaks: announced with nothing scheduled to remove
it (the notice ships and the removal never lands), announced and removed together
with no release in between for anyone to migrate in, removed in a cut that ships
*before* the one announcing it, and the announcement cancelled while the removal
was not. A removal tagged `removal` and blocked by no announcement is the same
break from the other end.

The announcement's title is what `--format markdown` prints, so write it as the
notice you want published. None of this fails a build: the report is where
somebody who has a runway goes looking, and a plan without one is not a plan with
a problem.

### Rollups this tool does not have

`summary` rolls up by phase. Anything else is `aplan tasks`, which emits the plan
as data with the awkward parts already resolved (the cut a task belongs to,
whether it is ready), so a consumer is not re-implementing the graph:

```sh
# effort by cut, live work only
aplan tasks --format json | jq -r '
  [.tasks[] | select(.milestone_task|not) | select(.status!="dropped")]
  | group_by(.cut)[] | "\(.[0].cut // "none")\t\(map(.effort_days // 0)|add)d"'

# effort by a tag family, for a plan that types its work (build/measure/review)
aplan tasks --format json | jq -r '
  [.tasks[] | select(.milestone_task|not) | select(.status!="dropped")]
  | group_by(.tags | map(select(startswith("kind-"))) | first)[]
  | "\(.[0].tags[0] // "untyped")\t\(map(.effort_days // 0)|add)d"'
```

The second is the rollup a pilot kept by hand, in a table that was wrong twice
during its own audit, because 20 of its 105.5 days were wall-clock bound and no
command could say so. `status` is carried rather than filtered, so cancelled work
is the consumer's decision. The document's shape is `schemas/v1/tasks.json`,
derived from the Rust types and gated like the others.

## Self-hosting

`aplan` plans, verifies, and renders `aplan`. Its own plan lives in
[`aplan/tasks/`](aplan/tasks/), and `aplan check` and `aplan verify` run clean on it on every
push (the "aplan lints its own plan" CI job), so it is a plan the tool itself vouches for,
evidence resolved against git, not merely asserted.

Including the advice above: this repository's `.aplan.yml` sets `deny_warnings`, because its
gate is automatic and nothing here reads a green log. Until it did, the `detect_drift` it
switches on deliberately could report a drifted section under a passing build, which is the
trap the *In CI* section describes. Its `require_min_strength: [test]` is the narrow end of the
progression the *Configuration* section describes, reached after `A1-120` configured a runner per
crate: a plan with nothing to cite yet should start at the wide end, not here.

The roadmap self-hosts too. Work is grouped into **cuts** (release slices), each a
**milestone** meta-task `blocked_by` the tasks in its cut (`APLAN021`/`APLAN022` keep that
model honest); `aplan roadmap` renders the plan grouped by cut, with per-cut rollups and the
critical path. From that, [`ROADMAP.status.md`](ROADMAP.status.md) is *generated* (`cargo
xtask gen-roadmap`) and gated whole-file (`gen-roadmap --check`), exactly as the JSON Schema
is: where the committed file and the plan disagree, CI fails and the plan wins, so the
roadmap's status can never quietly lie about what is done (ADR-0009). [`ROADMAP.md`](ROADMAP.md)
stays the hand-authored *rationale*; the live status is its generated companion.

No task or rule count is quoted here **unless a gate reads it**: a hand-written number is the
drift this tool exists to catch, and it has drifted twice already. There is exactly one, in
*In CI* above, where the argument does not land without it -- how much of this tool warns
rather than blocks -- and a test reads that sentence in all four documents that make it and
compares both numbers against `aplan facts`. Run `aplan facts` yourself for the
machine-readable list of every rule, format, and command the tool enforces (committed as
`facts.json`, gated in CI).

## Documentation

- [`ROADMAP.md`](ROADMAP.md), the release sequence and validation plan.
- [`HLD.md`](HLD.md), the technical design and evidence model.
- [`docs/adr/`](docs/adr/), architecture decision records.
- [`CONTRIBUTING.md`](CONTRIBUTING.md), build and test guidance.

## License

Apache-2.0 OR MIT.
