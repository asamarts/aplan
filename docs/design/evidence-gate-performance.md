# Evidence-gate performance and trust model

Status: accepted design and completed rollout audit (`A1-648`, `A1-647`). Interactive observations,
complete batch execution, bounded provider scheduling, portable receipts, current-only consumption
and fail-closed incremental refresh are implemented by `A1-590` and `A1-642` through `A1-646`.
Production shadow, hostile-control, direct-parity, local-latency and same-workflow artifact checks
all pass. The receipt-backed path is authoritative within the trust and identity boundaries below;
direct execution remains its independent oracle and one-switch rollback.

## Decision

The ordinary whole-plan evidence gate will resolve an exact plan from portable, content-bound
provider observations. It will not replay every cited test. Direct execution remains an independent
nightly, release and manual oracle.

These are different operations with different honest budgets:

- structural `check` parses and validates the plan without opening Git or starting providers;
- repository resolution checks Git and file evidence;
- provider discovery obtains complete test catalogues;
- direct execution runs selected tests and is bounded by product-suite cost;
- observation transport validates results produced by component jobs;
- exact-plan resolution applies current observations to every citation in one exact plan snapshot.

No mode may call deferred, partial, stale, incrementally refreshed or receipt-backed work "direct
full execution". A whole-plan verdict describes its scope; provenance describes where its provider
facts came from. Conflating those axes caused the original performance problem: a correct whole-plan
gate was treated as if it had to repeat all component work itself.

## Measured problem

P0000021's 256-task plan had 241 test citations over 199 distinct test identities and eight
configured runners. A warm structural check took 0.101 seconds. Git/file verification without
providers took 0.861 seconds. Warm provider catalogue discovery took 27.017 seconds locally and 2
minutes 58 seconds in one CI run. Direct L3 then ran 199 commands serially for 39 minutes 36 seconds,
after component jobs had already run the same suites.

The executable corpus in `ci/scripts/evidence-gate-performance.py` fixes that shape as
`140, 16, 21, 9, 6, 1, 5, 1` distinct tests across eight runners, plus 42 repeated citations. On the
development binary used while accepting this design, the no-delay synthetic direct case still took
109.4 seconds because it started 215 processes: eight catalogues, 199 exact tests and eight no-match
probes. The test bodies did no product work. This is process-protocol cost, not a Rust parsing cost.
Each measured case establishes its own provider state: cold samples remove every warm marker, while
warm, slow, failing and direct samples explicitly prepare the same warm catalogue state. Selecting a
case alone therefore measures the same fixture as selecting it in the complete suite.

## Phase model and telemetry

Every profile emits one versioned phase record. A phase that did not run has `state: not_run` and a
reason; it does not report zero work as if it ran quickly. Durations use a monotonic clock and are
observational. Counts and identities are correctness-bearing.

| phase | work | duration field | deterministic counters |
|---|---|---|---|
| `input` | load config, plan and exact input identity | `input_ms` | config loads, plan loads, candidate/read/parsed bytes and cache reuse |
| `structural` | validate tasks, graph, traces and policy | `structural_ms` | tasks, graph builds, trace reads/parses and findings |
| `repository` | open Git and resolve commit/file facts | `repository_ms` | Git opens, revisions, tree reads, file citations and repository findings |
| `catalogue` | obtain or import complete provider catalogues | `catalogue_ms` | provider spawns, imported catalogues, catalogue identities and listed tests |
| `execution` | directly run selected provider identities | `execution_ms` | batch spawns, legacy exact-test spawns, probes, selected identities and outcomes |
| `observation` | validate, authenticate and import/export provider observations | `observation_ms` | receipts, bytes, signatures, accepted/rejected observations and outcomes |
| `resolution` | bind all facts to every citation in the exact plan | `resolution_ms` | citations, reused outcomes, ambiguities, coverage checks and findings |
| `render` | serialize the verdict and telemetry | `render_ms` | output bytes and records |

`total_ms` covers process startup through completed output. `unattributed_ms` is total minus named
phases and must not become a bucket large enough to hide provider orchestration. Provider queue time,
adapter startup and test-body time are subfields of `catalogue` or `execution`; dependency install,
checkout and artifact download are workflow phases outside the aplan process and are reported beside,
not inside, the evidence-gate number.

The blocking regression surface is:

- exact provider-process spawns by operation;
- catalogue imports and catalogue command spawns;
- batch and legacy exact-test spawns;
- selected distinct test identities and no-match probes;
- imported and reused outcomes;
- citations resolved and findings produced.

All externally visible records and diagnostics are sorted after concurrent work by provider identity,
test identity, task id and diagnostic order. Wall-clock min/p50/p95/max is retained for rollout and
capacity decisions, but shared-host latency is not a flaky unit-test assertion.

## Provider scheduling boundary

ADR-0055 makes concurrency explicit. Existing single-runner and sequence configurations have
capacity one. The scheduled `test.schedule` / `test.runners` form supplies one positive weighted
capacity; each runner consumes one unit by default and can declare a larger `weight` for nested
fanout. Optional `exclusive` names are capacity-one resources. A job reserves its full weight and
all names atomically, so no partial acquisition can deadlock a peer.

Catalogue commands are independent jobs. During direct execution, one runner's skipped-list and
batch commands form one job and keep one reservation; legacy per-identity execution remains serial.
The same schedule is used by verify, discovery, explicit observation refresh, MCP and the TUI. It
does not replace each command's cgroup memory/process caps and it does not coordinate another aplan
process. Cancellation prevents another admission, kills/reaps active configured-command process
groups through A1-592, drains their completions and leaves an incomplete observation unpublished.

## Two identities, not one cache key

A provider observation and an exact-plan resolution are different objects.

### Provider execution identity

A reusable provider observation binds all material that can change its answer:

- receipt schema and verifier protocol versions;
- provider name, directory, complete runner policy and adapter command;
- adapter and invoked tool binary digests, including tools outside the repository;
- complete catalogue digest and exact selected identities;
- declared material input tree, generated inputs and dependency lock state;
- toolchain, operating-system/platform contract and relevant environment variables;
- typed outcome, retry/flake/skip semantics and a `complete` marker.

Wall-clock time and source revision are audit context, not freshness by themselves. A result may cross
a revision only when every material identity remains exact and configured trust policy permits it.
An undeclared mutable tool, host service, environment value or generated input makes the identity
incomplete and therefore non-reusable. This is the blind spot Bazel documents for compilers or
environment outside an action definition; aplan refuses rather than guesses.

### Exact-plan resolution identity

The derived verdict binds:

- exact plan and configuration bytes;
- repository HEAD/tree and dirty-worktree identity relevant to repository resolution;
- evidence policy and verifier version;
- the exact accepted provider observations;
- every citation, ambiguity, locality and task-level evidence rule.

A plan-only edit always invalidates this verdict and reruns resolution. It may reuse an unchanged
provider observation; it may never reuse the old whole-plan verdict. Observation publication and
the newly derived verdict are atomic. A cancelled or partial refresh cannot replace a current one.

## Provenance vocabulary

Machine output carries `scope`, `provider_source`, `freshness` and `completeness` separately.

| field | admitted values |
|---|---|
| `scope` | `whole_plan`, `selected_tasks`, `selected_providers` |
| `provider_source` | `direct`, `receipt`, `incremental_refresh`, `deferred`, `none` |
| `state` (human label: `freshness`) | `current`, `stale`, `not_observed`, `refresh_failed`, `incompatible`, `untrusted`, `fault`, `unsupported` |
| `completeness` | `complete`, `partial`, `cancelled` |

`whole_plan + receipt + current + complete` is an authoritative receipt-backed verdict after the
rollout audit. It is not direct execution. `whole_plan + incremental_refresh` says the final plan was
fully resolved while only invalidated provider work ran. `deferred` says configured provider work
was deliberately not executed by this observation; `none` is structural or repository-only work.
Neither can be labelled evidence-verified. Human output uses the same terms rather than a shorter,
more flattering vocabulary.

Profiles have these meanings:

- `structural`: provider work deferred; never authoritative for provider evidence;
- `receipt-current`: fail if any required observation is absent, stale, partial or untrusted;
- `refresh-missing`: refresh required provider observations, then derive a whole-plan verdict;
- `direct`: run every required provider identity through the direct protocol;
- `shadow`: compute a fast result and a direct result independently, then compare them.

No fast profile silently falls back to direct replay. That turns a seconds-scale command into an
unbounded command and makes a timeout-dependent assurance claim.

ADR-0058 implements `refresh-missing` as an explicit Git-bound operation. Provider reuse starts at
the complete-runner boundary. Optional `test.inputs_by_test` declarations may narrow execution only
when every current test and every relevant changed path has exactly one dependency owner, and the
path is inside the complete provider material declaration. Missing, overlapping, newly introduced
or outside-material inputs widen to the runner. The final resolver still judges the whole plan.
Manifest-last publication preserves the prior complete set on failure; local refresh cannot replace
an authenticated artifact or signed receipt. `--refresh-shadow` compares the direct oracle without
claiming direct provenance or rollout authority.

## Performance budgets

| boundary | target | enforcement |
|---|---:|---|
| cached structural `check` | p95 below 100 ms | observed on controlled release baselines; deterministic read work gates regressions |
| local receipt-backed whole-plan verify | p95 below 1 s | rollout target, zero provider spawns is blocking |
| CI receipt-backed whole-plan verify after artifacts are available locally | p95 below 10 s | rollout target including parsing, authentication and exact-plan resolution; transfer is measured by the next row |
| P0000021 post-setup evidence-gate path | below 60 s | rollout target including receipt artifact transfer, excluding job queue/setup/install |
| direct product-test replay | no seconds-scale promise | report catalogue, adapter, queue and test phases; nightly/release/manual budget |

On the shared development host, the 15-run release-build `receipt-current` audit on 2026-10-24
imported eight catalogues and 199 outcomes, resolved 241 citations with zero provider spawns, and
measured 54.65 ms minimum, 60.26 ms p50 and 131.67 ms p95/maximum. Median import phases were 0.042
ms for the manifest, 2.738 ms for material measurement, 1.549 ms for parsing and below timer
resolution for identity, authentication and catalogue projection. These wall times characterize a
shared host; the deterministic counts are the blocking invariant. The same-workflow result below
independently demonstrates both CI budgets.

## Executable corpus and negative controls

Run the current baseline with:

```sh
python3 ci/scripts/evidence-gate-performance.py \
  --binary target/release/aplan --profile release --runs 15
```

Add `--include-direct` only for the periodic oracle measurement. That switch runs three separately
named execution cases: legacy per-identity replay, serial complete-result batches and those same
batches under aggregate capacity four. The harness uses `~/.tmp` on this
host, fingerprints the complete Git source state, executable, protocol, policy, runner/catalogue
material and environment,
rechecks source, executable and policy before and after every sample, validates that imported
fingerprints are lowercase SHA-256 values, and rejects mixed identities. Its generated repository
has:

- eight complete, disjoint catalogues and 199 exact names;
- 241 citations, so repeated citations prove fanout without repeated execution;
- cold and warm catalogue states;
- one independently slow runner and one failing runner;
- a no-provider control that must spawn nothing;
- optional legacy, batched-serial and bounded-concurrent runs over the same identities and injected
  per-batch work. Exact event counts are blocking; their wall-time difference is characterization.

The release-build parity sample on 2026-10-24 produced one identical verdict digest and exit status
in all three modes after removing only volatile observation identities and observation time. State,
scope, provider source, completeness, resolver, strength, outcome, detail and refresh semantics stay
in the comparison (`A1-659`). Legacy replay took 63.087 seconds and 215 provider processes;
batched serial took 4.629 seconds and 16 processes; bounded concurrent took 1.206 seconds and 16
processes. The harness now refuses a parity report when any exit status or verdict digest differs,
and the refusal is mutation-tested independently of those characterization timings.

The current and intended deterministic work table is:

| case | provider spawns | catalogues | batches | exact-test processes | probes | selected tests | citations |
|---|---:|---:|---:|---:|---:|---:|---:|
| structural / no-provider | 0 | 0 | 0 | 0 | 0 | 0 | 241 |
| current catalogue verify | 8 | 8 | 0 | 0 | 0 | 0 | 241 |
| current direct replay | 215 | 8 | 0 | 199 | 8 | 199 | 241 |
| batched serial direct | 16 | 8 | 8 | 0 | 0 | 199 | 241 |
| bounded-concurrent direct | 16 | 8 | 8 | 0 | 0 | 199 | 241 |
| receipt-current | 0 | 8 imported | 0 | 0 | 0 | 199 outcomes | 241 |

The receipt producer and current-only consumer add executable negative controls for missing, malformed,
truncated, duplicate, unknown-version, partial, cancelled and replayed observations; wrong nonce,
provider, catalogue, policy, toolchain, material, plan or predicate; changed source under the same test
name; undeclared external tool/environment; retry-created green; skip; ambiguity; selector miss; and
untrusted-to-privileged flow. `A1-646` adds executable source, plan, config, dirty-worktree,
unknown-input, batch-selection, malformed-prior and shadow controls. `A1-647` expands those into the
representative mutation and CI-transport matrix. Every control must change a clean result to a typed
refusal, visible widening or finding.

The production audit maps each hostile claim to an executable owner rather than counting a prose
list as coverage:

| control | executable owner |
|---|---|
| forged subject or envelope/predicate mismatch | `aplan-attest` receipt authentication tests |
| cross-revision material movement, stale/partial/corrupt receipts | `aplan-attest` identity tests and the production receipt consumer |
| unchanged test name with changed code/test/fixture/generated input | the 25-revision incremental shadow matrix |
| retry-created green, skip and other terminal outcomes | receipt and batch terminal-semantics tests |
| ambiguous provider/name and selector miss | production provider-selection and catalogue tests |
| provider crash | the failing-runner corpus case |
| cancellation and resource contention | event-driven scheduler and process-group tests |
| untrusted-to-privileged transport or local trust downgrade | receipt authentication, incremental-refresh and rollout-artifact tests |

The transport test recomputes a valid outer payload digest for traversal, symbolic-link, hard-link,
FIFO, duplicate-normalized-path and file/child collision archives, then proves the bounded extractor
itself refuses each one. A damaged archive with a stale digest remains a separate earlier-boundary
control. Manifest parsing likewise refuses malformed direct verdict digests, boolean exit codes and
producer-environment fingerprints before either execution route starts. Receipt-versus-direct
comparison removes only the expected `provider_source` transport label in addition to volatile
identities/time; it retains the same semantic observation fields as execution-mode parity.

The shadow matrix changes plan tasks, code, tests, fixtures, generated data, configuration,
runner/catalogue material, toolchain, lockfile, workflow and CI policy inputs, declared inputs and an
unknown input, then covers deletion and rename. Every one of its 25 revisions asks the production
`--refresh-shadow` path to compare the incrementally refreshed whole-plan verdict with complete
direct execution. Any disagreement refuses the run; none remained.

## Same-workflow rollout and rollback

`.github/workflows/evidence-rollout.yml` is a daily and manually dispatchable audit, separate from
ordinary push CI. Its producer builds the checked-out source, records a batched direct oracle,
exports all eight receipts and uploads one bounded archive. Its dependent consumer checks out the
same workflow source, starts the post-setup clock before artifact download, validates archive paths,
types, count and expanded size, then validates payload, source, fixture, binary, protocol, policy and
runner digests before execution. It reconstructs `same_workflow` trust from the authenticated
repository, workflow, run, event and privilege context; it never imports a trust decision from the
artifact. Receipt consumption must match the direct verdict, start zero providers, stay below the
artifact-available 10-second p95 and finish download plus five measured samples inside 60 seconds.

Receipts intentionally hash the complete inherited provider environment. Ambient job variables
therefore make a cross-job receipt stale even when source bytes match. Producer and consumer run the
audited binary under the same minimal declared `PATH`, locale, timezone and color environment;
everything omitted is absent rather than guessed equivalent. A test proves that substituting an
untrusted consumer context refuses before extraction-derived trust can be used.

The final private-runner dispatch, Actions run `40152120519` at source
`05ca3eb6ffc9b752e095b2739d97df9e9899abee`, passed both jobs. The producer job took 1 minute 43
seconds, of which 87 seconds was the separately reported release build; its batched direct oracle
took 4.202 seconds. The dependent consumer took 9 seconds as a job. From the clock immediately before
artifact download, download plus validation and five receipt-backed samples took 3.975 seconds. The
samples started zero provider processes, matched the direct verdict digest, and measured 168.10 ms
p50 and 185.98 ms p95/maximum after the artifact was local. This passes the 10-second
artifact-available and 60-second post-setup budgets without crediting queue, checkout or build time
to the evidence gate.

The first live dispatch found that the private runner's external `CARGO_TARGET_DIR` made a
workspace-relative binary path false. The second found that its supported Python predates
`TarFile.extractall`'s filter parameter. The delivered workflow now follows Cargo's configured
target directory, and the consumer manually materializes only prevalidated regular files and
directories with normalized permissions and exact size checks. The final dispatch also uses the
current pinned Node 24 artifact actions. These were rollout findings fixed in `A1-647`, not waived
host assumptions.

`APLAN_EVIDENCE_ROUTE` is the one rollback switch. Its normal value is `receipt`; changing it to
`direct` makes the consumer run the independently invocable direct oracle against the same artifact
fixture. Receipt production remains independent in either mode. Ordinary nightly/manual full CI and
exact-source release validation continue to run direct repository evidence, so the synthetic audit
does not replace the product oracle.

## Batch interchange decision

Framework console text is never parsed as proof. The options have different useful roles:

| format | strengths | evidence gaps | decision |
|---|---|---|---|
| native structured reporter | richest framework identities, retry and attachment semantics | framework-specific and versioned by another project; often lacks requested-set and material identity | preferred adapter input where stable |
| JUnit XML | widely emitted and displayed by CI systems | dialects differ; duplicate identities may be dropped; no standard field for request nonce, complete requested set or material identity | optional adapter input and presentation output, never the trust envelope |
| TAP | simple streaming protocol with plans, pass/fail, skip/TODO and diagnostics | text protocol, producer differences, and no action/material/completeness binding | optional adapter input, not direct proof |
| aplan manifest/result v1 | exact bounded request, nonce, provider, identities, typed outcomes and completeness | requires a small adapter per framework | canonical batch and receipt boundary |

The aplan-owned input manifest contains protocol version, invocation nonce, provider identity and the
deduplicated exact requested identities. The result repeats the version, nonce and provider, records
one typed outcome for every request, declares completeness, and contains bounded diagnostics. Missing,
duplicate, extra or skipped requested entries fail closed. Native JSON, JUnit or TAP may help an
adapter produce that result; the adapter owns their framework-specific interpretation.

ADR-0054 fixes the implemented v1 limits and filesystem boundary: 100,000 identities, 16 KiB per
identity, 64 KiB per raw diagnostic and 32 MiB per document; a private invocation directory and an
initially absent result; and refusal of replacement, symbolic-link, wrong-nonce and wrong-provider
results, plus Unix hard-link and wrong-owner refusal. One adapter process owns one runner's complete
request and uses the same command limits, capture, redaction and process-group cancellation path as
legacy execution. `test.run` remains required as the compatible fallback.

This custom schema is justified narrowly. Existing reports answer "what did this framework say it
ran?" The evidence protocol must also answer "is this the complete answer to this exact bounded
request under these exact material inputs?" The schema must not grow into another general test-report
format.

## CI authority and duplicate work

| operation | normal CI owner | authoritative fact |
|---|---|---|
| dependency install/build | component job | preparation only, never evidence by itself |
| framework test execution | component job through an aplan adapter | provider observation for exact inputs |
| artifact transport | CI artifact service or local file | bytes and digest only |
| producer authentication | configured workflow identity/signature policy | whether the observation is trusted |
| whole-plan evidence verdict | aplan receipt-current gate | every citation resolved against the exact plan |
| independent oracle | scheduled/release direct gate | direct parity and rollback signal |

Component jobs currently run the same backend, frontend, admin and browser suites that aplan L3
repeats. They will publish observations through the same adapter contract used by batched direct
execution. The aplan job downloads them, validates trust/material identity and resolves the plan
without starting providers.

An artifact digest proves the downloaded bytes match the upload; it does not authenticate who was
allowed to produce them. Same-workflow transport, signed/attested envelopes and local files are
separate trust policies over one portable statement. Untrusted pull-request results cannot satisfy a
privileged release. GitHub artifact attestations are an optional producer-authentication layer, not
the baseline: private/internal use requires GitHub Enterprise Cloud, while portable file transport
must work on GitLab, self-hosted CI and offline systems.

If component and direct results disagree, the gate fails and retains both provenances. It never picks
the faster green. Direct is the rollout oracle; after rollout it remains the scheduled audit and one
routing change restores direct-on-every-push.

## Security and failure rules

- Receipt storage is untrusted input. Parsing has byte/count/depth limits and unknown major versions
  are refused.
- A receipt is an in-toto-style statement binding subject digests to an aplan predicate. An optional
  authenticated envelope is verified before policy trusts its producer.
- Timestamps are audit data, never freshness authority.
- Import is atomic and content-addressed. A producer crash before rename yields no current receipt.
- Diagnostics are bounded and secret-sanitized. Receipt paths and credentials do not enter ordinary
  machine or human output.
- Concurrency is bounded by aggregate worker/resource weights and exclusion groups. Completion order
  never changes output order.
- Cancellation owns every provider process group, starts no new work, drains completed output and
  publishes no partial clean observation.
- A cache is correctness-neutral acceleration. A provider observation is correctness-bearing evidence;
  loss of the former may cost time, while loss of the latter makes the gate incomplete.

## Rollout ownership

1. `A1-590` makes interactive inspection consume typed current observations without spawning.
2. `A1-642` defines the manifest/result protocol and batches exact identities per runner.
3. `A1-643` overlaps independent providers under declared resource and exclusion bounds.
4. `A1-644` serializes portable observations and their trust/material identities.
5. `A1-645` derives a zero-provider whole-plan verdict from current receipts.
6. `A1-646` refreshes changed observations without narrowing the final plan verdict.
7. `A1-647` runs shadow parity, hostile controls and latency measurement before the fast path becomes
   authoritative.

Repository CI work remains separate: `A1-615` splits structural checking from whole-plan L1/L2
evidence verification,
and `A1-618` audits affected development validation. Neither task may use this product-level receipt
audit as proof for its distinct selection boundary.

## Research basis

Local designs:

- `~/projects/alint/docs/design/deterministic-perf-gating.md`: gate deterministic work and treat
  contaminated-host wall time as characterization.
- `~/projects/alint/docs/design/v0.9/parallel_walker.md`: bounded parallel work followed by a total
  deterministic sort.
- `~/projects/alint/docs/design/v0.9/git-tracked-filtered-index.md`: build an expensive derived index
  once, only when a consumer requires it.
- `~/projects/alint/docs/design/v0.9/scope-filter.md`: apply cheap selection before I/O.

Primary external references:

- Bazel remote caching and its tool/environment caveat:
  <https://bazel.build/remote/caching>
- Remote Execution `Action`, `Command`, environment and platform identities:
  <https://github.com/bazelbuild/remote-apis/blob/main/build/bazel/remote/execution/v2/remote_execution.proto>
- in-toto Statement v1 and its subject/predicate separation:
  <https://github.com/in-toto/attestation/blob/main/spec/v1/statement.md>
- in-toto v1 parsing and monotonic-policy rules:
  <https://github.com/in-toto/attestation/blob/main/spec/v1/README.md>
- GitHub workflow artifacts and attestations:
  <https://docs.github.com/en/actions/concepts/workflows-and-actions/workflow-artifacts> and
  <https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations>
- GitLab's documented JUnit subset and duplicate-name behavior:
  <https://docs.gitlab.com/ci/testing/unit_test_reports/>
- TAP 13:
  <https://testanything.org/tap-version-13-specification.html>
- Vitest and Playwright native JSON/JUnit reporters:
  <https://vitest.dev/guide/reporters> and <https://playwright.dev/docs/test-reporters>
