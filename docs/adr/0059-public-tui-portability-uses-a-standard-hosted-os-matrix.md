# ADR-0059: Public TUI portability uses a standard hosted OS matrix

Date: 2026-10-27
Status: Superseded by ADR-0065
Tasks: `A1-448`
Supersedes the pre-public repository-boundary decision's `ubuntu-latest`-only public runner choice for the scheduled and manual TUI
portability job. Its private-runner and public self-hosted security decisions remain in force.

## Context

the pre-public repository-boundary decision deliberately gave every public job the same `ubuntu-latest` answer. That is the smallest
safe visibility transition, but it cannot substantiate a terminal application's macOS and Windows
claims. Compiling Unix-only integration machinery on those platforms would be a false test, while
calling an unexecuted target passing would be worse.

GitHub now documents standard public-repository runners for Linux, macOS and Windows as free and
unlimited. The macOS and Windows labels point at fresh GitHub-hosted machines, not the persistent
private runner. Private repositories instead consume an included or billed minute allowance on
those machines. The sources reviewed for this decision are:

- <https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job>
- <https://docs.github.com/en/actions/reference/runners/github-hosted-runners>
- <https://docs.github.com/en/billing/concepts/product-billing/github-actions>

The suite also needs a different shape from complete validation. The repository's main integration
target contains intentional Unix fixture code. The portable contract is the TUI crate's
platform-neutral unit and fault-injection tests plus a separate real-PTY binary journey that has no
dependency on those fixtures.

## Decision

**1. One small workflow owns portability.** A weekly and manually dispatchable workflow runs the
shared TUI library suite and the portable real-PTY module in the crate's existing consolidated
integration target. It is not added to the per-push path:
ordinary Linux validation already exercises these targets, and multiplying clean three-OS builds
on every development commit would recreate the CI bottleneck the development profiles removed.
Release readiness requires a current successful portability workflow after the repository is
public.

**2. Private execution stays Linux-only and zero-hosted-minute.** While the repository is private,
the Linux job uses `[self-hosted, linux, aplan]`. The macOS and Windows matrix has a job-level
private-repository guard, evaluated before runner allocation, and is recorded as skipped. A skipped
platform is reported as unavailable, never passing.

**3. Public execution uses three standard hosted systems.** Once public, Linux uses
`ubuntu-latest`, macOS uses `macos-latest`, and Windows uses `windows-latest`. No self-hosted label
appears in the public-only job. The existing visibility-flip procedure still deregisters every
repository runner before public code can execute.

**4. The selected module is deliberately portable and narrow.** The integration binary gates its
historical Unix-only fixture modules off Windows, preserving the one-target link-cost contract,
while the portability job selects only `tui_portability::`. That module exercises mode ownership, focus,
responsive resize and minimum-size recovery, ordinary and interrupt exits, and live external-editor
release/reacquisition with input-identity refresh. Unix signal and job-control cases remain in the
ordinary Linux integration suite. Hosted setup installs the exact repository toolchain but does
not install coverage, release, nextest or Linux-specific Cargo-profile machinery that these two
targets do not consume.

**5. Evidence distinguishes configuration from observation.** Before the repository is public,
the committed workflow and locally passing Linux suite prove that macOS/Windows execution is
staged, not that either platform passed. The support matrix records the latest observed run and
keeps manual emulator, SSH and multiplexer observations separate from compilation and emulated PTY
coverage.

## Consequences

- The public project earns real macOS and Windows terminal evidence without exposing the private
  runner or spending private hosted minutes.
- The moving `*-latest` labels intentionally detect current platform changes in a weekly lane. A
  failure is a portability finding, not a reason to pin an obsolete user environment silently.
- A release cannot infer multi-platform support from Linux. Until the first public matrix succeeds,
  macOS and Windows remain explicitly unobserved.
- Full and release validation remain governed by the pre-public repository-boundary decision's ordinary `ubuntu-latest` choice. This
  is one bounded portability exception, not a general matrix precedent.
