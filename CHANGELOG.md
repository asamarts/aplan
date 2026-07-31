# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
