# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Nothing yet.

## [0.3.0] - 2026-09-16

> Staging a build from `main` rather than pinning a release? See
> [`docs/upgrading.md`](docs/upgrading.md): upgrade the binary before adding new config keys, and
> check what `APLAN024` reports on your plan before wiring a blocking gate to it.

### Added

- **A task can say what it did to the released version** (`A1-312`, ADR-0034), through a new
  `version_impact` field and a new `version` block in `.aplan.yml`, and `APLAN044` is the one rule
  that reads them. Off unless you ask: a plan that never sets `version.require_impact` is
  unaffected, and nothing about it appears.

  The words describe the **change**, not the arithmetic -- `none`, `fix`, `feature`, `breaking` --
  because `breaking` stays true at every version and `major` does not: cargo formalises `0.x.y` as
  `0.INCOMPATIBLE.COMPATIBLE`, so below 1.0 a breaking change bumps the *minor*. An author writing
  the word for the bump would have to know that rule and apply it; writing the word for the change,
  they do not, and the scheme does the sum. The semver words are deliberately **not** accepted as
  aliases, because teaching that `major` means `breaking` teaches something false below 1.0.

  **A task's level supplies a floor**, so the ordinary task writes nothing at all: a unit of work
  owes a `fix`, a cut a `feature`, a release nothing of its own. The floors are configuration with
  a default, because the right map differs between projects -- where cuts are planning buckets
  rather than deliveries, the cut floor is set to `none`. `APLAN044` reports only the case a floor
  cannot cover: a closed task claiming *less* than its level implies and not saying why, which is
  legitimate to do and worth a reviewer being told about.

- **The test suite builds its fixtures on disk rather than in RAM** (`A1-257`). Nothing an adopter
  runs changes; this is about developing on this repository. `tempfile::tempdir()` writes under
  `/tmp`, which on any modern Linux is a `tmpfs`, and a `TempDir` only removes itself when its run
  finishes -- so every killed run leaks into memory. Measured on one machine: 3,966 stale
  directories, the oldest a month old, and `/tmp` mounted `usrquota`, so the next fixture write
  failed as `EDQUOT` inside a test whose subject was glob measurement.

  Fixtures now go under `<target>/tmp`, which `cargo clean` already empties, through one helper
  that works in a unit test as well as an integration test -- or, when `target/` sits inside the
  checkout, under a cache directory instead, because `.aplan.yml` discovery climbs to the
  filesystem root and a fixture below the checkout would inherit its policy. A gate refuses a new call that reaches
  for the RAM disk. **Two guards came with it**, because putting fixtures under `target/` means
  putting them *inside* the checkout: a fixture whose `git init` failed would otherwise have its
  `git add` and `git commit` find the checkout above -- reproduced, HEAD moved and the reflog
  recorded a commit nobody wrote. `GIT_CEILING_DIRECTORIES` now stops the walk at the fixture root,
  and fixture setup checks each command's exit status instead of only that `git` started.

- **Each distinct cited test runs once per `verify`** (`A1-243`). One runner and one exact listed
  name select one execution, so a second task citing the same test reuses what the first learned
  rather than running it again. The saving grows with citation reuse, which is what the evidence
  model is for: measured on one adopter's plan, 86 `test` citations over nine distinct names took
  about 55 minutes against about three for the nine suites run once. On this repository's own plan
  -- 567 citations over 540 distinct names -- it saves 27 executions, because a plan that mostly
  cites unique tests was never paying the cost.

  A failure is still every citing task's failure, reported against each of them, as is a command
  stopped at a declared cap; `APLAN037` still counts **citations** rather than executions, so the number an adopter uses to judge how much of
  their plan is unproven is unchanged. A command that could not be launched is not remembered --
  a launch that fails beside one that succeeded is a transient rather than a fact about the
  configuration. Nothing is shared between invocations. **If a cited test is flaky, one result now
  stands for every citation of it in that run**, where before each citation rolled the dice.

- **A runner may be told what it can spend** (`A1-310`, ADR-0033). `test.max_memory` and
  `test.max_processes` put every command a runner runs -- `list`, `run`, `skipped` -- inside a
  cgroup, and `test.wrap` prepends a command of your own for anything the two dials do not model.

  **A command stopped at its cap is reported as stopped, never as a failing test.** That
  distinction is the point: a test the kernel killed neither passed nor failed, and `APLAN014` now
  says which happened. A `max_processes` breach is the one nobody could infer -- `pids.max` does
  not kill anything, it makes `fork()` fail, and the command then exits however it likes, often
  zero. It is read from the cgroup's own counters rather than guessed from an exit status.

  **`max_memory` sets `memory.max` and `memory.swap.max` together.** Prototyped: with only the
  first at 64MB a process reached 3GB and the kernel counted no kill at all, because the pages went
  to swap. A cap a box with swap turns into a paging target is not a cap.

  **Where it cannot be enforced the run says so once and proceeds** -- a platform with no cgroup v2,
  or a container that was not given a cgroup subtree of its own, which is what most CI runs in. The
  note names which case it is, because the remedies differ, and it is a note rather than a warning
  for a reason ADR-0033 records: no corpus scenario could ever produce the condition, so a rule code
  would be one this project's own gates could never check. **The cost, stated: on such a platform
  the run really is unprotected and that line is all that says so.**

  Sizing one is your own run's business, not a number from here: measured on one machine,
  `cargo test` warm peaks at 156 MB, `go test ./...` at 1.2 GB, and `npm test` (vitest) at 33 GB.
  `aplan verify` reports what a runner reached (`A1-309`), which is where the number comes from.

  **`url.check` takes the same three keys**, because the cap covers every command this tool
  spawns rather than the test runner's alone. A checker stopped at its cap is reported against
  `.aplan.yml` as `APLAN016`, worded as *stopped* and not as *could not be run*: it ran, and no
  url is called dead on the strength of a check that never answered. Its cost is reported beside
  the runners' now, which it was measured for and never shown.

- **A run says what its commands cost** (`A1-309`). `aplan verify` now names, per test runner, the
  most processes it saw alive at once, the largest sum of their resident memory, and the wall time
  -- but only when a runner reached something worth knowing, so an ordinary run prints nothing new.

  The number is the harness's, not this tool's. `aplan` spawns **one command at a time**: measured
  on one machine, `cargo test -q -p <crate>` warm peaks at 4 processes and 156 MB, `go test ./...`
  at 34 and 1.2 GB, and `npm test` (vitest) at **160 processes and 33 GB**. A gate that quietly
  costs that much is worth saying out loud, and it is what makes a limit sizeable rather than
  guessed.

  **The line names which figure it is.** Summing each process's resident set overcounts memory they
  share -- the cargo case is 30.5 GB summed against 5.1 GB proportional, a factor of six -- so the
  cheap figure is taken and said, because an adopter sizing a limit from the wrong one would size it
  six times too large. Measured by reading `/proc` for the command's process group rather than a
  cgroup: a cgroup carries the peak for nothing and is unavailable in exactly the place this has to
  work, inside an undelegated container, which is what most CI runs in. **Where it cannot be
  measured at all the absence is reported rather than a zero**, which would be a lie.


- **New command `aplan export`: the plan written back out in somebody else's shape** (`A1-189`).
  `import --format` read five shapes and nothing wrote any of them, so the day the plan became the
  source of truth the document you imported from started going stale. This build writes
  `--format checklist`: a cut becomes a `##` heading, a `done` task becomes `- [x]`, and everything
  else becomes `- [ ]`. `--out FILE` writes it, `--check FILE` gates it the way
  `export-agents-md --check` gates the agent contract, and with neither the document goes to
  standard output.

  **Lossy outwards too, and it says so.** What the target shape cannot carry -- ids, evidence,
  paths, owners, estimates, edges -- is named with a count on standard error, so a redirected
  document is only the document. A shape that cannot be written honestly is refused by name with
  the one field a generated document would have had to invent: `openspec`, for instance, could not
  carry `id`.

  `aplan export-roadmap-json` and `aplan export-agents-md` are unchanged and stay separate. Both
  *merge* into a file somebody else maintains, so for them the target file is an input rather than
  a destination; ADR-0032 records the split, and `export --format roadmap-json` refuses by pointing
  at the generator.

- **New config key `policy`: evidence policy scoped to part of a plan** (`A1-183`). Every dial in
  `.aplan.yml` was whole-plan, and the one exception -- `evidence.exempt_tags` -- is the degenerate
  case of scoping: "required strength is *none* for these tags". `policy` is the general form. Each
  entry names a subset with the same four dimensions the command line takes (`cut`, `tag`, `owner`,
  `status`, with the same reading: within a dimension any, across dimensions all) and sets
  `evidence.require_min_strength` or `evidence.exempt_tags` for it.

  **The narrowest matching scope wins, and among equals the first entry in the file.** The
  whole-plan settings are the scope that names nothing, so any match beats them. An empty
  `require_min_strength: []` in a scope means "nothing is required here", which is how
  `exempt_tags` is expressible in the new form. A scope that names no subset, or sets no policy for
  the one it names, is refused when the file loads rather than read as a no-op.

  `aplan export-agents-md` states each scope beside the whole-plan rule, because an agent told one
  sentence where two apply is told the wrong rule for the subset it is working in.

  **Upgrade the binary before adding this key.** An unknown key is refused, so a plan carrying
  `policy` will not load on a binary that predates it.

### Deprecated

- **`aplan tree` will default to live-only ancestry** (`A1-235`). Announced here and in the
  command's own help; nothing changes in this release. In a later cut `tree` will show only the
  ancestry that can still block -- what `--live` prints today -- and `--all` will restore the full
  view. A `done` task cannot block anything, so the present default spends most of its output on
  constraints already met. Measured on this project's own plan on 2026-09-09,
  `aplan tree A1-183 --depth 2` prints 150 lines and the same query under `--live` prints 11. The
  figure moves with the task and with the plan, which is why it lives here rather than in the
  command's help: an adopter reads that beside their own repository, where our numbers describe
  nothing. The count of what was withheld is printed either way. **If you script against `tree`'s output**, pass `--all` once that flag exists, or pin a
  release; `aplan deprecations` reports the runway and the cut each half lands in.

### Fixed

- **A test the runner skips read as proof under `--execute`** (`A1-300`). `--execute` reads a zero
  exit status as "the cited test passed", and a runner asked for a test it *skips* exits zero too:
  `cargo test -- --exact <an ignored test>` prints `running 1 test`, `1 ignored`, and exits `0`, so
  a task whose only proof is an `#[ignore]`d test closed with nothing having run. `APLAN037` is
  right to be silent about it -- that rule asks whether a passing run is *indistinguishable* from a
  miss, and this one is plainly distinguishable; the runner said what it did.

  The new per-runner `test.skipped` is argv that prints the tests a runner will not run, one per
  line, in the names `test.list` prints -- for cargo, `[cargo, test, --, --list, --ignored]`. It is
  asked once per runner, the same way `list` is, rather than this tool learning what "ignored"
  looks like in each harness. `APLAN014` then reports a task whose *every* test citation is one of
  them. One skipped citation beside a test that runs is silent, and stays worth writing: it says
  where the rest of the proof lives.

  **Declaring it is opting in**, the same one-sided shape as `run_selects`: a repository that
  knowingly cites a nightly-gated test leaves the key out and nothing changes. **Upgrade the binary
  before adding it**, as for any config key: an unknown key is refused, so a plan carrying
  `test.skipped` will not load on a binary that predates it.

- **New rule `APLAN043`: a live task carries the scaffold's own prompt beside the criteria its
  author wrote** (`A1-299`). `aplan add` writes a `## Acceptance criteria` heading and one prompt
  under it, there is no command that edits a body, and the obvious hand-edit -- appending your own
  `## Why` and `## Acceptance criteria` -- leaves the file with two of each and the prompt still in
  it. The sixth pilot did that on its first day and a human re-reading the file found it, not the
  tool.

  Matched exactly and outside fenced code, on the same ground `APLAN039` stands on: the sentence is
  one this tool wrote, so recognising it is not a guess about prose, and a task *quoting* the prompt
  is silent. **Live tasks only** -- on a closed one the same leftover is already reported, unticked
  by `APLAN038` and ticked by `APLAN039` -- and only *beside* criteria of the task's own, because a
  body whose one criterion is the prompt has simply not answered yet. Ticked or unticked, since a
  tick does not make the scaffold survive less; it makes the line claim the instruction was met.

  Measured before it was built: across 2,768 task files in sixteen plans, two carry the shape, one
  of them in this repository's own plan. A warning, so an adopter meets it on tasks they have
  already written without a red build.

- **The generated block's opening marker is matched by its start** (`A1-294`,
  `--require contract.begin_marker_matched_by_prefix`). Placing the block by hand means writing the
  markers by hand, and the opening one is a hundred characters. A file carrying the obvious
  `<!-- aplan:begin -->` was reported as having *zero* opening markers, with no way to see the
  difference short of reading the source. Any line beginning `<!-- aplan:begin` and closing `-->`
  now opens the block, and `--write` replaces that line with the full marker, so the file still
  tells its next reader that edits between the markers are lost. A line that merely mentions the
  marker is not one: the match is anchored to the start of a line, and the name ends where the
  marker's does, so `<!-- aplan:beginning of the notes -->` is your comment rather than the marker.
  `<!-- aplan:end -->` is unchanged and needed no widening.

- **`require_min_strength` had no paragraph, so its two ends read as a disagreement** (`A1-283`).
  The README's configuration section explained `id_strategy`, `deny_warnings` and `advisory` and
  said nothing about the one key with a stage to choose. It now states the arc: start at
  `[commit, test, file]` with an exempt tag you review, because a plan authored before its
  implementation has commits at best and demanding the strongest rung leaves every task uncloseable
  until code arrives; narrow to `[test]` once a runner exists to point at. `aplan`'s own `[test]`
  is named as the far end of that arc rather than as the starting line, and a test reads both
  values out of this repository's `.aplan.yml` so the paragraph cannot describe a policy it no
  longer runs. The same sentence is in `docs/upgrading.md`'s row and in the file `aplan init`
  writes, which is where an adopter meets the key before reading either document.

  The gate `ci/scripts/spec.sh` runs also grew: it read `aplan check-docs` over three documents,
  and `README.md`, `HLD.md`, `AGENTS.md` and `CONTRIBUTING.md` carry 105 distinct task references
  between them that nothing checked. An id that stops resolving reads as *handled*, which is what
  that command exists to catch.

- **`aplan add --from` resolved a reference only backwards, and the two spellings of membership
  pulled in opposite directions** (`A1-293`). `blocked_by` on the milestone wanted its members
  written first; `cut:` on each task wanted the milestone written first. A file cannot do both, and
  the natural order -- the cut, then the work under it, the way a roadmap reads -- was refused for
  one of the two whichever you picked. An entry may now wait on, or join, any id the file declares,
  above it or below it. Nothing is weakened: the file is still applied as one change and the rules
  still run once on the plan the whole of it would produce, so an id neither the plan has nor the
  file declares is still refused, now in a message that says where it was looked for.

  The `cut:` refusal was the misleading one -- `no cut `M-a` in this plan. This plan declares no
  cuts`, about an id declared four lines further down. It now says which of three things went
  wrong: the file declares that id and it carries no `milestone` tag, or the file declares cuts
  under other ids and this is a label rather than one of them, or nothing anywhere declares it.

  **The edge was also being dropped.** Membership was written inside the entry loop, where a cut
  declared later had no milestone to write into, so accepting a forward `cut:` without moving that
  pass would have reported success and written a cut with no members. The edges are applied after
  the loop now.

  The plan is asked first, and a name it *refuses* rather than fails to find keeps being refused:
  where two of a plan's cuts answer to one label, `aplan` says so instead of choosing, and reading
  past that to a milestone the file declares under that name would have answered an ambiguity with
  a third cut.

- **A new plan could not be started under a strict policy, and both refusals pointed at each
  other** (`A1-292`). Where `deny_warnings: true` makes `APLAN021` block, a first cut cannot be
  added because it has no members and a first task cannot be added because it belongs to no cut --
  and `APLAN022`'s advice named `aplan add "the first cut" --tag milestone`, the command the other
  refusal declines. The advice now depends on whether an empty cut actually blocks: the two-step
  where it is allowed, and one `aplan add --from` file holding the cut and its first work where it
  is not. `README.md` gained a **Starting a plan** section whose recipe is executed by a test, so
  the documented path cannot rot.

  The same trap is not confined to an empty plan: under that policy `aplan add --tag milestone`
  cannot create a cut on *any* plan. A cut is created with its work -- `--blocked-by <ID>` where
  the work already exists and is in no cut, a `--from` file where it does not. A cut a change
  *empties* is a different thing: `aplan mv` moving the last member out raises the same
  `APLAN021`, and the rule's own remedy is the complete answer there, so nothing is added to it.

- **`aplan init` wrote a file missing eight of the nineteen top-level config keys** (`A1-292`).
  `policy`, `id_strategy`, `docs`, `effort_days_per_week`, `stale_after_days`, `traces_external`,
  `url` and `require_cut_membership` were absent, while the file's own header claims "the values
  shown are the defaults" and its `advisory` comment told the reader to "turn on
  require_cut_membership above". All are in it now, and two tests read the file: one walks the
  schema against it so the next key cannot be added without being shown, and one parses every
  example in it, because a file that is all comments parses as an empty config however wrong its
  examples are. The header no longer claims every value shown is a
  default: it says a default where there is one and an example where there is not, which is what
  the file actually holds.

- **Four messages carried their own source indentation** (`A1-189`). A `\`-continued Rust string
  joins with no separator, so a message broken across source lines and re-indented kept the
  indentation *inside* the sentence. `aplan graph`'s refusal on a plan with no dependency edges
  read "there is no graph." followed by fourteen spaces; `aplan import --format checklist`'s notice
  about a checkbox with no text had twenty-two; the `aplan_why` MCP tool's `all` description, which
  is what an agent reads, had thirty-seven; and one behaviour in `aplan facts` had sixteen, so the
  gap was in the published capability document. All four read as one space now, and a gate over
  every string literal in the workspace refuses the next one.

- **Five commands printed a row and never folded it** (`A1-291`). On an eighty-column terminal
  `stale` ran to 162 columns, `why` to 138, `deprecations` to 129, `due` to 118; `vacuity`'s `needs`
  column and `show`'s findings block were the same, and two notes under tables were unfolded as
  well. Each ran past the screen edge and the *terminal* broke it, at column zero, under the id
  rather than under the title. Every row folds inside its own column now.

  **The gate passed because its fixture made four of them print prose instead of rows**: `why`'s
  blocker was `done` so there was no unmet one, `stale` had no repository, `due` no `revisit_at`,
  `deprecations` no tagged task. It also measured exactly one width. It now renders a row for every
  command it measures -- a command printing none is a failure, not a pass -- at three widths, and
  the fixture's sixth vacuous case and a seventh unfolded renderer were found by those two changes
  rather than by inspection.

- **Four commands folded their stdout against the width of stderr** (`A1-291`). `ready`, `why`,
  `import` and `check-docs` asked `err_width()` for text they wrote to stdout, so with stderr
  redirected -- `aplan ready 2>/dev/null` -- nothing folded at all. `fold::row` and `fold::note`
  take no width: they ask the stream the text is going to, so a call site cannot pass the other one.
  The test that catches it needs stdout to be a terminal while stderr is not, which the pty harness
  could not express before: `script(1)` attaches a pty to both, so the two widths were the same
  number and the mistake was invisible.


- **`aplan_close` threw away the findings it did not refuse on** (`A1-224`'s audit). The MCP tool
  computed the post-state delta, refused a closure that would add an error, and then dropped the
  warnings, so an agent closing a task over a warning read a success payload identical to a clean
  closure. The sibling tools `aplan_add` and `aplan_dep` have always returned them as `notes`, and
  the command has always printed them as `note: this adds N finding(s)`. `close` now returns the
  same list under the same name.

  It matters most for the rule this audit belongs to: `APLAN038` reports a `done` task over its
  own unticked acceptance criteria, an agent is the actor that closes tasks, and an agent has no
  stderr to read (`A1-173`). The field was also asserted by no test in this repository on either
  sibling, so the one it was matching was itself driving nothing.

- **Two fence readers disagreed, and the newer one was wrong** (`A1-224`'s audit). `criteria_in`
  hand-rolled a fence toggle instead of using `anchor::unfenced_lines`, which the heading scan,
  the table-row scan and the trace scan already share. The toggle closed a fence on the *other*
  marker and on a run carrying an info string, both of which reopen a block that is still open:
  a body quoting `` ```markdown `` and then `` ```rust `` had the sample below the second run read
  as the task's own criteria. On the worked case the rule counted the sample's unticked box as
  the only criterion and missed the real ticked one, reporting `0 of 1` on a task that had
  answered itself. `unfenced_lines` exists for exactly this and says so in its own doc comment.

- **A task list written with `*` or `+` stated no criteria** (`A1-224`'s audit). Only `-` was
  matched. `aplan add` scaffolds `-` and this plan writes `-` everywhere, so this was invisible
  here and total in a repository that writes `*`: `APLAN038` would never fire, and a gate that
  appears to hold is worse than one that is absent. All three CommonMark list markers count.

- **Nothing checked that the suite invokes the surface the tool ships** (`A1-208`). `ROADMAP.md`
  section 5 asked for four audit dimensions and this was the third, deferred in a comment inside the
  corpus to a cut that has since closed.

  `cargo xtask cli-surface` enumerates 158 members from clap's own command tree -- commands and
  subcommands by full path, long flags per command, globals once -- and resolves them against 1,485
  invocations that actually happened: the recording `A1-207` added, plus the 126 `$ aplan` lines the
  `trycmd` corpus declares. Every argv is handed back to clap to be parsed, so `--config` before the
  subcommand, a flag's value, and a deliberately invalid case are all read the way the binary reads
  them. The residue is ratcheted like `coverage.json`: it may shrink and may not grow.

  It started at 41 unexercised members and is now 0. Among them: `--owner` was invoked by no test
  anywhere, `--repo` is accepted by six commands and was exercised on none, and the four selector
  dimensions were declared on nine commands with the suite reaching a handful of the pairs.

- **Forty-eight test files spawned the binary and thirty-six defined their own helper to do it**
  (`A1-207`). Twenty-five of those definitions were byte-identical, across six signatures in all.
  There is one spawn now -- `common::command`, with `output`, `run` and `aplan` over it -- and no
  integration test names the binary itself, which a gate enforces.

  The duplication was the visible half. The reason it mattered is that there was nowhere to stand:
  for a Rust test, an audit of *what was actually invoked* has no honest source but the spawn, and
  reading it out of the test sources fails twice over -- counting command names counts the word
  `check` in prose, and attributing flags by position misses `--config`, which is passed before the
  subcommand. Setting `APLAN_TEST_INVOCATIONS` now records one JSON line per invocation, argv as
  given, appended; unset, nothing is written and a run behaves exactly as before. Across the Rust
  tests that is 1,286 invocations over 30 subcommands and 41 flags. The `trycmd` corpus is the other
  half: `trycmd` spawns the binary itself, so its 126 invocations are not recorded, and do not need
  to be -- a corpus file declares its invocation verbatim.

- **`--format` was audited by a list compared against itself** (`A1-210`). `every_format_is_exercised`
  built an array holding every `Format` variant and asserted `Format::ALL` was contained in it: the
  enum against a copy of itself, driving nothing. It could not fail, and the doc comment above it had
  drifted to describing four formats when there were nine.

  The unit is a (command, format) pair, because support is per command -- forty of them, enumerated
  from the table the binary itself reads. Every pair is now rendered through the binary and the
  output classified by its bytes, which is the difference between *accepted* and *rendered*: the
  existing walk asked only whether the refusal message was absent -- honest for the 95 combinations
  that are *not* pairs, and weak for the 40 that are, so a command that took `--format sarif` and
  then printed a human table satisfied it. `verify`'s five pairs had never been
  rendered by that walk at all, since it runs where there is no repository and `verify` exits before
  it renders. Two further audits sit on top: no two formats of one command produce identical bytes,
  and no format in the vocabulary is emitted by nothing.

- **Two decision records were both numbered `ADR-0020`, and citations of it were a coin flip**
  (`A1-222`). `ADR-0020: An outcome is superseded, never reopened` (2026-08-14) held the number
  first; the discretionary-order record took it two days later and is now `ADR-0025`, which keeps
  the sequence in date order. `ADR-0026` is the reachability record, renumbered from `0025` for the
  same reason. Both files record the move, so a citation written before 2026-08-31 still leads
  somewhere. Thirty-four citations were re-read one at a time rather than substituted, which is what
  caught the one naming *Home Assistant's* `ADR-0020` and the one attributing a config key to an ADR
  that never mentioned it -- and, on the second pass, an audit line claiming `ADR-0025` was the next
  free number, which is a statement about the number and would have become false if it moved.

  `ADR-0017` now records that it was superseded, in the form `ADR-0008` and `the pre-public repository-boundary decision` already use.
  A reader opening it was told a priority field is deferred, with nothing saying the decision had
  been replaced three days later.

  **`cargo xtask check-docs` now gates the decision record itself**: a number filed twice, a
  filename that disagrees with its heading, a citation naming no record, and a path to a renamed
  one. It found eight more citations of `ADR-0003` through `ADR-0006` -- none of which exist --
  which turn out to be forward references to `HLD.md` section 19, the list of decisions still to be
  written. That list is read as a reservation now, every forward reference is named on each run
  rather than resolving silently, and a reservation is retired when its record lands: two had sat
  there since shipping.

- **A shallow clone no longer tells you to delete correct evidence** (`A1-177`). `actions/checkout`
  defaults to `fetch-depth: 1`, and a depth-1 clone has the tip and nothing else, so every `commit`
  citation fails to resolve. `APLAN011` fired per task with the fix "cite a commit SHA that exists
  in this repository, or remove the evidence" -- advice that destroys correct proof, in the one
  place an adopter meets it first, at scale, in CI. Measured against this repository's own plan:
  **164 findings across 187 tasks**, every one false.

  `verify` asks git whether the clone is shallow before resolving anything, and reports the
  checkout once instead of the plan 164 times, naming `fetch-depth: 0` and saying explicitly not to
  remove the citations. The rule code is unchanged -- `APLAN011` is still true in both shapes -- and
  a full clone is untouched: a citation that genuinely does not resolve still fires per task with
  the original message.

  **The quieter half of the same problem goes with it.** A shallow clone's *boundary* commit is
  present but its parent is not, and a missing parent was read as "a root commit, diff against the
  empty tree" -- which made that commit touch the entire repository, so `APLAN012` passed for any
  `paths` at all. The same citation errored in a full clone and reported `ok` in a shallow one. A
  boundary citation is now left unjudged and counted into the same single finding. If you have been
  gating on a shallow clone, expect a citation or two that passed to stop passing; nothing about the
  plan changed, only what could honestly be checked.

### Changed

- **A heading names the thing it is a heading for** (`A1-296`). Three kinds of heading existed and
  were styled three ways: a column header was bold, a section label -- `show`'s `paths:`, `tree`'s
  `waits on:` -- was nothing at all, and a cut's heading in `aplan roadmap` was the *version alone*,
  four bold characters in a line of a hundred, with the title a reader is scrolling for left
  unemphasised. All three carry the heading role now, and the rollup behind a cut heading is dimmed,
  so the contrast comes from both directions.

  **Styling only, and that is the decision rather than a limitation.** A rule line and a two-line
  heading are both more prominent and both change *layout*, which is what a pipe reads: six `trycmd`
  cases and the 372-line generated `ROADMAP.status.md` would move for a gain only a terminal ever
  sees. Nothing a script or a committed artefact reads has changed, and the corpus says so.


- **Automating the gate turns the warning tier off, and now three documents say so** (`A1-272`).
  `aplan check` exits `0` with warnings outstanding, by design. That is fine while a person runs
  the gate and reads its output, and stops being fine the moment CI is the only reader. Most of
  what this tool says about evidence is in that tier -- a diluted commit citation (`APLAN024`), a
  cited test that does not cover the task's files (`APLAN025`), a task `done` over its own
  unticked criteria (`APLAN038`), a traced section that moved after closure (`APLAN018`). Every
  one of them reports and not one changes the exit code -- under `--format github` they annotate
  the pull request and the check goes green anyway, which is visibility without a gate. (No count
  here: this file is a record of what a release said, and a living number in it would have to
  either rot or be rewritten after the fact. The living documents that do quote one are gated.)

  `p00002` measured it. Three warning-tier rules on, two `APLAN018` findings injected, and the
  whole plan gate green. Nothing in the tool had said that would happen, and the agent contract
  `export-agents-md` writes said *"warnings report without blocking, so read them: nothing else
  will make you"* -- true advice to a person, and advice to nobody once nobody is reading.

  No new rule: a repository that reads its warnings by hand is doing nothing wrong, and a
  diagnostic firing on every plan without `deny_warnings` would nag about an optional feature.
  What was missing is the connection between two decisions made in two sittings, so it is written
  where each is made -- `README.md`'s *In CI* section, `action.yml`'s `args` input, and
  `ci/README.md` for the staged-binary path. The generated contract's line now states the fact and
  names the condition instead of addressing a reader who may not exist.

- **This repository now takes the advice above** (`A1-276`). `.aplan.yml` sets `deny_warnings`.
  It switches `detect_drift` on deliberately, `ci/scripts/dogfood.sh` runs `aplan check` on every
  push, and nothing reads a green log -- so until this, the drift detection it had turned on could
  report a finding under a passing build, along with the sixteen other warning-tier rules. Measured
  before and after on the same tree and the same finding: `ok: 287 tasks, 1 warning(s)`, exit `0`,
  then `denied: 1 warning(s)`, exit `1`. Nothing an adopter consumes changed; a project that
  documents a trap and stands in it is worth less than one that never wrote it down.

- **The generated contract described the warning tier and the `advisory` list as one choice**
  (`A1-272`'s audits). They are two independent keys. A repository running both got *"**Every
  warning blocks** here: there is no advisory tier"*, which is false about the one rule an agent
  working there is most likely to meet; and a repository running `advisory` *without*
  `deny_warnings` was told nothing at all about it, which hides the case that matters most -- a
  downgraded **error**, a rule that does not block and looks like it must. `export-agents-md` now
  writes a tier clause and an exception clause, and all four states are asserted.

### Added

- **`import` and `add --from` say when a source carried a control character** (`A1-290`). `A1-289`
  stopped a plan's own text from steering its reader's terminal; that defends the reader and tells
  the *author* nothing, and `check` stays silent because nothing about such a plan is invalid. These
  are the two paths by which one arrives without being typed on purpose -- a document holding a
  terminal capture, or a generated manifest -- and both now name the count, to the person who ran the
  command, while they can still fix it.

  **It is not a rule, and that was measured rather than assumed**: not one field carries a control
  character, in any plan this tool has been pointed at -- fourteen plans and 2,336 tasks when it was
  measured, read through the parser rather than grepped. A rule code is a permanent contract, and
  `aplan vacuity` would report a new one here as having nothing to check on every plan that exists.


- **A plan's own text cannot steer your terminal** (`output.plan_text_cannot_steer_a_terminal`,
  `A1-289`). A control character in any field a plan supplies -- title, owner, a tag, a `paths`
  glob, a `traces` anchor, an evidence reference, and every rule message that quotes one -- is
  replaced by `U+FFFD` before it reaches a human rendering. The rest of the sequence stays as
  ordinary text, so a title holding an erase-display renders as a replacement character followed by
  `[2J`: inert, one column wide, and legible as what it is.

  YAML refuses a raw control character and then hands one over for the escaped form in a
  double-quoted scalar, so `aplan check` was clean -- nothing about the plan is invalid -- and the
  sequence reached whoever read it. Reviewing a plan before trusting it is a normal thing to do, and
  `aplan show` on somebody else's task is how it is done.

  It also *improves* what you can read. With colour off, `anstream` used to strip such a sequence
  and its parameters out of the text entirely, so a title reading `carrying <ESC>[41m a marker`
  printed as `carrying  a marker`: neither rendering showed what the title said. Both show the same
  thing now. **The machine formats are unchanged** -- `json` escapes a control character already,
  and a consumer parsing one must not have its input rewritten.


- **A census over the command surface, so colour cannot drift back out of it** (`A1-288`). Every
  command `clap` knows about is either styled -- proved under a pty, because the pipe it would
  otherwise be measured on emits nothing -- or held with a reason. Two source-level rules go with
  it: no style is built outside the palette module, and the CLI crate writes nothing to a raw
  stream, so the move that made refusals answer to `--color` cannot be undone one call site at a
  time.

  The census found the tail it was filed expecting to find. `check-docs` printed the same
  `ok:`/`failed:` verdict as `check` and `verify` and rendered it plain; `add` prints a row of the
  plan and did not style it; `close`, `amend` and `drop` name the task they acted on. All four are
  styled now, on the rule the census made necessary to state: an id carries the id role wherever the
  tool prints one.


- **A palette, and colour on every surface a person reads** (`A1-287`). Status, task ids, rule
  codes and severity now carry a role wherever they appear: `ready`, `why`, `show`, `tree`,
  `roadmap`, `tasks`, `due`, `vacuity`, `paths`, `critical-path`, `deprecations`, and the refusal
  paths. A reader scanning a hundred-row roadmap for what is not `done` was doing by eye what one
  hue does for them.

  Eight roles in one module, and no renderer builds a style of its own any more -- seven of the
  eleven previous call sites had each declared the same `bold`. Four-bit ANSI throughout, so the
  palette resolves against the reader's own terminal theme rather than looking right only on the
  machine it was chosen on. Colour is never the only carrier: `--color=always` with every escape
  stripped is byte-identical to `--color=never`, asserted over every human rendering the tool has.

  **Folding is style-aware now, which it had to become.** A styled run folded across a break left
  the style open: the first line opened one it never closed, the middle inherited it, and the last
  closed one it never opened -- and the continuation's indent was inside the style, so an indented
  continuation of a coloured row was a coloured margin. `fold`, `fold_after` and `fold_block` close
  what is open at a break and re-open it after the indent. Text carrying no escape is untouched.


- **`--color=auto|always|never`, and the tool's own prose on the same stream as its findings**
  (`color.explicitly_selectable`, `A1-286`). `auto` is the default and is what every earlier build
  did: colour at a terminal, plain bytes in a pipe, with `NO_COLOR`, `CLICOLOR_FORCE` and
  `CLICOLOR=0` honoured. What there was no way to say is *the detection is right and I want the
  other thing*: colour into a pager or a screen capture, or none at a terminal without exporting a
  variable for one run. The flag overrides the environment in both directions.

  The reach is the other half. The CLI crate wrote to `std::io::stdout()` and `eprintln!` directly
  -- thirty-seven, twenty, sixteen and seven call sites of the four spellings -- so its own lines
  could be neither styled nor stripped. `aplan show NOPE` printed identical bytes at every setting
  while a finding did not, and `clap` coloured a mistyped flag red while the tool's own refusal, the
  more serious of the two, stayed plain. Everything now writes through `anstream`, and the choice is
  written before `clap` parses, so a usage error obeys the flag that produced it.

  Colour carries emphasis and never information: `--color=always` with every escape stripped is
  byte-identical to `--color=never`. The machine formats -- `json`, `agent`, `github`, `sarif`,
  `markdown`, the Mermaid from `graph`, `facts`, `schema` -- are byte-identical at every setting,
  and `aplan mcp` is untouched.


- **A parked task can name when to look at it again, and `aplan due` reads it** (`A1-264`). Parking
  says *not now*; nothing said *when*. `p00013`'s `DN0-29` waits on GitHub creating a real weekday
  `schedule` event, and the earliest useful revisit is computable from a committed cron and a
  measured delay -- but the instant lived in prose, so a human had to copy it into a scheduler and a
  missed copy leaves correctly parked work invisible indefinitely.

  `revisit_at` is an optional RFC 3339 instant on a task, and `aplan due --as-of <instant>` lists
  the parked tasks whose instant has arrived, longest overdue first. **It never means the wait is
  over.** Reaching the hour says nothing about whether the event exists, so the field changes no
  status, satisfies no dependency, and the report says so in as many words. `check`, `ready` and
  `verify` still never read a clock; `due` is the only command that does, and `--as-of` makes it
  reproducible.

  This does not reverse `A1-175`, which declined a structured *reason*: all 22 measured parks waited
  on an unfalsifiable human judgement, and a field holding one would be a claim nothing could check.
  A revisit instant is the narrower claim *do not look before this*, and the reason still lives in
  the body.

- **`APLAN042`, a `revisit_at` that names a real instant on a task that is parked** (`A1-264`). Two
  ways one property fails: an instant this tool cannot parse is a hand-off no scheduler will act on,
  and one on a task that is not parked is inert, because `ready` already offers everything else and
  `due` reports only parked work. A warning, because neither weakens a claim the plan makes -- what
  it can do is quietly fail to reach the scheduler that was supposed to read it.

- **`aplan stale` reports parked work** (`A1-275`). `Status::Blocked` is *explicitly parked for a
  reason outside the dependency graph*, and it is live work: somebody means to do it. `ready`
  excluded it, correctly -- it is not startable -- and `stale` excluded it too, so between them
  **the only work nothing tracked was the work nobody was doing.**

  A park is a decision with a half-life, and how long it has held is exactly what this command
  already computes for everything else. It keys on the status the model already has, so a plan
  needs no tag convention and there is nothing to migrate. A plan that parks nothing reads exactly
  as before, and a parked *milestone* stays out like every other milestone.

  The report says what a `blocked` row means -- not waiting on the plan, waiting on something
  outside it -- because otherwise a reader cannot tell it from a task whose blockers are unfinished.

- **`aplan_identity`, a ninth MCP tool** (`A1-259`). `p0000014` pointed an agent at a configured
  MCP server whose binary predated its checkout, and spent a pass diagnosing a path-resolution
  defect the checkout had already fixed. The CLI could say what build it was; the server could not,
  and **asking an installed CLI does not identify a configured MCP process** -- the two can be
  different executables.

  It returns the same `aplan/tool-identity/v1` document `aplan facts --identity` prints, handed to
  the server by the binary that computed it rather than computed a second time. It takes no
  arguments and needs no plan, no repository and no clean tree, because the reason to ask is
  usually that something else did not work.

  The two documents are compared **over the transport**, not only through the shared function: a
  shared function is exactly what would still agree if the wiring handed the tool a document from
  somewhere else.

  **A configuration this build cannot read no longer stops the server**, it stops every tool except
  this one. An old aplan meets a config key a newer one added and calls it a typo, which is how a
  stale binary usually announces itself -- so `aplan mcp` used to exit before serving in exactly the
  case where *which build are you talking to* is the answer. Every other tool returns the config
  error, pointing at `aplan_identity`, so nothing proceeds under a policy nobody chose.

- **`aplan why` stops at a cut instead of walking through it** (`A1-263`). A plan that sequences
  its cuts the way ADR-0013 prescribes puts every earlier cut behind the one in front of it, so the
  transitive closure of a late task is mostly the roadmap. `p0000014` asked why one launch task was
  blocked and got 86 lines: two work blockers, the unfinished cut, and every live member of three
  earlier cuts.

  A cut is now reported as itself, with a count of what is behind it and how to see it, and `--all`
  gives the closure an audit wants. Measured on a plan a quarter of that size: 52 lines to 5.

  **An answer that folded anything says so**, because a fold that did not would be a truncation
  reading as exhaustive truth -- worse than the eighty-three lines it replaced. The human output
  names each folded cut with its count, and closes with `traversal: cut-bounded`; an answer that
  folded nothing is the complete answer and carries no caveat, so nothing changes for a plan
  without cuts in the way. The JSON always carries `traversal`, because a consumer diffs fields
  rather than reading for emphasis, and a folded cut carries `behind`.

  The default changed, deliberately and on the evidence: the cost is highest on the MCP surface,
  where the answer spends an agent's context on a roadmap it can query separately, and an agent
  will not pass a flag it does not know it needs. `aplan_why` takes the same `all`.

- **`add --from` entries carry a `body`** (`A1-260`). The bulk seam took every field `aplan add`
  takes as a flag and not the one thing a flag cannot carry: the Markdown that makes a task
  closeable. `p0000014` authored 134 tasks from a research document through it, and all 134 landed
  with the scaffold.

  That is not a tidying pass deferred until later. Leave the scaffold's line unticked and
  `APLAN038` reports every closure; tick it and `APLAN039` does. Both rules are right, and between
  them a plan authored through the supported surface could not be closed without editing every file
  through the format that surface exists to hide.

  Optional, and absent keeps the scaffold, so every existing manifest is unchanged. Present replaces
  it whole -- this is not a template to merge into, and merging would be inventing content. An empty
  string is refused and names the entry: a generated manifest producing `body: ""` is a lookup that
  came back empty far more often than it is an author asking for a task with no prose, and leaving
  the key out already says that.

  The run says how many of the tasks carry a body. A manifest is generated, so a template that
  emitted one for some entries and not others reads exactly like a run that worked, and the
  difference otherwise surfaces at closure when `APLAN038` reports every task that kept the
  scaffold -- which is the same finding one level up (this task's audit).

- **`APLAN041`, the generated `AGENTS.md` block gone stale** (`A1-273`). `export-agents-md` writes
  the contract an agent reads before touching the plan, and that block is a second copy of the
  policy -- the thing this tool exists to distrust, generated by this tool. `p00002`'s said *"a
  `done` task must cite `commit`, `file` or `test` evidence"* for a full day after
  `require_min_strength` had dropped `commit`, so the file taught a rule the tool would refuse. It
  was found by running `--check` on a hunch.

  `--check` was never the gap; **pressure to run it** was. `check` and `verify` now compare the
  block themselves, and so does `aplan_check` over MCP, which is the surface an agent actually
  reads the contract from.

  Scoped by the markers, which are the repository asking: no file, or a file carrying none, is
  silent, so an adopter who never ran `export-agents-md` is not nagged about a feature they did
  not want. A file carrying *half* a pair is a finding rather than a silence -- otherwise the way
  to switch this rule off is to delete one comment line.

  And silent wherever it would have nothing honest to say, which its audit found three of: under
  `--no-config`, where the block would be rendered from this tool's defaults rather than any
  repository's policy; under a `--config` naming a policy that is not the one the tree declares,
  which is the reviewing case that flag exists for; and on a plan that did not load, because the
  block names the id convention the plan teaches and a plan read from a subset renders a subset's
  block. All three reported a current contract as stale before the fix.

- **`export-agents-md` wrote a path relative to the directory it was run from** (`A1-273`'s audit).
  The block names where the plan lives, and resolved that against the working directory rather
  than the repository: run from a subdirectory with `--root ../aplan`, it wrote *"planning lives
  under `../aplan/`"* into a committed artefact whose whole point is reading the same on the next
  checkout. Found by `APLAN041` reporting a contract that was not in fact stale.

- **`APLAN040`, one `paths` entry written twice** (`A1-267`). A task may list the same glob more
  than once and nothing said so. It is not a correctness defect and the severity says so: the
  copies match the same files, `verify` resolves identically, `APLAN012` binds the same commits and
  no citation is weakened. It is a *signal* defect, and the signal is the one `paths` exists to
  give -- a repeated entry is almost always a botched edit.

  `p00016` found two tasks carrying the same glob twice while consolidating a crate map from thirty
  crates to fifteen, which meant rewriting `paths` across 22 task files. Both predated that
  rewrite, and the audit that found them was written by hand because nothing else would have.

  Two spellings of one path are one entry, read through the same normalisation the glob compiler
  uses, so the rule and the matcher cannot disagree about what "the same" means (`A1-254`).
  Containment is not duplication: `src/**` beside `src/a.rs` is silent, deliberately, because
  recognising that one glob subsumes another is a much larger claim than two writings of one entry.

- **`APLAN039`, the scaffold ticked as though it were a criterion** (`A1-271`). `aplan add` writes
  `- [ ] state what done means for this task, in this repository's terms`. That is the instruction
  to write a criterion, not one. `APLAN038` reports it left unticked; **ticked**, it passed clean,
  and the task recorded that the instruction had been carried out.

  Two pilots made it concrete rather than hypothetical. `p00020` closed four tasks over the
  placeholder unticked -- the version `APLAN038` already caught. `p0000014` authored **134 tasks in
  one `add --from`**, every one carrying the line, with 134 file edits standing between them and an
  honest closure; ticking is the shortcut that ends that job in a single pass.

  It is the one content this tool reads, and only because it is the one content this tool wrote.
  `A1-224` made `APLAN038` count checkboxes and never look inside them, on the grounds that a rule
  about a markdown convention must not demand the convention. The scaffold is the exception that
  proves it: the sentence is a constant here, so recognising it is not a guess about somebody's
  prose. Matching is exact after trailing whitespace and never a prefix, so a criterion of yours
  that begins the same way is silent.

  A warning, at the first unticked line. Silent on live work, since every task `add` creates
  carries the line. Reported even beside real criteria, because the false claim is recorded either
  way. The remedy is to write the criterion, not to delete the line, and the `agent_instruction`
  says so.

- **`APLAN038`, a `done` task over criteria nobody answered** (`A1-224`). `aplan add` scaffolds
  `- [ ] state what done means for this task, in this repository's terms`, and `close` wrote the
  status, the evidence and the trace fingerprints and touched nothing in the body. The tool
  authored the obligation and never read it.

  Measured over this plan's 220 closed tasks before the rule existed: 211 fully ticked, 7 with
  none ticked, 2 stating none at all, and **none partially ticked**. That zero is the informative
  number -- nobody was making a per-criterion call and leaving one open -- so this is a
  forgotten-step gate rather than one that overrides a judgement. All seven were closed on the
  same day through `aplan close`, used exactly as designed.

  It is a warning, and it reaches the author twice: `check` reports it at the first unticked
  line, and `close` reports it at the moment of the closure, because it compares the findings
  before and after the edit it is about to make. Under `deny_warnings` that second report is a
  refusal and nothing is written. `advisory: [APLAN038]` demotes it for a plan mid-migration.

  What it does not do is tick anything. A `--tick` flag that marked every criterion met because
  the task was closed would manufacture exactly the claim this tool exists to disbelieve, and the
  suite asserts both halves: no command offers one, and `close` leaves the body byte-identical.

  A checkbox inside a fenced code block is quoted rather than stated, and is not counted. The
  rule reads a *count* -- total, ticked, and the first unticked line -- rather than the body, so
  `Task` gains no field, the derived schema is unchanged, and importers have nothing to map. Its
  message says "criteria in its body" and names no heading: nine of this plan's own milestones
  head that section `## Exit criteria`, and an adopter's convention is theirs.

- **`dep` stops reformatting a list it was only adding to** (`A1-179`). `aplan mv` edits
  `blocked_by` surgically and keeps whichever form the author used; `aplan dep add` replaced the
  whole key with a block list, so a milestone written the way this tool writes them was silently
  reformatted by one command and not the other. `dep add`, `dep rm` and the MCP `dep` tool now keep
  an inline `blocked_by: [a, b]` inline, with the spacing after the colon and any trailing comment,
  and still write a block where they found one or are creating the key.

  Two smaller changes remain and are worth knowing: quoted ids come back bare, because the list is
  re-emitted from what was parsed rather than copied as written -- an id that *needs* quoting is
  refused rather than written wrong -- and a flow list broken across lines is collapsed to one,
  which is a shape `mv` refuses outright. `evidence` is still normalised to a block by `amend`,
  deliberately: it is a list of mappings, and the flow form is one this tool has never written.

- **An attestation says what an execution was worth** (`A1-178`). `attest` records `tests_executed`,
  and `A1-167` showed that can be true of a run that proved nothing: a runner whose filter matches no
  test skips everything, exits `0`, and reads as a pass. `verify --execute` reports that as
  `APLAN037`; the document did not carry it, so the artefact whose whole value is being checkable by
  somebody who was not there made the strongest claim this tool can make about a run that did not
  happen.

  The document now carries `runners_that_proved_nothing` beside it, a count of the runners
  `APLAN037` reported. **Absent when zero**, so a repository whose runners select properly gets a
  byte-identical document and the field's presence is the signal. Not a refusal: `verify` judges and
  `attest` records (`A1-040`), and a repository that has heard the warning may still attest -- what
  it may not do is publish a document that hides it.

- **The generated agent contract stops moving when work does** (`A1-180`). `export-agents-md` picks
  the id example from the plan's own ids, and it picked the family with the **most tasks** -- so in a
  plan running two conventions, ordinary work rewrote a *policy* block. In `p003`, `K4-*` and `K6-*`
  are equally real cohorts: adding one K6 task changed the contract from `K4-nnn` to `K6-nnn` in
  three places and failed `--check`, with no id policy changed. `AgentContract`'s own doc says what
  that costs -- a block that moves when work happens fails its own check daily and is turned off
  within a week.

  A family now counts when it still has **live work**, not when it has the most tasks, and every
  current family is named: `Task ids follow this plan's own conventions (`K4-nnn` or `K6-nnn`)`.
  Live work is the signal the majority rule was groping for -- a *stray* convention is one nobody is
  adding to, so it stays unnamed and a predominantly single-family plan keeps its single example.
  Where nothing is live the whole plan is history and every family is named, because there is no
  current convention to prefer.

  Unchanged: a plan whose ids teach no numeric convention, or which uses ULIDs, still says nothing
  about ids rather than inventing one. And this is a renderer repair only -- `add`, MCP `add` and
  importer inference mint through a different function, measured by minting the same id on binaries
  either side of the change.

- **`check` says which half of the gate just ran** (`A1-173`). `aplan check` is structural -- schema,
  the DAG, and the rules that need no repository -- so a `done` task whose cited commit or test
  stopped resolving passes it clean. That is the design, and what it did not do is *say so*:
  `ok: 1 tasks, 0 warning(s)` beside `--deny-warnings` reads like a thorough answer. The fifth pilot
  found two repositories gating on `check` alone and called it "the single most likely
  misconfiguration, because `check` alone looks like it is working"; one sat undetected until a
  later re-audit.

  A run over a plan holding evidence now ends with a note on **stderr** naming the count and
  `aplan verify`. Only where it would matter: it counts what `verify` would have opened -- `commit`
  and `file` against git, `test` and `url` against a configured program -- and says nothing when
  that is zero, so a plan whose only citations are `note`s is silent and a line on the fastest
  command in the tool does not become the one nobody reads. On stderr because a `--format json`
  consumer's stdout stays one document, which is where `denied:` already goes.

  **The MCP `check` tool carries the same fact**, as `evidence_unresolved` with the count and
  `resolved_by: "verify"`, because an agent has no stderr to read: it sees `errors: 0` and concludes
  the plan passed, which is the defect `A1-101` fixed for the verdict. Absent rather than zero, so a
  plan with nothing to open reads as it always did. `aplan_verify` does not carry it -- that is the
  half which opens them. The count itself lives in `aplan-core` and both surfaces read it.

  The third instance of a shape this tool has twice: `--no-config` announces the policy it did not
  apply, and `check-docs` refuses rather than report a clean run it cannot back.

- **`aplan mv --phase`, so a task's phase is a command and not a hand-edit** (`A1-171`). `phase` was
  the one field every task carries that nothing wrote after creation: `add --phase` set it once and
  `mv` took only `--cut`. Re-grouping work -- which is what re-prioritising *is* -- therefore ended
  in a hand-edit of the files this tool claims to own, in the same session that used `mv` and
  `dep add` precisely to avoid one. `summary` rolls up by phase, so a plan could express a change of
  cut and not the change of grouping that went with it.

  `--cut` is now optional and `--phase` is new; at least one is required. Given together they are
  **one** operation: one post-state, judged once, written as two steps behind the same
  all-or-nothing probe -- so a task file that cannot be written refuses the cut half too, rather
  than leaving membership saying one thing and the rollup another. A task already in the phase is
  said so and its file is not rewritten.

  The edit is surgical, like `close`'s: the value token is replaced and the key's spacing, any
  trailing comment and the line endings are yours. An indented `phase:` belongs to some nested
  mapping and is never the one rewritten. Nothing gates on a phase -- no rule reads it, and the
  schema says it is for rollups and ordering -- so this is the one move that cannot be refused for
  what it produces, only for a file it cannot write.

- **A cut this tool creates can be filled by this tool** (`A1-170`). `aplan add --tag milestone`
  writes a milestone with no `blocked_by` key, and an absent key was not a list `aplan mv --cut`
  could append to, so `mv` refused and told you to change the file by hand -- which is the thing
  `mv` exists to stop. Both doors were shut: an empty cut could not be filled, and
  `add --blocked-by` could not seed one with ordinary work, because the task would briefly belong
  to two cuts (`APLAN022`, correctly refused). Under `require_cut_membership: true` there was no
  third door, because a task cannot exist outside a cut to be moved in from.

  `mv` now creates `blocked_by` when it is absent, rather than refusing. Not by having `add` write
  an empty list: an absent key and `blocked_by: []` are indistinguishable to everything that reads
  a plan -- the same `APLAN021`, the same `show` -- so a convention in what `add` writes would fix
  nothing for a milestone somebody else wrote or an importer produced. The shapes that are still
  refused are the ones this editor would have to *guess* at: an anchor, or a list broken across
  lines. The two-cut refusal is untouched.

- **`APLAN037`, a test that "passed" under `--execute` without running** (`A1-167`). `--execute` is
  the strongest rung of the ladder: L1 says a runner knows the name, L3 says the test *passed*. L3
  is the run command's exit status, and that is worth what the runner makes it worth. The assumption
  underneath is invisible until it breaks: **the name a runner lists is the name that selects it**.
  `vitest list` prints `file > suite > test` while `vitest run -t` matches a test's own title, so a
  citation of the listed name selects nothing, every test is skipped, and the status is `0`.
  Measured on vitest 4.1.11: a deliberately failing test, cited and executed, verified clean.

  Exit status cannot catch it -- `cargo test -- --exact <no such test>` exits `0` too, and that is
  what a correctly configured cargo runner does. So the runner is asked what a *miss* looks like, by
  running `test.run` once against a name no test can have, and every passing run is compared with
  its answer. Nothing about any runner is built in. Measured on the three the pilots use: pytest
  9.1.1 exits `4` and is settled by status alone; cargo exits `0` and prints `running 0 tests` where
  a hit prints `running 1 test`; vitest exits `0` and prints the same bytes either way.

  One sentinel run per runner, only under `--execute`, and only for runs that **passed** -- a run
  that failed is already `APLAN014` and already sends you to look. Reported once per runner against
  `.aplan.yml`, naming how many citations rest on it. Compared with digits masked, because a clock
  and a duration are what a runner stamps its own output with; the cost is that a runner whose only
  difference between a hit and a miss is a *number* cannot be told apart, and `test.run_selects:
  true` is how a repository says its `run` selects exactly.

- **`APLAN036`, a configured test runner that lists nothing** (`A1-064`). A typo in `dir`, a `list`
  command that quietly stopped matching, a manifest path that moved: each produces an empty list, no
  error, and citations that fail later as "not one any of the configured runners list" -- which
  blames the citation, so the author opens the task file and the configuration that stopped seeing
  its tests is the last place they look. It matters more with several runners: four can work while
  one is quietly blind, scoped to whichever component nobody has cited yet.

  Reported once per runner against `.aplan.yml`, never per task, and a **warning** because the
  benign case is real -- the repository that found this has two runners that list nothing because
  those components have no tests yet. Say so with `test.expect_tests: false` rather than deleting a
  runner you will need. Only in a run that is already resolving `test` evidence, so `verify` on a
  plan that cites only commits does not start compiling a workspace to check.

  **What it does not catch**, measured and worth knowing: a runner that lists *something* is silent
  however wrong it is. A runner built for one feature set lists every test except the gated ones it
  was cited for, and `pytest --collect-only -q` prints `no tests collected in 0.00s` when it finds
  nothing, so an empty pytest runner lists exactly one "test". No count separates those from a small
  component.

- **A config key cannot ship without the changelog naming it** (`A1-169`). The same gate a rule code
  has had since `A1-067`, for the contract with the sharper failure: an unknown key is *refused*, so
  a repository that adopts one its staged binary predates loses every command that reads config, not
  one gate. Top-level keys only -- `test.name` arrives when you adopt `test`, and asking the
  changelog to restate the schema helps nobody. Run against this repository it found `docs`, added
  the day before and named nowhere.

  `docs/upgrading.md`'s account of that failure had drifted from the tool: its worked example showed
  a message `A1-068` replaced, using a field that has since become valid, so the one section a reader
  lands on when they hit this could not be reproduced. It now shows both real transcripts -- a typo,
  which gets a `did you mean`, and a newer key, which gets none -- and says how to tell them apart,
  that `aplan facts` settles it because it reads no config, and what answers a read-only question
  while the binary is still old. Every row about a config key now says the cost of adopting it.

- **The CI path a cross-account adopter can actually use is documented here** (`A1-056`). The
  README told everyone to write `uses: asamarts/aplan@v0.2.0`, which is the one path that cannot
  work from another account: this repository is private, and GitHub cannot resolve a private
  action from another repository at all, with or without a token. Two pilots hit it, and the
  arrangement that works lived only inside one of their repositories. The action snippet now says
  what it requires before a reader writes the workflow, and `ci/README.md` carries the procedure,
  the alternatives with their costs, and the hosted-runner variant neither pilot needed.

  Writing it found two things the consumers' own copies had wrong: neither verifies the release
  against its published `.sha256` asset, which exists and works, and the capability probe both use
  (`aplan facts --require`) is newer than `v0.2.0`, the only release there is, so on a downloaded
  binary it fails on the flag rather than reporting a capability.

- **`test` evidence is proved against runners that are not cargo** (`A1-034`). Every claim that
  this tool is language agnostic was an inference from the shape of `.aplan.yml`; it has now been
  run against `pytest 9.1.1` and `vitest 4.1.11`. pytest works end to end, including a
  parametrised node id whose name carries a space, a comma and brackets, and the citation reaches
  the runner as one argument.

  **Two things a real runner broke, both now documented where an adopter meets them.**
  `verify --execute` believes the run command's exit status, and that is worth what the runner
  makes it worth: `vitest list` prints `file > suite > test` while `vitest run -t` matches the
  test's title, so the listed name selects nothing, every test is skipped, the exit status is `0`,
  and a cited test that was *failing* verified clean. Check yours once by running your `test.run`
  command with a name no test could have. And `pytest --collect-only -q` ends with a summary
  sentence that cannot be suppressed, so a pytest runner that collected nothing lists exactly one
  test and a misconfigured runner is indistinguishable from a working one.

- **`aplan check-docs` reads a cut's membership, where the document declares one** (`A1-165`,
  `docs.membership`). A plan document that lists each cut's tasks beside a `M-<cut>` milestone
  holds two copies of one fact, and until now only this repository's own `xtask` read both. It
  fails when the copies disagree in either direction: a member the document does not declare, and a
  declaration the milestone does not include. This repository caught exactly that on itself while
  the feature was being specified.

  **Declared, not guessed.** A new top-level config key, `docs`, holds it -- so **upgrade the binary
before adding it**, since a build that predates the key refuses every command that reads config.
`.aplan.yml` says where a document declares membership -- a line with
  a `marker`, or a task table whose *first cell* is the member (ADR-0015) and whose remaining cells
  are prose. Both shapes come from a real document rather than from imagination, and the
  distinction is the whole feature: measured across the two, reading a cut's whole section instead
  of its declaration calls fourteen ordinary cross-references false declarations, and reading only
  the declaration calls none. A document a config does not name, and a milestone a document has no
  section for, are both silent, so turning this on cannot fail over a cut that shipped before the
  document grew sections. Absent the key, `check-docs` behaves exactly as before, and its JSON
  gains a `membership` key that is an empty list. A milestone the document has no section for is
  silent, and so is a release, which holds cuts rather than work. Fenced code is never a
  declaration, so a document may show its own format; two headings naming one cut is reported
  rather than resolved.

- **A gate that every pilot finding says what became of it** (`A1-166`). pre-public adoption evidence
  states the one rule those logs have -- a friction item that becomes work names the task id that
  owns it -- and nothing checked it. `check-docs` asks whether an id a document names resolves to a
  task; it cannot ask whether a finding named one at all, which is the direction that rots. Three
  had not, and the worst was `p00020` item 24, labelled "blocker for this consumer", which `A1-138`
  fixed and which said nothing about it: a reader saw the pilot's loudest finding standing open
  while the plan said otherwise.

  The gate describes the convention rather than imposing a new one: a disposition may be a
  `**Disposition.**` paragraph, a heading tagged `(no action)`, or the task id named in the
  finding's own prose, and a shared paragraph covering several findings has to say which item
  numbers it answers. It checks 88 findings, and a heading that opens with a digit but does not
  parse as one is itself reported, so a log written in an unrecognised style cannot pass by leaning
  on the other six. Repository-specific, beside the other prose gates, and
  not a shipped command.


- **`aplan vacuity`** (`A1-114`), which reports what each rule could actually have failed on. A rule
  that evaluates to "pass" and a rule that had nothing to evaluate produce the same output, and the
  second is the more common state -- the second pilot named that while reporting a capability check
  whose shell substitution had been eaten, so it asserted nothing and exited `0`. Four rules that
  catch a check having quietly not checked anything have shipped, each written from a pilot finding
  the vacuity by hand; this is the mechanism they are instances of.

  Per rule: the tasks it had a subject in, the tasks whose predicate had nothing to compare against,
  and what it needed. **Derived from the rules themselves** -- each declares its applicability on the
  same row that generates its code, its severity and its catalogue entry, because a second table of
  "which rules can be vacuous" is the same defect one level up. Hermetic and advisory: a repository
  mid-adoption is blind nearly everywhere, and the number is there to be watched coming down.

  Not in `facts`, deliberately: `facts` describes what a *build* is, and a vacuity count is a
  property of a *plan*. Commit dilution is not counted as vacuity either -- the predicate binds and
  the comparison happens, so it is a confidence question, which is `APLAN024`'s and stays there.

  A declaration beside a rule is only *adjacent* to it, so a gate checks it against behaviour: a
  rule that fires on a task must have declared a subject there and must not have called that task
  blind. It counts what the task carried; a rule can also have nothing to check for a reason in
  `.aplan.yml`, which the report says it does not measure.


- **`aplan export-roadmap-json <file>`** (`A1-033`), which writes the plan's statuses and dependency
  edges back into a `roadmap.json` contract so they stop being hand-maintained. `--check` compares
  and never writes, the gate shape `roadmap --format markdown` already has.

  **A merge, not a render**: that file carries fields this model does not hold -- `project`, `note`,
  `adrs`, `specs`, a workstream's `authority` and `status_validity`, a phase's `blurb` -- and a web
  build and a spec linter read them, so generating from the plan alone would produce a file that
  validates here and breaks there. The document is its own template; the plan supplies the half that
  drifts. A milestone the plan has and the file does not is **reported rather than invented**,
  because where it belongs in a foreign schema is not something this can know.

  A `done` the plan has not proved goes back as `not_started`, because writing it back would launder
  the claim through a file that has just stopped being hand-maintained. Demonstrated end to end
  against the real repository: import, close fifteen milestones with **their own document citations**
  (which `A1-159` made citable, zero refused), regenerate -- and all twenty-five milestones read
  exactly as the source had them, with that project's own integrity gate accepting the result.


- **`aplan stale --as-of <YYYY-MM-DD>`** (`A1-162`), and the report now names the clock it measured
  from. Ages come from `HEAD`'s own commit time so two runs at one commit agree, which is right for
  a gate and wrong for a repository that has stopped moving: **every age freezes with the last
  commit, so a plan abandoned six months ago reports nothing as idle.** Naming a date is the
  explicit way out that keeps the property -- the same date at the same commit gives the same
  report. The whole named day counts, so `--as-of $(date +%F)` works on a day you have committed; a
  date genuinely before the last commit is refused rather than clamped, because every age would be
  negative and the report would say nothing has sat.

  This is what came of the `p0000019` pilot's fourth finding: that plan declares how long its status
  is good for (`valid_through`), and this tool reported it clean six days after that window closed.
  **A declared expiry is deliberately not modelled**, and the pilot's own dates say why: its window
  closed on 2026-08-13 and its last commit is 2026-08-10, so only a wall clock sees it -- and a wall
  clock inside `check` means the same commit passes on Monday and fails on Tuesday with nothing
  changed. Measured across six real plans and 483 tasks, one task carries dated evidence at all: the
  kinds that could go stale are the ones nobody writes, and the ones everybody writes are pinned to
  git and do not rot. `aplan import --format roadmap-json` now says what that field declares and
  that the gate enforcing it has to stay.


- **A `file` citation may name the section that proves it** (`A1-159`): `docs/report.md#result`, or
  `docs/report.md#result@abc123` to read it at a rev. `verify` checks the document holds that
  section, using the same heading-and-table-row reader `traces` uses, so an anchor copied out of a
  rendered URL resolves in both places.

  It comes from a repository whose entire notion of proof is a reviewed document: **54 of
  `p0000019`'s 59 document references name a section, every one of them in a field that project
  calls `evidence`**, and none could be held here. Dropping the anchor lost which section was the
  proof; keeping it in `traces` said the document *caused* the task while `APLAN005` reported the
  task as unproven. Attaching that contract's own citations now resolves all 58 of them and takes
  its unproven `done` count from 38 to 23 -- the 23 being the items it genuinely cites nothing for.

  It stays a `file` citation on the ladder, so `require_min_strength` needs no new word: naming a
  section only narrows what the citation claims. The section is read out of the repository at the
  cited rev, never off disk, so an uncommitted edit to a proof cannot settle a citation for its
  author and fail for everybody else. `APLAN015` carries the three new sentences -- an empty anchor,
  a section that is not there, and a document with no text to look in -- because that is the same
  fault at a finer grain, exactly as `APLAN008` already covers a `traces` target and its anchor.
  Deliberately **not** fingerprinted: a document that must not drift is what `traces` is for, and a
  task may carry both. Writing the two halves the other way round (`path@rev#section`) is told which
  order to use rather than answered with a missing document, and `aplan import --format roadmap-json`
  now names the `file:` citation to attach when the imported task is closed.


- **The `p0000019` pilot** (`A1-036`), the experiment this tool's adoption effort was built to run,
  and the first against a hand-maintained plan that already has drift detection of its own: 1,269
  lines of Node validating a `roadmap.json` on every CI run. **It found no drift.** No dangling
  reference, no cycle, no milestone claimed done ahead of something it depends on, and all 49 of its
  document anchors resolve.

  **And on the day it ran, that plan was stale by its owner's own rule, and this tool said clean.**
  The workstream stamps its status `valid_through: 2026-08-13`; the run was six days later, and
  their gate exits `1` on it when given a clock. So the result is sharper than "no drift": on the
  axes this tool checks, a careful team with its own integrity gate had not drifted; on the one axis
  that plan was failing, this tool has no concept at all. pre-public adoption evidence records it with
  the numbers -- 4,083 tracked files, 464 commits, 47 tasks, `check` and `verify` in 8 ms each.

  Every error the run did produce was this tool's, and each is filed rather than noted. Fifty-four
  of that contract's 59 document references name a *section* and all of them sit in a field called
  `evidence`, which no citation kind can hold (`A1-159`); `not_applicable` -- which its own linter
  treats as satisfying a dependency -- has no status here that neither overstates the work nor
  blocks its dependents (`A1-160`); a phase's `depends_on` is dropped under a message saying the
  task model cannot hold a dependency edge (`A1-161`); and a status may declare how long it is good
  for, which is a stronger claim than `aplan stale`'s git proxy and one nothing here can read
  (`A1-162`).


- **`aplan deprecations`** (`A1-021`), the runways a plan declares. "Announce it in one release,
  remove it in a later one" is the most checkable promise a plan makes, and it needs no schema
  here: a runway is two ordinary tasks, one tagged `deprecation` that announces and one tagged
  `removal` that does it and lists the announcement in its `blocked_by`, each a member of its cut
  the way any task is. This reads that back and names the ways the pair breaks -- announced
  with nothing scheduled to remove it, announced and removed with no release in between for
  anybody to migrate in, removed in a cut that ships before the announcement, and the announcement
  cancelled while the removal was not -- plus a removal nothing announced, which is the same break
  from the other end. Hermetic (it never opens git) and
  advisory (it exits `0` whatever it finds), because across the seven repositories this was
  measured on **not one declares a runway**: the artifact is a property of projects carrying public
  API promises, and a rule firing on nobody's plan is one nobody can act on. `--format markdown`
  prints the table a project publishes.

  The git-side check that was scoped with it -- does a `done` removal task's cited commit delete
  its declared `paths`? -- is **refused with the numbers**: of 282 commits across seven
  repositories that describe removing something, 6 delete a file. Removing something is editing a
  file, not deleting one, so that check would refuse 98% of correct removals.


- **`aplan paths`** (`A1-151`), which answers "would almost any commit satisfy this task's `paths`?"
  A `commit` citation is checked by asking whether that commit touched the declared paths, so where
  the paths cover most of what a project does, the check is close to a coin flip -- the first pilot's
  case. Breadth against the *file tree* does not separate it; breadth against *commits* does. This
  samples the last 100 commits (`--sample`), reports each task's share broadest first, and prints
  the plan's own median beside them, because that is the only thing the number can be read against:
  measured across five real repositories, the median task's paths are matched by 8% of recent
  commits in one plan and 55% in another. It is **not a rule**: at any threshold that catches the
  pilot's task, two thirds of that pilot's plan is flagged, correctly and unactionably. Never fails
  a build.

  It also says when it measured nothing, which the first cut did not: a repository with no commits
  printed a full table of `0%` and a median of `0%` for a history it had not read, and a `paths`
  entry that will not compile was counted as `0%` -- the narrowest row the report can print.
  `sampled` is what says a measurement happened, `median_percent` is `null` when none did,
  `unmeasurable` counts the entries `APLAN032` names, `--sample 0` is refused, and a share too
  small to round reads `<1%` rather than `0%`, with `hits` in the JSON so the exact fraction
  survives (`--require paths.no_sample_is_not_a_zero`).
- **`--config <file>` and `--no-config`** (`A1-155`,
  `--require config.explicitly_selectable`), an explicit trusted-configuration boundary. `.aplan.yml`
  names the programs a `test` or `url` citation runs and was always *discovered* by walking up from
  the plan root or the repository -- so a trusted binary reviewing somebody else's checkout still let
  that checkout choose the programs, which a pilot demonstrated with a config whose runner wrote a
  marker file. `--config` reads exactly the file you name and discovers none; `--no-config` reads
  none at all, and says so, because `test` and `url` citations then stay unchecked rather than
  passing. A named file that is missing is an error, never a fallback to the tree under review.
  Relative values in a selected file resolve against its directory, while the plan's own location
  stays where git history is read. Both surfaces honour it: the CLI and `aplan mcp`. Choosing and
  authenticating the trusted file remains the caller's job, which `HLD.md` section 9 now states.
- **`aplan stale`** (`A1-020`), the drift question a tracker cannot ask: which live tasks have sat
  unchanged, and which of those could have started. FIELD-RESEARCH 7.3 found "how long has this sat"
  the highest-signal drift metric in the wild and recoverable from git without a database, and this
  reads history once -- one walk of `HEAD`, not one `git log` per task -- to report live tasks whose
  file has not changed inside the window (`--older-than`, or `stale_after_days`, default 30 days).
  The rows lead with the least-proxy case: a task whose blockers are all `done`, which can be started
  now and has not been. It **never fails a build** and exits `0` whatever it finds, because a task
  file's date is a proxy for the work's state; it reads git read-only, so it lives in the `verify`
  family and outside the hermetic `check`. Time is measured against `HEAD`'s commit rather than the
  clock, so the same commit gives the same report.
- **`aplan attest`** (`A1-040`), the end of the evidence ladder: the verdict `verify` reaches,
  written down and bound to the commit it was true of. Each `commit` citation is recorded as the
  full sha it resolved to and each `file` citation as its blob sha, so a reader re-derives both
  instead of trusting the document. A `commit` citation also says whether the attested commit can
  **reach** it: `verify` does not ask that, and a task closed on a branch nobody merged would
  otherwise read as work that is in this tree. The document says whether the tests were **run**
  (`--execute`) or the citations merely resolved to names a runner lists, because that difference is
  most of what a `test` citation is worth. It refuses a plan that does not verify, and a work tree
  that does not match the commit -- **including untracked files**, since a file git has never seen is
  not in the tree a reader checks out. There is no timestamp in it -- an attestation is a statement
  about a tree, not about a moment -- which is what makes two runs at one commit agree byte for byte.
  `--attach-note <ref>` attaches it to `HEAD` as a git note, tamper-evident because git hashes it;
  cryptographic signing is a second implementation of that same seam and is not built.
  **Who verified is now checkable** (`A1-154`): the `tool` object carries the version, the commit
  the build script saw (best effort; a release pipeline can set `APLAN_SOURCE_REV` itself), the
  SHA-256 of this
  build's own `facts` document -- its capability contract -- and the SHA-256 of the running
  executable. Two builds sharing a version cannot share the last two, which is the gap a pilot
  measured behind `aplan 0.3.0-dev`. Both digests reproduce with the tools you have:
  `sha256sum $(which aplan)` and `aplan facts | sha256sum`. The binary digest is self-reported, so
  it identifies rather than authenticates -- its worth is comparison against a digest you recorded
  when you pinned the binary -- and the crate's documentation states the rest of what stays outside
  the claim.
- **`aplan import --format spec-kit` and `--format openspec`** (`A1-039`), the two spec-driven task
  lists RESEARCH section 15 ranked first. Both documents carry their own ids, so nothing is
  invented: `T012` and `1.1` become the task ids and the source's prose still resolves against the
  plan; `--id-prefix` qualifies them when a second feature is imported. One parser and two names,
  because the formats differ by two tokens, and each flavor declines the other's document. What it
  refuses to guess is reported: spec-kit's `## Dependencies & Execution Order` is English about
  sequence (21 lines of it upstream) and becomes notices plus a pointer to `aplan dep add`, never
  edges; `[P]` and user stories become tags; file paths inside descriptions are left alone, because
  zero of the 34 items in the upstream template carry a resolvable one.
- **`aplan export-agents-md`** (`A1-038`), the agent contract a repository's own policy implies:
  where the plan lives, its id convention, the `ready`/`close --evidence` loop, the evidence
  `.aplan.yml` actually requires, the runners a `test` citation resolves against, and the MCP tools
  the build serves. It writes a **fenced block** rather than a file -- `--write` replaces what is
  between the markers and leaves your own prose alone -- and `--check` is the gate shape
  `gen-roadmap --check` has. It carries policy, never progress, so the block does not move when a
  task is closed. Written because two hand-written copies had already drifted: this repository's own
  said `commit`, `test` or `file` for three tasks after its policy narrowed to `test`, and the first
  pilot's tells agents the MCP surface is four tools when it is eight.
- **"Which of these are for you"** in [`docs/upgrading.md`](docs/upgrading.md) (`A1-149`, ADR-0024):
  one row per optional feature, written in the plan's terms rather than in benefits, whose
  right-hand column says who the feature is *not* for. A gate requires a row for every top-level
  config key and both halves answered in each. There is deliberately no command that recommends
  features: it was measured over five real plans first and gets 16 of 20 judgements right, telling
  the one pilot that sequences all seven of its cuts to skip cut sequencing.
- **`aplan check-docs <files>`** (`A1-148`), the gate every project with a plan document ends up
  writing: it exits 1 when a document names a task the plan does not have, with the file, the line
  and the ids a single edit away. Ids come from the plan rather than from a pattern -- a word must
  carry a prefix the plan's own ids use -- so it works on `A1-042`, `WH2-06`, `K0-3` and
  `M-phase-2` without this tool owning an id format. `--format json` for a gate that reports rather
  than fails; the diagnostic formats are refused, because a document naming a task is not a plan
  violating an invariant and carries no rule code. A plan whose ids teach no prefix (ULIDs, or
  `auth-login`) makes it refuse rather than report a clean run it cannot back. Measured over 43
  documents, 161 task bodies and four pilots' prose: 1,211 references, no false positives, and no
  findings in 575 committed document-revisions -- the drift it exists for is caught before a commit,
  by the audit this replaces. A reference ends at a word boundary as well as starting at one, and
  whether a dot continues an id is learned per prefix, so `T-999.md` names a task file while
  `M-v0.N` stays prose. Write example ids as `T-nnn`: a digit-shaped placeholder is a reference.
- **A duration is derived from effort, never declared** (`A1-147`, ADR-0023,
  `--require rollup.carries_effort`). Declare one number, `effort_days_per_week`, and `summary`
  reports the plan's weeks and the weeks remaining, while `roadmap` gives each cut its own `effort`
  and `weeks (n left)` in the human view, the JSON and the markdown table. No rule and no dates: a
  week here is a unit of capacity, not a position in a calendar, and nothing is stored, so nothing
  can drift from the effort beside it -- growing a cut moves its duration by the growth, unattended.
  A repository that declares nothing gets no durations, and still gets each cut's `effort_days`,
  which is new on every `roadmap` surface. The work in *no* cut carries its own effort and duration
  too, and the header line totals what the rows add up to, so the column reconciles against
  `summary` rather than silently omitting the plan's cross-cutting work. A throughput that is zero, negative or non-finite is
  refused at load rather than ignored.
- **A release is a milestone tagged `release`** (`A1-146`, ADR-0022,
  `--require milestone.release_holds_cuts`), a checkpoint over *cuts* rather than over work. Its
  `blocked_by` names the cuts it closes, `APLAN007` refuses to call it done while one is open,
  `ready` and `why` see it as they see any milestone, and `roadmap` rolls up the work of those cuts
  instead of reporting `0/0 done`. A task still belongs to exactly one cut, because a release claims
  none; a release holding work directly is reported by `APLAN021`.

  Measured before it was built: the encoding needs *no* model change -- `check`, `ready`, `why` and
  `APLAN007` already do the right thing with a milestone whose members are milestones. What did not
  work was telling it apart from a cut emptied of its work, which is the same graph (ADR-0013 reads
  a milestone naming a milestone as "follows"), so the tag is the author saying which they meant.

### Fixed

- **Evidence citations in generated guidance are spellable in a shell** (`A1-156`). An
  angle-bracket placeholder after `commit:` is not a metavariable in an unquoted shell word: `<` is
  redirection, so the line fails at parse
  time and does something other than what it says. The fifth pilot's audit parsed every command
  block rather than trusting the fences around them, and found the shape in `export-agents-md`'s
  fenced close example and in the remediation three importers emit -- a contributor copying the
  instruction literally was stopped before this tool could report a single plan finding.

  Now `commit:COMMIT_SHA`, `test:REGISTERED_TEST_NAME`, `file:FILE_PATH`, `url:URL`. Two gates keep
  it: one refuses `kind:<` in any string the tool can emit, and one hands every fenced line of the
  generated contract to `bash -n`, because whether something is shell is a question only a shell
  answers. **Regenerate `AGENTS.md`** if you keep one. Prose naming a *form* -- `aplan tree <id>`,
  where the reader is choosing the subject -- keeps its angle brackets.


- **Generated agent guidance no longer states a rule more strongly than the rule states itself**
  (`A1-157`). Three bullets in `aplan export-agents-md` said "every task" about rules that exempt a
  milestone -- `paths` (`APLAN023`), `effort_days` (`APLAN009`) and cut membership (`APLAN022`) --
  so a valid cut container read as noncompliant, and a literal agent would "repair" it with a
  meaningless estimate and evidence boundary. The fifth pilot found the first two and said it had
  not checked the rest; the third had it too.

  Each bullet names its own subject now rather than a blanket exemption at the end, because a
  blanket sentence is wrong the moment a bullet is added whose rule does apply to a milestone -- and
  that is this defect exactly. **The exemption is declared on the rule** rather than described in
  prose, and a gate reads it: any bullet citing a rule that declares it must say so. Nothing about
  milestone validation changed; a milestone still needs neither field.


- **`aplan dep add` works on an imported task** (`A1-033`'s audit). Replacing a top-level block
  skipped its continuation lines only when they were indented, and this tool's own importer writes
  an unindented list, so the old entries stayed behind the new block and the frontmatter no longer
  parsed. The round-trip check refused the write, so nothing was ever corrupted -- the command was
  simply unusable on every imported plan, which is exactly where re-sequencing is most likely to be
  wanted.

### Changed

- **`aplan check-docs` reports a reference whose *prefix* is the typo** (`A1-164`). It learns its
  vocabulary from the ids a plan carries, so a taught prefix with a number nobody used was caught
  while a mistyped *prefix* was silent -- and the second is the likelier mistake, since inventing a
  number takes more than a slip. The sixth pilot has eight cuts, `DN0-` through `DN7-`, which puts
  a ninth one keystroke from real and reading as tracked while naming nothing.

  A prefix differing from a taught one in exactly one position is reported, **where both characters
  are digits**: that is what keeps foreign vocabulary silent, since `AI-2027` differs in one
  position too and a letter is where another project's ids live. Every near prefix is named rather
  than the first, because a mistyped digit is equally close to each of this plan's two groups.


- **`aplan import --out` may point outside the plan** (`A1-163`). The id-collision check ran against
  the plan root whatever `--out` said, so importing a copy somewhere else to diff against the plan
  you already have was refused for a collision that was not one -- the ordinary shape of a pilot,
  and what `A1-160`'s upgrade note has to tell adopters to do. The check is scoped to a destination
  the plan will actually load; outside it, one `placement` notice says these tasks join no plan and
  names `--root`. Inside the plan the refusal is unchanged and now says which plan it means.
  `--cut` with an outside destination is refused, because the cut would edit a milestone to name
  tasks the plan cannot see. Containment follows symlinks as far as the destination exists, so a
  link pointing back into the plan is inside it.


- **`aplan import --format roadmap-json` keeps a phase's dependencies** (`A1-161`). That format puts
  `depends_on` on a phase as well as on a milestone, and the importer read only the milestone graph,
  dropping the rest under "is not held by the task model" -- untrue of a dependency edge, which is
  exactly what `blocked_by` is, and worth five real edges in the `p0000019` pilot's plan. They are
  validated on arrival now like every other edge, so a phase naming something absent is `APLAN003`
  on the first `check` rather than silence. A phase target is rewritten to the id this importer
  gives it and a milestone target keeps its own, resolved against the source's own phase list.
  `blockers` still reads as unholdable, because it is prose with its own anchors, and that message
  means something again now it is not also being said about an edge. An edge naming a *workstream*
  resolves to the cut task the import renames it to, rather than being reported as naming something
  unknown by the tool that renamed it.
- **`aplan import --format roadmap-json` reads `not_applicable` as a decision, not as work**
  (`A1-160`). A requirement that turned out not to apply arrived `todo`, which said three of the
  `p0000019` pilot's milestones were outstanding when its own linter had discharged them. It now
  arrives `dropped` -- aplan's word for "abandoned, kept for the record" -- with the source's word
  kept as a tag. Nothing is manufactured by that: this tool doubts a claim that work was *finished*,
  which is why a source's `done` still arrives `todo`, and a decision not to do something needs no
  proof.

  The two models differ in one place, and the import says so per task rather than leaving `check` to
  surprise you: that source counts `not_applicable` as satisfying a dependency, while here a
  `dropped` blocker stops its dependents. **That difference is deliberate and stays**: a requirement
  that no longer applies is not something to wait for, so the edge goes, and `APLAN020` already
  names that remedy. A status whose blockers were transparent would buy one saved edit and cost a
  plan where `blocked_by` no longer tells you what a task is waiting for.

  Its audit also gated `docs/upgrading.md`: the document you are reading while staging a binary
  names **59 `aplan ...` invocations** and nothing checked that any of them parse, where the
  README's command list and the tool's own messages both had gates. Two were already wrong -- an
  optional-argument bracket notation nobody can run, and a command wrapped across a line.


- **A value YAML retyped is reported as that, and quoting is named as the fix** (`A1-158`).
  `ref: 12345678` is a commit ref an author wrote and YAML read as an integer, and the schema
  refused it with `12345678 is not of type "string"` -- true, and no help to somebody looking at a
  line that plainly says a ref. About one abbreviated sha in forty-three is all digits, which is the
  worst frequency a trap can have: rare enough that nobody has met it, common enough that somebody
  will. The message now says YAML read it as a number and shows the quoted line to write.

  The suggested value is the text in *your* file, not what YAML made of it: `ref: 0x123456` reaches
  the validator as `1193046`, and offering that back would swap your ref for a different string that
  merely loads, and a trailing comment (`ref: 12345678 # the fix commit`) is not part of the value
  either. Where two lines could have produced the value, no value is suggested. Errors quoting
  would not fix -- a boolean where an integer was wanted -- keep the validator's own words.
  `aplan attest` also names each file that failed to load, rather than only counting them.

- **A mutation naming an id two tasks declare is refused** (`A1-135`). Every mutating command found
  its target with a linear search for the id, and nothing said that id was unique: on a plan
  declaring `A-2` twice, `dep add` edited one file and `drop` edited the other, neither said
  anything, and the plan afterwards held a `dropped` task and a live one sharing an id. The
  post-state check cannot catch it, because `APLAN002` is in the pre-state too and a mutation on an
  already-dirty plan is allowed by design.

  One predicate in `mutation::`, run by `close`, `amend`, `drop`, `mv`, `supersede`, `dep` and the
  agent surface, naming both files and writing nothing. Refused rather than disambiguated: a
  mutation that cannot know which task it is editing must not guess, and `APLAN002` already says
  where the duplicate is. One wording for "no task `X` in this plan" now, across the read commands
  too.

- **The generators say when the working tree holds work a clean checkout does not** (`A1-144`).
  `ROADMAP.status.md` is generated from the plan on disk, and a working tree holds files a clean
  checkout does not: a task written and not yet committed makes the rollup name work CI cannot see,
  so `gen-roadmap --check` fails there and nowhere else. Measured twice in one session, both red
  runs on `main`, and confusing in the way that costs most -- the gate says "regenerate and commit",
  the author does, and the regenerated file is identical to the committed one.

  `cargo xtask gen-roadmap` and `cargo xtask check-docs` now name the plan files that differ from
  what is committed -- untracked *and* deleted-but-committed, which is the same failure in the other
  direction -- and say what they mean for the gate. Files merely *modified* are not listed, because
  that is the normal state when a rollup is regenerated and a note on it would fire on every run. A note, never a refusal: writing a task and regenerating before
  committing is the normal order. This is a producer's hazard rather than an adopter's -- it needs
  two writers in one tree -- and it is this repository's own workflow.

- **A `--cut` may be named by its id or by its label, everywhere** (`A1-139`,
  `--require cut.named_by_id_or_label`). `graph --cut` accepted both; every command that *writes*
  took the id alone, so `aplan add --cut s1` answered "no task with id `s1` in the plan" about an
  id the reader never typed -- while `roadmap`, the surface they read first, prints exactly that
  label.

  One resolver now, in `aplan-interop::cut`, used by `add`, `mv`, `supersede`, `import`,
  `drop --cut`, `aplan_add` and `graph`: id first (unique by `APLAN002`, so a label can never shadow a cut), then label. A
  name nobody answers to is refused with the cuts the plan has, and a label two cuts share is
  refused as ambiguous rather than resolved by file order.

- **`APLAN021` counts a cut's members as live work** (`A1-134`, ADR-0013's consequence,
  `--require milestone.members_are_live_work`). It asked whether `blocked_by` was *empty*, and a
  sequenced cut keeps its predecessor there, so a cut emptied of all its work still had an entry and
  no rule saw it. `A1-081` also exempts a cut's own cancelled member from `APLAN020` on purpose, so
  a cut whose work was all dropped was silent for a second reason. Both are a cut with nothing in
  it, which is what this rule is named for, and the message says which of the two it is.

  `aplan mv` had hand-rolled the check, since the rule could not express it; that refusal is gone.
  Emptying a cut is now reported by the rule at the repository's own policy -- a note under the
  mutation gate, a refusal under `deny_warnings` -- which is the single answer to "is this legal"
  that `A1-113` exists to keep.

### Added

- **`APLAN035`: a cited `commit` that is a merge** (`A1-145`), a warning. A merge combines whatever
  both sides touched, so a path check against it passes for almost any `paths` -- which is how the
  first pilot closed a task on the merge of an unrelated pull request and had `verify` report 53
  tasks clean (pre-public adoption evidence, item 11). `git rev-parse HEAD` at the wrong moment is all
  it takes, and 19% of that repository's commits are merges.

  A warning rather than an error, because a repository that merges every feature with `--no-ff` has
  a merge that is the honest citation; the message says what it saw (a commit with N parents, whose
  diff covers everything both sides touched) rather than concluding the citation is wrong. Under
  `deny_warnings` it does block, and `close` refuses a citation `verify` would warn about, so a
  repository with that policy can no longer close a task on a merge without saying why.

  Measured over four plans, 177 commit citations, none of them a merge -- and the honest number is
  the smaller one, since only two of those repositories have merges at all, which makes 26 citations
  the evidence that discriminates. It was also checked against the repository that produced the
  finding: silent on p0000004 today, firing on the citation that plan carried before it was repaired
  by hand.

  Two neighbouring signals were measured and **refused, with the numbers**, in the same task: a
  citation older than the task itself would fire on 39 of 155 closed tasks across three plans and
  be right in none of them (it is what adoption looks like -- a plan authored after the work), and
  `paths` breadth measured against the file tree cannot separate the case (22% of files) from the
  legitimately broad tasks (46-50%). What the measurement found instead -- breadth measured against
  *commits*, where the same case sits at 47% against 1-5% for narrow tasks -- is filed as `A1-151`
  with its own thresholds to settle.

- **`APLAN034`: a `traces` target that leaves the repository has to be declared** (`A1-143`), with
  `traces_external` as the declaration. A trace is a filesystem read against `traces_root`, so
  `../p0000007/docs/STATUS.md#phase-1` resolved whenever that sibling happened to be checked out --
  and resolved *fully*: a bad anchor in it was refused and `detect_drift` fired across the boundary.
  The fifth pilot verified all three (pre-public adoption evidence, item 4c).

  That made a repository's `check` an undeclared hard dependency on another repository being
  present at an exact relative path and ref: green on the machine with the sibling, `APLAN008` on
  the job without it, and drift warnings for changes the repository did not make. The same rule
  stops a task file naming `/etc/passwd`, which matters because a task file can arrive in a fork's
  pull request -- the argument `A1-041` made when it refused every url scheme but `https`.

  Declaring a root in `traces_external` makes the dependency the repository's stated position, and
  its absence then stops the run as a **configuration error** (exit `2`) naming the key, rather
  than as a finding about tasks that are fine.

- **`prefer_after`, a discretionary order, and `APLAN033` to keep it honest** (`A1-150`, ADR-0025,
  `--require ready.honours_a_declared_preference`). A task may name tasks it prefers to follow, and
  `ready` offers it after them when both are offerable. It **never blocks**: a task whose
  preference is unmet is still ready and still offered, and `critical-path`, `why` and `APLAN007`
  do not read the field at all, because a reader asking "why can I not start this" must never be
  shown an opinion.

  ADR-0017 deferred this field rather than rejecting it, and said what would justify building it: a
  recorded instance, during the dogfooding period, of wanting "X before Y" that neither a dependency
  nor the derived order could express. A pass over six pilot logs produced one -- ten open findings
  with a maintainer's order over them, none of which is a dependency and none of which the derived
  rank reproduces, since they are unrelated leaves of comparable effort. ADR-0025 carries that
  instance and supersedes ADR-0017.

  `APLAN033` (error) refuses a preference nobody could act on: an unknown id, the task itself, a
  pair that is already a `blocked_by` edge -- the plan would then say two different things about one
  pair -- or a cycle. `aplan dep add <id> --after <other> --prefer` writes one, `--prefer` on `dep
  rm` removes it, and `ready --explain` names the preference beside the derived reasons so a
  judgement is never mistaken for a derivation.

- **Every configured command is bounded** (`A1-121`). `test.list`, `test.run` and `url.check` name
  programs this tool spawns, and none of them could time out: `Command::output()` waits for the
  child however long that is. Measured while auditing `A1-041`, a `url.check` of
  `["sh", "-c", "sleep 60", "{url}"]` made `verify` wait the full minute, and a command that never
  returns waited forever. In CI that is a job that hangs rather than fails, which is worse than
  either, because nothing reports it and the runner is held until the job's own timeout.

  The defaults are per key, with their reasons: **600 seconds** for `test.list`, because
  `cargo test --list` compiles and a cold cache is legitimately minutes, and **30 seconds** for
  `url.check`, because that is one network request and the failure it meets is the silent one --
  a black-holed packet does not refuse, it goes quiet. Both are overridable
  (`test.timeout_seconds`, `url.timeout_seconds`; `0` means no limit). `test.run` under
  `--execute` is deliberately unbounded: a long test run is the point rather than a symptom.

  A timeout is its own outcome, not a failed citation: `url.check` reports the url as *not
  checked*, keeping the distinction `A1-041` drew for the non-https case, and `test.list` takes the
  same route a runner that could not be asked takes (`A1-066`). The command is killed with the
  descendants it started, measured on a `url.check` that forks, which used to leave one process per
  timed-out citation on the machine.

- **`APLAN032`: a `paths` entry has to be a glob this tool can compile** (`A1-066`), and, more to
  the point, **one task's defect no longer stops the run**
  (`--require verify.one_tasks_defect_does_not_stop_the_run`).

  `paths: ['src/[']` used to abort `verify` before it reported anything: one task's typo hid
  `APLAN005` -- the rule this project is named for -- in a *different* task, and it exited `2`,
  which ADR-0007 reserves for the invocation or `.aplan.yml` and which sent the reader to neither
  of the two places the defect was. It also broke the promise `A1-058` made and the README repeats:
  that `verify` is a superset of `check` and the two never disagree. They disagreed here.

  The rule is hermetic, which is the repair rather than a detail: a glob's validity needs no
  repository, so both commands now give the same answer, name the entry, quote the compiler's own
  words, and judge the rest of the plan. It is an error rather than folded into `APLAN019`'s
  warning because the reader's question differs -- "this glob cannot be used" is not "this glob
  matched nothing" -- and so does the consequence: every path-based rule is silent about that task
  until it is fixed.

  A configured command that will not run had the same shape and now has the same answer: a test
  runner that cannot list is one `APLAN013` error naming the runner, a `test.run` that cannot
  launch under `--execute` is a runner fault rather than a failing test, and a `url.check` naming
  a program that is not installed is one `APLAN016` warning. Each is attached to no task, because
  no task is at fault, and located on `.aplan.yml`; the citations that depended on them are left
  unresolved rather than blamed for them. `A1-120` made the first of these an abort so a broken
  runner could not quietly invalidate its citations; the loudness stays, the stopping goes. A
  repository that cannot be opened still ends the run, which is the one place stopping is right.

- **`id_strategy: sequential | ulid`** (`A1-047`), the escape hatch HLD Section 4 promised and
  `A1-029` deliberately did not build. `sequential` is the default and is exactly what every plan
  already gets: `aplan add` continues the convention the plan uses. `ulid` replaces the counter with
  48 bits of timestamp and 80 random bits, so two authors working offline cannot derive the same id
  and discover it at the merge -- the failure `APLAN002` has so far only been able to *report*, at
  the point where the repair is a manual renumber.

  The trade is stated where the choice is made, in `README.md` and in the key's own documentation,
  because it is real: `A1-047` becomes `A1-01K9ZBW7YQ8N4TQGX2H6RC3PVD` in every `blocked_by`, every
  `why` and every filename. The plan's own prefix is kept -- it is the readable half, and nothing
  about it collides -- and a plan holding ULIDs is not read as a sequence to continue, so a
  repository can set the key, unset it, and keep its convention either way. `--id` still overrides
  both, `APLAN002` still owns identity, and both surfaces mint through one function so the CLI and
  `aplan_add` cannot disagree.

- **The behaviour-naming rule is an audit step, not a gate** (`A1-142`). A change that alters what an
  existing rule, command or tool *does* earns a `Behaviour`, and seven of the nine such changes in
  this repository's history shipped without one -- the gap the fourth pilot has reported three times.

  Both diff-shaped gates were measured before either was written. A gate on the rule catalogue would
  have caught three of the nine while firing on twelve wording changes (25% precision); one on the
  scenario corpus, four of nine at 19%. A gate wrong three times in four teaches an author to type
  its escape hatch, which is the dismissal-training this project refuses elsewhere, so neither was
  built. What has caught it, four times out of four, is a person asking the question during an audit
  -- so `AGENTS.md` gains it as an audit step, and the four points where a behaviour moves carry the
  reminder where the author is already typing. A test keeps all five in place, because prose is
  worth what it is worth only while it is there.

- **Two behaviours a build can now assert**: `ready.reports_a_closeable_cut` (`A1-105`, `ready` names
  a cut whose members are all done rather than answering with silence) and
  `mutation.refused_when_check_would_reject` (`A1-113`, a command or tool that writes runs the rules
  against the plan it would produce). Both shipped unnamed and were found by the audit of `A1-142`
  itself, which had counted seven instances and missed these two -- both on the *command* surface,
  the one surface its first pass of reminders had skipped.

- **`APLAN019` ignores a dead glob beside a bound one** (`A1-141`). `A1-138` scoped the rule to
  where its stated consequence can occur, and the consequence is that the task's *evidence* binds to
  nothing. A dead glob beside a live one the citation binds through does not do that: nothing is
  ever bound through an empty glob, so asking the question per glob reports a harm that is not
  there. It is a question about the `paths` as a set, which is how `APLAN031` already reads them --
  so this also settles a disagreement between two rules about what a multi-glob `paths` means.

  The fourth pilot met it where it costs most: `close` refuses what `verify` would fail, so under
  `deny_warnings` a *bound* citation could not close a task whose `paths` named a directory the task
  had not created yet. It filed the question against its own interest, with both readings argued and
  the numbers -- 43 tasks where the rule is unambiguously right, 28 of this shape, including all
  twelve of the cut it was starting (pre-public adoption evidence item 26).

  Unchanged: a task whose `paths` bind nothing it cites, a task citing only a `test` (bound by
  runner coverage rather than by a path match), and `A1-022`'s reading that a live task's globs are
  judged against what the repository tracks now. Assertable as
  `--require paths.unmatched_ignores_a_bound_sibling`, added beside the previous name rather than
  replacing it, so a consumer that pinned that one is not refused by a build that has both.

- **`APLAN031`: a cited file must be inside the task's `paths`** (`A1-045`). `APLAN012` makes a
  `commit` citation mean "a commit that touched this work" by checking its diff against `paths`.
  `APLAN015` had no equivalent: it asked whether the file exists at the cited rev and stopped, so
  `file: README.md` resolved for a task about the renderer and proved the repository has a README.
  `require_min_strength` counts `file` among the strong kinds, so the same reads-as-a-guarantee
  problem applied.

  Two consumers found it independently. The first filed it from reading the rules; the fourth
  reproduced it in one command on a real plan -- closing a task about ADR authorship with a
  Containerfile, `check` and `verify` both `ok` afterwards -- and noted the sharper half: its
  conventions tell authors to prefer `file:` over `commit:` *because* `APLAN024` is right that a
  116-file commit names a Tuesday. A repository that took the earlier advice was more exposed, not
  less (pre-public adoption evidence item 25).

  An **error**, matching `APLAN012` rather than `APLAN023`'s warning: that rule is a warning about a
  claim nothing can check, and this is a claim that was checked and does not hold. A task with no
  `paths` stays `APLAN023`'s business, a file that does not exist stays `APLAN015`'s, and a
  catch-all `paths` fails relevance rather than satisfying it, exactly as `APLAN012` reads one.

  It found five on this repository's own plan the moment it ran, each a `file:` naming the *test
  file* that proves the task -- written before a `test:` citation could resolve at all (`A1-120`),
  and redundant since `A1-123` gave each of them the test itself.

- **`aplan amend --remove-evidence`** (`A1-137`). The third shape of repair, after proof that
  *rotted* and proof that was never *whole*: proof that was never proof. `A1-123` wrote a citation
  that would not have failed if the task's work were reverted, noticed, and deleted it in a text
  editor because nothing here could; `A1-045` then found five more in one run. The objection was
  that removal is how a closed task quietly loses its proof, and `A1-113`'s post-state check answers
  it: a removal that leaves the task failing `APLAN005` or `APLAN017` is refused like any other bad
  mutation, so an author cannot repair a task below the bar its repository set.

- **The traces that predate row anchors were migrated** (`A1-122`). ADR-0015 made a table row an
  anchor, because a register kept as a table has one heading and N independent facts, so editing
  row 11 re-flags the task that owns row 7. The mechanism shipped with `A1-095` and *nothing moved
  to it*: nine traced sections of more than three rows, twelve tasks, including two 32-row tables
  where a task owns a single row. Not theoretical -- `A1-008` and `A1-009` were re-stamped three
  times in one session, and twice more in the session that fixed it, every time because an
  unrelated row changed.

  Six migrated (`aplan close`, `aplan init`, `APLAN016`, the drift-gates row, the GitHub Action and
  GitHub Releases rows), each re-stamped through `aplan close --restamp` rather than hand-edited,
  which walks `A1-098`'s migration path for the first time in the repository that built it: it
  reported every stale stamp by name, with the remedy. Three sections kept their heading and say
  why -- a task that built the evidence ladder owns all of its rungs, and a pilot log's entry is the
  unit of that document.

  The CLI table's rows had to be keyed on something stable first, which is the criterion doing its
  job before the migration rather than after: the first cell was the *signature*, and `A1-127` had
  changed one two commits earlier, so a row-scoped trace written today would have dangled tomorrow.
  The column is the command name now.

  **Appending a section no longer re-flags the one before it.** A section's fingerprint ran to the
  next heading, so the blank line separating them joined the text: a document that only *grew* moved
  the previously-last section's hash and re-stamped whoever traced it. It cost this repository a
  re-stamp of `A1-138` the day after it closed, on a pilot log that gains a pass every session --
  drift training a reader to dismiss drift, which is item 14's family. Trailing blanks are trimmed
  before hashing now, and a stamp written by an earlier build is still accepted, so no plan is
  re-flagged wholesale by the upgrade and the form converges as tasks are re-reviewed anyway.

  **No rule**, decided on the measurement: a heuristic flagging "a trace whose section is a table"
  would have been wrong for a third of these, in the direction that teaches people to dismiss the
  warning -- the defect ADR-0015 exists to remove. `AGENTS.md` carries the convention instead, where
  an author meets it before writing a trace.

- **`aplan show <id>`, and `aplan_show`** (`A1-118`). Nothing answered "show me this task". `why`
  gives blockers and nothing else, `tasks --format json` omits `paths`, `traces` and `evidence`
  entirely, and `cat` gives the file. So the two fields carrying this tool's whole argument -- the
  evidence that closed a task and the trace that says what caused it -- were reachable only by
  opening the file by hand, which is the fourth pilot's complaint about the write surface arriving
  on the read one.

  What it adds to `cat` is **resolution**, and that is the case worth building for: a closed task
  with a rotted citation and a drifted trace reads as perfectly healthy in its own file. `show`
  carries the cut and whether the task is ready (from the graph), what it waits on *and what it
  releases* (the graph in both directions, where the file has one), and every finding the rules and
  the evidence ladder attach to that one id.

  It resolves by default and takes `--no-resolve`, which is `close`'s convention rather than a
  second one (`A1-109`), and a plan kept outside a repository reads as unresolved rather than
  refusing to answer -- also `close`'s answer. `--body` includes the prose, off by default because
  it is long. `--format json` carries the same content, field for field: the projection is one
  struct in `aplan-core`, serialised by both surfaces, and the parity audit compares the whole
  document rather than a field somebody remembered.

  **MCP gains it**, which `A1-104` forces to be an explicit decision: an agent had `ready`, `why`,
  `check`, `verify`, `add`, `dep` and `close`, and no way to see the `paths` its commit citation
  will be judged against -- the one thing `close` refuses it for. The surface is eight tools now.

  Two things the audit fixed. The ladder ran over the **whole plan** to answer about one task, which
  cost a full `verify` -- 1.77s here, scaling with every *other* task's citations; it is scoped to
  the task now (0.03s for one that cites nothing), sound because every rung asks about this task's
  citations while the rules that need the other tasks are hermetic and still run over all of them.
  Both answers were compared across all 149 tasks before the change was kept. And `--format sarif`
  printed the human form: a flag that takes a value and ignores it. `show` renders one task, so it
  has `human` and `json`, and says so when asked for a report format.

- **`add --traces`, on both surfaces** (`A1-116`). `add` accepted eight fields and not the one that
  binds a task to its cause. 67 of this plan's 125 tasks carry `traces:`, `APLAN008` checks those
  anchors, and `close` stamps a fingerprint per trace so `APLAN018` and `APLAN028` can see drift --
  so a trace added by hand afterwards is a drift detector that was off in between. Every
  findings-derived task was created and then hand-edited, in a repository whose premise is that the
  tool owns those files.

  The agent case is sharper: `A1-103` gave an agent `add` so it could create the work it is told to
  create, and an agent asked to file a task for a finding could create the task and not record the
  finding. Its only route was to hand-edit YAML, which is the anti-pattern this project names,
  reached from the surface `A1-104` exists to keep honest.

  An anchor that does not resolve is refused at creation, and the refusal is `APLAN008` itself with
  its own remedy, because `add` already runs the rules on the plan it would produce (`A1-113`) --
  no new check, no copy of one. **A trace names the cause of the task, which exists, not the
  document the task will write**: that is the awkward case the task asked to have decided rather
  than inherited, and it is what the help says.

  This removes one obstacle to `add` being the authoring route. The body is still a stub someone has
  to write, so the claim stops there.

  Two things the audit found beside it. **A repeatable input given twice was written twice**, on
  every field `add` collects: nothing rejects a duplicate and each one costs something quietly -- a
  repeated tag is what `APLAN026` had to be taught not to double-count, a repeated glob is judged
  twice by `APLAN012` and `APLAN019`, and a repeated trace stamps one fingerprint under two entries,
  so `APLAN028` compares a set against a list. `drop` and `mv` already treated an id named twice as
  one; `add` does now, on both surfaces through one helper, keeping the author's order. And
  `APLAN008` answered `DOC.md#` -- a trace whose author meant to name a section and stopped -- with
  "add a heading whose text slugifies to ``", sending them to fix the document rather than the
  trace. It names the empty anchor now, and offers `DOC.md` on its own for the whole document.

- **`add --cut` takes the phase from the cut** (`A1-108`). Adding to cut 1 of a plan whose highest
  phase is 6 produced `R6-09`, `phase: 6`, a member of `M-s1`. Nothing rejects that -- the phase
  field and the release cut are different axes on purpose (ROADMAP section 10) -- and it still
  misleads: `summary` bills the effort to phase 6, so a per-cut rollup disagrees with where the work
  was placed, and a plan whose ids encode the cut gets an id that misstates it, on the first step the
  fourth pilot's own instructions document (pre-public adoption evidence item 22).

  The defect is smaller than "the task contradicts itself" and harder to argue with: `add` had the
  placement in its hand and guessed instead. `--phase` defaults to the highest phase already in use,
  which is the best guess available when nothing better is known, and `--cut` is something better.
  An explicit `--phase` still wins, and `add` with no `--cut` is unchanged.

  Taken from the milestone rather than from its members, because a cut whose members span phases is
  legal and deriving from them would mean picking one. Both surfaces changed together, which
  `A1-104`'s audit is there to insist on, and the pilot had already noted that they were identically
  wrong -- parity working, faithfully reproducing the CLI.

- **`APLAN019` fires only where something is bound to the paths** (`A1-138`). The rule reports a
  `paths` glob matching no tracked file and explains itself with "so every rule that judges evidence
  against it has nothing to bind" -- and for a task citing no `commit`, `file` or `test`, that clause
  is vacuously true: it reported a harm that was not yet possible. Planning past the current tree is
  what a plan *is*, and a task that builds `crates/foo` declares `crates/foo/**` before it exists.

  The fourth pilot met it at scale and **declined an upgrade over it for the first time in eleven
  passes**: 140 findings across 71 `todo` tasks, under `deny_warnings`, on a plan written six cuts
  deep before any product code exists (pre-public adoption evidence item 24). Both documented ways out
  were worse than declining -- `advisory: [APLAN019]` is a warn-only tier its constitution forbids,
  and deleting the globs would surrender the falsifiability `APLAN012` rests on.

  The scope is taken from the rule's own justification rather than from task status, which is the
  correction the pilot filed the same day: it covers `done` without naming it, silences this
  repository's own `A1-040` for the same reason it silences p00020's 71, and keeps every case the
  rule exists for. `close` already applied exactly this scope at the moment evidence appears, so the
  rule and the close gate now agree about when a glob matters instead of disagreeing.

  The message says what is unbound now that it only fires where something is: "so the commit this
  task cites binds to nothing through it". A `note` or a `url` is judged against nothing and does not
  make a glob reportable, which is one step further than the pilot's wording and follows from the
  same sentence.

- **A code task closes on a test, not on a commit** (`A1-123`). This repository set
  `require_min_strength: [commit, test, file]`, which accepts the weakest of the three on its own:
  `APLAN012` binds a cited commit to a task by *path overlap*, so a commit that happens to touch the
  right directory is proof. Measured on this plan: **53 of 108 closed tasks cited no test, 24 of them
  owning code** -- and the tests existed in every case. The plan simply did not point at them, which
  is this tool's thesis turned on itself.

  `.aplan.yml` now sets `require_min_strength: [test]`. The one exemption is the `untestable` tag,
  and it is a claim with a stated meaning: *no test would fail if this task's work were reverted*.
  Fourteen tasks carry it -- prose, ADRs, pilot logs, CI scripts this workspace cannot execute -- and
  each says which case it is in its body, enforced by `crates/aplan/tests/policy.rs` rather than by
  review. `milestone` is exempt for a different reason: a cut's `done` is proved by its members
  (ADR-0008), so asking a checkpoint for a test is a category error.

  All 24 code tasks were backfilled with `aplan amend --add-evidence`, each citation chosen by
  reading that task's own commits for the tests they added and keeping the one whose failure follows
  from reverting the work. Two were left uncited on purpose and say so: `A1-016` shipped a gate whose
  proof is that CI runs it, and `A1-120` shipped configuration -- its earlier backfill wrote a
  citation and then removed it, which is the finding that made this rule worth having.

  **Three defects the rule found on its first run.** `aplan-e2e` had no runner at all, so the crate
  holding the scenario corpus could not be cited by anything: a task with a test was refused for
  naming one "no runner lists". `APLAN025` was vacuous because no runner declared `covers`;
  declaring it produced 10 findings, all of one shape (an integration test proving a library crate),
  which two `covers_by_test` entries answer -- and the 2 that remained were real, `A1-074` and
  `A1-075` citing `aplan-core` tests while their `paths` named only `aplan-output`. Their commits
  touched `summary.rs` and `export.rs`; the `paths` were incomplete and are now right.

  Auditing the backfill against its own criterion found four citations that were true of the task's
  *neighbourhood* and would not have failed if the task's own work were reverted: `A1-008` cited the
  trace stamp rather than the closure, `A1-013` the milestone passthrough rather than the longest
  chain, `A1-019` the emitter's file shape rather than an import, and `A1-106` a parity test about a
  plan that fails verification rather than one that fails to load. All four were re-chosen. The
  locality claim was narrowed too: `covers_by_test: ['*']` on the two integration crates said
  something false about their unit tests, and a runner declaring no `covers` at all is the documented
  way to say "this makes no locality claim" -- which leaves `APLAN025` live for all seven library
  crates and silent where silence is honest.

  `aplan amend --remove-evidence` is decided *for* and filed as `A1-137`: the objection is that
  removal is how a closed task quietly loses proof, and `A1-113`'s post-state check answers it, since
  a removal that leaves a task below the minimum is refused like any other bad mutation.

- **`aplan mv` takes a set** (`A1-127`). `drop` already took several ids and a whole cut; `mv` took
  exactly one task, so moving nine tasks into an earlier cut -- which is what re-prioritising a group
  means -- was nine invocations and nine chances to stop half way. The fourth pilot measured the
  shape: per-task commands are a footnote at 33 tasks and "the dominant cost after authoring" at 105
  (pre-public adoption evidence item 6).

  All-or-nothing across the whole set, which is the decision with teeth here. Every refusal runs and
  every write is computed before any file is touched, so a member that cannot move refuses the move
  rather than leaving the ones before it in the new cut: that is the property `A1-030` fixed and
  `A1-084` called `mv`'s reason for being trustworthy, and a bulk form that applied what it could
  would re-create the half-written state the single form avoided, at nine times the size.

  Two things the set changes rather than repeats. The refusal that stops a cut being emptied of its
  work counts **across the set**, so moving two of three members is fine and moving all three is
  refused; asking per task would get both of those wrong. And a task named twice is moved once, so a
  repeat cannot make the command believe every member is leaving. It **names what it moved**, never
  a count, because the set is exactly what the author is unsure of.

  The audit found the write phase, not the refusals. Every file a move will write is now proved
  writable **before any of them is**, the probe `supersede` already used (`A1-130`), because `mv`
  writes the destination first and then each source: with one source read-only it wrote the
  destination, failed, and left the task in **two cuts** while `check` still said `ok`, since
  `APLAN022` is off unless a repository asks for it. `drop --cut` had the same shape -- a file per
  member -- and now probes too, so an unwritable member cannot leave half a cut cancelled. If a
  write still fails after the probe, the message names the state the plan is in rather than only the
  file, because nothing else would say so. `CutError`'s io failure also printed its reason twice,
  once in the message and once as the error's source.

  The single-task form is unchanged, since it is a shipped contract. `dep add` is not given a bulk
  form: "everything in this set comes after that" is a coherent operation with no measured demand
  behind it, and this project builds for the demand it has measured. Neither surface question
  arises, since `mv` has no MCP tool to keep in step (`A1-104`).

- **`aplan tree <id>`, and `graph --cut`/`--tag`** (`A1-126`). `graph` took no argument but `--root`,
  so on a 105-task plan the fourth pilot got 285 lines and 12.6 KB of Mermaid: "a graph that size
  does not render usefully anywhere, so on a plan of this shape the command is effectively
  unavailable" (pre-public adoption evidence item 13). That finding dispositioned *itself* as not worth
  fixing alone, and `A1-125` changed the arithmetic: an order that is **derived** has to be
  inspectable, or it is an assertion the reader takes on trust, which is the one thing this project
  asks nobody to do.

  Two commands rather than one with a mode flag, because they answer different questions with
  different arguments in different formats. `tree <id>` walks both directions with `--depth N`:
  upwards is why a task is not offered yet, downwards is what finishing it releases, which is the
  rank's own first input, so the view is exactly the two halves of the order's claim. It shows the
  rank for a task `ready` would offer and not for a blocked one, since the rank is an order over
  *offered* work; it labels a cut as `(the cut it is in)`, because ADR-0008 makes membership an edge
  and `A1-125` excludes those, and unlabelled a reader would count one and conclude the rank had
  forgotten it; and it marks a task already shown rather than walking a cycle forever, since a
  cyclic plan is something this view exists to let you *see*.

  `graph` gains `--cut` (by milestone id or by the label the cut reports itself under) and `--tag`
  (repeatable), which **intersect**: "the output tasks in this cut" is one question. An edge is
  drawn only when both ends are in view, and what left is counted in Mermaid's own comment syntax,
  so the diagram a reader pastes is still a diagram: a dangling arrow to an invisible node would say
  "something outside blocks this" without saying what. A view with no edges says so too, because
  boxes and no arrows is itself a claim. A filter matching nothing is refused rather than answered
  with an empty diagram, which renders as a blank rectangle and reads as "this plan is empty".

  Not on the MCP surface. An agent already has `why` for blocking and, since `A1-125`, `releases`
  and `critical` on every ready task, which is the same evidence in the shape an agent consumes;
  `tree` is a picture for a person, and the tool surface is small on purpose.

  Three things the audit fixed. The walk **inverted the graph per node**, so a 4000-task chain took
  7.7s: the same quadratic shape `A1-125` was audited for, one commit later, and the index is shared
  now rather than noted. It was also **recursive**, and aborted the process with a stack overflow on
  a 3000-deep chain, which is not a hypothetical shape since a strictly sequenced plan *is* a chain;
  a view command that kills the process on a large plan is worse than the diagram it replaces. And
  `--depth` counted the children it stopped at rather than what it was hiding, so a cut that
  concealed 3998 tasks reported "1 more task": it now counts everything below the cut that the view
  did not print elsewhere, which is neither the children nor a closure that double-counts a sibling
  three lines down.

- **`ready` answers in a derived order, and `--explain` says why** (`A1-125`). It answered in id
  order, which is not an order: on this plan `A1-020` outranked `A1-113` because twenty sorts before
  a hundred and thirteen. A list presented as an answer, in an order that means nothing, teaches a
  reader that the order means nothing, and then a real order is not believed either.

  The rank is derived from what the plan already records (ADR-0017), so it carries no claim the tool
  could not check: **what a task releases** (unfinished tasks that become workable when it is done),
  then **the critical path**, then **effort, largest first**, then id as a stable tiebreak. The
  precedence lives on `Rank::key`, with the argument for each level and the one rejected alternative
  (quick wins first, which optimises the count of finished tasks rather than what "do this next"
  asks).

  **Two of the four inputs ADR-0017 named were measured and dropped, rather than kept for
  appearance.** Counting cut membership as unblocking power gave 27 of 30 ready tasks the same score,
  because ADR-0008 makes membership an edge; counting only real dependents leaves one task scoring 1
  and the rest 0, which is the truth about this plan. And *cut sequence* cannot discriminate between
  two ready tasks at all: where cuts are sequenced, a later cut's members inherit the earlier
  milestone as a blocker and are not ready, and when that cut closes its own members are done. On
  this repository the input is empty twice over, since sequencing `M-v0.8` after `M-v0.7` is refused
  with a cycle and sixty `APLAN007`s. Both facts are pinned by tests rather than left as prose.

  Assertable by name: `--require ready.order_is_derived`, because `--require ready` passes on a
  build that sorts alphabetically and a consumer reading the first line as "what to do next" cannot
  tell those apart. Two more things the audit found and fixed: a *zero-day* critical path is no
  longer treated as a spine, since a plan that estimates nothing would otherwise have one arbitrary
  task marked critical, which is the arbitrary-answer-dressed-as-a-real-one this change exists to
  remove; and the first version of the ranking made `ready` quadratic, taking a 2000-task plan from
  46ms to 765ms, because it asked what each ready task released by scanning the whole plan. The
  graph is inverted once instead, which puts it back to 49ms, and a test fails at the shape rather
  than at the timing.

  `ready` is consumer-visible, so `docs/upgrading.md` says what moved. `tasks --format json` carries
  `rank` (1 first, `null` for anything not ready) because `A1-075` made it the surface a rollup is
  built on, and "the next five things" must not be a second implementation of the derivation. The
  MCP tool returns the same order with `releases` and `critical` beside each task, so an agent can
  interrogate the order instead of trusting it, and a **closeable cut** now says how many tasks
  closing it would free: that is the number an agent needs at the boundary where `ready` is empty,
  and `0` there would have re-created the silence `A1-105` fixed.

- **`aplan dep add <id> --after <blocker>`, and `dep rm`** (`A1-124`). `blocked_by` is the only
  per-task ordering the model has, and it was written by `add` and never again: checked across every
  write path, `add` set it at creation, `mv` edited a *milestone's* membership, and nothing edited a
  task's. So the one mechanism this tool uses to say "this comes first" was maintainable only in a
  text editor, in a repository whose premise is that the tool owns those files. That is the
  unfinished half of the fourth pilot's item 6, whose other half `A1-077` closed with `drop` and
  `mv`, and it is what made re-sequencing a plan of 133 tasks a frontmatter edit and a hope that
  `check` caught the mistake afterwards.

  The shape is the ordering rather than the field: `dep add`/`dep rm` name what they maintain, where
  a general `aplan edit --set blocked_by=...` would have invited editing any field through a
  stringly-typed interface. Repeating an edge that exists, or removing one that does not, changes
  nothing and says so rather than rewriting the file.

  One refusal is *not* a rule, and the audit is what found it: a milestone's `blocked_by` is which
  tasks are in that cut, so editing one end of it here was `mv` with both guards removed. Measured
  rather than argued: `dep rm` on the last work member of a **sequenced** cut left the predecessor in
  `blocked_by`, so `APLAN021` saw a non-empty list and `check` printed `ok`; and `dep add` put a task
  in two cuts, which only `APLAN022` catches and only when `require_cut_membership` is set. Both are
  refused and point at `aplan mv`. Sequencing one cut after another is the same field and stays
  allowed, because it is what `mv` sends you here to do -- its refusal now names `aplan dep` instead
  of telling you to edit `blocked_by` by hand, which was advice this task exists to retire.

  Every other refusal is `A1-113`'s post-state check rather than a fourth private copy of the
  pre-checks that task deleted: a cycle is `APLAN004`, an unknown id `APLAN003`, a dropped blocker `APLAN020`,
  and a `done` task acquiring an unfinished one `APLAN007`, each named with its remedy and nothing
  written. That is why this task waited for that one. A task cannot come after *itself*, which is
  refused by name rather than left to `APLAN006`, because the message an author wants is about what
  they just typed. Both surfaces gained it together as `aplan_dep`, and `A1-104`'s parity audit
  failed once in between, naming the unclassified tool; the audit found the tool shipped without the
  command's guard against editing a plan that did not load, which `mutation::clone_of` documents as
  something every mutating command refuses before reaching it.

- **Every mutating command is judged by the rules before it writes** (`A1-113`). The rule engine had
  four call sites outside `aplan-core`'s own tests and all four were read commands, so each mutation
  hand-rolled a partial pre-check and they drifted: the fourth pilot's complaint about `mv` was never
  that the plan stayed broken, it was that the command which broke it said nothing. `add`, `close`,
  `close --restamp`, `amend`, `drop`, `mv` and `supersede` now compute the plan they would produce
  and report what is *new* about it. A new error refuses and names the rule with its remedy; a new
  warning is reported and left to `deny_warnings`; what the plan already had is not attributed to
  whoever touched it next. `--no-resolve` skips it as well as resolution, and says what it skipped,
  because one escape hatch is better than two.

  Five bespoke pre-checks are deleted rather than left beside it, including two that refused a
  closure whose blockers were unfinished under a comment saying they existed to prevent the
  `APLAN007` the rules now catch. One is *not* deleted: `APLAN021` asks whether a cut's `blocked_by`
  is empty, and a sequenced cut keeps its predecessor there, so a cut emptied of all its **work** is
  invisible to every rule. `mv` keeps a refusal that counts work members until `A1-134` writes the
  rule. Both surfaces gained the check together, and `A1-104`'s parity audit failed once in between,
  which is the audit doing its job.

- **`close.evidence_resolves_before_write`, so the gate can be required rather than pinned**
  (`A1-132`). `A1-109` taught both surfaces to resolve a proposed citation before writing a `done`
  claim, and it widened an existing command and an existing tool: no rule code, config key, format
  or evidence kind appeared, so nothing in `facts` moved and `--require close` went on passing for a
  build that recorded citations it had not resolved. A version string is not a capability, which is
  the rule `A1-099` made general; this is the change that most needed it. The fixture proving an
  older build does not advertise it is `facts.json` from the commit before the widening, which
  advertises `close`, `aplan_close` and `traces.table_rows` and is exactly the build an adopter could
  have pinned by name. The name is conservative by construction: it arrived after the behaviour, so
  it can refuse a build that has the guarantee and cannot accept one that does not. Its sentence
  carries the one exception, because `facts` prints it into a consumer's terminal: `--no-resolve`
  opts out on the command and says what it skipped, and the agent surface has no equivalent, which
  the schema an agent is handed is asserted to show.

- **`aplan supersede`, the repair `APLAN018` named and did not have** (`A1-130`, ADR-0020). The rule
  told an adopter to "reopen the task". There is no reopen: `drop` refuses to revive terminal work,
  and hand-editing `status: done` is the thing this tool exists to stop, so the published remedy was
  not executable. It never will be, and that is the decision rather than the shortcut. The work
  *was* done, and what changed is the ground under it: reopening would claim the work never
  happened, would make a finished cut's `done` false along with every downstream cut's premise, and
  would leave the old evidence either attached to unfinished work or deleted.

  `aplan supersede <id> <title>` files the successor and re-stamps the predecessor as reviewed. The
  predecessor keeps its status and its evidence, because both are still true about the moment they
  describe. The successor inherits `paths` and `traces`, which are the predecessor's own answers to
  the questions it asks again, and nothing else: no evidence, no blockers, no tags. The re-stamp is
  what clears a warning somebody has already dispositioned, and a warning nobody can clear is one
  everybody learns to ignore. CLI only, like `amend`.

  A milestone is refused: a cut is not superseded, its members are, and a successor to a cut would be
  work that cut does not contain, filed into no cut. Everything fallible happens before either write,
  including probing the predecessor for writability, so a failure is a refusal that writes nothing
  rather than a successor filed with the stamp not taken. There is no lock: a concurrent edit loses
  one write, exactly as it does for `close`, and git is what recovers it.

- **The import refusal says what it read, what it wanted, and what works instead** (`A1-129`,
  ADR-0019). The fifth pilot pointed the only Markdown importer at a 1,887-line programme plan and
  got `no importable proposal found` and an exit code. The refusal is right: that document has 45
  tables and 451 data rows describing streams, gates, risks, budgets, metrics and role assignments,
  and perhaps two of those tables are work. It now measures the document (`45 table(s) with 451 data
  row(s) and 81 heading(s)`), names the shapes it does read, and points at a documented extraction
  recipe; an empty file says it is empty rather than reporting tables it does not have.

  There is deliberately **no table importer**. A checkbox has one meaning wherever it appears, which
  is why `--format checklist` can read a document it has never seen; a table row has none, and an
  interface that never guesses would be a per-table mapping declared in `.aplan.yml`, which is a
  second place where a plan's meaning is written down. The extraction belongs in the consumer's
  repository, and `docs/importing-proposals.md` carries a three-pass `awk` recipe that this
  repository runs in its own suite, so the published commands are ones that have been executed.
  Whatever it produces goes through the checklist importer unchanged, so the gate does not move.

  The measurement reads the document the way the scanner does, outside fenced blocks: a file that
  *shows* a table in a ``` example has none, and counting them made the sentence explaining the
  scanner disagree with the scanner, which is the mistake the scanner itself already learned not to
  make when a fenced example was imported as a task. And the title report names a rewrite count only
  when there is one to do, rather than reporting zero on every import.

- **`APLAN030`: a task carries the number of members a declared family requires** (`A1-133`).
  `tag_families` said what a vocabulary contains and nothing about how many of it a task should
  carry, so `priority-p0` and `priority-p1` together, and no priority at all, were both clean. The
  fifth pilot needs exactly one priority and kind on live work, one primary stream on programme
  work, and one disposition on cancelled work. A family may now carry `require`
  (`optional` | `at-most-one` | `at-least-one` | `exactly-one`), `status`
  (`any` | `live` | `done` | `dropped` | `terminal`) and `milestones`
  (`include` | `exclude` | `only`). The list shorthand is the short form of the same thing with no
  requirement, so an existing `.aplan.yml` is unchanged.

  Scope is two keys rather than a list of predicates, so a scope that can never match cannot be
  written. An **error** rather than a warning, unlike the two rules beside it: they report something
  that looks wrong and may be a repository mid-authoring, and this reports a requirement the
  repository wrote down being unmet, which is what `APLAN022` and `APLAN017` already are; adopting
  it before the plan satisfies it is what `advisory:` is for. Only declared members count, so a task
  tagged `priority-p9` under `exactly-one` is `APLAN026`'s finding and not proof the requirement was
  met. `aplan add` refuses before writing, since it knows the tags, the status it writes and whether
  the task is a milestone; `close`, `drop` and `mv` rely on the whole-plan gate, because `drop`
  carries no `--tag` and refusing there would leave no way to cancel a task under a requirement
  scoped to `dropped`.

  It is silent on a family nothing in the plan carries, which is `APLAN029`'s finding and the reason
  that rule is this one's dependency: a requirement over a vocabulary no tag matches is unbindable
  rather than unmet, `exactly-one` would fail every task and `at-most-one` would pass every one, and
  both would be confident verdicts about a declaration that was never bound. One mis-declaration is
  one finding, not one per task and one more.

- **`APLAN029`: a declared tag family is carried by at least one task** (`A1-085`). The pilot that
  asked for `APLAN026` then mis-declared its own vocabulary and nothing said so: `epic: [a, b]` with
  tasks tagged `epic:a`, where the documented shape is the whole tag. Every layer accepted it. The
  family is named and non-empty, `group_by` names a declared family, and `APLAN026` judges only tags
  that already look like members, of which there were none, so `check` reported nothing and the
  roadmap rendered every task under `(no group)`. This is the defect `APLAN026` exists to prevent,
  one level up: that rule makes a vocabulary checkable so a typo in a *tag* is not silent, and
  nothing asked whether the vocabulary itself was used.

  A question about the plan rather than about `.aplan.yml`, deliberately. A family may legitimately
  list members that do not carry its prefix, `epic: [a, b]` with tasks tagged `a`, so a config-time
  check would refuse a working shape; only a rule can see whether any tag matches. A warning, on
  `APLAN026`'s own reasoning: a vocabulary declared before its first use is as likely as a
  mis-declaration, and the repository, which can set `deny_warnings`, is the judge. A tag in a
  family's shape counts as using it even though `APLAN026` is about to reject it, so one typo
  produces one finding rather than two. Opt-in with `tag_families`, and silent for a repository that
  declares none.

  It points at `.aplan.yml`, since the declaration is what it is about. Every other rule concerns a
  task and names that task's file, so this was the one rule in twenty-nine that could point nowhere,
  and a second unused family rendered as a bare sentence with neither a location nor a fix line,
  `check` printing the fix once per rule. The annotation now lands on the config file a reviewer
  would edit. Being the first task-less finding in practice, it also showed that such an annotation
  titled itself `APLAN029 -`, with a dash where the task would be, which reads like a title that was
  cut off; the title is now the code alone.

- **An unknown config key says which of its two causes it is** (`A1-068`). The first adopter added
  `test.covers` before staging the binary that knows it, their pre-push hook runs `aplan`, and the
  whole gate refused to load: they did not lose the new feature, they lost the ability to push. The
  message was `unknown field \`covers\`, expected one of ...`, which is exactly what a misspelling
  looks like, and the one fact that separates a typo from a binary one release behind, the running
  version, was the one thing the tool did not print. It now names the version and how many keys the
  build accepts, and says that a key from a newer `aplan` reads identically from here. Rejecting
  unknown keys is unchanged and stays. Two things came free with computing it properly: a near match
  (`requre_effort` suggests `require_effort`, printed *before* the version note so the common case
  leads), and every offending key rather than the first one `serde` reached. The keys come from the
  schema-derived inventory rather than from re-reading `serde`'s sentence, so they carry their path
  (`test.covers`, not a bare `covers`) and an adopter's own `tag_families` vocabulary is not reported
  as a dozen mistakes. A version table was rejected as unbuildable rather than costly: the error
  comes from the *old* binary, which cannot describe a key that did not exist when it shipped.

  It also reports what the file *still* fails on once those keys are fixed, because replacing
  `serde`'s message with a better one left the rest of its faults for the next run: `serde` stops at
  its first error, so a type error or a runner with no `{test}` placeholder below an unknown key
  waited until the adopter had fixed the key and run again. The answer is computed by applying this
  message's own advice, renaming a key that has a correction and removing one that does not, then
  re-running the whole load: whatever fails then is by construction a different problem and is
  exactly what they would have met next. Renaming rather than dropping matters, or a misspelled
  `test.lst` would be reported as a runner missing its `list`, a fault the real fix never produces.
  The second error drops its line number, which belongs to that repaired document rather than to the
  file the reader has.

- **The generated roadmap's banner names a command a consumer can run** (`A1-059`). It named this
  repository's own self-hosting invocation, `cargo run -q -p aplan -- roadmap --format markdown >
  ROADMAP.status.md`, in every consumer's committed file. In the third pilot there is no `-p aplan`
  package to run and the file is not at that path, so the line was wrong twice
  (pre-public adoption evidence). That is worse than a cosmetic slip: the banner convention exists so a
  generated artifact names the command that regenerates it, and the file is diffed against its
  generator, so a consumer cannot correct it by hand. The default is now the plain command, naming
  no output path rather than the wrong one, since only the consumer knows where their copy lives.
  `--regenerate-with` names a wrapper for a repository whose generator is one, which is how this
  repository's own file names `cargo xtask gen-roadmap` truthfully, and a consumer who wants their
  real output path in the line can write it there. The value is validated because it lands in a file
  that regenerates identically corrupt: `-->` would close the banner's comment, a backtick would
  make the field unparseable since backticks delimit it, a newline or a Unicode line separator
  (U+2028/U+2029, which `char::is_control` does not cover) would split the banner, padding would
  make two repositories naming the same command produce two different files, and on a format with no
  banner the flag would be accepted and ignored rather than refused.

- **`advisory:` in `.aplan.yml`, so a rule can be on during the migration it exists to help**
  (`A1-112`). `APLAN022` requires every task to belong to a cut, and the first pilot could not turn
  it on: the check most useful *during* a migration was the one a migration could not adopt until
  it was finished (pre-public adoption evidence, item 3). A listed code is now reported **in full and
  at its declared severity**, and excluded from the verdict. Severity stays the rule's own, in the
  output, in `aplan facts` and in the catalogue, because a consumer asserting what a rule *says*
  must not be asserting something a config can change; the verdict is what a repository configures,
  which is where `deny_warnings` already lives. Nothing is hidden: the summary names the count and
  the codes, `--format json` carries an `advisory` count, and a code this build does not know is
  refused rather than ignored, since a typo would downgrade nothing while reading like a choice.

- **`aplan amend --add-evidence`, for proof that was never whole** (`A1-123`). `A1-110` built the
  repair for a citation that *rotted*; this is the one for a fix whose tests were written and never
  cited. Measured on this plan: 52 of 90 closed tasks touch code and cite no test at all, so their
  machine-checkable proof is a commit. Gated identically to a replacement, so it cannot record what
  `verify` would reject, and refused if the task already carries the citation.

- **This repository resolves its own `test` evidence** (`A1-120`). `.aplan.yml` required
  `test` as an acceptable strength and configured no runner, so `APLAN013` had never run here:
  28 citations across 27 tasks, none ever resolved, while `verify` reported `evidence resolved`.
  Measured before changing anything: **0 of 28 resolved**, because every one used a
  `crate::path::to::test` form that `cargo test -- --list` does not print -- the first pilot's
  exact mistake, at scale, in the plan that shipped the rule. One runner per crate rather than a
  single `--workspace` one: bare names are unambiguous today (602 names, no collisions, measured),
  and split this way a future collision is refused by `APLAN013` instead of resolving against
  whichever crate ran. All 28 citations were repaired through `aplan amend`, which is what it was
  built for.

- **`aplan amend`, the supported repair for a closed task's evidence** (`A1-110`). `test` evidence
  is cited by name, so a rename leaves closed tasks pointing at nothing, and the answer used to be
  `close` refusing twice and then telling you to "edit the rest by hand": the repair for a
  tool-detected problem was to hand-edit the files the tool owns. `aplan amend <id>
  --replace-evidence <from> <to>` replaces one citation in place, without reopening the task.
  **Both sides are named**, so the diff shows what was replaced and by what, a `from` that matches
  nothing is refused rather than guessed at, and there is no shape of this that swaps a whole block
  for something nobody checked. The replacement is resolved exactly as `close` resolves one
  (`A1-109`), because a repair that accepts anything would be that defect with a second door.
  It does not touch `trace_fingerprints`: repairing a citation is not re-reviewing a traced
  section, and `close --restamp` remains the other repair, named in `amend`'s help so the two are
  findable together.

- **`APLAN016`: `url` evidence answers when it is checked** (`A1-041`, warning, `verify`, opt-in).
  `url` was the one evidence kind `verify` never resolved -- checked for shape and nothing else --
  which made it the weakest link in an otherwise resolved ladder, and a regulator page or vendor
  quote that has since 404'd is exactly the proof that rots quietly. The checker is a **configured
  command**, `url.check`, for the reason the test runner is one: the tool does not decide on your
  behalf what touching the network means, timeout and redirect policy live in the command where
  they are yours to set, and no key means no network, so `check` keeps its hermetic contract and no
  default run reaches out. The citation is passed as a whole argv element (`{url}`, validated like
  `{test}`), never spliced into a command. Reported as *did not answer*, because a link down for an
  hour is not a plan that is wrong. One request per url, however many tasks cite it.
  **`https` citations only**: the citation comes from a task file, and a task file can arrive in a
  fork pull request, so the configured command cannot defend itself against the target it is
  handed. Any other scheme is reported as not checked and never passed on.

- **`APLAN019`: a task's `paths` globs each match a tracked file** (`A1-022`, warning, `verify`).
  `paths` is the fixed point three shipped rules judge evidence against -- `APLAN012` (does a
  cited commit touch it), `APLAN024` (how much falls outside it), `APLAN025` (does a cited test
  cover it) -- and it was the one input nothing checked, so each phrased its finding as a defect
  in the *evidence* when the paths were at least as likely to be wrong. The shape every pilot hit
  is a bare directory: `paths: [libs/x/src]` matches nothing, and `APLAN012` then reports a commit
  as touching none of the task's paths even where it plainly did. That case gets the
  one-character remedy by name rather than generic advice. Reported per glob, so the dead entry is
  named rather than the list it sits in, and distinct from `APLAN023`, which fires when a task
  declares *no* paths: there the remedy is to add a glob, here it is to fix one.
  A `done` task is judged at the revision its evidence cites rather than at `HEAD`, so deleting
  code a closed task owned does not turn a correct glob into a finding advising you to correct it;
  a glob that matched nothing at any revision is still reported.

- **`mcp_tools` in `aplan facts`, and `--require` asserts them** (`A1-119`). `facts` described
  every surface except the agent one, so an adopter wiring an agent had no machine-checkable way
  to ask whether a build's server offers `aplan_verify`. `aplan facts --require aplan_verify` now
  answers it, which is `A1-082`'s argument applied to the surface it had skipped.

- **`docs/rule-catalogue.md`, what every rule says, generated and gated** (`A1-088`). Every rule,
  its severity and title, and every distinct message and `agent_instruction` its scenarios
  produce, compared on every test run and regenerated with
  `APLAN_CATALOGUE=overwrite cargo test -p aplan-e2e --test coverage`. Three text defects had
  shipped from here with every predicate behind them correct: `APLAN024` named the first cited
  commit while scoring the worst and reached the p00025 pilot, `APLAN026` named the family a tag
  was nearest to rather than the one it claimed, and a footer told readers that suppressed
  findings repeated a fix that in fact differs per finding. None was visible in a diff, because a
  sentence changes without any test noticing. Now it is a diff, next to the code that changed it.

  Generating it exposed two defects in itself. The first version read only the hermetic
  `validate`, so all seven evidence-ladder rules rendered as "no scenario produces this
  diagnostic" and a note calling that a gap: a generated file asserting something false about a
  quarter of the rules, in the change written to stop messages asserting falsehoods. Their cases
  were firing all along, in a second corpus whose helper reduced its report to `(code, severity)`
  at the point of production, putting every ladder rule's wording out of reach. And the artefact
  could never have compared equal to itself, since messages quote the scenario's temp root and
  cited SHAs, both fresh per run; both are normalised and stability is verified by regenerating
  four times and diffing.

  A second audit found the artefact promising more than it held: titled "What every rule says",
  it pinned exactly one wording for 24 of the 25 rules against 45 diagnostic construction sites.
  A rule that interpolates its reason speaks several sentences, and only the branch some scenario
  reached was recorded, so `APLAN008`'s *anchor not found as a heading* (the branch an adopter
  meets after renaming a heading, and the commoner of its two) and `APLAN021`'s *carries work*
  went unpinned. Both have scenarios now, 26 wordings to 28, the title says *as the corpus
  exercises it*, and the one branch still unreached is named in the file rather than left to be
  assumed away.

### Added

- **Three mechanical properties over the tool's own text** (`A1-089`), which `A1-087`'s sweep made
  possible. Every backticked `aplan ...` a message names must exist, asking whether the command
  and its flags are real rather than whether the line is runnable: an instruction naming
  `aplan close --restamp` names a *form*, and demanding a complete invocation would force every
  message to invent a task id. No message may leave a plural unresolved, `entr(y/ies)` and
  `capabilit(ies)` having both shipped, while the house idiom `1 task(s)` is permitted. And a
  diagnostic may name only ids its own scenario contains, scoped to id-shaped spans so a message
  may still backtick a field name, a status or a glob. Each was proven by planting the defect it
  exists to catch, and each asserts it checked something so one that stops matching fails rather
  than passing over an empty set.

  Stated plainly: the first two are regression guards, and the third catches only a message naming
  something *absent entirely*. None of them catches `APLAN024` naming the wrong one of two commits
  that are both present, which is the defect that reached a pilot. Only `A1-091` prevents that.

  Writing them found a defect in the harvester underneath: string literals kept their escapes
  verbatim, so a message continuing with a backslash-n read as `n(` and the plural property
  reported a welded plural in a line that has none. Escapes are decoded now, including the
  line-continuation form every long message here is written with, so the properties see the text a
  consumer sees.

  The audit then found two defects in the properties themselves. `looks_like_a_task_id` was
  documented as requiring "at least one digit somewhere" while implementing no such rule, and the
  same sentence offered `DUP` and `GHOST` as examples, neither of which has a digit: a comment
  asserting something false about the predicate beside it, in the change written to catch text
  that asserts something false. And `backticked` had become a second implementation in the crate
  the first could not reach. Unifying it was refused on layering, since `aplan-e2e` sits at the
  top of the dependency graph and reaching it from `aplan` would leave a cycle waiting to happen;
  both copies now carry the same unit test so a divergence fails rather than hides.

### Fixed

- **The `aplan_close` description said two contradictory things, and one had become false**
  (`A1-131`). It promised "Refuses exactly what the CLI refuses" and, in the next sentence,
  "Recorded here, not resolved: run `aplan_verify` to check it against the repository". The second
  was true until `A1-109` taught close to resolve evidence before writing, so an agent was told to
  distrust or duplicate a gate that had already run. It now says what close resolves (commit, file,
  and a test where a runner is configured), what it will not do (run a test, fetch a url), and that
  anything nothing could check is named in `unchecked` rather than left looking like a pass. A test
  holds every clause against actual behaviour and fails for under-promising as much as for
  over-promising, which is the guard neither `A1-119` nor `A1-104` provides: one holds the command
  help to the tool list, the other compares the two surfaces to each other.

- **`aplan_close` reached the network where `aplan close` does not** (`A1-131`, found writing that
  test). The command deliberately resolves a closure with no `url.check`, because a link down for an
  hour must not block recording a citation, and `APLAN016` is a warning precisely because it is not a
  statement about the plan. The agent surface passed the whole policy, whose `url_check` belongs to
  `aplan_verify`, so under `deny_warnings` an agent was refused a closure the command records. The
  guarantee is that neither surface can do what the other cannot, and it holds in both directions
  now; `aplan_verify` still checks urls.

- **`aplan_add` promised a guarantee it does not keep, and told an agent nothing when it broke the
  plan** (`A1-131`, found extending that guard one tool over). "Refuses before writing what `check`
  would reject after" is total, and under `require_cut_membership` `add` writes a task in no cut,
  which `check` rejects. The command says so to a human; the payload said nothing to an agent, so
  the surface that exists for agents was the one that did not mention the plan it had just broken.
  The description now names what it can judge from the task alone, and the payload carries a `notes`
  list naming `APLAN022` where it applies and nothing where it does not.

- **`APLAN025` no longer asserts a cause it has not established** (`A1-070`). Its single-citation
  message ended "so the citation borrows proof from another part of the repository", which is one
  explanation of no overlap and was not the true one in the case that produced the finding: the
  third pilot's citations were correct and unchanged, and the task's `paths` had lost the entry for
  the file it wrote. The two explanations have opposite remedies, so the message sent the reader to
  fix the half that was right. `paths` is the least verified input in the model and three rules
  triangulate evidence against it as ground truth, so a rule of that shape may state what it saw and
  not why: the message now names the coverage the citation resolved under and the `paths` that do
  not reach it, and leaves the reader both remedies, which its fix line always offered. The
  multi-citation form was already an observation and is unchanged, including the `paths` it prints,
  which is the detail that let the pilot find their own mistake and which nothing had pinned.

  The line an agent acts on had the same lean and now matches: it read "or widen `paths` if the work
  really does span that component", which frames editing `paths` as an accommodation for work that
  genuinely reaches elsewhere. The pilot's case was the other one, so an agent applying that
  condition would have concluded the work does not span and gone to change the citation, which was
  correct. It now reads "or correct `paths` if they do not name all of it. This cannot tell which of
  the two is wrong", mirroring `APLAN012`'s. And the rule's third wording, the one a `covers_by_test`
  subset produces, was in no corpus scenario, so it was the one message this rule can print that
  could change without a reviewable diff; it is in the catalogue now, along with a fixture that no
  longer names a runner after the glob it covers.

- **`--root` no longer switches off configuration checking** (`A1-068`, found building it).
  `ready`, `summary`, `tasks`, `critical-path` and `graph` read no config keys of their own, so they
  met `.aplan.yml` only while resolving the plan root, and passing `--root` returned before that.
  All five answered confidently and exited `0` in a repository whose configuration could not load,
  and all five already failed without the flag, so the flag was deciding the verdict. `--root`
  chooses where the plan is, not whether the configuration is checked.

- **The policy reaches every surface, not just the two that were easy** (`A1-112`, found
  auditing it). `--format github` and `--format sarif` ignored `advisory:` entirely, so the one
  format a repository uploads to a code-scanning dashboard was the one that could not tell an
  accepted finding from a blocking one: error-level alerts, no marker, on a build that passed. A
  SARIF result for a downgraded rule now carries a `suppressions` entry, which is the word that
  format already has for exactly this, and keeps its `level` so the dashboard still learns what the
  rule says. A GitHub annotation says `(advisory)` in its title, since annotations have no
  suppression concept.

- **`close` no longer refuses what `verify` accepts** (`A1-112`, found auditing it). The close gate
  read one flag of the policy rather than the policy, so a repository that made a rule advisory got
  a green `verify` and a refused `close` on the same citation. That is the disagreement `A1-109`
  exists to prevent, rebuilt one config key later. The flag-only predicate was deleted rather than
  fixed in place, so the next caller cannot make the same mistake, and the count each surface
  reports now comes from one implementation instead of four.

- **A plan that did not load is refused by every command that reads one** (`A1-112`, found auditing
  it). `verify`, `ready` and `why` already stopped; `check` called the same plan `ok` once
  `APLAN001` could be downgraded, and `verify` printed `ok: 3 tasks ... evidence resolved` beside
  an exit code of 1 having resolved nothing. A file the loader could not read is not a finding a
  repository can be tolerant of, because every other rule's silence about that file means nothing.
  Both now fail, and say `1 task file(s) did not load` on the verdict line itself rather than only
  on stderr, which a pipe separates from the report.

- **A passing run no longer summarises as `failed`, and a failing one no longer summarises as
  `ok`** (`A1-112`). The human summary counted raw severities while the exit code came from the
  policy, so the two could disagree in both directions: under `deny_warnings`, a plan with one
  warning and no errors exited `1` beneath `ok: 1 tasks, 1 warning(s)`, which predates this task
  and was found building it. The word is now the verdict's and the counts are the blocking ones,
  in the human summary, in the JSON `errors`/`warnings` fields, and in the `denied:` note that
  says which knob stopped the build.

- **The house style no longer follows an adopter into their own pilot log** (`A1-052`).
  `cargo xtask lint` forbids em dashes and the like in every tracked file, which is this
  repository's style and stays its default. But a pilot friction log lives in `private pre-public adoption records` by our
  convention while being a *consumer* writing about *their* repository, so the rule required an
  adopter's prose in our dialect, and one of them carried a warning in their own contract file
  about not copying prose across in either direction. The scope is now read from
  `.house-style.yml` rather than compiled in, `private pre-public adoption records` is exempt there, and
  pre-public adoption evidence says so where an adopter reads it first. `**` or `.` declines the style
  entirely, a blank entry is refused so a stray line cannot switch the gate off, and the lint
  reports how many files it skipped. A run that scanned nothing says **not applied** rather than
  clean, because a gate that looked at nothing is not a gate that passed.

- **A section heading keeps its letters, whatever script they are in** (`A1-051`). The checklist
  importer slugified a heading with an ASCII-only test, so every accented letter and every
  non-Latin script was deleted: a heading reading `Cafe naive <CJK> resume` with its accents became
  `caf-na-ve-r-sum`, and a heading in a non-Latin script produced no tag at all. Preserved rather
  than transliterated, because a transliteration table is a guess about a language nobody here
  reads, and because nothing else in this tool imposes an ASCII restriction: `check` accepts a
  unicode title, `add` writes one back unchanged, and the schema says a tag is a string. A plain
  ASCII heading is unchanged, the Unicode test being a superset of the ASCII one. Combining marks
  are kept with the letter they belong to, so the same word written decomposed (which many editors
  and macOS produce) keeps its accent rather than losing it the way the ASCII test did; marks in
  script-specific blocks are a stated limit, because answering those needs Unicode category data. And a heading
  with nothing alphanumeric in it is now **reported** rather than silently producing no tag, which
  is the contract `import` already advertises and the one loss it was making in silence.

- **A checklist item's title is its first sentence or its lead-in, not the whole paragraph**
  (`A1-055`).
  `p00025` writes checklist items as miniature changelog entries, so a faithful import produced an
  **832-character title** that cannot be read in `ready`, a roadmap table, or a diagnostic. The
  title is now the item's first sentence, the item survives verbatim in the task body, and the
  report says how many were shortened. On that file the worst case went 832 -> 65 characters.
  Only 5 of 40 items have a second sentence, because that file writes a bold lead-in, a dash, then
  the description, so a second rule takes the text before the first top-level separator and the
  **shorter** of the two candidates wins: the longest item has a 213-character first sentence and a
  38-character lead-in. Measured on that file, the median title went from 118 to 88 characters and
  the worst case from 832 to 65. Three items stay long on purpose, being comma-separated lists of
  filenames with no sentence and no separator, because cutting them would mean choosing a boundary
  the author never wrote.

- **A test runner that could not answer was read as a runner that listed nothing** (`A1-120`,
  found auditing). `test.list`'s exit status was ignored, so a package that failed to compile
  produced an empty list and every citation against it was reported as "not one the configured
  runner lists" -- true of the empty list, and the wrong cause, sending the reader to fix a
  citation that was correct. Eight runners made it likely rather than theoretical. A failing
  `test.list` now stops the run and reports the runner, its exit status, and its own first line
  of stderr.

- **A repair was refused for a citation it was not touching** (`A1-110`, found dogfooding
  `A1-120`). `amend` judged the whole task, so a task with two rotted citations could not be
  repaired one at a time: each repair was blamed for the other's defect. That is the first pilot's
  own case, seven citations across two tasks. A repair is now judged as a delta, on what it
  introduces, and a replacement that is itself unresolvable is still refused.

- **`close --restamp` moved the `trace_fingerprints:` block to the end of the frontmatter**
  (`A1-086`, found auditing `A1-110`). Acknowledging drift on a task whose fingerprints sat before
  other keys arrived as a diff that reordered the frontmatter, in a module whose whole premise is
  that a rewrite is surgical so the one line that changed is the whole diff. Both repairs now
  substitute the block where the author put it.

- **`close` refuses evidence `verify` would reject, instead of certifying it** (`A1-109`). The
  first pilot closed a task citing three integration tests, was told `closed`, and `verify`
  rejected all three seconds later: the citations used `guider::name` where the runner lists them
  bare. The tool contradicted itself about its own central claim, and the task sat `done` on
  evidence this project's own gate rejects. `close` now resolves the citation before recording it,
  through the same ladder `verify` runs rather than a second copy of it, and refuses exactly what
  would fail the build. `APLAN013` names the near match when the runner lists one, which is the
  difference between a typo and a dead end. `--no-resolve` records without checking and says so;
  a repository with no runner, or no git at all, still closes and is told which citations went
  unchecked rather than left to read silence as a pass.

- **`aplan mcp --help` named four tools where the server exposes six** (`A1-119`). `aplan_verify`
  (`A1-102`) and `aplan_add` (`A1-103`) both landed without the sentence being touched, so the
  help retracted two shipped features: an operator reading it concluded an agent could not
  resolve the evidence it cites or create the work it is told to create. Seven other live claims
  said the same thing, in `README.md`, `HLD.md`, `CLAUDE.md`, and the crate's own module docs. A
  test now holds the help to `aplan_mcp::TOOLS` in both directions, so a tool cannot be added
  without being announced, and the help cannot promise one that does not exist.

- **MCP `check` dropped the verdict, so a denied plan read as clean** (`A1-101`). On a repository
  setting `deny_warnings`, the CLI printed `denied: 1 warning(s)` and exited 1 while `aplan_check`
  returned `errors: 0, warnings: 1` with `isError: false`. The diagnostics were all there; only the
  answer was missing, and `isError` is the field an MCP client treats as "did this succeed", so an
  agent concluded the plan passed and handed CI a build that failed on the gate it thought it had
  just run (pre-public adoption evidence, item 18).

  Same cause as the stamping defect above: `verdict` was private to the `aplan` binary crate.
  `Report::denied` now lives in `aplan-core`, the CLI's exit code and the MCP payload both come
  from it, and `deny_warnings` travels to the server beside the options rather than inside them,
  since it is a verdict policy and not a validation one. `isError` is untouched and goes on meaning
  "the tool ran", which is the distinction ADR-0007 draws between exit codes 1 and 2: a violation
  is not a malfunction.

  Auditing it found the wiring unpinned, which is the hole the stamping fix shipped with: the
  parity comparison passes the policy by hand, so nothing checked that `aplan mcp` resolves
  `deny_warnings` from `.aplan.yml`. A test now drives the real stdio transport from a foreign
  directory for both, and `Report::denied` gained the truth table it lacked in the crate that owns
  it.

- **Closing through MCP left drift detection off for every task an agent closed** (`A1-100`).
  `aplan_close` wrote `status: done` with the evidence and no `trace_fingerprints` key at all, so
  `APLAN018` was silently disabled for that task while its file still declared its `traces`. The
  fourth pilot found it by driving the surface its own `CLAUDE.md` tells agents to use
  (pre-public adoption evidence, item 17), and `APLAN028` could not have caught it: that rule's
  predicate is "stamps all if it stamps any", and zero stays silent deliberately, which is exactly
  the shape MCP produced.

  The cause was a boundary, not an oversight. `trace_fingerprints` was a private helper in the
  `aplan` binary crate, so `aplan-mcp` could not call it and closing without stamping was the only
  option available. It now lives in `aplan-core` beside `close`, which consumes it, and both
  surfaces use that one implementation rather than two that are supposed to agree. The code comment
  defending the old behaviour is corrected too: it said stamping was `close --restamp`'s job, and an
  ordinary `close` stamps.

  Auditing that fix found it dead above the tools layer, which is where the parity audit cannot
  look: `aplan mcp` read `.aplan.yml` from the process's directory rather than the plan's, and let
  `traces_root` fall back to `.`, so a client that started the server anywhere else got
  `detect_drift: false` and unreadable documents. Both halves are load-bearing, isolated on clean
  fixtures, and pinned by a test that drives the real stdio transport from a foreign directory.

### Added

- **The drift loop is tested end to end** (`A1-107`). Four of the last six pilot findings were
  about trace drift, and every one was proved by a unit test on `anchor_fingerprint` or by a run in
  a throwaway directory that nobody kept. Those prove the function and the afternoon; they do not
  prove the command, and if `close` stopped stamping tomorrow every test in `anchor.rs` would still
  pass. Five tests now drive the built binary through the sequence a repository performs: close,
  edit a sibling, edit the traced thing, re-stamp, retarget. Each was run against the defect it
  exists to catch.

  Not a `trycmd` case, and the reason was measured rather than assumed: the corpus runs a case
  inside its own `.in` directory with no sandbox, so a writing command edits the fixture. A
  throwaway case that closed a task changed the fixture's hash.

### Fixed

- **`verify --format json` answers a broken plan in the format it was asked for** (`A1-106`).
  `check --format json` hands a machine consumer `APLAN001` with the file and the parse error; the
  other commands that refuse a partial plan wrote nothing to stdout, so a consumer learned only
  "something is wrong, go run another command". `verify` renders a report, the same document
  `check` renders, and now reports what loaded under the repository's own options, with nothing
  resolved and the exit code still 1. That also closed a divergence: the MCP `verify` already
  returned the check report there, so the two surfaces now agree exactly rather than approximately.

  `summary` and `tasks` render rollups rather than reports, so carrying diagnostics would make
  their shape conditional on the plan being broken, which is worse for a consumer than an empty
  stream. Their refusal stands and is documented in their help, where a consumer meets it instead
  of by breaking a plan.

  The leak gate that keeps this project's decision numbers out of consumer text now covers task ids
  too. It caught the help text this task had just written, and one legitimate example that it
  cannot tell apart from a citation; that example states the rule instead of showing two ids from
  somebody else's plan.

  Auditing it caught the human output saying everything twice: the stderr refusal lists each load
  failure and the new report carries the same diagnostics, so one finding read as two problems.
  `verify` now prints a one-line note instead, which is also more honest than the message it
  replaced, since "any answer would be incomplete" was written for a command that gave no answer.

### Added

- **`ready` says which silence it is** (`A1-105`). At the moment a cut completes, `ready` printed
  nothing at all: indistinguishable from a finished plan or a broken config, while every task in
  the next cut waited on a milestone somebody could close. It is the command an adopter opens every
  session with, and the fourth pilot met this on its first cut boundary
  (pre-public adoption evidence, item 21). It now names the closeable cut and the command that opens what
  follows, and says so explicitly when nothing is closeable either, because an empty list answers
  two questions and telling them apart is what makes the first answer trustworthy.

  `why` stops calling a milestone "ready", on the reasoning its own parked branch already used one
  line above: `ready` means workable, unblocked and not a checkpoint, and using the word for a
  different predicate made the tool contradict itself. `aplan_ready` over MCP gains
  `closeable_cuts` beside `ready`, never inside it, so an agent filtering for work does not find a
  checkpoint there.

  Auditing it found two defects it had introduced and one older one. An empty cut was offered as
  closeable, because `all()` over no members is true, so the reader was told to record the
  completion of a cut that scoped no work. A cut already `done` was advised to be closed again, a
  command `close` would refuse. And `why` had long said "is ready" about any finished task, which is
  the same mismatch: `ready` excludes terminal tasks exactly as it excludes checkpoints. All three
  are fixed together, since they are one question about what the word may be used for.

- **`aplan_add` on the MCP surface** (`A1-103`). The host repository's instruction to its agents is
  "before you write code, find or write its task", and step one is `aplan add --cut`, precisely
  because `add` maintains the milestone's membership and a hand-written file will not satisfy
  `require_cut_membership`. An agent could list ready work and close it and could not create it, so
  the instruction was not performable through the interface it was given
  (pre-public adoption evidence, item 20).

  Every refusal the CLI makes is made here in the same order, and all of them before anything is
  written: an empty title, a non-finite estimate, an id that exists, a blocker that does not, a cut
  that is not a milestone. The id follows the plan's own convention rather than being supplied by
  the caller, which is the reason `add` is a command and not a template.

  It also moved `ADD_BODY` into `aplan-interop::emit`. That is the third piece of shared behaviour
  found on the wrong side of the crate boundary, after the stamping helper and the verdict, and
  each of the first two had already drifted by the time a second surface needed it.

  Auditing it found `add` leaving a task behind when its cut could not be written: two writes, and
  the second can fail after the first succeeded, so a plan gets a task in no cut, which
  `require_cut_membership` rejects. Both surfaces did it identically, so the parity audit was
  satisfied and the plan was still broken, which is the difference between parity and correctness.
  `emit::write_task_placed` does both writes or neither, for both surfaces, and says so if the
  rollback itself fails.

- **`aplan_verify` on the MCP surface** (`A1-102`). The four tools were `ready`, `why`, `check`,
  `close`, and `check` is hermetic by design, so the whole loop an agent could perform was blind to
  unresolvable evidence: it could close a task citing a file that does not exist, see `0 errors`,
  and hand CI a plan that fails on the citation it just wrote. `close`'s own description called
  evidence "a citation to be resolved", so the surface promised the product's central claim and
  could not perform it (pre-public adoption evidence, item 19).

  The new tool resolves evidence against the repository and is a superset of `check`, as the CLI's
  `verify` has been since `A1-058`. It never executes tests, whatever `.aplan.yml` says: running a
  repository's tests because an agent asked is a different trust question from reading its history.
  `close`'s description now says "recorded here, not resolved: run `aplan_verify`". The pilot's own
  case, over the real transport: `check` reports clean and `verify` reports `APLAN015`.

  Auditing it caught the guard the happy path had left behind: the CLI refuses to resolve evidence
  on a plan that did not load whole, and the new tool did it anyway, so a plan with one unreadable
  file got findings that read as complete and were not. The parity comparison had also promised
  "the same diagnostics and the same verdict" while comparing only the verdict; it compares both
  now, which is what found the guard.

- **An audit that the MCP surface agrees with the CLI** (`A1-104`). The eighth pilot pass drove
  `aplan mcp` for the first time and found two divergences in one sitting, both invisible: closing
  through MCP left drift detection off for every task an agent closed, and `check` dropped the
  configured verdict so a denied plan read as clean. They are one shape, since the MCP tools are a
  second implementation of behaviour this binary crate owns, and the pieces the server cannot reach
  are the pieces that drift.

  `crates/aplan/tests/mcp_parity.rs` compares each tool against the real binary, run as a
  subprocess, with "agree" defined per tool: `close` leaves a byte-identical file, `check` reports
  the same verdict, `ready` and `why` list the same ids in the same order. It walks `tools::TOOLS`
  in both directions, so a tool added later must be classified. Written before the fixes and
  watched fail on exactly `close` and `check`, which is the evidence it would have caught them;
  those two are listed in `PENDING` against the task that fixes each, so the evidence stays
  runnable rather than becoming two scattered `#[ignore]`s.

  Auditing it found the classification decorative: `Parity` appeared only in the table and drove no
  comparison, so a new tool could be classified while nothing compared it. `compare` dispatches on
  the classification now, and the match is exhaustive, so being in the table is being checked. Two
  comparisons could also have passed on empty lists, and now say so.

- **`check-docs` reads `CHANGELOG.md`** (`A1-067`). `APLAN024` shipped closed, green and
  unmentioned in the changelog; two commits later `APLAN025` did the same, and only then did anyone
  look. Nothing here could have caught either, because the docs gate read `ROADMAP.md` and
  `private pre-public adoption records` and nothing read this file at all. The immediate cause was a scripted edit whose
  anchor did not match and wrote the file back unchanged, but the durable point is the one this
  project exists to make: the commit message claimed the entry, and the claim was taken on trust.

  Two checks, both without judgement. Every code in `Rule::ALL` must be named here, since a rule
  code is a consumer contract that is never renumbered and one reaching a release unannounced is a
  contract change nobody was told about; the failure names the missing codes. And every task id
  this file names must resolve, which is what the roadmap and the pilot logs already had. Requiring
  every closed task to appear was considered and refused: most are not consumer-visible, and a gate
  people route around is worse than no gate.

  A rule counts as announced when it is the subject of a top-level entry or covered by a range
  (`APLAN001`-`APLAN010` announces ten). The first version tested `changelog.contains(code)`, which
  is the predicate `A1-092` deleted from this same file as useless: ten of the twenty-six rules
  were passing on an incidental mention inside another rule's entry, and the founding set, written
  as a range, was not seen at all.

- **`facts` names behaviours, and `--require` asserts them** (`A1-099`). `--require` exists because
  a version cannot answer "can *this* build run my gate", and it asserts names `facts` lists:
  rules, commands, config keys, formats, evidence kinds. A change that widens what an *existing*
  name accepts adds none of those. ADR-0015 taught `APLAN008` to resolve a table row, and
  `--require APLAN008` passes on every build since that rule existed, so the fourth pilot depended
  on a behaviour it could not assert (pre-public adoption evidence, item 15).

  `--require traces.table_rows` now asserts it. The name sits in its own namespace rather than as
  a dotted rule code, because a rule code is a stable consumer contract and `APLAN008.table_rows`
  would read as a rule. `AGENTS.md` records when a change earns a name, and that this widening was
  survivable by luck: an older binary *rejects* a row anchor loudly, where a widening the other way
  would be a plan passing while checking less than it says.

  The audit then proved the claim rather than reasoning it: a test runs the capability lookup
  against this repository's own `facts.json` from the commit before `behaviours` existed, where the
  name is correctly unadvertised while rules, commands and config keys still resolve. It also found
  the namespace unguarded, since `advertised` is one flat set: a behaviour sharing a name with a
  rule or config key would make `--require` pass for the wrong reason, which is worse than failing.
  Collisions, rule-code spellings, and behaviours missing from `docs/upgrading.md` are now all
  rejected.

- **`APLAN028`: a closed task stamps every trace it stamps any of** (`A1-098`). `trace_fingerprints`
  is keyed by the trace string, so *editing* a `traces` entry on a `done` task orphans the old key
  and leaves the new trace unstamped. `APLAN018` then has nothing to compare and says nothing: the
  plan reads clean under `deny_warnings` with `detect_drift` on, and the traced section can change
  afterwards with **no diagnostic at all**. Both of the fourth pilot's traced tasks sat in that
  state and passed, and the ADR-0015 migration is exactly that edit, so the path this repository
  recommends walked an adopter through the hole.

  The pilot proposed "every `done` task has a fingerprint for every trace", which would be wrong as
  written: `APLAN018` deliberately treats an unstamped trace as unverifiable rather than in
  violation, and a task closed before `detect_drift` was enabled has traces and no fingerprints
  legitimately. Zero stamps stays silent; stamping *some* has no honest reading. The message names
  the orphaned key when there is one, because it is the only surviving record of what the task was
  closed against, and `close --restamp` repairs both halves.

  The audit found it naming one unstamped trace where a plan had two, which reads as "that is the
  one"; every unstamped trace is named now, with the sentence written out at both counts rather
  than a `(s)` construction. It also verified the instruction's promise rather than repeating it:
  `--restamp` really does drop the orphaned key, checked against the live reproduction.

### Fixed

- **A linked heading minted an anchor GitHub does not** (`A1-097`). `slugify` reads the raw source
  line and stripped punctuation, so `## [Linked](http://e.com)` became `linkedhttpecom` where
  GitHub mints `linked`. An author who copied the anchor from the rendered document's URL, which
  `anchor.rs` says is the intended workflow, was told by `APLAN008` that a correct trace was
  dangling, and their only recourse was to write the mangled form, which then does not link.
  Reachable since `APLAN008` shipped; found while auditing `A1-095` and confirmed pre-existing,
  since rows and headings share `slugify`.

  A link's target is now dropped before slugifying, so reference links and images slug to the text
  a reader sees. Underscore emphasis is deliberately left alone: matching GitHub there means
  treating `_` as emphasis, and `_` is load-bearing in identifiers like `require_cut_membership`
  and `covers_by_test`. That boundary is pinned by test with its reasoning, rather than left as a
  gap someone later "fixes" into a regression. No heading in any tracked document contains a link,
  so no existing anchor moves.

### Added

- **A trace may name a table row, so drift is scoped to the fact** (`A1-095`, ADR-0015). A
  fingerprint covered a whole heading, which is right for an ADR and wrong for a register: p00020
  keeps twelve invariants in one table, so closing a task that edits invariant 11 re-flagged every
  task tracing invariant 7, and the answer was `--restamp` every time. Ten more such edits are
  scheduled. A warning that is always correct to dismiss trains dismissal of the firings that are
  real, so the cost was to the whole rule rather than to one document.

  `constitution.md#invariant-7` now fingerprints that row alone. **No new grammar and no schema
  change**: a row's anchor is minted from its first cell with the heading slug rule, and
  `trace_fingerprints` is keyed by the trace string. Headings resolve first, so every existing plan
  fingerprints exactly what it did before; rows mint on their own counter, so adding a heading
  cannot move a row's anchor; and the header row is not an anchor, since markdown says which row it
  is and it names columns rather than facts. `APLAN008` widened to match, and now reports "matches
  no heading or table row".

  The price of content addressing is stated rather than discovered: renaming a row's first cell
  breaks the trace, and `APLAN008` reports it dangling instead of `APLAN018` reporting drift. That
  is the right failure, since renaming the key changes what the row is.

  Auditing it verified the end-to-end path that the record had only argued for: closing stamps the
  row, editing a sibling row leaves `check` silent, and editing the traced row raises `APLAN018`.
  It also corrected `APLAN008`'s instruction, which told an author to add "a table row whose first
  cell reads `renamed`" when the cell must *slugify* to that, and pinned two limits by test: two
  rows sharing a first cell are told apart by order, and a cell carrying markup slugifies exactly
  as a heading does, which had been working by accident.

- **`APLAN018` hands over the command that shows what moved** (`A1-096`). The warning named the
  task, the file and the anchor, and left the reader to work out which lines of the section had
  changed, often for a closure somebody else made. The instruction now opens with
  `git diff <sha> -- <file>`, taking the SHA from the commit the task cites.

  Named only where the task cites **exactly one** commit. `aplan close` refuses a task that already
  carries evidence, so every citation arrives in one block and nothing distinguishes which of
  several the stamp was taken at; picking one anyway would put a specific SHA into a sentence that
  cannot justify it, which is the defect `APLAN024` shipped. At none, several, or a blank ref the
  instruction keeps its previous wording.

  The p00020 pilot proposed reporting the diff itself and called it the cheapest of its three
  options. It is the most valuable and not the cheapest: `trace_fingerprints` stores a hash and not
  the content, so there is nothing to diff against, and a real diff means keeping a copy of every
  traced section in every task file. Printing the command costs no storage and keeps the rule
  hermetic, which is what fits `check` to a pre-commit hook.

  The audit found the command naming a path that could match nothing: it used the trace's own
  spelling, and `traces_root` is configurable, so under `traces_root: docs` a trace of `d.md#intro`
  is `docs/d.md` from the repository and `git diff <sha> -- d.md` matches nothing and exits *zero*.
  A command that looks like it worked and shows an empty diff is worse than one that fails, and
  worse than the prose it replaced, because prose does not promise to run. The path is joined onto
  the traces root now; the default root is `.`, so the common case is the plain relative path.

- **The message branches nothing pinned are swept, not met one audit at a time** (`A1-094`).
  `docs/rule-catalogue.md` pins what a rule says *as some scenario reaches it*, and the corpus
  supplies one scenario per rule, so a rule whose message interpolates its reason had only one of
  its sentences recorded. Three separate audits had found one branch each before anyone counted
  them. The sweep compares distinct message constructions per rule against the catalogue's
  wordings, and four branches now have scenarios: `APLAN010`'s `captured` that is present and not
  a date; `APLAN001`'s unquoted `: ` hint that names the offending field, as against a bare
  "invalid YAML frontmatter"; and `APLAN020`/`APLAN007`'s "its cut follows `M-s0`, which is
  `dropped`", the sentence a *sequenced* plan gets. 28 pinned wordings to 33, and the floor with
  them. What stays unpinned is listed in the task with the reason, and so is the method's blind
  spot: it counts constructions, so a rule that substitutes a reason into one string still reads as
  one sentence.

  A group-wide audit then found a fifth: `APLAN021` joins its reasons from a list, so
  `declares \`paths\`` is a distinct sentence, and the catalogue had been *recording its absence*
  rather than covering it, which is the honest-but-lazy option when the fixture is four lines. 34
  wordings now. The same pass removed a needle in the rule-test audit that matched nothing, and
  confirmed the two ADR sweeps are complementary rather than duplicated: the corpus one reads
  rendered diagnostics and so sees interpolated values, the source one reads literals and so sees
  branches no scenario reaches.

- **`--require`'s help cannot drift from the set it validates against** (`A1-093`). The flag reads
  five parts of `aplan facts`, and its help is a doc comment, so nothing but a gate makes the two
  agree. It once accepted output formats and evidence kinds while naming only commands: an adopter
  asserting `--require test` was doing something supported and undocumented, and one reading the
  help would not have known to try. That wording had already been corrected by an earlier audit;
  what was missing was anything stopping it recurring. `CAPABILITY_SOURCES` is now one list the
  validator iterates and a test holds the help to, so adding a sixth source fails until the help
  says so. Rendering the help *from* the list is not possible for a compile-time doc comment, and
  the task records that rather than implying more than shipped.

  The audit then found the CLI corpus reproducing the very defect it documents: `capability.trycmd`
  exercised three of the five kinds and its prose said "a config key, a rule code, or a command",
  leaving out output formats and evidence kinds, which is exactly the pair the original bug
  accepted while the help stayed silent. A corpus checking three would have passed on the day it
  shipped. All five are exercised now.

- **Every rule now has a test asserting what it *says*, and a gate that requires one** (`A1-090`).
  The existing audit proved each rule has a test; it never asked what that test checked, and
  thirteen of the twenty-five asserted only that a diagnostic with the right code came back. That
  is how `APLAN024` shipped naming the wrong commit: its test proved the warning appeared on a
  diluted citation, which was the interesting part, and nobody read the sentence. The new audit
  works per `#[test]` function rather than per crate, because every crate mentions `.message`
  somewhere and a whole-file check would pass for a rule whose own test asserts nothing of the
  kind. Each backfilled assertion names the *entity* the message must carry, the form the naming
  convention asks for: `APLAN020` must name the dropped blocker and not merely fire on the task
  waiting for it, `APLAN018` must name the section that drifted, `APLAN011` must quote the SHA it
  could not resolve.

  Backfilling found a fixture that could not have proved its own claim: `duplicate_id_is_an_error`
  built both tasks through a helper that puts every task at one path, so the message named that
  file whichever definition it came from, and the test could not tell "names the first definition"
  from "names whichever it happened to hold". That is the one-candidate blindness `AGENTS.md` warns
  about, in the fixture for the rule whose whole job is telling two files apart.

  Auditing the gate then found it believing its own documentation: it matched the bare word
  `message`, and the backfill had just added eight comments reading "the message must name ...",
  so a rule could be credited by a comment *about* asserting. `APLAN005` was passing that way, and
  the count went into the record as thirteen on the strength of it. It is fourteen. The matcher now
  requires field access, and line comments are stripped before the brace scan, because an
  apostrophe in ordinary prose ("the rule's message") opened a char literal that never closed and
  silently moved where a test body ended.

- **A convention in `AGENTS.md`: a diagnostic may name only what the predicate returned**
  (`A1-091`). When a rule judges a collection and reports one finding it has to choose what to
  name, and that choice is the defect that escaped this repository twice. `APLAN024` scored every
  cited commit, kept the least diluted, and formatted the *first* commit into the sentence: both
  are `evidence[i].reference`, both exist, and they differ exactly when a task cites more than one
  commit, which is the case the rule exists for. It reached the `p00025` pilot as a warning naming
  a commit the number beside it was not about. `APLAN026` did the same with families. The
  convention is to carry the subject with the verdict in one binding and interpolate only what you
  were handed, and it names the test form that follows: several candidates, with the offending one
  not first, since a single-item fixture cannot tell the two shapes apart.

  Every rule that reports one finding over a collection is now reviewed and the findings recorded
  in the task, including the ones already correct, so a later pass does not repeat the work. No
  predicate remains in the shape the convention forbids: both offenders were corrected when found,
  each has a multi-candidate regression test, and the sweep turned up no third. The honest limit is
  stated too: nothing mechanical enforces the convention, because "the subject came from the same
  binding as the verdict" is not observable at runtime. What is enforced is the test form.

### Changed

- **The audit over this tool's own words read 35 of its 286 consumer-facing strings** (`A1-087`).
  `A1-044` built a gate to keep `aplan`'s private decision numbers out of text addressed to
  somebody else's repository, and it harvested that text with a single pattern, `#[error(`. Every
  `anyhow::bail!` refusal a person meets when a command declines to act, every interop notice
  about what an importer dropped, every line `check` and `verify` write, every `agent_instruction`
  and every diagnostic message was read by nothing. That is most of what an adopter actually sees.
  Proven rather than argued: an `ADR-0013` planted in the `mv` refusal passed all eight audits in
  that file, which corrects the claim, made when the leak was removed by hand in `A1-084`, that
  the gate would have caught it.

  One harvester now covers seven surfaces, taking each call's balanced-paren span and reading the
  string literals in it: 35 `error`, 39 `refusal`, 13 `context`, 30 `notice`, 41 `instruction`,
  39 `diagnostic` and 102 `output`. Each kind carries its own floor, since one total would hide a
  single pattern going to zero. The adversarial pass supplied the seventh: `.context(...)` and
  `.with_context(...)`, 18 call sites whose text anyhow prints above the error it wraps, were
  missing from the first version of the very task that exists to stop a surface being missed. It
  also found that an unbalanceable span was skipped *silently*, so a message could leave the sweep
  with every audit still green; unreadable call sites now fail by name. `#[cfg(test)]` modules are stripped first, because the only ADR literals in
  `crates/*/src` are importer fixtures in `roadmap_json.rs`: a sweep over every literal would have
  reported a leak while pointing at a string nobody is shown, and a false positive is how a gate
  gets switched off. Each surface was verified by planting a citation in real source and watching
  it fail, including `#[error(` as a regression check on the one that already worked.

- **The docs gate asked whether a rule was mentioned, not whether it was documented**
  (`A1-092`). It tested `HLD.md.contains(code)`: a substring, anywhere in the file, and it never
  consulted `README.md` for a rule at all. A rule named once in a paragraph satisfied it forever,
  so everything about *how* a rule was documented went unchecked, and two defects were sitting in
  the tree it could not see. `HLD.md` carried the `APLAN027` row **twice**, identical, because
  `contains` is true for one row and for two. `README.md` said "`APLAN009`, `APLAN021` and
  `APLAN023` are warnings" directly above a table marking seven of its own rows warning: the
  sentence was true when written, four warnings shipped after it, and it sat one clause away from
  the boast that this README "does not hand-count them, which is the drift the tool exists to
  catch". Both are fixed, and the gate now reads the tables' *structure*: one row per rule,
  severity agreeing with the code that declares it, every rule mentioned somewhere in the README,
  every README warning marker matching `facts`, and an HLD CLI row claiming a command `exists`
  naming one that does. Each of the seven checks was verified by planting the defect it exists to
  catch and watching it fail.

  **The adversarial pass found the same defect one layer down.** The first version strengthened
  the *rule* half of this gate and left the *command* half on the identical anywhere-in-the-file
  `contains` test it had just removed as useless. It passed, and `aplan drop`, `aplan mv` and
  `aplan tasks` were **absent from the README's `## Commands` list entirely**, satisfying it
  through incidental mentions in prose. The paragraph under that list also still said `add`,
  `close`, `init` and `import` "are the only commands that write", which stopped being true when
  `A1-077` shipped the mutations. All four are fixed, and a command must now appear in the list
  and hold a row in the HLD CLI table.

  **Every invocation the README documents is now handed to the real clap parser.** Two of the
  three command lines added while fixing the above were wrong: `aplan mv P1-003 --to M-v0.7` named
  a flag that does not exist (it is `--cut`), and `drop --cut` is a boolean, not a flag taking a
  cut id. A reader who copies advice that fails on first use has no way to tell which half of the
  documentation to trust afterward. Tables inside code fences are also no longer read as tables,
  so a document *showing* this format cannot break the gate.

  Filed as "generate the tables", by analogy with `schemas/v1/*.json` and `ROADMAP.status.md`, and
  changed after reading them: **none of the three is purely derived.** The HLD catalog documents
  `APLAN016` and `APLAN019`, which are planned and absent from `facts`, and cites the task that
  shipped each rule; the HLD CLI table carries planned commands with a status column; the README
  command block is curated examples with arguments; and the README rule table is the *always-on*
  set on purpose, with the three opt-in rules covered in their own paragraph below it. Generating
  any of them would have deleted real content in order to gate a fraction of it.

- **`aplan verify` now runs the hermetic rules too** (`A1-058`), so it is a superset of `check`
  rather than a sibling. It used to resolve evidence, skip every `check` rule, and print
  `ok: N tasks, 0 warning(s)` on a plan `check` rejected: a claim about the subset of rules it
  ran, worded as a claim about the plan. An adopter wiring one command into CI reaches for this
  one, because the documentation calls it the deeper level of the ladder, and got a permanently
  green build with nothing in the output hinting a second command existed. Found in the `p00025`
  pilot while probing `APLAN023`, which looked unimplemented because the probe ran under
  `verify`. The two commands now always agree on whether a plan passes; `check` remains the
  hermetic one, which is what fits it to a pre-commit hook. `verify` also gained `check`'s
  hermetic flags (`--no-traces`, `--no-require-effort`, `--traces-root`), without which the
  agreement held only for the default invocation: `check --no-traces` passed a plan `verify`
  then failed on the trace it had been told to ignore.

- **The documented `test.run` command was one cargo rejects**, and is corrected (found while
  building `A1-054`'s fixture). `cargo test --exact {test}` fails with "unexpected argument
  '--exact'": it is a libtest flag and has to follow `--`. It appeared twice in `README.md` and
  in the `.aplan.yml` scaffold `aplan init` writes into every consumer repository, so an adopter
  who copied it had a `run` that could not execute, silently, until their first
  `verify --execute`. A test did guard the scaffold, by asserting it matched a fixture the
  parsing tests use, so the two agreed with each other and both were wrong; nothing had ever
  handed the argv to cargo.

- **The CI runner no longer deregisters itself on an ordinary restart** (`A1-053`). The SIGTERM
  handler ran `config.sh remove`, so `podman restart`, `podman stop` and host shutdown each
  deleted the server-side registration; the config volume then restored credentials the server
  had discarded and `--restart unless-stopped` looped on them. Found in the `p00025` pilot, which
  copied this runner pattern: one restart minutes after provisioning produced fifteen container
  restarts and zero registered runners. It had never bitten here because `config.sh remove` needs
  a still-valid registration token and `|| true` swallows the failure once it expires, so the
  window is the first hour after any provision or image rebuild. The handler now stops the agent,
  deregistration is an explicit `teardown.sh --purge` through the API, and credentials the server
  has deleted are discarded so the next start re-registers rather than looping. Verified on this
  repository's own runner: the registration survives a restart with the same id and no restart
  loop. Anyone running this pattern should rebuild their runner image.

- **`import` refuses a batch that would land half-written** (found reviewing `A1-031`). A
  source declaring one id twice wrote the first task and then failed on the second with a
  message that read as though the file had pre-existed; a source with an empty title wrote a
  task `check` rejected on sight. Both checks now run before the first file is written, and in
  the shared write path, so every flavor gets them.

- **A non-finite `effort_days` is now a schema error, and so is an empty `title`** (found
  reviewing `A1-029`). `effort_days: .inf` and `.nan` passed `check` and then poisoned every
  rollup that adds the field up, which is `summary`'s phase totals and the critical path. The
  cause was a disagreement between the two halves of loading: frontmatter is schema-checked as
  a `serde_json::Value`, JSON has no infinity or NaN, so both render as `null` and satisfy
  `["number", "null"]`, while the typed deserialisation reads the same YAML and yields
  `Some(f64::INFINITY)`. Caught in the loader, so it covers hand-written files and not just
  what `aplan add` writes. `title` gains `minLength: 1` beside it: a task nobody can name is a
  blank row in `ready`, `roadmap`, and the generated companion, and no plan means it.

- **`aplan` no longer prints its own ADR numbers into a consumer's repository** (`A1-044`,
  found by the p0000004 pilot, item 7). "ADR-0009" resolves here and carries the reasoning;
  in an adopting repository it is a citation to a decision record that does not exist, and
  the reader cannot tell whether it was deleted or never existed. Four sites, in order of how
  long they persist: `import`'s provenance body, which is **written into the consumer's task
  files and committed there, never regenerated**; the generated `ROADMAP.status.md` footer;
  the `roadmap` and `import` `--help` text; and `aplan facts`, which reuses the same strings.
  Each keeps its guarantee and loses only the number ("gated by `aplan verify` in CI" says
  what a consumer needs). The decision numbers now live in `//` comments beside the strings,
  where a contributor reading the source still finds them.
  - **Adopters will see one diff.** A committed `ROADMAP.status.md` generated by an older
    `aplan` disagrees with this version's output by that footer line, so a roadmap drift gate
    goes red until the file is regenerated. Regenerating is the fix; nothing else changes.
  - Kept honest by `crates/aplan/tests/consumer_facing.rs`, which walks the whole clap command
    tree (so a subcommand added later is covered without anyone remembering the file exists)
    and checks the *rendered* artifacts rather than source text, because help comes from doc
    comments and a task body is assembled at runtime, so a grep for a string literal would
    miss both. Eight surfaces in all: the four above, plus the `.aplan.yml` scaffold `init`
    commits into the consumer's repository, the MCP tool descriptions an agent reads while
    working there, error messages, and every rule's diagnostic text (gated where the scenario
    corpus lives, so a rule added later is covered by construction).

### Added

- **`check` prints the fix, not only the finding** (`A1-086`). Every diagnostic has carried an
  `agent_instruction`, the specific thing to do about it, and only `--format agent` printed it. The
  first pilot hit an `APLAN018` drift warning, saw a finding with no remedy, and concluded in its
  log that its only options were deleting the fingerprint, which disables that guard permanently,
  or hand-writing a hash: `aplan close --restamp` had existed for ten days and that very field
  named it. The remedy was computed, written, and withheld from the only reader who needed it. It
  now prints as a `fix:` line, once per rule code rather than once per finding, so a 75-finding
  report gains three lines and a footer saying how many findings carry their own fix, which
  `--format agent` still prints in full. That format is unchanged.

- **`drop --cut` and `mv` stop confusing a sequencing edge with a membership claim** (`A1-084`),
  and `APLAN027` reports the state they produced. `A1-073` put the cut a plan follows into a
  milestone's `blocked_by`, and `A1-077`'s mutations read that whole list as members, so dropping
  one cut cancelled the cut before it while leaving that cut's work live, and moving a milestone
  silently deleted the sequencing so `ready` began offering work again that the order had held
  back. `check` called both results clean. The roadmap has distinguished the two edges since
  ADR-0008; the mutations now do, `mv` refuses a milestone outright and refuses to empty a cut at
  the point of the action rather than leaving `APLAN021` to report it afterwards, and `APLAN027`
  warns when a cancelled cut still lists live work, which a hand-edit or `drop <milestone>` can
  reach without any bug. **Found by the fourth pilot**, whose plan cancels four cuts at once, which
  is this command on this shape; its test fixture now sequences its cuts, which is what nothing did
  before, and a refusal-only corpus pins the messages, since a fixture a writing command runs
  against is one it rewrites.

- **The upgrade guide says what a change lets you stop doing** (`A1-083`), which is not the same
  list as what it adds. Twice an adopter kept a workaround after the tool had made it unnecessary:
  one generated 81 cross-cut edges that `A1-073` reduced to seven, and one kept a `paths`
  accommodation for `APLAN025` until `A1-069`'s guide named its cost. Neither had reason to read
  the entry for a feature they believed they did not need. `docs/upgrading.md` now carries a "What
  you can stop doing" section with both cases and the reasoning, its change table is current again,
  and `AGENTS.md` records the convention so the next change that obsoletes a workaround says so
  where it ships rather than waiting to be noticed.

- **`aplan facts --require`, so a consumer asserts a capability instead of grepping for one**
  (`A1-082`). `A1-060` published the fact and stopped there, and the first adopter wrote thirty
  lines of shell to use it: `aplan facts | grep -q '"key"'`, once per key, plus a separate probe
  for whether the field existed at all. That is a JSON contract matched with a substring search, in
  every repository that stages a binary. One repeatable flag now asserts config keys, rule codes
  and commands alike, names every missing capability rather than the first, and keeps the two
  failures apart: a build that answers no exits 1 and says what it lacks, while a build too old to
  be asked has no such flag and exits 2. Conflating those is precisely what the adopter's first
  draft did, telling an operator a binary lacked a key it supports.

- **An importer stops discarding the evidence a source brought** (`A1-080`). `p0000019`'s
  `roadmap.json` cites the documents proving 16 of its 25 milestones, `path#anchor` entries
  pointing at decision records, and every one was thrown away with the report
  ``dropped: `milestones[...].evidence` is not held by the task model``. That reason was false:
  the model holds it, in exactly that shape, and the same importer already mapped `artifact` to a
  `traces` entry. For a tool whose thesis is that a status is a claim and evidence settles it,
  importing a source that brought evidence, discarding it, marking the task `todo` and telling the
  adopter to go and find some was the sharpest loss it had. Those citations now become `traces`,
  so `APLAN008` checks the anchor resolves on every run, which is a stronger guarantee than the
  source had; `file` evidence was refused because manufacturing evidence during an import is what
  ADR-0012 declined to do. The report names the mapping and warns that an unresolvable anchor will
  fail the first `check`.

- **A cancelled member no longer makes its cut permanently incompletable** (`A1-081`). Hit while
  withdrawing a task from this repository's own plan, and there was no way out: leaving a cancelled
  task in the cut it belonged to raised `APLAN020`, and taking it out raised `APLAN022`, so a plan
  could not cancel work that belonged to a cut at all. The tool already disagreed with itself,
  since `roadmap` has always excluded a dropped member from a cut's rollup so the counts reconcile.
  A `dropped` member is now treated as satisfied **when the dependent is a milestone**, and only
  then: a live task blocked by cancelled work genuinely cannot start, which is what `APLAN020` is
  for and is unchanged, and a cancelled *predecessor cut* still blocks because a milestone in
  another milestone's `blocked_by` is a sequencing edge rather than a member. The rule now has one
  implementation, `Graph::blocks`, shared by `check`, `close`, `why`, `ready` and the roadmap
  rollup: relaxing it in the validator alone left `close` refusing to close the very cut this was
  meant to unblock.

- **`roadmap.group_by`, so a project with a second grouping level gets its view back** (`A1-078`).
  The one real cost of the nesting decision, and the argument a second modelled level would
  otherwise have won on: `roadmap` rendered a cut as a flat block of 17 to 24 tasks, where the
  pilot's own document rendered the same tasks under their epic headings and was easier to read
  for it, so adopting `aplan` made that view worse. Naming a declared tag family now groups each
  cut under its headings, in the order the repository declared them, with an explicit `(no group)`
  section for tasks carrying none, since putting them under a heading they do not have would be a
  lie and dropping them would be worse. The family must be one `tag_families` declares, or a typo
  could invent a heading. Grouping is additive: the flat member list is unchanged and a plan that
  declares nothing renders byte-identically, which this repository's own gated `ROADMAP.status.md`
  confirms.

- **The nesting question is decided: one modelled grouping level, and a second is a declared tag
  family** (`A1-032`, ADR-0014). Two adopted projects nest a level deeper than the cut, and this
  decision waited for the second so it could be taken against real files. Both surprised it.
  `p0000019`'s nesting is nominal, since its 21 phases carry no items and one workstream holds all
  25 milestones, and the distinction it actually uses is five state fields that no nesting would
  carry because ADR-0012 already made a stage a tag. `p00020`'s nesting is real, but of the four
  things it wanted from a second level, two shipped while the question waited: `APLAN026` makes a
  tag vocabulary checkable and `aplan tasks` makes a rollup by one a `jq` line. What remains is a
  view (`A1-078`), and an importer that records the container it flattened rather than discarding
  it (`A1-079`), which the decision found by importing the real file instead of describing it.
  Nothing in the model changes and no existing plan changes meaning.

- **`aplan mv` and `aplan drop`, the mutations that maintain an invariant** (`A1-077`). What
  mutated a plan was `add`, `close`, `import` and `init`; everything else was opening the file.
  At 33 tasks that is a footnote, and at 105 it is the dominant cost: moving a task between cuts
  meant editing two milestone files and hoping, and dropping a cut meant forty hand edits, which
  is exactly what the pilot's own kill criterion required. `mv` edits both milestones or neither;
  `drop` records a cancellation rather than deleting the record of it, and `--cut` takes a cut's
  members and then the milestone. Both resolve everything before writing anything, so one bad id
  leaves no file touched, and both parse back what they wrote before saving it, on `close`'s
  standard that a rewrite the tool cannot read again is a corrupted file. Terminal states are
  refused: `dropped` records a decision and `done` records evidence, and rewriting either would
  launder it. **A status command was deliberately not built**: hand-editing an estimate, a glob or
  a live status breaks no invariant `check` does not catch on the next run, and a command for it
  would be write surface with nothing to maintain. Neither is on the MCP surface, because dropping
  work is not a claim any gate can adjudicate.

- **`APLAN026`, a declared tag vocabulary, so a typo in one is not silent** (`A1-076`). A second
  grouping level always ends up in tags, because tags are what exists. ADR-0008 rejected them for
  cut membership precisely because a typo is invisible, and then a pilot put 22 epics there anyway
  with a build/measure/review taxonomy beside them: one validated grouping level and one
  unvalidated one in the same frontmatter, indistinguishable by eye. `tag_families` in `.aplan.yml`
  declares the vocabulary, which makes a claim the repository already believes checkable without
  making tags into a model, so it does not pre-empt the nesting question. A tag carrying a family's
  prefix that the family does not list is reported, and so is one a single edit from a member,
  which is what catches `epci-k` where a prefix rule sees nothing. Tags outside every declared
  family stay legal and silent, and a plan declaring nothing behaves exactly as before. A warning:
  an undeclared tag in a family's shape is as likely to be a member nobody wrote down as a mistake.

- **`aplan tasks`, the plan as data** (`A1-075`). `summary` rolls up by phase and by nothing else,
  so a project that types its work another way, one pilot marks every task build, measure or
  review because 20 of its 105.5 days do not shrink by working faster, had to keep the numbers by
  hand: a second copy of exactly the fact `aplan` owns, and it was wrong twice during that
  pilot's own audit. A `--group-by` flag would have answered one question and invited the next, so
  this emits the tasks and every other rollup is a `jq` line. The document resolves what a
  consumer would otherwise re-derive: the cut a task belongs to, and whether it is ready by the
  same code `aplan ready` uses, so two answers to one question are impossible. `status` is carried
  rather than filtered, so cancelled work is the consumer's call; `effort_days` is `null` and not
  `0` when there is no estimate, because an unestimated task is not a free one. A new command, so
  `check --format json` keeps its contract, and `schemas/v1/tasks.json` is derived and gated like
  the others.

- **`summary` stops reporting cancelled work as scope** (`A1-074`). `roadmap` counted live tasks
  and `summary` counted every task, so the two disagreed about what "the plan" is. On a plan whose
  headline feature is a kill criterion, where a decision gate can cancel four cuts and `dropped` is
  chosen over deletion so the measurement that rejected the work stays findable, the command an
  operator reaches for first reported pre-cancellation scope forever: 98 tasks and 105.5 days where
  39 tasks and 46.5 days had been cancelled. `summary` now counts what can still happen and names
  the cancellation on its own line rather than omitting it, since a total that silently drops work
  is how this happened the first time. **The JSON is now `aplan/summary/v2`**: `total_tasks` and
  `total_effort_days` kept their names and changed their meaning, and a field that changes its
  number while keeping its spelling is how a consumer reads a wrong answer confidently. A dropped
  cut also no longer renders `crit -0.0d`, which came from `Sum` for floats folding from `-0.0`.

- **Cut sequencing, and a cut labelled by its own tag** (`A1-073`, `A1-037`, ADR-0013). A milestone
  is `blocked_by` its members, so members cannot be blocked by their own milestone and nothing said
  one cut follows another. On a strictly sequential plan that made two commands wrong rather than
  unhelpful: `ready` offered work from three cuts ahead, and `critical-path` reported a 105-day
  plan as eleven days. A milestone listing another milestone in `blocked_by` now sequences the two
  cuts and the later cut's members inherit the edge, so seven declarations express what 81
  hand-maintained ones did. **You can stop generating them**: an adopter carrying cross-cut edges
  for this can delete every one, and `ready` and `critical-path` report exactly what they did.
  **No new field**: that edge was already legal and already excluded from
  cut membership, it simply never reached the members. A plan whose milestones do not block one
  another, including this one, is unchanged. Cuts are also labelled by their own tag now rather
  than rendering `?` when it is not a `vN.N` version, and non-version labels order naturally, so
  `s2` precedes `s10`. Two pilots reported the `?`, and the second inverted the reading of the
  first: a release version is the unusual case among the projects adopting.

- **A file that fails to parse no longer manufactures errors about other files** (`A1-072`). A
  task whose file did not load is absent from the graph, so every `blocked_by` pointing at it
  reported an unknown task. It is not unknown, its file is right there, and the cascade scaled
  with how well connected the plan was: in the fourth pilot one unquoted `: ` in 33 titles
  produced 85 errors of which **52 were phantoms**, and the real fault, a punctuation rule, was
  named in none of them. Edges into an id whose file failed to load are now dropped, the way a
  compiler drops unresolved imports from a module that would not parse; only that id's edges are
  affected, so a genuinely dangling reference still reports. `APLAN001` also names the field and
  the fix on the unquoted-colon case, which is the most common authoring mistake there is and the
  one the raw YAML message explains least.

- **`test.covers_by_test`, so a repository-wide invariant is not read as borrowed proof**
  (`A1-069`). `APLAN024` tells an adopter a commit citation proves little and to cite a test;
  `APLAN025` then told them that test was borrowed from another component. Both fired on the same
  eleven tasks in the first adopter, in that order, within an hour. Neither rule is wrong about
  what it measures: `APLAN025` modelled a test as living beside the code it tests, which holds for
  a unit test and fails for exactly the class `APLAN024` asks for, an invariant that lives where
  the harness can read the tree and asserts a fact about a directory it is not part of. A runner
  can now say a named subset of its tests covers something else, and anything unmatched falls back
  to `covers` then `dir`, so the case the rule was built for still fires from the same runner. The
  workaround it replaces was not free: naming the test's own file in each task's `paths` also told
  `APLAN012` a commit touching only that file was relevant to all of them, and told `APLAN024`
  those paths were related.

- **A build from `main` no longer claims to be a release, and `facts` reports `config_keys`**
  (`A1-060`). `[workspace.package].version` was not bumped after `v0.2.0` shipped, so every commit
  since reported the released version, and a consumer pinning a staged binary with
  `aplan --version | grep -qF 'aplan 0.2.0'` pinned nothing: the check passed for a build that
  could not run their gate. `main` now carries `0.3.0-dev`, and `ci/scripts/release.sh` refuses to
  tag while a pre-release suffix is set, so releasing is an explicit bump rather than a mismatch
  error to decode. Because a version is a name and not a capability, `aplan facts` now lists
  `config_keys`, every key this build accepts, dotted (`test.covers`,
  `evidence.require_min_strength`) and derived from the schema the file is validated against, so
  an image build can assert what it depends on instead of what the binary is called.

- **[`docs/upgrading.md`](docs/upgrading.md), for adopters who stage the binary** (`A1-061`).
  Every build from `main` reports `0.2.0`, so a repository needing an unreleased feature builds
  its own, and nothing said how. It gives the containerised musl build (verified, and static,
  which the runner image requires), names the three traps on the direct route, and leads with the
  ordering that matters most: upgrade the binary *before* adding new config keys, because an
  unknown key fails the whole file and the gate stops running rather than losing one feature. It
  also tabulates what each recent change reports on a plan you have not touched, since two of
  them change existing output. Gated by `check-docs` like `ROADMAP.md` and the pilot logs.

- **`test.covers`, what a runner's tests cover rather than where it runs** (`A1-065`), which is
  what makes `APLAN025` usable. Reading `dir` alone, that rule could fire on no real repository:
  a runner selecting a component with `--manifest-path`, an npm filter, or `--rootdir` still runs
  at the repository root, which is where the lockfile is. `covers` takes precedence over `dir`,
  an empty one is a config error, and a runner declaring neither still makes no locality claim.
  Annotating the first repository's five runners immediately produced a false positive, a task
  citing two tests in the component it changed plus a cross-component integration test, so
  `APLAN025` now judges a task's test citations together: one test covering the task's own code is
  proof and the rest are context, the reasoning `APLAN024` already used. **You can stop adding a
  proving file to a task's `paths`** to quiet `APLAN025`: that accommodation also tells `APLAN012`
  a commit touching only that file is relevant, and `APLAN024` that those paths are related.

- **`APLAN025`, a cited test that belongs to another part of the repository** (`A1-063`). `test` is
  the strongest evidence kind here and the one `require_min_strength` steers repositories toward,
  yet it was the only kind whose *relevance* nothing checked: `APLAN012` binds a commit to the
  task's `paths` and `APLAN015` binds a file, while a test only had to exist somewhere. When one
  runner lists the cited test and that runner declares a `dir`, a task whose `paths` never reach
  that directory is warned about, in either nesting direction; a glob that could match at any depth
  counts as reaching everywhere, so the rule reports mismatches it can demonstrate rather than ones
  it suspects. A runner without a `dir` makes no locality claim and stays silent, which is every
  single-runner config. **It fires on no pilot as they are configured today**: `p00025`'s five
  runners each select a component with `--manifest-path` while running at the repository root, so
  `dir`, which records where a runner runs rather than what it covers, is absent everywhere.
  `A1-065` carries that gap.

- **`APLAN024`, evidence that is about the task rather than mostly about other work** (`A1-062`),
  the second rule an outside pilot produced. `APLAN012` asks whether a cited commit touches the
  task's `paths` at all, which a large enough commit satisfies by accident: `p00025` measured 31 of
  64 citations pointing at commits touching more than fifty files, 16 of them the same 124-file
  scaffold, and for those `APLAN012` could not have rejected anything. The measure is the count of
  paths a commit touches *outside* the task's `paths`, so a large change wholly inside them is
  focused and a small one cannot swamp anything; `evidence.max_unrelated_files` tunes it (default
  50, `0` disables). Checked once per `done` task rather than per citation, so it names the least
  diluted commit it counted and no guard upstream of it can suppress the finding. A warning, on
  `APLAN023`'s reasoning: bootstrapping from a scaffold is legitimate and the repository is the
  judge of it, but it must not read the same as a focused citation. Silent when the task also cites
  a test, since a passing test says what a diff cannot. It found a true positive in this
  repository's own plan on first run, and reports 11 on `p00025`, the number that repository's
  audit had reached by hand.

- **`--deny-warnings`, and `deny_warnings` in `.aplan.yml`** (`A1-049`), for a repository whose
  policy is that every gate either passes or blocks. Warnings stay advisory by default and
  should: `APLAN023` warns rather than errors precisely because a citation may be honest and the
  repository is the judge. This is how a repository says it would rather be the judge in the
  strict direction too. `--no-deny-warnings` relaxes a repository policy for one run, so the
  flag beats the file in both directions, and asking for both at once is a usage error.
  - **The exit code does not change.** A denied warning is still `1`, "at least one finding",
    rather than a third code every consumer's CI would have to learn (ADR-0007).
  - The reason is printed to stderr, so a `--format json` consumer's stdout stays one document
    and the human output does not read `ok:` and then fail without saying why.
  - `verify` honours the same policy through the same code path, though today it has no
    warning-severity rule to deny: `APLAN011` to `APLAN015` are all errors, and the first
    warning there is `APLAN016` (`A1-041`).
  - Written because `p00025`'s constitution forbids a warn-only tier, so adopting `aplan` there
    would have been constitutionally inconsistent on day one over a missing flag
    (ADOPTION-ANALYSIS section 7.5).

- **An intra-doc link gate** (`A1-048`). `cargo doc --workspace --no-deps` under
  `RUSTDOCFLAGS=-D warnings` now runs in CI and in the local preflight bundle. A broken
  `[`Type`]` link compiles, passes clippy, passes the tests, and is dead on the rendered page,
  so rustdoc is the only thing that sees it: a first run found seven of them across four
  crates, each dating from a rename every other gate was happy with.

- **`aplan import --format roadmap-json`** (`A1-031`), reading a hand-maintained status
  contract: phases, plus workstreams whose milestones carry a `depends_on` graph.
  - **The graph is the point.** `depends_on` becomes `blocked_by`, so a reference to a
    milestone that does not exist is `APLAN003` and a cycle is `APLAN004`. The source's own
    linter proves identifiers resolve to files and says nothing about the graph; p0000019's
    real file (26 milestones, 49 edges) imports and validates clean, which is a negative
    result worth recording as one.
  - **The status vocabulary is one table, printed in the report**: `not_started` to `todo`,
    `in_progress`/`blocked`/`dropped` carried across, and `done` to `todo` carrying
    `roadmap-json:done`, because a hand-maintained `done` is the claim this tool exists to
    doubt. An unknown word is kept as a tag and flagged, never guessed.
  - **A workstream becomes a cut** (ADR-0008): a `milestone`-tagged task over its own
    milestones, so `APLAN007` refuses to let it be `done` while a member is not. Its
    `artifact` becomes a `traces` entry that `check` resolves.
  - **`SPEC-`/`ADR-` references in prose stay prose and are named**: the file gives ids, not
    paths, and a fabricated trace breaks the plan whenever the guess is wrong. `adrs` and
    `specs` are reference records rather than work, so they are reported with their counts
    rather than imported.

- **`aplan import --format checklist`** (`A1-030`), reading a markdown roadmap of `- [ ]`
  items under `##` headings: the commonest planning shape there is, and what the first pilot
  kept before it adopted `aplan`. A wrapped item is joined into one title, the heading
  becomes a tag, and the prose under that heading is carried into each task's body, which is
  what importing a list of one-liners would otherwise discard. The parser is line-based on
  purpose: the shape is a convention, not a language, and a markdown parser would bring an
  opinion about every other construct in a file it should pass through untouched.
  - **A ticked box is a claim**, so it imports `todo` tagged `checklist:checked` and the
    report names what would close it; there is nothing in a checkbox to resolve, so `done`
    would be `aplan` asserting what it cannot check. `--implemented-as done` opts into the
    weaker record with `--as-of`, as for a KEP.
  - **A checklist has no ids**, so the importer generates them (`--id-prefix`, default `T`)
    and says it did. Inventing a field deserves the same disclosure as dropping one.
  - **Order is not a dependency**: nothing is written to `blocked_by`, because inferring one
    from position would be a guess recorded as a fact. Nested items become tasks of their own
    and the flattening is reported.
  - Validated on the real file: p0000004's pre-`aplan` roadmap imports to 18 tasks that pass
    `check`. `Import` now carries a body per task, so the CLI writes what the source decided
    rather than a flavor-specific template it hard-coded.

- **`aplan add`** (`A1-029`), the first thing every adopter needs and the friction the first
  pilot ranked above all others: thirty-three task files, hand-authored, before any of the
  tool's value arrives. It scaffolds one task with the fields `check` needs and a body to fill
  in, and its whole design is what it refuses to do.
  - **It continues the plan's own id convention rather than imposing one.** `A1-046` gives
    `A1-047`, `AL1-07` gives `AL1-08`, a family that has grown to four digits keeps them, and
    a phase with no tasks yet takes the established prefix and starts at 1. Milestones are
    excluded by tag, so `M-v0.7` never donates its shape to a task. An **empty plan is refused**
    rather than given an invented prefix: `--id` sets the first one, and every later `add`
    follows whatever convention it established.
  - **It fills in nothing it cannot know.** No estimate, no evidence, no status but `todo`. It
    says what is missing instead: no `effort_days` (`APLAN009` will warn), no `paths` (a commit
    cited later would prove only that it exists, `APLAN023`), and, when the plan requires cut
    membership, that the task needs `--cut` or a `cross-cutting` tag or `check` will fail.
    That last note is the difference between a scaffold and a trap.
  - **It refuses before it writes**: an id that already exists (by id, not by path, since
    identity is the `id` field), a `--blocked-by` naming a task that does not exist, and a
    `--cut` that is not a milestone or is already settled. `--dry-run` prints the whole result
    and writes nothing.
  - `--cut` places the new task in a milestone's `blocked_by`, reusing `import`'s machinery, so
    a task can be authored and placed in one command.
  - The collision question it leaves open (two authors generating the same id offline, caught
    by `APLAN002` at the merge) is `A1-047`, and HLD Section 4 no longer describes an
    `id_strategy` config key that does not exist.

- **A testing contract, and two gates under it** (`A1-046`). `cargo xtask coverage --check`
  holds every crate to a committed per-crate line-coverage floor (`coverage.json`), ratcheted
  in both directions: below the floor fails, and rising two points above it fails asking for
  the gain to be banked, so a floor nobody raises cannot quietly stop being one. Regenerating
  prints every crate's measured percentage and refuses to write a floor downward without
  `--allow-regression`, because the command that banks a gain would otherwise be the one that
  erases a regression. A second
  audit (`crates/aplan-e2e/tests/rule_tests.rs`) requires every rule in `Rule::ALL` to have a
  unit test in the crate that raises it, not only the end-to-end fires-and-silent pair, with
  the registry guard in `diagnostic.rs` excluded because it names every code by construction.
  `AGENTS.md` and `CLAUDE.md` now exist and carry what neither gate can check. The honest
  framing is in all three places: `APLAN023` shipped with the required scenario pair and *full
  line coverage* of its rule and was still wrong three ways, so a coverage number is a floor
  and never evidence that the tests are thorough.

- **`APLAN023`, evidence that can actually be checked** (`A1-043`), the first rule produced by an
  outside pilot rather than by this repository's own reasoning. `APLAN012` gives a `commit`
  citation its meaning by requiring the diff to touch one of the task's `paths`; `paths` is
  optional, so a task without them has nothing to compare against and the same citation quietly
  degrades from "a commit that touched this work" to "a commit exists". p0000004 had seventeen of
  thirty-three tasks in that state while `check` reported zero warnings and `verify` reported
  evidence resolved, having written the rule into its own contributor docs correctly and drifted
  from it anyway, because nothing checked. A warning, always on, and hermetic (the question is
  whether the task declares `paths` at all, which needs no git). Scoped to `done` tasks: a `todo`
  often cannot know its paths yet, and warning there would teach people to write `paths: ["**"]`
  to quiet it, which is the bypass this must not encourage. Milestones are exempt, since
  `APLAN007` already refuses to mark one `done` while a member is undone.

## [0.2.0] - 2026-08-01

_Reconstructed at the v0.3.0 release: this section was never closed when the tag was cut, so
its bounds are the CHANGELOG.md the tag itself carries, not a later reading of it._

### Added

- **MCP server** (`A1-024`, `aplan mcp`), a deliberately small four-tool surface (`ready`,
  `why`, `check`, `close`) over stdio. `aplan_close` takes a task id and evidence
  citations and nothing else: its schema declares `additionalProperties: false`, so a score
  cannot even be transmitted, and a test fails if `score`, `confidence`, `completeness` or
  `percent` ever appears in the close path. Building it caught a real weakening: MCP `close`
  did not refuse a task with unfinished blockers, because that refusal lives in the CLI's
  command over the graph rather than in `aplan_core::close`, which only sees one file. Fixed
  with one shared predicate (`Graph::unmet_blockers`) rather than a second copy that could
  drift weaker. The tool surface is small on purpose so discovery and context remain bounded.
- **SARIF output** (`A1-023`), `aplan check --format sarif`: SARIF 2.1.0, the interchange
  format static-analysis dashboards already read, so a plan violation lands where a team
  already looks instead of only in a CI log. It is a pure formatter over the
  existing `Report`, adding no rule and no evidence, only a consumer. The rule catalog is
  built from `Rule::ALL` rather than from whatever fired in this run, so a result's `ruleId`
  always resolves to a described rule and a clean run still tells a dashboard what aplan
  checks. Validity is machine-checked, not asserted: the official SARIF 2.1.0 schema is
  vendored under `crates/aplan-output/tests/data/` and a hermetic test validates the document
  against it, because a gate that needs the network is a gate that fails when the network
  does. Deferred and recorded rather than dropped: a code-scanning *ingestion* self-test needs
  GitHub Advanced Security on a private repository, so it waits for the go-public decision.

## [0.1.0] - 2026-07-31

_Reconstructed at the v0.3.0 release: this section was never closed when the tag was cut, so
its bounds are the CHANGELOG.md the tag itself carries, not a later reading of it._

### Added

- **`aplan import`, and the `aplan-interop` crate** (`A1-019`, ADR-0012), the first import
  path: `--format kep` reads a Kubernetes `kep.yaml` into tasks. Proposal formats distinguish an accepted design from an implemented result. A `todo` task already represents accepted but unfinished work, so a proposal's stage becomes a **tag**
  (`kep:implementable`, `kep:stage:beta`, `sig:node`), never a new `Status` variant.
  - **Lossy, and says so.** Every field the task model cannot hold is reported, never dropped
    in silence (HLD Section 13). The unmodelled fields are collected by `#[serde(flatten)]`
    rather than a hand-written list, so a field added upstream *after* this code was written is
    still reported; the tests pin that against the real KEP template's seventeen fields.
  - **An imported claim is not proof.** A source that says `implemented` still arrives as a
    `todo` tagged `kep:implemented`, and the report names the fix. That default is what keeps
    import output valid under *any* evidence policy, including a strict `require_min_strength`
    that excludes `note`. `--implemented-as done` opts into the weaker record, requires an
    explicit `--as-of` date (`APLAN010` needs a real one and aplan does not invent provenance,
    the same reason `close` refuses to author note evidence), and then fails a strict gate on
    purpose.
  - **`--cut <milestone>`** places imported tasks in a release cut by appending to its
    `blocked_by`, editing that one line and leaving the rest byte-for-byte alone. It refuses a
    cut that is already `done` or `dropped` (adding an unfinished task to a settled cut would
    break `APLAN007`), validates the destination *before* writing anything so an import is
    all-or-nothing, and reads its own edit back before saving it.
  - **`--dry-run`** prints the whole result, including the placement, and writes nothing.
    Import never overwrites: an id that already exists is a refusal naming the file that holds
    it, checked by *id* rather than by path because identity is the `id` field (ADR-0010).
  - Documented in [`docs/importing-proposals.md`](docs/importing-proposals.md) with a worked
    round trip: a `kep.yaml`, the emitted task file, and a clean `check`.
- **`APLAN022`, membership totality** (`A1-018`), the advanced-integrity rule the A1-014 audit
  named. Opt-in via `.aplan.yml` `require_cut_membership`: every non-milestone task must be a
  member of exactly one cut (in exactly one milestone's `blocked_by`), or tagged
  `cross-cutting`. This closes the gap milestone hygiene (`APLAN021`) does not, a real task in
  *no* cut, so a closed milestone's `done` proves the cut is *exhaustively* complete, not just
  that its listed members are (the strong claim ADR-0008 deferred). `aplan`'s own plan opts in
  (`A1-006`, the house-style gate, is tagged `cross-cutting`; every other task is in exactly one
  cut), and the v0.7 cut (`M-v0.7`) now exists. The trycmd fixture plans are isolated from the
  repo's own policy by an `.aplan.yml` at their parent directory, so opt-in rules do not leak
  into sandboxed test plans that never opted in.
- **`gen-roadmap --check` and the roadmap self-hosting cutover** (`A1-016`), the last v0.5
  deliverable. `cargo xtask gen-roadmap` renders the plan (via the `aplan roadmap` library,
  not the built binary, so the `spec` CI job stays build-order-clean) into the committed
  `ROADMAP.status.md`, byte-identical to `aplan roadmap --format markdown`; `gen-roadmap
  --check` fails CI on any whole-file drift, the same gate shape as `gen-schema`, wired into
  the `spec` job. The cutover: the plan (`plan/tasks/`) is now the source of truth for what
  `aplan` builds and its status; ROADMAP.md becomes hand-authored rationale linking to the
  generated companion, its "single source of truth" header dropped (and HLD Section 18's);
  Section 10's cut/task/status table migrates to the companion, keeping only the HLD-section
  and rule-code cross-references the plan does not model; the milestone bodies drop their
  `## Theme` (rationale lives in Section 3, ADR-0009 Section 4). **`aplan` now generates and
  gates its own roadmap.**
- **`aplan roadmap`** (`A1-015`), the third v0.5 self-hosting deliverable: render the plan
  grouped by cut (the milestone meta-tasks), with per-cut rollups (done/total, ready,
  blocked) and the cut's critical path, plus the tasks in no cut. Human, `--format json`, and
  `--format markdown` (the committed `ROADMAP.status.md` companion, ADR-0009). Hermetic (it
  renders *cited* evidence, not resolved; `aplan verify` owns resolution) and advisory (never
  fails the build). Cuts order by their `v0.N` tag parsed *numerically*, so `v0.10` sorts
  after `v0.9`; the markdown uses non-aligned tables so a content change never re-pads a
  neighbouring cell, keeping the future `gen-roadmap --check` byte-exact. The model lives in
  `aplan-core` (`Graph::roadmap`, reusing a `critical_path_within` factored out of
  `critical-path`), rendered to a `String` by `aplan-output` so the xtask can call it.

- **Milestone modelling hardening** (`A1-017`), from an adversarial audit of `A1-014`:
  - **`APLAN021` (milestone hygiene, warning):** a task tagged `milestone` must have members
    and carry no `effort_days` or `paths`. The tag silently excludes a task from every
    work-view, so a real task mis-tagged `milestone`, or an empty cut, is now surfaced rather
    than swallowed, the "unvalidated tag" failure the model rejects for membership.
  - **Honest completeness.** A `done` milestone proves its *listed* members are done
    (`APLAN007`), not that the cut is exhaustive; the docs no longer overclaim, and
    membership totality is recorded as a planned rule (ROADMAP §0.7). ADR-0008 carries a
    dated correction note.
  - **`critical-path`** now treats a milestone as a zero-weight passthrough, so a task that
    depends on a whole cut (`blocked_by` a milestone) is no longer severed from the cut's
    work (an under-count fixed).
  - **End-to-end coverage** through the real loader (`crates/aplan-e2e/tests/milestones.rs`).
- **Milestone modelling** (`A1-014`), the second v0.5 self-hosting deliverable: the plan can
  now express its own roadmap. A cut (a release slice like v0.3) is an ordinary task tagged
  `milestone`, `blocked_by` the tasks in that cut, its body holding the cut's theme and exit
  criteria, its cut version a second tag (`v0.3`). No new field or edge type. Because
  membership is a `blocked_by` edge, a mistyped member fails `check` (`APLAN003`) and a
  milestone cannot be `done` while a *listed* member is undone (`APLAN007` plus resolved
  evidence), proof of the listed scope rather than a label (ADR-0008; membership totality is
  a planned rule). The work-views (`ready`, `critical-path`, `summary`,
  and `APLAN009`) exclude milestones, because a checkpoint is not work. aplan's own cuts
  v0.1 through v0.6 are now authored as milestone tasks (`M-v0.1` .. `M-v0.6`); authoring
  them surfaced that ROADMAP Section 10 had dropped `A1-012` from v0.4, exactly the omission
  generation prevents.
- **`aplan critical-path`** (`A1-013`), the first v0.5 self-hosting deliverable. It reports
  the longest `effort_days`-weighted chain of dependent tasks, the plan's spine, in
  execution order with the total, human and `--format json`. Advisory, never a gate (a plan
  is not wrong for being long). It is a longest-path DP over a deterministic topological
  sort (Kahn's algorithm, ties broken by id), so the same plan always yields the same path;
  dropped tasks are excluded (they are not going to happen), and a cyclic plan is reported,
  not miscomputed. The tested `toposort` it needs is the ordering primitive `aplan roadmap`
  and the rest of v0.5 build on.
- **Four-code exit contract** (`A1-012`, ADR-0007). The old three codes folded *usage* and
  *internal* into one `2`; now `0` clean, `1` at least one error, `2` a usage or
  configuration error (your input, your flags, your `.aplan.yml`), `3` an internal error (a
  bug in aplan). A consumer's CI can finally tell your fault from aplan's. `2` is the
  default (most CLI errors are the user's); only aplan's own invariant failure (a
  `close`/`restamp` that fails its round-trip self-check) exits `3`, tagged explicitly and
  unit-tested. A hard panic stays `101`. `facts.json` carries the four codes. Landed now,
  pre-1.0 with no consumers, when a contract break is free; no black-box case moved from
  `2`, so the change refines the contract without regressing tested behaviour.
- **`facts.json` and `aplan facts`** (`A1-011`), the "generate the counts, never hand-write
  them" gate (HLD Section 16, after this repo's own README once claimed "six invariants"
  over a table of ten). `facts.json` at the repo root is generated from the code's
  enumerable registries, the rules (with severity) from `Rule::ALL`, the output formats
  from `Format::ALL`, the evidence kinds from `EvidenceKind::ALL`, and the subcommands
  introspected from the clap command tree, and `aplan facts` prints it. `tests/facts.rs`
  pins the committed file to that output, so adding a rule, format, command, or kind
  without regenerating fails CI, exactly as `gen-schema --check` gates the schema. Two
  enablers came with it: rule **severity** is now a declared property (`Rule::severity()`,
  from the `define_rules!` table), with the coverage audit asserting each firing site
  honours it; and the **CLI moved into the `aplan` library** (a thin `main.rs` shim) so its
  command tree is introspectable and testable.
- **`APLAN018`, stale-`done` drift detection** (`A1-010`, opt-in via `detect_drift`).
  Adopting doorstop's fingerprint model (HLD Section 5.6): `aplan close` stamps a content
  fingerprint of each traced document *section* (the anchor's heading through the next
  same-or-higher heading), and `check` warns, advisory, when a closed task's traced section
  has changed since. The hash is whitespace-normalised so it survives reindentation and
  reflow, and content-addressed so it survives rebases and file moves; a stable,
  dependency-free FNV-1a keeps `aplan-core` hermetic. `aplan close --restamp <id>` re-takes
  the stamp after a deliberate re-review (doorstop's `clear`). A task with no stamp is
  unverifiable, not in violation, so adoption is incremental and false-positive-free. The
  heading/anchor logic moved to a shared `aplan_core::anchor` module. Pinned by the anchor
  unit tests (section extraction, reflow-invariance), the validator drift tests, the
  `close`/`restamp` tests, and a fires/silent pair in the coverage audit.
- **The house-style gate now also forbids invisible and dash-variant characters**:
  the non-ASCII dashes that stand in for a plain `-` (horizontal bar, minus sign,
  Unicode and non-breaking hyphens), and the zero-width and exotic-space characters
  (zero-width space, joiners, word joiner, BOM, narrow/thin/hair spaces) that hide in a
  diff and double as a trojan-source vector. The repo was already clean of all of them.
- **`aplan init`** (`A1-009`), scaffolds a starter `.aplan.yml` that lists every
  setting, commented, at its default, so the file is a valid no-op that documents
  itself: a menu, not an imposed policy (matching the opt-in ethos of the rules). It
  refuses to overwrite anything already at that path, a file, a directory, or a broken
  symlink (`symlink_metadata`, not `exists`), so it never clobbers a policy a repo has
  committed to, and it writes only on the success path. The scaffold lives in
  `aplan_config::SCAFFOLD`, and its runner and evidence examples are byte-identical to
  the shapes the `aplan-config` tests prove valid, so uncommenting them yields a valid
  file. Pinned by the `aplan-config` init tests (writes-then-loads-as-default, refuses
  to overwrite) and `tests/cmd/init.trycmd` (the overwrite guard).
- **`aplan close <id>`** (`A1-008`), the first command that writes. It sets a task
  `done` and appends an `evidence:` block in place, deriving the current `HEAD` commit
  when no `--evidence kind:ref` is given, so the proof a task is finished becomes a
  by-product of finishing it rather than a hand-typed SHA (the friction that kills
  adoption, RESEARCH §11). The rewrite is surgical: only the `status:` line changes and
  the block is appended, the rest of the frontmatter and the whole body are byte-for-byte
  preserved (a typed round-trip would reflow `1` -> `1.0` and flow lists to block lists).
  It refuses to author a false record, an unknown or still-blocked task (the `APLAN007`
  deadlock, caught at authoring time), one already `done` or already carrying evidence,
  or a `note` (which needs a `captured` date it cannot invent), and it never commits: the
  write lands in the working tree to stage yourself (surprise auto-commits are the
  top-reacted git-write hazard, RESEARCH §15). Pinned by the `aplan-core` close unit tests
  (surgical rewrite, quoting, every refusal) and `tests/cmd/close.trycmd` (the CLI guards).
- **`APLAN017`, minimum evidence strength** (opt-in): a repo can decree in
  `.aplan.yml` that a `done` task's evidence must include at least one of an accepted
  set of kinds (`evidence.require_min_strength: [commit, test, file]`), so a bare
  `note` no longer closes a task the way a `commit` or a `test` does. `evidence.
  exempt_tags` spares the work where a dated note genuinely is the best evidence there
  is (a phone call). Off unless configured; empty evidence stays `APLAN005`'s business,
  so the two never double-report. The kinds are typed (an invalid one is a parse
  error). Pinned by the validator tests, an `aplan-config` parse test, and a
  fires/silent pair in the coverage audit.
- **`aplan summary`** (`A1-002`): rolls the plan up by phase (tasks, how many are
  `done`, total estimated effort) with an overall ready count, as an aligned table
  or, under `--format json`, a stable `aplan/summary/v1` document. The rollup lives
  in `aplan_core::summary` and replaces the untested, unused `Graph::effort_by_phase`
  (removed). Pinned by the summary unit tests and `tests/cmd/summary.trycmd`.
- **`.aplan.yml` configuration** (`A1-001`), in a new `aplan-config` crate. A repo
  declares its policy once, in a schema-derived `Config`: `require_effort`,
  `traces_root`, `no_traces`, and the `test` runner (which folds in the minimal
  reader `A1-004` pulled forward, so there is one parser, not two). Three properties
  give it teeth: `schemas/v1/config.json` is derived from the type and gated by
  `gen-schema --check` (which now emits both schemas); an unknown key is a reported
  error, never silently ignored (`deny_unknown_fields`); and precedence is
  flag-over-file-over-default, so CI can pin a policy while a one-off run overrides
  it. The file is optional and is discovered by walking up from the plan (or repo)
  path, so `check` and `verify` converge on the same top-level file from anywhere in
  the tree. Pinned by the `aplan-config` tests (including discovery and the malformed
  / empty / unknown-key paths), the CLI precedence tests, and `tests/cmd/config.trycmd`.

- **`aplan verify`**, the verification ladder (HLD §5). Where `check` asks only
  whether evidence is *present* (rung L0, hermetic), `verify` opens the git
  repository and *resolves* it: a `commit` must resolve to a real commit
  (`APLAN011`, L1) whose diff touches at least one of the task's `paths`
  (`APLAN012`, L2, the claim-to-artifact binding that is the differentiator);
  a `file` must exist at the cited rev or at `HEAD` (`APLAN015`, L1); a `test`
  must be one the configured runner lists (`APLAN013`, L1), and under
  `aplan verify --execute` it must pass (`APLAN014`, L3). New `aplan-evidence`
  crate, with an `EvidenceResolver` trait (one impl per kind) and `git2`-backed
  commit/file resolvers plus a runner-backed test resolver. This completes
  `A1-004`, the evidence ladder.
- **The trust boundary** (HLD §5.5), the security-critical core of `--execute`.
  A task file can arrive in a fork pull request, so it may never name a command:
  it names only a *test*, and that name is substituted, as a whole argument, into
  a runner template that lives only in the trusted top-level `.aplan.yml`, then
  spawned with no shell. A runner that would splice the name into a larger
  argument is rejected at load. `--execute` is off by default; CI must not enable
  it for fork PRs. Pinned by `a_hostile_test_name_cannot_inject_a_command` and the
  runner-validation tests; a Kani proof is the follow-up (ROADMAP §4).
- **`paths:` field** on `Task`, repo-relative globs naming the files a task owns.
  It is what makes `APLAN012` a checkable relation rather than a citation. Schema
  regenerated and gated by `gen-schema --check`.
- `check` hermeticity is now a **tested invariant**: a source-level guard fails CI
  if the check path (`aplan-core`, `aplan-output`) ever names a process, socket, or
  git API. With `unsafe_code` forbidden workspace-wide there is no raw-syscall
  escape hatch, so this is a real guarantee. The `verify` rules get the full
  fires+silent coverage-audit treatment, backed by real temp git repos.
- A **house-style gate** (`cargo xtask lint`, CI job "house style") forbids the
  typographic characters that read as machine-generated: em and en dashes, curly
  quotes, the ellipsis character, and non-breaking space. Functional Unicode (the
  arrow in cycle output, the section sign, the accented letters in the slug tests)
  is untouched. The existing content was cleaned to plain ASCII in the same change,
  so the gate is green from the start rather than grandfathering violations.
- Task model (`id`, `title`, `phase`, `status`, `owner`, `effort_days`,
  `blocked_by`, `evidence`, `paths`, `traces`, `tags`) as markdown + YAML
  frontmatter.
- Eleven invariants, `APLAN001`-`APLAN010` and `APLAN020`. `status: done` requires
  typed evidence; the dependency graph is proven acyclic on every run; `traces`
  anchors must resolve to real headings.
- `APLAN020`: a live task may not be blocked by a `dropped` one. Such a task can
  never legitimately start, so the plan is deadlocked, but `check` used to pass it
  clean. `Dropped` failing to satisfy a dependency already shaped `ready` and `why`;
  the gate itself stayed silent, which made the "an invariant nobody checks" claim in
  `HLD.md` and `pre-public product research` false about aplan itself. It takes `020` because
  `APLAN011`-`APLAN019` were already reserved for the planned evidence ladder, and a
  rule code is a contract, not a tidy sequence.
- A CLI black-box corpus (`trycmd`, `crates/aplan/tests/cmd/`): every command, format,
  flag, and exit code, against a set of sandboxed fixture plans. The binary previously had
  **zero** tests, and every defect below lived on exactly that surface.
- `--no-traces`, to switch `APLAN008` off explicitly.
- An enumerable rule registry: `aplan_core::Rule`, with `Rule::ALL`, `Rule::code()`, and
  `Rule::title()`, generated together with the `code::` constants from one `define_rules!`
  table so a rule's variant, code, and title cannot drift apart. `Format` likewise gains
  `Format::ALL` and `Format::as_str` (the inverse of its `FromStr`).
- The **coverage-drift audit** (`crates/aplan-e2e`, ROADMAP §5): an end-to-end scenario
  corpus with a *fires* and a *silent* case for every rule, each materialised to a real
  temp tree and run through the loader and validator, plus meta-tests that walk the code's
  own `Rule::ALL` and `Format::ALL` and **fail CI** if any rule or format lacks coverage.
  Adding `APLAN021` or a new `--format` without a test now turns the build red; verified by
  reintroducing each drift. This is the mechanism that makes "test every rule" a property
  of the build rather than a discipline, and the reason `APLAN020` above could no longer
  have shipped, as it briefly did, without a test.
- `aplan check` (`human` / `json` / `agent` / `github` output), `ready`, `why`,
  `graph` (generated Mermaid), `schema`.
- `schemas/v1/task.json` derived from the Rust types via `schemars`, gated by
  `cargo xtask gen-schema --check`.
- Exit-code contract: `0` clean, `1` violations, `2` usage/internal.

### Changed

- **CI runs on a self-hosted runner** (`A1-026`, `A1-027`, the pre-public repository-boundary decision), ending all dependence on
  GitHub-hosted runner minutes: an Actions billing block had halted every job (a pull request's
  checks reported failure with zero steps executed). Every job in `.github/workflows/ci.yml` now
  runs on `[self-hosted, linux, aplan]`; there is no `ubuntu-latest` job, so no run bills a
  minute, and if the runner is offline jobs queue as pending rather than fail. The box is a
  rootless `podman` container (the Rust toolchain and the actions-runner agent baked into the
  image) stood up by `ci/runner/{setup,entrypoint,teardown}.sh`, with persistent volumes for the
  registration identity, the cargo registry, and `CARGO_TARGET_DIR` (the last mounted outside the
  checkout so `git clean` cannot wipe it), and a `systemd --user` unit for reboot persistence. The
  toolchain baked into the image replaces `dtolnay/rust-toolchain`, and the local volumes replace
  `Swatinem/rust-cache`; each gate is a `ci/scripts/*.sh` the workflow and the opt-in `pre-push`
  hook share. Because the repository is private and single-owner there are no untrusted forks, so
  every job routes to the box (the pre-public repository-boundary decision records the tripwire to add fork-PR isolation before ever
  going public).
- **The plan directory is `aplan/`, and its location is configurable** (`A1-025`, ADR-0010).
  The default `--root` moved from the generic `plan/` to the tool-namespaced `aplan/`: a verified
  survey of comparable git-native tools shows durable, hand-edited, reviewed content lives in a
  visible directory named for its tool, while only a machine-managed DB blob goes hidden, and
  `aplan`'s plan (reviewable markdown) is squarely the former. A new optional `root:` key in
  `.aplan.yml` sets the location (a relative value resolves against the config's directory), with
  the usual precedence: the `--root` flag over the `root:` key over the built-in default, so a
  team that prefers a hidden directory sets `root: .aplan` in one line. `schema`, `facts`, and
  `init` never read the config, so they cannot fail on a missing or invalid one. `aplan`'s own
  plan was `git mv`-ed from `plan/` to `aplan/` (history preserved), the docs and trycmd fixtures
  moved with it, and `schemas/v1/config.json` regenerates to carry `root` (gated by
  `gen-schema --check`). Task files stay flat: identity is the `id` field, never the path, so
  nesting stays free and optional.

### Fixed

- **Documentation consistency sweep.** The README no longer hard-codes rule or command counts
  in prose (they drifted twice, "six invariants" then "eleven" over a table of twenty): it
  points to `aplan facts` for the authoritative list and its always-on table now carries
  `APLAN021`, with `APLAN022` in the opt-in set. A **Self-hosting** section documents the
  roadmap generation, and `ROADMAP.status.md` is in the doc index. ROADMAP `Section 3` was
  stripped of live status (rationale only; live status is the generated companion), and
  `CONTRIBUTING`'s add-a-rule steps gained the `facts.json` regen and opt-in-config wiring they
  omitted.
- **Adversarial audit of `aplan roadmap` (`A1-015`).** A non-ASCII `commit` evidence `ref`
  no longer panics the tool (the 12-char shortening took bytes, not chars, and `check` never
  validated the ref is ASCII hex). The `--format markdown` companion now escapes `|` and
  newlines in every cell, so an id, title, or evidence ref carrying one cannot corrupt the
  gated table (the same class of hardening `mermaid` already had). A `dropped` member is now
  excluded from the cut's rollup *and* its critical path (it was inflating the per-cut path
  while the whole-plan `critical-path` excluded it), so `done + ready + blocked == total`
  reconciles. The markdown member table now carries the `title` (ADR-0009 lists it, and the
  old `id == title` fixtures hid its absence). `--format` is validated before the plan is
  loaded, so a bad format is a usage error (exit 2), not masked by a plan error. Added render
  unit tests (escaping, title, reflow-proofness, versionless cuts) and a `--format json`
  corpus case, the render paths that shipped untested.
- **Second adversarial audit: more false negatives, a regression, and a facts-drift
  hole.** The `APLAN018` drift fingerprint no longer collapses whitespace, so a change
  to indentation-significant content (a Python block, a YAML nesting, a Makefile recipe)
  is caught rather than silently ignored; it hashes leading whitespace, blank lines, and
  internal spacing, surviving only trailing-whitespace and line-ending churn (the
  "reflow-invariant" claim is corrected accordingly). A leading BOM on a traced document
  no longer hides its first heading (a spurious `APLAN008`), and an info string on a
  *closing* code fence no longer ends the block early.
- **Evidence with an invisible `ref`** (a zero-width space, and the rest of the Unicode
  Cf class) no longer counts as proof for `APLAN005`/`APLAN017`, the same invisibles the
  house-style gate forbids; `str::trim` alone missed them.
- **A task in a nested `target`/`node_modules` directory is no longer silently dropped**
  from the gate, a regression from the previous audit's directory prune, which matched
  those names at any depth. VCS metadata (`.git`, `.jj`, `.hg`, `.svn`) is still pruned
  everywhere, but build output only as an immediate child of the plan root.
- **A hex-named branch or tag** (e.g. a tag `deadbeef`) no longer passes `verify` as an
  immutable `commit` SHA: a real SHA is a prefix of the resolved object id, a ref name
  is not.
- **Duplicate task-frontmatter keys are rejected** instead of silently keeping the last
  (the task now deserialises straight from the YAML, as `.aplan.yml` already did), and a
  `.aplan.yml` that is a *directory* is a reported error, not a silent fallback to
  defaults.
- **`facts.json` can no longer silently omit a format or evidence kind.** `Format::ALL`
  and `EvidenceKind::ALL` are generated from the same table as their enum (a new
  `str_enum!` macro, the discipline `define_rules!` gives `Rule`), so a variant cannot be
  added without joining the enumerable registry `facts.json` reads. The verify-only rule
  severities (`APLAN011`-`APLAN015`) are now asserted against their firing sites by the
  coverage audit, so a declared severity cannot lie.
- **`aplan close --restamp`** now confirms the status and evidence survived the rewrite
  (not just the fingerprints) and no longer swallows a blank line after the block; the
  facts gate reports a missing `facts.json` with a clear cause.
- Docs honesty: the README self-showcase no longer quotes a task count (it drifted
  twice); HLD Section 8 marks `verify` shipped and `graph --format dot` planned; HLD
  Section 7/9 list the `evidence` and `detect_drift` config keys; the CHANGELOG's
  fixture-plan count and ROADMAP's stale `proptest` line are corrected.
- **Adversarial audit: closed several ways a bad plan could pass `check`** (false
  negatives, the worst bug class for this tool). Evidence with a blank `ref` no longer
  counts as proof (`APLAN005`/`APLAN017`); the file walk no longer lets `.gitignore` /
  `.ignore`, a hidden dotfile, or an uppercase `.MD` silently drop a task from the gate;
  `APLAN010` rejects a `captured` that is present but not an ISO-8601 date; a negative
  `effort_days` is now a schema error.
- **... and closed false positives**: `APLAN008` now resolves setext headings (underlined
  by `===`/`---`) and no longer resolves a heading that exists only inside a fenced code
  block; a relative `traces_root` in `.aplan.yml` resolves against the config's directory,
  not the process CWD, so the verdict no longer depends on where `check` runs; a trace
  listed twice reports once.
- **Evidence resolution (`verify`) no longer accepts forged proof from an untrusted task
  file**: `file` evidence must resolve to a blob, not a directory (`file: crates` used to
  pass `APLAN015`); a catch-all `paths` (`**`, `*`, `**/*`) fails `APLAN012` instead of
  making every commit trivially relevant, and a single `*` no longer crosses `/`; `commit`
  evidence must be an immutable SHA, not a mutable branch/tag/`HEAD`; a `file` path
  containing `@` (a scoped npm package) is no longer misparsed as `path@rev`.
- **Trust boundary**: `TestRunner::validate` rejects `{test}` as the command
  (`run: ['{test}']` would have executed the untrusted test name as the program under
  `--execute`).
- **Write commands (`close`/`init`)**: `close` preserves CRLF line endings and a leading
  BOM byte-for-byte, keeps a trailing comment on the `status:` line, and finds a `status :`
  key; it refuses a `dropped` (terminal) task and an existing `evidence: []`, and confirms
  the exact evidence round-tripped (not just the count). `init` writes with `create_new`
  (atomic `O_EXCL`), closing a symlink race in the never-overwrite guarantee. Both tolerate
  a closed stdout pipe instead of panicking (exit 101), and `close`'s HEAD-derivation
  errors are no longer mislabelled as test-runner failures.
- Dropped the unused `miette` and `proptest` workspace dependencies; corrected stale README
  claims (flag coverage, the self-showcase task count) and HLD Section 16 (the exit-code
  contract is tested, and `proptest` is no longer declared).
- `aplan summary` rejects `--format agent`/`github` (which it silently rendered as JSON)
  instead of contradicting its own `--help`.
- `APLAN007` lowercased the whole diagnostic to normalise the status name, which
  mangled the task id (`C-001` → `c-001`). Status now renders through `Display`;
  regression test `done_before_blocker_preserves_task_id_case` pins it.
- `APLAN008` slugified headings incorrectly: it mapped `.` and other punctuation
  to `-` and handled only ASCII, so `## 2.1 Delaware: the filing channel` yielded
  `2-1-delaware-...` instead of GitHub's `21-delaware-...`, the `traces` example
  in the README failed against the README's own heading. `slugify` now follows
  GitHub's algorithm (strip punctuation, keep Unicode letters and `_`, spaces to
  `-`, no coalescing), disambiguates repeated headings with a `-N` suffix as
  `github-slugger` does, and tightens ATX heading detection. The `title.contains`
  fallback that masked the bug is gone. Pinned by `readme_trace_example_resolves`,
  `slugify_matches_github`, and `duplicate_heading_resolves_by_suffix`.
- A task file with a leading UTF-8 BOM (common from Windows editors) was silently
  skipped as "not a task", so a `status: done` with no evidence slipped through,
  the exact failure the tool exists to prevent. The loader now strips a leading
  BOM. Pinned by `bom_prefixed_task_is_loaded_not_skipped`.
- Output was not deterministic across environments: the diagnostic sort key
  `(code, task)` was not total and the file walk was unsorted, so tied diagnostics
  (e.g. two schema errors) kept filesystem `readdir` order, divergent across CI
  runners. The walk is now sorted and the sort key is total (`..., file, message`).
  Pinned by `load_order_is_deterministic` and `tied_diagnostics_sort_deterministically`.
- `aplan why` reported "ready, nothing blocks it" for a task blocked only by an
  unknown id, contradicting `check` (which flags `APLAN003`). `why` now lists
  unknown blockers. Pinned by `direct_unknown_blockers_finds_ghosts`.
- `ready`, `why`, and `graph` panicked with exit code 101 on a closed pipe
  (`aplan ready | head`); they now exit cleanly. `check` no longer downgrades a
  clean/violations verdict to exit 2 on a broken pipe either.
- A self-dependency was double-reported as both `APLAN006` and `APLAN004`;
  `find_cycle` now ignores self-edges, which `APLAN006` already covers. Pinned by
  `self_loop_is_self_dep_not_also_cycle` and `self_loop_is_not_reported_as_a_cycle`.
- `aplan schema` still panicked (exit 101) on a closed pipe, the broken-pipe fix
  had missed it. It now routes through the same tolerant writer.
- A non-task markdown file that merely opened with `---` (a `----` rule, a `---foo`
  line) produced a false `APLAN001`, and a file with an *unterminated* fence
  reported "no fence at start of file". The walker now treats only a bare `---`
  first line as a task candidate (non-fence files are skipped silently), and an
  unterminated fence gets an accurate message. Pinned by
  `non_fence_markdown_is_skipped_not_errored` and `unterminated_fence_reports_accurately`.
- An unreadable task file was silently skipped, so `check` could pass while hiding a
  task; it is now reported. Pinned by `unreadable_task_is_reported_not_skipped`.
- The loader stripped only one leading BOM; it now strips any number. Pinned by
  `strips_multiple_leading_boms`.
- `--format github` did no escaping, so a task file (which can arrive in a fork PR)
  with a comma in an id corrupted the annotation and a newline in a message could
  **inject a forged GitHub workflow command**. Output is now escaped per GitHub's
  rules (`%25`/`%0D`/`%0A`, plus `%3A`/`%2C` in properties). Pinned by
  `github_line_escapes_injection` and the `escape_*` tests, `aplan-output`'s first.
- `aplan graph` emitted invalid Mermaid for ids containing spaces or metacharacters;
  node keys are now sanitised (conventional `A0-001` ids render unchanged). Pinned by
  `node_key_preserves_ids_and_sanitises`.
- Several load/parse diagnostics (unreadable, invalid YAML, deserialise, internal)
  lacked an `agent_instruction`, contradicting the `agent` format's promise that
  *every* diagnostic carries one. They now do. Pinned by
  `parse_error_diagnostics_carry_an_agent_instruction`.
- A `--root` naming a directory that does not exist loaded as an **empty, clean
  plan**: `aplan check --root typo` printed `ok: 0 tasks` and exited 0. Renaming the
  plan directory would have left CI green forever while validating nothing, the gate
  silently disabling itself, which is worse than having no gate. It is now a usage
  error (exit 2). Pinned by `tests/cmd/usage.trycmd`.
- `aplan check` silently skipped `APLAN008` unless `--traces-root` was passed:
  `traces_root` defaulted to `None`, so a task tracing a document that did not exist
  printed `ok` and exited 0, while the README advertised the rule as one of ten that
  fail CI. Anchors now resolve against the current directory by default, and skipping
  the rule requires asking for it (`--no-traces`). Pinned by `tests/cmd/traces.trycmd`.
- `ready` and `why` announced explicitly parked tasks as ready. A `status: blocked`
  task has no unmet *graph* blockers, and readiness was tested with `!is_terminal`, so
  `aplan ready` offered work the plan says not to do and `aplan why` replied "ready,
  nothing blocks it" about a task whose status is literally `blocked`. Readiness is now
  a status question (`Status::is_workable`). Pinned by `parked_task_is_never_ready` and
  `tests/cmd/parked.trycmd`.
- `ready`, `why`, and `graph` ignored load diagnostics, so on a plan with an unparseable
  task file, one `check` rejects with exit 1, they printed a confident, partial answer
  and exited 0. They now report the load errors on stderr and stop with exit 1. Pinned by
  `tests/cmd/unloadable.trycmd`.
- `aplan graph` emitted a raw newline inside a Mermaid node label when a task title
  contained one (YAML permits it), producing a graph Mermaid cannot parse. The same
  escaping gap as the `--format github` bug, on the renderer that was only half-hardened.
  Pinned by `escape_neutralises_quotes_and_newlines` and `mermaid_output_is_single_line_per_node`.
- `blocked_by: [GHOST, GHOST]` reported the identical `APLAN003` twice (two identical PR
  annotations). Edges are deduplicated: one broken edge, one diagnostic. Pinned by
  `a_repeated_blocker_is_reported_once`.
