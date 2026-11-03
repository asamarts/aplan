# ADR-0049: The plan cache is disposable and never authoritative

Date: 2026-10-18
Status: Accepted; section 8's artifact boundary and parts of sections 4-7 are superseded by
ADR-0050. Everything else stands.
Tasks: `A1-589`

## Context

Every one-shot read currently discovers, reads, validates and parses every task even when the same
unchanged checkout was read by the preceding process. The current plan is large enough for that
work to exceed the product's 100 ms bounded-read target, especially in an ordinary development
build. `A1-588` removed redundant work inside one invocation and `A1-322` made traced documents a
run-scoped parse, but neither can reuse work across processes.

A persistent optimization crosses a trust boundary. Markdown task files, selected configuration,
trace targets and configured documents are authoritative repository or caller inputs. A cache is
host-local derived state which may be stale, truncated, maliciously substituted by another process
running as the same user, or written by a build with different parser semantics. Treating a hit as
authority would let an optimization change a verdict, which contradicts aplan's central claim that
proof must be independently resolvable.

The application layer already owns typed plan operations shared by CLI, MCP and TUI. Core already
owns `Plan`, `Task`, `Graph` and diagnostics. A cache must reuse those types without becoming a
second model, a second validation implementation, or a self-referential owner of the borrowing
graph. The detailed measurements and rejected alternatives are recorded in
`docs/design/read-path-performance.md`.

## Decision

**1. `PlanStore` is the sole plan-loading and reuse boundary.** It lives at the shared application
boundary and owns one immutable loaded plan snapshot plus its exact input identities. Each
operation obtains a short-lived view and builds the existing borrowing `Graph` once. CLI creates
one store per invocation; long-lived MCP and TUI hosts keep and atomically replace one store. No
adapter owns an independent task, trace or validation cache.

**2. Source inputs remain authoritative.** The store may reuse a parsed artifact only after the
current filesystem observations prove that artifact belongs to the current bytes. Any unavailable,
ambiguous, racing or inconsistent observation is a miss and takes the ordinary loader path. Cache
failure is never a plan diagnostic and never changes stdout, findings, ordering or exit status.
`--cache=off` reads no cache and writes none; `--cache=refresh` deliberately ignores an old
generation but may publish the newly computed one.

**3. Persistent state lives in the operating system's per-user cache directory.** Aplan uses its
own namespace and keys a checkout by a digest of the canonical plan root and configuration
selection. Tests may select an explicit cache root through a process-level override. Repositories
cannot configure that host path. A missing, read-only or unsafe location is a quiet miss. Best-
effort age and aggregate-size eviction bound abandoned checkout entries without affecting a
command verdict.

**4. The private format is versioned, bounded and replaceable.** The first format is checksummed
JSON using existing dependencies. It carries a magic value, platform tag, format epoch, cache-
semantic epoch, exact schema/validator identity, writer identity, root and configuration identity,
sorted candidate and trace manifests, content digests and derived parsed artifacts. Every decoded
count, string and aggregate byte budget is checked before the document can allocate unbounded
state. A wrong version, platform, writer, root, checksum or bound is a miss. There is no migration
or compatibility promise for this disposable format.

**5. Invalidation is conservative and content-validated.** Candidate-set identity detects adds,
removals, renames and file-type changes. Known inputs record size, high-resolution modification
time and available file identity/change time. Equal metadata is not sufficient for a racily clean
file: an input whose timestamp can overlap the generation or whose metadata is incomplete is
content-hashed before reuse. Changed metadata is read with before/read/after observations and
hashed once; unchanged content may retain its parsed artifact. Symlink identity and resolved target
are inputs. Configuration selection, relevant options, trace documents, configured document bytes,
schema/rule identity and explicit absence are inputs too. Semantic digests use a canonical typed
encoding and deterministic map order, never Rust `Debug` output.

**6. Validation reuse requires provable implementation identity.** Package version and Git `HEAD`
do not identify an uncommitted development binary. Persistent semantic results are reusable only
when the cache-semantic epoch, derived schema/rule identities and exact writer identity all match.
A development build without exact implementation identity may reuse only artifacts whose decoder
and source identity are independently exact; otherwise it recomputes. Under-accepting a hit is the
safe direction.

**7. Publication is atomic but locks are not authority.** Cache-owned directories and files are
user-only and must not be symlinks. A writer creates a unique sibling with create-new semantics,
writes and verifies the complete generation, then atomically renames it. A short advisory lock
avoids duplicate work; contention makes the loser compute in memory and skip publication. Readers
never wait for or consume a partial generation. Checksums detect accidental corruption and torn
writes; they do not authenticate against another process with the same user privileges.

**8. The cache stores inputs and typed artifacts, not rendered answers.** Reports and command
output continue to derive from the existing plan and graph. Evidence-provider observations have a
separate freshness and trust contract owned by later work; arbitrary configured commands are not
made cacheable by this decision. A daemon, filesystem watcher, SQLite, memory mapping and a new
binary encoding remain deferred until measurements show the file index cannot meet its budget.

## Consequences

- An exact hit can parse no task or trace document while still validating current input identity.
  A one-file edit reparses only that candidate and reconstructs ordinary typed projections.
- Read-only commands may update private host cache state. `--cache=off` is the strict no-write mode
  for sandboxes and equivalence testing; plan and repository bytes remain untouched in every mode.
- Cache correctness has a larger adversarial test surface than a normal optimization: racy writes,
  timestamp precision, editor renames, symlink retargets, invalid bytes, corruption, hostile sizes,
  build skew, lock contention and killed writers all require explicit miss-path proof.
- Long-lived adapters gain in-memory reuse through the same store rather than carrying divergent
  caches. The CLI obtains cross-process reuse without requiring a resident daemon.
- JSON is an implementation starting point, not a user contract. Dependency, size, permission,
  compile-time and latency measurements decide whether a later private format is warranted.

## Alternatives

| option | why not |
|---|---|
| Trust size and mtime | Same-size writes inside timestamp granularity are documented and reproducible; a stale task could be called current |
| Key only by Git `HEAD` | Misses dirty and untracked inputs, excludes non-Git plans and conflates source control with exact plan identity |
| Put an index in the repository | Makes derived host state look authoritative, creates tracked churn and lets repository policy choose a host write location |
| Cache rendered command output | Multiplies semantic keys and compatibility state while bypassing the existing typed projections |
| Persistent daemon or filesystem watcher | Adds discovery, lifecycle, version-skew, recovery and security boundaries before a file index has failed the measured budget |
| SQLite or a new binary format first | Adds dependencies and format machinery before existing JSON support has been measured and found inadequate |
| Reuse validation by package version | Development builds can change semantics without changing their pre-release version or Git commit |
| Treat the checksum as authenticity | A same-user process can replace both bytes and checksum; it protects against corruption, not a hostile principal |
