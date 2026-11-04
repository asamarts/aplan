# Development validation without release-loop latency

Status: accepted; implementation is `A1-610` through `A1-619`, `A1-650` and `A2-013`.

## Decision in one paragraph

`aplan` should stop treating every local push and every commit to `main` as a release candidate.
Keep one declarative gate inventory and expose three named profiles: `development`, `full`, and
`release`. Development validation is an always-reported, fail-closed selection over changed files,
affected workspace packages, reverse dependencies, generated artifacts, and structural plan
checks. Full validation is the complete current contract, including isolated coverage, MSRV and
whole-plan evidence verification. Release validation runs full validation on the exact commit it
will package, then builds and checks the artifact. Full runs on release, nightly, explicit dispatch,
and during the rollout audit; ordinary local pushes and CI use development validation.

This changes cadence, not standards. A development result must say which gates it ran and deferred;
it must never call a selected result "full" or "verified". Unknown inputs widen selection. A
release cannot consume a result from another commit, bypass a gate, or publish after only the
development profile.

## Why this is now the bottleneck

The repository grew the right gates one incident at a time, but its baseline orchestration had one
shape. Before `A1-613`, `ci/scripts/preflight.sh` invoked all ten gates in sequence and the opt-in
pre-push hook invoked that script for every non-deletion push. Before `A1-617`,
`.github/workflows/ci.yml` independently ran the same work as six jobs on every push to `main` and
every pull request. It now reports one change-aware Linux development status plus required macOS
and Windows portability statuses on those events, and runs one complete Linux job nightly or by
explicit dispatch. Every job uses fresh standard GitHub-hosted compute. `A1-616` measured and
rejected dependency, target and archive reuse; the routed workflows do not blur the MSRV,
coverage, release or verdict boundaries merely to avoid rebuilding them.

The result is release assurance at task-edit frequency:

| gate | warm or observed local time on 2026-10-20 | dominant work |
|---|---:|---|
| format | 2.5-2.8 s | whole-workspace source scan |
| strict clippy | 0.6 s unchanged; about 17 s after relevant edits | incremental compile |
| rustdoc | 0.4 s unchanged; about 5.8 s after relevant edits | incremental compile and links |
| workspace tests plus CLI audit | 126-156 s | test execution, not compilation |
| schema/roadmap specification | 3 s warm; 28-34 s after rebuild | generators and their build |
| declared MSRV | 27.2 s in this sample | separate compiler/target |
| repository style | 0.5-0.6 s | tracked-file scan |
| isolated coverage | 329-467 s | full instrumented rebuild and suite |
| structural check | 0.093 s | cached plan load and validation |
| whole-plan verify | 146 s warm; 383 s after invalidation/load | twelve configured test catalogues |

These are wall-clock observations on a shared machine, not performance assertions. They are enough
to identify the shape: `mbx` made ordinary compilation cheap, while executing the complete suite,
building an isolated coverage universe, and listing evidence providers remained expensive. A full
local pass is commonly 12-18 minutes. Closing a task can itself spend minutes listing runners, then
pre-push repeats that work and remote CI repeats it again.

The live Actions history confirms the orchestration cost. Run `39525226073` spent 11 minutes 53
seconds on the single runner before failing. Its coverage job alone compiled for 6 minutes 4
seconds and ran for over 7 minutes. The same run then discovered a Clippy difference between the
runner's 1.98 toolchain and the development host's 1.99 toolchain. Coverage separately found that a
signal proof assumed an external `kill` executable the runner image does not contain. Those defects
are repaired by `A1-608` and `A1-609`; more importantly, they show why a shorter loop still has to
report exact toolchain and environment identity.

The confirming push exposed the same contract gap in a second form: the three tests for the
committed Python performance harness passed on a development host with `python3`, but the former
runner image did not declare or install it. `A1-620` made the runtime an audited input, and hosted
preparation retains that audit. A validation profile is not reproducible if it
names only Rust while its tests execute another undeclared runtime.

## Baseline contract and gaps

### What is sound and must remain

- Every gate is a repository script, so local and CI entry points share implementation.
- Workspace tests use `--workspace`; the root's `default-members` would otherwise omit eleven
  packages.
- Coverage owns a fresh run directory. `A1-457` proved that reusing instrumented objects can merge
  mappings and make the measurement false. No optimization may reuse that object set.
- Dogfood checks the repository under its real warning policy and full Git history.
- The MSRV is compiled rather than inferred from metadata.
- CI concurrency already cancels obsolete runs for one workflow/ref.
- Long scale checks already run nightly, establishing precedent that a useful gate need not run on
  every commit.

### What is missing or misleading

1. **There is no named assurance level.** `PREFLIGHT_SKIP` subtracts arbitrary gates from a script
   still named "full CI gate bundle". The resulting pass has no machine-readable account of what it
   means.
2. **Release does not require full CI on the release commit.** `release.yml` validates artifact
   shape, version and static linking, then publishes. It neither calls preflight nor depends on a
   full result for the tag SHA. The requested "full CI for any release" is therefore not enforced.
3. **Every ordinary change pays for unrelated work.** Prose edits run an MSRV compile and isolated
   Rust coverage; a leaf-crate edit tests every package; task prose runs the complete evidence
   catalogue.
4. **The most expensive work is repeated, not shared.** Local close/verify, pre-push, and the remote
   evidence gate each list the same configured runners. Hosted jobs will independently prepare and
   build overlapping artifacts.
5. **The required status cannot safely be implemented as a path-filtered workflow.** GitHub states
   that a workflow skipped by path or branch filtering leaves its check Pending, blocking a pull
   request that requires it. One stable development job must always report and make selection
   inside the job.
6. **Rolling `stable` is not reproducible across hosts.** The then-active private runner lagged the
   development host by one release. Rustup supports a committed toolchain file, but pinning policy
   needs an explicit compatibility decision: `rust-version` is the consumer floor, while the
   development compiler is the lint and build reference.
7. **There is no latency telemetry.** Job timestamps can be reconstructed after the fact, but no
   durable record separates queue, setup, compile, execution, evidence and coverage phases.
8. **Branch protection cannot currently enforce a required check.** GitHub's API returns HTTP 403
   for this private repository/account tier. The workflow should still publish one stable result;
   protection becomes an explicit public-repository setup step rather than an assumed current
   control.

## Invariants

The implementation is acceptable only if all of these survive:

1. `full` contains every gate in the current preflight. A generated/executable audit proves that
   relationship; it is not a table maintained beside shell code.
2. `release` invokes `full` on the same immutable Git SHA before artifact construction and
   publication. A green run from another SHA, branch or cache is not evidence.
3. `development` always returns one check with a deterministic record of selected, skipped and
   widened gates. "Nothing selected" is a successful decision only for a proven no-op input.
4. An unknown, malformed, untracked, non-UTF-8, renamed or deleted path cannot narrow work. It
   widens to a conservative development set or refuses.
5. Selection changes only when the declarative inventory and its negative-control tests change.
   Adding a tracked file without classifying it falls back wide.
6. Coverage remains isolated and authoritative. Neither normal Cargo caches nor nextest archives
   contribute instrumented mappings from another run.
7. A selected test result is not presented as complete workspace evidence. Full and release retain
   `cargo test --workspace` unless a separately proved runner is admitted.
8. Whole-plan `verify` remains the authoritative evidence gate. Development may defer it, but must
   name that deferral; it may not turn a partial verification into a clean whole-plan verdict.
9. Untrusted pull requests do not write caches later consumed by privileged release work. Cached
   files contain no secrets and are correctness-neutral: a miss changes time, never outcome.
10. Timing budgets are observational rollout criteria, not flaky wall-clock test assertions on a
    shared runner. Correctness uses deterministic work counters and mutation/negative controls.

## The three profiles

### Development

Purpose: the default local pre-push, pull-request, and ordinary `main` feedback loop.

Always run cheap repository-wide checks:

- formatting and plain-ASCII/style;
- the change classifier and its audit;
- `aplan check` under the repository configuration;
- a deterministic summary of selected and deferred gates.

Then select by affected surface:

| change class | additional development work |
|---|---|
| task/plan prose | roadmap generation check, document-reference checks, structural check |
| narrative docs | document-reference checks; rustdoc only for Rust doc comments/API docs |
| one Rust package | strict clippy and tests for that package plus workspace reverse dependents |
| public/core/config/schema surface | affected packages plus cross-cutting registry, schema, corpus and compatibility audits |
| generator or generated input | run the matching generator check and every consumer named by its inventory entry |
| Cargo manifests, lockfile, build scripts, features, toolchain or CI machinery | widen to all-package development checks; add MSRV when its inputs can change |
| evidence runner configuration or evidence semantics | structural check plus the evidence-specific development disposition |
| unknown/unclassifiable path | widest development profile, with the widening reason |

The first implementation should be conservative. Whole-repository fmt/style and a warm rustdoc
are cheap. It is better to keep a five-second check than to build a brittle selector to save it.
The first meaningful wins are omitting isolated coverage, full workspace tests, unconditional MSRV,
and whole-plan provider resolution from ordinary changes.

The intended warm-host objectives are:

- plan or prose-only handoff: under 10 seconds;
- ordinary one-package Rust handoff: under 60 seconds;
- cross-cutting Rust handoff: under 120 seconds.

They are p95 rollout objectives on an otherwise available host, not test timeouts. A run that is
slower remains valid and reports why.

### Full

Purpose: authoritative repository integration independent of packaging.

It contains the current exact gates:

1. format;
2. strict workspace/all-targets clippy;
3. workspace rustdoc with warnings denied;
4. workspace tests and executable CLI-surface audit;
5. schema and roadmap drift checks;
6. declared-MSRV compile;
7. house style and document checks;
8. isolated workspace coverage floors;
9. structural check and whole-plan evidence verification against full Git history.

It runs nightly, by explicit dispatch, before release, and during rollout comparison. It can also
be invoked locally as `ci/scripts/preflight.sh --full`. It has no gate-subtraction option. A person
can terminate it, but a terminated or skipped run cannot produce a full receipt.

### Release

Purpose: prove and package the exact released source.

It runs `full` in the tag/manual-dispatch checkout, records the commit and tool identities, builds
the headless and TUI feature shapes already required by `release.sh`, verifies version/static
artifact/checksum properties, and only then permits publication. The publish job has a direct
`needs` or same-job dependency on that exact validation. A manual run that is not on a tag may stage
an artifact but cannot create a release.

The validation receipt is an Actions summary/artifact, not repository state and not a bypass token.
It records SHA, profile, gate inventory digest, toolchain, gate outcomes and durations. Reuse is
allowed only inside one workflow execution for the same SHA; the simpler initial implementation is
to run full and release steps in one checked-out job.

## Affected-surface selection

### Inputs

The classifier uses explicit modes rather than guessing what "changed" means:

- working tree: merge staged, unstaged and untracked paths for local handoff;
- push/PR: diff the event's base and exact head, with a fetched merge base;
- explicit: accept two immutable revisions for tests and diagnostics.

The local pre-push route is a fourth composition over those primitives. Git supplies every
`remote-sha -> local-sha` update; the hook preserves each non-deletion pair and the classifier unions
their direct revision diffs with staged, unstaged and untracked paths in the working tree. A
deletion-only push is an explicit no-op because it publishes no new source. Invoking the hook with
no update records is distinct: it says so and validates the working tree. A missing new-branch base,
a pushed head other than the checked-out source, malformed history, or any other uncertainty widens
instead of disappearing.

The selector-to-dispatcher boundary is schema-versioned, NUL-delimited records, not sourced shell or
whitespace-split JSON. It carries direct gates, package names, widening reasons and whether the
input is a proven no-op. A failed, truncated, unknown-schema or malformed classifier result runs no
gate and produces no development verdict. `no_changes=true` is emitted only when both the observed
change set and the uncertainty set are empty.

Renames contribute old and new paths. Deletions still select the deleted path's former owner.
Missing bases, shallow history, parse errors, path encoding the classifier cannot represent, and
files outside the inventory widen rather than silently disappearing.

### Rust packages

Use `cargo metadata` as the source for workspace members and dependency edges. Cargo documents its
`resolve.nodes` as the resolved package graph; the policy generator reverses it into a host-neutral
checked-in graph, then binds the root manifest, lockfile, Cargo/toolchain configuration and every
member manifest by content digest. The runtime classifier validates those inputs and reads that
graph without starting Cargo or rustc. A stale or missing graph refuses a verdict rather than
guessing, and every graph input is already a global change which requires regeneration. The
classifier then includes the changed package and all workspace dependents whose build or tests can
consume it. `cargo test -p` is repeatable, so one invocation can test the selected package closure.
Root `Cargo.toml`, `Cargo.lock`, workspace profiles, feature topology, toolchain files and build
infrastructure are global inputs and widen selection.

In development, the `rust-workspace` gates receive the selected closure as repeated Cargo package
arguments. The same scripts receive `--workspace` under full/release or when no safe package scope
exists. Any widening reason discards package narrowing and runs the complete selectable development
gate set workspace-wide. Formatting, style and structural plan validation run before selected or
widened work, so even a no-change decision still exercises the always-on contract. Coverage and
whole-plan evidence remain explicitly deferred rather than being reported as passed.

Dependency reachability is necessary but not sufficient. Integration tests in `aplan`, executable
surface audits in `aplan-e2e`, and generators in `xtask` intentionally test other crates. Those
cross-cutting owners belong in the declarative inventory and mutation tests. Do not infer them from
filenames.

### Non-Rust surfaces

Each tracked prefix has one disposition: gate set, generator relation, documentation-only, fixture,
or conservative fallback. An executable census walks `git ls-files` so a newly tracked file cannot
remain silently unclassified. The fallback is wide development validation, never no validation.

Workflow-level GitHub `paths` filters are deliberately rejected for the required job. GitHub
documents that skipped filtered workflows leave required checks pending. Selection happens after
checkout and the stable job always reports.

## Structural and evidence gates

`aplan check` is already sub-100 ms here and belongs in every development run. `aplan verify` is a
different operation: it opens Git and all configured test catalogues, and correctly refuses
selection because a partial clean result would misdescribe the plan. The profile must preserve that
semantic distinction.

The initial development profile therefore:

- runs structural `check` every time;
- runs the repository's document and generated-plan checks when their inputs change;
- relies on `aplan close`/`amend` to resolve evidence at mutation time and on affected tests to run
  changed code;
- reports whole-plan evidence verification as deferred;
- receives authoritative whole-plan L1/L2 `verify` from nightly/full/release.

This creates a bounded detection window for manually edited or indirectly stale evidence, not a
false clean verdict. Eliminating that window safely belongs to the typed observation/catalogue work
already designed in `A1-590`, not to an mtime cache in a shell script. A future fast path may reuse a
test catalogue only when runner definition, declared material inputs, executable/toolchain identity,
environment contract and exact output are all in its key. Missing declarations mean non-cacheable.

`A1-615` makes the split visible as the `structural` and `evidence` inventory gates. The structural
script invokes the GitHub renderer once: its finding records serve as both readable terminal output
and Actions annotations, and an explicit pass/fail line replaces the former second human-format
parse. Plain `verify` still performs its own structural validation because its whole-plan verdict
cannot trust a result from another process or checkout; that semantic precondition is the justified
remaining overlap. The development summary names `evidence` as deferred and cannot say "verified".
The evidence script runs plain `verify` for whole-plan L1/L2 repository and catalogue resolution;
it is not L3 direct execution and does not implement receipt-backed authority. Persistent provider
observations remain `A1-590`'s design, with `A1-647` owning the audit before rollout. This task does
not add `verify --task`; `A1-214` correctly rejected gate selection because it would make a subset
sound whole.

## Test runner and nextest evaluation

Cargo's package selection provides the first reduction without adding a dependency. Nextest is a
promising second step because it builds test binaries, lists tests and schedules each test as a
separate process; its own design documentation identifies long-pole binaries as the limitation it
addresses. It also adds a thicker harness interface, does not run doctests, changes process
isolation, and can affect signal, PTY, environment and CLI-invocation recording tests. It is not a
drop-in conclusion.

`A1-610` was therefore a measured admission test, not an implementation commitment. It had to:

- compare complete test inventories, including doctests and ignored tests;
- compare failures after seeded negative controls, not just green outcomes;
- exercise trycmd, PTY, signal/process-group, Git, concurrent-id and invocation-census tests;
- measure cold, warm and contended runs using fixed/pinned nextest;
- account for the CLI-surface audit's invocation log;
- evaluate `cargo llvm-cov nextest` separately while retaining doctest coverage and the isolation
  guarantee;
- reject adoption unless the resulting contract is complete and the p50/p95 gain is material.

The completed evaluation is `docs/design/development-validation-nextest.md`. `A1-650` implements
its admitted contract: pinned nextest is the non-doctest executor in the shared test gate, paired
with an explicit Cargo doctest stage, zero retries, fail-on-flaky behavior and a serialized
workspace-source group. The gate compares nextest with Cargo's actual inventory and audits resolved
group membership before execution. It rejects parallel partitions because the group mutex does not
cross processes, and retains Cargo for coverage because process-per-test execution changed the
per-crate covered-line set. `A1-616` measured hosted cache and archive reuse without changing this
correctness boundary, rejecting archive fanout and executable target caches. `A1-617` routes push
and pull-request events through that shared development gate and nightly/manual events through
full.

## Caching and build reuse

### Local and historical self-hosted measurements

Continue using the development host's shared compiler cache/orchestrator for ordinary Cargo work.
Do not add another repository cache over it. The former persistent CI target made compile time a
minority of the warm loop; these measurements remain evidence for optimizing executed work first.

Do not share these as correctness inputs:

- coverage object/profraw directories;
- pass/fail verdicts;
- plan validation reports without the existing source-authoritative cache checks;
- configured provider output without `A1-590`'s complete identity contract.

#### Coverage allocation lifecycle and reuse decision

`A1-633` kept the isolated correctness boundary and repaired its persistent-host lifecycle. Every private
`.aplan-coverage-<pid>-<attempt>` parent carries a bounded versioned owner record: Linux boot and
process-start identities for the xtask owner and cargo child, the child's process group, and a
unique lease inherited by descendants. A catchable termination signal stops the cargo process
group, lets ordinary cleanup finish and is then re-raised. A hard kill is recovered by the next
xtask run or the runner's pre-job hook. Both reclaimers require the owner and child to be gone, the
process group to be empty and no readable process to carry the lease. Missing, malformed, reused
or uninspectable recorded identities are retained. Names, real-directory checks and a same-inode
recheck bound recursive removal to the exact allocation.

`A1-603` separately bounds instrumented libtest execution to four threads. The ordinary runner
passed the same process-sensitive cases, while host-sized coverage fan-out first starved the CLI
termination provider before readiness and later left an idle real-terminal graph journey unable to
consume `q` within its 20-second transport bound. Four matches the smallest planned public runner,
retains useful parallelism on the private host and changes neither the Cargo coverage runner nor
its isolated objects, measured lines or floors. The provider fixture also publishes complete live
wrapper, provider and descendant PIDs plus an explicit exit-status/stderr channel, so a real startup
failure is distinguished from a process that has not yet received a scheduling opportunity.

An allocation created by a pre-`A1-633` binary has no owner record and therefore cannot satisfy
that proof. Ordinary sweeps retain it. The former private-runner runbook provided a one-time
`--legacy-quiescent` migration that could remove only exact, real, ownerless allocation directories
after the runner stopped. `A2-013` retired that operational path after hosted migration; the
recorded algorithm remains evidence for why live cleanup never guessed.

The two adjacent optimization ideas were measured on kbox on 2026-10-22 against the A1-633 source
tree rooted at `bbdd2ef5`:

- Reusing the existing stable instrumented directory after cleaning only `profraw` files took
  159.131 seconds wall for the 2,486-test workspace. It grew the directory from 4.4 to 6.4 GiB and
  produced six impossible mappings whose reported coverable-line count exceeded the current source
  file's physical line count. This is rejected as a correctness input. cargo-llvm-cov documents
  `--no-clean` as retaining old build artifacts and recommends `clean --workspace` because those
  artifacts can affect results; that full clean removes the build reuse the proposal sought.
- The exact pushed baseline ran the ordinary build-and-test job in 114 seconds and isolated
  coverage in 181 seconds. They ran in parallel, so deleting the ordinary test execution would not
  shorten that workflow's critical path, while it would remove the non-instrumented test oracle and
  delay ordinary test feedback to the longer job. This task therefore does not merge the two.
  `A1-617` removes that duplicate execution by routing ordinary changes to development and
  nightly/manual runs to full, without calling a narrower gate full or weakening release coverage.

Reference: cargo-llvm-cov's documented clean and `--no-clean` semantics:
<https://github.com/taiki-e/cargo-llvm-cov/blob/main/README.md>.

### GitHub-hosted runners

Fresh jobs have no persistent registry or target volumes. `A1-616` measured the alternatives
on standard `ubuntu-latest` runners before admitting any reuse. Runs `39903909478` and
`39906843788` established the untrimmed baseline; the corrected shared-profile run is
`39910657606`. These are single-run observations on elastic hosts, not latency budgets. Phase
receipts and same-run comparisons decide architecture; they do not claim stable benchmark
percentiles.

The untrimmed baseline separated checkout/tool setup, cache transfer, Cargo compilation, nextest
plus doctests, the CLI audit and whole job wall time:

| alternative | setup/transfer | compile | execution after compile | observed wall/outcome |
|---|---:|---:|---:|---|
| no cache, direct test gate | 15 s setup | 454 s | 146 s runner/docs + 169 s CLI audit | 775 s gate; 793 s job; passed |
| full registry/Git/`target` cache, cold | 13 s setup + 44 s save | 450 s | 145 s runner/docs + 163 s CLI audit | 763 s gate; 822 s job; passed; 1,544,710,337-byte entry |
| same full cache, exact warm hit | 140 s restore | 215 s | 136 s runner/docs + 84 s CLI audit | 440 s gate; 599 s job; passed |
| Clippy + doc + test + spec + structural in one target | 15 s setup | 131 s Clippy + 25 s doc + 557 s test build | 163 s runner/docs + 224 s CLI audit | 1,107 s gate; passed; free disk fell from 13.39 to 5.12 GB |

The shared CI profile then removed incremental state, retained line tables only for workspace code
and removed dependency debuginfo. Run `39910657606` measured the same direct and combined shapes:

| alternative | setup/transfer | compile | execution after compile | observed wall/outcome |
|---|---:|---:|---:|---|
| no cache, direct test gate | 16 s setup | 335 s | 129 s runner/docs + 114 s CLI audit | 582 s gate; 600 s job; passed |
| crates.io cache, cold miss | 13 s setup/lookup + 3 s save | 303 s | 128 s runner/docs + 109 s CLI audit | 544 s gate; 563 s job; passed; saved 34,439,528 bytes |
| same crates.io cache, exact warm hit | 16 s setup + 3 s restore | 468 s | 146 s runner/docs + 176 s CLI audit | 793 s gate; 812 s job; passed |
| Clippy + doc + test + spec + structural in one target | 15 s setup | 123 s Clippy + 24 s doc + 416 s test build | 144 s runner/docs + 167 s CLI audit | 879 s gate; passed; free disk fell from 13.39 to 10.76 GB |

The cold-miss row is not credited as a cache gain: it downloaded the dependencies it later saved,
and its 38-second lead over the same-run direct job is ordinary elastic-host variation. The warm
hit moved in the other direction because hosted compiler throughput varied. The controlled
acquisition boundary is more useful: direct Cargo reached its first compilation 2.58 seconds after
the gate began, while the warm-cache job reached it after 1.94 seconds. Saving 0.64 seconds before
compilation did not repay the cache action's three-second restore. Paired run `39914045629` at one
source revision repeated the result: the direct job reached compilation in 2.60 seconds and
finished its gate in 760 seconds (776-second job), while the exact cache hit restored in two
seconds, reached compilation in 2.29 seconds and finished its gate in 796 seconds (816-second job).
The 0.31-second acquisition saving did not repay restore, and the 36-second gate difference is
reported as host variation rather than attributed to cache contents.

The 1.54 GB full cache saved 194 seconds of compilation and 85 seconds in the variable CLI audit,
but spent 140 seconds restoring. More importantly, it is source-specific executable state: only
about six such entries fit GitHub's default 10 GB repository allowance, and Cargo target
fingerprints are a rebuild mechanism rather than cryptographic authentication for adversarial
object files. It therefore fails the requirement that a hostile restored entry can change only
performance. A cache key cannot repair that trust property. Production rejects all `target`, Git,
unpacked-source and installed-tool caching.

The 34 MB compressed-crate candidate was the only cache shape that could satisfy the verdict
boundary. Its adversarial prototype admitted only regular, singly linked crates.io archives whose
filename and SHA-256 matched `Cargo.lock`; private registries, ambiguity, corruption, unreadable
files, links, nesting and unsafe roots became misses. That established technical safety, not value:
the measured transfer cost exceeded the subsecond acquisition saving. Production therefore omits
the validator and cache actions instead of making every pull request maintain code for a negative
optimization.

The audit also tested the proposed access control rather than relying on YAML parsing. GitHub's
`cache-mode` accepts only literal enum values; run `39914316541` proved that a conditional value
fails workflow validation before any job starts. Preserving explicit read-only pull requests would
therefore require a separate trusted post-test publisher with a static write-capable token. That is
the correct topology if a future cache earns admission, but it adds a hosted job and cannot rescue
this candidate's negative latency result. Production has no cache action, so no low-trust trigger
receives a cache write path and no job can mix ordinary, MSRV, coverage or release-musl artifacts.

GitHub documents that fork pull requests can read base-branch caches, that `pull_request` caches are
scoped to their merge ref, and that cache contents are visible to untrusted workflows. Any future
proposal must therefore store no tokens, credentials, task claims, provider observations or
verdicts even when its write topology is correct.

Build/archive fanout and additional job merging are rejected by the hosted measurements recorded in
`development-validation-nextest.md`. `A1-617` changes cadence and job routing while inheriting this
no-cache decision, the trust boundaries and the one-process test-runner contract.

## GitHub Actions topology

Implemented cadence:

| trigger | required work | purpose |
|---|---|---|
| local handoff/pre-push | development | fast feedback before network work |
| pull request | Linux development plus macOS and Windows portability | required review feedback on all supported hosts |
| ordinary push to `main` | Linux development plus macOS and Windows portability | verify the landed portable source contract |
| nightly schedule | full | bounded detection of deferred integration/evidence/coverage failures |
| manual dispatch | full | diagnosis and pre-integration confidence |
| version tag or release dispatch | release, which includes full | publication gate on exact SHA |

Use the repository scripts as the authority. A reusable workflow may remove YAML duplication, but
the called workflow must use `workflow_call` and callers must still bind the same checkout SHA.
GitHub documents reusable workflows as whole jobs, not steps; a shell profile remains the common
local/CI contract.

Retain concurrency cancellation for development runs so obsolete commits do not consume hosted
capacity. Do not cancel a release after publication has begun, and give full nightly/manual
runs their own concurrency group. A new push may cancel an obsolete fast run; it must not erase the
only scheduled full run without a replacement.

Branch protection requires the stable Linux development, macOS portability and Windows portability
statuses for ordinary merges. Release protection is the release workflow dependency, not a PR
status from a different SHA. Full can additionally be required at explicit stabilization
checkpoints without returning to every-task cadence.

## Observability, rollout and rollback

Every profile emits a bounded machine-readable result and a human summary containing:

- commit/base and dirty-tree identity;
- classifier version/inventory digest;
- selected packages and gates, widening reasons, deferred gates;
- queue/setup/compile/test/provider/coverage durations where available;
- toolchain and runner type;
- final assurance profile.

The implemented `aplan/validation-receipt/v1` record is emitted once on success or gate failure.
It is canonical JSON in the log and Actions summary, with an optional atomic private file for local
aggregation. A passing receipt is refused unless it contains every selected and widened gate; a
failed receipt retains the ordered prefix through the failing gate. Existing paths, symlinks,
control characters, absolute host paths, duplicate fields, invalid identities and records above
32 KiB are refused. GitHub queue duration remains explicitly unavailable inside the job and is
joined from API run/job timestamps by the rollout audit. Setup begins in hosted preparation, with
the nextest installer retaining a direct-invocation fallback. Compile is a labeled aggregate of
Clippy, rustdoc and MSRV gate time; test, provider and coverage retain their direct gate durations.
Unavailable, deferred and not-selected phases are distinct from an observed zero.

Do not commit raw timing receipts. Retain them in the Actions job summary or an ephemeral local
aggregation directory and aggregate p50/p95 by profile. No source paths beyond repository-relative
names and no command output that may contain secrets belongs in telemetry.

The observer is part of this contract too. A failed GitHub API query must be a distinct monitoring
failure, not an empty queue and not a response body parsed as a workflow status; `A1-621` proves
that boundary. `A1-622` separately distinguishes an absent run from one observed queued or running
but incomplete at the timeout, so the monitor preserves unavailable, absent, incomplete, successful
and failed outcomes.

Roll out in four stages:

1. Land the manifest and profile/superset audit while both development and full still run.
2. Run development in report-only shadow mode for at least twenty representative changes. Compare
   its selection against full failures and seed negative controls in every class.
3. Make local pre-push and ordinary CI use development; keep nightly/manual full and exact-release
   full. Retain an immediate `--full` escape for investigation.
4. Audit p50/p95, false negatives, widening rate, cache trust, receipts and release dependency.
   Close `A1-618` only after no undispositioned miss remains.

Rollback is one profile routing change: set the ordinary job's `APLAN_ORDINARY_PROFILE` from
`development` to `full`. The same value selects hosted `coverage` preparation and the
non-subtractive full execution route, and the ordinary job already retains the full job's
60-minute ceiling. The executable workflow audit admits only those two values; its mutation test,
the hosted preparation resolver and the preflight route prove the one-line rollback end to end.
The full scripts and manifest remain live throughout, so rollback does not reconstruct removed
checks. Any development/full disagreement defaults to full until classified and tested.

### Achieved rollout audit (2026-10-23)

The rollout corpus executes 24 paired development/full shadows rather than maintaining a prose
matrix. Each case changes one exact surface and injects one failing gate. Together they cover plan
structure, deferred evidence, documentation, all twelve workspace packages, schema and facts
generators, root and package manifests, dependency policy, validation policy, workflow routing and
unknown untracked input. The controls exercise all eleven gates and therefore all seven gate input
classes. A selected or
widened control fails both profiles. Three coverage/evidence controls are intentionally deferred by
development, name that deferral in the receipt and still fail full. There were no unexplained
disagreements.

Five sequential warm samples were taken on an otherwise idle host from clean `f9b5027c`, with one
temporary untracked probe per class. Nearest-rank p95 over five samples is the maximum; these
observations are not CI thresholds.

| class | selected shape | samples (seconds) | p95 | objective |
|---|---|---|---:|---:|
| plan/prose | `fmt spec style structural` | 6.451, 5.346, 5.508, 5.413, 6.673 | 6.673 | 10 |
| leaf package | `aplan-tui` closure and Rust gates | 41.437, 39.059, 47.531, 41.021, 41.417 | 47.531 | 60 |
| cross-cutting CI | all packages plus MSRV widening | 44.157, 56.807, 60.825, 60.864, 68.683 | 68.683 | 120 |

A supplemental `aplan-output` probe selected ten reverse-dependent packages and observed 55.729,
66.617, 46.061, 52.905 and 44.957 seconds. That is not the four-package leaf class; its 66.617s
p95 remains inside the cross-cutting objective.

The first production development receipt came from push run `39989744182` on `f9b5027c`. GitHub's
timestamps show a 2s queue and a 2m14s job. The receipt records 19s setup/classification, 23s
compile and 80s test phases, with provider and coverage explicitly deferred. Manual full run
`39990109489` used the same SHA, queued for 2s and completed in 7m15s. Its receipt records 12s
compile, 92s tests, 279s isolated coverage and 41s provider resolution; all ten then-current gates
and all twelve
coverage floors passed.

The adversarial audit also executed the cancellation-group model, full-inventory superset,
cache-free direct hosted path, isolated coverage ownership and exact-source release tests. No reuse
was reintroduced. The one-line rollback mutation changes `APLAN_ORDINARY_PROFILE` to `full`; the
same value resolves hosted preparation to `coverage`, routes all eleven gates through `preflight.sh`,
refuses a foreign checked-out head and retains the 60-minute full ceiling.

Three findings were repaired inside the rollout: the selector gained an explicit root for isolated
audits, validation fixtures gained the receipt inventory and helper they actually consume, and gate
children no longer inherit the outer receipt/summary controls. The hosted rollback defect described
above was repaired before closure. No budget miss or undispositioned selection disagreement remains.

A later plan-only closure push added an observational startup finding without changing that verdict.
Run `39994570639` spent 78.500 seconds between classifier input and outcome; the preceding plan-only
runs `39982781669`, `39991316902` and `39993750878` spent 15.148, 26.592 and 43.706 seconds. All four
classified correctly, and the acceptance objective above is deliberately the warm local profile,
not variable shared-runner wall time. However, `cargo run -q -p xtask` exposes classification to a
177-package normal dependency graph and the receipt cannot separate rebuild, metadata, Git and host
delay. `A1-651` instruments those phases and removes heavyweight build startup from the production
classifier while preserving the shadow corpus and fail-closed source-freshness boundary.

### Achieved compiler-free classifier boundary (2026-10-23)

The production entry point now runs the checked-out standard-library Python selector directly. It
does not invoke Cargo metadata at runtime. `cargo xtask validation` remains the single generator for
both dispatcher data and selector data: it reads the one reviewed YAML inventory, obtains Cargo's
resolved workspace graph, reverses workspace edges and writes a host-neutral JSON policy. That
policy binds `Cargo.toml`, `Cargo.lock`, Cargo/toolchain configuration and every member manifest by
SHA-256. Runtime refuses missing, linked, oversized, malformed, duplicate, unordered or stale
inputs before it produces a selection. The retained `cargo xtask validation-select` interface is
only a compatibility launcher to that same script and is deliberately not used by preflight.

Bounded stderr telemetry reports Python startup, inventory parsing, workspace-graph validation,
tracked-file census, Git subprocesses, classification, output and total time, plus Git, Cargo and
rustc process counts. Census and Git durations are exclusive rather than double-counted. A process
negative-control places failing Cargo and rustc shims first on `PATH`; the production selector still
passes, reports zero for each and starts only Git. The prior 23 development/full shadow pairs now
drive this entry point directly, including every gate input class and the NUL-delimited dispatcher
boundary.

Seven local fresh-process `HEAD^ -> HEAD` samples on Linux x86_64/Python 3.14.7 exercised six Git
processes apiece. The first sample was 164.516 ms; p50 was 167.711 ms and p95/max was 190.495 ms.
Warm p95 was also 190.495 ms. Every output had the same digest and every sample started zero Cargo
and zero rustc processes. Here "cache-free" means no Rust compiler cache can affect the result; it
does not claim an operating-system page-cache flush.

The measurement harness makes that evidence boundary executable. It requires a clean checkout,
resolves the head and first parent once, gives every sample those immutable object ids and compares
the commit, worktree and harness/selector/policy digests again after sampling. Selector reports
must name the same resolved revisions, timing fields must be finite values of the promised types,
and the Git process count must agree across every sample. Dirty or changing source, malformed
counters and non-finite limits refuse the run instead of being recorded beside an otherwise passing
percentile.

The immutable implementation checkpoint is `d87eaa49eefa205de40c1af2244dfeaeb938402a`.
Private-runner push run `40002782069` classified that source in 78.628 ms with seven Git and zero
Cargo/rustc processes, then passed its selected development profile. Measurement run `40002852524`,
persistent job `120583158344`, ran seven fresh Python 3.11.2 processes against the same checkpoint:
90.500 ms cold, 88.334 ms p50, 96.191 ms cache-free and warm p95, and 96.191 ms max. All outputs
shared one digest; every sample started six Git and zero Cargo/rustc processes.

The paired standard `ubuntu-latest` job never started repository code because GitHub rejected
billed hosted work under the private account's payment/spending state. It is not a timing sample.
At that private checkpoint the production workflow deliberately used the persistent runner and was
scheduled to change to standard hosted only at the public boundary, while `M-public` waited on this
hardening cut. Requiring the future environment there would have created a lifecycle cycle. `A1-652`, a blocking member
of `M-public`, repeats the same canonical harness on the actual standard hosted production path and
records that observation before the public cut closes.

That deferred compatibility check completed after publication in Actions run `41581679126`, job
`125737950459`, against immutable public `main` commit
`f720381e6fc14cc916eb80fc39c67a3555dfa06b`. The standard `ubuntu-latest` label resolved to
GitHub-hosted runner `GitHub Actions 1000033202`, Linux x86_64, Ubuntu 26.04.1, with Python 3.14.4.
The canonical `validation-select-performance.py --runs 7 --check` harness reported 86.015 ms cold,
86.538 ms p50, and 98.939 ms p95/max for both warm and cache-free distributions. All seven samples
produced the same output digest
`53e5c959203e0ba7ea93694ee975e640f5f3ebcf43932b9bc3cfb4b0cca058ac`, started exactly six Git
processes, and started zero Cargo and zero rustc processes. The clean source, harness, selector and
policy digests were respectively the commit above,
`6e76f8ff22bee799130b08bb5b09ee908bc88dedb2cdbf251d7d7ccd04679d75`,
`7d84a00afea0039c28b835a1015acdd3c115a874d3e976ee1df07394d897c2ae`, and
`15adc96263a4b69a873e57011c907d6afff6a51fe69a5e557c66cf2311e9ce92`.

The hosted p95 is less than one tenth of the one-second warm objective and less than one fiftieth
of the five-second cache-free objective. The public production environment therefore preserves the
compiler-free boundary with no compatibility accommodation. The measurement used a one-run
external-fork trigger that checked out the immutable source directly; it is removed rather than
becoming another standing workflow. The ordinary protected development workflow passed that same
revision in run `41578153087`, job `125726387644`, and remains the production verdict.

This is a repository-CI selection audit, not evidence-observation authority. `A1-647` still owns at
least twenty separate shadow runs for `A1-590`'s receipt-backed product evidence mode, including
forged, stale, cross-revision, partial and untrusted artifacts. Neither corpus contributes to the
other's threshold or justifies the other's assurance claim.

## Alternatives considered

| option | disposition | reason |
|---|---|---|
| Keep full pre-push and tell developers to use `PREFLIGHT_SKIP` | reject | arbitrary subtraction is unaudited and still leaves remote duplication |
| Run full only for tags | reject | deferred failures could accumulate until publication; nightly/manual full gives an earlier bounded signal |
| Use GitHub workflow path filters | reject | required skipped workflows can remain Pending, and a new file can evade a stale filter |
| Test only the directly edited crate | reject | workspace reverse dependencies and cross-cutting audits are first-class consumers |
| Cache coverage objects | reject | contradicts the merged-mapping failures proved by `A1-457` and cargo-llvm-cov's clean-default rationale |
| Add `verify --task` to make evidence verification fast | reject | a subset cannot honestly report whole-plan evidence health; keep selection out of the gate |
| Adopt nextest immediately | defer to measured spike | likely scheduling benefit, but doctest/custom-harness/process semantics need proof here |
| Pin no toolchain and accept moving Clippy | reject for release; decide for development | the current 1.98/1.99 split already produced different verdicts |
| Split profiles into more hosted jobs | reject after `A1-616`/`A1-617` | parallelism duplicates preparation/build work; no safe executable-artifact reuse erased that cost, and the single stable development verdict is simpler |
| Prebuild the existing `xtask` before classification | reject | moves the same 177-package exposure earlier in the job and leaves checkout-to-verdict latency unchanged |
| Split a minimal Rust selector crate from `xtask` | reject | reduces dependencies but still starts Cargo and rustc after every fresh checkout; it does not meet the compiler-free deterministic boundary |
| Check in or trust a cached native selector executable | reject | introduces platform artifacts and a stale-source/trust problem before the selector can decide what to validate |
| Keep Rust and Python classifiers with a parity test | reject | two implementations can agree on the same omission and double the policy surface; the shadow corpus must drive the production entry point |
| Run `cargo metadata` inside the Python selector | reject after negative control | Cargo metadata itself started `rustc -vV`; generated content-bound graph data preserves Cargo as the graph authority without crossing the runtime compiler boundary |
| Generate a content-bound graph and run one standard-library selector | accept | one reviewed inventory, one classifier, host-neutral generated data, fail-closed freshness and no runtime compiler process |

## Delivery order

1. `A1-608`, `A1-609` and `A1-620`: restore current CI portability (delivered in the proposal
   audit).
2. In parallel, `A1-619` establishes one exact development/release toolchain and `A1-611` creates
   the executable profile inventory and assurance-order audit.
3. `A1-614`: close the release/full enforcement gap immediately after profiles and the toolchain
   identity exist.
4. In parallel, `A1-610` evaluates nextest and `A1-612` builds the affected-surface classifier.
5. `A1-613` makes the local development profile and hook the default; `A1-615` makes the
   structural/evidence split explicit.
6. `A1-650` implements the admitted shared test gate and its dependency/configuration proofs.
7. `A1-616` admits only hosted reuse that survives latency and trust measurements; its evaluation
   selected a shared build profile and rejected caches and archive fanout.
8. `A1-617` changes Actions cadence after every underlying profile is executable: one stable
   development result for pushes/pull requests, and uncancellable full runs nightly/on dispatch.
9. `A1-618` runs the shadow comparison, mutation audit and latency review, then records the achieved
   budgets and any follow-up.

`A1-590` remains the owner of persistent evidence observations. It can later reduce authoritative
provider refresh cost, but the cadence improvement does not wait on that larger product change.

## Primary references

- Cargo package/workspace test selection and default-member behavior:
  <https://doc.rust-lang.org/cargo/commands/cargo-test.html>
- Cargo workspace sharing and package selection:
  <https://doc.rust-lang.org/cargo/reference/workspaces.html>
- Cargo metadata's resolved dependency graph:
  <https://doc.rust-lang.org/cargo/commands/cargo-metadata.html>
- Cargo profile inheritance, dependency overrides and selection:
  <https://doc.rust-lang.org/cargo/reference/profiles.html>
- Rustup's committed toolchain override:
  <https://rust-lang.github.io/rustup/overrides.html>
- cargo-llvm-cov, including its clean/default object behavior and nextest integration:
  <https://github.com/taiki-e/cargo-llvm-cov>
- Nextest execution model and custom-harness tradeoff:
  <https://nexte.st/docs/design/how-it-works/>
- Nextest coverage/doctest limitation:
  <https://nexte.st/docs/integrations/test-coverage/>
- Nextest build archives and exact-revision reuse:
  <https://nexte.st/docs/ci-features/archiving/>
- GitHub skipped-workflow required-check behavior:
  <https://docs.github.com/en/actions/how-tos/manage-workflow-runs/skip-workflow-runs>
- GitHub workflow concurrency cancellation:
  <https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency>
- GitHub dependency-cache scope and low-trust write restrictions:
  <https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching>
- GitHub reusable workflows:
  <https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows>
