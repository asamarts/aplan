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
| `APLAN008` | `traces` anchors resolve to a real heading, table row, or YAML/JSON node, in a real document |
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
| `APLAN045` | A *live* task's traced section has changed since anybody last touched the task, so its premise moved while it waited. `APLAN018` watches a closed task against the stamp taken at closure; a live task has no stamp, so nothing watched it at all and work can ship under it unnoticed *(warning, `verify`)* |
| `APLAN046` | A `done` task whose ratifier has not finished. `ratified_by` is the half of a dependency that constrains *counting done* rather than *starting*: `ready` ignores it, so a task awaiting a blessing is still offered, and only its `done` claim is refused *(error, `check`)* |
| `APLAN047` | A `contains` that cannot be walked: it names nothing, names a task rather than a milestone, or closes a loop. Containment is how a checkpoint says the work beneath it rolls up into it; a milestone in `blocked_by` *sequences* two cuts instead, and the two meanings are what this separates *(error, `check`)* |
| `APLAN048` | The version this tree declares is smaller than the tasks closed since the last release tag require -- a breaking change shipping in a patch release, found while it is still preventable rather than by a consumer. It fires on a version that moved and did not move far enough, never on one that has not moved yet. Off unless `version.current` declares how to read the version *(error, `verify`)* |
| `APLAN049` | The compatibility checker this repository declares found a break the plan does not claim. Every rung above this takes the author's word: a task that removed a public function and wrote `fix` passes them all. It runs the checker you declare -- `cargo semver-checks check-release` exits non-zero on a break -- and reports what it said; it never diffs an API itself. Silent on an over-claim, and on a checker that could not be asked, which is reported as the configuration fault it is *(error, `verify`)* |
| `APLAN050` | A recorded `version` that is not on a release, or is not a version this tool can read back. A release carries the version it produced, written when it closed, so a later run can ask what a release contained without re-deriving from a plan that has since moved. A cut is a planning bucket rather than a delivery, and a `version` on one reads as a claim that it shipped *(error, `check`)* |
| `APLAN051` | A tag one edit away from one of the five words that change what this tool does: `milestone` makes a task a cut, `release` makes it a checkpoint over cuts, `deprecation` and `removal` drive the runway rules, and `cross-cutting` waives `APLAN022`. A typo does not misname anything -- `milestne` dissolves the cut, and `deprecatoin` makes `aplan deprecations` answer that the plan declares no runway, at exit 0. Also reported the other way: a `tag_families` entry that lists one of the five is declaring a vocabulary over a word the tool already acts on *(warning, `check`)* |
| `APLAN052` | `cross-cutting` on a task that *is* a member of a cut. The tag is `APLAN022`'s escape hatch and means "belongs to no cut", so on a task in one it cannot mean what the rule reads it as -- and it switches that check off, so the task could fall out of its cut or into two and the plan would still be clean. On a task in no cut the same tag is what the rule reads it as, and is silent: that asymmetry is deliberate *(warning, `check`)* |
| `APLAN053` | A task that is `in_progress` and whose declared `paths` match nothing the repository tracks: it says work has started and there is nothing to show for it. The one status the product's own argument could not otherwise defend -- `done` is believed because `verify` resolves what it cites, and a stored `in_progress` has nothing to resolve against, so it is checked rather than trusted. Read from the index, so a file written and staged counts; silent where the task declares no `paths`, since there is then nothing to judge the claim by *(warning, `verify`)* |
| `APLAN054` | A `done` task's mutable `file` evidence has changed since it was recorded, or is missing from an otherwise present evidence baseline. `APLAN015` catches a reference that no longer resolves; this catches the same reference silently naming different proof. Whole-file citations compare the exact committed blob, including binary files; anchored citations compare only the cited section. Dirty worktree bytes do not count as evidence until committed. An `@rev` that resolves as the object id it spells is immutable; `@HEAD`, branch and tag names (including hex-named refs) are stamped because they can move. Historical tasks with no stamps remain silent *(warning, `verify`, opt-in via `detect_drift`)* |
| `APLAN055` | A `captured` date appears only on authored `note` evidence. On a `commit`, `file`, `url`, or `test`, the date is not bound to immutable identity, reviewed content, a resolver observation, or an executed subject, so accepting it would make ignored metadata look like freshness proof *(error, `check`)* |
| `APLAN056` | A task's `owner` belongs to the repository's declared `owners` vocabulary. With no declaration, owners remain free text. Once declared, every present value must match exactly; an unfamiliar value is reported and a one-edit spelling gets a correction. Unassigned tasks remain valid *(warning, `check`, opt-in)* |
| `APLAN057` | Every `.inbox` entry is one canonical, valid and cross-record-consistent `aplan/capture/v1` record. Syntax, filename/id agreement, UTC whole-second instants, configured kinds, lifecycle fields and source uniqueness are checked without printing opaque source keys. Captures are never tasks or graph nodes *(error, `check` and `verify`)* |

`APLAN023` is the one the first outside pilot produced. `paths` is optional, and `APLAN012`
binds a cited `commit` to a task by checking its diff against those globs, so a task with no
`paths` has nothing to compare against: the citation resolves and proves only that `HEAD` reaches
some commit. That happened on seventeen of thirty-three tasks in a real repository while `check`
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

`aplan` models **one task-membership level**, the cut. A project-specific second dimension is a
declared tag family rather than another containment field (ADR-0014), while the built-in numeric
phase is only an ordered reporting coordinate (ADR-0046). A stage, a lifecycle, or a per-group
state is a tag such as `artifact:accepted`, because `Status` carries a verdict about work and a
stage does not.

Where a project has that second level, `roadmap.group_by` names the family and each cut renders
under its headings, with an explicit `(no group)` section for tasks carrying none, since putting
them under a heading they do not have would be a lie. A plan that declares nothing renders exactly
as it did before the option existed.

`APLAN026` is opt-in and exists because a second task-membership dimension ends up in tags. ADR-0008
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
`APLAN018`, `APLAN022`, and `APLAN054` are the opt-in rules below; `APLAN016` (url reachability)
and `APLAN019` (`paths` globs match a tracked file, once the task cites evidence) run
under `verify`, the first
only where `.aplan.yml` sets `url.check`. A rule code is a
contract, so it is not renumbered for tidiness.)

The following policy rules are **opt-in**, off unless `.aplan.yml` turns them on.
`APLAN017` requires a `done` task's evidence to meet a configured minimum strength,
so a real project can decree that a bare `note` does not close a task the way a
`commit` or a `test` does. `APLAN018` detects *stale-`done` drift*: `aplan close`
stamps a content fingerprint of each traced section, and `check` warns (advisory)
when a closed task's traced section changes afterward, cleared by re-reviewing and
`aplan close --restamp` (which refuses while drift detection is disabled rather than erasing the
baselines it cannot refresh). With the same `detect_drift` switch, `APLAN054` stamps each
mutable `file` citation and `verify` warns when the committed referent changes under
the same citation. `APLAN022` requires every task to be a member of exactly one
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

**An anchor may address a YAML or JSON node** (ADR-0035). A repository whose source
of truth is a spec file rather than prose can put it under drift detection directly,
instead of tracing the paragraph that narrates it -- which is a second copy of the
fact, in a tool whose argument is that second copies drift. The document's extension
picks the resolver, so nothing about `#` becomes ambiguous:

```yaml
traces:
  - spec/resource-states.yaml#/states/provisioning     # JSON Pointer, RFC 6901
  - spec/openapi.json#$.paths['/users'].get            # JSONPath, RFC 9535
```

`spec.yaml#$` and `spec.yaml#/` address the document itself, which is the one
whole-file trace drift can watch: a markdown trace with no anchor cannot be
fingerprinted, and a structured one that names the root can.

The fingerprint is of the node's *value*, canonically: reindenting the file,
reordering two keys or reflowing a comment is not drift, while changing, adding or
removing a value is. Only JSONPath expressions that address a single node are
accepted -- names and whole-number indices. A wildcard, descendant, slice, union or
filter is refused by name, because a fingerprint is taken of one node, and because
this tool resolves an address rather than running a query.

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
| `commit` | An immutable git SHA reachable from this repository's `HEAD` |
| `url` | An external URL, a filing receipt, a regulator page, a vendor quote |
| `file` | A repo-relative path to a committed artefact, optionally naming the section that proves it (`docs/report.md#result`) |
| `test` | A named test that must pass |
| `note` | A dated human note. The weakest kind; requires `captured:` |

An undated note is a rumour, so `APLAN010` rejects it. `commit`, `file`, and
`test` are the kinds `aplan verify` resolves against the repository (below); the
rest are checked structurally.

`captured` is deliberately note-only (`APLAN055`). Its author or importer records when the inline
human assertion was made. `close` and the resolvers do not invent that provenance. A commit already
has immutable identity; a mutable file is tied to reviewed content by its `APLAN054` fingerprint;
and URL or test resolution describes the current run. A past test execution needs a subject-bound,
digest-addressed report with producer and execution identity, not an editable date beside the test
name.

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
| **L1** Resolved | Does the referent resolve for the verified subject? | `aplan verify` |
| **L2** Relevant | Does it connect to *this* task? | `aplan verify` |
| **L3** Executed | Does the cited test *pass*, now? | `aplan verify --execute`; focused `aplan close --execute` |

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

A `commit` SHA must resolve to `HEAD` or one of its ancestors (`APLAN011`), and
its diff must touch at least one of the task's declared `paths` (`APLAN012`).
An object that exists only through an unmerged branch, linked worktree or
reflog is not evidence about the verified tree. This local ancestry check does
not contact a remote or prove that `HEAD` has been pushed. A `file` must exist
at the cited rev, or at `HEAD` (`APLAN015`); a `test` must be one the configured
runner lists (`APLAN013`), and under `--execute` it must pass (`APLAN014`).

A runner that *skips* the cited test exits zero too, and a zero exit is what
`--execute` reads as a pass. Declare `test.skipped` on the runner -- argv that
prints the tests it will not run -- and a task whose every test citation is one
of them is reported rather than believed (`APLAN014` again).

Discover the exact names before closing a task rather than reconstructing a
runner command by hand:

```console
$ aplan test list widget
runner #1 `python-unittest`: 1 of 184 matching
  test:package.tests.WidgetTests.test_handles_case
```

`aplan test list [query]` asks every configured runner through the evidence
resolver itself, using the same repository-relative `dir`, wrapper, timeout,
capture bound and name normalization as `close` and `verify`. The optional query
is a case-insensitive substring. Human, JSON and agent formats distinguish no
configured runners, a runner that answered with no tests, a failed runner and a
complete search with no match. A shared name is shown with every runner that
listed it and marked invalid as bare evidence; a runner failure makes the
catalogue partial and exits 1, so an absent result is never presented as proof
that the test does not exist. As on `verify`, a repository can explicitly put
`APLAN013` in `advisory`; the catalogue remains visibly partial but that
repository policy makes the finding non-blocking and the exit status zero.

When a component job already owns a complete test provider, it can export that execution as a
portable provider receipt instead of asking a later job to run the same tests merely to establish
L3 again:

```sh
aplan test receipt backend \
  --output backend.test-evidence.json \
  --producer-repository "$CI_REPOSITORY" \
  --producer-workflow "$CI_WORKFLOW:$CI_JOB" \
  --producer-run-id "$CI_RUN_ID" \
  --producer-event "$CI_EVENT" \
  --producer-privilege trusted
```

The runner is its one-based index or exact configured label. Receipt creation requires exhaustive
`evidence.observation_inputs`, lists the complete provider namespace, executes every listed
identity, and publishes only when every request has an attributable typed outcome and its material
and repository observations stayed stable. Use `--output -` for canonical JSON on stdout.

This is an in-toto-style `aplan/test-evidence/v1` provider statement, not a cached plan verdict.
It binds the runner/adapter, catalogue, exact input declaration and matched bytes/modes, inherited
environment, toolchain, executable and existing file-argument bytes (including wrapper scripts),
aplan build identity, producer and audit timestamps. It deliberately contains no plan identity: a
consumer must derive the current plan verdict again. Only an unconditional pass proves L3; skips,
expected failures, retries, flakes, cancellation, timeouts, resource stops, framework crashes and
incomplete output remain distinct non-passing outcomes. A successful invocation indistinguishable
from the runner's exact-name miss sentinel stays `unproven_selection`, so receipt-backed
verification reproduces the direct `APLAN037` refusal rather than upgrading a false green.

The producer fields in the JSON do not authenticate themselves. A local caller may trust a file it
controls; CI must separately bind the artifact digest and authenticated repository, workflow, run,
event and privilege, or authenticate a DSSE envelope. A digest is integrity, not producer identity,
and an untrusted pull-request job must use `--producer-privilege untrusted`. Receipt consumption by
whole-plan verification is explicit and current-only. Trusted orchestration writes one import set
entry per configured provider, for example for locally controlled files:

```json
{
  "schema": "aplan/test-evidence-import-set/v1",
  "receipts": [
    {
      "provider": "backend",
      "path": "backend.test-evidence.json",
      "trust": { "mode": "local_file" }
    }
  ]
}
```

```sh
aplan verify --test-receipts test-evidence.import.json
```

The import set is not self-authenticating. Do not accept trust claims from a manifest supplied by
untrusted repository or pull-request code; create it from authenticated artifact or signature facts
inside the trusted consumer job. Verification requires one row for every configured provider,
recomputes current provider/material/tool/repository identity, and derives a new verdict over the
exact current plan. It starts no provider command and never silently falls back. Missing, stale,
partial, untrusted or incompatible observations are typed findings; choose `--execute` explicitly
for full direct replay. The derived import-set schema is
`schemas/v1/test-evidence-import-set.json`.

To refresh a locally controlled receipt set without narrowing the plan verdict, give `verify` an
explicit Git boundary:

```sh
aplan verify --refresh-test-receipts test-evidence.import.json \
  --refresh-base origin/main --refresh-head HEAD
# Or include HEAD, index, tracked edits and untracked files explicitly:
aplan verify --refresh-test-receipts test-evidence.import.json \
  --refresh-base HEAD --refresh-working-tree
```

The safe default refreshes a complete invalidated provider. A runner can opt into exact-test
selection with `inputs_by_test`, an exhaustive dependency claim separate from proof coverage:

```yaml
test:
  name: workspace
  list: [cargo, test, --workspace, --, --list]
  run: [cargo, test, --workspace, --, --exact, '{test}']
  inputs_by_test:
    - tests: ['core::*', 'integration::core_*']
      paths: ['crates/core/**', 'fixtures/core/**']
    - tests: ['web::*']
      paths: ['crates/web/**', 'fixtures/web/**']
```

Every listed test and every relevant changed input must have exactly one owner. Missing or
overlapping ownership, a new test or path, config/runner/toolchain/material movement, or another
unknown edge widens visibly to the complete provider. Plan-only edits outside declared provider
material still re-derive every current citation without rerunning unchanged outcomes. Refreshed
documents are published under content-derived names and the complete manifest is replaced last;
failure leaves the previous manifest in place. Put that set outside the checkout, or in a
Git-ignored directory outside `evidence.observation_inputs`, so its own publication cannot become
the next change boundary. The mode reports reused, refreshed, widened and
unavailable providers plus selected-test counts in human and machine output. It refreshes only
`local_file` rows; authenticated artifacts and signed receipts must be regenerated inside their
authenticated producer. `--refresh-shadow` compares the answer with complete direct execution for
rollout evaluation, but does not turn the incremental result into release evidence (ADR-0058).

This repository's rollout audit is an executable example rather than a generic GitHub-specific
product requirement. `.github/workflows/evidence-rollout.yml` produces receipts and a direct oracle
in one trusted job, then a dependent job validates a bounded artifact and reconstructs
`same_workflow` trust from authenticated workflow context. Its one `APLAN_EVIDENCE_ROUTE` value
switches receipt consumption back to direct execution. Other CI systems should preserve those
boundaries - independently invocable production and direct routes, authenticated producer context,
content-bound transport, zero-provider receipt verification and a retained periodic direct oracle -
without copying GitHub's artifact mechanism as part of the receipt format.

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
with `detect_drift: true` -- reports if a mutable `file` citation changes under the same
reference (`APLAN054`, opt-in). A `traces` anchor independently watches the premise named by the
task (`APLAN018`). Whether the decision was right is a review, and no tool here pretends otherwise.

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

An `add --from` entry may also make existing or explicitly named same-file work wait on it with
`blocks`. That reverse edge and every created task are one publication:

```yaml
- id: T-003
  title: Implement the shared gate
  effort: 2
  blocks: [T-existing-gate, T-004]
- id: T-004
  title: Exercise the gate end to end
  effort: 1
```

Blueprint V1 has the same reverse relationship, using its explicit reference namespaces:
`blocks: [task:T-existing-gate, local:exercise]` makes an existing task and the entry whose key is
`exercise` wait on the declaring entry.

| the new cut holds | how |
|---|---|
| work that does not exist yet | another `--from` file, the milestone entry and its tasks together |
| work the plan has, in no cut | one command: `aplan add "Second cut" --id M-v0.2 --tag milestone --blocked-by T-003` |
| work that is in another cut | one guarded move: `aplan add "Second cut" --id M-v0.2 --phase 2 --tag milestone --member T-002` |

`--member` is deliberately narrower than `--blocked-by`. Each repeated member must be live work
in exactly one existing cut; `add` removes it there and creates the destination milestone listing
it in one compare-and-swap batch. In that mode `--blocked-by` names only milestone predecessors,
so `--blocked-by M-v0.1` sequences the new cut without confusing sequencing with membership. The
new cut's `--id`, `--phase` and `milestone` tag are required decisions. A duplicate, terminal,
cross-cutting, unowned, multiply-owned or milestone member is refused, as is a move which would
leave a source cut with no live work.

An entry takes the fields `aplan add` takes as flags, plus `evidence` and `body`: live `status`,
`prefer_after`, `ratified_by`, `contains`, `revisit_at`, `version_impact` and its reason all land in
the same atomic post-state. A `done` entry is a historical-ledger correction, not ordinary forward
authoring: it needs evidence which resolves before any file is written and may then join a `done`
cut without reopening it. `dropped` is deliberately refused at creation because it records a later
decision about work which existed; use `drop` for that decision. Recorded release `version` and
fingerprint maps remain derived rather than authored.

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
aplan verify --test-receipts receipts.import.json # current L1/L2/L3, zero provider spawns
aplan verify --refresh-test-receipts receipts.import.json --refresh-base origin/main --refresh-head HEAD # selective provider work, whole-plan verdict
aplan verify --fix             # apply exact computed evidence repairs as one guarded batch, then verify again
aplan test list widget         # discover exact configured test: evidence names and their runners
aplan ready                    # workable tasks whose blockers are all done, best first
aplan ready --explain          # and why each one is where it is
aplan ready --phase 2          # narrow a report by phase; cut, tag, owner and status work too
aplan why P1-003               # what blocks a task: its own blockers, and any unfinished cut
aplan version M-v0.9      # what that cut owes the released version: fix, feature, breaking
aplan why P1-003 --all         # ... walking through those cuts into their membership too
aplan paths                    # how much of recent history each task's `paths` would match
aplan stale --older-than 30    # live tasks that have sat -- including parked ones, and how long the park has held
aplan due --as-of 2026-09-10T06:00:00Z   # parked tasks whose `revisit_at` has arrived: due for a look, not unblocked
aplan vacuity                  # per rule: what it checked, and what it could not have failed on
aplan deprecations             # runways: announced in one cut, removed in a later one
aplan attest                   # what this plan proved, bound to the commit it proved it at
aplan check-docs               # check the documents declared by `docs.paths`
aplan check-docs NOTES.md      # one-off override: check exactly this document
aplan export-roadmap-json roadmap.json  # regenerate a foreign status contract from the plan
aplan export-agents-md         # the agent contract your policy implies, fenced
aplan export-agents-md --check AGENTS.md   # ... and the gate that keeps it true
aplan summary                  # tasks, effort, and ready count by phase, or summary.group_by
aplan summary --format json    # the same, machine-readable
aplan summary --at v0.4.0      # the committed plan and discovered config at a revision; resolved SHA on stderr
aplan tasks --format json      # the plan as data, so any other rollup is a `jq` line
aplan explain rule APLAN012    # the stable meaning and severity behind a diagnostic code
aplan critical-path            # the longest effort-weighted chain of dependent tasks
aplan critical-path M-v0.9     # ... and what must finish before one checkpoint can,
                               #     with how much of that chain is not yet done
aplan roadmap                  # the plan grouped by cut: rollups + per-cut critical path
aplan roadmap --format markdown # the committed ROADMAP.status.md companion (ADR-0009)
aplan roadmap --format markdown --regenerate-with 'make roadmap'  # when a wrapper generates it
aplan capture "Follow up on pilot friction" --label pilot --body-file notes.md # guarded durable intake, not a task
aplan inbox                    # actionable open captures; deferred intake is counted separately
aplan inbox show I-01K8Z3A4    # focused review accepts one unambiguous human id prefix
aplan inbox discard I-01K8Z3A4 --reason 'Already addressed' # retain an explicit terminal disposition
aplan add "Second cut" --id M-v0.2 --tag milestone --blocked-by T-003  # a cut is created with its work
aplan add "Public CI" --id M-public --phase 2 --tag milestone --member T-003  # move existing work into a new cut atomically
aplan add "Wire the sequencer" # scaffold the next task file, in this plan's own id convention
aplan add "Fix what the log found" --traces pre-public-adoption-evidence#24-a-finding   # and record what caused it
aplan add "Ship it" --cut M-v0.6 --effort 2 --paths 'src/**'   # and place it in a cut
aplan add "Wait for review" --status blocked --revisit-at 2027-01-15T12:00:00Z # live state and scheduling at creation
aplan add --from plan.yml      # a written plan or proved historical ledger as one atomic change
aplan blueprint apply --file feature.yml --cut M-v0.9 --set feature=bulk-close --dry-run # predicted ids, exact input digest, no write
aplan blueprint apply --file feature.yml --cut M-v0.9 --set feature=bulk-close --format agent # apply once; report exact published ids
aplan close P1-003             # set done + attach the HEAD commit, in place
aplan close P1-003 --evidence commit:d213c9a --evidence test:mod::name
aplan close P1-003 --evidence test:mod::name --execute # run only this proposed closure's tests; require L3
aplan amend P1-003 --replace-evidence test:old_name test:new_name   # repair a closed task's citation
aplan supersede P1-003 "Re-fix the rail" --clear-traces --body '## Why ...' # control inherited successor scope and prose
aplan dep add P1-004 --after P1-003     # P1-004 comes after P1-003; `dep rm` takes it back, and a cut's membership stays `mv`'s
aplan dep add P1-004 --after P1-003 --after P1-002  # repeat an operation for one atomic target rewrite
aplan dep apply --from edges.yml --dry-run --format json # mixed multi-target add/remove, judged as one final graph
aplan tag add P1-004 cli                # put a tag on an existing task; `tag rm` takes it off
aplan tag rename epic-a epic-b          # ... and rename one across every task that carries it, as one change
aplan drop P1-003              # cancel a task, recording the decision rather than deleting it
aplan drop --cut M-v0.6        # cancel a whole cut: its members, then the milestone
aplan park P1-003 M-v0.7 --reason 'Waiting for a funding decision' --revisit-at 2027-01-15T12:00:00Z --tag post-mvp
aplan unpark P1-003 M-v0.7     # return all named parked work to todo, preserving history and tags
aplan mv P1-003 P1-004 --cut M-v0.7   # move tasks between cuts, editing both milestones or neither
aplan mv P1-003 --phase 2      # change its reporting coordinate, never its stable id
aplan rekey P1-003 P1-103      # change one stable id and every structured reference atomically
aplan claim P1-003             # local ephemeral claim shared by this repository's linked worktrees
aplan claim P1-003 --from ada  # checked takeover; task owner remains unchanged
aplan release P1-003           # explicit recovery; claims never expire silently
aplan start P1-003             # record that work has begun; APLAN053 reports one the repo contradicts
aplan stop P1-003              # record that it has stopped, back to todo (not a closure)
aplan ready --unclaimed        # ... and only work with no observed active claim
aplan import kep.yaml          # read a Kubernetes KEP into a task (lossy, and says so)
aplan import ROADMAP.md --format checklist     # a markdown `- [ ]` roadmap, the commonest shape
aplan import roadmap.json --format roadmap-json # a status contract, depends_on -> blocked_by
aplan import kep.yaml --dry-run --cut M-v0.6   # preview, and place it in a release cut
aplan export --format checklist --out ROADMAP.md  # write the plan back out as the checklist it came from
aplan export --format checklist --check ROADMAP.md  # ... and the gate that keeps it true
aplan mcp                      # MCP server over stdio: identity, ready, why, check, verify,
                               #   add, show, dep, close, claim, release, start, stop
aplan init                     # scaffold a starter .aplan.yml (never overwrites)
aplan graph                    # attended layered graph; a redirect stays Mermaid for compatibility
aplan graph --format text --width 120  # deterministic redirected layered document
aplan graph --format text --focus A1-126  # emphasize one visible task and its direct neighbours
aplan graph --format text --critical      # overlay the deterministic whole-plan critical route
aplan tui                      # overview/tasks/diagnostics/graph/reports/inbox; Ctrl-N captures
aplan tree A1-126              # one task's neighbourhood: what precedes it, what it releases (`--depth N`)
aplan show A1-126              # one task whole; reads observations and never starts providers
aplan show A1-126 --refresh-evidence       # explicit slow provider refresh; progress is on stderr
aplan show A1-126 --require-current-evidence # render, then exit 1 unless every observation is current
aplan show A1-126 --no-resolve             # skip stored and repository evidence resolution too
aplan schema                   # the JSON Schema, derived from the Rust types
aplan facts                    # the tool's rules/formats/commands as JSON (see facts.json)
```

Use a blueprint when a reviewed task shape has recurring fields or sibling topology, not as a
substitute for authoring a plan from its evidence. V1 reads one explicit local file: repeat
`--set` exactly once for each parameter it declares, use `local:<key>` for a new sibling and
`task:<id>` for an existing task, and select one existing cut for the complete graph. The file is
plan-authoring data rather than an executable program, but it can still propose every task and
relationship in that graph. Review the dry run's source path and digest, predicted tasks, ids,
files and findings before rerunning the same command without `--dry-run`; only the successful
apply makes its reported ids durable. V1 has no discovery, catalogue, composition, replay, built-in
methodology, installation or fetch path.

### TUI quick start

The standard binary includes the interactive interface:

```sh
aplan facts --require build.tui
aplan tui
```

Use `1` through `6` for Overview, Tasks, Diagnostics, Graph, Reports and Inbox, `Ctrl-N` for a
minimal capture without leaving the current destination, `Ctrl-P` to find actions, `?` for
contextual help, `Enter` to open, and `Esc` to reverse one layer. Mutations use
typed forms, exact previews and guarded confirmation; they edit the working tree but never create
a Git commit. The [TUI user guide](docs/tui.md) covers the complete work loop, bounded graph
traversal, progress, responsive layouts, accessibility and recovery. The generated
[action reference](docs/tui-reference.md) lists every binding, native mutation and deliberate
CLI-only boundary.

The `check-docs` reference vocabulary comes from the plan. Alongside numeric and dotted families
such as `A1-042` and `M-v0.8`, it learns when the numeric core continues through hyphen-separated
suffixes: a plan containing `M-p0-design` and `M-p0-build` resolves those complete ids and reports
an unknown `M-p0-review` whole, rather than inventing a dangling `M-p0`. Require that scanner
contract before depending on it with `aplan facts --require check_docs.complete_multi_hyphen_ids`.
Word-only ids and ULIDs still teach no safe prose pattern, so a plan containing only those makes
the command refuse instead of claiming it checked text it could not distinguish.

The same longest-family rule applies when `check-docs` recognizes a one-digit typo in a family
prefix: an overlapping `A11-999` candidate is never shortened to an invented fragment. Consumers
that depend on the exact diagnostic subject can require
`check_docs.near_prefixes_prefer_longest_family` through `aplan facts --require`.

`ready`, `tasks`, `summary`, `due`, `stale`, `vacuity`, `paths`, and `graph` share five repeatable
selectors: `--cut`, `--phase`, `--tag`, `--owner`, and `--status`. Values within one dimension
widen (`--phase 1 --phase 2`); different dimensions narrow. `--unclaimed` additionally asks for
work with no observed ephemeral claim. It is independent of `--owner`: a team can remain
responsible for work nobody is actively doing. Claim state is private to the local Git common
directory, shared by linked worktrees, and has no automatic expiry; a non-Git plan stays readable
but `--unclaimed` refuses because unavailable coordination is not evidence that work is free.
Selection changes what is shown, never what is computed, so a task selected by phase remains
blocked by a prerequisite outside that phase. `check` and `verify` accept these flags only to
refuse a qualified gate rather than let a partial plan read as clean. The same durable equality
model scopes repository policy under `policy[].when`.

The text graph keeps scope and emphasis separate. `--focus <ID>` marks an already visible task,
its visible direct dependency blockers (`B`) and dependents (`R`); it never pulls the task through
a selector. `--critical` projects core's deterministic whole-plan route (`C`) onto the current
view, with `>` on the node entered by each exact highlighted step. Status remains an exact word,
and a legend explains every mark, so colour adds hierarchy but no information. A terminal receives
Unicode box drawing and stream-resolved colour; a redirect receives deterministic ASCII and plain
text unless colour was explicitly forced. These two presentation flags require `--format text`.
Use JSON, DOT or Mermaid when another program needs the graph rather than parsing the drawing.

Commands that mutate a plan write into the working tree for you to review and stage; they never
create a commit. Commands with a different write target say so in their own help: exports can
write the output path you name, and `attest --attach-note` writes an explicit Git note. This is a
shared safety guarantee, not an exhaustive hand-maintained list of writers.

`verify` normally only reports. Where a finding has one exact evidence replacement, it prints a
separate `repair:` line and the JSON and agent forms carry the structured operation. `--fix` is the
explicit write: it resolves every replacement as `amend` would, judges the combined post-state,
publishes all affected task files or none, then reruns `verify`. Advice that still needs a choice
has no typed repair and is never changed: choosing a new id, correcting versus removing a document
anchor, choosing between several near matches, collapsing duplicate citations, superseding an
outcome, and accepting a changed trace premise remain human decisions.

`close` changes one task file in place, the `status:` line and a new `evidence:` block, and leaves
the rest byte-for-byte untouched; it refuses to author a false record: an unknown or still-blocked
task, one already `done` or already carrying evidence, or a `note` (which needs a `captured` date it
cannot invent). Ordinary close resolves test names at L2 and never runs them. Explicit
`close --execute` runs exactly the proposed task's distinct test citations, not historical plan
evidence, and requires every typed outcome to be an unconditional L3 pass. It binds the preview to
the plan, changed repository material, selected configuration and selected provider runtime;
movement or any failed, skipped, ambiguous, missing, non-selecting, stopped or unavailable result
causes aplan to publish no plan mutation. The configured test command is trusted code and its own
effects are not rolled back; inspect the working tree after it runs. The MCP close tool remains
L2-only. `init` refuses to overwrite an existing `.aplan.yml`, so it never clobbers a policy
you have already committed to.

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

For a local performance diagnosis, the global `--timings` flag appends one line of phase timings and
non-sensitive deterministic work counters to stderr. It does not change stdout or the command's exit
status. The counters describe configuration and plan loads, candidate/read/parsed task files, retained
source bytes, graph constructions, persistent-cache hits/reuse/bytes, and traced-document reads,
bytes and Markdown/structured parse attempts. Actual output bytes and time to first byte belong to the external
`ci/scripts/read-performance.py` harness so every output path is measured rather than estimated.

Read commands use a disposable per-user cache by default. It lives in the operating system's cache
directory (or `APLAN_CACHE_DIR` for a hermetic host/test override), never in the repository, and is
only an optimization: incompatible, corrupt, unavailable or contended state is a quiet miss. Use
global `--cache=off` to read and write no persistent cache state, or `--cache=refresh` to ignore the
old generation and publish a replacement. Cache files are private, bounded, checksummed and safe to
delete. Their checksum detects corruption; it is not authentication against another process running
as the same user.

Evidence-provider observations use a separate private cache namespace and never replace the
authoritative gate. Plain `show` reads a complete current raw test catalogue if one exists and
otherwise reports typed `not_observed`, `stale`, `refresh_failed` or `unsupported` state promptly;
it does not start `test.list` or `url.check`. Use `--refresh-evidence` when waiting for configured
providers is intended, `--require-current-evidence` when a script requires freshness, and `verify`
for the authoritative whole-plan answer. JSON consumers must accept `aplan/show/v3`; its retained
`resolved` field is true exactly when the aggregate observation is `current`.

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
says a parked task is parked and points there, and `aplan park`/`aplan unpark` make the human
decision as one guarded all-or-none change. Parking accepts only unclaimed `todo` tasks and
milestones; unparking clears `revisit_at` but preserves body history and tags, and unfinished graph
blockers continue to govern `ready`. Every command that reports on the plan (`ready`, `why`, `graph`, `summary`,
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
- **Agents** get `--format agent`, where every diagnostic carries an
  `agent_instruction` string that can be acted on without reading the source, or `aplan mcp`, an
  MCP server over stdio with a deliberately bounded tool surface. The exact inventory in `aplan
  mcp --help` is checked against the executable server registry; `aplan facts` exposes the same
  inventory to machines. The surface covers server identity, selecting and inspecting work,
  validation, bounded task and edge mutations, evidence-gated closure, and coordination. Broad
  rollups, host operations and mutations that require human judgement remain CLI-only by an
  executable command census.

  Its `close` goes through the same evidence gate as the CLI's, and takes no score, confidence, or
  completeness input: an agent supplies a *citation*, never an assessment of its own work.
  Refusing that is the point, and it is pinned by a test.

  `identity` is the one that answers about the *server* rather than about a plan, and it
  exists because **asking an installed CLI does not identify a configured MCP process**:
  the two can be different executables, and a pilot spent a pass diagnosing a defect its
  own checkout had already fixed. It returns the same document `aplan facts --identity`
  prints, and needs no plan, no repository and no clean tree -- the reason to ask is
  usually that something else did not work.

  Every plan tool's optional `root` selects a complete context, not just another directory of
  Markdown. Pass either a repository directory containing `.aplan.yml` or that repository's
  configured plan directory; a nested partial directory is refused. The server resolves that
  selection's plan root, configuration-relative traces, warning and vocabulary policy, evidence
  providers, ID strategy and Git worktree together. An explicit-root result is wrapped as
  `{ "context": ..., "result": ... }`; its context paths are repository-relative and state
  whether a symlink was resolved. Omitting `root` preserves the launch context and its historical
  payload shape. A root containing `..` and a symlink both mean their canonical target; a missing
  path, non-directory, mismatched configured root, or ambiguous config-free subdirectory fails
  closed and never borrows the launch repository's policy.

  Treat an explicit repository root as a trust choice. Its validated `.aplan.yml` is the policy in
  force. `aplan_show` never starts configured providers; `aplan_verify` may run configured
  test-list, URL and version providers but still never executes a cited test. Do not let an
  untrusted caller select arbitrary host directories. In a config-free Git worktree only the
  worktree root (using its `aplan/` child) and
  that child are accepted. An existing directory outside Git remains a config-free plan root, with
  its parent as the repository fallback. The launch context and every explicit context honour
  `traces_root` and `no_traces`; `aplan_show` resolves historical traced-document findings in that
  same frame.

  A server that entered identity-only mode because its launch configuration could not be read
  still refuses every plan tool. An explicit `root` is not a way to bypass that startup refusal;
  call `aplan_identity`, correct or upgrade the configured server, and restart it.

Slow human CLI commands use one delayed stderr progress row across plan loading, repository work,
configured providers and final projection. `--progress auto` shows it only when both streams are
attended terminals outside CI; `always` opts in beside redirected or structured stdout, and
`never` is the explicit zero-byte contract for agents and scripts. Concurrent providers appear as
one bounded count without exposing configured names or output. `NO_COLOR`, `TERM=dumb` and
`--color never` retain meaningful plain or non-animated forms, and progress never changes stdout
or the command verdict.

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
  batch:                       # optional one-process adapter; `run` stays as fallback
    protocol: 1
    run: ["./ci/aplan-test-batch", "{manifest}", "{result}"]
  inputs_by_test:              # optional exhaustive refresh dependency map; not proof coverage
    - tests: ['unit::*']
      paths: ['src/**', 'fixtures/unit/**']
  skipped: ["cargo", "test", "--", "--list", "--ignored"]   # tests it will not run (APLAN014)
  timeout_seconds: 600          # how long `list` may take before the runner is reported
  max_output: 1M                # bytes retained from each output stream; default 1M
docs:                           # the default input to `aplan check-docs`
  paths: [README.md, 'docs/*.md'] # literals or globs; relative values use this file
  membership:                   # these documents join `paths` automatically
    - doc: ROADMAP.md
      declared_by: line
      marker: "**Tasks:**"
owners: [ada, grace]            # canonical task owners (APLAN056); unassigned stays valid
capture:                        # repository-portable intake organization; no user-local defaults
  kinds: [question]             # appends to built-in task, idea and note
  profiles:
    triage:                     # selected explicitly by a capture creation command
      kind: idea
      labels: [inbox]
      body: "## Context"
      promotion:                # inert defaults; ordinary task validation still applies later
        phase: 2
        effort_days: 1
        tags: [feature]
evidence:                       # minimum evidence strength for `done` (APLAN017)
  require_min_strength: [commit, test, file]  # a note alone is not enough...
  exempt_tags: [phone]          # ...except on tasks tagged thus, where it is
  observation_inputs: [Cargo.toml, Cargo.lock, 'src/**', 'tests/**'] # exhaustive test-list inputs
deny_warnings: false            # make a warning a finding, so every gate blocks
advisory: [APLAN022]            # hear these rules in full, without being stopped by them
id_strategy: sequential         # or `ulid`, for authors who mint ids offline in parallel
```

`docs.paths` makes the document gate a repository decision instead of an argument list copied
into CI. `aplan check-docs` with no paths expands every declaration deterministically; each
literal must be a file and each glob must match at least one file, so a misspelling or a directory
that became empty cannot produce a vacuous green run. A single `*` stays within one directory and
`**` crosses directories. A document named by `docs.membership[].doc` is already declared and need
not be repeated. Passing paths explicitly overrides the declaration for a narrow one-off check.
Directories are not implicit inputs: declare a glob so additions to the intended set join the gate.

`owners` turns the task model's free-text `owner` into an opt-in repository vocabulary. With no
list, every spelling remains legal. With one, `APLAN056` reports any present owner outside it and
offers a correction when a declared value is one edit away; a task with no owner remains valid.
The same vocabulary validates `policy[].when.owner`, so one configuration cannot call a spelling
non-canonical and also use it as a policy boundary. The list is field-specific on purpose: `owner`
is currently the only free-text categorical scalar, so a generic string-field vocabulary would add
a second schema with one real member. An unused listed owner is silent because a team roster may
legitimately contain someone with no current task, and scalar/optional `owner` already supplies the
useful cardinality.

Capture intake is separate repository data below the resolved plan root: one strict YAML file at
`.inbox/I-<26-character ULID>.yml`. A missing directory is an empty inbox. `check` and `verify`
report malformed records as `APLAN057`, but task graph reads such as `ready`, summary and roadmap
prune the whole reserved subtree and never reinterpret an idea or note as work. The generated
contract is [`schemas/v1/capture.json`](schemas/v1/capture.json). Timestamps are UTC RFC 3339 at
whole-second precision; open, promoted and discarded are the complete lifecycle vocabulary.

`capture.kinds` adds lower-case ASCII kebab-case organization keys to the built-ins `task`, `idea`
and `note`. A named profile can default capture kind, labels and body plus promotion phase, cut,
effort, owner, paths and task tags. It cannot set identity, source, lifecycle, status, dependencies,
evidence, traces or commands. Applied effective values are snapshotted into each record, so a later
config edit cannot silently change promotion or source-retry equivalence. This is committed shared
policy; per-user config has no capture keys.

The shortest intake path creates one open `task`-kind capture without putting it in the task graph:

```sh
aplan capture "Investigate incremental graph layout"
aplan capture "Could this be simpler?" --kind idea --label ux
printf '%s' "$DETAILS" | aplan capture "Renderer edges" --body-file -
```

`--body`, `--body-file`, `--edit`, `--profile`, `--source namespace:key`, `--revisit-at` and
snapshotted `--promotion-*` defaults are optional. `--edit` parses `VISUAL` or `EDITOR` into an
executable and arguments and launches it directly, without a shell. Capture and disposition clocks
are supplied by aplan; only the review hint `revisit_at` is user-authored. Use `--` before a title
that begins with `-`.

`aplan inbox` lists actionable open captures newest first and always reports how many open captures
are deferred. `--kind`, `--state`, `--label`, `--revisit`, bounded `--search` and `--limit` filters
have stable JSON equivalents. `inbox show`, `amend`, `discard` and `restore` accept a canonical
unambiguous `I-...` prefix for a human, but every result carries the full id. Amend applies only to
open records; discard records an injected disposition instant and optional reason; restore applies
only to discarded records and clears that disposition.

`aplan inbox promote I-... --cut M-next --effort 1 --path 'src/**'` creates an ordinary `todo`
task from the capture title and body, then retains the capture as an immutable origin snapshot.
Snapshotted profile values are defaults: explicit phase, cut, effort, owner, paths and tags win.
The ordinary typed `add` constructor and complete post-state warning policy still decide whether
the task is valid. A dry run previews the task, cut and capture files and reports both full ids.
Publication holds the plan and complete inbox snapshot together; a race or late write failure
leaves neither side changed. Repeating an equivalent request returns the original task id without
creating another task, while a conflicting retry refuses. Promoted records are irreversible. V1
has no purge, delete, archive or automatic expiry command. Every mutation supports `--dry-run`,
uses the shared application `ChangeSet`, and rechecks complete inbox membership under one
publication lock so concurrent source retries and prefix changes cannot select or create the wrong
record.

An `owner` is durable responsibility, not a lock and not necessarily the person working now.
`aplan claim` records that shorter-lived fact outside the plan, under Git's common administrative
directory, so linked worktrees see one compare-and-swap record without dirtying task files.
`claim` and `release` never change `owner`; `--owner team --unclaimed` therefore usefully means
"team is responsible, but nobody has claimed it locally." See ADR-0051 for the failure and recovery
contract.

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

There are three separate layers of bounds because they constrain three separate things.
`max_memory` and `max_processes` constrain the child command through cgroups where the platform
permits it.
`output_lines` constrains how much of a failed test reaches a finding. `max_output` constrains the
bytes **aplan itself retains** while draining a configured command, defaults to 1 MiB for each of
stdout and stderr, and works on every platform. The reader keeps the first and last halves of that
budget and drains everything between them, so a chatty child neither grows the parent without bound
nor blocks on a full pipe; a failed-test excerpt still gets the tail where runners usually explain
the failure. When capture drops bytes, the diagnostic names their count and `test.max_output`.
Commands whose stdout is their answer (`test.list`, `test.skipped`, and `version.current`) fail
loudly on truncation rather than treating a partial catalogue or value as complete. The same
`max_output` key is available under `url` and each declared version command.

**Batch execution is optional and adapter-owned.** A runner with `batch.protocol: 1` receives a
private `aplan/test-batch-manifest/v1` JSON document at the whole-argument `{manifest}` path and
must publish `aplan/test-batch-result/v1` at the initially absent `{result}` path. The manifest
contains a nonce, a digest identifying the runner and every distinct cited identity that runner
owns. The result repeats the nonce and provider, declares `complete: true`, and reports exactly one
`passed`, `failed`, `skipped`, `stopped` or `unavailable` row for every request. Missing, duplicate,
extra, malformed or non-passing rows fail closed; a nonzero adapter exit cannot be overruled by a
file claiming passes. Names never enter argv, and aplan starts at most one adapter process for that
runner. The configured adapter owns framework selection and native report parsing; it should write
a sibling temporary file and rename it to `{result}` only when the complete answer is ready.

Legacy `run: ... {test}` remains required and has no deprecation date. It is used when `batch` is
absent and is the rollback path if an adapter must be removed. ADR-0054 specifies the bounded JSON
contract and maps pytest node ids, Vitest file/full-name selectors and Playwright file/title
selection without putting framework parsers in aplan.

**Provider concurrency is explicit, weighted and local to one aplan invocation.** The original
single-runner mapping and runner sequence remain serial. To overlap independent catalogue commands
and, under `--execute`, independent runners' skipped-list/batch stages, opt into the scheduled form:

```yaml
test:
  schedule: {capacity: 4}
  runners:
    - name: rust
      weight: 2                    # this runner consumes two capacity units
      exclusive: [compiler-cache] # capacity-one names, acquired together
      list: [cargo, test, --workspace, --, --list]
      run: [cargo, test, --workspace, --, --exact, '{test}']
    - name: web
      weight: 1
      exclusive: [browser-port]
      dir: web
      list: [pnpm, test, --list]
      run: [pnpm, test, '{test}']
```

`capacity` and every `weight` are positive integers; a runner defaults to weight one and cannot
weigh more than the schedule. `exclusive` is an optional nonempty set of exact, case-sensitive
names. A job starts only when its complete weight and every named resource are free, so it never
holds one resource while waiting for another. Weight is admission accounting for harnesses that
fan out internally, not a second OS limit: `max_memory` and `max_processes` still constrain each
child command independently. Named resources coordinate only provider work in this process; they
are not a machine-wide or distributed lock. Results, findings and cost rows retain configured
runner order even when commands finish in another order. ADR-0055 records the scheduling and
cancellation contract.

`id_strategy` is the one key whose cost is paid by everyone who reads the plan. `aplan add`
normally continues the sequence a plan already uses (`A1-046` -> `A1-047`), which is what
makes it fit a repository rather than impose a format on one, and it has dstask's documented
failure: two people working offline both derive `A1-047` and find out at the merge.
`APLAN002` catches the collision there, so a plan cannot end up holding two tasks under one
id. After preserving both sides of Git's add/add conflict, repair the losing allocation with
`aplan rekey OLD NEW`: it changes the frontmatter id, the conventional filename, every structured
task reference, and an id-based milestone policy as one guarded write. It never rewrites prose,
because the same spelling may correctly name the other branch's task; instead it reports each
possible occurrence by path and line for review. Git history, published links, external documents
and other repositories remain outside that boundary. A `done` or `dropped` task therefore needs
the explicit `--allow-terminal` acknowledgement.

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
citation this policy tolerates is one ordinary close records at L2, because close refuses exactly
what ordinary verify rejects and reads the same policy to decide. Focused `close --execute` raises
that same proposed citation to L3 and refuses unless it passed. And it cannot make a plan that
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

Runner `dir` is always relative to the selected repository root. This remains
true when `--config` selects a trusted policy file outside the checkout: the
policy chooses what may run, while the command inspects the repository under
review.

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

### Per-user settings, and the one thing they hold

`.aplan.yml` is the **repository's** policy: committed, shared, and binding on everybody who works
the checkout. There is a second, optional file for the other kind of setting, the kind that is about
*you* rather than about the plan:

```yaml
# ~/.config/aplan/config.yml  (or $XDG_CONFIG_HOME/aplan/config.yml)
me: ada          # unauthenticated label `aplan claim` records on this machine
```

That is the whole file today. **It deliberately cannot hold anything `.aplan.yml` holds**, and the
reason is worth stating because it is the design rather than an omission. `.aplan.yml` has keys like
`deny_warnings`, `advisory` and `evidence.require_min_strength`; a per-user file that accepted them
would let anybody turn a repository's gate down on their own machine and still pass it, silently,
exactly where nobody reviews. So the two files are two types with no key in common, and a test keeps
them that way. Precedence is not a rule anybody has to remember: there is nothing in this file for a
repository to lose an argument with.

Why identity is not in `.aplan.yml`: several people and agents work one checkout, so a name recorded
there would be the right one for at most one of them. And the file has to be *outside* the
repository for the same reason: both `XDG_CONFIG_HOME` and `HOME` are required to be absolute, so a
checkout cannot supply one by leaving `HOME` empty. Pin the behaviour with
`aplan facts --require identity.reads_a_per_user_file`.

The label is attribution, not authentication. Each claim records whether it came from `--as`,
`APLAN_OWNER`, this file, Git, or the operating-system fallback, and warns on common machine/test
placeholders. Use distinct labels for distinct actors sharing a host.

`--no-config` does not suppress this file, and that is deliberate: the flag exists so a tree under review cannot say what a run may do, and this file is yours rather than the tree's. It holds nothing a repository can require, so there is nothing in it for that flag to protect you from.

Where a name comes from, in order: `--as <name>`, then `APLAN_OWNER`, then this file, then
`git config user.name`, then the operating system's user. A refusal names all five. The environment
variable comes first because it is something you set for this run; this file comes before git's name
because replacing that guess is what it is for.

**There is no `.env` support, and that is a decision rather than a gap.** `APLAN_OWNER` already
covers "set it for this shell", so a dotenv would be a second way to say one thing -- and it would be
in the wrong place twice. Committed, it is shared between actors, which is what a repository key was
rejected for. Ignored, it is per-machine configuration living in a directory that is not per-machine.


## Install

Build from source. **Rust 1.88 or newer**, which is `rust-version` in
`Cargo.toml` and is checked against the locked dependency graph on every CI run
(`cargo xtask msrv`), so the number here is the one the reviewed revision
actually builds with rather than an aspiration:

```sh
cargo build --release        # standard binary with the default-on TUI
cargo build --release --no-default-features # explicit headless binary
```

The headless shape omits `aplan-tui`, Ratatui and Crossterm but keeps an actionable `aplan tui`
stub. Assert the package shape instead of guessing it with
`aplan facts --require build.tui`.

Or take the released binary, a statically linked Linux x86_64 build published with
its `sha256` (`v0.1.0` onward), so it runs whatever glibc the host ships. `cargo
install aplan` will work once the crate is released.

A build from `main` reports a pre-release version (`0.5.0-dev`), never the version
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
- uses: asamarts/aplan@v0.4.0    # pinning the action pins the binary
```

That installs the release matching the tag you pinned, verifies it against the
published checksum, and runs `aplan check` with `--format github`, so a violation
becomes an inline annotation on the pull request. Policy comes from your
`.aplan.yml`, not from flags threaded through the workflow; `command: verify`
resolves evidence against git instead (check out with `fetch-depth: 0` for that).

**Automating the gate turns the warning tier off.** Both commands exit `0` with warnings
outstanding, by design -- and **27 of this tool's 57 rules are warnings**, among them
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

`add`, `close` and `import` create and complete; the rest of a plan's life includes
edits for which hand-written YAML can break a cross-file invariant. Those edits
have guarded commands, including:

```sh
aplan mv RD1-04 --cut M-s0     # edits both milestones, or neither
aplan mv RD1-04 --phase 2      # changes the rollup coordinate, not the stable id
aplan drop RD3-01              # records the cancellation rather than deleting it
aplan drop M-s3 --cut          # the whole cut: its members, then the milestone
```

They resolve everything before writing anything, so a refusal leaves the plan
exactly as it was, and parse back what they wrote before saving it, on
`close`'s standard that a rewrite the tool cannot read again is a corrupted file.
`drop` refuses a task that is already `done` or `dropped`: those record a
decision, and rewriting one silently would launder it.

Fields such as an estimate or a `paths` glob remain YAML on purpose: changing
one breaks nothing that `check` will not catch on the next run, and a command for
it would be write surface bought with no invariant to maintain.

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

### Rollups beyond the built-in views

`summary` rolls up by phase by default. A plan with a declared tag family can
make that family the view without changing the roadmap's independent grouping:

```yaml
tag_families:
  kind: [kind-build, kind-measure, kind-review]
summary:
  group_by: kind
```

These are three independent concepts, not nested spellings for the same thing
(ADR-0046). A **phase** is the required ordered number used by the default
summary and as a hint when minting a new id; it gates nothing and changing it
does not rekey the task. A **cut** is the only membership level and owns work
through its milestone. A **tag family** is a repository-declared vocabulary for
a view such as kind, epic, team, or stage. A cut may span phases, and
cross-cutting work with no cut still has a phase.

A task carrying several members appears in every matching row, and the report
says those rows overlap; totals still count it once. A task carrying none lands
in `(no group)`. `APLAN026` validates member spelling but does not impose
cardinality; declare an `APLAN030` policy on the family if multiple membership
is invalid for the plan.

The human report names the family and quotes member labels. The quotes are
semantic rather than decoration: a real member called `(no group)` or `total`
stays distinct from the table's synthetic rows, and whitespace is escaped
rather than flattened into a different member. JSON keeps the raw strings.

Any other or ad hoc rollup is `aplan tasks`, which emits the plan as data with
the awkward parts already resolved (the cut a task belongs to, whether it is
ready), so a consumer is not re-implementing the graph:

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

The second is the rollup that motivated `summary.group_by`; the `jq` form remains
useful when a one-off view should not become repository policy. `status` is
carried rather than filtered, so cancelled work is the consumer's decision. The
document's shape is `schemas/v1/tasks.json`, derived from the Rust types and
gated like the others.

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
