# HLD: `aplan` high-level design

> This is a design document, not a specification. Per
> [ADR-0001 of `alint`](https://github.com/asamarts/alint): *never try to make English
> prose a regenerable source of truth.* The Rust types remain the spec. Where this
> document and the code disagree, **the code is right and this document is stale.**

---

## 1. Purpose, scope, non-goals

`aplan` is a file-based, in-repository project planning format and the validator that
enforces it. Tasks are Markdown files with YAML frontmatter, committed beside the work.
A plan is a set of falsifiable claims, and `aplan` refuses claims that do not satisfy
the configured structural and evidence policy.

In scope are the task format, dependency graph, invariants, evidence verification, CLI,
CI integration, agent interfaces, and interactive terminal interface.

The following remain out of scope:

- resource scheduling, calendars, and Gantt planning;
- comments, attachments, notifications, and assignment workflows;
- personal task-list management;
- bidirectional synchronization with remote trackers; and
- model-based validation. Product checks are deterministic and explainable.

---

## 2. Thesis

> **A status is a claim, not a proof.**
> **An edge is a claim, not a fact.**

A completed task is credible only when its evidence resolves under a declared policy.
A dependency is credible only when both ends exist and the graph remains valid. These
claims must fail loudly on missing references, cycles, stale traces, or evidence that
does not bind to the work it is offered to prove.

The evidence ladder in Section 5 separates structural validation from repository
resolution and explicitly trusted execution. `check` stays hermetic. `verify`
opens the repository and may resolve configured providers. `--execute` is always an
explicit trust decision.

---

## 3. Architectural constraints

- The task schema carries typed evidence because an evidence gate cannot be reliable
  when proof exists only as prose.
- The validator is deterministic, offline by default, and independent of any one
  agent, forge, or hosted service.
- The editable source remains repository-local text. Generated views never become a
  second source of status.
- CLI, MCP, and TUI adapters share typed application operations and the same
  post-state validation gates.
- Forge integration is one-way projection. Remote edits do not silently rewrite the
  authoritative plan.
- The format and schemas remain open, versioned consumer contracts.
- A compact interface is preferred when a broader surface would duplicate policy or
  load unnecessary context.

---

## 4. Domain model and format

A task is one Markdown file. The frontmatter is data; the body is prose and is never
parsed. One file per task is a load-bearing choice: independent task files reduce merge
conflicts across parallel agents and branches.

```markdown
---
id: P0-003
title: Confirm the external approval path
phase: 0
status: in_progress          # todo | in_progress | blocked | done | dropped
owner: release-team
effort_days: 3
blocked_by: [P0-001]
paths:                       # NEW (§5.2), what this task is allowed to touch
  - crates/aplan-core/src/**
traces:
  - docs/requirements.md#external-approval
evidence:
  - kind: commit             # commit | url | file | test | note
    ref: d213c9a
  - kind: note
    ref: Approval authority confirmed the required filing path
    captured: 2026-07-09     # required for `note`; an undated note is a rumour
tags: [external, blocking]
---

## Why
## Acceptance criteria
- [ ] Written confirmation of the approval path
```

**`Status` semantics.** `Done` is the only status that satisfies a dependency.
`Dropped` deliberately does **not**, a dropped blocker means its dependents can never
legitimately start, and pretending otherwise silently unblocks work that has no
foundation. `APLAN020` raises this to a hard error, so a deadlocked plan fails `check`
rather than sitting there looking healthy.

`Blocked` is *parked*, not merely unstarted: it has no unmet blockers in the graph, so
`ready` must exclude it by status. A tool that answers "what can I work on now" with a
task whose status is literally `blocked` is contradicting its own plan.

**Milestones are a convention, not a new type.** A cut (a release slice) is an ordinary
task tagged `milestone`, `blocked_by` the tasks in that cut, its body holding the cut's
exit-criteria (its rationale lives in ROADMAP §3; ADR-0009 §4). Membership is therefore a
checked `blocked_by` edge, and a milestone cannot be
`done` while a *listed* member is undone (`APLAN007`, plus resolved evidence), proof of the
listed scope rather than of exhaustive membership (made exhaustive by `APLAN022`, opt-in via
`require_cut_membership`).
The work-views (`ready`, `critical-path`, `summary`, `APLAN009`) skip milestones, because a
checkpoint is not work, keyed on `Task::is_milestone`. `critical-path`'s weighted walk does read
`contains` as an ordering edge, which is not that field becoming a dependency (ADR-0042): on a plan
`check` accepts it is the same edge `APLAN047` already requires in `blocked_by`, and it keeps a plan
part-way through adopting `contains` from showing a chain that contradicts its own rollup
(`A1-345`).
`APLAN021` keeps the marker honest (a
milestone has members and carries no effort or paths). No new field or edge type (ADR-0008);
the sequencing itself stays in ROADMAP.md.

**A task's product is not an edge either.** Work whose deliverable is *more work* -- a spike, a
review round, an evaluation -- says so in its own acceptance criteria, and the follow-ups name the
investigation that produced them in their bodies. There is no `produces:` field. Neither form checks
the claim -- `produces: none` and a ticked box are equally unfalsifiable -- so what is on offer is
two ways of refusing the same silence, and the criterion is the one that already exists and
generalises to every other statement of what done means. `supersede` reached the same answer for the
same edge shape one release earlier (ADR-0020, ADR-0027), and a field that changes no answer is the
half of ADR-0025's bar this one fails. The enforcement is the general one: `close` reading
acceptance criteria at all (`A1-224`).

**A release is a milestone tagged `release`** (ADR-0022, `A1-146`). It is a checkpoint over *cuts*
rather than over work: its `blocked_by` names the cuts it closes, `APLAN007` refuses to call it done
while one is open, and `roadmap` rolls up their work. A task belongs to exactly one cut still, since
a release claims none, and `APLAN021` reports a release that holds work directly. No new field: a
second task-membership dimension stays in tags where ADR-0014 put it.

**Membership stays an edge** (ADR-0030, `A1-228`). Whether a cut's members should be a `cut:` field
on the task instead was measured because it reopened an accepted ADR. The symptoms attributed to
the edge had narrower fixes: `A1-216` derived grouping from the existing pair, `A1-227` corrected a
quadratic traversal, and `A1-226` made writes compare-and-swap guarded. Large synthetic fixtures
showed no material read advantage for a new field. A migration would still rewrite task files,
change the drift-gated schema, and turn a derived public field into a declaration under the same
name, so the model remains unchanged.

**And the level above a cut is a repository** (ADR-0029, `A1-200`). One repository holds one plan,
and `blocked_by` names a task in *this* plan and nothing else: no qualified id, no second `root`, no
cross-plan edge. Repository scope keeps `APLAN003` answerable ("exists" means "is in this plan"),
`APLAN022` countable, and every rollup a single answer. A portfolio view may aggregate validated
plans, but it does not weaken task identity inside any one plan.

**New field: `prefer_after`.** A list of task ids this one prefers to follow: **discretionary**
order, which `ready` honours and nothing else reads (ADR-0025, superseding ADR-0017). It never
blocks, so a task whose preference is unmet is still ready and still offered, only offered later,
and `critical-path`, `why` and `APLAN007` do not read it at all -- a reader asking "why can I not
start this" must never be shown an opinion. `APLAN033` refuses a preference that cannot be
honoured: an unknown id, the task itself, a pair that is already a `blocked_by` edge, or a cycle.

**And no second declared signal, for now** (ADR-0031, `A1-231`). `prefer_after` is pairwise, so
"this one matters" against many peers can require many edges; the question was whether an ordered
declared family should rank instead. Existing dependency structure remains sparse enough that a
new opinionated priority field would outrun what the graph can derive. `A1-298` therefore extends
the derived ranking before another declaration is considered.

**Phase, cut, and a declared family are different axes** (ADR-0046). `phase` is a required `u32`
on every task: an ordered coordinate for the default summary and a hint to the plan's convention
when minting a new id. It creates no edge, membership, readiness condition, completion gate, or
execution order. A cut remains the only membership level and owns work through its milestone edge.
A tag family remains the project-defined vocabulary for epic, kind, team, stage, or another view.
A cut may span phases, a phase may span cuts, and cross-cutting work still has a phase.
The shared equality selector therefore has five dimensions: cut, phase, tag, owner and status.
Selecting a phase is an explicit request to narrow a rendered view after the full graph has been
derived; `policy[].when.phase` is an explicit policy scope. Neither gives phase an implicit edge,
membership, readiness or execution meaning.

**Active work is not task ownership** (ADR-0051, `A1-531`). `owner` remains durable plan data: the
person, role or team responsible for a task, and the `owner` selector and repository policy keep
that meaning. A claim is unauthenticated, local ephemeral coordination stored below Git's common
administrative directory, so linked worktrees contend on one guarded record without rewriting the
task. It carries holder label, identity source, time and local origin, lasts until explicit release
or a checked takeover, and has no lease or liveness claim. `--unclaimed` is a separate selector
dimension and composes with `--owner`; without an available local claim snapshot it refuses rather
than equating "not observed" with "free". Corrupt, unsafe or orphaned records likewise never
become an empty set. Non-Git plans remain readable with an explicit unavailable scope. `in_progress`
stays the durable, evidence-checked assertion that work began; neither operation implies the other.

**New field: `paths`.** A glob list naming the files this task is responsible for. It
is what turns evidence from a citation into a checkable relation (§5.2). Optional;
absent means relevance-checking is skipped for that task.

**IDs.** Human-readable ids follow the plan's own convention, often
`<letter><phase>-<nnn>`. This project's tasks use `A<phase>` (`A` for aplan, `A1-004`); the
illustrative examples above use `P` for a hypothetical project. Phase is only a hint while a new id
is minted: moving an existing task to another phase never rekeys its stable id, and no rule requires
an id to encode the coordinate (ADR-0046). dstask's documented footgun is that sequential IDs
**collide across machines**. `aplan add` derives the next id from the convention the plan already
uses (`A1-046` -> `A1-047`, `AL1-07` -> `AL1-08`), and refuses rather than renumbering when the id
it would write already exists; two people generating the same id offline is caught by `APLAN002`
at the merge. `id_strategy: sequential | ulid` **is built** (`A1-047`):
`ulid` removes the collision instead of detecting it, minting 48 bits of timestamp and 80
random bits in place of the counter, and keeping the plan's own prefix because that is the
readable half and nothing about it collides (`A1-01K9ZBW7YQ8N4TQGX2H6RC3PVD`). `sequential`
stays the default, since a 26-character id is a real cost paid by everyone who reads the
plan and a repository with one author has nothing to buy with it. A plan holding ULIDs is
not read as a sequence to continue, so a repository may set the key, unset it, and keep its
own convention either way.

`aplan rekey OLD NEW` is the bounded recovery operation when a collision has already happened.
It treats the id and all structured references to it as one graph migration: `blocked_by`,
`prefer_after`, `ratified_by`, `contains`, a conventional filename, and an id-based
`policy[].when.cut` move together behind compare-and-swap locks and a post-state check. A custom
filename stays custom because identity is frontmatter, not path (ADR-0010). Prose is deliberately
outside the rewrite: exact-token occurrences are reported with locations for adjudication, since
one may name the other branch's task. History, external links, external documents and other
repositories are outside the worktree transaction. Terminal work needs `--allow-terminal` so that
boundary cannot be crossed accidentally. An id carrying a local active claim must be released
first, so rekeying cannot strand the old key while the new one appears free.

**A capture is intake, not a task** (ADR-0063). A capture has no task status, phase, cut,
dependency edge, effort, path, trace or evidence, and therefore cannot affect readiness, critical
path, roadmap or release claims. Promotion atomically creates one ordinary validated task and
freezes a link and versioned request fingerprint on the retained capture; it does not convert the
capture into a graph node. The fingerprint makes lost-response retries independent of later task
edits and is neither evidence nor a reconciliation contract.

Captures are versioned separately as one `aplan/capture/v1` YAML record per
`<plan-root>/.inbox/I-<ULID>.yml`. The task loader prunes that reserved subtree before recursive
Markdown discovery, while `check` and `verify` invoke the capture validator explicitly. The fixed
lifecycle is open to promoted or discarded, with discarded alone restorable to open and promoted
irreversible. Injected capture/disposition clocks, immutable adapter origin, a namespaced external
retry key and guarded directory identity make concurrent writes and lost-response retries
checkable without a monolithic index. Open captures may carry an optional non-transition
`revisit_at`; the default inbox reports deferred count instead of hiding that data.

The V1 reader validates canonical regular-file membership, filename/id agreement, strict UTC
whole-second instants, configured kinds, terminal-field combinations and source uniqueness under
`APLAN057`. It retains a deterministic typed snapshot separate from `Plan`: focused reads may show
the complete record, while list projections omit source keys, bound search and result count, filter
kind/state/label/revisit eligibility, and order parsed `captured_at` newest first with full id as the
tie breaker. Every time-dependent projection takes an injected clock and reports its evaluation
instant and deferred count. Source retry resolution compares the proposed effective values with the
stored snapshot, not with configuration that may have changed later.

Kinds (`task`, `idea`, `note`, plus configured organizational keys) and small repository profiles
change defaults only. Profiles may prefill capture kind, labels and body plus promotion phase, cut,
effort, owner, paths and tags. They cannot set lifecycle, identity, commands or proof, and the
promoted task still crosses the ordinary constructor and whole-plan warning gate. Promoted and
discarded records are retained in v1; there is no automatic expiry, purge or compaction contract.

---

## 5. Evidence and the verification ladder

**This is the core of the design.** Evidence is not a string that exists; it is a
claim about the world that can be resolved, and sometimes executed.

### 5.1 Four rungs

| Rung | Name | Question it answers | Needs | Command |
|---|---|---|---|---|
| **L0** | **Asserted** | Is the evidence field present and well-formed? | nothing | `aplan check` |
| **L1** | **Resolved** | Does the referent resolve for the verified subject? | a git dir | `aplan verify` |
| **L2** | **Relevant** | Does the referent connect to *this* task's claim? | git + `paths` | `aplan verify` |
| **L3** | **Executed** | Does the named test actually pass, now? | a runner | `aplan verify --execute`; focused authoring with `aplan close --execute` |

Per evidence kind:

| Kind | L1 Resolved | L2 Relevant | L3 Executed |
|---|---|---|---|
| `commit` | immutable SHA resolves to `HEAD` or an ancestor in this repo | the commit's tree-diff touches ≥1 path in `paths` | n/a |
| `file` | path exists at `HEAD` (or at the cited rev) | path matches `paths`; file is non-empty | n/a |
| `test` | a test with that name exists in the corpus | its file matches `paths` | the test **passes** |
| `url` | syntactically valid | n/a | reachable (opt-in, memoised only within the run, off by default) |
| `note` | `captured:` is present and is a valid ISO-8601 date | n/a | n/a |

`captured` is provenance for the one inline human assertion, `note`, and the author or importer
sets it. `APLAN055` rejects it on the four machine-resolved kinds. For a commit, immutable identity
is the provenance; for a mutable file, `APLAN054` binds review to content; URL and test resolvers
describe the current run. A date written by the task author is none of those things. A future
captured test-report provider must carry a digest, subject, producer and execution identity rather
than reuse an unbound date beside a test name.

`note` remains the escape hatch for human assertions that have no machine-resolvable
artifact. It is visibly **the weakest rung on the evidence ladder**,
rather than an undifferentiated peer of a commit SHA. Config can ban it for any selected subset,
including a tag or phase, with `policy[].evidence.require_min_strength`. That is the honest way to
let a team tighten the screw on itself without us pretending we removed the escape hatch.

### 5.2 Why `paths` exists

Without it, L2 is impossible and the whole scheme is cosmetic. A dishonest or careless
agent cites a real commit that has nothing to do with the task. With `paths`, the
question "does the cited commit touch anything this task claims responsibility for?"
becomes a tree-diff, and the answer is a diagnostic.

The path set binds a task's claim to a specific artifact: this cited commit or file must
overlap this task's declared responsibility. Execution alone cannot establish that
relationship. The binding is verified deterministically and persisted in the plan.

### 5.3 Command split, and why

`aplan check` stays **hermetic**: no git, no network, no subprocess. It is what runs
on every push, in a pre-commit hook, and inside an editor on every keystroke. Fast,
deterministic, offline.

`aplan verify` opens the repository. A commit resolves at L1 only when its immutable SHA names
`HEAD` or one of its ancestors: an unmerged branch or an abandoned linked-worktree object shares
the database but is not part of the tree being verified. This is a local ancestry statement, not
proof that the commit or `HEAD` exists on a remote. In a shallow checkout, an ancestry result the
available graph cannot establish stays a checkout-scoped failure rather than becoming task blame.
It runs in CI on pull requests and on `main`.
`verify --execute` additionally runs every cited test in the plan. `close --execute` is the
authoring-time focused form: it runs only the proposed post-close task's distinct, unambiguous
test identities and requires a typed unconditional pass before the guarded write. Both are
explicit and off by default; ordinary `close` remains L2-only.

This split matters because it keeps the fast path fast and the dangerous path explicit,
and because it keeps the *deterministic* path deterministic. L1 (commit resolves) and L2
(commit touches `paths`) are pure functions of the repo. L3 (run a test) and `url`
reachability are not, tests flake, networks flake. That non-determinism is precisely why
they are opt-in and off by default: we headline "deterministic, no LLM," and the rungs
that would undercut that adjective are the ones a team must deliberately switch on.

Ordinary task inspection is a third operation, not a disguised whole-plan verification
(`A1-590`, ADR-0053). `show` and `aplan_show` recompute Git-backed citations but never launch a
configured provider. They may read a complete raw test catalogue from a separate private
observation store when `evidence.observation_inputs`, provider configuration, executable bytes,
applicable Rustup toolchain manifests and the inherited environment still identify the same
discovery inputs. The resolver then recomputes
ambiguity, locality and findings against the current plan and evidence policy; no prior verdict is
cached. Each citation
and the aggregate report `current`, `stale`, `not_observed`, `refresh_failed` or `unsupported`, and
the compatibility `resolved` field means exactly `current`. Explicit `show --refresh-evidence`,
whole-plan `verify` and TUI verification own configured work. Refresh uses the repository's explicit
weighted/resource provider schedule; legacy runner forms remain serial. It reports exact
runner/completed/remaining/elapsed boundaries without a guessed percentage and atomically publishes
only a stable complete observation. A missing observation does
not make plain inspection fail; `--require-current-evidence` renders it and returns exit 1.

### 5.4 The `APLAN008` slug algorithm (fixed in `0fef234`)

`APLAN008`, the rule the README singles out as the one people underestimate, once
**did not implement GitHub's anchor algorithm.** The old `validate.rs::slugify` mapped
`.` to `-` and handled only ASCII, so `## 2.1 External approval` anchored as
`2-1-external-approval` where GitHub produces `21-external-approval`, the `traces` example printed
in our own README did not resolve, the exact drift this tool
exists to catch. It is fixed; documented here as the reference algorithm and as a
cautionary tale about a validator whose own docs drifted.

The algorithm now implemented, per the canonical
[`github-slugger`](https://github.com/Flet/github-slugger) source:

1. Lowercase (Unicode-aware).
2. Remove every character that is not a Unicode letter, Unicode digit, `_`, `-`, or a
   space. (So `.`, `:`, `!`, `(`, `)`, and emoji all vanish. `über`, `привет`, `你好`
   survive intact, we do **not** use the `slug` crate, which transliterates.)
3. Replace each remaining space with `-`, *after* stripping, so `"a - b"` → `a---b` and
   runs of spaces are not coalesced.
4. Deduplicate with a stateful per-document counter: `setup`, `setup-1`, `setup-2`.

Heading detection was also tightened (proper ATX: 1-6 `#` plus a required space, 0-3
indent, closing-`#` stripping) and the `title.contains(anchor)` fallback that masked the
bug was removed. Five tests pin it (`readme_trace_example_resolves`,
`slugify_matches_github`, `heading_anchors_disambiguate_repeats`, `atx_heading_detection`,
`duplicate_heading_resolves_by_suffix`). **Remaining follow-up:** a property test against
`comrak`'s `Anchorizer` (GitHub-compatible, actively maintained) for exotic-Unicode
divergence, deferred rather than pull a heavyweight parser into dev-deps; and fenced
code-block tracking (a `#`-line inside a fence is still read as a heading).

### 5.5 Trust boundary, the security-critical part

`aplan verify --execute` and an explicitly requested `aplan close --execute` run test commands.
A task file can arrive in a pull request
from a fork. **A task file must therefore never be able to name a command.**

Mirroring `alint`'s ADR-0004 (spawning rule kinds are top-level-config-only):

- Evidence of kind `test` carries a **test name**, never a command line. The name is
  substituted into a runner template configured in the **top-level `.aplan.yml`
  only**, and never shell-interpolated. `{}` placeholder, argv vector, no shell.
- An optional `test.batch` adapter keeps names out of argv entirely. Aplan writes a private,
  bounded `aplan/test-batch-manifest/v1` document and supplies only whole-argument `{manifest}` and
  `{result}` paths to trusted top-level configuration. The nonce- and provider-bound result must
  contain exactly one typed outcome for every exact requested name. Missing, duplicate, extra,
  skipped, expected-failure, retry, flaky, cancelled, timed-out, resource-stopped, framework-failed,
  incomplete, unavailable or nonzero-exit answers prove nothing. A retry-created green cannot use
  the unconditional `passed` spelling. Symlink, replacement and
  replayed-result checks fail closed before any row reaches the resolver; Unix additionally binds
  the open file's device, inode, owner and link count (ADR-0054). `test.run` remains the compatible
  fallback.
- `--execute` is **off by default**, and CI must not enable it for `pull_request`
  events from forks.
- Focused close constructs a one-task proposed post-state, so only that closure's test citations
  can reach `test.run` or `test.batch`; repeated runner/name identities execute once and historical
  citations are structurally absent. Every citation must return typed L3 `resolved`. The preview
  binds proposed plan bytes, exact changed worktree material, selected configuration and selected
  provider runtime material before publication (ADR-0061). Ordinary close and MCP close remain
  L2-only.
- The no-publication guarantee is about aplan's guarded plan mutation. A configured test command
  is trusted code and may have effects of its own; focused close detects resulting input movement
  but does not attempt to roll those effects back or describe the whole working tree as unchanged.
- `url` evidence is **not fetched by default** (`APLAN016`, `A1-041`). When enabled by
  `url.check`, the fetch is a **configured command** rather than a built-in client, so
  redirect policy, timeout and body-size caps live in that command where the repository
  owns them (`curl --max-time`, and curl does not follow redirects unless asked). The one
  part the command cannot own is **which target it is given**, because the citation comes
  from a task file and a task file can arrive from a fork: `aplan` therefore hands over
  **`https` citations only**, and reports any other scheme as not checked. Measured while
  auditing `A1-041`: before that restriction a task file made the checker request
  `file:///etc/passwd` and `http://169.254.169.254/`, the cloud metadata address.
  A content-addressed cache is not built; the resolver asks once per url per run.
- Every configured command is bounded at three independent layers (`A1-310`, `A1-365`,
  `A1-366`). `max_memory` and `max_processes` constrain the child through cgroups where
  the platform provides them. `test.output_lines` constrains the sanitized failure excerpt.
  `max_output` constrains raw bytes retained by aplan itself, before any excerpt exists:
  1 MiB per stream by default, with stdout and stderr separate so thread scheduling cannot
  allocate one stream's budget to the other. Both pipes are still drained; the first and last
  halves are retained, and anything between is counted and discarded. A command whose stdout
  is an answer is refused if that answer was truncated; a command whose exit status is the
  answer keeps that status.
- Those common controls are one typed command-policy view and one execution path for test,
  URL and version commands (`A1-185`). Their public config shapes stay separate because the
  questions differ. Test and URL resolution share first-fault-wins state as well, so one broken
  checker produces one configuration finding rather than one finding per citation.
- Batch adapters use that same command policy, runner directory, bounded capture and process-group
  lifecycle. One adapter process owns all distinct unambiguous cited identities of one runner and
  returns one complete result; framework-internal workers or chunks do not weaken the complete-set
  check. `test.timeout_seconds` continues to bound discovery rather than execution, just as it does
  for legacy `test.run`.
- Provider scheduling is opt-in and bounded by one aggregate weighted capacity (ADR-0055). The
  legacy mapping and runner-list forms remain serial. The scheduled `test.schedule` / `test.runners`
  form overlaps independent catalogue work and independent runners' skipped-list/batch stages.
  Each runner consumes `weight` units (one by default) and may name capacity-one `exclusive`
  resources. The coordinator reserves the complete weight/resource set atomically, orders queued
  work and final results by configured runner, and treats completion order only as live progress.
  These are in-process admission controls, not machine-wide locks; per-command cgroup memory/process
  caps remain the enforcement boundary for each child.
- Configured children lead their own process groups, and on Unix every CLI entry point that can
  reach one installs a process-wide catchable-termination owner before dispatch (`A1-592`). `SIGINT`,
  `SIGTERM`, `SIGHUP` and `SIGQUIT` close the spawn boundary, kill all registered groups and are
  re-raised so the caller observes the original signal. The TUI owns those signals separately only
  so it can restore terminal state before invoking the same group cleanup.
- `aplan check` spawns nothing and opens no sockets, ever. That property is worth a
  test.

### 5.6 Anti-drift

Because `traces` anchors are checked and mutable file evidence can carry a content stamp, we can
detect a subtler drift: **a document section changed after the task that owned it was
closed.**

The mechanism is a content fingerprint captured at review time. Content addressing is
independent of commit identity and file location, while a timestamp comparison would be
invalidated by rebases and unrelated history.

**The trace half landed** in `A1-010`, opt-in via `detect_drift`. `aplan close` stores
`trace_fingerprints:` for each resolvable trace anchor, and `APLAN018` warns when the live
section no longer matches. Markdown section fingerprints trim only line endings and trailing
blank lines; reindentation and prose reflow are material changes. Structured-document anchors
use the canonical addressed value. The fingerprint is content-addressed, so it is independent
of commit ids and file location. `aplan close --restamp` takes a new stamp after deliberate
re-review. `APLAN028` reports a closed task that could have stamped a trace
but did not, so trace adoption is explicit rather than silently unverifiable.

**The evidence half landed** in `A1-196`, behind the same switch. A mutable `file`
citation is the evidence kind whose reference can keep resolving while its referent changes.
`close` and `amend` store `evidence_fingerprints:` keyed by the complete citation, and
`APLAN054` compares the stamp during `verify`. A whole-file citation hashes the exact blob bytes,
so binary proof is covered; an anchored citation hashes only the addressed section, so an
unrelated sibling edit is silent. Both stamp and comparison read the committed `HEAD` blob. A
dirty worktree edit is not durable evidence and does not move the result until it is committed.
`close --restamp` refreshes both trace and evidence stamps after review. It refuses while
`detect_drift` is disabled rather than reporting success while erasing baselines it did not
refresh.

Fingerprint-baseline writers replace only an ordinary block mapping of inline string pairs or an
ordinary one-line flow mapping of the same (`A1-716`). A mapping carried through an anchor, alias,
explicit tag, multiline value, structural key or other non-local YAML source form is refused before
`close`, `close --restamp`, `amend` or a computed evidence repair constructs candidate bytes. The
refusal names the field and asks the author to simplify it by hand; it is an input limitation, not
an internal writer failure, and no partial in-memory edit is published. Ordinary replacement still
keeps unrelated frontmatter, comments, newline convention and the Markdown body byte-for-byte. It
also retains the accepted mapping's literal top-level key, spacing around its colon and boundary
comment; clearing the final derived baseline keeps that comment as a standalone note (`A1-719`).

Citation amendment has the analogous source-locality boundary for its different data shape
(`A1-717`). An existing `evidence:` value is replaceable only when its source is an ordinary block
sequence of citation mappings whose `kind`, `ref` and optional `captured` fields are inline scalars
and agree exactly with the loaded task. Anchors, aliases, explicit tags, flow collections,
multiline values and structural key spellings refuse before add, remove, replace or computed
repair reconstructs the block. The explicit command changes no task bytes, and a computed batch
publishes no earlier candidate. Ordinary edits preserve citations and freshness baselines outside
the requested semantic delta plus unrelated frontmatter and body bytes. Within an accepted
ordinary sequence, add, remove and replace are source deltas as well: every unaddressed mapping
keeps its indentation, key spacing, field order, scalar quote style, comments and newline
convention; only the addressed mapping fields or one deterministic added mapping move
(`A1-722`). Resolution, mutation policy and freshness then read the parsed candidate produced by
that one rewrite rather than a second implementation of citation identity; a `captured` date on
the source cannot hide a newly introduced reference from its resolver (`A1-717`).

Initial close applies the same presence rule before it owns any citation source: it asks the
parsed top-level mapping whether an `evidence` field exists, rather than searching only for a
literal `evidence:` line. Quoted, explicit, tagged, anchored and aliased YAML key spellings
therefore refuse as live-task input before a duplicate-key candidate is constructed, with the task
bytes unchanged. Because `amend` accepts only `done` tasks, this refusal asks the author to review
why the field is present and reconcile it instead of recommending a command that cannot accept the
live task (`A1-723`).

Release close makes the same distinction for its optional record: an absent top-level `version`
field may receive the number derived at closure, while an authored null value is still a present
source field. Ordinary empty values plus quoted, explicit, tagged, anchored and aliased keys which
resolve to `version` refuse before candidate construction, name the derived number and leave the
task bytes unchanged. An absent field still receives that number; a nonempty record still remains
untouched when it agrees and names both values when it disagrees (`A1-724`).

Line-local scalar mutation has a separate source boundary. Before changing `id`, `status`, `phase`,
`owner` or `revisit_at`, the writer compares the scalar token on the ordinary top-level key line
with the complete value in the parsed YAML mapping. A plain scalar continued on a following
indented line therefore refuses before candidate construction: replacing or removing only its
first physical line would change a different value. Optional-scalar set and clear use the same
predicate, and portfolio preparation keeps that refusal atomic across every requested task.
Ordinary one-line plain and quoted scalars remain surgical, retaining key spacing, trailing
comments, line endings and unrelated bytes (`A1-725`).

The kind boundary is deliberate and executable in the evidence-capability audit. `commit` and a
`file@rev` whose revision resolves as the object id it spells name immutable Git objects. Hex shape
alone is insufficient because Git permits a branch or tag named `deadbeef`; those refs, `HEAD`, and
other symbolic revisions can move and are fingerprinted. `url` reachability can be checked live,
but a reachable URL
does not reveal whether its content or meaning changed. A named `test` can be re-listed and run,
but neither operation can detect that its assertions were weakened; a rename is ordinary
reference rot under `APLAN013`. A `note` is inline and has no independent referent. Claiming
freshness for any of those would be stronger than what the resolver observes.

`detect_drift` remains off by default. Enabling it stamps future `close` and resolved `amend`
operations; an older task with no evidence stamp remains silent rather than producing a migration
finding with no recorded baseline. Once a task carries any evidence stamp, a mutable citation
missing its own is reported: partial coverage is observable without mistaking historical absence
for a failed check. That makes evidence freshness incremental, while trace
completeness retains its separately chosen `APLAN028` policy.

---

### 5.7 Portable provider evidence

`aplan test receipt <runner>` is the cross-process boundary for test execution (`A1-644`,
ADR-0056). One canonical in-toto Statement v1 carries one complete provider observation under
predicate type `https://aplan.dev/attestation/test-evidence/v1`. It serializes the same typed
outcomes as the local batch adapter; there is no parallel pass/fail vocabulary.

The identity is deliberately split:

| identity | contains | invalidated by |
|---|---|---|
| provider execution | runner position/label/definition, complete catalogue, exact request and outcomes, exact input declaration and matched bytes/modes, inherited environment, platform/toolchain/executable and existing file-argument bytes (including wrappers), aplan/protocol identity | anything that could change what the provider ran or concluded |
| repository audit context | commit, complete Git tree identity, modified/untracked counts and exact status identity, start/completion timestamps | recorded for review; never substitutes for material currency |
| derived plan resolution | the exact current plan, policy, ambiguity, locality and all other evidence | any plan or policy edit; recomputed by the consumer rather than stored in a provider receipt |

This separation lets a prose-only plan edit reuse unchanged provider work without accepting an old
whole-plan verdict. Source-revision movement may be allowed only when every stronger material and
tool identity remains exact. Receipt age is an audit fact, not a freshness heuristic.

The statement records `complete` independently of success. Every exact request must have one row.
Only `passed` with no hidden retry can satisfy L3; failed, skipped, expected-failure,
unexpected-pass, retried-pass, flaky, cancelled, timed-out, resource-breach, framework-crash and
incomplete rows remain attributable but non-passing. A successful invocation indistinguishable
from the exact-name miss sentinel is `unproven_selection`; importing it reproduces the same
runner-level `APLAN037` refusal as direct verification. A provider-wide crash, failed auxiliary query,
missing row or material mutation publishes nothing. Diagnostics use the same byte, line, terminal
and credential bounds as local execution. File publication is flush-then-rename atomic, so a crash
before publication cannot replace a prior complete artifact.

Trust is a consumer policy over bytes and external authentication facts. A local file is useful
where the caller controls it. A same-workflow artifact additionally binds its independently
obtained SHA-256 and authenticated repository/workflow/run/event/privilege. A signed policy accepts
a DSSE envelope only after its payload digest, signature and producer were authenticated outside
the receipt parser. The producer object inside JSON is self-reported until it agrees with those
facts. An artifact digest proves integrity, not authorship; an untrusted pull-request producer
cannot satisfy privileged release policy by writing `trusted` into its own file.

The provider schema is derived at `schemas/v1/test-evidence-receipt.json`. Receipt import rejects a forged
subject, unknown type/schema, noncanonical or truncated JSON, duplicate or mismatched identities,
contradictory retry metadata, unsafe diagnostic controls, changed provider/material/tool identity,
transport-policy mismatch and an authenticated envelope over the wrong predicate.

`aplan verify --test-receipts <set>` is the closed, current-only consumer (ADR-0057). Its derived
`aplan/test-evidence-import-set/v1` schema lives at
`schemas/v1/test-evidence-import-set.json`. The set supplies exactly one current observation or
typed rejection per configured provider. It is created by trusted orchestration: trust fields in a
repository-controlled manifest do not authenticate themselves. The consumer re-derives current
provider, material, tool and repository identity, then resolves the exact current whole plan and
all evidence. It starts no catalogue, skip, probe, batch or exact-test command, and it never drops a
rejected row in a way that could narrow ambiguity. `--execute` remains explicit full direct
execution; changed-input refresh is a separate operation (`A1-646`).

`aplan verify --refresh-test-receipts <set>` is that separate operation (ADR-0058). It requires an
explicit committed base plus either a clean committed head or an explicit dirty-worktree mode.
Git-native deltas include additions, modifications, deletions, both rename sides, the index and
untracked non-ignored files; missing/non-ancestor history and non-UTF-8 names refuse. The default
invalidation unit is a complete provider bound to its exhaustive `evidence.observation_inputs`.
Optional `test.inputs_by_test` declarations map exactly one set of repository input globs to exactly
one owner for every listed test. They are distinct from `covers_by_test`: one controls execution
invalidation, the other proof locality. Any missing or overlapping owner, new test/path, changed
configuration, provider, environment, toolchain, protocol or undeclared material edge widens to the
complete provider.

Selected identities still use the configured batch protocol. Authenticated prior outcomes may be
merged only after their complete base receipt is validated, and a replacement set is manifest-last
atomic only after every current catalogue member has an attributable outcome. The ADR-0057 importer
then recomputes the exact current whole-plan verdict; incremental selection never filters tasks or
citations. Structured provenance carries the Git boundary plus reused, refreshed, widened,
unavailable and selected-test counts. A local refresh cannot replace an externally authenticated or
signed receipt, because it cannot recreate that authentication. `--refresh-shadow` executes the
complete direct oracle and refuses disagreement, but remains evaluation evidence until `A1-647`.

Receipt-backed results report `whole_plan + receipt` provenance separately from freshness and
completeness. Paths and secrets from the import boundary do not enter diagnostics. Export itself
still makes no claim that a current plan passes.

## 6. Rule catalog

Stable codes. Adding one is a breaking change for consumers, so they live in one
module (`diagnostic::code`) rather than being sprinkled through the validator.

**Today (shipped).**

| Code | Rule | Severity | Rung |
|---|---|---|---|
| `APLAN001` | Frontmatter satisfies the derived schema | error | n/a |
| `APLAN002` | Task ids are unique | error | n/a |
| `APLAN003` | `blocked_by` references a task that exists | error | n/a |
| `APLAN004` | The dependency graph is acyclic | error | n/a |
| `APLAN005` | `done` carries at least one piece of evidence | error | L0 |
| `APLAN006` | No task blocks itself | error | n/a |
| `APLAN007` | A `done` task does not depend on an unfinished one | error | n/a |
| `APLAN008` | `traces` anchors resolve to a real heading or node | error | n/a |
| `APLAN009` | Active tasks carry an effort estimate | **warning** | n/a |
| `APLAN010` | A `note` used as evidence carries a date | error | L0 |
| `APLAN020` | A live task is not blocked by a `dropped` one | error | L0 |
| `APLAN021` | A `milestone` has members and carries no effort or paths | **warning** | n/a |
| `APLAN022` | Every task is a member of exactly one cut, or is `cross-cutting` (opt-in, `require_cut_membership`) | error | n/a |

`APLAN020` is numbered out of sequence deliberately: `APLAN011`-`APLAN019` were
already reserved for the ladder below when the deadlock rule shipped, and a rule code
is a contract with consumers, it is not renumbered for tidiness. `APLAN021` (milestone
hygiene, ADR-0008) is the first code past the reserved ladder block; it warns because
the `milestone` tag silently excludes a task from every work-view, so a mis-tag or an
empty cut must be surfaced, not swallowed.

**The ladder, and evidence policy.**

| Code | Rule | Severity | Rung | Status |
|---|---|---|---|---|
| `APLAN011` | `commit` evidence resolves to an immutable commit reachable from `HEAD` | error | L1 | done |
| `APLAN012` | `commit` evidence touches at least one path in `paths` | error | L2 | done |
| `APLAN013` | `test` evidence names a test that exists | error | L1 | done |
| `APLAN014` | `test` evidence names a test that **passes**, and a failure carries a bounded, redacted tail of what the runner printed so it can be diagnosed from the CI record | error | L3 | done |
| `APLAN015` | `file` evidence resolves at the cited rev | error | L1 | done |
| `APLAN016` | `url` evidence is reachable | warning | L1 | exists (`A1-041`, opt-in) |
| `APLAN017` | Evidence meets the configured minimum strength | error | n/a | done (opt-in) |
| `APLAN018` | A closed task's `traces` anchor has changed since closure | warning | n/a | done (opt-in) |
| `APLAN019` | `paths` globs match at least one tracked file, once the task cites a `commit`, `file` or `test`: where nothing is bound to the paths, nothing binds to nothing (`A1-138`) | warning | L1 | exists (`A1-022`) |
| `APLAN023` | A `done` task's `commit`/`file` evidence is path-checkable at all | warning | L0 | done |
| `APLAN024` | A `done` task's commit evidence is about that task, not mostly other work | warning | L2 | done (`A1-062`) |
| `APLAN025` | A cited test belongs to the part of the repository the task's `paths` cover | warning | L2 | done (`A1-063`, `A1-065`, `A1-069`) |
| `APLAN026` | A tag that looks like a member of a declared family is one | warning | L0 | done (`A1-076`) |
| `APLAN027` | A cancelled cut's members are cancelled too | warning | L0 | done (`A1-084`) |
| `APLAN028` | A `done` task stamps every trace it stamps any of | error | n/a | done (`A1-098`) |
| `APLAN029` | A declared tag family is carried by at least one task | warning | L0 | done (`A1-085`) |
| `APLAN030` | A task carries the number of members a declared family requires | error | L0 | done (`A1-133`) |
| `APLAN031` | A cited `file` is inside the task's `paths` | error | L2 | done (`A1-045`) |
| `APLAN032` | Every `paths` entry is a glob this tool can compile | error | L0 | done (`A1-066`) |
| `APLAN033` | A `prefer_after` entry names a task that can actually be preferred | error | L0 | done (`A1-150`) |
| `APLAN034` | A `traces` target outside this repository is declared as external | error | L0 | done (`A1-143`) |
| `APLAN035` | Commit evidence names a commit that did work, not a merge | warning | L1 | done (`A1-145`) |
| `APLAN036` | A configured test runner lists at least one test, or says it expects none | warning | L1 | done (`A1-064`) |
| `APLAN037` | A test that passed under `--execute` is distinguishable from one the runner never ran | warning | L3 | done (`A1-167`) |
| `APLAN038` | A `done` task has answered the acceptance criteria its own body states | warning | L0 | done (`A1-224`) |
| `APLAN039` | A `done` task has not ticked a prompt this tool wrote as though it were a criterion | warning | L0 | done (`A1-271`) |
| `APLAN040` | A task declares each `paths` entry once | warning | L0 | done (`A1-267`) |
| `APLAN041` | The generated `AGENTS.md` block says what this repository's policy says | warning | L0 | done (`A1-273`) |
| `APLAN042` | A `revisit_at` names a real instant, on a task that is parked | warning | L0 | done (`A1-264`) |
| `APLAN043` | A live task does not carry the scaffold's own prompt beside the criteria its author wrote | warning | L0 | done (`A1-299`) |
| `APLAN045` | A live task's traced section is unchanged since the task was last touched | warning | L1 | done (`A1-301`) |
| `APLAN046` | A done task's ratifiers are done | error | L0 | done (`A1-306`) |
| `APLAN047` | A `contains` names a milestone that exists, and closes no loop | error | L0 | done (`A1-333`) |
| `APLAN048` | The released version has moved as far as the closed tasks require | error | L2 | done (`A1-313`) |
| `APLAN049` | The artifact agrees with what the plan claims about compatibility | error | L3 | done (`A1-314`) |
| `APLAN050` | A recorded `version` is on a release, and is a version this tool can read | error | L0 | done (`A1-339`) |
| `APLAN051` | A tag is not one edit from a word that changes what this tool does | warning | L0 | done (`A1-212`) |
| `APLAN052` | `cross-cutting` is not on a task that is a member of a cut | warning | L0 | done (`A1-212`) |
| `APLAN053` | A task that is `in_progress` has something in the repository to show for it: its declared `paths` match at least one tracked file | warning | L1 | done (`A1-359`) |
| `APLAN054` | A closed task's mutable `file` evidence is covered by a current fingerprint | warning | L1 | done (`A1-196`, opt-in) |
| `APLAN055` | Captured dates are used only for authored note evidence | error | L0 | done (`A1-197`) |
| `APLAN056` | A task's owner belongs to the declared owner vocabulary | warning | L0 | done (`A1-190`, opt-in) |
| `APLAN057` | Every inbox entry is one canonical, valid and cross-record-consistent V1 capture | error | L0 | done (`A1-514`) |
| `APLAN044` | A closed task claims less of the released version than its level implies, without saying why | warning | L0 | done (`A1-312`) |

Every diagnostic carries a stable code, a severity, an optional task id, an optional
file, a message, and an `agent_instruction`. A **byte span** into the source YAML, so a
renderer can underline `/status` rather than name it, is *planned* (§7), not yet on
`Diagnostic`.

---

## 7. Crate architecture

Uses `aplan-<role>`, with the bare `aplan` package providing the library and binary so
`cargo install aplan` works. `aplan-core` is the runtime dependency sink: it has zero
intra-workspace dependencies, enforced by a test. Runtime support crates depend on it;
the `aplan` package composes those crates into the public CLI, and `xtask` plus
`aplan-e2e` are workspace-only development packages.

```text
    aplan (CLI) --------+
                        v
    aplan-tui ---------+
                       v
    aplan-mcp ----> aplan-app ----------> aplan-core
                       |                      ^
                       +----> aplan-output ---+

    aplan -> config, output, evidence, interop, attest, mcp
    runtime support crates -> aplan-core where they consume the domain model

    aplan-attest -> config, core, evidence, output
    xtask and aplan-e2e -> the runtime crates they exercise
```

The first three lines isolate the shared use-case path; they are not a claim that the other
runtime support crates route through `aplan-app`. The composing `aplan` crate keeps their
transport and integration concerns, while CLI, MCP and the TUI call concrete application
operations without either delivery adapter depending on another.

The application layer depends on `aplan-output` only for its typed graph projection input
(`GraphView`); terminal painting still belongs to the adapters, and neither `aplan-app` nor
`aplan-output` depends on Ratatui or Crossterm. This preserves one canonical graph selection,
edge policy and lane layout without pulling a terminal framework into non-TUI clients.

The CLI text graph has a presentation layer over that canonical scene (`A1-456`). Selectors and
`--reach` alone decide membership. `--focus` only marks one already visible task and its visible
direct dependency neighbours; `--critical` overlays the deterministic whole-plan route without
changing scope or enabling cut-membership edges. Core returns that route's exact nodes and steps
from the same predecessor table as `critical-path`, retaining zero-weight milestone passthroughs
that the work-only report intentionally hides. The text adapter adds exact status words, fixed
role badges, edge glyphs, a legend and stream-resolved styles after selection. JSON, DOT, Mermaid,
the undecorated `GraphView`, and the TUI scene therefore do not acquire CLI decoration. A cyclic
ordering graph reports that the route is unavailable; a scoped view reports visible and total
route nodes and steps rather than presenting a fragment as the whole route. Visibility is counted
once per exact route-step identity, not once per matching scene edge record, and an omitted edge
with no connector is not visible (`A1-565`).

An application operation receives an `IdentifiedPlan`, not a `Plan` paired later with a fresh
filesystem scan. A source load returns the exact bytes beside what it parsed, including candidates
which are not currently tasks. A persistent exact hit may carry their SHA-256 identities instead
when no body projection or mutation requested source bytes; canonical application identity treats
the bytes and their digest-only form identically. This keeps an asynchronous result and its stale-
result digest about the same input without retaining megabytes solely to hash them. Resolved
operation policy is hashed directly; the application layer does not reopen the selected
configuration and risk labelling an old policy with newer source bytes. The identified load is not
a filesystem-wide transaction.

Mutation clients receive an immutable `ChangeSet` before anything is written (`A1-436`). It binds
the complete application input identity to exact before/after file content, typed semantic effects,
the post-state diagnostic delta and denial verdict, and any evidence resolution still required. A
commit consumes that reviewed value, reloads and compares its input identity, then publishes the
same bytes through the existing locked batch compare-and-swap writer. No-op, denied and stale
previews are typed non-publications. A writer failure retains its `SaveError` and measures whether
each affected path has its exact before state, exact after state or divergent bytes, then derives
an unchanged, complete, partial or diverged batch state. An interactive client retains the reviewed
change and safe form input through that result, gives state-specific recovery instead of claiming
batch atomicity, and treats publisher success as provisional until a fresh application snapshot
itself contains every after-state. The identified input includes the exact selected-configuration
bytes as well as resolved semantics; a semantic-only embedding is visibly unobserved and fails
closed at publication. Dependency editing is the first concrete operation on this contract; its
existing CLI/MCP operation is a compatibility wrapper over the same preview, policy and commit
implementation.

`PlanStore` is the one CLI, MCP and TUI loading boundary (`A1-589`, ADR-0049, ADR-0050). It
enumerates the authoritative candidate set, conservatively validates file observations, and reuses
typed task artifacts from a private checksummed generation under the operating system's per-user
cache directory. Exact semantic builds may also reuse typed structural reports whose canonical key
includes rule policy and trace-document observations; rendered output and evidence-provider
observations are not cached there. Each operation still constructs the existing borrowed `Graph`
once rather than putting a self-reference in the store. `--cache=off` reads and writes no persistent
state; `--cache=refresh` ignores the prior generation. Any cache error, incompatibility, race or
corruption is a quiet miss and cannot change the plan verdict.

Structural gates may defer parsed-artifact publication until their structural report is ready so
one atomic generation contains both. A semantic-only consumer such as layered graph cannot request
that mode: it publishes the parsed generation when loading finishes, otherwise an identical next
process cannot reuse its work because the report that would trigger publication is never computed.

Its progressive entry points emit the application operation contract (`A1-665`): ordered,
generation-identified lifecycle events over a fixed privacy-safe phase vocabulary, with honest
indeterminate or completed/total measures and child identities for snapshot attempts. Verification
and configured test-catalogue discovery adapt their internal runner boundaries into the same model;
runner labels and output remain in final results rather than progress. The quiet entry points do
not construct a sink. Consumers validate operation, sequence, lifecycle and monotonicity before
presenting an event.

Provider observations deliberately do not live in `PlanStore`. Their separate private,
checksummed and atomically replaced namespace has a different identity and trust boundary
(`A1-590`, ADR-0053). It stores complete raw test catalogues rather than task verdicts, rejects
unsafe paths and symlink substitution, and treats deletion or corruption as `not_observed` rather
than as a plan diagnostic.

| Crate | Responsibility | Publish | Status |
|---|---|---|---|
| `aplan-core` | task model, loader, graph, validate, diagnostic and the private YAML compatibility contract | yes | **exists** |
| `aplan-app` | identified plan context and concrete typed operations shared by CLI, MCP and interactive clients, including configured test-catalogue search and ambiguity projection | yes | **exists** |
| `aplan-output` | formatters: human, json, agent, github, sarif, markdown, mermaid, dot, text; pure stream-progress policy and grace/rate-limit clock | yes | **exists** |
| `aplan` | library + thin binary; clap CLI; `facts()` introspection | yes | **exists** |
| `xtask` | `gen-schema` (exists); `gen-facts` shipped as `aplan facts` + `tests/facts.rs`; `gen-roadmap` (exists, `A1-016`) | no | **exists** |
| `aplan-evidence` | the verification ladder; commit, file, test and URL resolvers; the test resolver also exposes its normalized configured catalogue to application clients | yes | **exists** |
| `aplan-config` | `.aplan.yml` loading and schema-derived `Config` | yes | **exists** |
| `aplan-interop` | import/export adapters and bulk task manifests | yes | **exists** |
| `aplan-lsp` | LSP server behind `aplan lsp` | yes (internal) | **planned** |
| `aplan-mcp` | MCP server behind `aplan mcp` | yes | **exists** |
| `aplan-forge` | one-way mirror to GitHub / GitLab | yes (internal) | **planned** |
| `aplan-attest` | tamper-evident plan audit export plus canonical portable test-evidence statements and trust validation | yes | **exists** |
| `aplan-tui` | terminal ownership, event decoding and Ratatui painting behind `aplan tui` | yes (internal) | **exists** (foundation, `A1-421`) |
| `aplan-testkit` | scenario DSL, tree-spec materializer, proptest strategies | no | **planned** (materializer currently folded into `aplan-e2e`) |
| `aplan-e2e` | scenario corpus + coverage-drift audits | no | **exists** (rule + format audits) |
| `aplan-bench` | criterion benches + fixture generator | no | **planned** |

Throughout this document, tables mark a feature **exists** or **planned**; *which cut*
delivers each planned item is sequenced only in [ROADMAP §10](ROADMAP.md), HLD carries
no phase numbers, so the two cannot drift.

**Error handling:** library crates use typed errors, while executable orchestration in
the `aplan` binary and `xtask` may use `anyhow` for context. Exit-code routing distinguishes
usage/configuration errors from internal failures. Application adapters must retain typed rewrite
sources across that boundary: dependency and portfolio operations map a writer self-check to the
published internal exit rather than flattening an error whose text and process code disagree
(`A1-420`, `A1-720`).

**On `miette`.** `alint` declares it and never uses it, its human renderer is
hand-rolled ANSI over `anstyle`. We have a stronger case than `alint` does, because we
validate YAML frontmatter and can point a caret at a byte range. But `miette` 7.6.0 has
not cut a release since April 2025 (repo active, just slow). **Decision: hand-roll the
human renderer on `anstyle` now, but record byte spans in `Diagnostic` from day one**
so adopting `miette` later is a renderer change, not a data-model change.

---

## 8. CLI surface

Exit codes are a contract. `aplan` adopts `alint`'s four-code contract (ADR-0007,
`A1-012`), which distinguishes a config error from an internal bug so a consumer's CI
can tell *your* fault from *aplan's*:

| Code | Meaning |
|---|---|
| `0` | clean (warnings allowed) |
| `1` | at least one error |
| `2` | usage or configuration error (the invocation, the input, or `.aplan.yml`) |
| `3` | internal error (a bug in `aplan`) |

Most CLI errors are the user's, so `2` is the default; only aplan's own invariant
failures (a `close` that fails its self-check) exit `3`. A hard panic remains `101`, the
Rust convention for an unhandled crash. `facts.json` carries these codes so tooling reads
them from one place, and the version is the single source of truth (HLD §17).

`--progress auto|always|never` is global (`A1-665`). `auto` writes delayed progress only for a
human-format command when stdout and stderr are terminals and `CI` is inactive. `always` is an
explicit stderr-only override, including beside structured stdout; `never` emits no stream
progress. The TUI owns emitted interactive progress in-frame at every setting. Short operations
finish inside a 120 ms grace without flashing a line. The portable renderer is ASCII and
non-animated; non-empty `NO_COLOR`, `TERM=dumb` and reduced motion cannot remove its phase meaning.
Progress never changes stdout, the operation result or exit status. One command-lifetime renderer
spans plan, repository and provider child operations plus later validation and projection
(`A1-667`). It validates each operation's sequence and lifecycle, aggregates concurrent providers
by their bounded public count rather than exposing child or configured identities, and rejects
late or duplicate events. An attended capable terminal gets one width-fitted animated row; a
portable or redirected stream gets rate-limited newline ASCII. The owner clears the transient row
around every ordinary output write and on every terminal outcome without hiding the cursor.

| Command | Purpose | Status |
|---|---|---|
| `aplan check` | validate; hermetic; fails on any error | exists |
| `aplan verify` | resolve evidence (L1/L2); `--execute` for direct L3. `--test-receipts <set>` re-derives whole-plan L1/L2/L3 from an exact complete current receipt set without provider spawns and never falls back (`A1-645`, ADR-0057). `--refresh-test-receipts <set>` plus an explicit Git boundary selectively refreshes authenticated provider outcomes, visibly widens ambiguity, atomically publishes a complete current set, and still derives the whole-plan verdict (`A1-646`, ADR-0058); `--refresh-shadow` compares the direct oracle without changing provenance. Diagnostics may carry a typed exact evidence replacement; `--fix` applies every valid one as one guarded batch, then verifies the result (`A1-199`) | exists (`A1-004`) |
| `aplan test` | `list [query]` lists or searches the exact `test:` evidence namespace through the resolver, retaining runner identity and distinct empty, failed, partial, no-match and ambiguous answers in human, JSON and agent output (`A1-478`). `receipt <runner>` executes one complete listed namespace and writes a canonical provider-scoped statement to `--output <path>` or stdout; it requires explicit producer coordinates/privilege and exhaustive `evidence.observation_inputs`, and publishes no plan verdict (`A1-644`) | exists |
| `aplan ready` | tasks whose blockers are all `done`, ordered by a derived rank: what a task releases, then the critical path, downstream depth, effort and id. Declared `prefer_after` edges then make a stable topological pass over that order; `--explain` says which. Reports the complete local ephemeral claim and coordination scope; `--unclaimed` filters that snapshot independently of durable owner (`A1-125`, `A1-150`, `A1-531`) | exists |
| `aplan why` | `<id>`: what blocks a task -- its own blockers and any unfinished cut, reported as itself with a count of what is behind it; `--all` walks into a cut's membership. An answer that folded anything says so, and the JSON always names its `traversal` | exists (`A1-263`) |
| `aplan version` | `<id>`: what that node owes the released version -- its own claim, or its level's floor, or the strongest of what its members owe. Answered from the plan alone, so it is `check`'s rung; `--format json` names the member that raised it. Without an id it needs the release window and refuses until that exists | partial (`A1-313`) |
| `aplan graph` | Layered terminal text, Mermaid, Graphviz DOT or JSON; whole plan or scoped by selectors and `--reach`, which intersect. An attended bare command opens Layered; redirected bare output remains deterministic Mermaid, and explicit `--format` always wins (`A1-696`). Edges leaving the view and omitted membership are reported rather than hidden. Text alone can emphasize an already-visible `--focus` and overlay the deterministic whole-plan `--critical` route without changing scope. Text uses the same typed multiscale Layered projection and cell scene as the TUI, with level, orientation, peer order, detail, exact width and glyph controls; `--layout lanes` is the explicit byte-compatible row-and-lane document. Redirected layered text requires `--width`; it renders the complete admitted scene or refuses with exact omissions rather than clipping | exists (`A1-219`, `A1-221`, `A1-456`, `A1-681`, `A1-683`, `A1-696`) |
| `aplan show` | `<id>`: one task whole: every persisted relationship and version claim, stable owner, ephemeral local claim and scope, `paths`, `traces` and `evidence` with their fingerprints, plus graph state and typed evidence observations. The default recomputes repository evidence, reads only current retained provider catalogues and never starts configured work. `--refresh-evidence` owns slow bounded discovery with stderr progress; `--require-current-evidence` renders then exits 1 for non-current state; `--no-resolve` skips stored and repository resolution. `--format json` is `aplan/show/v3`; `--body` includes prose | exists (`A1-118`, completed by `A1-428`, claim split by `A1-531`, observations by `A1-590`) |
| `aplan explain` | `rule <APLANxxx>`: the stable severity and invariant from the same `Rule` row that generates the catalogue heading. The vocabulary is explicit, so a task, tag and rule may share a name without hidden precedence; it needs no plan, repository or configuration | exists (`A1-198`) |
| `aplan tree` | `<id>`: one task's neighbourhood in both directions, `--depth N`: what must precede it, and what it releases. The inspection a *derived* order needs (ADR-0017), so `ready`'s rank is checkable rather than asserted | exists (`A1-126`) |
| `aplan schema` | the derived JSON Schema | exists |
| `aplan facts` | the tool's generated facts (rules, formats, commands, kinds) | exists (`A1-011`) |
| `aplan paths` | what share of recent commits each task's `paths` would match, against the plan's own median; advisory, never blocking | exists (`A1-151`) |
| `aplan due` | `--as-of <instant>` (RFC 3339): parked tasks whose `revisit_at` has arrived, longest overdue first. Due for a look, never unblocked; the only command that reads a clock | exists (`A1-264`) |
| `aplan stale` | live tasks unchanged in git for longer than a window, readiest first; parked work included, since a park is a decision with a half-life. Reports active claim age from its own observation clock without a claim refreshing task history; advisory, never blocking | exists (`A1-020`, `A1-275`, `A1-531`) |
| `aplan vacuity` | per rule, the tasks it had a subject in and the tasks whose predicate had nothing to bind against -- the ones it ran on and could not have failed. Hermetic, advisory | exists (`A1-114`) |
| `aplan deprecations` | the runways a plan declares -- announced in one cut, removed in a later one -- and the ways that pair breaks. Hermetic, advisory, `--format markdown` for the table a project publishes | exists (`A1-021`) |
| `aplan attest` | the verified plan as a commit-stamped document, optionally attached as a git note | exists (`A1-040`) |
| `aplan check-docs [<files>...]` | every task id a document names resolves to a task. With no paths, expands the literals and globs in `docs.paths` plus every `docs.membership[].doc`; every declaration must bind. Explicit paths override that set for a one-off check | exists (`A1-148`, `A1-203`) |
| `aplan export-roadmap-json <file>` | the plan's statuses and edges written back into a `roadmap.json` a foreign build reads, merged into the file so the fields this model does not hold are carried through. `--check` gates it | exists (`A1-033`) |
| `aplan export-agents-md` | the agent contract the plan and `.aplan.yml` imply, fenced and gate-able | exists (`A1-038`) |
| `aplan export` | `--format <shape>`: the plan written back out in somebody else's shape, the other half of `import`. This build writes `checklist`, where a cut is a `##` heading and a `done` task is a ticked box; the other shapes `import` reads are refused by name with the one field a generated document would have had to invent. What the shape could not carry goes to standard error, so a redirected document is only the document. `--out` writes, `--check` gates | exists (`A1-189`) |
| `aplan summary` | tasks + effort per phase by default, or per member of the declared family named by `summary.group_by`; readiness per family and unique plan totals. `--at <REV>` reads the task files and discovered config from one resolved commit without a checkout, naming that commit on stderr | exists (`A1-002`, `A1-186`, `A1-194`) |
| `aplan tasks` | the plan as data, including stable owner, complete ephemeral claim records and their explicit local coordination scope, so any other rollup is a `jq` line | exists (`A1-075`, `A1-531`) |
| `aplan drop` | drop a task, or a whole cut, recording the cancellation | exists (`A1-077`) |
| `aplan park` | `<id>... --reason <text> [--revisit-at <instant>] [--tag <tag>]`: atomically move unclaimed `todo` tasks or milestones to `blocked`, append the reason to each body and optionally schedule human review. Existing multiline or otherwise non-local revisit source refuses the whole batch before candidate construction. Active, already parked and terminal targets refuse the whole batch; elapsed time never changes status | exists (`A1-343`, `A1-725`) |
| `aplan unpark` | `<id>...`: atomically return unclaimed `blocked` tasks or milestones to `todo`, clear an ordinary source-local `revisit_at`, preserve body history and tags, and leave unfinished dependency blockers to `ready`. Quoted-key, multiline or otherwise non-local optional-scalar source refuses actionably before any batch target is written. Live non-parked and terminal targets refuse the whole batch | exists (`A1-343`, `A1-721`, `A1-725`) |
| `aplan mv` | `<id>... --cut <cut>` moves tasks between cuts, editing both milestones or neither; `<id>... --phase <n>` changes their reporting coordinate without changing their stable ids. At least one destination is required and a set is one atomic move: every refusal runs before any file is written | exists (`A1-077`, `A1-127`, `A1-171`) |
| `aplan rekey` | `<old> <new>`: change one stable id, its conventional filename, every structured task reference, and any id-based cut policy as one guarded migration. An id continued over multiple YAML source lines refuses before a candidate is built; ordinary one-line plain and quoted ids retain their surgical source edit. Prose is reported for review, never rewritten; terminal work requires `--allow-terminal`, and a local claim on the old id must be released first | exists (`A1-409`, `A1-531`, `A1-725`) |
| `aplan claim` | `<id> [--as NAME] [--from HOLDER]`: record an unauthenticated holder label, source, time and origin in a private Git-common ephemeral store shared by linked worktrees. It never edits durable task `owner`. Taking a held task needs `--from` naming the holder, which is checked; common placeholder identities are warned | exists (`A1-359`, `A1-531`, ADR-0051) |
| `aplan release` | `<id>`: drop a local claim, naming the displaced record. Any caller may run it, including when the task is absent, because a claim with no expiry needs an explicit recovery path independent of the holder | exists (`A1-359`, `A1-531`) |
| `aplan start` | `<id>`: record that work has begun, setting `in_progress`. `APLAN053` checks that durable status against repository paths; the independent local claim is reported when observable but never gates the transition. Refused on a terminal task because nothing reopens finished work | exists (`A1-359`, `A1-531`, ADR-0051) |
| `aplan stop` | `<id>`: record that work has stopped, returning the task to `todo`. Not a closure: it claims nothing about what was done | exists (`A1-359`, ADR-0044) |
| `aplan critical-path` | `[<ID>]`: longest path over `effort_days`, **advisory, never a gate**. With an id, the longest chain that *ends at* that task, cut, container or release: what must finish before that checkpoint can. The whole graph is kept and only the endpoint is chosen, which is why this is sound where the refused `--cut` is not -- filtering first turns a fragment into a false path | exists (`A1-013`, `A1-345`) |
| `aplan init` | scaffold `.aplan.yml`; refuses to overwrite | exists (`A1-009`) |
| `aplan close` | `<id>`: set `done` + attach evidence; derives `HEAD`; resolves the evidence and refuses what `verify` would reject (`--no-resolve` opts out); with `detect_drift`, stamps traces and mutable file evidence from committed content; `--restamp` refreshes both after review. Fingerprint replacement accepts ordinary string mappings and refuses anchors, aliases, tags, multiline values and structural keys before constructing a candidate; never commits | exists (`A1-008`, `A1-109`, `A1-196`, `A1-716`) |
| `aplan amend` | `<id> --replace-evidence <from> <to>`: repair a closed task's evidence in place through the same typed replacement primitive diagnostics use: replace one citation, both named, resolved exactly as `close` resolves one; leaves `trace_fingerprints` alone and, with `detect_drift`, stamps newly introduced mutable file evidence. Structural fingerprint mappings and structural evidence sequences receive actionable no-write refusals before reconstruction; computed repairs share the evidence boundary and publish the whole batch or none | exists (`A1-110`, `A1-196`, `A1-199`, `A1-716`, `A1-717`) |
| `aplan dep` | `add <id> --after <other>...` and `dep rm` add or remove one or more `blocked_by` edges; `apply --from <file>` accepts a YAML list of `{id, add, remove}` mappings and judges every edit against one final graph. All forms support action-result dry-run and human/JSON/agent output. Duplicate or conflicting operations, unknown ids, settled targets, cancelled blockers, membership edits and a cyclic final graph refuse before publication. A safe rewire may be cyclic only in an operation-by-operation intermediate state because no intermediate state is judged or written. Existing scalar sequences use the shared source delta, and every changed file publishes through one guarded batch. `--prefer` remains the single-edge, human-output spelling for the separate non-blocking `prefer_after` relation. A milestone's work membership still belongs to `mv`; milestone-to-milestone sequencing remains a dependency. `add --from` and Blueprint V1 use reverse `blocks` entries when new work must gate existing or explicitly named same-batch tasks, publishing creation and edges together | exists (`A1-124`, `A1-150`, `A1-655`, `A1-556`) |
| `aplan tag` | `add <id> <tag>`, `rm`, and `rename <from> <to>` across the plan: the tag edits, which were creation-only until a family's cardinality made `APLAN030` an error and brought them inside `A1-077`'s test. Each ordinary scalar sequence is edited in its source form: retained member bytes, indentation, quotes, comments and newline convention survive, while anchored, aliased, tagged, multiline and otherwise structural sequences refuse without a write. Judged by the shared post-state check before writing, and written through the locked compare-and-swap path, so a rename is all files or none. The five words that decide what a plan *is* are refused on either side of a `rename`, because that edit reaches every task at once and `tag rename milestone epic` dissolves every cut in the plan, and `tag rm` refuses to take `milestone` off a cut that still holds work, which is the same loss one task at a time and quieter; `add`, `mv` and `drop` change what a task is | exists (`A1-213`, `A1-715`) |
| `aplan supersede` | `<id> <title>`: file the successor to a closed task whose traced outcome no longer holds, and re-stamp the closed one as reviewed. The predecessor keeps its status and its evidence: there is no reopen (ADR-0020). Omitted path, trace and body controls inherit the predecessor's scope and generated template; repeated `--path`/`--trace`, their explicit clear flags and `--body` replace those values atomically. Subject tags are inherited -- not the five reserved words, `cross-cutting` among them -- unless `--tag` supplies them; `--cut` is part of the guarded post-state | exists (`A1-130`, `A1-251`, `A1-343`) |
| `aplan add` | scaffold one task in the plan's id convention with live initial status, every creation-time relationship, optional revisit scheduling and version impact; `--cut` places it. `--from` applies the same contract with forward references and adds evidence-resolving `done` ledger correction, including into a done cut, while refusing `dropped`; the complete batch publishes or none does. `--member <ID>` creates an explicitly identified and phased milestone around existing live work and atomically removes old membership. CLI, application, MCP and TUI adapters share the typed operation and an executable field audit | exists (`A1-029`, `A1-704`, `A1-343`) |
| `aplan capture` | `<title>` creates one open, task-kind intake record without entering the task graph. Optional kind, labels, profile, source retry key, literal/file/stdin/editor body, revisit instant and snapshotted promotion defaults cross the typed application preview and one guarded inbox publication. `--dry-run` and human/JSON/agent output report the full generated id and exact input identity | exists (`A1-515`, ADR-0063) |
| `aplan inbox` | list actionable open captures with a visible deferred count and bounded kind/state/label/revisit/search/limit filters; `show`, `amend`, `discard`, `restore` and `promote` are the retained-record lifecycle. Human commands resolve only an unambiguous canonical prefix from the guarded snapshot, while machine data always carries full ids. Promotion overlays explicit phase/cut/effort/owner/path/tag fields on the snapshotted defaults, transfers title/body through the ordinary typed add and post-state gate, and previews the task, cut and capture writes. The guarded publication binds plan, configuration and complete inbox identity, rolls back a late batch failure, and freezes `promoted_to` plus a canonical request fingerprint. An equivalent lost-response retry returns the original task id; a conflicting retry refuses. Promoted is irreversible and V1 has no purge or delete | exists (`A1-515`, `A1-516`, ADR-0063) |
| `aplan blueprint` | `apply --file <path> --cut <cut> [--set NAME=VALUE]...`: capture one explicit V1 file, bind and resolve its whole graph, and pass the result through ordinary guarded authoring. `--dry-run` reports the selected path and content digest, plan snapshot, canonical cut, every predicted id, affected file and post-state finding; apply reports those same fields with the exact ids actually published. Human, JSON and agent renderings carry the same answer. V1 has no discovery, `list` or `show`. MCP is concretely deferred because the stdio server has neither an attachment channel nor a caller-relative local-path trust boundary; `--format agent` is the usable agent path. The TUI exposes the same copyable CLI-only boundary until it has a file picker and complete multi-task review | exists (`A1-389`) |
| `aplan import` | `<source>`: read another tool's plan. `--format kep`, `checklist`, `roadmap-json`, `spec-kit`, and `openspec` exist; further ecosystems remain candidates, not claimed formats. `--phase` assigns the destination reporting coordinate rather than translating a similarly named source hierarchy. `--cut` places what it writes and `--tag` gives it the vocabulary the plan requires, because a source document supplies neither; the run names any declared `tag_families` member the import would leave missing, as it already names cut membership and effort | partial (`A1-019`, `A1-030`, `A1-031`, `A1-039`, `A1-357`) |
| `aplan mirror` | one-way push to a forge | planned |
| `aplan lsp` | editor server | planned |
| `aplan mcp` | agent server: `identity`, `ready`, `why`, `check`, `verify`, `add`, `show`, `dep`, `close`, `claim`, `release`, `start`, `stop` over stdio | exists (`A1-024`, `A1-102`, `A1-103`, `A1-124`, `A1-118`, `A1-359`) |
| `aplan tui` | interactive terminal interface in the standard binary: Overview, Tasks, Diagnostics, a multiscale layered Graph, Reports and the complete guarded mutation catalogue consume the same typed application boundary as CLI/MCP. It restores raw, alternate-screen, cursor, mouse, focus and paste modes on every supported exit. A feature-free source build keeps an enablement stub; `facts --require build.tui` asserts the package shape | exists (`A1-421`, `A1-444`, `A1-448`, `A1-679`, `A1-680`) |
| `aplan roadmap` | render the plan grouped by cut: rollups, per-cut critical path, `--format markdown` companion (ADR-0009) | exists (`A1-015`) |
| `aplan migrate` | schema version upgrades | when `v2` exists |

The execution spike justified the multi-task surface narrowly: parameters remove repeated text
edits and local keys preserve sibling topology while ids are minted. V1 therefore stops at one
reviewed explicit file, preview, and atomic apply. Discovery, catalogues, composition, replay,
built-ins, installation, and fetching remain outside the accepted boundary.

**Flag conventions**, from `alint`: `--format` is a bare `String` on the clap struct
plus a hand-written `FromStr` on `aplan_output::Format`, specifically so `aplan-output`
carries **no clap dependency** and stays usable as a library. Global flags use
`global = true`. Cross-subcommand constraints that clap cannot express declaratively
are enforced by hand with a clear error, not silently ignored.

**Revision selection is input selection, not a clock** (ADR-0047). `summary --at <REV>` resolves
the revspec once, reads both task blobs and an ordinarily discovered `.aplan.yml` from that commit,
and runs the ordinary loaders over those bytes. It never checks out the tree. The report stays on
stdout and the resolved full commit goes to stderr, so the data schema is unchanged and a clean
`--at HEAD` answer is byte-identical to the unqualified one. An explicit `--config` remains current
caller-selected policy and `--no-config` remains none. Missing roots and invalid historical input
are refused; neither is an empty plan. `--as-of` remains the separate evaluation-clock override an
age report may eventually compose with a selected revision. Committed symlinks are followed inside
the selected tree, never through today's worktree; a root link to `.` selects the top tree and a
broken root link is refused as invalid input. An implicit config above the repository and a
submodule at or beneath the plan root are refused because the named superproject commit cannot
identify their contents. An explicit root lexically inside the invocation repository remains owned
by that historical repository even when today's path has gained its own `.git` marker. A root in a
different repository, including one reached through a symlink alias, remains supported after the
caller selects current policy with `--config` or declines it with `--no-config`; discovery cannot
cross that repository boundary implicitly, whether or not it would find a current caller-side file.

`aplan close P0-003 --evidence commit:d213c9a` is *easier* than editing YAML by hand,
and `aplan close P0-003` with no flag is easier still: it derives the current `HEAD`.
Where evidence can be derived without ambiguity, the current commit or the test that just passed,
the command does so (`A1-008`, `A1-705`). Focused `close --execute` derives the just-passed test
only after the user requests the configured-process boundary; ordinary close remains L2. The write
lands in the working tree and never creates a commit.

---

## 9. Configuration

`.aplan.yml` at the repository root, **shipped** in `A1-001` (the `aplan-config` crate).
It is discovered by walking up from the start path, so `check` and `verify`, given any
path inside the repo, converge on the same top-level file. It is optional: with none, the
built-in defaults apply.

The `Config` type is `schemars`-derived into `schemas/v1/config.json`, gated by
`cargo xtask gen-schema --check`, exactly as `Task` is. Validation is serde with
`#[serde(deny_unknown_fields)]`, so an unknown key is a reported error, never ignored
(as `alint` also validates its config with **serde, not the `jsonschema` crate**; the
committed schema exists for editor autocomplete and the tests). A config the tool ignores
would be drift on day one, the failure this tool exists to catch.

The shipped shape, every key optional:

```yaml
require_effort: true                            # APLAN009, default true
traces_root: .                                  # APLAN008 root, default the cwd
no_traces: false                                # or switch trace checking off
test:                                           # the runner for `verify --execute`
  list: ["cargo", "test", "--", "--list"]       # prints test names, one per line
  run:  ["cargo", "test", "--exact", "{test}"]  # `{test}` is a whole argument, no shell
evidence:                                       # APLAN017, off unless set
  require_min_strength: [commit, test, file]    # a `done` task needs one of these...
  exempt_tags: [phone]                          # ...unless tagged, where a note is best
detect_drift: true                              # APLAN018/APLAN054, stamp + watch traces/files
effort_days_per_week: 7.5                       # ADR-0023, derive durations; no rule, no dates
owners: [ada, grace]                            # APLAN056, canonical task owners; optional
```

**The trust boundary lives here** (§5.5). The `test` runner, `url.check`, `version.current`
and `version.check` are the only places a command may be named, and all are top-level config
only, never a task file. `{test}` and `{url}` are each substituted as a whole argv element
and spawned with no shell, so a task file (which can arrive in a fork PR) can never inject a
command; a template that would splice the placeholder into a larger argument is rejected at load.
What a task file *can* choose is a `url` citation's target, which is why only `https`
citations are handed to the checker (§5.5).

**Which config is trusted is the caller's choice, and `aplan` cannot make it** (`A1-155`).
Discovery walks up from the plan root or the repository, so pointing `--root` and `--repo` at
somebody else's checkout selects *their* `test` and `url` programs -- correct for your own
repository, wrong for a review. `--config <file>` reads exactly that file and discovers none;
`--no-config` reads none at all, so no configured command can run and `test`/`url` citations stay
unchecked rather than passing. A named file that is missing is an error, never a fallback to the
tree under review. Its `root` and `traces_root` values resolve against the selected file's
directory. A test runner's `dir` instead resolves against the selected repository root, so an
external trusted policy runs its command in the tree under review; the plan's own location remains
where git history is read.

Choosing and authenticating the trusted file stays the caller's responsibility: `aplan` reads the
path it is given. A reviewer that needs more than "this path is mine" -- a signature, a digest, a
file only root can write -- has to establish that outside the tool, exactly as it establishes that
the binary it runs is the one it pinned.

**Precedence:** a CLI flag overrides the file, which overrides the default
(`if cli.flag { .. } else { config.field.unwrap_or(default) }`). A flag misused on the
wrong subcommand **errors loudly** rather than being ignored.

**Planned keys**, each shipping with the rule or command that reads it, never before, so
the tool never carries a key it ignores: a `url` reachability switch (`APLAN016`, network
off by default), a `root` key so `--root` need not be repeated, and a `version` field.

**No `extends:` in v1.** `alint` needs remote ruleset composition; a plan validator does
not, and the SRI/SSRF hardening that comes with it is cost we should not pay yet.

---

## 10. Output

One report, many consumers. `alint` ships eight formatters; we should converge.

| Format | Consumer | Status |
|---|---|---|
| `human` | a person reading a failure, coloured, file paths, later carets | exists |
| `json` | deterministic automation; `aplan/check-report/v1` | exists |
| `agent` | every diagnostic carries `agent_instruction`; `aplan/agent-report/v1` | exists |
| `github` | workflow-command annotations, inline on the PR | exists |
| `sarif` | GitHub code scanning, and the lingua franca of static analysis | exists (`A1-023`) |
| `gitlab` | Code Quality report | planned |
| `junit` | generic CI test-report ingestion | planned |
| `markdown` | a PR comment / job summary | exists |
| `mermaid` | generated Mermaid graph views, never hand-edited | exists |
| `dot` | generated Graphviz graph views, never hand-edited | exists |
| `text` | stable plain-text command output | exists |

Colour is a property of the *stream*, not of a formatter: every human renderer writes through
`anstream`, which styles a terminal and strips into a pipe, and `--color` writes the
process-global choice `anstream` consults rather than being threaded to each call site
(`A1-286`). That is what lets one flag reach the renderers, the CLI's own refusals and `clap`'s
usage errors alike. The machine formats gain nothing from it at any setting, which is asserted
rather than assumed.

Progress is also a property of the stream, but its eligibility is stricter than color. It has a
separate global policy because terminal color remains useful in a piped human document while live
progress would corrupt or distract from a protocol. The application emits typed facts with no
terminal knowledge; `aplan-output` resolves the pure environment/mode policy; the CLI owns a
delayed, joined stderr renderer over the complete command; the TUI owns the same state in-frame.
Machine stdout remains byte-identical at every progress setting. Explicit color choices reach
human progress, while machine-format progress remains unstyled even under `--color always`.

Two ideas worth stealing from `alint`'s `agent` format: an **`argv` field** giving the
exact command an agent can run to fix one diagnostic (regression-tested against the CLI
parser, so it can never name a flag the binary does not accept), and a **strict
stdout/stderr split** so progress noise never contaminates the parseable stream.

**SARIF earns its Phase-1 slot.** It makes diagnostics visible in forge security views
and other static-analysis consumers.

---

## 11. Interfaces

### CI
An `action.yml` composite action, the one-line adoption path (`A1-003`). It installs
a pinned release and runs `aplan check --format github`. `verify` runs as a separate,
opt-in step, because it needs `fetch-depth: 0` to see history, and must not enable
`--execute` for fork pull requests (§5.5).

Also: a `.pre-commit-hooks.yaml`, since `check` is hermetic and fast enough to run on
every commit.

### Agents
Two surfaces, and both matter.

1. **`--format agent`.** Structured stdout an agent's shell loop can consume. This is
   `alint`'s entire agent story, and it is a good one: no protocol, no server.
2. **An MCP server** (`aplan mcp`). The official Rust SDK,
   [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) **2.2.0 (2026-07-08)**, is
   maintained under the Model Context Protocol org with a weekly cadence and is
   production-usable.

   Expose a **deliberately bounded** tool surface. A CLI command earns an MCP tool when it is on
   the repository-scoped loop an agent must complete and can use the same implementation and
   gates as the CLI: select or explain work, inspect one task, validate it, create or order one
   piece of work, close it with evidence, or coordinate who is working on it. Server identity is
   the bounded `facts --identity` projection: it identifies the configured process while the
   command's capability inventory and requirement checks remain host operations.

   Broad and alternate read views, including every rollup, remain CLI machine output instead of
   making each presentation a tool. Consequential rewrites whose semantic correctness needs human
   judgement remain human operations. Repository bootstrap, bulk interchange, artifact
   generation, signing, capability inspection and process administration remain host operations.
   `TOOLS` and `MCP_COMMAND_POLICY` record both surfaces beside one another. A policy row either
   mirrors a command, states the boundary of a deliberately narrower projection, or records an
   exclusion reason. An executable census compares the rows with both the clap command tree and
   the tool registry, so neither a new command nor a new tool can be forgotten or counted twice.

   A small tool surface limits duplicated policy and unnecessary context. The command-policy
   census keeps that boundary explicit and executable.

   `close` must go through the same evidence gate as everything else. An MCP mutation
   cannot replace repository evidence with a caller-supplied confidence score.

   **Per-call repository selection is atomic context selection** (`A1-604`). Omitting `root`
   uses the context fixed when the server starts. Supplying it accepts either (a) a repository
   directory that carries `.aplan.yml`, from which the configured plan root is derived, or (b)
   that exact configured plan directory, whose nearest applicable configuration and containing
   Git worktree are discovered. Plan bytes, config-relative trace and external roots, structural
   and verdict policy, vocabularies, ID strategy, evidence providers, configuration diagnostics,
   cache identity and Git operations all use that one resolution. A task subdirectory that is not
   the configured root is not a smaller plan; it is an ambiguous partial context and is refused.

   Resolution canonicalizes an existing directory before discovery, so `..` and a directory
   symlink select the target's context rather than combining a lexical parent's config with target
   bytes. Missing and non-directory roots are refusals. A `.git` directory or linked-worktree
   `.git` file is a repository boundary: a config-free nested worktree does not inherit an outer
   repository's unrelated config. In a config-free worktree only its root and conventional
   `aplan/` plan are accepted. Outside Git, an existing config-free directory retains the legacy
   meaning of a plan root and its parent is the repository fallback. A config may deliberately
   declare a plan elsewhere, as the CLI permits; provenance marks paths outside the selected
   plan's repository without printing the unrelated host path.

   Explicit-root payloads are `{context, result}`. `context` names repository versus plan form,
   gives repository-relative plan/config paths, and reports symlink resolution; omitted-root
   payloads retain their existing shape. This makes a cross-repository answer auditable without
   leaking ancestor host paths. The root is also a trust choice: the selected config's provider
   commands become the non-executing verification policy (test discovery may run, cited tests do
   not), just as for the equivalent CLI invocation. A host must not expose arbitrary root
   selection to an untrusted MCP caller.

   This does not weaken the identity-only startup state from `A1-259`. If the launch
   configuration cannot be read, the process has no usable plan-tool context and an explicit
   `root` does not bypass that refusal; only `aplan_identity` answers until the server is fixed and
   restarted.

### Editors
`aplan lsp` over stdio, and every editor integration is a thin config that spawns it,
`alint` proves this scales to eight editors without reimplementing diagnostics anywhere.

**Important correction for the stack:** `alint` uses `tower-lsp` 0.20, which is **dead**
frozen since 2023-08-11. The maintained successor is
[`tower-lsp-server`](https://github.com/tower-lsp-community/tower-lsp-server) **0.23.0
(2025-12-07)**, same tower/async API. (The alternative, rust-analyzer's `lsp-server`
0.8.0, is healthy and rust-lang-backed but is a low-level synchronous scaffold: you own
the dispatch loop.) **Use `tower-lsp-server`. `alint` should migrate too.**

LSP features, in value order: diagnostics on `did_open`/`did_change`/`did_save`;
completion of task ids inside `blocked_by`; go-to-definition from a `blocked_by` entry
to that task's file; hover showing a blocker's status and title; go-to-definition from a
`traces` anchor into the document heading. That last one makes the anchor a real link,
which is the whole point of `traces`.

### TUI

The detailed product, interaction, dependency and evaluation design is
[`docs/design/tui.md`](docs/design/tui.md); ROADMAP sequences its functional releases and the plan
holds their executable tasks. The durable architecture is:

- **Ratatui immediate mode over Crossterm**, beginning with Ratatui's minimal `crossterm` feature
  rather than its top-level default bundle. The backend dependency retains some defaults of its
  own, so the resolved feature graph is measured rather than inferred. On Unix, `aplan-tui` also
  names that same Crossterm 0.29 package directly to feature-unify `use-dev-tty`; the
  level-triggered source consumes a ready key before a simultaneous `SIGWINCH`, avoiding the
  edge-triggered lost-wake hang proved by a stop/queue/resume PTY regression. This is one package
  version, not a second backend. No component framework, async runtime, input widget or effects
  library is admitted without a measured need. Integrated, feature-gated and companion-binary
  packaging are measured before deciding which users pay the TUI's binary and compile cost.
- **Typed shared application services.** The TUI consumes the same plan, graph, diagnostics,
  verification and mutation gates as CLI and MCP. It neither invokes the CLI as a subprocess nor
  writes task files directly. One read and one mutation prove the seam before it earns a crate.
- **Terminal ownership is explicit across process handoffs.** The live Add form leaves every
  renderer and terminal mode before starting `VISUAL`/`EDITOR` directly on one private bounded
  Markdown draft. It accepts returned bytes only through the ordinary preview and guarded publish
  path, then re-observes plan, configuration and repository identity before certifying the resumed
  snapshot. No shell, persistent transcript or second task writer is introduced.
- **Portability evidence separates build, PTY and physical-terminal claims.** One independent
  platform-neutral module in the existing consolidated integration binary runs the same
  mode/focus/resize/editor lifecycle on standard public Linux, macOS and Windows runners without a
  second link target. Unix signal/job-control proofs remain in the deeper Linux corpus. Private CI
  allocates no macOS/Windows hosted runner, so those rows stay unavailable until public CI observes
  them; manual tmux, SSH and emulator evidence is recorded separately.
- **Work discovery is one typed projection.** `ready`, `why` and `due` are adjacent answers over
  one identified graph and explicit clock, not terminal transcripts embedded in three screens.
  The task inspector retains every ready-rank factor and declared preference, defaults blocking
  explanations to a cut-bounded walk with exact fold counts, and can request the complete closure.
  That closure includes an undefined blocker reached below loaded work; the bounded walk hides one
  behind a cut, and CLI, JSON, MCP and TUI consume the same graph answer. Preference labels name
  whether the inspected task or its preferred predecessor is absent from the ready offer.
  A due revisit is labelled as a time to re-read, never evidence that the task's external condition
  became true. Blank and whitespace-only clock input is canonical snapshot-following state across
  refresh; an explicit epoch or RFC 3339 value remains pinned. Loaded blockers and dependents carry
  task identity into the existing inspector back stack; missing and unloadable references remain
  visible but cannot become false destinations.
- **Explicit model/update/view and effects.** Input maps to semantic actions, effects carry input
  identity, and only events or visible transient work trigger a redraw. A repository plan does not
  need a monitor's permanent frame clock or background data collector. An active TUI performs a
  measured five-second, context-only identity probe on the existing bounded worker. The identity
  separately names plan and resolved configuration digests plus HEAD and exact ordered Git-status
  entries. A changed probe schedules a full replacement; refresh reloads identity after building
  every projection and rejects a raced result, so the old complete snapshot remains visible until
  one coherent replacement is ready. Focus gain probes immediately and manual refresh remains
  available. If deliberate user work owns the bounded worker when a periodic probe is due, the
  snapshot becomes unconfirmed and the overdue probe starts as soon as capacity returns instead of
  advancing its deadline. The probe has its own production input-only loader, so it does not build
  and discard refresh-only verification inputs. Process suspension enters the same focus-epoch
  boundary before stopping; resume repaints retained bytes as unconfirmed and obtains a
  current-epoch observation as soon as the bounded worker permits. A continue delivered after
  screen teardown but before the default stop handler cancels that stale stop decision, so the
  process cannot consume its wake-up and then strand itself. Every observed continue also enters
  that freshness boundary before interactive work resumes, including the first observable event
  after uncatchable `SIGSTOP` and the cancelled handoff. The exact production paths measured a 78.104 ms
  observation and 1.266 s complete refresh at the release median (16.2x); polling adds no
  filesystem-watcher or async-runtime dependency.
  Every production observation receives the worker cancellation token and yields at identified
  load boundaries. Graceful quit therefore cannot restore the terminal and then wait for a probe
  which began immediately before shutdown. A probe uses the identity-only loader rather than
  constructing refresh-only verification projections.
- **Contextual commands and responsive panels.** GitUI's derived command bar and bottom's dense,
  focusable visual hierarchy are interaction references, not application architectures to copy.
  Key handling, the footer, contextual help and the palette consume one semantic action catalogue;
  focus, route and entity selection survive responsive collapse. Search, structured filters and
  action discovery remain separate interactions, and disabled actions state why.
- **Layered tests.** Reducer tests prove state transitions, `TestBackend` plus snapshots pin buffers,
  and a small PTY suite proves launch, input, resize, teardown and exit behavior that an in-memory
  backend cannot see.
- **The shipped lane graph remains an explicit CLI compatibility layout.** `A1-218` measured the
  alternatives on the actual plan and `A1-219` implemented deterministic Unicode/ASCII lanes. The extracted
  `GraphScene` consumes `GraphView` once and carries raw node and edge identity, typed connector
  cells, typed omissions, grapheme-aware terminal-cell bounds, row-range traversal and hit testing.
  Duplicate edges receive deterministic ordinals; missing endpoints and cyclic back-edges remain
  inspectable rather than disappearing. A malformed public `GraphView` with duplicate node ids is
  canonicalized deterministically and exposes a typed `SceneIssue`, rather than quietly looking
  complete. Fan-in joins begin at their actual destination lane, so unrelated routes to the left
  retain both their vertical shape and their sole edge identity. CLI Unicode and ASCII painters consume the scene with
  byte-for-byte parity on valid plans, while display strings are sanitized separately from their
  raw identities. The live TUI uses the later bounded layered `CellScene`; the lane scene remains
  its explicit byte-compatible layout after A1-683; live navigation consumes only the layered
  scene and semantic explorer. `aplan-output` gains only the workspace's existing `unicode-segmentation`
  dependency: no Ratatui, terminal backend or graph-layout engine. A second layout remains an
  adapter-level option only after it meets the MSRV/display-width gates and a measured
  comprehension problem earns its added dependency and parity surface.
- **Static layered text is an adapter over application geometry, not a second layout.**
  `aplan graph --format text` (or explicit `--layout layered`) asks `GraphWorkspace` for the same plan, phase,
  milestone or task projection used by the TUI, measures cards and peer hints with shared
  application policies, and runs the same bounded `CellSceneJob`. The application-to-output
  adapter carries typed node identities, represented tasks, ranks, peer order, phase membership,
  edge ordinals/provenance and exact omission accounting; a missing semantic node is an explicit
  invariant error rather than a panic. `aplan-output` paints only validated rectangles and routes.
  Plan and phase levels first select a source-bound semantic window capped at 10 aggregate cards
  and 20 visible aggregate edges (or a tighter caller budget); milestone/task detail retains the
  general graph window. Both clients expose the same continuations and raw-task omissions, so an
  oversized source remains traversable without widening the 64/96 cell-scene work ceiling.
  Redirected output requires an exact width and is unstyled; an attended terminal may discover its
  width. If the complete scene does not fit, the command refuses with the required width and
  narrowing/export alternatives rather than clipping. Static Auto orientation retains both
  validated direction candidates until this exact width is known, then chooses the best-quality
  complete scene which fits; an explicitly requested direction remains forced, and the TUI's Auto
  scene remains viewport-independent. Layered is the text-layout default; `--layout lanes`
  remains the explicit compatibility document with its prior bytes. A bare attended command
  chooses text, while a bare redirect chooses Mermaid and explicit formats always win. Mermaid,
  DOT and JSON never accept terminal-only controls (`A1-681`, `A1-687`, `A1-683`, `A1-696`). At
  terminal-cell adaptation, a route coincident with an unrelated card border is deterministically
  diverted while preserving ports and flow direction; a route which cannot be repaired still
  refuses the complete scene.
- **The Graph destination shares graph meaning rather than lane coordinates.** Its application
  projection composes the CLI selector dimensions with core reachability, the shared `tree` walk
  and critical-path analysis, then narrows the canonical `GraphView`. The TUI visits only visible
  layered scene cells, maps mouse cells back to typed node/edge identities, and keeps the task
  inspector on the same selection. The lane scene remains a compatibility adapter rather than the
  live painter's coordinate source. Search does not mutate scope; grouping omissions and out-of-scope reasons
  remain explicit. Exact row/column extents and scope breadcrumbs were retained instead of a
  minimap because they expose values without consuming the small-terminal drawing area.
- **The multiscale explorer is one coordinate-free semantic projection.**
  `GraphWorkspace::project_explorer` applies the existing selector and structural scope once, then
  presents plan, phase, milestone or task detail without changing the canonical task graph. Phase
  is an ordered band, never a node. A milestone card partitions admitted raw identities into its
  checkpoint and member work, keeps the checkpoint's persisted status separate from exact member
  status/readiness counts, and retains every internal or crossing task dependency as provenance.
  An acyclic task graph can therefore yield a cyclic milestone quotient: those aggregate edges are
  typed layout feedback, while strongly connected task identities remain an invalid-plan state.
  Ungrouped work remains a task node. Cross-phase and cross-cut edges, disconnected components,
  empty selected phases, milestone-only and no-milestone inputs are explicit composable
  conditions. Narrow phase focus carries exact previous/next phases, omitted semantic identities
  and boundary-edge provenance. Milestone and task detail hand an identified dependency-only
  source to the existing bounded window, so off-view relationships keep its exact continuations
  and accounting. Search returns raw admitted task ids without narrowing scope. Collapse, drill
  and return retain one raw selection, and their inverses name the immutable input digest so a
  refreshed workspace refuses stale history. The result contains no coordinates, glyphs or
  Ratatui types; layered layout and painting consume it in later stages (`A1-677`).
- **Layered geometry is a bounded terminal-cell application scene, not painter state.**
  `CellSceneJob` consumes one `GraphExplorerProjection`, grapheme/display-column card sizes,
  phase groups, peer-only ordering hints, orientation, budget and an optional prior scene. A
  private adapter pins `graphlay` 0.3.1 with default features disabled and exposes none of its
  identities, floating coordinates, errors or options. Fixed-effort cycle disposition, ranking,
  long-edge handling, crossing reduction, compaction and orthogonal routing run for at most 64
  nodes and 96 edges; the kernel independently caps routing waypoints at 1,536. One checked
  eight-units-per-cell pass emits integer node boxes, ranks, phase boundaries and label anchors,
  exact edge routes/ports/arrowheads, bounds and quality/stability metrics. The adapter permits any
  valid node-boundary port because the matched Lanes gate found that the facing-port preference
  added avoidable bends; both directions remain within its crossing and bend budgets (`A1-688`).
  It refuses node overlap
  and refuses the whole current scene when any route is collapsed, non-orthogonal, non-monotone,
  off-port, out of bounds or node-crossing, or when a non-empty phase band lacks valid containment.
  The fallback preserves a typed omission tied to every admitted identity and edge; a nominally
  complete partial scene is never paintable. Phase lanes occupy only the cross-flow grouping axis
  and never create rank or an edge. Quotient-only feedback, canonical
  task-cycle feedback and other kernel reversals remain distinct. Auto compares both directions by
  crossings, unsafe routes, bends, displacement and area, keeps a prior direction within 12
  percent, and recentres the same semantic focus when it really changes. Status-only input churn
  reuses cells; other relayouts report displacement and typed causes. Completion is generation
  tagged for off-input-thread execution. Failure retains prior geometry only as a recovery seed,
  never as current paint or hit state, and exposes an exact scrollable/selectable fallback with
  node/edge omissions. The lane `GraphScene` remains the explicit supported compatibility adapter
  (`A1-678`, `A1-683`).
- **The live Graph painter is a semantic Ratatui widget, not a formatted graph document.** It
  consumes the validated `CellScene` directly and paints phase outlines, compact or comfortable
  cards, routes, ports, arrowheads, feedback/bundle forms, crossings and junctions in display-cell
  coordinates. Routes are painted before cards; explicit ports and target arrows are restored over
  card borders. Selected, directly related and unrelated structure remain distinguishable without
  color. Relations Only filters routes and their derived crossing/junction marks without changing
  scene identity or scope. One shared scene-to-screen transform drives clipped paint and node,
  edge, phase, port and scrollbar hit regions; labels are sliced only at grapheme/display-cell
  boundaries. Cell loops are clipped to the viewport, fallback formats only its visible node slice
  from an extent cached at projection time, and repaint performs neither semantic graph traversal
  nor layout. Plan/phase selection updates selection-dependent projection fields and retained
  focus in place; bounded detail reprojects when its anchor can change membership. Resize and
  refresh rebuild only the visible source-sized destination, while route entry reconciles a
  deferred destination from the adopted snapshot. Auto density changes at deterministic width/height thresholds with a
  hysteresis band; a density transition receives a fresh deterministic layout so returning to a
  smaller card policy does not preserve expanded whitespace, while selection-only changes reuse
  exact geometry. The synchronized inspector explains an automatic/contextual anchor as a
  presentation choice, never plan priority or evidence. Narrow screens retain identity,
  phase/rank, exact extents, directional off-screen counts and adjacent-phase cues. Failed or
  over-budget geometry becomes a complete scrollable and selectable semantic list. Ratatui Canvas
  is rejected for this release because sub-cell edge marks cannot share the exact label/hit-test
  grid and add no information to validated orthogonal routes. No minimap consumes task cells
  (`A1-679`).
- **Graph interaction follows semantic levels and cell geometry, never formatted order.** A first
  direct visit opens Overview on a visibly explained in-progress, ready or stable first-work
  anchor; later direct visits restore the last valid in-memory level. Enter drills through phase,
  milestone and task detail, while Escape restores exact selection, continuation and viewport
  state from a bounded history. Contextual entry from another destination opens the exact raw
  identity, temporarily widening a saved scope when required, and restores both the origin and the
  prior Graph session. Arrow movement consumes scene direction, ranks and visible dependency
  edges. Relation keys cycle every sibling around their original subject but continue through a
  chain when only one relation exists. Dedicated commands traverse layers, sources, sinks,
  critical work, milestones, ordered phase bands and diagnostic-bearing nodes without confusing
  phase order with dependency flow. Manual pan, focus, compact fit, semantic level and source-bound
  continuation are distinct states. Readability, plan-priority, id, title, status/readiness and
  criticality ordering affect only peers and expose their crossing cost; Auto or pinned direction
  and density survive refresh and resize. Every relayout reports stability or exact movement and
  cause. Search and relation following open omitted raw identities through typed task detail or
  continuation APIs, while Relations Only changes paint alone (`A1-680`).
- **An oversized interactive graph becomes an explicit semantic window, never clipped output.**
  `GraphWorkspace::project_window` applies node, edge and peak-lane budgets only after the complete
  selector/scope/membership projection. It preserves or truthfully replaces the requested anchor,
  distinguishes complete, windowed, empty, cyclic and failed answers, and accounts exact source,
  visible and omitted node/edge totals. Typed incoming, outgoing and disconnected-component
  continuations use raw identities and stable non-overlapping cursor ranges, so high-degree pages
  are deterministic, reversible and eventually expose every relationship. Every cursor also names
  the immutable input identity that emitted it; refresh therefore invalidates history instead of
  reinterpreting an ordinal range against different relationships. The lane budget is measured by
  `GraphScene`'s canonical geometry pass. CLI text still draws the complete view or refuses it, and
  Mermaid/DOT remain complete; the bounded contract is an interactive application projection, not
  a change to exported graph meaning (`A1-668`).
- **The TUI navigates that window as semantic state, not as a painted crop.** A fitting projection
  stays complete; an oversized one opens on the selected raw id (or a visibly explained stable
  replacement) with exact visible/source and omission counts. Typed continuation selection opens
  incoming, outgoing, membership, component and high-degree sibling pages, while bounded history
  makes each transition reversible. Direct relationship following and graph search recenter an
  off-window identity; pan remains presentation-only inside the current scene. Neighbourhood depth,
  selectors, structural scopes and membership rebuild over the same complete projection. A changed
  input identity drops cursor history before rebuilding, and a cursor is independently source-bound
  if one reaches the engine anyway. The TUI lays out at most the current 64-node, 96-edge, 20-lane
  default window on a navigation or paint path, while the complete projection remains available for
  relationship and exclusion semantics. Keyboard labels, monochrome badges and pointer targets all
  carry typed identities; no action parses clipped text or connector glyphs (`A1-669`).
- **The Reports destination is a typed projection, not a terminal transcript.** One immutable
  application workspace captures resolved report configuration, cloned domain tasks and the Git
  observations needed by path breadth and staleness. Roadmap, summary, version, paths, vacuity,
  stale work and deprecations then project without I/O as focus, selectors or an as-of date change.
  The body retains the same core/output values the CLI JSON renderer consumes; a separate row
  projection carries exact contributor ids for navigation. Summary, paths, vacuity and stale work
  accept the shared selector. Roadmap, version and deprecations remain whole-plan computations and
  say so even while filters are present. Missing history, an unreadable repository and invalid
  parameters are distinct states, as are a zero-sized sample and a measured zero. Opening Version
  displays the declared `version.current.command` as provenance but never executes it: merely
  visiting a read screen cannot cross into configured-process trust.

The aesthetic goal remains btop/htop **feel**, not features: dense but legible panels, clear
information hierarchy, contextual key hints, mouse acceleration, accessible themes and compact
visualizations whose exact values remain readable.

---

## 12. Integrations

**git.** Read-only, and only in `verify`. Needs: does a SHA resolve; read a file at a
rev; did a commit touch a path (a tree-diff filtered by pathspec). Both candidates do
all three. **`gix` 0.85.0** is pure Rust, needs no C toolchain, and is one of the most
active crates in the ecosystem, but is pre-1.0 with breaking minors, so pin exactly.
**`git2` 0.21.0** is mature and its `diff_tree_to_tree` + `DiffOptions::pathspec` is the
better-documented path for exactly the operation `APLAN012` needs.

*Recommendation:* start with **`git2`** for `APLAN011`/`APLAN012`, because path-diff is
the one operation where libgit2's maturity is a real advantage and correctness here is
load-bearing. Revisit `gix` once the ladder is proven; the C-toolchain-free build is
worth real money for distribution.

**Forges.** One-way mirror only. A CI job may render the *already validated* plan into
forge issues and native dependencies for stakeholder visibility, each marked read-only:
*"managed by `aplan`; edit the plan file."* The plan DAG is validated before projection.

Never sync back. If tracker-side edits must be accepted, they arrive as a **proposed PR
a human reviews**.

---

## 13. Interop

Import adapters translate supported external plan formats into aplan's typed model:

| Source | Shape | Notes |
|---|---|---|
| **Backlog.md** | md + YAML frontmatter, `dependencies[]` | Near-lossless. |
| **task-master** | `tasks.json` | Split into one file per task. |
| **spec-kit** | `specs/NNN/tasks.md` | Ordering is prose; import is lossy and must say so. |
| **beads** | `issues.jsonl` export | Dolt is the source of truth there; we read the export. |

Import must be **honest about loss**. A lossy import that silently drops a field is
precisely the drift this tool exists to detect. Emit a diagnostic per dropped field.

---

## 14. Determinism, performance, scale

**Determinism is a correctness property**, not a nicety: byte-identical output across
runs, no `HashMap` iteration order leaking into diagnostics. `aplan-core` already uses
`BTreeMap` throughout and sorts diagnostics by a *total* `(code, task, file, message)`
key (the determinism fix, `(code, task)` alone let ties reorder with walk order). Test it.

**Cycle detection.** Keep the current iterative DFS so deep cyclic input cannot exhaust the
call stack. Add a `proptest` that throws deep and pathological graphs at it.

**Adopt `petgraph` 0.8.3** anyway. Not for cycle *detection*, we have that, but for
cycle **diagnostics**: `tarjan_scc` returns the actual strongly-connected components, so
we can report *which* tasks form the cycle rather than one path through it. We also get
a tested `toposort` for free, which `summary` and `critical-path` both want. It is
ubiquitous, dual-licensed, and cheap.

**Targets.** A bounded local read over a current plan has an end-to-end p95 below 100 ms,
including process startup and rendering. State and output size are part of that sentence: the target
applies to an exact persistent-snapshot hit and to one ordinary input edit on the current and
representative 1,000-task corpora. A 10,000-task exact hit has the same bounded-read target, while a
clean checkout publishes its parse budget separately; output-dominated commands must begin a bounded
response below 100 ms and report transfer separately. A changed trace or configuration, an unavailable
cache and an explicit external-provider refresh are distinct states with distinct budgets. The cache is
never authority, so deleting or corrupting it may change cost but not bytes, diagnostics or exit status.

`ci/scripts/read-performance.py` records min/p50/p95/max wall time, CPU, time to first byte, output bytes and
deterministic loader/graph/trace work for development and release binaries. Wall time on a busy host is an
observation, not a shared-runner gate; portable gates assert work counts and the existing 1,500/3,000/
6,000-task complexity ratios. `docs/design/read-path-performance.md` owns the complete state matrix,
invalidation model and latest baseline. This replaces the earlier ambiguous and unmet "10,000 tasks in
under 100 ms, cold" claim without weakening its intended scale bound.

---

## 15. Schema versioning and format stability

The committed format is a consumer contract. Therefore:
- `schemas/v1/` tracks the frontmatter envelope. Additive, non-breaking changes bump an
  internal `version` field in place.
- A new field on `Task` fails CI until the schema is regenerated and committed, the
  change is *forced into the diff where a reviewer sees it* (ADR-0002).
- `aplan migrate` exists before `v2` does.
- A stable, versioned, machine-checked format is required for reliable automation.

---

## 16. Quality infrastructure

Copying `alint`'s tiering, which is unusually good:

| Layer | Mechanism |
|---|---|
| Unit | inline `#[cfg(test)] mod tests` in `aplan-core` and `aplan-output`; no hand-typed count here, a generated one lands with `facts.json` |
| CLI black-box | **`trycmd` 1.2.0**, `.toml` cases + `.stdout` fixtures. **Pins the exit-code contract.** |
| Snapshot | **`insta` 1.49.0** as the current dev-only candidate; also the official ratatui TUI recipe, revalidated when admitted |
| Property | **`proptest` 1.11.0**, the cycle finder, and `slugify` against `comrak`'s `Anchorizer` |
| Corpus e2e | **landed** in `aplan-e2e/tests/coverage.rs` as an in-Rust fires/silent table per rule, materialised to real temp trees; migrates to one `#[test]` per scenario YAML later |
| Coverage-drift audit | **landed**, meta-tests walk `Rule::ALL` / `Format::ALL`; a rule or format without a test fails CI (ROADMAP §5) |
| Bench | **`criterion` 0.8.2** |
| Drift gates | `xtask gen-schema --check` and `gen-roadmap --check`; `facts.json` via `tests/facts.rs` |
| Dogfood | `aplan check` on `aplan/`, *a plan tool whose own plan does not validate has no business shipping* |

**Immediate gap, closed.** The CLI crate once had **zero tests** and the exit-code
contract the README calls "a contract" was asserted nowhere. Both are now fixed: the
`trycmd` corpus (`crates/aplan/tests/cmd/`) pins every command, format, and exit code, and
the `aplan-e2e` coverage audit forbids a rule or format from shipping untested. Property
testing is not wired yet: the cycle-finder property test (§4) is the next slice, and
`proptest` will be added to the workspace when it lands rather than declared unused before.

**Borrow `facts.json`.** `alint` generates a machine-readable project-facts file from
the code (rule count, output formats, subcommands) and gates it in CI, after a real
incident: a README claim of "60 rule kinds" was *ten behind reality for months*. We have
already lived the small version of this, a README claiming "six invariants" above a
table of ten. **Generate the counts. Never hand-write them.**

**Landed** (`A1-011`). `facts.json` at the repo root is generated by `aplan facts` from
the code's enumerable registries: the rules (with severity) from `Rule::ALL`, the formats
from `Format::ALL`, the evidence kinds from `EvidenceKind::ALL`, the subcommands
introspected from the clap tree, the MCP tools from `aplan_mcp::TOOLS` (`A1-119`), and
(`A1-060`) `config_keys`, every `.aplan.yml` key
this build accepts, walked out of the derived config schema. That last one is a
*capability* rather than an inventory: a consumer staging a binary needs to assert what
it can do, and a version string cannot answer it. `tests/facts.rs` pins the committed file to that output,
so it fails CI on drift, exactly as `gen-schema --check` gates the schema. Two enablers
shipped with it: rule severity became a declared property (`Rule::severity()`, asserted
against every firing site by the coverage audit), and the CLI moved into the `aplan`
library so its command tree is introspectable. `gen-roadmap` (§18, v0.5) is the next layer
on this foundation.

---

## 17. Distribution

The first public distribution transaction is deliberately small and closed. An immutable version
tag enters the protected `release` environment, where an external reviewer admits one exact source
commit. A fresh hosted runner prepares every tool required by the complete release profile: the
pinned Rust and coverage toolchains, the musl target, and the GitHub publication clients. The exact
source then passes all release gates before any artifact, registry credential, package, or release
is created.

After validation, `ci/scripts/release.sh` builds one static Linux archive and records its SHA-256.
GitHub OIDC obtains a short-lived crates.io token only for a tag run. The ten public workspace
packages publish in dependency order, and each new package must become resolvable from the registry
before a dependent can publish. GitHub release creation is last. Both publication scripts are
rerunnable: an existing crate or asset passes only when its bytes agree, a missing asset can be
filled, and drift is refused rather than overwritten. No long-lived crates.io token is stored in
the repository.

| Channel | Current mechanism |
|---|---|
| GitHub Releases | guarded static archive after exact-source release validation |
| `cargo install` | crates.io package `aplan`, after its nine public workspace dependencies |
| `cargo binstall` | the GitHub archive follows the target-qualified naming convention |
| GitHub Action | composite `action.yml` installs the release tag the workflow pins |

Homebrew, npm, container, and pre-commit distribution remain later channels. They must consume the
same qualified source and immutable artifact identity rather than create parallel release paths.

Single source of truth for the version is `[workspace.package].version`. The gate that
keeps the documentation honest is `ci/scripts/check-version-pins.sh`, and it is
**narrower than the sentence that used to stand here**, which promised a gate failing
"when any documented version string disagrees" (`A1-318`).

That sentence is not implementable, which was measured rather than argued. The unit is
an occurrence and not a file: `docs/upgrading.md` says on one line that a repository
needs a feature "newer than `v0.3.0`", which must move when v0.4.0 ships, and on the
next that most of the page "shipped in `v0.3.0`", which must not. Nor is syntax enough:
matching every `uses: ...@vX.Y.Z` found four, all of them quotations -- a CHANGELOG
entry, two task bodies, and an observation record. A record of an instruction is not an instruction.

So the gate checks install pins in the documents that *instruct* an adopter, and leaves
records alone. On an ordinary `X.Y.Z-dev` tree it compares against the newest release
tag. On a release tree with a bare `X.Y.Z` workspace version it instead requires
`vX.Y.Z` before that tag exists. The distinction is necessary: using only the tag lets
the pre-tag release profile accept the previous pin, then makes the exact same source
fail as soon as its public tag exists.

---

## 18. Roadmap

**Sequencing rationale lives in [ROADMAP.md](ROADMAP.md); the plan (`aplan/tasks/`) is the
source of truth for what `aplan` builds and its status, generated into `ROADMAP.status.md`
(ADR-0009).** This section used to carry the phase plan; two roadmaps drift the moment one
is edited, so it defers rather than duplicates.

The historical phase summary that used to follow this paragraph was removed because it recreated
the drift this section says it avoids: it still assigned the TUI to v0.8 after the active roadmap
moved it. ROADMAP.md carries the current rationale, sequencing, testing plan and cutover checklist;
this document is the design that sits under it.

---

## 19. Decisions that need an ADR

Each of these is a real fork in the road, not a formality.

1. **ADR-0003: numeric rule codes.** We use `APLAN001` because a stable code is a
   consumer contract. Write the closed-numbering decision down so nobody changes it
   accidentally.
2. **ADR-0004: evidence verification levels.** The ladder, and why `check` is hermetic
   while `verify` is not.
3. **ADR-0005: trust boundary for `--execute`.** Why a task file can never name a
   command. Mirrors `alint`'s ADR-0004.
4. **ADR-0006: one-way mirror only.** Record the multi-writer ambiguity and make the
   refusal permanent and public.

Two more were on this list and have since been written: the exit-code contract is
`ADR-0007`, and `paths` as the relevance mechanism is `ADR-0008`. A list telling a reader
to write what already exists sends them to do work that is done, so an entry leaves it when
its record lands (`A1-222`, which added the gate that noticed).

---

## 20. Open questions

- Should `traces` support line ranges, not just heading anchors? Anchors are stable
  across edits; line numbers are not. Probably no.
- Signed evidence (git notes / sigstore) is **no longer an open question**, it is planned
  as `aplan attest` (ROADMAP v0.6).
- Multi-repo plans. The current model is repository-scoped. Do not add portfolio
  semantics without a concrete technical contract and evidence that it is needed.
