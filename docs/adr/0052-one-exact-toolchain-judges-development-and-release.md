# ADR-0052: One exact toolchain judges development and release

Date: 2026-10-21
Status: Accepted
Tasks: `A1-619`

## Context

The workspace declares `rust-version = "1.88"`. That is the oldest compiler a source consumer may
use to build the supported package graph; it is not an instruction to lint, test or publish with
1.88. Before this decision, the development toolchain file, hosted preparation and private runner
image all selected the moving name `stable`. The development host reached Rust/Clippy 1.99 while
the private runner still used 1.98, and strict Clippy gave the two revisions different verdicts.
A release could likewise inherit whichever stable compiler its runner happened to know that day.

Rustup already gives a checkout one committed toolchain override, including components. Keeping a
second version in a container argument or hosted script would turn that mechanism into another
copy to reconcile. Conversely, never looking at newer stable compilers would move surprise to the
eventual upgrade. These are separate questions: which compiler is authoritative for this source,
which compiler is the supported floor, and whether the next stable compiler remains compatible.

## Decision

**1. `rust-toolchain.toml` is the only development and release toolchain source.** Its `channel` is
an exact three-part Rust release, not `stable`, a major/minor alias or a date-moving channel. It
also names the required `rustfmt` and `clippy` components under the minimal profile. Local rustup
selects it normally. `ci/scripts/toolchain.sh` validates and reads that same file for consumers
that cannot ask rustup to interpret repository policy directly.

**2. The MSRV remains a different promise with a different source.** `Cargo.toml`'s
`[workspace.package].rust-version` names the consumer floor. The MSRV gate derives that value and
runs `cargo +$MSRV check --locked --workspace`; it never substitutes the development pin. A
dependency may raise the floor only through the existing metadata gate and a deliberate manifest
change.

**3. Every authoritative environment derives the pin.** Hosted preparation reads the channel and
components through `toolchain.sh`. The private runner image copies the root toolchain file and the
reader into its build, which is why runner setup supplies the repository root as its container
context. The image has no Rust-version argument. Rebuilding the image is still deliberate, but it
can no longer select a compiler different from the checkout that requested the rebuild.

**4. Full and release work verify and print the identity before running.** The validator refuses
when the active rustup toolchain, `rustc` release, or required components disagree with the
contract. Its receipt includes release, host, commit hash, components and a canonical cache-key
fragment. The release script repeats the check at its direct entry point, so calling it outside the
workflow cannot inherit a moving hosted default. `A1-614` separately owns requiring the full
profile on the exact source before publication.

Cargo's private-runner target directory remains persistent and Cargo includes compiler identity in
artifact validity. There is no external Actions cache today. If one is introduced, every key must
include the canonical toolchain fragment; `A1-616` owns the wider OS, architecture, lockfile,
profile, feature, flag and trust separation.

**5. Latest stable is observation, never authority.** A separate scheduled/manual workflow updates
the moving `stable` alias and runs formatting, strict Clippy, rustdoc and workspace tests under an
explicit process-local `RUSTUP_TOOLCHAIN=stable`. It reports both the pin and observed release and
asks for an explicit upgrade decision when they differ. It does not invoke `validate.sh`,
`release.sh` or a publication step, and no release workflow invokes it. Its failure is visible but
cannot change the compiler of an in-progress full or release run.

**6. Updating the pin is one reviewed edit.** Inspect the latest-stable result, edit only the
channel in `rust-toolchain.toml`, let rustup install it, and run the full profile plus the
latest-stable script. Review new warnings and lockfile effects rather than suppressing them merely
to complete the bump. Rebuild the private image and confirm its receipt before relying on it. The
contract tests simulate another exact version and prove every environment still derives the
channel and cache fragment without a second version edit.

## Consequences

- Two builds of one revision use the same Rust and Clippy contract, whether local, private CI,
  hosted CI or release.
- A compiler upgrade is visible in one source diff and in receipts. It can change warnings and
  output, so it receives ordinary review instead of arriving with a runner-image refresh.
- The latest-stable lane may be red while authoritative validation remains green. That is useful
  advance warning, not contradictory assurance: its workflow name and final message say it is
  compatibility observation rather than full or release proof.
- A fresh private image build now sends the repository root as build context. The Containerfile
  copies only the two contract inputs it needs before installing Rust; ordinary source remains out
  of the image unless a later `COPY` admits it explicitly.
- The MSRV job may use an older compiler than every other job by design. Sharing its target objects
  or external cache key with development, coverage or release would violate this decision.

## Alternatives

| option | why not |
|---|---|
| Keep moving `stable` everywhere | two hosts can judge one source differently, as the 1.98/1.99 Clippy split demonstrated |
| Use the MSRV for development and release | a consumer floor is not the compiler whose current diagnostics and artifacts the project chooses to ship |
| Pin the container and let local/hosted use stable | makes the image argument a second authority and preserves cross-environment drift |
| Let the latest-stable workflow update the pin automatically | compatibility success does not review changed lints, generated bytes, dependencies or release risk |
| Make latest stable another validation profile | it would create a fourth assurance meaning and could be mistaken for the exact compiler required by full/release |
| Add hosted dependency caching in this decision | trust-separated cache architecture needs its own measurements and negative controls; `A1-616` owns it |
