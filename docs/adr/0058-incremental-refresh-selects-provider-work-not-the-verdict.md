# ADR-0058: Incremental refresh selects provider work, not the verdict

Date: 2026-10-24
Status: Accepted
Tasks: `A1-646`

## Context

ADR-0056 makes a receipt a complete provider statement rather than a cached plan verdict, and
ADR-0057 makes receipt-backed verification current-only and closed. A changed-source workflow still
needs a safe way to replace invalid observations without executing every test in every provider.
Selecting tasks or citations is not sufficient: a runner policy, shared fixture, generated input,
removed citation or newly listed test can invalidate work that no changed task path names.

The P0000021 pilot implemented a conservative shell classifier. It widened on policy changes, but it
could not authenticate and merge old observations, reason over the current provider catalogue, or
publish a complete replacement before deriving the exact current plan verdict. Keeping that second
dependency model outside aplan would make the fast gate least reliable where its answer is hardest
to audit.

## Decision

**1. Refresh has an explicit Git boundary.** `aplan verify --refresh-test-receipts <set>` requires
`--refresh-base <rev>` and exactly one of `--refresh-head <rev>` or
`--refresh-working-tree`. The clean mode requires the selected head to be the checked-out `HEAD` and
the worktree to be clean. The dirty mode compares the base with committed `HEAD`, index, tracked
worktree and untracked non-ignored paths. Libgit2 supplies additions, modifications, deletions and
both rename sides without parsing localized command output. An absent revision, non-ancestor base,
missing history or non-UTF-8 changed path refuses rather than guessing.

**2. The default unit of reuse is the complete provider.** A provider receipt is reused only when
its authenticated provider, tool, environment, toolchain and complete declared material identities
still agree. Plan and evidence prose are not provider material unless the repository declared them
as such, so those edits can re-derive the whole plan without executing an unchanged provider. A
runner, adapter, wrapper, toolchain, environment or material change refreshes the complete provider.
The configured `evidence.observation_inputs` remains an exhaustive trust claim, not a performance
hint.

**3. Per-test narrowing uses a separate exhaustive dependency declaration.** A runner may opt into
`inputs_by_test`, whose entries map test-name globs to repository input globs. It is deliberately
not `covers_by_test`: coverage says what a pass proves, while input dependency says what can change
the result. Every currently listed identity must match exactly one entry. A relevant changed path
must also match exactly one entry and be inside the provider's declared material. Missing or
overlapping test ownership, missing or overlapping input ownership, a newly listed test, a newly
introduced unknown path, changed configuration, or an input outside material widens to the complete
provider. Shared inputs therefore belong in one entry whose `tests` patterns name every affected
test group; overlapping entries are treated as ambiguity, not as an implicit union.

Selected exact identities use ADR-0054's batch adapter when configured, so one provider refresh is
still one attributable request rather than one process per citation. Old outcomes are merged only
from an authenticated complete base receipt, and every current catalogue member must have exactly
one current or refreshed outcome before publication.

**4. Publication is manifest-atomic and never narrows the verdict.** Refreshed receipt documents are
written under content-derived names next to the import set. They are unreferenced until the complete
new import set is flush-then-rename published; a crash may leave an inert orphan but cannot replace
the previous complete set with a partial set. Receipt and manifest outputs are refused inside
`evidence.observation_inputs`, which would invalidate their own material after observation. An
in-repository output must also be Git-ignored; otherwise it would appear as an introduced input on
the next dirty-worktree boundary. External artifact directories avoid both conditions.

The ordinary ADR-0057 importer then authenticates the published set and resolves every current task
and citation. The resulting provenance is
`whole_plan + incremental_refresh + current + complete`, with structured reused, refreshed,
widened, unavailable, selected-test and boundary fields. "Incremental" describes provider work,
never the plan scope. Provider failure or repository/material movement publishes no manifest and
refuses the refresh.

**5. Trust cannot be silently downgraded.** A current authenticated artifact or signed receipt may
be reused unchanged. A local refresh cannot replace one because it cannot recreate the external
artifact or signature authentication; it refuses and tells the caller to refresh inside the
authenticated producer and rebuild the trusted import set. Locally refreshed statements use
`local_file` trust and an explicitly untrusted local producer identity. Missing local rows may be
created because possession of the caller-controlled import set is already the local trust boundary.

**6. Shadow comparison is explicit and non-authoritative.** `--refresh-shadow` additionally executes
the complete direct oracle and refuses any difference in ordered diagnostics or per-citation typed
outcomes. It reports that the comparison ran but does not change the incremental result into direct
execution or satisfy release policy. `A1-647` owns the representative mutation, latency and trust
audit required before routing ordinary CI through the mode.

## Consequences

- A source change can execute one affected test while all current citations are still judged.
- A plan-only edit or commit movement with identical provider material can publish a current
  statement and re-derive the verdict without starting a provider.
- Repositories get no per-test speedup until they make and maintain an exhaustive dependency claim.
  Omission is slower, not less safe.
- The first unknown or ambiguous edge widens visibly. An explanation consumer does not have to infer
  reuse from elapsed time or logs.
- Direct full execution remains the nightly, release and manual oracle. Rollout authority depends on
  `A1-647`, not on this implementation existing.

## Alternatives

| option | why not |
|---|---|
| Select changed tasks or citations | task `paths` and proof coverage do not describe test inputs, shared fixtures or removed evidence |
| Reuse by test name alone | unchanged names can hide changed source, runner, environment, toolchain or selection behavior |
| Treat overlapping declarations as an implicit union | an accidental broad glob would silently expand trust; one explicit owner makes ambiguity reviewable |
| Publish each refreshed receipt in place | a crash can expose a mixed old/new set; content-derived documents plus manifest-last publication preserve the previous set |
| Fall back to direct execution on any error | availability would silently change both latency and provenance; explicit widening and refusal keep the contract bounded |
| Replace authenticated artifacts with local receipts | integrity and producer authentication would be downgraded without the caller asking for it |
