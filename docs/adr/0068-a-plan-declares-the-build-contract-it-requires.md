# ADR-0068: A plan declares the build contract it requires, while artifact authenticity stays external

Date: 2026-11-22

Status: Accepted

Tasks: `A2-081`, `A2-193`, `A2-194`

Extends the capability and identity model (`aplan facts`, `aplan facts --require`,
`aplan facts --identity`, `A1-068`, `A1-256`) and coordinates with `A2-122` on separating
required-build selection from generated-contract provenance (`ADR-0037`). Supersedes nothing.

## Context

Adopting repositories repeatedly hit five related build-skew failures:

1. **Stale machine-wide binaries.** A contributor with an older `aplan` on `PATH` opens a
   repository whose `.aplan.yml` or task frontmatter uses keys introduced by a newer build. Because
   unknown keys are rejected fail-closed (`A1-068`), the first command fails, and the message is
   already clear -- but the plan itself cannot state what build contract it requires before
   ordinary schema validation runs.
2. **Displacement among pinned development builds.** On a shared machine working multiple
   repositories, each pinned to a different development build, whichever binary sits first on
   `PATH` silently becomes the judge. Adopters write local wrapper scripts to compare version and
   digest, because the plan has no place to state that expectation.
3. **Same version text, different builds.** Development builds from `main` all print the same
   pre-release string (for example `0.8.0-dev`), even after a commit adds a new frontmatter field,
   config key, rule, or behaviour. Version text alone cannot distinguish them.
4. **Upgrading is not "install newer".** A newer binary on `PATH` can introduce new warnings and
   fail a previously green plan, or regenerate `AGENTS.md` with a different banner and trigger
   `APLAN041` (`A2-122`). Moving from one build to another is a reviewed gate transition, not an
   automatic "latest wins" lookup.
5. **Same source, different binaries (and vice versa).** Two legitimate builds from the exact same
   source commit -- for example a static Linux musl release archive and a contributor's macOS or
   local `cargo build` -- share `source` and `facts_sha256` while having different `binary_sha256`.
   Conversely, a diagnostic-wording or bugfix commit can move `source` and `binary_sha256` while
   leaving `facts_sha256` unchanged. Conflating those three identities either breaks portable
   checkouts or falsely treats unreviewed code as identical.

## Decision

### 1. An explicit, two-layer combination in `.aplan.yml`, plus an external artifact boundary

A repository may declare an optional top-level `requires` mapping in `.aplan.yml` combining a
**semantic floor** and an optional **reviewed exact-build pin**, while **artifact authenticity and
exact binary digest pinning remain external** to `.aplan.yml`:

```yaml
requires:
  # Layer 1: semantic floor (releases and development builds)
  min_version: "0.8.0"
  capabilities:
    - build.tui
    - capture.refused_promotion_reports_real_state

  # Layer 2 (optional): exact reviewed-build pin (prevents both older and unreviewed newer builds)
  source: "66ed9997c3ace3d55f64326b1a3bd78fbbf392f1"
  facts_sha256: "1ba8dbe141bdc68bba7233422db2babea1a74ab0df5f07019b7952529f0a8a3c"
```

Every sub-key under `requires` is optional, and all present sub-keys are ANDed:

- **`requires.min_version`** (bare released SemVer `X.Y.Z`, no pre-release suffix): minimum
  semantic version for repositories pinning released builds. Pre-release values (such as
  `"0.8.0-dev"`) are refused at config load because two development builds sharing one `-dev`
  string can have incompatible schemas and rules, making a pre-release version floor vacuous; the
  refusal directs authors to `requires.capabilities`, `requires.source`, or
  `requires.facts_sha256`. A running pre-release binary (`0.8.0-dev`) still satisfies any released
  floor strictly below `X.Y.Z` (for example `0.7.0`, not `0.8.0`) under SemVer precedence.
- **`requires.capabilities`** (list of exact `aplan facts --require` names): every named config
  key, rule code, command, output format, evidence kind, named behaviour, MCP tool, or build
  capability must be advertised by the running build. Works identically for released and
  same-version development builds and survives version bumps.
- **`requires.source`** (exact full 40-hex Git commit SHA): pins the exact source revision the plan
  was reviewed against. Refuses both older builds and unreviewed newer builds, preventing silent
  `PATH` displacement and accidental "newer on PATH" upgrades. **Fails closed when the running
  build's `source` is absent or unknown** (for example a tarball build without `APLAN_SOURCE_REV`).
- **`requires.facts_sha256`** (exact 64-hex SHA-256 of `aplan facts`): pins the exact advertised
  capability, schema, rule, command, behaviour, and feature surface. Catches feature-flag skew (for
  example a headless build against a plan expecting `build.tui`) and unreviewed capability changes
  that occurred without moving a release version.

### 2. Distinct identities and explicit treatment of same-source rebuilds

The three identities reported by `aplan facts --identity` answer three distinct questions and are
never conflated:

| Identity | What it bounds | Admits same-source rebuilds? | Where it is enforced |
|---|---|---|---|
| `source` | Exact source commit reviewed by the repository | **Yes** (by design, without claiming byte reproducibility) | `.aplan.yml` (`requires.source`) and/or external wrapper |
| `facts_sha256` | Exact advertised rules, keys, commands, behaviours and build features | **Yes**, provided features match (`tui` vs headless differs) | `.aplan.yml` (`requires.facts_sha256`) and/or external wrapper |
| `binary_sha256` | Exact bytes of one compiled executable | **No** (refuses any byte difference, even from the same commit) | **External only**: CI Action (`uses: asamarts/aplan@vX.Y.Z`), release checksum + SLSA attestation (`ADR-0066`), or repository wrapper script |

`binary_sha256` is deliberately **not** a key in `.aplan.yml`:

- A single executable digest in committed `.aplan.yml` breaks every multi-platform checkout (Linux
  musl release archive vs macOS vs Windows vs local contributor builds) and forces a false choice
  between portability and reproducibility.
- Reproducibility of the official Linux musl archive is a property of the digest-pinned builder
  (`ci/release.Containerfile`) verified externally by checksum and keyless SLSA provenance
  (`ADR-0066`), not an assumption extended to arbitrary local `cargo build` invocations.
- Checking `binary_sha256` inside the candidate binary itself would still be a self-check; enforcing
  it in the wrapper, installer, or GitHub Action **before** executing the binary gives genuine
  external artifact authentication.

A repository that wants to admit portable same-source builds pins `requires.source` (and optionally
`requires.facts_sha256`) in `.aplan.yml`. A repository that wants to refuse same-source rebuilds
with different bytes enforces `binary_sha256` (or `gh attestation verify` against the published
release archive) in its external wrapper or CI workflow.

### 3. Pre-schema evaluation, older-binary behaviour, and trust boundary

**Pre-schema pass.** Before strict `Config` deserialization (`deny_unknown_fields`), the config
loader parses ONLY the top-level `requires` mapping from raw YAML using a small, frozen reader that
ignores all other top-level keys (mirroring `aplan_core::load::id_and_status`). If `requires` is
present:

1. Validate `requires`'s own strict sub-keys (`min_version`, `capabilities`, `source`,
   `facts_sha256`). Malformed values or unknown sub-keys under `requires` fail closed (exit 2).
2. Evaluate every declared constraint against the running build's own `version`, `facts`, `source`,
   and `facts_sha256`.
3. If any constraint fails, exit 2 immediately with a dedicated build-contract refusal naming the
   declared requirement, the running build's `version`, `source` (or `unknown`), and
   `facts_sha256`, and pointing to `docs/upgrading.md` -- **before** strict schema deserialization
   reports confusing `unknown key` errors for newer config keys added alongside the raised contract.
4. Only after `requires` passes does full `Config` deserialization (`deny_unknown_fields`) run.

`aplan facts`, `aplan facts --identity`, `aplan facts --require`, `aplan --version`, `aplan schema`,
and `aplan --no-config` continue to read no `.aplan.yml`, so diagnostic and inspection commands
remain reachable while a plan's required-build gate is refusing the running binary.

**How an older binary reacts.** A binary compiled before `requires` shipped cannot know about a key
invented after it. Because `.aplan.yml` uses `deny_unknown_fields`, that older binary fails closed
on its first command with `unknown key 'requires'` (exit 2), prints its running version, accepted
key count, and `docs/upgrading.md` pointer, and refuses to judge the plan. It never silently skips
`requires`, and no documentation or diagnostic may claim an older binary can diagnose a capability
invented after it. Once a build supporting `requires` is installed, every subsequent bump to
`requires` is diagnosed cleanly before ordinary schema validation.

**Why this cannot become a green-by-construction self-check.** Checking `requires` inside `aplan`
is an **accidental-skew usability guard**, not a cryptographic security boundary:

- A tampered or broken binary could lie about its own `facts` or `source`. Authenticating *which*
  executable is allowed to run belongs strictly outside the executable (pinned GitHub Action with
  keyless SLSA verification per `ADR-0066`, verified release checksums, or a repository wrapper
  comparing SHA-256 before `exec`).
- Every test for `requires` must exercise mismatched builds through explicit injected
  `ToolIdentity` / `BuildFacts` fixtures (representing older, newer, headless, unknown-source, and
  same-version-different-commit builds), never only the test runner's own binary against a contract
  generated from itself.

### 4. Unknown-field diagnostics and staged upgrade procedure

- Unknown `.aplan.yml` keys (`crates/aplan/src/lib.rs`) and unknown task frontmatter fields
  (`crates/aplan-core/src/load.rs`) keep their fail-closed rejection and leading typo correction,
  and additionally direct same-version development users to `aplan facts --identity` (alongside
  `aplan facts`) so two `-dev` builds printing the same version can be told apart immediately
  (`A2-194`).
- `docs/upgrading.md` publishes the canonical five-step **staged upgrade procedure**: identify both
  builds (`aplan facts --identity`), compare advertised facts (`aplan facts`), run the candidate
  read-only on the unchanged plan (`aplan check` and `aplan verify`), disposition newly introduced
  findings, and only then move the authoritative pin (`requires` and/or CI/wrapper pin) before
  adopting new keys or regenerating `AGENTS.md` / `ROADMAP.status.md`.

### 5. Coordination with `A2-122`

Choosing which build is authoritative for a repository belongs to `requires` in `.aplan.yml` and
the external CI/wrapper pin, not to `APLAN041`'s generated-block provenance comment in `AGENTS.md`.
`A2-122` can therefore narrow `APLAN041` so provenance-only differences do not oscillate a
blocking committed file, without leaving repositories with no way to declare the build they want.

## Consequences

- Adopters can express either a portable floor (`min_version` / `capabilities`) or a strict
  reviewed-build pin (`source` / `facts_sha256`) directly in `.aplan.yml`, catching accidental
  `PATH` displacement on the first CLI, MCP, or TUI command.
- Cross-platform teams pinning `requires.source` admit Linux, macOS, and Windows builds of that
  exact commit without falsely claiming byte reproducibility, while teams demanding exact artifact
  bytes keep `binary_sha256` / attestation verification in their external wrapper or Action.
- Two follow-up implementation tasks are filed in `M-v0.8`:
  - `A2-193`: implement the pre-schema `requires` reader, config schema, CLI/MCP/TUI enforcement,
    and old/new/same-version/headless/unknown-source executable fixtures.
  - `A2-194`: update unknown config-key and task-frontmatter-field diagnostics to direct
    same-version development users to `aplan facts --identity`, with executable tests.

## Alternatives considered

| Alternative | Disposition |
|---|---|
| Semantic-version floor only (`min_version`) | Rejected as sole mechanism: cannot distinguish two `0.8.0-dev` builds and cannot stop an unreviewed newer `PATH` binary from silently introducing new warnings. Kept as one optional sub-key for release floors. |
| Single `binary_sha256` key inside `.aplan.yml` | Rejected: breaks cross-platform checkouts and legitimate local rebuilds, conflates source review with reproducible packaging, and remains an in-process self-check. Exact binary digest verification belongs in the external wrapper/Action/attestation boundary. |
| Capabilities-only floor (`capabilities`) | Rejected as sole mechanism: cannot catch source commits whose wording or bugfix left `facts.json` unchanged, and cannot stop an unreviewed newer superset build from taking over. Kept as the portable semantic-floor sub-key. |
| Decline in-plan declaration completely (wrapper-only) | Rejected: every adopter with multiple projects or contributors would have to re-implement wrapper discovery across CLI, MCP, and TUI, and direct `aplan` invocations would still give opaque unknown-key errors on future schema evolution. External wrappers remain required only for artifact-authenticity (`binary_sha256` / attestation) pinning. |

## References

- `A2-081` (this decision), `A2-193`, `A2-194`, `A2-122`.
- `ADR-0037` (generated-artifact writer identity), `ADR-0066` (keyless release provenance).
- `docs/upgrading.md` ("Staged upgrade procedure" and "Upgrade the binary first, then the config").
