# ADR-0050: The plan cache persists typed structural results

Date: 2026-10-19
Status: Accepted
Tasks: `A1-589`
Supersedes ADR-0049 section 8's artifact boundary and refines parts of its format, invalidation,
writer-identity and publication decisions. Its source-authority and disposable-cache decisions
stand.

## Context

ADR-0049 allowed parsed source artifacts but rejected persisted reports until measurement showed
they dominated an exact hit. The implementation measurement supplied that evidence. Rehydrating a
typed plan removes YAML parsing, but structural commands still repeat complete rule evaluation and
trace parsing. Persisting a rendered command response would be unsound and would multiply output
format keys; persisting each parsed trace tree would add another domain representation and still
repeat validation.

The cache also has to detect task additions. Without an independently trusted directory monitor,
that requires enumerating the candidate set on every process. On Unix, device, inode, size, mtime
and ctime with nanosecond precision provide a strong unchanged-file observation; filesystems which
cannot provide that observation must fall back to a content digest. Candidate enumeration and JSON
decoding remain linear even when task and trace parsing reach zero.

## Decision

**1. A structural `Report` is a cacheable typed artifact, not rendered output.** `PlanStore` may
persist the existing core `Report` only under an exact structural-input key. CLI, MCP and TUI still
derive their own typed projections and rendered bytes. No command-specific text, exit code or
evidence-provider observation is persisted by this decision.

**2. The report key encodes rule semantics rather than irrelevant prose.** It includes canonical
typed validation policy, task fields, locations, criteria, dependency and cut structure, malformed
candidate bytes, unreadable-candidate observations, the live generated-agent contract, trace-root
selection, the invocation's root spelling and observations of every referenced document. The root
spelling matters because a persisted diagnostic already contains the relative or absolute path form
that invocation produced, even though parsed artifacts can be reconstructed under either spelling.
Valid task titles and non-task Markdown prose do not affect structural rules and therefore do not
invalidate a report; their freshly parsed projections still change. A malformed candidate remains
keyed by its raw digest because parsing failure details are part of the report.

**3. Exact artifact and report reuse requires an exact semantic writer identity.** The build supplies a
compile-time digest over workspace Rust sources and manifests, the dependency lockfile, compiler
identity, target, profile, feature/configuration environment and compiler flags. The private format
also carries separate format and semantic epochs plus the derived task-schema digest. Package
version and Git `HEAD` are insufficient. The same identity guards parsed artifacts because criterion
scans, source locations and parser behaviour can change without changing the derived task schema.
Any uncertainty under-accepts the hit and recomputes.

**4. Trace parse trees are not a second persisted model.** The structural result contains their
typed diagnostic consequence. Its key observes each trace document conservatively, so an exact hit
performs zero reads or parses through the trace parser and a trace change recomputes through core's
run-scoped trace cache. Because the first format does not persist trace observations with a capture
horizon, key construction still content-reads and hashes each referenced document rather than
trusting racy metadata. This keeps one trace parser and one rule implementation.

**5. Candidate discovery remains authoritative and portable fallback is conservative.** Every load
enumerates the candidate set with the ordinary loader rules, which detects additions, removals,
renames and type changes. A strong Unix change identity permits stat-only reuse; incomplete or
coarse observations hash content. Stable reads compare metadata before and after reading. Native
path units, symlink targets, explicit configuration absence and selected configuration path are
encoded without lossy display conversion.

**6. Identified inputs may retain a digest instead of a body.** A reused candidate supplies its
exact SHA-256 to application input identity when no caller requested source bytes. The canonical
identity encoding gives body bytes and their digest-only form the same result. Mutations and body
projections request retained authoritative bytes and cannot operate on the digest placeholder.

**7. Publication is atomic completeness, not a durability promise.** One create-new private sibling
is written and renamed as a complete generation. A checksum makes truncation, corruption and a
post-crash partial file a quiet miss; the cache is disposable, so a filesystem durability barrier
would add latency without strengthening source authority. A short create-new lock only coalesces
writers. Contention skips publication and never blocks a plan answer. Incremental artifacts and a
new report are published together when one operation computes both.

**8. Cache integrity is not same-user authenticity.** Bounds, native-path validation, restrictive
permissions, symlink rejection and checksums prevent accidental corruption and unsafe path use.
Another process with the same user privileges can replace both an artifact and its checksum and is
outside this cache's trust boundary. Source files remain the authority, and `--cache=off` bypasses
all persisted state.

## Consequences

- Exact structural hits parse zero YAML and zero trace documents while preserving ordinary core
  report types and adapter output.
- A title-only task edit reparses that task for the visible projection but can retain an unchanged
  structural report. A semantic field or trace edit invalidates it.
- JSON decoding, candidate enumeration and per-file observations remain linear. The measured
  10,000-task hit therefore belongs to the final scale audit rather than being hidden behind a zero-
  parse claim.
- A build-system input omitted from the writer digest could make a stale semantic artifact appear
  compatible. The explicit semantic epoch remains the manual backstop for parser or validator
  inputs outside the derived set.
- Skipping `fsync` can lose the optimization after a crash; it cannot turn the cache into evidence
  or cause a checksum-valid partial generation to be accepted.

## Alternatives

| option | why not |
|---|---|
| Persist rendered command output | Couples the cache to format, terminal and selection semantics and bypasses typed projections |
| Persist independent trace parse trees | Creates a second parser artifact model while structural validation still dominates |
| Trust task mtime and size everywhere | Same-size writes and coarse timestamps can return stale task semantics |
| Trust directory mtime for discovery | Its reliability is filesystem-dependent and it can hide a newly added task |
| Hash every unchanged file | Correct but discards the measured stat-only fast path on filesystems with a strong change identity |
| Hash the runtime executable | Identifies unrelated linked bytes and made every command read a large binary merely to open a cache |
| Require crash-durable cache publication | Adds a durability cost to state whose specified recovery is deletion and recomputation |
