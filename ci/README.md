# ci/ - public hosted CI and private self-hosted compatibility

Public `asamarts/aplan` uses standard GitHub-hosted runners and has no registered self-hosted
runners. The separate private `aplan.dev` maintainer repository retains the persistent runner. The
same scripts deliberately support both placements.
This directory holds both execution profiles:

- `Containerfile` - the runner image (Debian + Rust toolchain + Python for the
  performance-harness tests + `cargo-llvm-cov` for the coverage gate + the agent).
- `runner/{setup,entrypoint,job-started,teardown}.sh` - container lifecycle and the pre-job target
  volume bound.
- `validation.yml` - the declarative inventory of every gate and its profile membership.
- `cargo-ci-config.toml` - the one immutable-checkout Cargo profile copied into the private image
  and installed on clean hosted runners.
- `scripts/validate.sh` - the profile-neutral entry point used by workflows and hooks.
- `scripts/validation-receipt.py` - the bounded JSON and job-summary serializer for every executed
  profile; it reads normalized arguments from `validate.sh`, never the ambient environment.
- `scripts/{fmt,clippy,doc,test,spec,msrv-build,style,supply-chain,coverage,structural,evidence}.sh` - one
  implementation per validation gate, reached through that entry point so the identical command
  runs in both places.
- `scripts/toolchain.sh` - the reader/verifier for the exact toolchain contract; and
  `scripts/latest-stable.sh` - the separate moving-stable compatibility observation.
- `scripts/release-source.sh` - the clean, full-commit identity boundary shared by release
  validation and artifact construction.
- `scripts/prepare-hosted.sh` - the toolchain pieces a clean hosted VM needs; its `development`
  and `full` aliases are the preparation half of the ordinary-job rollback switch, while its
  `release` profile composes coverage tooling with artifact and publication tools; its narrow
  `portability` profile installs only the pinned Rust toolchain used by the portable TUI targets.
- `scripts/tui-portability.sh` - the shared TUI library/fault suite and portable real-PTY module in
  the existing consolidated integration target
  used by the three-OS public portability workflow.
- `scripts/install-cargo-ci-config.sh` - the fail-closed shared Cargo-profile installer.
- `scripts/check-public-runner-boundary.sh` - the public repository boundary assertion.
- `githooks/pre-push` - opt-in local gate bundle.
- `env.example` - config template (copied to the gitignored `ci/.env`).

## What runs where

Except for the public portability matrix described below, every workflow job makes the same
visibility selection: `[self-hosted, linux, aplan]` while the repository is private,
`ubuntu-latest` while it is public. If the private runner is offline, jobs
queue as pending (and cost nothing) until it returns. On a hosted VM the workflow first invokes the
preparation profile its gate needs; the gate itself remains the same `ci/scripts/*.sh` command used
locally and on the private runner.

The bounded exception is `TUI portability` (ADR-0059). Its Linux job follows that ordinary
visibility selection. Its macOS and Windows matrix is skipped before runner allocation while the
repository is private, and uses standard `macos-latest` and `windows-latest` machines only after the
repository is public. It is weekly/manual rather than per-push and runs only the shared TUI library
tests and portable real-PTY module. A skipped private platform is unavailable evidence, not a pass.

The `CI` workflow routes events to two script-owned profiles:

| event | job and reported status | profile |
|---|---|---|
| pull request to `main` | `development` / `CI / development validation` | change-aware development |
| push to `main` | `development` / `CI / development validation` | change-aware development |
| nightly at 07:37 UTC | `full` / `CI / full validation` | all gates |
| manual dispatch | `full` / `CI / full validation` | all gates |

There are no workflow path filters: even a proven no-op reaches the development job and reports a
verdict. The job passes the fetched base and head to `preflight.sh`; the versioned selector inside
that script decides what runs and fails closed on missing or ambiguous history. A newer push or
pull-request run cancels obsolete development work for the same ref or pull request. Every nightly
or manual full run has a unique group and cannot be cancelled by development or silently replaced
by another full run. Release is a separate, non-cancelling workflow that runs the release profile
on the exact source SHA before publication. Hosted release preparation installs both the pinned
coverage toolchain needed by that profile's validation and the musl/publication tools needed after
validation; the executable capability resolver is regression-tested so neither half can drift out.

The ordinary job has one `APLAN_ORDINARY_PROFILE` setting. Its normal value is `development`.
Changing only that value to `full` makes hosted preparation select the coverage toolchain and
makes `preflight.sh` execute the non-subtractive full profile against the checked-out head. The
ordinary job keeps the full job's 60-minute ceiling so this rollback does not need a second edit.
The profile audit accepts exactly those two values, and the runner census executes both preparation
aliases and the full route.

Every executed profile prints one canonical `aplan/validation-receipt/v1` JSON record and, in
Actions, appends the same bounded record plus a human phase table to the job summary. Set
`APLAN_VALIDATION_RECEIPT` to an initially absent path when a local caller also needs a private
0600 file; publication refuses an existing path, symlink or symlinked parent. The receipt binds the
source and optional base/head, dirty state, validation-inventory digest, exact verified toolchain,
runner class, selection/deferment and every observed gate outcome. It contains no hostname,
workspace root, command output or inherited environment.

Queue and setup are explicit phase states rather than invented zeroes. GitHub queue time is outside
the job and is therefore `unavailable` in the in-job receipt; rollout audits derive it from the run
and job timestamps. Hosted preparation records setup start before tool installation, while the
unconditional nextest installer supplies the same boundary on a private runner. Compile is the
sum of the Clippy, rustdoc and MSRV gate wall times when any ran; test, provider and coverage map to
their named gates. An omitted phase is `deferred` or `not_selected`, never a successful duration.

Public branch protection requires exactly `CI / development validation` and the action self-test
for ordinary merges. Keep the stable context name when changing workflow internals.

The gates: `fmt` (rustfmt), `clippy`, `doc` (intra-doc links, `RUSTDOCFLAGS=-D
warnings`), `test`, `spec` (gen-schema and gen-roadmap `--check`), `msrv-build` (declared compiler
compatibility), `style` (house-style lint), `supply-chain` (advisories, licenses, sources,
duplicate/features policy and product dependency budgets), `coverage` (per-crate line-coverage
floors), `structural`
(`aplan check --format github` once on this repo's plan), and `evidence` (whole-plan L1/L2
`aplan verify` with full Git history and configured test catalogues).

Every compiling gate on the pinned or latest-stable toolchain exports
`CARGO_BUILD_WARNINGS=deny`. Cargo then rejects compiler warnings in workspace packages without
putting `-D warnings` into rustc's argument list, so a local gate and an ordinary development build
share one artifact fingerprint. Clippy still passes `-- -D warnings`, because its lint namespace is
the point of that gate, and rustdoc still uses `RUSTDOCFLAGS=-D warnings` for intra-doc links. The
declared MSRV is the one exception: Cargo 1.88 predates `build.warnings`, so `msrv-build.sh` uses
`RUSTFLAGS=-D warnings` inside its already isolated `target/msrv` directory.

The ordinary workspace test gate also executes `ci/scripts/read-performance.py`; `python3` is
therefore a declared test dependency. The private image installs it, hosted preparation checks it
before repository work, and `ci_runner_census` audits both declarations.

CI is deliberately cheaper than a local debugging build: workspace crates retain line tables,
dependencies carry no debuginfo, and incremental compilation is disabled so immutable checkouts
do not accumulate session state they almost never reuse. The committed `cargo-ci-config.toml` is
copied into the private image and installed by hosted preparation; the installer refuses a
different or symlinked `config.toml` and any higher-precedence legacy `config` spelling. Cargo warns
when both spellings exist and then silently selects the legacy one, even if their current bytes
match. Keeping the profile settings out of the repository's `.cargo/config.toml` leaves local
debugging profiles unchanged while making private and hosted artifact size and fingerprints agree.
`setup.sh` persists
an 8-CPU quota, CPU shares 64, a 24 GiB `memory.high`, and the existing
16,384-process ceiling on the container. It verifies both the stored Podman configuration and the
live `cpu.max`/`memory.high` values before installing reboot persistence.

**`.github/workflows/scale.yml` is the exception, and it is not on the push
path.** It runs `ci/scripts/scale.sh` nightly (04:17 UTC) and on
`workflow_dispatch`, on the visibility-selected runner. That gate generates 24,000
task files and runs for about twenty seconds -- a 29s job on the runner,
measured 2026-09-09 -- which is not a per-commit cost, so it is
not in `ci.yml` and not in `preflight.sh`, and the test it runs is `#[ignore]`d so
an ordinary `cargo test` skips it (`A1-229`).

That same nightly/manual lane owns the TUI release-performance gate (`A1-447`). It first runs an
in-process release suite that separates graph projection/window construction, scene layout,
viewport painting, hit-testing, traversal, refresh, search, progress and responsive reflow. It
then runs a serial real-PTY suite over 32-, 700- and 10,000-task fixtures, interleaving forced
cache refreshes with cache hits and measuring first feedback, first usable content, visible
navigation/search/refresh and Linux resident memory. Every measurement is printed even on success.
The committed budgets and their variance rationale live in `docs/design/tui.md`. These absolute
shared-runner observations are intentionally absent from push and development preflight; releases
run this lane before the release gate is accepted.

What it asserts is a *ratio*, and it asserts it on `load_plan` and `validate`
**separately**: `check` on a fixed number of tasks spread across four cut widths,
slowest against fastest, rather than a millisecond budget. The two phases are
timed apart because loading is O(tasks) and identical in every shape -- 87% of the
total here -- so a ratio on the sum would have needed a *25x* regression in
`validate` before it moved, where the attributed one fails at 4x. A
wall-clock floor on this box would be flaky, and worse than flaky -- `A1-228`
published an 11% difference between two plan shapes that was contention from a CI
run on this same machine, and re-measuring with the runner idle reversed the sign.
A ratio survives a busy box because every shape is slowed together. Run it by hand
with `ci/scripts/scale.sh`, or
`gh workflow run scale.yml`.

**`.github/workflows/latest-stable.yml` is a second observational exception.** Each Tuesday and on
manual dispatch it updates Rust's moving `stable` alias, reports how it differs from the exact
`rust-toolchain.toml` pin, and runs format, strict Clippy, rustdoc and workspace tests. A failure is
an upgrade decision to investigate early; a success is not a `full` or `release` receipt. The
workflow has no publication step, and release never consumes its moving compiler.

`ci/scripts/read-performance.py` is the complementary end-to-end observation harness. It is also
on-demand, not a wall-time CI gate: build `target/debug/aplan` and `target/release/aplan`, then run
the script once with each binary. It records min/p50/p95/max wall time, child CPU, time to first byte,
actual output bytes, per-phase CLI medians and deterministic load/graph/trace counters for the live plan
and a generated 1,000-task corpus. It refuses nonzero command paths, fails when samples change output or
work, fingerprints the binary and complete Git source state before and after the run, and bounds each
subprocess tree with `--timeout-seconds` (30 seconds by default). The harness is Linux-specific because
it measures child CPU with `getrusage`. The canonical state matrix, latest published baseline and exact
commands are in `docs/design/read-path-performance.md`.

`ci/scripts/evidence-gate-performance.py` is the corresponding evidence-path harness. Its generated
repository fixes the P0000021 shape at eight runners, 199 distinct tests and 241 citations. Cold,
warm, slow, failing and no-provider controls report deterministic provider-process counts beside
observational wall percentiles; `--include-direct` adds the intentionally expensive 199-test replay.
It also adds matched batched-serial and bounded-concurrent execution cases, so process-count and
wall-time gains from batching are not misattributed to scheduling (or vice versa).
Every sample binds the complete Git source state, binary, protocol, runner policy and environment,
rechecks mutable identities around each sample, and refuses malformed or mixed fingerprints. Each
case prepares its own cold or warm provider state, so selecting one case cannot inherit setup from a
case that happened to run before it. The `receipt-current` case produces all eight portable
provider receipts outside the measured boundary, then measures current-only whole-plan import:
eight catalogues, 199 outcomes and 241 citations with zero catalogue, skip, probe, batch or exact
test-provider spawns. It separately reports manifest, material, parse, identity, authentication and
catalogue-projection phases. The phase model, trust boundary, budgets and exact commands are in
`docs/design/evidence-gate-performance.md`.

`coverage` is an isolated stage of the nightly/manual full job: `cargo llvm-cov` rebuilds the
workspace instrumented into its own target subdirectory, sharing nothing with the ordinary test
build. It is therefore deferred from ordinary pushes rather than doubling every development run.
Its disk lands on the same `/build`-backed volume as the rest. The
instrumented libtest processes run with `RUST_TEST_THREADS=4`: that matches the
smallest planned public runner and prevents expensive PTY/provider fixtures from
being starved by host-sized fan-out. This is coverage-only scheduling; the ordinary
test command, coverage line set, isolated artifacts and floors are unchanged.

## One-time private-runner provisioning (on the runner host)

Requirements: an always-on Linux host with `podman` (rootless is fine), roughly
2 cores and 4 GB free. This can be any machine you keep on.

1. Clone the repo on the host and `cd` into it.
2. On the repository owner's account, mint a runner registration token:
   Settings -> Actions -> Runners -> New self-hosted runner (Linux). Copy the
   token from the `./config.sh --token <TOKEN>` line.
3. Configure and start:
   ```sh
   cp ci/env.example ci/.env
   # edit ci/.env: set GITHUB_TOKEN to the token from step 2
   bash ci/runner/setup.sh
   ```
   `setup.sh` builds the image, creates the persistent volumes, starts the
   container, and installs a `systemd --user` unit (with linger) so the runner
   returns after a reboot.
4. Confirm the runner shows **Idle** under Settings -> Actions -> Runners, then
   push a commit and watch the jobs run on the box:
   ```sh
   podman logs -f aplan-runner
   ```

The registration token is short-lived and used once; the durable runner identity
is persisted to the `aplan-runner-config` volume, so restarts and reboots need no
new token.

## Where the runner's disk usage lands (`/build`)

All of the runner's heavy disk usage, the image layers, the container's writable
layer (which includes the `_work` checkout tree), and the cargo registry and
`CARGO_TARGET_DIR` volumes, lives wherever this host's rootless podman is
configured to store data, its `graphroot`. `setup.sh` uses podman *named* volumes
rather than host bind mounts precisely so they inherit that location automatically,
with no path hard-coded in the repo.

On the current runner host that `graphroot` is the dedicated `/build` drive, set
once in `~/.config/containers/storage.conf`:

```
[storage]
driver = "overlay"
graphroot = "/build/containers/rootless"
```

So the runner's volumes resolve to `/build/containers/rootless/volumes/aplan-runner-*`
and nothing lands on `/` or `/home` (the same arrangement the alint runner already
uses on this host). Verify it before relying on it:

```sh
podman info --format '{{.Store.GraphRoot}}'   # expect a path under /build
df -h "$(podman info --format '{{.Store.VolumePath}}')"
```

Before each job, `job-started.sh` delegates coverage reclamation to
`reclaim-coverage.py`. It considers only real directories named exactly
`.aplan-coverage-<pid>-<attempt>` with a bounded versioned owner record. That record binds the
owner and cargo child to their Linux boot/start identities and a process group; every descendant
also inherits an allocation-specific lease. Reclamation requires the owner and child to be gone,
the group to be empty and no readable process to carry the lease. A live or reused PID, missing or
malformed record, uninspectable recorded identity, symlink and malformed name are retained. The
same sweep runs inside `cargo xtask coverage` before it allocates a new run, while the hook remains
the backstop after a hard kill.

If the target exceeds 100 GiB after that sweep, the hook clears the known Cargo profile, package,
documentation and target-triple directories; it never recursively clears the mount itself or an
unknown entry. This is a safe pre-job cleanup threshold, not a filesystem quota: output may grow
past it during a job and is reclaimed before the next one.

Allocations made before `A1-633` have no owner record and are deliberately retained: while the
runner is live, an absent parent PID cannot prove that no child still uses them. Clean that legacy
state once, with the runner stopped so the missing record is the only missing fact:

```sh
bash ci/runner/teardown.sh
target_mount="$(podman volume inspect aplan-runner-target --format '{{.Mountpoint}}')"
python3 ci/runner/reclaim-coverage.py --legacy-quiescent "$target_mount"
bash ci/runner/setup.sh
```

The explicit mode still accepts only the exact allocation name and a real same-inode directory;
it removes only candidates with no owner file. A present but malformed or future owner record and
every symlink remain untouched for investigation. Never use the mode while the runner container or
a local coverage command is active.

A fresh runner host must point `graphroot` at its build drive the same way before
running `setup.sh`; otherwise the volumes default to `~/.local/share/containers`
on the home filesystem.

## Day to day

```sh
podman logs -f aplan-runner          # follow runner output
podman restart aplan-runner          # restart (e.g. after editing .env)
systemctl --user status container-aplan-runner.service   # reboot-persistence unit
podman inspect aplan-runner --format '{{json .HostConfig}}' # persisted resource contract
podman exec aplan-runner sh -c 'cat /sys/fs/cgroup/cpu.max /sys/fs/cgroup/memory.high'
```

The defaults can be changed at creation with `RUNNER_CPUS`, `RUNNER_CPU_SHARES`,
`RUNNER_MEMORY_HIGH_BYTES` and `RUNNER_TARGET_CAP_GIB`. Memory is a byte count because the OCI
runtime rejects human-size suffixes for `memory.high`. Re-run `teardown.sh` followed by `setup.sh`
after changing one; setup refuses and removes a newly created container when the stored or live
values disagree.

Update the runner agent or `cargo-llvm-cov` by editing `Containerfile`'s `RUNNER_VERSION` or
`LLVM_COV_VERSION`, then rebuild. The Rust version is deliberately not a container argument: the
image derives it from the repository-root `rust-toolchain.toml` build context.

```sh
bash ci/runner/teardown.sh           # keeps the identity + cache volumes
bash ci/runner/setup.sh              # rebuilds the image, re-attaches the runner
```

After a `teardown` + `setup`, or any `podman restart`, the log shows
`A session for this runner already exists` and the agent retries until GitHub
releases the session it is still holding. Measured at about three minutes on
2026-08-06, not the "up to a minute" this said before. It reaches
`Listening for Jobs` on its own and the registration is unaffected. Wait for that line before
concluding anything is wrong: re-registering with a fresh token at this point
fixes nothing and only churns the identity.

Fully remove the runner (drops the cargo caches too; the identity volume is kept
unless you also `podman volume rm aplan-runner-config`):

```sh
bash ci/runner/teardown.sh --purge
```

## Public/private runner boundary

The workflow router is not a security boundary. A pull request supplies its own workflow and can
write `[self-hosted, linux, aplan]` directly. The boundary is repository identity: public
`asamarts/aplan` has no registered runner for those labels, while private `aplan.dev` owns the
persistent runner and confidential operational history.

Assert the public half independently:

```sh
ci/scripts/check-public-runner-boundary.sh asamarts/aplan
```

Hosted workflow runs must report GitHub-hosted runners. Never register the private persistent runner
to public `aplan` as a temporary fix.

## Cutting a release

Development commits on `main` carry the next pre-release version, on purpose: a staged build must
never report the same string as a published release, or a consumer pinning by `--version` pins
nothing (`A1-060`). A release commit temporarily carries the bare release version. Cutting one is
an explicit bump, in this order:

1. close `CHANGELOG.md`: leave an empty `## [Unreleased]` at the top and move its entries under
   `## [X.Y.Z] - YYYY-MM-DD`
2. set `[workspace.package].version` to the release version, move the internal `path`
   dependencies in `[workspace.dependencies]` with it, and update every current install pin to
   `vX.Y.Z`. The pin gate recognizes a bare release workspace version before its tag exists, so a
   stale example fails here instead of making the tagged source fail later
3. run a build so `Cargo.lock` records the new version: `release.sh` builds `--locked`, so a
   lockfile still naming the old one fails the release with
   `cannot update the lock file ... because --locked was passed`
4. commit the release tree, then bind and run the workflow's validation and artifact phases
   **before creating or pushing the tag**:
   ```sh
   export APLAN_RELEASE_SHA="$(git rev-parse HEAD)"
   TAG=vX.Y.Z ci/scripts/validate.sh --profile release
   TAG=vX.Y.Z ci/scripts/release.sh
   ```
5. for the first crates.io release only, publish the ten new package names from that same clean
   commit with `TAG=vX.Y.Z ci/scripts/publish-crates.sh`; then create a trusted publisher for each
   package naming `asamarts/aplan`, `release.yml`, and the `release` environment, and require
   trusted publishing for later versions. crates.io rate-limits creation of new package names and
   reports the exact retry time in its HTTP 429 response. Wait until that time and rerun the same
   command; it proves the checksum of every completed package before resuming at the first missing
   one. Calls to the crates.io API during this bootstrap must send a descriptive `User-Agent`;
   crates.io rejects curl's default agent with HTTP 403
6. create the annotated `vX.Y.Z` tag on that checked commit and push it
7. after the release lands, set the version to the next `-dev`

The release workflow performs full validation, artifact construction, crates.io publication and
GitHub publication sequentially in one checkout and one job. Each script independently refuses
unless that clean checkout is the full commit ID in `github.sha`; another workflow's result, a ref
name or an abbreviated SHA cannot authorize publication. A failed, cancelled, skipped or stale
validation therefore cannot reach publication. A manual dispatch on a non-tag may stage the
artifact but publication stays ineligible. A tag rerun packages every crate again and compares its
checksum with crates.io, then compares every existing GitHub asset byte for byte. It may fill a
missing asset but never replaces different bytes.

The GitHub `release` environment has no secrets. It requires approval from `kaminsod`, prevents
self-review, and admits only `v*` tags plus manual runs from `main`. The tag path is the publishing
path; the `main` branch rule exists only so a maintainer can stage validation and artifact
construction with `workflow_dispatch`. The repository Actions allowlist admits GitHub-owned
actions and `rust-lang/crates-io-auth-action@*` only. Repository-wide SHA pinning remains enabled,
and the workflow pins that OIDC action to its audited full commit SHA; omitting the allowlist entry
causes GitHub to reject the workflow at startup before creating a job.

The validation log and Actions summary record the exact source, tag/version, toolchain, SHA-256 of
`ci/validation.yml`, and every gate outcome and duration. Artifact construction reports its total
duration separately. The 90-minute job bound covers the measured roughly 15-minute private-runner
full profile plus a cold hosted build while remaining finite. `ci/scripts/release.sh` also refuses
before building when the tag, manifest, changelog section, or plan-derived version disagree. The
changelog check requires a real top-level section, not matching text inside a fenced example.

## Updating Rust deliberately

`rust-toolchain.toml` is the only development/release Rust and component source. `Cargo.toml`'s
`rust-version` is separately the source-consumer floor; do not move it merely because the project
adopts a newer lint/build compiler.

1. Inspect the scheduled latest-stable compatibility run, or dispatch `latest-stable.yml`.
2. Edit only `channel` in `rust-toolchain.toml` to the exact three-part release under review.
3. Run `ci/scripts/toolchain.sh verify`, the full profile, and `ci/scripts/latest-stable.sh`.
   Review new diagnostics, generated bytes and dependency effects as changes, not as bump noise.
4. Rebuild the private image with `teardown.sh` and `setup.sh`. Confirm the next CI receipt names
   the same release, host, commit and components.
5. Commit the one source edit plus any deliberate compatibility fixes. Never add a version to
   `Containerfile`, hosted preparation or a workflow.

The receipt includes `cache-key=rust-<release>-<components>`. Cargo validates artifacts in the
private runner's persistent target by compiler identity.

### Hosted reuse decision

Hosted CI deliberately uses no Actions cache and no build/archive fanout. `A1-616` measured four
shapes on standard hosted runners. A 34 MB compressed crates.io cache saved less than one second
before compilation in its first valid warm comparison while restore itself took three seconds.
Executable `target` reuse was faster warm but failed the adversarial-input requirement, consumed
about 1.54 GB per exact source key and spent 140 seconds restoring. A nextest archive remained
slower than direct execution after all mandatory stages were counted. Combining gates made the
critical path and disk pressure worse.

The accepted hosted optimization is instead the shared immutable-CI Cargo profile described
above: it reduces what each direct job builds without adding a cache service, a new trusted writer,
or executable cross-job input. `.github/workflows/ci.yml` therefore has no `actions/cache`,
`actions/upload-artifact` or `actions/download-artifact` step, and `ci_runner_census` makes that
measured rejection executable. MSRV, ordinary, coverage and release-musl remain distinct build
universes.

The phase receipts and rejected alternatives are recorded in
`docs/design/development-validation.md` and `docs/design/development-validation-nextest.md`. A
future proposal may reconsider reuse only with a same-gate hosted measurement, an explicit
read-only consumer/trusted-writer topology and adversarial proof that restored bytes cannot change
the verdict. Archive size or compiler time alone is not enough.

### Evidence observation rollout artifact

`.github/workflows/evidence-rollout.yml` uses an artifact for correctness evidence, not build reuse,
and never runs on a pull request or ordinary push. The producer records a direct oracle and exact
provider receipts; the dependent consumer validates the bounded archive and every source, binary,
protocol, policy and runner identity before reconstructing same-workflow trust from its own GitHub
context. The post-setup clock starts before download. Receipt verification must start zero providers,
match the direct verdict, remain below the artifact-available 10-second p95, and finish transfer plus
five samples inside 60 seconds.

`APLAN_EVIDENCE_ROUTE` in that workflow is the complete rollback switch: `receipt` is the candidate
route and `direct` is the oracle route. This does not reverse `A1-616`'s build-cache decision;
`.github/workflows/ci.yml` still has no artifact or cache fanout, and no downloaded compiler output
is admitted to an ordinary or release verdict.

## Run the gates locally

```sh
bash ci/scripts/preflight.sh         # change-aware development validation
bash ci/scripts/preflight.sh --full  # the exact complete repository contract
```

The shared `test` gate is `ci/scripts/test.sh`. Install its exact runner once with:

```sh
ci/scripts/install-nextest.sh install
```

`ci/nextest.env` is the single artifact identity: release, upstream commit and SHA-256 for each
supported archive. The private image preloads it, and the `test` and `release` jobs invoke the same
idempotent installer unconditionally after environment preparation. The preload is therefore only
a speed optimization: a stale private image is repaired before repository work, and hosted and
private execution verify the same artifact. Every test-gate run checks the observed release and
commit before compiling, then:

1. compares nextest's machine-readable inventory with Cargo's actual libtest binaries, including
   ignored state, and prints the canonical inventory digest;
2. resolves `.config/nextest.toml` through nextest and requires exactly the applicable
   workspace-source mutex members at `max-threads = 1`;
3. runs one unpartitioned `cargo nextest run` at the selected workspace/package scope with
   `--retries 0 --flaky-result fail`;
4. runs Cargo `--doc` at that same scope, because nextest does not run doctests; and
5. checks the complete CLI surface against the invocation ledger from that one run.

There is intentionally no shard or retry route. Separate nextest processes do not share test-group
mutexes, and a retry would turn failed evidence green. The gate gives both the parsed-plan cache
and invocation ledger unique run-owned paths and removes them on exit. Full and release profiles
call this same gate; coverage deliberately retains its isolated Cargo runner because the admission
study found small per-crate coverage differences under process-per-test execution.

Direct `cargo test --workspace` is supported when diagnosing a Cargo-versus-nextest difference,
but it does not produce the shared-gate verdict. It lacks the pin check, inventory and mutex census,
run-owned state, and CLI audit. Never use bare `cargo test`: workspace `default-members` selects
only `crates/aplan`.

To update nextest, change `ci/nextest.env` and re-run the complete checklist in
`docs/design/development-validation-nextest.md`: inventory parity, the seven process-sensitive
surfaces, seeded failures, workspace mutation stress, archive replay and latency samples. If the
configuration changes without a runner update, repeat every control except artifact installation.
Any new test that mutates or observes the source workspace joins the named test group in the same
commit, and the resolved membership test must be updated with it.

Before an actual profile or non-MSRV inventory gate runs, `toolchain.sh verify` refuses a moving or
wrong active compiler and prints its release, host, commit, components and cache-key fragment. A
`--plan` invocation only reports selection and does not pretend it observed an installed compiler.

`ci/validation.yml` is the source of gate names, commands, input classes, isolation and profile
membership. `cargo xtask validation` generates both the data-only Bash table used by the dispatcher
and `ci/validation-selection.json`, the host-neutral selector policy and workspace graph used before
Rust compilation. `cargo xtask validation --check` rejects drift, an unknown/duplicate gate, a
stale graph input, a missing workflow profile, or a workflow that bypasses the shared entry point.

The `supply-chain` gate runs the checksum-pinned `cargo-deny` release against `deny.toml`, then runs
the metadata/tree-derived product audit in `cargo xtask dependency-audit`. Licenses, sources,
advisories, duplicate-version exceptions and selected dependency features therefore fail closed,
while `ci/dependency-policy.json` makes each TUI admission, package budget and measured binary
ceiling reviewable. It is selectable when dependency/build policy changes and required by full and
release profiles. The advisory database is the gate's only network input; the product-shape audit
uses the committed lockfile and Cargo's selected-package resolver.

The assurance words have one narrow meaning here:

- **selected** (the `development` profile) means only the gates printed under `selected` and
  `widened` ran; every omission is printed under `deferred`. It is useful feedback, never a `full`
  or `verified` repository claim.
- **full** means every gate in the inventory ran successfully, with no subtraction option. It is
  the explicit local integration contract and the nightly/manual remote contract.
- **release** means `full` is required on the same immutable source identity before release work.
  The release workflow encodes and mutation-tests that ordering before publication.

The plan gates are deliberately separate. `structural` is always in development and emits the
complete GitHub-annotation form plus an explicit result without rerunning `check`. It does not open
Git, resolve citations or claim the plan is evidence-verified. `evidence` is deferred in development
and required by full/release; it resolves the entire plan at L1/L2 with full Git history and refuses
configured catalogue failures or ambiguity. Plain `verify` does not execute L3 tests, so this gate
does not call itself direct execution. It revalidates structure as a semantic precondition because
a whole-plan evidence verdict cannot trust a result produced by another process or checkout; the
removed duplication is the second explicit `check` renderer, not that safety boundary. The gate also
does not implement or import the receipt-backed provider observations designed by `A1-590`;
`A1-647` owns the audit required before those can become authoritative.

The development profile is the ordinary no-argument preflight and pre-push route. `A1-612` supplies
the fail-closed classifier; inspect the working-tree selection without running gates with:

```sh
python3 ci/scripts/validation-select.py --working-tree
python3 ci/scripts/validation-select.py --working-tree --format json --timings
```

`cargo xtask validation-select ...` remains a compatibility launcher, but it compiles the
monolithic development tool before delegating and is therefore not the preflight or measurement
entry point.

CI can select the fetched merge-base-to-head range with `--base REV --head REV`. Tests and
diagnostics can compare two resolved commits directly with `--revisions FROM TO`. These modes are
deliberately distinct: the working-tree mode merges staged, unstaged and untracked records, the
base/head mode computes a merge base, and the revision mode does not. Renames always contribute
both names.

The hook passes every non-deletion update as repeated `--pre-push REMOTE_SHA LOCAL_SHA` input. That
mode unions the exact direct revision ranges with staged, unstaged and untracked paths in one
classification. A deletion-only push exits with an explicit no-op; an empty hook input explicitly
falls back to working-tree validation. A new branch's all-zero remote id, a missing object, a head
other than the checked-out source, an unknown path or any malformed/ambiguous input widens to every
selectable development gate. Uncertainty is never printed as `no_changes=true`.

Preflight consumes the selector through a schema-versioned NUL record stream. It never evaluates
selector output as shell. Direct gate names and package names become dispatcher arguments;
widening reasons make Clippy, rustdoc and tests workspace-wide even when package names were also
observed. A selector failure, truncated stream, unknown record or wrong schema stops before any
gate and says that no development verdict was produced. The dispatcher always prints selected,
widened and deferred gates and selected packages. Coverage and whole-plan evidence are deferred in
development, never presented as passing.

`ci/validation.yml` owns the path dispositions, global inputs, Rust gates and cross-cutting package
owners. Package roots and reverse dependencies are generated from `cargo metadata`, not guessed
from directory names. The generated graph binds the root manifest, lockfile, toolchain/configuration
and every member manifest by SHA-256; a missing, linked, malformed or stale input refuses
classification and directs the author to regenerate it. The checked-out Python selector starts
neither Cargo nor rustc, validates the generated source and graph before use, and reports bounded
startup/inventory/metadata/census/Git/classification timings. Member manifests (including feature
topology), `build.rs`, root build inputs, generated contracts and CI machinery widen to the complete
selectable development set. Missing history, non-UTF-8 or unknown paths also widen, with every
reason in the human and JSON reports. Reports and telemetry contain repository-relative paths only.

The spec gate runs the executable tracked-file census as part of `cargo xtask validation --check`.
It refuses a new unclassified tracked path, an overlapping disposition, a stale exact path or
prefix, a package with no tracked files, and a missing cross-cutting owner. It can also be run and
reported directly:

```sh
python3 ci/scripts/validation-select.py --census --timings
python3 ci/scripts/validation-select-performance.py --runs 7 --check
```

The performance harness runs `HEAD^ -> HEAD`, so every sample exercises the tracked-file census,
immutable revision resolution, Git diff, checkout/source check, classification and JSON output.
`cold_ms` is the first fresh Python process. `cache_free_*` covers every fresh process and means
independent of any Rust compiler cache (the checked process count is zero); it does not claim that
the operating system page cache was dropped. `warm_*` excludes the first process. Wall-clock limits
are rollout measurements, while the zero Cargo/rustc process boundary and stable output digest are
deterministic checks. The harness resolves both revisions once, refuses a dirty checkout, passes
those immutable object ids to every sample, and rechecks the commit, worktree and selector/policy
digests after the last sample. The recorded source includes the harness digest as well, so the
measurement program is part of its own provenance. It also requires the Git process count to be an
exact non-negative integer and identical across samples. A source race, malformed counter or NaN
threshold therefore produces no performance verdict.

The immutable A1-651 checkpoint `d87eaa49eefa205de40c1af2244dfeaeb938402a` measured 90.500 ms
cold and 96.191 ms cache-free/warm p95 across seven fresh processes on the private persistent
runner (Actions run `40002852524`, job `120583158344`), with six Git and zero Cargo/rustc processes
per sample. The paired standard-hosted job was rejected before execution by the private account's
billing state, so it is not evidence. `A1-652` repeats this harness on the production standard
hosted path at the public boundary.

The dispatcher can also be inspected without running gates with:

```sh
ci/scripts/validate.sh --profile development --select test --widen 'unclassified input' --plan
```

Enable the pre-push hook so change-aware development validation runs automatically before every
push:

```sh
git config core.hooksPath ci/githooks
# bypass once: git push --no-verify
```

There is deliberately no per-gate skip for `full` or `release`. Cancelling or bypassing a hook
produces no profile verdict; a passing hook is selected development assurance, not full assurance.
Run `bash ci/scripts/preflight.sh --full` explicitly at integration/release boundaries.

## Confirming a push was actually checked

```sh
bash ci/scripts/pushed.sh            # waits for the run on HEAD and reports it
```

**Run creation is not immediate and is not bounded.** Measured on 2026-08-26, two
pushes to `main` waited **40 seconds** and **18 minutes** before a run object
existed at all, with the runner online and idle and nothing else queued. Querying
`actions/runs?head_sha=...` shortly after a push and finding `total_count: 0` is
therefore not evidence that the push failed to trigger anything, and twice that
day it was read as though it were.

**A manual dispatch is not a substitute for this result.** Dispatch runs the independent full
profile in its own non-cancelling group. `pushed.sh` waits specifically for the push event and
ignores a manual full run on the same SHA, so one assurance level cannot be mistaken for another.

Its exits distinguish the observation from the observed verdict: `0` is a completed successful
workflow, `1` is a completed failed workflow, `2` means the GitHub API could not be queried, and
`3` means the observation timed out. Its message distinguishes a run that never appeared from one
that appeared but did not complete, and the latter names its last status and URL. In particular, an
authentication or visibility error is not retried or printed as though an API response body were a
run status. If one commit has several matching `CI` runs, the monitor selects the newest by GitHub's
`created_at` and run id fields before the response reaches the shell. `PUSHED_TIMEOUT` accepts a
non-negative number of seconds and `PUSHED_INTERVAL` a positive number; invalid polling values fail
before the API is queried.

The local hook is what makes ordinary feedback independent of the wait. With `core.hooksPath` set,
the selected development preflight has already passed before the push left the machine, so a late
or missing run delays remote/full confirmation rather than claiming it already happened.

## Running aplan in someone else's CI

Everything above is about this repository's runner. A consumer repository can use the public action
directly, including across GitHub accounts, without a repository token:

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0
- uses: asamarts/aplan@v0.5.2
  with:
    command: verify
```

Use a full commit SHA for both actions in a production repository. `verify` needs full history to
resolve commit evidence. `check` is hermetic and may use the default shallow checkout.

By default, errors fail and warnings annotate. Currently 27 of aplan's 57 rules are warnings. Set
`deny_warnings: true` in the consumer's `.aplan.yml` when every warning should block.

For an unreleased capability, build from the audited source commit or stage a locally built binary
in a self-hosted runner image. For a released binary outside the action, download both the archive
and its `.sha256` asset, verify the checksum, and assert required capabilities with `aplan facts
--require`. Do not commit an opaque binary to the consumer repository.

## Security notes

- The runner is a rootless `podman` container running as a non-root `runner`
  user, with a `--pids-limit` fork-bomb guard.
- The agent runs with `--disableupdate` and the exact repository-derived toolchain baked in, so
  versions change only by a committed pin plus a deliberate image rebuild.
- **Repository identity plus visibility is the tripwire.** The persistent runner is valid only on
  private `aplan.dev`. Public `aplan` must report zero registered runners. Event routing cannot
  substitute for that absence because a pull request controls its workflow.
