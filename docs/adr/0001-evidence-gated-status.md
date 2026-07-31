# ADR-0001: Status is a falsifiable claim, not a field

- Status: Accepted
- Date: 2026-07-09
- Deciders: kaminsod

> This public record omits the private comparative research that originally accompanied the
> decision. The technical context, decision and consequences are complete without it.

## Context

The requirement that prompted this tool was stated as "tracking and work must be consistent,
with zero drift." A task file committed beside code records what an author claimed. Its storage
location does not prove that the claim is true.

A useful plan gate therefore needs more than a writable status field. It needs a typed statement
of what would prove completion, a deterministic validator for structural claims, and a repository
aware verifier that can resolve cited evidence.

## Decision

Build a single tool. **Make status falsifiable, then run a machine over it.**

1. A task's `status: done` is an assertion that requires evidence. The evidence list is typed and
   must be non-empty to close a task. An undated note is rejected.
2. The dependency graph is proven acyclic on every run. A done task may not depend on unfinished
   work.
3. A task may declare `traces`, anchors in documents it is supposed to resolve. The anchors must
   exist, and closing the task records their reviewed state.
4. The tool exits non-zero on a denied finding, so CI can enforce the policy.

We do not claim zero drift. We claim drift is detected. A repository can select stronger evidence
requirements and can exclude note evidence entirely.

## Consequences

**Good.** The plan becomes machine-checkable. Human, agent and CI consumers receive the same
diagnostics. Evidence resolution can bind a completion claim to repository state.

**Bad.** Evidence has a cost. Every closure requires an artifact that the configured policy
accepts. Repositories must decide where that cost is justified.

**Also bad.** Note evidence is self-attestation. It remains available for work with no stronger
artifact, but it is dated, visible in review and removable through policy.

## Alternatives rejected

- **A status-only task file.** It records a claim without providing a way to test it.
- **State held only in a hosted tracker.** A clone cannot validate it offline, and the plan does
  not travel with the code.
- **A plain `PLAN.md`.** Unstructured prose offers no stable contract for validation.

## References

- alint ADR-0001, "Adopt spec-driven development", applies machine-checkable contracts to claims
  that matter. `aplan` applies the same principle to plans.
