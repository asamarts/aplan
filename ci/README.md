# ci/ - validation and release automation

All repository workflows run on fresh standard GitHub-hosted runners. The public repository has no
registered self-hosted runner and contains no runner provisioning path. ADR-0065 defines the
platform boundary.

## Workflow topology

| workflow | trigger | runner | verdict |
|---|---|---|---|
| `ci.yml` development | pull request, `main` push | `ubuntu-24.04` | required change-aware Linux validation |
| `ci.yml` platform | pull request, `main` push | `macos-15`, `windows-2025` | required portable compile, lint, unit and PTY contract |
| `ci.yml` full | nightly, manual | `ubuntu-24.04` | all validation gates including coverage and evidence |
| `tui-portability.yml` | weekly, manual | moving Linux, macOS and Windows labels | observational current-image PTY compatibility |
| `release.yml` | `v*` tag, manual staging | `ubuntu-24.04` | protected full validation and publication transaction |
| other scheduled workflows | scheduled, manual | `ubuntu-24.04` | bounded scale, evidence and compatibility observations |

Required platform jobs set `fail-fast: false`. One platform failure therefore does not erase the
other platform's evidence. No platform job uses `continue-on-error`.

The Linux development job is authoritative for the repository's complete integration shape. The
macOS and Windows job runs `scripts/platform.sh`, which:

```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib --bins -- --test-threads=1
cargo test -p aplan --test integration tui_portability::
```

This compiles every target and runs portable behavior without claiming that Unix-only fixtures ran
on Windows. Full validation, coverage, evidence resolution and release authority remain Linux-only.

## Files

- `validation.yml` is the declarative gate and change-classification contract.
- `validation-selection.json` and `scripts/validation-generated.sh` are generated projections.
- `scripts/preflight.sh` is the ordinary change-aware entry point.
- `scripts/validate.sh` runs named profiles without subtraction.
- `scripts/prepare-hosted.sh` prepares one clean hosted image for a declared capability profile.
- `scripts/platform.sh` is the required macOS and Windows contract.
- `scripts/tui-portability.sh` is the smaller scheduled moving-image contract.
- `scripts/test.sh` owns the pinned nextest, doctest and CLI-surface audit.
- `scripts/release.sh` builds the static artifact in `release.Containerfile`.
- `scripts/publish-crates.sh` and `scripts/publish-github-release.sh` form the idempotent publish
  transaction.
- `nextest.env`, `deny.env` and `llvm-cov.env` pin downloaded CI tools and checksums.
- `cargo-ci-config.toml` is installed only for authoritative Linux builds. Local debugging keeps
  Cargo's workspace defaults.

Every external action is pinned to a full commit SHA. `crates/aplan/tests/ci_runner_census.rs`
parses every workflow and fails on an uncensused job, action, runner selector or preparation path.

## Validation profiles

- **selected** is change-aware development assurance. It records every gate it ran, widened or
  deferred and never calls the result complete verification.
- **full** executes every gate in `validation.yml` without subtraction on one source state.
- **release** inherits full assurance on the exact committed source, then permits artifact and
  publication work in that same job and checkout.

Development validation always runs formatting, house style, structural plan validation and the
classifier audit. It widens Rust gates according to `validation.yml` and reports every deferred
gate. Set the one `APLAN_ORDINARY_PROFILE` value in `ci.yml` to `full` to roll ordinary CI back to
the complete profile without reconstructing steps.

By default, 27 of aplan's 57 rules are warnings. A consumer that needs every finding to block sets
`deny_warnings: true`; the repository dogfood already does.

The full profile runs every gate:

```sh
bash ci/scripts/preflight.sh --full
```

The release profile inherits the full profile and adds same-source release requirements. Validation
receipts record the exact source, toolchain, policy digest, gate outcome and duration.

Coverage builds into isolated `llvm-cov-target` directories. It is deferred from ordinary pushes
because an instrumented rebuild would duplicate most compile cost. Coverage floors in
`coverage.json` may ratchet upward but cannot fall without an explicit regression override.

Hosted CI deliberately uses no Actions cache or build archive fanout. The measured cache candidates
either lost time on restore/save or introduced executable cross-job input. Every hosted job starts
from source and immutable downloaded tool identities.

## Hosted preparation

`scripts/prepare-hosted.sh` accepts these profiles:

- `development`, `full`, `test`, `coverage`: authoritative validation capabilities
- `platform`, `portability`: exact Rust toolchain only, using the platform's native linker
- `base`: Rust plus the authoritative Cargo profile
- `msrv`, `latest-stable`: intentionally select their own compiler
- `release`: coverage plus Docker, GitHub CLI and JSON tooling

The script derives Rust and components from `rust-toolchain.toml`. Never copy an exact development
toolchain version into a workflow or script. `Cargo.toml`'s `rust-version` remains the source
consumer floor and moves independently.

## Run the gates locally

Use the same entry points CI uses:

```sh
bash ci/scripts/preflight.sh
bash ci/scripts/preflight.sh --full
```

The first command is the ordinary change-aware handoff. The second is the integration and release
oracle. Direct `cargo test --workspace` is diagnostic only because it omits the pinned nextest
policy, doctests and the complete CLI invocation audit.

To observe the push run for the current `HEAD`:

```sh
bash ci/scripts/pushed.sh
```

Run creation can be delayed. The monitor distinguishes no run, an API failure, a timeout and a
completed failing verdict. A manual full dispatch is not a substitute for the push verdict.

## Public runner boundary

The workflow is not the security boundary because a pull request supplies workflow source. The
public repository must have zero registered self-hosted runners. Check it independently with:

```sh
ci/scripts/check-public-runner-boundary.sh asamarts/aplan
```

The same gate requires approval for every external contributor. Never register a persistent runner
to the public repository as a temporary fix. `aplan.dev` does not run copied product workflows; it
needs a separate private-content validation design before Actions are enabled there.

## Cutting a release

Development commits carry a `-dev` version so a staged binary cannot be mistaken for a published
release. A release commit temporarily carries the bare release version.

1. First, close `CHANGELOG.md`: leave an empty `Unreleased` section and move the curated entries
   under `X.Y.Z`.
2. Set `[workspace.package].version` and every internal workspace dependency version. Then
   update every current install pin to `vX.Y.Z`.
3. Build once so `Cargo.lock` records the release version.
4. Commit the clean release tree and run the exact local release profile before tagging:

   ```sh
   export APLAN_RELEASE_SHA="$(git rev-parse HEAD)"
   TAG=vX.Y.Z ci/scripts/validate.sh --profile release
   TAG=vX.Y.Z ci/scripts/release.sh
   ```

5. After the gates pass, create the annotated `vX.Y.Z` tag on that exact commit and push it.
6. After publication, move `main` to the next `-dev` version.

`release.sh` enters the immutable x86_64 environment in `release.Containerfile`. The upstream Rust
image is digest-pinned, Debian packages resolve from a fixed snapshot, and the script mounts source
read-only. The builder owns Rust, musl, the C toolchain, linker, archive and compression inputs.
Fixed archive metadata and path remapping make two clean builds of one committed source
byte-identical within this declared builder boundary.

The release workflow keeps full validation, artifact construction, crates.io publication and
GitHub publication in one job, checkout and exact `github.sha`. A previous workflow result, moving
ref or abbreviated SHA cannot authorize publication. Manual dispatch on `main` stages validation
and artifact construction but cannot publish because it has no tag.

The protected `release` environment:

- requires an independent reviewer and prevents self-review
- admits `v*` tags and manual runs from `main`
- contains no secrets
- grants the job `id-token: write` for a short-lived crates.io credential

Every package binds its trusted publisher to `asamarts/aplan`, `release.yml` and the `release`
environment. The repository stores no crates.io token. The Actions allowlist admits GitHub-owned
actions and the checksum-pinned crates.io authentication action.

Publication is resumable and drift-refusing. A rerun packages each crate again and compares its
registry checksum before skipping it. Existing GitHub assets are downloaded and compared byte for
byte. A missing asset may be filled; different bytes are never replaced.

The GitHub release contains one static `x86_64-unknown-linux-musl` archive and an adjacent SHA-256
file. Native macOS and Windows CI does not create a distribution promise or additional assets.

## Release audit checklist

For a published version, independently verify:

- the annotated tag resolves to the intended release commit
- all workspace manifests, lockfile packages and changelog section agree on the version
- the successful hosted run used the protected release environment and exact tag source
- all ten crates.io packages are visible, unyanked, checksum-addressed and owned by the maintainer
- a fresh locked crates.io install reports the released version and expected facts
- the release archive checksum matches its sidecar and GitHub's asset digest
- the archive contains one static executable that reports the same version
- rerunning the release workflow proves existing registry packages and assets without replacing
  bytes
- the next development commit carries a distinct `-dev` version

Do not replace a published asset to incorporate a later automation fix. Preserve the released
bytes and apply the corrected path to the next version.

## Updating Rust deliberately

Rust has one source in `rust-toolchain.toml`. Review the scheduled latest-stable result, edit the
exact channel, run `scripts/toolchain.sh verify`, the full profile and `scripts/latest-stable.sh`,
then update the digest-pinned release builder in the same reviewed change.

Required runner image labels are explicit in workflow YAML. The weekly moving-label portability
run is the upgrade signal. Promote a newer GA label only after both the moving-image observation and
the complete local profile pass.
