# What every rule says, as the corpus exercises it

Generated from the scenario corpus in `crates/aplan-e2e/tests/coverage.rs`, committed,
and compared on every test run. Do not edit by hand: change the rule, then regenerate
with `APLAN_CATALOGUE=overwrite cargo test -p aplan-e2e --test integration coverage::the_rule_catalogue_is_committed_and_current`.

A rule's message and its `agent_instruction` are what an adopter and an agent act on.
They are pinned here so that changing one is a reviewable diff rather than something
discovered in a consumer's CI log (`A1-088`).

**What this does not pin.** A rule that interpolates its reason says several different
sentences, and only the ones some scenario reaches appear below. The title says *as the
corpus exercises it* because that is the honest scope. `A1-094` swept for the gaps and
lists the ones left unreached, each with the reason its fixture is not worth building
yet; adding a scenario for one is how a wording joins this file.

## APLAN001 (error)

frontmatter parses and satisfies the schema

- **says:** invalid YAML frontmatter: did not find expected ',' or ']' at line 2 column 6, while parsing a flow sequence at line 1 column 5
  - **fix:** Fix the YAML syntax in the frontmatter.
- **says:** invalid YAML frontmatter: mapping values are not allowed in this context at line 2 column 15: the value of `title` contains `: `, which starts a nested mapping unless the value is quoted
  - **fix:** Quote the value: `title: 'text with: a colon'`.

## APLAN002 (error)

task ids are unique

- **says:** duplicate task id, first defined in <tmp>/tasks/one.md
  - **fix:** Rename one of the two tasks to a unique id.

## APLAN003 (error)

blocked_by references a task that exists

- **says:** `blocked_by` references unknown task `GHOST`
  - **fix:** Either create task `GHOST` or remove it from `A-1`'s `blocked_by`.

## APLAN004 (error)

the dependency graph is acyclic

- **says:** dependency cycle: A-1 → A-2 → A-1
  - **fix:** Break the cycle by removing one `blocked_by` edge; a plan with a cycle has no valid execution order.

## APLAN005 (error)

done carries at least one piece of evidence

- **says:** `status: done` with no `evidence`: a status is a claim, not a proof
  - **fix:** Add at least one evidence entry (commit, url, file, test, or a dated note), or move the task back to `in_progress`.

## APLAN006 (error)

no task blocks itself

- **says:** task lists itself in `blocked_by`
  - **fix:** Remove `A-1` from its own `blocked_by`.

## APLAN007 (error)

a done task does not depend on an unfinished one

- **says:** is `done` but blocker `A-2` is `todo`
  - **fix:** Either close `A-2` with evidence, or `A-1` was not actually done.
- **says:** is `done` but its cut follows `M-s0`, which is `todo`
  - **fix:** Either close `M-s0` first, or drop the cut-sequencing edge if this task does not really wait for it.

## APLAN008 (error)

traces anchors resolve to a real heading or node

- **says:** `traces` anchor `#$.states[*]` is not an address into `spec.yaml`: `[*]` cannot be part of one, because a wildcard selects every member, and an anchor addresses one node
  - **fix:** Address one node of `spec.yaml`: a JSON Pointer (`#/states/provisioning`, RFC 6901) or a JSONPath (`#$.states.provisioning`, RFC 9535). Only the singular subset of JSONPath is accepted, names and whole-number indices, because a fingerprint is taken of one node, and this tool resolves an address rather than running a query.
- **says:** `traces` anchor `#/states/provisionng` addresses no node in `spec.yaml`
  - **fix:** Add that node to `spec.yaml`, or did you mean `#/states/provisioning`? An address is checked against the document as it is now, so a key that was renamed is reported here rather than silently tracing nothing.
- **says:** `traces` anchor `#renamed` matches no heading or table row in `DOC.md`
  - **fix:** Add to `DOC.md` a heading, or a table row, whose text slugifies to `renamed`; or correct the anchor to one the document already has. A **list item is not an anchor**, deliberately: a bullet's identity would be its whole text, so rewording it would break the reference rather than report drift. Give the item a heading or a table row, which is what an author tracking items one by one wants anyway.
- **says:** `traces` target `NOPE.md` does not exist
  - **fix:** Create `NOPE.md` or correct the trace path.
- **says:** `traces` target `spec.yaml` is not one YAML document this tool can address into: it does not parse, holds more than one document, nests deeper than 128 levels, or uses a shape with no JSON equivalent such as a non-string key
  - **fix:** Fix `spec.yaml` so it parses, or trace a document that does. Addressing half of a file this tool could not fully read would make the fingerprint mean something nobody chose.

## APLAN009 (warning)

active tasks carry an effort estimate

- **says:** no `effort_days` estimate
  - **fix:** Add an `effort_days` estimate, or drop the task.

## APLAN010 (error)

a note used as evidence carries a date

- **says:** evidence of kind `note` requires a `captured` date
  - **fix:** Add `captured: YYYY-MM-DD` to the note evidence entry.
- **says:** note evidence `captured: someday` is not an ISO-8601 date (YYYY-MM-DD)
  - **fix:** Write the note's `captured` date as YYYY-MM-DD (for example 2026-07-18).

## APLAN011 (error)

commit evidence resolves to an immutable commit reachable from HEAD

- **says:** commit `<sha>` does not resolve to a commit in this repository
  - **fix:** Cite a commit SHA that exists in this repository, or remove the evidence.

## APLAN012 (error)

commit evidence touches at least one of the task's paths

- **says:** commit `<sha>` touches none of this task's `paths` (src/**)
  - **fix:** Cite a commit that changes a file this task owns, or correct the task's `paths`.

## APLAN013 (error)

test evidence names a test the runner can find

- **says:** test `ghost` is not one the configured runner lists
  - **fix:** Run `aplan test list` with a search term, then cite an exact, non-ambiguous returned name or fix the evidence.
- **says:** test `guider::known` is not one the configured runner lists, but `known` is
  - **fix:** Cite `known`, which is how the runner reports it. Run `aplan test list` to inspect the exact configured names.

## APLAN014 (error)

test evidence names a test that passes

- **says:** test `known` failed under `--execute` and printed nothing
  - **fix:** Nothing was captured, so the runner is silent on failure or `test.output_lines: 0` is set. Run `known` through the configured command to see why, then make it pass or stop citing it as evidence.
- **says:** test `known` was not run under `--execute`: the runner lists it among the tests it skips, and no test this task cites was run
  - **fix:** Cite a test this runner actually runs as well, so the claim has proof the gate executes. The skipped citation may stay: it says where the rest of the proof lives.

## APLAN015 (error)

file evidence resolves at the cited rev

- **says:** file `../sibling/docs/log.md` is outside this repository, so no commit here can contain it
  - **fix:** A `file` citation resolves against this repository's history. For a deliverable that landed elsewhere, cite `url` evidence naming it at a commit on its forge, or track the work in that repository's own plan.
- **says:** file `docs/proof.md#` ends in an empty anchor
  - **fix:** Name the section after the `#`, or cite `docs/proof.md` on its own to mean the whole document.
- **says:** file `docs/proof.md` exists at HEAD, and `#outcome` matches no heading or table row in it
  - **fix:** Correct the anchor to a section `docs/proof.md` has, or cite `docs/proof.md` on its own to mean the whole document.
- **says:** file `docs/scan.bin` at HEAD is not text, so `#result` cannot be resolved
  - **fix:** Cite `docs/scan.bin` without an anchor, or point the citation at a text document whose sections can be named.
- **says:** file `src/nope.rs` does not exist at HEAD
  - **fix:** Point the `file` evidence at a path that exists at the cited rev (or at HEAD).

## APLAN016 (warning)

url evidence answers when it is checked

- **says:** url `http://169.254.169.254/latest/meta-data/` was not checked: only `https` citations are fetched
  - **fix:** Cite the `https` form. A citation names the target, and a task file can arrive from a fork, so this tool will not hand an arbitrary scheme or address to the configured checker.
- **says:** url `https://example.invalid/receipt` did not answer when it was checked
  - **fix:** Check the link. If it has moved, cite the new one; if it is gone, the proof is gone with it and the task needs different evidence.

## APLAN017 (error)

done evidence meets the configured minimum strength

- **says:** `done` but no evidence of a required kind (need one of: commit, test, file)
  - **fix:** Add evidence of a stronger kind (commit, test, file), or exempt the task with a configured tag if nothing stronger exists.

## APLAN018 (warning)

a closed task's traced section is unchanged since closure

- **says:** traced node `spec.yaml#/states/provisioning` has changed since this task was closed
  - **fix:** See what moved with `git diff <sha> -- <tmp>/spec.yaml`, then re-review the task; if it still holds, re-stamp with `aplan close --restamp`, otherwise `aplan supersede` it with the task that now needs doing.
- **says:** traced section `DOC.md#intro` has changed since this task was closed
  - **fix:** See what moved with `git diff <sha> -- <tmp>/DOC.md`, then re-review the task; if it still holds, re-stamp with `aplan close --restamp`, otherwise `aplan supersede` it with the task that now needs doing.

## APLAN019 (warning)

a task's paths globs each match at least one tracked file, once it cites evidence

- **says:** `paths` entry `src/gone/**` matches no tracked file, so the file this task cites binds to nothing through it
  - **fix:** Correct `src/gone/**` to name files this task owns, or remove it. Nothing the repository tracks matches it.
- **says:** `paths` entry `src` matches no tracked file, so the file this task cites binds to nothing through it
  - **fix:** `src` names a directory, and a glob without a wildcard names one file. Write `src/**` to own everything under it.

## APLAN020 (error)

a live task is not blocked by a dropped one

- **says:** is `todo` but blocker `A-2` is `dropped`: it can never legitimately start
  - **fix:** Either drop `A-1` too, revive `A-2`, or remove `A-2` from its `blocked_by` and name a real blocker.
- **says:** is `todo` but blocker `M-s0` is `dropped`: it can never legitimately start
  - **fix:** Either drop `M-s1` too, revive `M-s0`, or remove `M-s0` from its `blocked_by` and name a real blocker.
- **says:** is `todo` but its cut follows `M-s0`, which is `dropped`: it can never legitimately start
  - **fix:** Either drop `B-1` too, revive `M-s0`, or remove the cut-sequencing edge.

## APLAN021 (warning)

a milestone has members and carries no effort or paths

- **says:** is a checkpoint over cuts and holds `A-1` directly: a release closes cuts, so a task in it is a task in two cuts
  - **fix:** List the cuts this release closes in its `blocked_by`, and leave the work in them. A release is a checkpoint over cuts, not a second cut.
- **says:** is a checkpoint over cuts and holds `W-1` directly: a milestone declaring `contains` holds the checkpoints beneath it, so a task in it is a task in two cuts
  - **fix:** Move the work into one of the checkpoints this one contains, or stop declaring `contains` if this is a cut rather than a checkpoint over cuts. A checkpoint borrows the work beneath it; work it also holds itself belongs to two cuts at once.
- **says:** is tagged `milestone` but carries `effort_days`
  - **fix:** Remove the `milestone` tag if this is work, or make it a checkpoint: give it its cut's tasks as `blocked_by`, and drop `effort_days` and `paths`.
- **says:** is tagged `milestone` but declares `paths`
  - **fix:** Remove the `milestone` tag if this is work, or make it a checkpoint: give it its cut's tasks as `blocked_by`, and drop `effort_days` and `paths`.
- **says:** is tagged `milestone` but has no live work in it: every task it lists is `dropped`
  - **fix:** Move the work that belongs to this cut into its `blocked_by` (`aplan mv <id>... --cut <this>`), or cancel the cut too (`aplan drop --cut <this>`) if it is not going to happen.
- **says:** is tagged `milestone` but has no members (`blocked_by` is empty)
  - **fix:** Move the work that belongs to this cut into its `blocked_by` (`aplan mv <id>... --cut <this>`), or cancel the cut too (`aplan drop --cut <this>`) if it is not going to happen.
- **says:** is tagged `milestone` but has no work in it: its `blocked_by` names only other cuts
  - **fix:** Move the work that belongs to this cut into its `blocked_by` (`aplan mv <id>... --cut <this>`), or cancel the cut too (`aplan drop --cut <this>`) if it is not going to happen.

## APLAN022 (error)

every task is a member of exactly one cut, or is tagged cross-cutting

- **says:** belongs to no cut
  - **fix:** Add the task to exactly one milestone's `blocked_by` (its cut), or tag it `cross-cutting` if it deliberately belongs to none.

## APLAN023 (warning)

a done task's path-checkable evidence can actually be path-checked

- **says:** `done` with commit evidence but no `paths`: even if the citation resolves, nothing binds it to this task; resolution alone is not evidence that the task's files changed
  - **fix:** Add `paths` naming the files this task is responsible for, so `verify` can check the cited commit actually touches them (APLAN012).

## APLAN024 (warning)

a done task's commit evidence is about that task, not mostly about other work

- **says:** the only commit proving this task, `<sha>`, touches 60 path(s) outside its `paths`: it overlaps them, so `APLAN012` passes, but a commit that size would overlap almost any `paths`
  - **fix:** Cite the commit that did this task's work, or add `test` evidence; if the work really did arrive in one large commit, say so in the task body.

## APLAN025 (warning)

a cited test belongs to the part of the repository this task's `paths` cover

- **says:** test `known` is listed by a runner that is called `component` and covers `component/**` for tests matching `known*`, which this task's `paths` (elsewhere/**) do not reach
  - **fix:** Cite a test that covers the code this task owns, or correct `paths` if they do not name all of it. This cannot tell which of the two is wrong.
- **says:** test `known` is listed by a runner that is called `component` and covers `component/**`, which this task's `paths` (elsewhere/**) do not reach
  - **fix:** Cite a test that covers the code this task owns, or correct `paths` if they do not name all of it. This cannot tell which of the two is wrong.
- **says:** test `known` is listed by a runner that is called `component` and runs in `component/`, which this task's `paths` (elsewhere/**) do not reach
  - **fix:** Cite a test that covers the code this task owns, or correct `paths` if they do not name all of it. This cannot tell which of the two is wrong.

## APLAN026 (warning)

a tag that looks like a member of a declared family is one

- **says:** tag `epci-b` is one edit from `epic-b` in the `epic` family: did you mean `epic-b`?
  - **fix:** Correct the tag, or add it to `tag_families.epic` in `.aplan.yml` if it is a real member.

## APLAN027 (warning)

a cancelled cut's members are cancelled too

- **says:** is `dropped` but 1 member(s) are still live (A-1): the plan says this cut is cancelled and that its work is going ahead
  - **fix:** Cancel the work too (`aplan drop <id>...`), or move it to a cut that is still live, or revive the cut.

## APLAN028 (error)

a done task stamps every trace that can be stamped

- **says:** is `done` and does not stamp `DOC.md#11`, `DOC.md#7`, and stamps `DOC.md#intro`, which it no longer traces: drift is not checked for them, so what they trace can change and nothing will say so
  - **fix:** Re-review `DOC.md#11`, `DOC.md#7`, then `aplan close A-1 --restamp`, which stamps every trace and drops keys no trace names.
- **says:** is `done` and does not stamp `DOC.md#7`, and stamps `DOC.md#intro`, which it no longer traces: drift is not checked for it, so what it traces can change and nothing will say so
  - **fix:** Re-review `DOC.md#7`, then `aplan close A-1 --restamp`, which stamps every trace and drops keys no trace names.
- **says:** is `done` and does not stamp `DOC.md#intro`: drift is not checked for it, so what it traces can change and nothing will say so
  - **fix:** Re-review `DOC.md#intro`, then `aplan close A-1 --restamp`, which stamps every trace and drops keys no trace names.

## APLAN029 (warning)

a declared tag family is carried by at least one task

- **says:** no task carries the `epic` family: it declares 2 member(s), `a`, `b`, and a task joins by carrying one of them as a whole tag
  - **fix:** Tag a task with one of them, or correct `tag_families.epic` in `.aplan.yml` if the members are not the tags your tasks actually carry.

## APLAN030 (error)

a task carries the number of members a declared family requires

- **says:** `epic` is declared exactly-one and this task carries `epic-a`, `epic-b`
  - **fix:** Carry exactly one of `tag_families.epic` on this task, or change what `.aplan.yml` requires of it.

## APLAN031 (error)

file evidence names a file inside the task's paths

- **says:** file `README.md` is outside this task's `paths` (src/**): it exists, which is all the citation proves
  - **fix:** Cite a file this task owns, or correct the task's `paths` if they do not name all of it. This cannot tell which of the two is wrong.
- **says:** file `src/a.rs` is outside this task's `paths` (src): it exists, which is all the citation proves
  - **fix:** Cite a file this task owns, or correct the task's `paths` if they do not name all of it. This cannot tell which of the two is wrong.
- **says:** file `src/a.rs` is outside this task's `paths` (src/gone/**): it exists, which is all the citation proves
  - **fix:** Cite a file this task owns, or correct the task's `paths` if they do not name all of it. This cannot tell which of the two is wrong.
- **says:** this task's `paths` include `**`, which matches every file: a cited file cannot be bound to the specific work this task owns
  - **fix:** Replace the catch-all glob with the specific files this task owns.

## APLAN032 (error)

a task's paths entries are globs this tool can compile

- **says:** `paths` entry `src/[` is not a glob this tool can compile: error parsing glob 'src/[': unclosed character class; missing ']', so this task's evidence cannot be bound to its `paths` at all
  - **fix:** Correct the `paths` entry so it is a valid glob. `*` matches within one path segment, `**` crosses directories, and `[` opens a character class that has to be closed.

## APLAN033 (error)

a prefer_after entry names a task that can actually be preferred

- **says:** `prefer_after` names unknown task `GHOST`
  - **fix:** Either create task `GHOST` or remove it from `A-1`'s `prefer_after`.

## APLAN034 (error)

a traces target outside this repository is declared as external

- **says:** `traces` target `../sibling/DOC.md` is outside this repository, and no `traces_external` declares it: a plan that reads another repository's files passes only where that repository happens to be checked out
  - **fix:** Trace a document in this repository, or declare the sibling root in `traces_external` so its absence is a configuration error rather than a surprise in one CI job.

## APLAN035 (warning)

commit evidence names a commit that did work, not a merge

- **says:** commit `<sha>` has 2 parents, so its diff covers everything both sides touched: a `paths` check against a merge passes for work this task did not do
  - **fix:** Cite the commit that made the change (`git log --no-merges -- <paths>` finds it). A merge is almost never that commit; if this repository merges every feature with `--no-ff` and the merge really is where the work landed, say so in the task body.

## APLAN036 (warning)

a configured test runner lists at least one test, or says it expects none

- **says:** the `blind` test runner lists no tests, so every citation against it will fail as a name nothing lists
  - **fix:** Check the runner in `.aplan.yml`: its `dir`, and that its `list` command prints test names when run there by hand. If the component genuinely has no tests yet, say so with `expect_tests: false` rather than deleting the runner.

## APLAN037 (warning)

a test that passed under `--execute` is distinguishable from one the runner never ran

- **says:** the `runner 1` test runner ran test `known` and produced exactly what it produces for a name that does not exist, so `--execute` did not prove the test ran
  - **fix:** The names this runner's `test.list` prints are not the names its `test.run` selects. Run one of them by hand and check that the test really runs; then change `list` to print what `run` selects, or `run` to select what `list` prints. If the runner is right and simply says nothing either way, declare `run_selects: true` on it.
- **says:** the `vitest-shaped` test runner ran test `tests/math.test.js > adds` and produced exactly what it produces for a name that does not exist, so `--execute` did not prove the test ran
  - **fix:** The names this runner's `test.list` prints are not the names its `test.run` selects. Run one of them by hand and check that the test really runs; then change `list` to print what `run` selects, or `run` to select what `list` prints. If the runner is right and simply says nothing either way, declare `run_selects: true` on it.

## APLAN038 (warning)

a `done` task has answered the acceptance criteria its own body states

- **says:** is `done` with 1 of 2 criteria in its body ticked
  - **fix:** Re-read the criteria and tick the ones that are met. If one is not met, the task is not done: say what is left, supersede it, or hand that criterion to the task that will do it and tick it as a decision, naming where it went. Nothing ticks them for you.

## APLAN039 (warning)

a `done` task has not ticked a prompt this tool wrote as though it were a criterion

- **says:** is `done` and ticks a prompt this tool wrote: `state what done means for this task, in this repository's terms` asks for a criterion, and is not one
  - **fix:** Replace that line with what done means for this task, in this repository's terms, and tick it only when it is met. If the task is finished and nobody wrote the criterion, say what was delivered instead. Do not delete the line to silence this.

## APLAN040 (warning)

a task declares each `paths` entry once

- **says:** `paths` lists `src/**` 2 times. The copies match the same files, so nothing is weakened -- but a repeated entry is usually an edit that went wrong
  - **fix:** Delete the repeated entry. If the two were meant to name different trees, one of them is not the path you meant.

## APLAN041 (warning)

the generated `AGENTS.md` block says what this repository's policy says

- **says:** 1 `aplan:begin` and 0 `aplan:end` marker(s), so the generated block cannot be read and nothing can say whether it is current
  - **fix:** Leave one `aplan:begin` and one `aplan:end`, in that order, then run `aplan export-agents-md --write <tmp>/AGENTS.md`.
- **says:** the generated block is not what this repository's policy produces now: it is what an agent reads before touching the plan, so a stale one teaches a rule the tool would refuse
  - **fix:** Run `aplan export-agents-md --write <tmp>/AGENTS.md`.

## APLAN042 (warning)

a `revisit_at` names a real instant, on a task that is parked

- **says:** `revisit_at: next tuesday` is not an RFC 3339 instant, so nothing can act on it
  - **fix:** Write the instant as `YYYY-MM-DDTHH:MM:SSZ`, or with an offset (`YYYY-MM-DDTHH:MM:SS+02:00`). A date alone is not an instant: a scheduler has to know which moment of the day to return at.
- **says:** carries `revisit_at` and is `todo`: a revisit says when to look at *parked* work again, and nothing reads it here
  - **fix:** Park the task (`status: blocked`) if it is waiting on something outside the plan, or drop the `revisit_at`. Reaching the instant never changes a status, so it cannot be what makes this task start.

## APLAN043 (warning)

a live task carries the scaffold's own prompt beside the criteria its author wrote

- **says:** carries the scaffold's own prompt beside 1 criterion of its own: `state what done means for this task, in this repository's terms` is the line `aplan add` writes to be replaced, not kept
  - **fix:** Delete that line, and the `## Acceptance criteria` heading it was written under if that heading now has nothing else beneath it. Keep the criteria you wrote. A body with two of that heading is the scaffold left beside the answer.

## APLAN044 (warning)

a closed task claims less of the released version than its level implies, without saying why

- **says:** claims `none` of the released version where its level implies `fix`, and says nowhere why
  - **fix:** Add `version_impact_reason` saying what this task changed that the level's floor gets wrong -- a plan edit or a pilot log really does change nothing that ships. If it did change the artifact after all, raise `version_impact` instead.

## APLAN045 (warning)

a live task's traced section is unchanged since the task was last touched

- **says:** is still open and `DOC.md#intro` has changed since this task was last touched: the ground it was written on has moved
  - **fix:** Re-read `DOC.md#intro` against this task. If the work is already done, close it with the evidence; if the task still holds, editing it records that somebody looked and clears this.

## APLAN046 (error)

a done task's ratifiers are done

- **says:** is `done` but its ratifier `A-2` is `todo`
  - **fix:** Close `A-2` first, or drop the ratification if this task does not actually need its blessing. The work itself was never blocked by it: `ratified_by` says who signs off, not what this waits on.

## APLAN047 (error)

a `contains` names a milestone that exists, and closes no loop

- **says:** `contains: M-1` is not also in `blocked_by`, so this checkpoint does not wait for what it holds and may be closed while it is unfinished
  - **fix:** Name a milestone that exists and list it in `blocked_by` too, or move the id to `blocked_by` alone if it is work this cut holds rather than a checkpoint beneath it.
- **says:** containment cycle through `contains`: M-1 → M-2 → M-1
  - **fix:** Decide which of these contains the other and remove the reverse edge. A rollup over a loop has no answer: whichever checkpoint is walked first swallows the other, and the number changes with iteration order.

## APLAN048 (error)

the released version has moved as far as the closed tasks require

- **says:** this tree declares version `0.2.0`, which is older than `v0.3.0` -- the last release tag reachable from HEAD. Either the version was reverted, or `version.release_tag` is matching tags that do not name this artifact's releases, in which case every version this derives is measured from the wrong place
  - **fix:** Set this repository's version to something at or above `0.3.0`, or narrow `version.release_tag` in `.aplan.yml` so it matches only the tags that name this artifact's releases.
- **says:** this tree declares version `0.3.1` and the plan requires `0.4.0`, measured over 1 task(s) closed since `v0.3.0`: `A-1` is `breaking`
  - **fix:** Set this repository's version to `0.4.0` before tagging, or change the impact the window declares if `0.3.1` is the honest number.

## APLAN049 (error)

the artifact agrees with what the plan claims about compatibility

- **says:** the configured checker found this artifact is not compatible with its last release, and the plan claims `fix`, measured over 1 task(s) closed since `v0.3.0`
  - **fix:** Declare `version_impact: breaking` on the task that made the incompatible change, or correct the artifact if the break was not intended. The checker compared the built artifact; the plan is what disagrees with it.

## APLAN050 (error)

a recorded `version` is on a release, and is a version this tool can read

- **says:** records `version: 1.2.3`, which says what a *release* produced. This node is not one, and a cut is a planning bucket rather than a delivery
  - **fix:** Remove `version`, or tag this node `release` if it really is the thing that ships.

## APLAN051 (warning)

a tag is not one edit from a word that changes what this tool does

- **says:** tag `milestne` is one edit from `milestone`, which changes what this tool does: did you mean `milestone`?
  - **fix:** Correct the tag to `milestone` if that is what was meant. If `milestne` is the repository's own word, declare it in `tag_families` so it is a vocabulary rather than a near miss.

## APLAN052 (warning)

`cross-cutting` is not on a task that is a member of a cut

- **says:** tagged `cross-cutting` and a member of a cut: the tag is how `APLAN022` is told a task belongs to no cut, which this task's own membership contradicts
  - **fix:** Remove `cross-cutting` from this task's `tags`: it is in a cut, so the requirement it waives is one this task already meets. If the word was meant to say the work spans components, declare a family for that in `tag_families` and use a word of your own.

## APLAN053 (warning)

a task that is `in_progress` has something in the repository to show for it

- **says:** this task is `in_progress`, and nothing the repository tracks matches its `paths`, so there is nothing to show the work has started
  - **fix:** Either the work has not started, in which case `aplan stop <id>` says so, or it has and its files are not added yet, in which case `git add` them. If the task owns files that do not exist yet, its `paths` are a plan rather than a claim and the status is the thing to change.
- **says:** this task is `in_progress`, and nothing the repository tracks matches its `paths`, so there is nothing to show the work has started
  - **fix:** `src` names a directory, and a glob without a wildcard names one file. Write `src/**` to own everything under it. The work may well have started; nothing can tell while the declaration matches nothing.

## APLAN054 (warning)

a closed task's mutable file evidence is covered by a current fingerprint

- **says:** file evidence `docs/proof.md` still resolves, but what it cites has changed since this task recorded it
  - **fix:** Review the current `docs/proof.md`. If it still proves this task, run `aplan close A --restamp`; otherwise replace the citation with `aplan amend A --replace-evidence file:docs/proof.md <kind>:<ref>`.

## APLAN055 (error)

captured dates are used only for authored note evidence

- **says:** `file` evidence carries `captured: 2026-07-14`, but that date has no provenance semantics for this kind: the evidence fingerprint binds review to committed content; a separate author-entered date is not bound to that fingerprint and cannot date the approval it claims
  - **fix:** Remove `captured` from this `file` evidence entry. `aplan` accepts the field only on authored `note` evidence; use this kind's resolver or freshness mechanism instead.

## APLAN056 (warning)

a task's owner belongs to the declared owner vocabulary

- **says:** owner `kaminsdo` is not declared in `owners`: did you mean `kaminsod`?
  - **fix:** Change this task's `owner` to `kaminsod` if that is who it means, or add `kaminsdo` to `owners` in `.aplan.yml` if it is a distinct identity.

## APLAN057 (error)

every inbox entry is one canonical, valid and cross-record-consistent V1 capture

- **says:** invalid capture YAML: did not find expected ',' or ']' at line 3 column 1, while parsing a flow sequence at line 2 column 5
  - **fix:** Repair the YAML so it is one `aplan/capture/v1` mapping.

