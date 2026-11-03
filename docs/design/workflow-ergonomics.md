# Workflow ergonomics

Status: proposed, implementation planned in `M-v0.14`

This proposal covers three related daily-use gaps: seeing what finished recently, reading
Markdown task bodies comfortably in a terminal, and recording an incomplete thought without
pretending it is already a valid task. They share a product boundary: each turns plan data into a
useful human workflow, but none may weaken the evidence-gated task graph to do it.

## Decision summary

1. Add `closed_at` to a done task and write it in the same guarded change as `status: done`.
   The value is the instant aplan accepted the closure claim, not proof of when the underlying
   work happened. Add `aplan completed` for newest-first history, recent-N queries, filters and
   machine output. Existing done tasks remain valid; one bounded Git-history pass may report a
   selected first observation of `done`, with its inference and completeness labelled and never
   persisted as exact fact. Timestamp plus task-id ordering is deterministic for a report, but is
   not presented as an exact causal sequence across simultaneous closes or Git branches.
2. Parse Markdown with a mature CommonMark parser, then own the terminal rendering and layout in
   `aplan-output`. The current leading candidate is `pulldown-cmark` with default features off.
   Do not write another Markdown parser, and do not adopt an end-to-end terminal renderer until a
   measured spike shows that its extra policy, dependencies and capabilities fit aplan better.
3. Add a first-class capture inbox rather than another `Task::status`. A capture has a fixed
   lifecycle but a configurable organizational kind, and may carry a revisit hint and an
   idempotent external-source key. It is not ready work, a blocker, roadmap membership or an
   evidence claim. Promotion atomically creates an ordinary valid task and records the link from
   the retained capture.
4. Implement concrete typed operations through the shared application boundary. Keep broad
   read-only MCP views as bounded resources and reserve MCP tools for bounded actions admitted by
   the existing executable policy. Do not introduce a universal query language, runtime plugin
   system or second mutation engine in the name of extensibility.
5. Ship all three across CLI, MCP and TUI, then run one adversarial cross-surface release audit.
   `M-public` depends on `M-v0.14`, making the requirement executable.

The recommendations are deliberately specific enough to cut work, but the dependency, history
and storage spikes may revise implementation details. If they do, they must preserve the product
semantics above and record the changed decision before code makes it a de facto contract.

## Requirements and non-goals

### Jobs to be done

- Answer "what were the last N tasks we completed?" immediately and reproducibly.
- Filter completion history by time, cut, phase, owner and tag without scraping human output.
- Distinguish an exact recorded close instant from a best-effort historical observation.
- Make `aplan show ID --body` pleasant to read at normal, narrow, no-color and redirected output.
- Render headings, emphasis, lists, quotes, code, links and tables without executing or fetching
  anything named by the task body.
- Capture one line with one fast command, later add context, and deliberately turn it into a task.
- Keep discarded and promoted captures for provenance instead of making deletion the workflow.
- Let a human use an editor and short identifiers for review while keeping stdin, JSON and stable
  idempotency keys available to scripts and agents.
- Give CLI, MCP and TUI the same domain answers while letting each surface present them suitably.

### Non-goals

- Completion timestamps are not time tracking, duration measurement or proof that work happened.
- Completion history is not a general event log. A future audit log must represent more than one
  latest lifecycle timestamp and should not be smuggled into this field.
- Terminal Markdown is not HTML, a browser, a network client or an image protocol. Pager
  integration is a delivery decision, not part of parsing or the semantic document.
- Syntax highlighting does not justify shipping native parsers, a large grammar bundle or a
  hidden network/runtime dependency without measured value.
- A capture is not a weak task. It cannot block, satisfy a dependency, appear in `ready`, carry
  evidence or let an incomplete record bypass task policy.
- v0.14 does not add remote inbox sync, notifications, comments, collaboration or an embedded
  full-screen editor.
- Extensibility does not mean arbitrary capture fields, a generic query DSL, transport-specific
  domain models or repository-loaded executable plugins.

## Shared engine and transport boundary

The three workflows need reusable engine operations, not three new copies of policy. They build on
`A1-420`'s identified `PlanContext` and concrete typed request/response boundary and, for writes,
`A1-436`'s previewable `ChangeSet`. Completion and inbox reads may share small value types for a
limit or cursor, completeness and input identity, but remain separate operations. The existing
equality-based `Selector` is reused where its dimensions fit; this increment does not turn it into
a search expression language or compute a filtered graph with new semantics.

Every read result states which identified inputs it describes. A bounded collection result states
its deterministic order, applied limit, continuation if any and whether the answer is complete.
A continuation is tied to the same input identity and query so a working-tree change is refused as
stale rather than causing duplicates or omissions. CLI JSON, MCP structured content and the TUI
consume those values without reimplementing the query. Human text is a rendering of the value,
not a parsing contract.

MCP follows the shipped `A1-195` selection rule. A focused action that the shared gate can judge
may be a tool; a list or rollup is a bounded resource or resource template. `A1-520` first adds
generated input/output schemas, schema-conforming `structuredContent` alongside concise text for
older clients, identified bounded resources and an executable tool/resource disposition audit.
Its compatibility matrix covers both the handshake-era protocol aplan already serves and MCP
`2026-07-28`, whose lifecycle, result discriminator and resource cache metadata differ. Tool
structured results use an object envelope so the same domain shape remains legal for older
revisions that do not accept an arbitrary JSON value. Mutable repository resources advertise
private, zero-freshness caching.

Completion history and the capture inbox then extend those contracts. Domain collection paging
is not MCP pagination of the resource catalogue: the resource-template URI canonically carries
the concrete request and its opaque continuation, while `resources/list` and
`resources/templates/list` retain their protocol meaning. A continuation is strictly parsed,
size-bounded and unable to widen the selector if altered. No daemon, subscription or hidden
server-side session is required.

For automation, retry safety is part of the mutation contract. Capture accepts an optional stable,
namespaced source key so a caller can retry after losing a response without creating a duplicate.
Promotion is naturally idempotent: repeating an equivalent request returns the already-linked task;
a conflicting request is refused. Preview identifiers remain predictions bound to input identity,
not authorization tokens or replayable patches.

## 1. Completion chronology

### Semantics

Use this optional task field:

```yaml
status: done
closed_at: 2026-10-11T19:24:08Z
```

`closed_at` means "the instant this task's transition to `done` was accepted and recorded by
aplan." The name follows the command and avoids overstating that the tool observed the real-world
work complete. It is a claim in the plan, like status; evidence remains the proof of the work.

The canonical encoding is an RFC 3339 UTC instant at second precision. Normalizing the value avoids
two spellings of one instant and makes lexical display stable. A successful close writes status,
evidence, fingerprints, any release version and `closed_at` through one atomic replacement. A
refused close writes none of them. Restamping or amending a done task preserves its original
`closed_at`. Ordinary `close` does not accept a backdating flag: `--at` already means historical
input selection on reports, and a caller-supplied time would contradict the field's accepted-at
semantics. An explicit import path may preserve a source repository's valid `closed_at`, but must
durably distinguish that provenance from a local record. Otherwise it must leave the time unknown
or record the local import transition according to the ADR; an import-time message that is lost on
the next read is not provenance.

The field is valid only on `done`. Dropping work is not completing it and must not populate the
field. All code paths that can construct or import a done task need an explicit disposition; the
audit cannot assume `close` is the only path.

### Legacy tasks and provenance

Adding a required field would make every existing plan invalid, including this repository. A done
task without `closed_at` therefore remains valid and means "done at an unknown time." This matches
Taskwarrior's documented interpretation of a completed task with no end key and makes migration
honest rather than fabricating precision.

For useful history on an established Git-backed plan, `aplan completed` may derive a fallback from
committed transitions to `status: done`. Report it as `history_first_seen`, never as `closed_at`,
because it can be later than the actual close, can be truncated by a shallow clone, and can be
affected by imports, rebases, clock skew, merges or a later revert. "Earliest reachable commit" is
not a sufficient definition in a DAG: the ADR must define how a candidate is recognized against
its parents, which commit time is displayed, and what happens when several transition commits are
incomparable. The algorithm must:

- walk repository history once for the relevant plan paths, not once per task;
- follow path changes and task rekeys where the repository supplies enough evidence;
- keep the selected commit id with the timestamp and define merges, reverts, repeated transitions,
  skewed/equal timestamps and incomparable candidates deterministically;
- report shallow or incomplete history visibly;
- leave an untracked or unresolved legacy task as `unknown`;
- never write inferred timestamps into task files automatically.

Source and completeness are part of the typed result:

```text
recorded            exact value of the task's `closed_at` claim, with durable local/import origin
history_first_seen  best-effort selected committed observation of a transition to `done`
unknown             no defensible instant is available
```

If the ADR declines a persisted import origin, imported source times cannot use `recorded`; they
remain unknown or take the local import-transition time according to that decision.

History completeness is scoped to the named basis revision and distinguishes at least complete
for reachable history, shallow/incomplete, bounded/capped and unavailable. It cannot claim that
rebased or unreachable history never existed. An ambiguous DAG result remains visibly inferred
even when a deterministic candidate is selected. The task result keeps the basis and selected
commit so a human or agent can inspect it. Historical task data is parsed as untrusted repository
content without executing configuration or following paths out of the repository. This is
separate from `A1-277`: a close transition timestamp is lifecycle metadata, not focused effort,
elapsed duration or external wait time.

Known instants sort newest first by default, then task id ascending for ties. Unknown instants sort
after known ones and remain stable by task id. A reversed view changes the time direction without
making unknown values falsely oldest. JSON includes the source, instant or null, task identity and
filter-relevant fields. This is a stable report order, not proof of which of two same-second or
concurrent branch transitions happened first. Exact causal history would require an event model
with branch/merge semantics and is explicitly not smuggled into this latest-transition field.

### Command contract

The proposed human surface is:

```console
aplan completed
aplan completed --limit 10
aplan completed --since 2026-10-01T00:00:00Z --until 2026-11-01T00:00:00Z
aplan completed --cut v0.9 --owner t --tag cli --phase 2
aplan completed --oldest --history never
aplan completed --at COMMIT
aplan completed --format json
```

`--limit N` is applied after filters and ordering. Zero is refused rather than becoming an
ambiguous unlimited query. A typed `HistoryPolicy` with CLI values `auto`, `never` and `required`
makes cost and completeness deliberate. `never` gives a fast, repository-independent answer using
only recorded values and unknowns. `auto` uses history only when legacy done tasks need it and says
when the repository cannot supply a complete answer. `required` refuses rather than returning a
partial historical answer. Date bounds apply only to known instants, so an unknown task does not
accidentally pass a chronological predicate. The engine may stop early only when the selected
policy, ordering, filters and limit make that result provably complete.
The existing report `--at` keeps its revision meaning: task data, input identity and history basis
all come from that revision. It never becomes a caller-supplied close time.

Human output includes absolute timestamps and a compact source cue. Relative ages may be a TUI
presentation, but are not the only displayed value because they change with the clock. Color is
never the only source distinction. MCP exposes the bounded completion result as a resource
template rather than adding a rollup tool, and the TUI gets a completion-history view plus recent
completions on the overview where space permits.

### Tests and failure cases

The implementation must cover an injected clock, offset normalization, invalid and impossible
instants, refused-close atomicity, restamp/amend preservation, simultaneous timestamps, no Git,
empty Git, shallow history, renames/rekeys, merges, reverts and repeated transitions, skewed commit
times, incomparable candidates, untracked done files, unsupported historical task shapes,
non-UTF-8 Git output, stale continuations, inclusive time boundaries, filter/limit ordering,
no-color and JSON stability. Milestones and dropped work have explicit dispositions. Latency and
memory budgets use repositories large enough to expose per-task walks. A behavior name is required
because adding the field also changes what the existing `close` command records.

## 2. Terminal Markdown rendering

### The boundary

There are two different products that happen to contain Markdown:

1. A human report contains Markdown-authored content, such as a task body inside `show`.
2. A command emits a Markdown artifact, such as a generated roadmap meant to be saved or piped.

Only the first is pretty-rendered. Artifact output remains exact Markdown. JSON continues to carry
the exact body string. The human command gains an explicit raw-body route so existing scripts and
people can recover only the source body bytes, under a documented final-newline rule, without
relying on terminal detection. Redirecting human
output disables ANSI according to the existing color policy, but does not silently change which
semantic document was requested.

This distinction must be an executable census of Markdown-bearing output sites. A future command
cannot inherit pretty or raw behavior merely because its author overlooked the question.

### Dependency decision

The initial recommendation is `pulldown-cmark` with default features disabled and only required
GFM options enabled. Its event iterator separates parsing from rendering, it is pure Rust by
default, and its documented Rust requirement is below aplan's Rust 1.88 floor. aplan should build
the terminal layout and styles over that parser using its existing `anstyle`, `unicode-width`,
folding, terminal-width and sanitization machinery. Display width alone does not define grapheme
boundaries, so the spike must measure a compatible segmentation/line-breaking dependency rather
than claim grapheme-safe wrapping from `unicode-width`.

| Candidate | Strength | Mismatch or risk | Disposition before implementation |
|---|---|---|---|
| `pulldown-cmark` | Mature CommonMark pull parser; tables, task lists and strikethrough options; MIT; parser/render split | aplan must implement layout | Baseline to measure |
| hand-written parser | No new dependency | Markdown edge cases become aplan's permanent parser and security burden | Reject |
| `termimad` | Ready terminal styling and wrapping; MIT | Own Markdown model, terminal/event utilities and palette policy overlap aplan; not a generic CommonMark contract | Compare in spike, unlikely default |
| `pulldown-cmark-mdcat` library | Broad renderer, links, images, math and Ratatui support | Capabilities exceed the requirement; default image/SVG stack, OSC/link policy and MPL-2.0 plus Apache-2.0 licensing need review; it is distinct from the `mdcat` application and latest releases may move MSRV | Compare with defaults off; reject unless measured benefit is decisive |
| `comrak` | Full GFM parser and AST | Default CLI and highlighting stack is much broader than a display-only event parser | Compare parser-only only if the event model proves insufficient |
| `syntect` | High-quality multi-language highlighting | Grammar/theme data and regex backend materially affect size, build time and supply chain | Optional spike; not a prerequisite for semantic code styling |

The spike records resolved packages, duplicate terminal stacks, clean/release build time, binary
size, parser coverage, license compatibility, MSRV, maintenance activity, security posture and
API fit. It records exact measured versions in the report and lockfile but the ADR chooses a
feature/compatibility policy rather than freezing a patch release as architecture. "Time is
available" removes schedule pressure; it does not remove ongoing dependency and maintenance cost.

### Rendering architecture

Parsing produces a small owned, transport-neutral semantic document rather than ANSI text. Parser
crate types do not leak through its public API. Blocks and spans carry an exhaustive semantic role
such as heading, code, quote, link or table header; caller-provided render capabilities decide
width, ASCII fallback, color, link display and theme. This is the customization seam: a repository
or client may map stable roles to presentation, but Markdown cannot load code, invent escape
sequences or add executable renderer plugins. Two sinks consume the document:

- the CLI sink performs width-aware line layout and emits styles through `anstyle`;
- the TUI sink maps the same blocks and spans into its own cells and scrolling viewport.

The semantic layer supports paragraphs, six heading levels, emphasis, strong emphasis,
strikethrough, inline code, fenced/indented code with optional language, ordered/unordered/task
lists, nested lists, block quotes, thematic breaks, links, images as text, tables, hard/soft breaks
and footnotes if the selected parser enables them. Unsupported extensions render their visible
text; they never disappear silently. Parser options that rewrite source text, including smart
punctuation, stay off: pretty-printing changes presentation, not the author's characters.

The semantic-member table is executable: every member drives at least one layout assertion, and
each sink has a disposition for every member. This is the same enum-to-driven-audit pattern used
for rules, evidence kinds and MCP tools. It prevents an added construct from compiling while one
sink silently drops it.

Presentation rules:

- headings use weight, spacing and the existing heading role; color is supplementary;
- inline and fenced code use a distinct code role and preserve content exactly after sanitization;
- fenced code shows its language label when present;
- links show readable text and a sanitized destination when it adds information;
- images show alt text and optionally the sanitized URI, and are never fetched;
- raw HTML is displayed as sanitized literal content, never interpreted;
- tables measure display width, align cells and switch to a labelled vertical form when the
  minimum useful columns do not fit;
- list indentation and quote markers remain understandable in ASCII and monochrome modes;
- wrapping never splits a grapheme or computes width from ANSI escape bytes.

All task-controlled text passes through the existing control-character and ANSI sanitizer before
styles are introduced by aplan. No OSC hyperlinks, terminal queries, image protocols, subprocess,
file reads or network requests are allowed from Markdown content. Parser nesting and input size
receive adversarial resource tests.

Syntax highlighting is an enhancement, not permission to fake language support. The baseline uses
semantic code styling plus a visible language label. The spike may admit token highlighting if it
meets the dependency and size budget, has a plain fallback, does not panic on unknown languages,
and can share theme roles with CLI/TUI. Otherwise the documented baseline ships without token
colors and remains a complete Markdown renderer.

### Compatibility and testing

`aplan show ID --body` becomes pretty by default in human format and offers an explicit raw-body
mode. This is a behavior change and receives a `Behaviour` plus a proof that fails when rendering
is reverted. Help, changelog and upgrading guidance explain the boundary. Snapshot/golden tests
cover every block/span kind at wide, normal and narrow widths; Unicode, nested and malformed input;
no-color and redirected output; malicious CSI/OSC/control input; raw HTML; long unbreakable cells;
and exact JSON/raw preservation. CommonMark conformance belongs to the parser dependency, while
aplan tests its event-to-terminal contract.

Long human output gets an explicit pager decision in the surface task. If paging is admitted, it
is a delivery adapter used only for an interactive terminal, honors `PAGER` and a documented
disable path, launches the executable directly without a shell, and never changes redirected,
raw, JSON or artifact bytes. The renderer itself remains I/O-free and testable.

## 3. Quick capture inbox

### Why this is not a task status

`Task::status` answers a work verdict. `todo` participates in readiness, `done` satisfies blockers,
and terminal states carry evidence semantics. A half-formed thought cannot honestly supply title,
cut, effort, paths, dependencies and acceptance criteria, but relaxing those requirements for a
new `draft` or `idea` status would make every rule ask whether the record is a real task. A tag has
the same problem: the object would still be a task in the DAG.

A separate capture entity keeps both sides honest. It is deliberately outside ordinary task
views until promotion, similar to a triage inbox that precedes the normal workflow. Its own state
describes disposition of an intake item, not progress of work. Org Capture supplies the other
useful precedent: the entry path should interrupt the current workflow as little as possible,
while later filing/refiling is a separate deliberate action.

### Model and storage

ADR-0063 fixes the capture boundary. The minimum on-disk record is one versioned YAML document:

```yaml
schema: aplan/capture/v1
id: I-01J...
title: Explore a faster graph layout
kind: idea
state: open
captured_at: 2026-10-11T19:24:08Z
origin: cli
revisit_at: 2026-10-18T13:00:00Z
body: |
  Compare incremental layout after the graph prototype lands.
labels: [graph, performance]
profile: triage
promotion_defaults:
  cut: M-v0.14
  effort_days: 1
  tags: [performance]
source:
  namespace: opencode
  key: session-127:graph-layout
```

`task`, `idea` and `note` are built-in kinds, and `task` is the default for the shortest command.
Repository configuration may add validated lower-case ASCII kebab-case kinds, but kind remains
organizational metadata without lifecycle behavior. Lifecycle state is instead closed to `open`,
`promoted` and `discarded`. Open may promote or discard; discarded may restore to open, clearing its
disposition fields; promoted is irreversible. Both disposition operations record an injected
`disposed_at` and optional reason, and promotion alone requires `promoted_to`. Capture and
disposition instants come from an injected application clock; the ordinary fast path does not
accept backdating. The original title/body are retained and frozen after promotion so the record
shows what was captured rather than becoming a second mutable copy of the task.

Id, `captured_at`, the closed adapter `origin` and optional source pair are immutable. Open records
may amend title, kind, body, labels, revisit time and promotion defaults. Capture labels are sorted,
unique intake keys rather than task tags. Profiles snapshot their name and effective promotion
defaults into the record, so a later configuration edit cannot change what an existing capture
means.

`revisit_at` means "do not surface this as due for review before this instant"; it is a hint, not
an automatic state transition, notification or task due date. Default inbox views must show that
deferred open items exist so snoozing cannot make data disappear. The default view omits not-yet-due
records from its actionable rows but always reports their count; focused show and an explicit
filter expose them.

The optional source pair is a namespaced idempotency key, not arbitrary metadata. It is unique
across open, promoted and discarded records. The same namespace/key and equivalent creation
content returns the existing capture without a write, while conflicting content is refused and
names its id, state and namespace. Equivalence is over the effective persisted fields after the
selected profile and explicit overrides are applied, including origin and snapshotted promotion
defaults, but excluding generated id, clock values and later lifecycle fields. A later profile
change or amendment therefore cannot make different content look like a safe retry. Source keys
are clear-text versioned repository data, not secrets: callers pseudonymize sensitive upstream ids,
ordinary lists and diagnostics omit the opaque key, and only the file and a focused result expose
the full pair.

Captures live as `<plan-root>/.inbox/I-<ULID>.yml`, one record per file, with a separate schema and
loader. The task loader prunes the reserved `.inbox` subtree before recursive Markdown discovery,
so even an accidental Markdown file there cannot become a task. The directory is anchored only
after ordinary root/config selection and moves with the plan; an absent directory is an empty
inbox. No generated index or database competes with the editable versioned files.

The identifier is always `I-` plus a 26-character Crockford-base32 ULID, independent of task id
configuration. The choice uses the repository's measured result that sequential ids gave one
success among 24 concurrent writers because all chose the same next counter. ULIDs remove that
counter, while the guarded inbox snapshot still protects source uniqueness and prefix resolution.
Human commands accept only a prefix that names exactly one record in that snapshot; machine
requests and all results use the full id. A stale or newly ambiguous prefix refuses at commit.

`aplan check` validates capture syntax, ids, timestamps, lifecycle combinations and cross-record
uniqueness, and `verify` runs that structural gate before evidence resolution. Inbox operations
fail closed on any malformed record. Read-only graph commands and roadmap generation remain task
only, so a capture never changes their answer; guarded mutations refuse a post-state that leaves
the full structural gate invalid. Open captures remain exempt from task-only cut, effort, path and
evidence policy by construction.

Repository-configured capture profiles may prefill kind, labels, a body scaffold and promotion
phase, cut, effort, owner, paths or tags. They cannot set identity, origin, source, state,
timestamps, task status, evidence, traces, dependencies or commands. Explicit values win, and the
resulting task still passes the ordinary constructor and whole-plan gate. Repository configuration
owns both the kind vocabulary and named profiles; v1 has no per-user capture configuration or
implicit personal default. Profiles are non-executable defaults, not hidden policy, a workflow
language or a rival to explicit-file blueprints.

### CLI workflow

The fast path stays one command:

```console
aplan capture "Investigate incremental graph layout"
aplan capture "Could plans expose saved views?" --kind idea --label ux
printf '%s' "$DETAILS" | aplan capture "Renderer edge cases" --body-file -
```

The management surface is grouped under the inbox:

```console
aplan inbox
aplan inbox --kind idea --label ux --limit 20
aplan inbox show I-01J...
aplan inbox amend I-01J... --body-file notes.md --kind task
aplan inbox amend I-01J... --edit
aplan inbox amend I-01J... --revisit-at 2026-10-18T13:00:00Z
aplan inbox discard I-01J... --reason "covered by A1-430"
aplan inbox restore I-01J...
aplan inbox promote I-01J... --cut M-v0.14 --effort 1 --paths crates/aplan-output/**
```

Exact flag names remain subject to the CLI design task, but the split is required: `capture` must
remain fast, and review/disposition must be discoverable as one inbox workflow. Input supports a
literal body, a file or stdin without invoking a shell. The human workflow also supports an
explicit `$VISUAL`/`$EDITOR` edit route with safe temporary-file, unchanged-file and failing-editor
semantics; editor arguments are parsed without a shell rather than treating the whole setting as
one executable name. `--` preserves titles beginning with a dash. List/show accept an unambiguous
id prefix when the chosen ids are long, but machine callers always receive and supply the full
identifier and a stale or ambiguous prefix never selects the first sorted match. Search over
title/body is a bounded concrete inbox filter, not a general plan-query language.

Promotion is one guarded transaction. It constructs an ordinary task through the same typed add
and post-state validation path, previews or refuses every warning according to policy, writes the
task and marks the capture promoted only if both replacements succeed, then reports both ids. A
failure leaves both unchanged. The generated task owns its body from that point; later task edits
do not rewrite the historical capture. Repeating an equivalent promotion returns the existing
`promoted_to` result without another write; a conflicting retry is refused and explains why. A
versioned fingerprint of the effective task-creation request makes that comparison independent of
later task edits, while the preview identity binds capture, plan and selected configuration bytes.

Discard is reversible through the explicit restore operation, which clears the disposition fields
without creating an internal history; promoted captures cannot be restored. Listing defaults to
actionable open captures, newest first, with deterministic id ties and a visible count of deferred
open captures. `--state all` exposes retained history. Machine collection results include state,
kind, timestamps, source namespace, completeness and promotion link; only focused results expose an
opaque source key. Promoted and discarded files are retained indefinitely in v1. There is no purge,
delete, archive, automatic expiry or compaction command, and `A1-519` measures large retained
inboxes before a later task can justify one.

### MCP and TUI

MCP receives bounded inbox and focused-capture resources. A capture or promotion action may become
a tool only when the executable `A1-195` policy admits its boundary; list is not a tool, and amend
or discard do not become tools merely to claim CLI parity. Every admitted mutation uses the same
application request and `ChangeSet` gates as the CLI, reports input identity and effects, and has
idempotent retry semantics. The generated agent contract teaches the distinction between a thought
and a task so an agent does not file a capture when it already has enough information to author
valid work, or create weak task files to avoid promotion.

The TUI gets an Inbox destination, global quick-capture action, filters, previewable edits and a
promotion form that reuses the normal task-add `ChangeSet`. Open-count badges are informational,
not a pressure metric. Keyboard-only, narrow-screen, no-color and large-inbox behavior join the
existing release-quality contract.

### Tests and edge cases

Tests cover empty/blank/invisible titles, multiline and control-character input, stdin errors,
unknown kinds/states, malformed YAML, duplicate ids, concurrent capture, deterministic order,
lifecycle and restore invariants, deferred review, source-key retry/conflict, editor failure,
ambiguous id prefixes, profile defaults, promotion fingerprints, interrupted atomic promotion,
task-policy refusal, already promoted captures, labels versus tag families, task-id collisions,
generated schemas, MCP resource/tool policy, agent export and TUI reducer/PTY journeys. A capture
parser must never make a malformed task file vanish or cause the task loader to consume an inbox
record.

## Cross-cutting release plan

The work is one release cut because partial combinations are misleading. A completion field
without a query does not answer the request; a renderer without raw/artifact boundaries can break
automation; capture without promotion becomes a second backlog; CLI-only features make the TUI
release gate immediately stale.

| Order | Task | Outcome |
|---|---|---|
| design | `A1-505` | completion timestamp and provenance ADR |
| completion | `A1-506` | atomic recorded close time |
| completion | `A1-507` | one-pass labelled legacy-history inference |
| platform | `A1-520` | versioned MCP structured outputs and bounded resources |
| completion | `A1-508` | typed engine and CLI completion history |
| completion | `A1-522` | bounded MCP completion-history resource |
| completion | `A1-509` | TUI recent-completion workflow |
| design | `A1-510` | measured Markdown dependency decision |
| rendering | `A1-511` | safe Markdown semantic document and terminal sink |
| rendering | `A1-512` | pretty CLI surfaces and raw/artifact compatibility audit |
| rendering | `A1-523` | TUI inspector Markdown sink |
| design | `A1-513` | capture model, storage, ids and lifecycle ADR |
| inbox | `A1-514` | schema, loader, validation and read model |
| inbox | `A1-515` | quick capture and inbox mutation CLI |
| inbox | `A1-516` | atomic promotion into a valid task |
| inbox | `A1-517` | bounded MCP inbox resources, admitted actions and generated agent contract |
| inbox | `A1-518` | native TUI inbox workflow |
| release | `A1-519` | cross-surface docs, migration, performance and adversarial audit |
| planning audit | `A1-521` | adversarial task, graph and proof-obligation review |

The three lanes can proceed independently after their design decisions. Engine integrations wait
on the shared application boundary, and multi-file capture writes wait on its `ChangeSet` contract;
this prevents the increment from creating temporary rival abstractions that would later become
compatibility baggage. TUI work names the exact existing foundation it extends: overview/browser
for completion history, task inspector for Markdown and task-add mutations for capture promotion.
CLI, MCP and TUI adapters depend on their shared engine result but not on one another merely for
parity, so a transport migration cannot hold an otherwise complete human surface hostage.
`A1-519` waits on all three lanes and on the completed `M-v0.13` cut, so it is the final proof over
the complete release surface without putting every domain task behind that earlier cut. Waiting
on the milestone rather than only its last member preserves the claim that v0.13 itself is done.

## Research basis

Research was refreshed on 2026-10-11 against the repository and primary project documentation:

- [Taskwarrior `done`](https://taskwarrior.org/docs/commands/done/) sets completed status and an
  end date in one operation.
- [Taskwarrior task representation](https://taskwarrior.org/docs/task/) explicitly treats a
  completed record without an end time as completed at an unknown time, which is the safe legacy
  migration model here.
- [Linear Triage](https://linear.app/docs/triage) is a separate inbox in which incoming items are
  reviewed before entering the team's normal workflow. aplan adopts the separation, not Linear's
  service-specific states or automation.
- [Org Capture](https://orgmode.org/guide/Capture.html) optimizes quickly storing tasks and notes
  with little workflow interruption, then finalizes or refiles them deliberately. aplan adopts
  the short capture path, not Org's editor, template or storage model.
- [MCP tools](https://modelcontextprotocol.io/specification/2026-07-28/server/tools) distinguish
  model-controlled actions, support declared output schemas and require `structuredContent` to
  conform when one is declared. They recommend returning structured data alongside text for
  compatibility, which is the migration adopted by `A1-520`.
- [MCP resources](https://modelcontextprotocol.io/specification/2026-07-28/server/resources)
  identify application context by URI and provide resource templates. That is a better semantic
  home for bounded completion and inbox reads than adding presentation rollup tools.
- [The MCP 2026-07-28 release](https://blog.modelcontextprotocol.io/posts/2026-07-28/) removes the
  initialize/session lifecycle and adds result/cache metadata, so supporting the resource model is
  not the same as claiming current-revision wire compatibility. `A1-520` tests both explicitly.
- [Git revision ordering](https://git-scm.com/docs/rev-list-options#_commit_ordering) distinguishes
  date order from topological order, and [Git commit dates](https://git-scm.com/docs/git-commit#_date_formats)
  are author-controlled and second-granularity. They cannot prove a total causal completion order.
- [Command Line Interface Guidelines](https://clig.dev/) recommend honoring `EDITOR` and `PAGER`,
  using `-` for stdin/stdout and designing human and machine use deliberately. aplan keeps editor
  and possible pager launching out of the parser and invokes programs without a shell.
- [`pulldown-cmark`](https://github.com/pulldown-cmark/pulldown-cmark) documents a pure-Rust pull
  parser targeting CommonMark, optional GFM features and a parser/render separation.
- [`termimad`](https://github.com/Canop/termimad) is a capable terminal-oriented Markdown library
  whose rendering, colors and event utilities overlap policy aplan already owns.
- [`pulldown-cmark-mdcat`](https://docs.rs/pulldown-cmark-mdcat/latest/) offers broad terminal and
  Ratatui rendering including images and math, illustrating both the reuse opportunity and the
  dependency/capability excess the spike must measure.

Local package metadata at the same date found `pulldown-cmark` 0.13.4 requiring Rust 1.71.1 and
supporting a no-default-features parser build; `termimad` 0.35.5; `syntect` 5.3.0 with substantial
default grammar/theme/regex features; `comrak` 0.55.0 with broad default CLI/highlighting features;
and the locally resolvable `pulldown-cmark-mdcat` 2.14.0 with image, SVG and sixel defaults. These
are screening observations, not a lockfile decision; `A1-510` repeats them in an isolated build
and records the exact selected versions and costs.

The locked `rmcp` 3.4.1 source already exposes tool output schemas, structured content, resources,
resource templates and paginated list requests. Current upstream documents `2026-07-28` support,
but protocol-version-sensitive result serialization has changed within the 3.x line. `A1-520`
therefore starts as an application and compatibility migration, not an assumed SDK replacement;
it retains the lock when local wire tests pass and measures any upgrade before shipping.
