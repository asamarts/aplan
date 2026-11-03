# ADR-0063: Captures are versioned intake outside the task graph

Date: 2026-11-01
Status: Accepted
Tasks: `A1-513`

## Context

A task is actionable work. Its status participates in readiness, its dependency edges can block
other work, its cut contributes to a release claim, and `done` requires evidence. A thought that
has only a title cannot meet those obligations honestly. Making `draft`, `idea` or `note` another
task status would force every task rule and graph view to carry an exemption for an object that is
not yet a task.

The intake path still needs durable identity, retry safety, validation and a bounded route into the
plan. Those properties cannot be left to adapter-local scratch files: CLI, MCP and TUI callers must
see the same record and a concurrent promotion must not create a task without disposing the
capture, or dispose the capture without creating the task.

The repository already measured the relevant identifier tradeoff. Twenty-four concurrent writers
minting sequential task ids produced one success and twenty-three existing-file refusals, whether
or not they targeted one milestone (`ADR-0030`). The task ULID implementation replaces the shared
counter with a 48-bit millisecond timestamp and 80 random bits, retains sortability, and refuses
an overwrite even in the collision case. The 26-character cost is real for task ids; capture
commands can hide that cost behind checked human prefixes while machine callers keep the full id.

The storage candidates were evaluated against the resolver that already exists:

| candidate | discovered/explicit config | plan move | concurrent edit | task-loader isolation |
|---|---|---|---|---|
| one file beside `.aplan.yml` | binds data to policy, and an external `--config` points outside the repository | does not follow a moved plan root | every capture replaces one file | suffix filtering only |
| per-user data directory | ignores repository root selection | identity changes by machine | local only | isolated, but not shared or versioned |
| one file below the plan root | follows root selection | follows the move | every capture replaces one file | needs a reserved subtree |
| one file per capture below the plan root | follows root selection | follows the move | independent filenames | reserved subtree can be pruned |

The current root tests cover the three material selections: explicit `--root`, a relative or
absolute `root` in selected configuration, and the built-in `aplan` default. Anchoring storage to
that resolved root is the only candidate whose meaning stays the same under all three. One-file
records also let a human edit, diff or merge one capture without rewriting unrelated intake.

## Decision

### Entity and relation to the plan

**1. A capture is not a `Task`, task status or graph node.** It has no `phase`, task status,
dependency edge, cut membership, effort, path, trace or evidence. It never appears in readiness,
critical path, roadmap, release, summary or evidence calculations. Promotion creates an ordinary
task and records the resulting task id on the retained capture; it does not turn the capture into a
task or add a graph edge between the two.

**2. Capture kind is organization, while capture state is lifecycle.** `task`, `idea` and `note`
are always valid kinds, and `task` is the fast-path default. A repository may append kinds in
configuration. Kind names are non-vacuous, lower-case ASCII kebab-case keys; configured kinds may
not remove or redefine the built-ins. Kinds change no validation, transition or promotion rule.
The closed state vocabulary is `open`, `promoted` and `discarded`; repositories cannot extend it.

### Record and lifecycle

**3. Each record has a small closed schema.** Every capture has:

- schema version, id, title, kind and state;
- an application-injected `captured_at` instant;
- an application-injected immutable origin from the closed adapter vocabulary `cli`, `mcp` and
  `tui`;
- optional body, labels, `revisit_at`, applied profile, snapshotted promotion defaults and
  namespaced external source;
- for a disposed record, an application-injected `disposed_at`, an optional reason and, only for
  `promoted`, the required `promoted_to` task id and `promotion_fingerprint`.

The id, capture instant, origin and source never change. An open capture may amend its title, kind,
body, labels, revisit instant and promotion defaults. Labels are sorted, unique, non-vacuous intake
keys; they are not task tags and do not participate in tag families. Promotion freezes the capture
as the origin snapshot for its task. The new task receives the then-current title and body, and
subsequent task edits never rewrite the capture.

The allowed transitions are:

| from | operation | to | required effect |
|---|---|---|---|
| `open` | promote | `promoted` | set `disposed_at`, `promoted_to` and `promotion_fingerprint`; optionally set reason |
| `open` | discard | `discarded` | set `disposed_at`; optionally set reason; no promotion link |
| `discarded` | restore | `open` | clear disposition time and reason; preserve all other fields |

`promoted` is irreversible. Amend, promote and discard refuse a non-open capture. Restore refuses
an open or promoted capture. The record carries only current state, not a transition log; Git
retains an edit history when the repository commits one. This is why restore clears disposition
fields instead of inventing a second task-history model.

Promotion fingerprints the versioned canonical effective task-creation request, including whether
the caller explicitly selected a task id but excluding an application-minted id and operation
clock. The task write and all three capture fields publish in one guarded transaction. Repeating
the same request against a promoted capture returns `promoted_to` without a write, even if the task
has since been edited; a different fingerprint refuses. The fingerprint is retry identity, not
evidence, task provenance or a promise to reconcile later task changes.

**4. Timestamps are RFC 3339 UTC instants at whole-second precision.** The application clock, not
ordinary user input, supplies `captured_at` and `disposed_at`; tests inject that clock. A future
import or migration surface would need an explicit trust decision before it could accept historical
times. `revisit_at` is different: it is optional user intent saying not to put an open capture in
the actionable list before that instant. It causes no transition, notification, task due date or
task `revisit_at`.

The default inbox lists non-deferred open captures newest first, with id as a total-order tie
breaker, and reports the count of deferred open captures even when that count is zero. An explicit
filter exposes deferred items, and focused show always does. Crossing the instant changes only list
eligibility. Machine results carry the evaluation instant and deferred count so a client does not
have to infer why an item is absent.

### Storage and identity

**5. Captures are one YAML file each under `<plan-root>/.inbox/`.** The canonical filename is the
full id plus `.yml`; a filename and record id must agree. Each document declares
`schema: aplan/capture/v1`, and unknown schema versions refuse rather than being guessed. The
directory is resolved only after the ordinary root and configuration selection, so it moves with
the plan. An empty or absent directory is an empty inbox.

The task loader prunes the reserved `.inbox` subtree before recursive Markdown discovery. Relying
only on the `.yml` suffix would let an accidental Markdown file in that directory become a task.
The capture loader accepts only the canonical regular YAML files and never broadens task discovery.
The files remain readable, editable, diffable and versionable without a database or generated
index.

**6. Capture ids are always `I-` followed by one canonical 26-character Crockford-base32 ULID.**
They do not inherit the plan's task-id strategy or phase prefix. ULIDs remove the shared sequential
counter measured above; a collision still refuses an overwrite and retries with a newly generated
id. Sorting uses the parsed capture instant and full id rather than trusting filesystem order or
the ULID clock alone.

Human inbox commands may accept a canonical leading substring beginning with `I-` only when it
matches exactly one capture in the guarded snapshot. Zero matches and multiple matches refuse,
and a membership change before commit makes a mutation stale rather than selecting a different
record. Machine requests and every result use the full id. Repository moves do not change ids.

Creation, source-key uniqueness and prefix resolution use one guarded inbox snapshot. The existing
application transaction boundary compares that snapshot, including directory membership, again
before publishing a change. Independent ULID filenames avoid routine create collisions; the guard
prevents concurrent same-source creations or prefix changes from both committing. A duplicate
introduced by a Git merge is a validation error, not a winner chosen by traversal order.

### External retry identity and privacy

**7. External identity is one optional `{ namespace, key }` pair.** Both components are non-vacuous;
the namespace is a lower-case ASCII dotted or kebab-case key and the opaque key is length-bounded
text without control characters. The pair must be unique across every retained record, including
promoted and discarded captures. It is the only caller-defined provenance field; arbitrary
metadata maps are not admitted.

A repeated pair is an idempotent success only when its proposed creation content equals the
existing effective content after the selected profile and explicit overrides have been applied.
The comparison includes title, kind, body, canonical labels, revisit instant, origin, applied
profile and snapshotted promotion defaults. It excludes the generated id, capture/disposition
times, current lifecycle state and promotion link. Thus a lost-response retry can return an
already disposed capture, while an amended field or a changed profile default creates a conflict
rather than silently blessing different content. An equivalent retry returns the existing full id
without writing.

The source pair is repository data, not a secret store. Callers must hash or otherwise pseudonymize
sensitive upstream identifiers before submission. Ordinary lists, counts, diagnostics and conflict
messages never print the opaque key; a conflict names the existing capture id, state and namespace.
The full pair appears only in the versioned file and a focused capture result. Titles and bodies are
likewise absent from unrelated validation errors.

### Configuration and profiles

**8. Portable repository configuration owns capture kinds and profiles.** The selected `.aplan.yml`
may add `capture.kinds` and named `capture.profiles`. `--no-config` exposes only the three built-in
kinds and no profiles; `--config` and discovered configuration retain their existing trust and
root-discovery semantics. V1 adds no per-user capture configuration, implicit personal default
profile or executable extension. A caller chooses a profile explicitly on each creation.

A profile may default only:

- capture kind, labels and body scaffold; and
- promotion phase, cut, effort, owner, paths and task tags.

It cannot choose title, id, origin, source, lifecycle state, timestamps, evidence, task status,
dependencies, traces or commands. Explicit creation values override capture defaults. The applied
profile name and effective promotion defaults are stored on the capture, so later repository-config
drift cannot silently change promotion. Explicit promotion values override the snapshot, and the
resulting task still passes the ordinary typed constructor, tag-family rules and whole-plan warning
policy. Preview input identity binds the selected configuration bytes as well as the capture and
plan snapshots, so changing policy between preview and commit refuses instead of changing the
result. Profiles have no conditionals, interpolation, includes, execution, task sets or dependency
topology; blueprints remain the separate explicit-file authoring language for those concerns.

### Validation and retention

**9. Structural validation sees the inbox; graph derivation does not.** `aplan check` validates
every capture file, cross-record id and source uniqueness, and lifecycle combinations. `verify`
performs the same structural gate before resolving task evidence. Capture diagnostics block under
the repository warning policy, but open captures owe none of the task-only rules.

Inbox reads and all capture mutations fail closed if any inbox file is malformed or the snapshot
has a duplicate. Guarded plan mutations also refuse a post-state that would leave the repository's
full structural check failing. Read-only task graph commands, including `ready` and generated
roadmap views, load tasks only: a malformed capture neither becomes a task nor changes a graph
answer, and those commands do not silently present it as work. The required normal way to discover
the malformed data is `check` or `verify`; an inbox command reports it directly.

**10. Retention is explicit and migration is empty.** Promoted and discarded files are retained by
default without automatic expiry. V1 has no purge, delete, archive or compaction command, no
capacity-triggered cleanup and no conversion of draft tasks or arbitrary notes. Direct repository
file deletion remains an author-visible Git operation, not a lifecycle transition promised by the
tool. Restore is the single admitted reversal and belongs to `A1-515`; schema/loading and
validation belong to `A1-514`, promotion to `A1-516`, MCP to `A1-517`, and TUI to `A1-518`.

Because the entity is new, migration consists of treating a missing `.inbox` as empty. Future
schema evolution requires an explicit versioned migration; an unknown version is never rewritten
opportunistically. The final cut audit in `A1-519` measures retained large inboxes before any later
retention mechanism can be justified.

## Consequences

- A quick thought stays cheap without weakening the evidence-gated task model.
- One-file records and ULIDs remove the plan-wide id counter, but cross-record source uniqueness
  still requires a guarded directory snapshot.
- Versioned source keys and capture bodies are intentionally visible to repository readers; this
  surface is unsuitable for secrets.
- Restoring a discard is useful correction, not hidden history. Promotion remains irreversible and
  retains the intake snapshot beside the independently editable task.
- Repository profiles can remove repetitive intake fields but cannot manufacture task validity or
  execute a workflow.
- Graph commands remain stable as an inbox grows, while `check` and `verify` make malformed intake
  impossible to overlook at the repository gate.

## Alternatives

| option | why rejected |
|---|---|
| Add `draft`, `idea` or `note` to task status | It makes non-work a graph node and spreads exemptions through task rules, cuts and evidence. |
| Represent capture kind as task tags | The object would still need to be a task, and organizational kind would become lifecycle policy. |
| Store one monolithic inbox file | Parallel captures would contend on one replacement and produce merge conflicts despite independent records. |
| Put capture Markdown below the plan root | The recursive task loader can consume it as a task; a suffix convention is too weak a boundary. |
| Store captures outside the repository | Per-user state is not portable, versionable or visible to another adapter or worktree. |
| Sequential capture ids | The existing 24-writer measurement showed the shared counter is the contention point. |
| Inherit each repository's task id strategy | Capture identity has no phase or task convention, and mixed strategies would complicate one machine contract. |
| A mutable arbitrary metadata map | It creates an unversioned extension system and makes retry equivalence and privacy unbounded. |
| Treat source keys as secrets | The clear-text versioned file cannot provide that promise; focused visibility and pseudonymization are honest. |
| User-local profiles or an implicit default profile | The same command would produce different repository data on different machines and retries could drift invisibly. |
| Automatic expiry or compaction | No retention limit has been measured, and deletion would erase the very provenance promotion retains. |
