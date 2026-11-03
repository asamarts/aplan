# Nextest admission evaluation

Status: accepted and implemented by `A1-650`; hosted archive reuse was rejected by `A1-616`.

Date: 2026-10-22

This document evaluates whether nextest can replace Cargo's test execution in aplan's development
validation loop without weakening what that loop proves. It is deliberately narrower than a generic
runner comparison. A fast runner that omits doctests, loses CLI invocation evidence, changes a
process-sensitive test, or makes a shared-workspace race probabilistic is not an improvement for
this repository.

## Decision

Admit cargo-nextest 0.9.143 as the non-doctest executor in the repository's shared test gate,
subject to all of these conditions:

1. Run one nextest process with the repository configuration shown below. Do not shard it.
2. Run an explicit Cargo doctest stage after nextest. Nextest does not run doctests.
3. Keep retries at zero and treat flaky results as failures.
4. Give the CLI invocation recorder and parsed-plan cache fresh owner paths, then run the existing
   252-member CLI-surface audit against that run's ledger.
5. Install the checksum-pinned prebuilt nextest binary. Do not make nextest a workspace dependency
   or compile it with the workspace MSRV toolchain.
6. Run this same complete test gate in development, full and release profiles. Keep full/release
   coverage on the existing isolated Cargo runner; do not change coverage as part of this adoption.

The admitted execution shape is therefore equivalent to:

```sh
cargo nextest run --workspace --test-threads num-cpus --retries 0 --flaky-result fail
cargo test --workspace --doc
cargo run -q -p xtask -- cli-surface --check
```

The executable implementation lives in the one shared gate, preserving the profile-superset
invariant. `A1-616` evaluated and rejected hosted archive reuse without changing the runner
contract. `A1-617` routes profiles while preserving the commands, environment, inventory and
failure behavior proved here.

Reject these alternatives:

- unconfigured nextest, because it produced a real workspace-source race;
- an eight-test concurrency cap, because both p50 and p95 were materially slower than the 24-CPU
  default without buying a correctness benefit;
- multi-process partitions, because nextest test groups are logical mutexes within one run and do
  not coordinate separate shard processes;
- nextest as the coverage runner, because the current Cargo gate is not on the development critical
  path and process-per-test execution changed the per-crate covered-line set;
- automatic retries or a flaky-pass result, because either would turn failed evidence green;
- archive fanout without hosted-runner measurement, because a correct 181 MB archive can still cost
  more to transfer than it saves.

## Evaluated inputs

| input | value |
|---|---|
| aplan source | `85903555e73fad0e5dd09553a4cdcddb0894777c` |
| relevant predecessor | `A1-630`, which consolidated 134 integration binaries into nine |
| cargo-nextest | 0.9.143, commit `60fa45f638ffc3f35e74afa65737f45fcd32db2a` |
| nextest Linux x86-64 archive SHA-256 | `66786b9abe23920d022a182d1416b1bbc8130dd4872a9553d76985a1708dcd1e` |
| cargo-llvm-cov | 0.8.7 |
| Rust/Cargo used for the measurement | 1.99.1 |
| workspace MSRV | 1.88 |
| host | Linux x86-64, 24 logical CPUs, 62 GiB RAM |
| build orchestration | `mbx`, using the repository target and its shared compiler cache |

The host was shared. Load varied from roughly 9 to 65 during the study. The benchmark therefore
uses paired, interleaved samples on one revision and records contended data separately; it does not
pretend that shared-host wall time is a stable laboratory constant. Every command had a 180-second
deadline except coverage, which had 300 seconds. No accepted sample timed out.

The evaluated release declares Rust 1.91 as its source MSRV, three releases above aplan's 1.88
floor. The official release supplies platform archives and checksum files. Installation must mirror
the repository's cargo-llvm-cov practice: derive one version pin, select the host archive, verify a
committed SHA-256, install the binary, and print the observed version. This keeps the runner out of
`Cargo.lock`, avoids coupling the consumer MSRV to a CI utility, and makes a changed upstream asset
a loud failure rather than a mutable input.

Primary references:

- release and assets: <https://github.com/nextest-rs/nextest/releases/tag/cargo-nextest-0.9.143>
- process-per-test design: <https://nexte.st/docs/design/why-process-per-test/>
- scheduler design: <https://nexte.st/docs/design/how-it-works/>
- configuration and retries: <https://nexte.st/docs/configuration/reference/>
- test groups: <https://nexte.st/docs/configuration/test-groups/>
- archives: <https://nexte.st/docs/ci-features/archiving/>
- partitions: <https://nexte.st/docs/ci-features/partitioning/>
- coverage and doctest limitation: <https://nexte.st/docs/integrations/test-coverage/>
- cargo-llvm-cov integration: <https://github.com/taiki-e/cargo-llvm-cov>

## Method

### Inventory

Cargo's `--list` output and nextest's machine-readable list were normalized to the tuple
`kind/binary/test`, sorted, compared byte for byte, and hashed. Ignored tests were retained in the
inventory even though default execution skips them. Cargo's documentation harnesses were listed
separately because nextest does not discover or execute them.

### Timing

Each series contains five samples. p50 is the middle observation after sorting. With only five
observations, p95 is conservatively reported as the maximum rather than interpolated. Warm and
contended series reused compiled artifacts and alternated the two runners. Cold samples used fresh
detached worktrees and fresh repository targets while retaining only mbx's machine-wide compiler
cache, which is the host behavior the repository actually uses. Each runner received a fresh
`APLAN_CACHE_DIR` and CLI invocation ledger per sample.

The Cargo control was `cargo test --workspace`. The corrected nextest candidate used 24 test slots,
zero retries, failing flaky results, and the source-workspace mutex below. The eight-slot series was
an explicit alternative, not silently mixed into the candidate data. A separate five-sample Cargo
doctest series measures the cost nextest cannot replace.

### Correctness controls

Correctness was tested four ways:

- exact inventories, including ignored tests and Cargo's doctest inventory;
- direct execution of seven process-sensitive surfaces under both runners;
- seeded failures in ordinary, trycmd and signal/process-lifecycle tests;
- repeated complete suites with fresh runtime state, plus archive, partition and coverage trials.

The candidate was allowed no retry. A negative control counted only if the runner itself returned a
failure status and named the seeded test failure.

## Inventory result

The normalized Cargo and nextest inventories both contain 2,904 non-doctest tests across 22 test
binaries. Their sorted SHA-256 is identical:

```text
54148561d995d2df44342eb65906a3459a8e641bd438984b509d4e0baec93a76
```

Default execution runs 2,899 and skips these five ignored measurements:

- `tests::measure_production_identity_observation_against_complete_refresh`
- `tui_journey::configured_real_plans_open_every_read_destination_in_a_real_terminal`
- `scale::check_does_not_care_how_wide_a_cut_is`
- `large_scene_construction_viewport_and_hit_testing_baseline`
- `shell::tests::oversized_graph_viewport_render_latency_baseline`

Cargo currently registers zero doctests, and nextest exposes no doctest binaries. Zero is not a
reason to erase the stage: a future documentation example would otherwise disappear from the
development contract without changing the runner configuration. Five warm Cargo doctest samples
were 1.628, 1.661, 1.667, 1.676 and 1.801 seconds (p50 1.667, p95 1.801).

## Latency result

All values are wall seconds. `Candidate` is nextest alone. `With docs` charges the measured warm
Cargo doctest p50/p95 to show the complete admitted runner pair. The identical CLI-surface audit
ran after every complete suite as a correctness condition but outside both runner stopwatches, so
these are runner comparisons rather than end-to-end profile timings. Charging the warm doctest
stage is conservative for cold samples because nextest has already built the workspace before the
documentation stage.

| condition | Cargo p50 | Cargo p95 | candidate p50 | candidate p95 | with docs p50 | with docs p95 | candidate failures |
|---|---:|---:|---:|---:|---:|---:|---:|
| warm, ambient low contention | 33.700 | 34.214 | 18.344 | 19.476 | 20.011 | 21.277 | 0/5 |
| warm, ambient contention | 41.369 | 48.663 | 24.161 | 25.213 | 25.828 | 27.014 | 0/5 |
| fresh worktree and repository target | 77.619 | 103.021 | 62.843 | 73.561 | 64.510 | 75.362 | 0/5 |
| warm, eight execution slots | 48.011 | 64.928 | 31.849 | 50.306 | 33.516 | 52.107 | 0/5 |

Before charging doctests, nextest improved warm p50/p95 by 45.6%/43.1%, contended p50/p95 by
41.6%/48.2%, and cold p50/p95 by 19.0%/28.6%. After charging the mandatory doctest stage, the warm
improvement remains 40.6% at p50 and 37.8% at p95. The eight-slot candidate is 73.6% slower at p50
than the 24-slot candidate and has a much worse tail, with no observed correctness improvement.

Raw samples in milliseconds:

| condition | Cargo | nextest |
|---|---|---|
| warm | 33108, 34214, 33959, 33646, 33700 | 18185, 19103, 19476, 18344, 18324 |
| contended | 39945, 48663, 46819, 41268, 41369 | 25213, 24734, 21579, 19764, 24161 |
| cold | 76869, 103021, 79209, 75824, 77619 | 62843, 60317, 59659, 73561, 64418 |
| eight slots | 60463, 64928, 48011, 45388, 40619 | 50306, 30345, 35368, 31849, 30700 |

Twenty complete Cargo suites and twenty corrected nextest suites passed across these four series.
Every passing full-suite sample produced a complete 252-member CLI audit. The configured nextest
runs had zero lost-input, teardown-timeout, descendant-leak, fixture-publication or source-change
failures. This does not excuse the unconfigured failure described next; it establishes that the
candidate configuration repaired the one-process schedule in the measured series.

## The source-workspace race

The first unconfigured nextest full suite failed 1/1. It scheduled
`untracked::the_generators_note_an_untracked_plan_file`, which deliberately creates
`aplan/a1-144-probe.md`, alongside
`timings::evidence_gate_corpus_exercises_the_rolestat_shape_and_fault_control`, which fingerprints
the Git workspace around a long-running sample. The latter correctly refused its measurement:

```text
evidence-gate-performance: failing-runner Git source changed while its sample was running
```

Cargo had not reproduced the overlap because its consolidated binary schedule differs. This was
not classified as a flaky benchmark or hidden with a retry. Both tests interact with the same
workspace-source resource, so the candidate configuration names that resource explicitly:

```toml
[test-groups]
workspace-source = { max-threads = 1 }

[[profile.default.overrides]]
filter = 'test(the_generators_note_an_untracked_plan_file) | test(evidence_gate_corpus_exercises_the_rolestat_shape_and_fault_control)'
test-group = 'workspace-source'
```

`cargo nextest show-config test-groups` confirmed that the group contains exactly those two tests.
The 20 corrected whole-suite repetitions then passed.

This mutex has a hard boundary: it is local to one nextest run. Slice partitioning puts the observer
in slice 1/2 and the mutator in slice 2/2 (and in slices 3/3 and 1/3 respectively). One concurrent
two-slice run happened to pass, which is exactly the dangerous result if treated as proof. An
adversarial two-process control ran the observer once while a second nextest process ran the mutator
16 times. The mutator passed every repetition and the observer failed after 10.273 seconds with the
same source-change refusal. Therefore:

- one configured nextest process is admitted;
- partition listing and sequential partitions are useful diagnostic tools;
- parallel partitions are rejected even when an individual run is green;
- sharding can be reconsidered only after workspace mutation becomes hermetic or a cross-process
  resource coordinator is designed and proved.

## Process-sensitive parity and negative controls

Each row passed directly under Cargo and nextest, in addition to its presence in every complete
suite:

| surface | proving test |
|---|---|
| trycmd corpus | `cli::cli_corpus` |
| invocation recording | `spawn_recording::a_spawn_through_the_helper_reaches_the_recorder` |
| real PTY | `tui_journey::graph_explorer_runs_end_to_end_in_a_real_terminal` |
| signal and process group | `cli_termination::noninteractive_signals_stop_configured_groups_and_preserve_the_signal` |
| Git observation | `revision::summary_at_head_is_identical_to_the_clean_worktree_answer` |
| concurrent ID/coordination | `claim::exactly_one_of_eight_simultaneous_claimants_is_told_it_holds_the_task` |
| custom environment | `color::the_environment_contract_still_holds` |

In a disposable worktree, a runtime assertion failure was seeded separately into an ordinary core
invariant, the trycmd corpus and the signal/process-lifecycle test. Cargo rejected all three with
status 101; nextest rejected all three with status 100. The status numbers are runner conventions,
not a semantic difference: in every case the command failed and identified the seeded test.

## Archive result

`cargo nextest archive` produced a 181,256,855-byte zstd archive containing 24 binaries (22 test and
two non-test), two build-script output directories and one standard library. With artifacts needing
a rebuild after coverage instrumentation, build plus archive took 33.660 seconds; nextest reported
the archive-writing portion itself as 0.570 seconds. Extraction took 1.460 seconds. A full replay
from an on-disk extraction directory ran all 2,899 active tests in 18.334 seconds, took 20.160
seconds wall including extraction/setup, and again exercised 252/252 CLI members.

`A1-616` then measured archive fanout on standard `ubuntu-latest` runners. The first valid
untrimmed build took 483 seconds and produced 947,974,573 bytes; upload took about 7 seconds,
download 50 seconds and raw extraction 41 seconds. That shape is rejected on size alone: it costs
nearly the complete no-cache compilation again in transfer, and each exact-source artifact is more
than five times the optimized archive measured below.

A shared immutable-CI Cargo profile removed incremental state and dependency debuginfo while
retaining workspace line tables. In run `39910657606`, build plus archive then took 361 seconds and
produced 187,367,950 bytes (187,368,226 bytes as the uploaded artifact); upload took about 3 seconds,
download 5 seconds and raw extraction 2 seconds. The size is close to the local 181 MB observation
and removes the untrimmed archive's accidental debug payload, but it still does not make fanout a
win. Exact-source build plus upload, a dependent job's setup/download/extraction, the archived
nextest execution and the mandatory CLI census exceed the same run's 600-second direct-job wall
lower bound even before charging Cargo doctests. The CLI census cannot move to the builder: it reads
the invocation ledger created by the archived test processes.

The adversarial replay also found why a green local extraction was insufficient. The consolidated
aplan and xtask integration harnesses used compile-time `CARGO_BIN_EXE_*` paths, which continued to
name vanished build-host targets after extraction. Run `39911820692` proved the first repair by
passing 2,756 archived tests before the first remaining xtask path failed; a source-wide search then
found and repaired all three xtask call sites rather than waiting for them to fail one at a time.
Both harnesses now prefer nextest's runtime-remapped executable variables and retain compile-time
values only for runners that provide no runtime contract. Run `39913194064` then rebuilt
187,374,044 bytes in 464 seconds, uploaded in about 3 seconds, downloaded in 4 seconds, extracted in
3 seconds, passed all 2,915 archived tests in 138 seconds and passed doctests in 258 seconds. Its
temporary workflow used the obsolete `APLAN_CLI_INVOCATIONS` variable, so the CLI census correctly
refused after compiling instead of reporting a vacuous pass. Run `39915218903` corrected that final
fixture input: its builder spent 459 seconds producing 187,374,135 bytes, and the dependent replay
extracted in 3 seconds, passed all 2,915 archived tests in 145 seconds (147 seconds including the
wrapper), passed doctests in 296 seconds, and exercised all 252 CLI members from 3,434 invocations
in 171 seconds. Builder plus replay was already more than eighteen minutes, nearly twice the
600-second direct-job observation, without relying on the invalid trials. Manual-remap,
missing-extraction-directory, stale-roadmap, compile-time-path and stale-ledger-name failures are
invalid trials, not evidence against nextest; their completed phase receipts still establish the
hosted lower bound.

Archives remain technically admissible only when bound to the exact source revision, target
platform, runner version and workspace remap. They do not preserve Cargo doctests, and the archive
is source-specific executable content rather than a cross-trust cache. Production therefore retains
direct execution. Reconsideration requires a new same-run measurement that includes the CLI census
and doctests; archive size or nextest runtime alone is not the decision metric.

## Coverage result

The valid coverage trial used a fresh dedicated `llvm-cov-target` under the repository's on-disk
target mount, never a shared coverage-object directory. An earlier setup placed that directory under
the host's `TMPDIR`; the repository's fixture guard correctly rejected the setup because fixtures
would then live under temporary scratch. That invalid setup is not counted as a runner failure.

`cargo llvm-cov nextest` with the candidate configuration ran 2,899/2,899 active tests, retained the
252/252 CLI audit, and reported 85,129/88,513 covered lines (96.18%). Every crate remained at or
above its committed floor:

| crate | Cargo coverage | nextest coverage | delta covered |
|---|---:|---:|---:|
| aplan | 8367/8713 | 8368/8713 | +1 |
| aplan-app | 11814/12142 | 11824/12142 | +10 |
| aplan-attest | 526/530 | 526/530 | 0 |
| aplan-config | 2483/2527 | 2483/2527 | 0 |
| aplan-core | 17031/17376 | 17031/17376 | 0 |
| aplan-e2e | 105/108 | 105/108 | 0 |
| aplan-evidence | 10248/10448 | 10248/10448 | 0 |
| aplan-interop | 5619/5800 | 5619/5800 | 0 |
| aplan-mcp | 3024/3140 | 3016/3140 | -8 |
| aplan-output | 6252/6376 | 6249/6376 | -3 |
| aplan-tui | 13305/14443 | 13305/14443 | 0 |
| xtask | 6353/6910 | 6355/6910 | +2 |

The total differs by only two covered lines, but the compensating per-crate changes matter: an
aggregate can hide a loss. Process-per-test execution changes which startup, shutdown and shared
state paths execute. Because coverage is a full gate rather than a development-loop cost, there is
no reason to exchange the current known measurement for a different one. Keep the Cargo coverage
gate and its run-owned object allocation. If coverage ever changes runners, it needs its own floor
transition review rather than inheriting this development-runner decision.

Nextest still cannot run doctests. cargo-llvm-cov documents a separate nightly-only merge procedure
for doctest coverage; this repository does not need to acquire that toolchain complexity merely to
adopt faster ordinary tests. The authoritative test contract keeps Cargo doctest execution, while
the existing coverage contract remains exactly as it is.

## Implemented contract and maintenance checks

`A1-650` makes the admitted constraints executable rather than leaving them in this document:

- `ci/nextest.env` pins nextest 0.9.143, its upstream commit and every supported archive checksum;
- the shared installer verifies the archive before extraction and the binary's observed release and
  commit before repository work; both test and release workflows invoke it on private and hosted
  runners, so the private image preload is only a latency optimization;
- `.config/nextest.toml` defines the mutex, while the gate audits nextest's resolved membership and
  serial limit rather than trusting TOML text;
- zero retries and fail-on-flaky behavior are explicit, and the gate exposes no partition route;
- unique invocation/cache paths and the complete CLI-surface audit remain part of the same run;
- every workspace/package selection drives both nextest and matching Cargo `--doc` selection;
- Cargo and nextest inventories, including ignored state, are compared before execution and their
  canonical digest is printed;
- version, config and inventory identities and separate runner-plus-doctest/CLI-audit timings
  appear in gate output; and
- coverage retains its isolated Cargo runner.

Implementation verification on 2026-10-22 compared 2,914 registered tests across 22 suites,
including five ignored tests; the Cargo and nextest inventories had the same canonical SHA-256
(`76042c68b6f7aa8bef1183d403bce719625e33a26b1a728bb1b18410c2f08040`). Two complete
post-implementation workspace runs passed after restoring the generated roadmap baseline. The
final run executed 2,909 active tests, every Cargo doctest harness and all 252 CLI-surface members.
Its final exact-profile receipt reported 32 seconds for runner plus doctests, 4 seconds for the
common CLI audit and 40 seconds for the complete gate. A later warm pre-push run reported 24, 2 and
29 seconds respectively. An earlier contended run reported 43 seconds for runner plus doctests,
26 seconds for the common CLI audit and a separate 45.82-second compile/list stage; the gate keeps
that setup cost visible instead of presenting runner latency as end-to-end latency. These samples
are observations from this host, not hosted-runner performance claims. A package-scoped
`aplan`/`xtask` run also passed with 1,283 active tests and the applicable two-member mutex. Ten
further repetitions of all seven process-sensitive surfaces passed with zero retries. Integration
negative controls rejected wrong or absent runners, corrupt archives, malformed or semantically
different inventories, stale, missing or non-serial mutex membership, ordinary-test and doctest
failures, and CLI-audit failure.

With that implementation in place, `A1-616` measured and rejected hosted archive fanout. The
ordinary test job retains direct execution, and neither that decision nor the absence of hosted
dependency caching alters the runner contract.

Upgrade maintenance is intentionally explicit. A nextest version change re-runs inventory parity,
the seven sensitive surfaces, seeded failures, the source-mutation stress control, archive replay
and latency samples. A configuration change re-runs the same controls except installation. A new
workspace-mutating or source-observing test joins the named resource group in the same commit, or
the development runner is no longer the admitted profile.
