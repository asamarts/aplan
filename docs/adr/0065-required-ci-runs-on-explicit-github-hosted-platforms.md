# ADR-0065: Required CI runs on explicit GitHub-hosted platforms

Date: 2026-11-03
Status: Accepted
Tasks: `A2-013`
Supersedes ADR-0059.

## Context

The public repository has no registered self-hosted runner, but its workflows retained a visibility
expression that selected a persistent runner whenever the same source was private. Ordinary pull
requests also produced an authoritative Linux verdict while macOS and Windows ran only a narrow
weekly TUI check. The old machine container remained running after its public registration was
removed, and `aplan.dev` retained a different offline registration. The supposed fallback could
not run a job, but its code, credentials, schedules, and host cost remained.

GitHub documents each standard hosted runner as a fresh virtual machine. Standard runners are free
and unlimited for public repositories, while private repositories consume included or billed
minutes. GitHub also documents explicit version labels and moving `*-latest` labels. The latter move
only after an announced image migration, but that migration is still an unreviewed input if it
changes a required check. Sources reviewed for this decision:

- <https://docs.github.com/en/actions/reference/runners/github-hosted-runners>
- <https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job>
- <https://docs.github.com/en/billing/concepts/product-billing/github-actions>
- <https://github.com/actions/runner-images/blob/main/README.md>

The workspace is not equally portable at every layer. Its Rust libraries, binaries, unit tests and
real-terminal journey are portable. Complete integration fixtures, coverage instrumentation,
evidence resolution, static Linux artifact construction and crates.io publication remain Linux
contracts. Calling a skipped Unix fixture a Windows pass would weaken the evidence rather than
widen it.

## Decision

1. **Public workflow source is hosted-only.** Every job names an explicit standard GitHub-hosted
   selector. No visibility router, self-hosted label, runner image or runner lifecycle script
   remains in the public repository. The external boundary check continues to require zero
   registered repository runners and approval for every external contributor.

2. **Three stable checks gate pull requests and `main`.** Linux development validation runs on
   `ubuntu-24.04`. A non-cancelling matrix runs macOS on `macos-15` and Windows on
   `windows-2025`. Branch protection requires all three contexts. A platform failure is blocking;
   `continue-on-error` is not an availability mechanism.

3. **The portable matrix owns a real contract.** Each platform compiles and lints every workspace
   target, runs all library and binary unit tests, then executes the independent real-terminal TUI
   journey. Compiling all targets catches platform-specific `cfg` drift. The selected test scopes
   avoid claiming that Unix-only integration fixtures ran on Windows.

4. **Linux keeps complete and release authority.** Change-aware development validation, nightly
   full validation, coverage, evidence, scale, release staging, the pinned Linux artifact builder,
   crates.io OIDC and GitHub publication remain Linux jobs. Passing macOS and Windows checks does
   not imply multi-platform release assets.

5. **Stable and moving images answer different questions.** Required checks pin reviewed GA image
   labels. The weekly and manual TUI matrix retains `ubuntu-latest`, `macos-latest` and
   `windows-latest`, with `fail-fast: false`, to detect upcoming platform changes without changing
   the merge contract mid-review.

6. **The maintainer repository is not a second product CI authority.** `aplan.dev` keeps private
   operational material, not an independently releasable source tree. Its copied product workflows
   are disabled instead of consuming billed hosted minutes or retaining a machine runner. A future
   private-content-specific validation workflow requires its own task and threat model.

7. **Retirement is exact and delayed until replacement proof.** After the public hosted pull
   request passes and merges, remove only the `aplan-runner` container, its `aplan-runner-*`
   volumes, its project-specific user service and the stale `aplan.dev` registration. No other
   project runner, container, service or host setting is in scope.

## Consequences

- Pull requests receive native macOS and Windows feedback before merge, not days later.
- A compromised public workflow has no persistent aplan machine to target.
- Clean hosted builds cost more wall time than a warm persistent target. The public repository does
  not pay hosted-minute charges, and measured cache alternatives remain slower or introduce an
  executable cross-job trust boundary.
- Explicit required labels need deliberate upgrades. The scheduled moving-label matrix provides
  the signal for that review.
- The release remains one reproducible Linux x86_64 artifact until distribution work explicitly
  chooses another promise.
