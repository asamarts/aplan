# ADR-0057: Receipt-backed verification is current-only and closed

Date: 2026-10-24
Status: Accepted
Tasks: `A1-645`

## Context

ADR-0056 defines a portable receipt as one provider observation, never a plan verdict. Reusing that
observation saves no work if the consumer lists or executes the configured provider again. It is
unsafe, however, to obtain the speedup by accepting a prior whole-plan result, resolving only the
providers whose files arrived, or silently falling back to direct execution when an artifact is
missing. Those choices respectively import a verdict for the wrong plan, narrow ambiguity, and turn
a bounded gate into an unbounded one.

The consumer also needs trust facts that the receipt cannot authenticate about itself. A manifest
checked out from pull-request code is not a trusted source merely because its JSON says that an
artifact came from a protected workflow.

## Decision

**1. Receipt verification is an explicit closed mode.** `aplan verify --test-receipts <set>`
requires exactly one row for every configured provider. Each row becomes either a current complete
observation or a typed rejection. The resolver performs no catalogue, skip, probe, batch or exact
test command in this mode. A missing, stale, partial, untrusted, incompatible or faulty row remains
in the provider-ordered input, so absence can never narrow a shared-name ambiguity.

`--test-receipts` conflicts with `--execute` and `--fix`. Full direct execution remains the explicit
`--execute` choice. Refreshing only invalid observations is a separate explicit operation owned by
`A1-646`; current-only verification never turns into refresh or direct replay by timeout or error.

**2. The consumer derives a new current verdict.** The import set maps configured provider selectors
to receipt paths and caller-owned trust facts. The consumer recomputes the current runner, declared
material, environment, toolchain, aplan binary and repository identities, validates each receipt,
then resolves every citation against the exact current plan and policy. Commit, file, URL, note,
task-local coverage and ambiguity semantics are unchanged. Receipt bytes contain no reusable
whole-plan clean claim.

The current material and repository context are observed around whole-plan resolution. Movement
during that interval refuses the import. If current material cannot be measured, that provider is a
typed fault and the verdict remains non-clean; the verifier does not replace a useful finding with a
top-level crash.

**3. Trust metadata belongs to trusted orchestration.** The import set schema is
`aplan/test-evidence-import-set/v1`. Its `local_file`, `same_workflow` and `signed` entries project
facts established by the caller onto ADR-0056's verifier policy. The manifest is not
self-authenticating. A repository-controlled or untrusted-job manifest cannot confer trusted
producer status on itself; trusted CI creates it from authenticated artifact or signature metadata
after checkout policy has been applied.

Receipt paths are resolved relative to the import set for orchestration convenience, but neither
paths nor secrets enter findings or aggregate output. Imported bytes are bounded and checked for
movement while read.

**4. Provenance is part of the result.** Aggregate observations independently report scope,
provider source, freshness and completeness. A clean authoritative import is
`whole_plan + receipt + current + complete`. Rejections produce partial provenance and typed
evidence findings, never a flattering clean label. The aggregate state retains `untrusted`,
`incompatible`, `fault`, `stale` or `not_observed` rather than calling a current-only rejection a
failed refresh. Human, JSON, MCP and TUI views use the same vocabulary. Timing output separately attributes manifest, material, parse, identity,
authentication and catalogue-projection work and states the zero provider-spawn invariant.

## Consequences

- A complete receipt set can fan 199 provider outcomes across 241 current task citations without
  starting any provider, while all task-local and per-citation rules still run.
- The import manifest is a privileged orchestration input. Checking one into an untrusted change and
  believing its trust claims violates this decision even if schema validation succeeds.
- A stale artifact fails quickly and visibly. It does not gain availability by weakening assurance
  or surprise the caller with minutes of provider work.
- Receipt-backed and direct results can be compared as independent oracles. `A1-647` owns that
  rollout audit; implementing the mode does not by itself switch repository CI to it.

## Alternatives

| option | why not |
|---|---|
| Accept whichever receipt files are present | a missing provider can erase an ambiguity or citation owner and create a false green |
| Fall back to `test.list` or `--execute` automatically | changes bounded latency and assurance semantics based on failure state |
| Store the final plan verdict in the import set | plan, policy and non-test evidence may differ at consumption time |
| Trust producer coordinates written in the manifest | an untrusted author can write the same coordinates; authentication must come from orchestration |
| Merge current-only and incremental refresh | callers could not distinguish zero-spawn validation from newly executed work |
