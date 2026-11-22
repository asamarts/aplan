# ADR-0072: Agent-contract policy drift is separate from build provenance and live tools

Date: 2026-11-22

Status: Accepted

Tasks: `A2-122`

Partially supersedes `ADR-0037` only where writer identity or the checking build became part of the
blocking repository-policy verdict (`APLAN041` and `aplan export-agents-md --check`). Coordinates
with `ADR-0068` (`A2-081`, `A2-193`), which owns declaring the build a repository requires.

## Context

`ADR-0037` embedded a provenance comment (`<!-- written by aplan VERSION, surface DIGEST -->`) inside
the generated `AGENTS.md` block and widened `APLAN041` and `aplan export-agents-md --check` to
report any difference in that comment as a stale contract. In addition, the generated block
statically enumerated every MCP tool served by the generating build (`## Through MCP`).

Field adoption measured four distinct consequences:

1. **Oscillating commits on unchanged policy.** In an adopting repository using development builds,
   the provenance comment flipped six times in four days between legitimate builds, reached a state
   no installed binary could reproduce, and led a first-time contributor to create and then revert
   a commit after the diagnostic told them to regenerate with an unpinned local binary.
2. **Real policy drift remains valuable.** A second adopter confirmed that a runner-configuration
   change moving the policy-derived block and giving an exact regeneration repair is useful and
   worth gating.
3. **Static MCP tool catalogues couple two independent facts.** Because `AGENTS.md` listed every
   MCP tool from the build that happened to run `export-agents-md`, two capable builds serving
   different tool sets could not agree on one committed block for one unchanged `.aplan.yml`. A
   byte gate cannot simultaneously require both, and a static list can falsely promise tools that a
   selected server build lacks.
4. **Build-only provenance skew is still useful diagnostic context.** A third adopter kept the
   build-only mismatch visible as an advisory signal during an unrelated audit while both gates
   passed; a fourth hit the blocking failure as soon as a build capability moved the surface digest
   before structural validation could pass.

`ADR-0068` (`A2-081`, `A2-193`) now gives repositories an explicit mechanism to declare the build
contract they require (`requires` in `.aplan.yml` plus external CI/wrapper pins). Choosing the
authoritative build therefore no longer needs to ride on `APLAN041` oscillating `AGENTS.md`.

## Decision

Separate the three distinct concerns that `AGENTS.md` previously conflated:

### 1. Repository policy content (blocking contract)

Everything derived purely from `.aplan.yml` and the plan's own shape -- plan location, established
id conventions, work loop, blueprint workflow, evidence strength and exemptions, scoped evidence
policy, warning/advisory posture, effort/cut/drift/url switches, and configured test runners -- is
**repository policy**.

- `APLAN041` and `aplan export-agents-md --check` continue to gate this policy content and retain an
  exact repair (`aplan export-agents-md --write AGENTS.md`).
- Two valid builds reading the same `.aplan.yml` and plan produce byte-identical policy content.

### 2. Live MCP tool capability (discovered from the serving process)

The committed `AGENTS.md` block no longer statically enumerates the generating build's MCP tool
list. Instead, `## Through MCP` teaches agents that `aplan mcp` enforces the same gates as the CLI
and directs them to discover the live tool catalogue and build identity from the running server
(`tools/list` and `aplan_identity`).

- A generated block never promises tools a selected server lacks.
- Adding or refining an MCP tool in a newer `aplan` build never invalidates an unchanged
  repository's committed `AGENTS.md`.
- Repositories that require a specific MCP tool or capability declare it in `.aplan.yml` under
  `requires.capabilities` (or pin `requires.source` / `requires.facts_sha256` per `ADR-0068`).

### 3. Generator provenance (retained for diagnosis, non-blocking when well-formed)

The generated block keeps its opening provenance comment (`<!-- written by aplan VERSION, surface
HEX12 -->`) from `ADR-0037` so humans and auditors can still see which build last wrote the block:

- **Malformed or duplicate provenance is never silently normalized away.** Duplicate
  `written by aplan` comments, unclosed or trailing-text comments, invalid version/surface fields,
  or a provenance comment placed mid-body continue to fire `APLAN041` and fail
  `export-agents-md --check`.
- **Well-formed (or pre-ADR-0037 absent) build-only provenance skew does not block.** When the
  policy body matches and provenance is structurally valid (or absent), a difference solely in the
  writer build versus the checking build does **not** fire `APLAN041` and does **not** fail `check`,
  `verify`, `aplan_check`, or `export-agents-md --check` (even under `deny_warnings: true` and
  without `advisory: [APLAN041]`).
- **Actionable visibility is preserved.** Instead of silencing the difference or advising blind
  regeneration with whatever `aplan` happens to be on `PATH`, `check`, `verify`, `aplan_check`,
  `aplan_verify`, and `export-agents-md --check` emit a non-blocking provenance notice naming both
  known build identities and directing readers to `.aplan.yml` `requires` and `docs/upgrading.md`.
- **No blind regeneration advice across mismatched builds.** Even when genuine policy drift occurs
  alongside a writer-build mismatch, the diagnostic names both build identities and reminds the
  reader to regenerate with the repository's required build rather than an arbitrary `PATH` binary.

### What remains unchanged from `ADR-0037`

- `ROADMAP.status.md` keeps its `<!-- written by aplan ... -->` provenance banner.
- `AGENTS.md` keeps writing its `<!-- written by aplan ... -->` provenance line inside the markers.
- Genuine drift reports continue to name the running build.

## Consequences

- Two builds rendering the same repository policy (including same-version `-dev` builds or builds
  serving different MCP tool sets) no longer oscillate a blocking committed `AGENTS.md`.
- First-time contributors on an unpinned `PATH` build are no longer instructed to rewrite `AGENTS.md`
  merely because their build's version or surface digest differs.
- Repositories that previously placed `APLAN041` in `advisory` solely to tolerate mixed builds can
  retire that workaround while continuing to receive the non-blocking provenance signal and full
  blocking protection against genuine policy drift and malformed markers/provenance.
- Because `APLAN041`'s meaning narrowed without adding a new rule code, the build advertises the
  named behaviour `agent_contract.provenance_does_not_block_policy` with executable proofs.

## References

- `A2-122` (this decision), `A2-081`, `A2-193`.
- `ADR-0037` (partially superseded for `AGENTS.md` blocking verdict), `ADR-0068` (required build
  contract in `.aplan.yml`).
- `docs/upgrading.md`.
