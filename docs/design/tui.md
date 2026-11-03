# TUI delivery design

Status: feature implementation complete for the accepted scope. The foundation, Overview, task
browser, task inspector and joined ready/why/due workflow, Diagnostics, evidence-verification
workbench, interactive Graph destination, Reports hub, capture Inbox and guarded mutation catalogue are
implemented. The public cross-platform hosted matrix is qualified. The deferred unfamiliar-user
review remains a release gate, not an unimplemented product surface.
The executable scope and order live in the TUI milestones under `aplan/tasks/`; this document
records the product and architecture decisions those tasks implement.

Research snapshot: 2026-10-14. Adversarial plan audits: 2026-10-08, 2026-10-11 and 2026-10-14.

## Outcome

The TUI is an interactive view and control surface over the same plan, graph, diagnostics,
verification and mutation policy as the CLI and MCP server. It does not parse a second model, run
CLI commands as subprocesses or write task files directly.

The initial target is a daily-usable TUI that covers the high-value read and mutation workflows:

1. understand plan health and what is ready;
2. find and inspect any task, blocker, cut or diagnostic;
3. explore dependency shape and critical paths;
4. run checks and evidence verification without freezing the interface;
5. capture and disposition intake without prematurely turning it into planned work;
6. claim, start, stop, add, sequence, classify, close and repair work through previewed mutations;
7. recover the terminal and preserve repository state on every failure path.

Fast means low startup latency, no idle redraw loop, bounded work off the input thread and measured
behavior on large plans. Visually engaging means information hierarchy, typography, spacing,
semantic color and useful compact visualizations, not gratuitous animation. Beautiful means a
coherent system that remains readable in monochrome, ASCII, narrow terminals and reduced-color
environments. Sophisticated means deep workflows and good state handling, not a large framework.

## Product principles

- The plan remains the source of truth. The TUI renders typed application results and submits typed
  mutations to the same gate as the CLI.
- Every screen answers a question. Dense panels are useful only when their hierarchy remains clear.
- Keyboard use is complete. Mouse input is an accelerator, never the only route.
- Current context determines the visible commands and help. Disabled actions say why.
- Long work is visible, cancellable where the provider permits it and never blocks navigation.
- Work queues are bounded. Repeated refresh or verification requests cannot create an invisible
  backlog whose late answers overwrite current state.
- External edits are normal. Refresh preserves selection where identity survives and never applies
  a mutation to a snapshot other than the one previewed.
- Color, symbols and motion never carry meaning alone.
- The UI refuses silent clipping. Small-terminal and oversized-content states explain how to
  recover or narrow the view.
- Plan contents, verifier output and environment values are not persisted in logs by default.
  Restoring the terminal precedes printing a fatal diagnostic.

## Product contract

The primary user already works in a terminal and has a repository containing, or about to
contain, an aplan plan. They may be maintaining it alone, coordinating people and agents, or
reviewing a repository they do not trust. The TUI optimizes the repeated loop - orient, choose,
understand, act, prove - while preserving the CLI for scripts, pipes and artifacts. It is not a
beginner mode with weaker policy and it is not an expert mode that requires memorizing keys.

A session is bound to one explicit plan root, repository and configuration/trust choice. Those
inputs and the running build identity remain inspectable. A view is never called healthy merely
because it is filtered: `check`, `verify` and mutation gates judge the whole plan. Filters may
narrow displayed findings only after the whole verdict has been computed, and the screen labels
that distinction. This is especially important while `A1-494` repairs CLI help that currently
advertises selectors a gate deliberately refuses.

The first public version succeeds when an unfamiliar terminal user can complete the journeys
below without knowing the CLI spelling, while an experienced user can reach every action through
keys and keep context across destinations. It fails the contract if it:

- reports a weaker answer than the equivalent typed CLI service;
- makes a mutation without showing the exact candidate post-state and gate result;
- requires a mouse, color, a wide terminal or a memorized shortcut;
- loses the selected entity merely because the layout collapses or the plan refreshes; or
- hides a CLI-only boundary instead of giving the safe exact command and the reason it stays out.

### Representative journeys

Each journey names its entry, question, route, failure recovery and successful exit. These are
the scripts for the structural prototype review below and become executable journeys in later
milestones.

#### First use: open or repair a plan

Entry: run `aplan tui` in a repository, possibly with `--root`, `--config` or `--no-config`.
The user needs to know whether a plan was found and whether its policy is trusted. A valid plan
opens Overview with the root, repository, configuration source and verdict visible. A missing
plan, missing root, unreadable input and invalid configuration open distinct onboarding states.
The route explains what was searched and offers retry plus the exact non-overwriting `aplan init`
or repair command; it does not write before the mutation foundation exists. Because `init` writes
only `.aplan.yml` into an existing directory, a missing-root state separately explains how to
create the configured task directory and never claims that `init` creates it. Success is a loaded
Overview. Failure remains an actionable onboarding state rather than an empty healthy dashboard.

#### Planning: choose and take work

Entry: Overview. The user asks what can start and why it matters, narrows ready work by cut, phase,
tag and ownership, then opens a task in Tasks. The inspector shows its body, blockers, releases,
cut, critical-path contribution and available actions. If it is not ready, `why` opens the exact
blocking chain without changing the selection; Graph can expand the same task in context. Claim
and start each produce a preview and gate result. Success returns to the same selected task with
its new owner/status visible. A concurrent edit invalidates the preview and offers recomputation,
never a blind retry.

#### Diagnosis: turn a finding into an explanation

Entry: Overview health or Diagnostics. The user runs or refreshes whole-plan checks, filters the
already-computed result by severity, rule, task or source, and opens one finding. The detail keeps
the exact code, message, agent instruction, subject and location; `explain rule CODE` appears as
context, not a disconnected document. Related task and graph routes preserve the finding as a
breadcrumb. Success is a concrete repair or an understood CLI-only boundary. A failed verifier or
configured runner leaves the last complete result visible and names the failure rather than
rendering an empty result.

#### Evidence: decide what can prove a closure

Entry: a task inspector or diagnostic. The user sees existing citations, configured runners,
resolution state and the minimum required strength. Candidate citations are resolved through the
same service as `close`; executable evidence is never run without a separate trust-bearing choice.
An unresolved, ambiguous or insufficient citation stays attached to its explanation. Success is a
reviewable closure candidate whose evidence would pass the current gate, not merely a filled form.
Cancelling returns to the unchanged task.

#### Completion: close, repair or supersede honestly

Entry: the selected task's actions. Close shows the exact evidence, trace stamps, changed files,
post-state findings and denial verdict. Restamp, amend and supersede are different named actions,
because they make different claims; reopening is not invented. Explicit confirmation commits only
the previewed input identity, reloads the plan and shows the actual result. A stale preview is
discarded and recomputed. Success is a terminal task with resolved evidence or a clearly filed
successor; any refusal leaves repository bytes unchanged.

## Research findings and dependency policy

### Rendering and terminal backend

Ratatui 0.30.2 is the baseline. Its crate metadata declares Rust 1.88, matching this workspace,
and its main crate re-exports the selected Crossterm backend and Crossterm types. Ratatui's official
application-pattern documentation describes model/update/view, component and Flux arrangements
rather than prescribing a framework. Its official event-driven template demonstrates that a
synchronous `std::sync::mpsc` event loop plus `crossterm::event::poll` is sufficient.

Start with:

```toml
ratatui = { version = "0.30.2", default-features = false, features = ["crossterm"] }
```

Use `ratatui::crossterm` for the shared API. A1-682 established one Unix-only feature-unification
exception: `aplan-tui` also declares the same Crossterm 0.29 package directly with `use-dev-tty`.
This does not add a second version. It selects Crossterm's level-triggered `/dev/tty` poller because
the default Unix mio source can return a simultaneous `SIGWINCH` before consuming already-ready
terminal input, then strand that byte without another edge. The resolved graph adds the existing
`filedescriptor` package on Unix and is covered by a deterministic stop/queue/resume PTY test.
Ratatui's top-level default feature set also enables calendar support, layout caching, macros and
underline colors. Turning those defaults off avoids the first three; the foundation dependency
spike records the exact underline features selected through the backend rather than claiming Cargo
can subtract a transitive default. Layout caching can be enabled later only if a benchmark shows a
material win.

The implementation begins with a blocking event loop and `std` channels. There is no fixed 30 or
60 FPS loop: input, resize, refresh and worker completion mark the UI dirty; a low-rate tick exists
only while a visible transient state needs it. An active session caps the otherwise blocking input
poll at five seconds to schedule an identified external-change probe, but an unchanged probe does
not dirty or redraw the model. Tokio already exists elsewhere in the workspace but is not part of
the TUI architecture merely because it is available. Crossterm documents that `poll`/`read` must
use one thread and must not be combined with `EventStream`; the event source has one owner and
sends typed events into the update loop. On Unix, the selected source consumes ready terminal input
before reporting resize. Repository observations receive the worker cancellation token, so quit
cannot restore the terminal and then wait for a newly started uncancellable identity probe.

The resolved graph was measured rather than inferred from that top-level list. The optional
`ratatui-crossterm` dependency retains its own defaults, so underline-color support, Crossterm
0.29 and Crossterm's event, bracketed-paste, Windows and derive-helper features remain. That is the
smallest supported Ratatui/Crossterm edge without patching upstream. It is accepted explicitly;
calendar support, Ratatui's layout cache and Ratatui macros are not.

The responsive shell declares `unicode-width` and `unicode-segmentation` directly because cell
fitting and editable palette boundaries call those contracts rather than depending accidentally on
Ratatui's private use of them. Both exact packages were already present in the locked Ratatui graph,
so these direct edges add no package, feature or MSRV cost. Width alone is insufficient: truncating
between a base character and its combining mark can preserve the column count while corrupting the
visible label and cursor boundary. Extended grapheme iteration supplies that missing unit.

### Packaging

The TUI is integrated into the ordinary `aplan` binary behind a default-on Cargo feature named
`tui`. Standard builds, `cargo install aplan` and release artifacts will ship one binary with
`aplan tui`; release automation will name the feature explicitly so a later default change cannot
silently remove it. `aplan-tui` is a library crate and optional dependency of the composing CLI,
not a companion executable. It depends downward on `aplan-app` and the domain/output crates.

An explicit `--no-default-features` source build omits `aplan-tui` and the rendering stack. It
keeps a visible `tui` command stub that explains how to enable the capability, while `facts`
reports whether the capability was compiled. This is a supported headless build, not a release
artifact. It measured byte-identical to the pre-TUI CLI. The standard linked probe added 229,456
bytes (1.79%) to the stripped Linux release binary. A companion pair was larger in aggregate,
would require sibling discovery and subprocess handoff, and would contradict the one-binary
release contract. Noisy cold-build timings are not part of the release contract.

Primary sources:

- [Ratatui 0.30.2 crate documentation](https://docs.rs/ratatui/0.30.2/ratatui/)
- [Ratatui application patterns](https://ratatui.rs/concepts/application-patterns/)
- [Ratatui official templates](https://github.com/ratatui/templates)
- [Crossterm 0.29 event documentation](https://docs.rs/crossterm/0.29.0/crossterm/event/)

### Frameworks and optional widgets

Do not adopt `tuirealm` (the crate published by the tui-realm project), `iocraft`, Cursive, a
hand-built terminal renderer or another application framework at foundation time. They are active,
legitimate projects; the reason is architectural rather than a maturity claim.
`aplan` already has a domain model and needs a small explicit state machine around typed services.
A second component runtime, declarative layout engine or message abstraction would add policy and
dependencies before it removes measured complexity.

The same rule applies to `tui-input`, `tui-textarea`, `tui-tree-widget`, `tachyonfx`, clipboard
crates and fuzzy matchers. A task may admit one after recording:

- the user behavior it enables;
- why Ratatui, the standard library and existing aplan code do not provide it;
- direct and transitive dependencies, license, MSRV, maintenance and binary-size change;
- a focused test or benchmark that would fail without the behavior.

Tokio and Crossterm `EventStream` are also outside the TUI loop even though Tokio already exists
elsewhere in the workspace. Ratatui macros need a maintained example that is materially clearer
than builders; layout caching needs a representative render benchmark; input/search widgets need
editing, selection, IME or ranking tests; and visual effects need a comprehension result, frame
budget, deterministic clock and reduced-motion behavior. Until those proofs exist, each remains an
explicitly declined dependency rather than an unexamined convenience.

The original lane decision has reached its recorded revisit condition. The current
`GraphScene::layout` remains the shipping implementation and the compatibility baseline, but real
use has shown that row lanes cannot provide readable dependency ranks, spatial cards, compound
phase/milestone context or spatial navigation. ADR-0060 therefore selects an aplan-owned typed
cell scene over a replaceable layered geometry adapter. `A1-677` through `A1-683` implemented,
hardened and evaluated it; Layered now owns the text default while the lane surface remains an
explicit compatibility document.

The reusable current work is above and around lane geometry. `GraphWorkspace` owns graph truth and
scope, `GraphWindowProjection` owns a bounded identified subset with exact omissions and
continuations, and raw ids remain the interaction identity. `A1-677` now adds the coordinate-free
`GraphExplorerProjection` between those facts and layout: it presents plan, phase, milestone and
task levels; partitions admitted raw identities into milestone aggregates; retains exact internal
and crossing dependency provenance; distinguishes quotient feedback from invalid task cycles; and
binds reversible semantic transitions to input identity. Phase focus carries exact adjacent bands,
omitted identities and boundary edges. Milestone and task detail reuse the existing bounded task
window. Persisted checkpoint status, member status/readiness counts and presentation selection are
separate values, so no aggregate invents a status or priority claim. Search locates raw ids without
changing scope. The replacement cell scene carries node/group boxes, ranks, ports, routes, bounds,
metrics, semantic hit regions and typed degradation. Both the Ratatui widget and static CLI painter
consume that scene without parsing glyphs or re-deriving graph policy. The lane scene remains a
separate adapter; its row coordinates, connector cells, scene-order navigation and width refusal
are not promoted into the new boundary.

The measured adapter candidate is exact `graphlay` 0.3.1 with default features disabled. It met
Rust 1.88 and deterministic bounded-fixture checks and materially reduced crossings against the
small in-house ranked prototype. Aplan still owns stable semantic keys, terminal-column card sizes,
internal-scale conversion, cell quantization, scene invariants, bounds, work limits, stale-result
handling and fallback. A1-678 now admits that dependency behind one private `aplan-app` adapter;
no kernel id, coordinate, error, option or route type reaches `CellScene`. The locked normal feature
graph has graphlay's renderer, JSON, schema, fixture and parallel features off. Its only new package
is MIT-licensed `libm`; its unconditional derive edge reuses the workspace's existing `serde`.
Graphlay is MIT OR Apache-2.0 and declares the same Rust 1.88 floor as this workspace. The final
whole-product license, source, advisory, duplicate-version and binary-size policy remains a
separate executable gate. Exact methods and evaluation results are in
`docs/design/graph-layout-evaluation.md`.

`CellSceneJob` owns its immutable projection, sizes, ordering hints, orientation, prior scene and
budget, so the TUI can schedule it on the bounded worker rather than call layout from update or
paint. Completion carries the requested generation and is refused when a newer generation owns the
screen. The scene pipeline is fixed and deterministic:

1. refuse non-layoutable source state and any input above 64 nodes or 96 edges before calling the
   coordinate kernel, which independently refuses more than 1,536 routing waypoints as soon as
   ranking makes that derived count knowable; fail closed before layout when node identities,
   sized-node identity, ordering subjects or phase membership disagree;
2. assign canonical semantic keys and phase groups, apply an explicit sort key only inside a
   same-group/same-rank peer slot, and use a prior cell centre only as a seed;
3. let the layered kernel dispose cycles for ranking, assign ranks, insert long-edge waypoints,
   minimize crossings, compact coordinates and route orthogonal ports under one fixed effort
   budget; allow any valid node-boundary port because the matched A1-688 gate found fewer bends
   without exceeding Lanes' crossing count;
4. normalize at eight internal units per display cell and perform one checked integer conversion;
5. validate node separation, route monotonicity, orthogonality, ports, bounds and unrelated-node
   clearance, removing unsafe route geometry while retaining its exact typed omission; and
6. derive phase boundaries, focus, crossings, shared segments, junctions, parallel groups,
   displacement and the quality report from semantic identities and validated cells.

The scene never infers causality from phase order or containment. Strict phase lanes occupy the
cross-flow axis; dependency rank occupies the flow axis, and exact task dependencies may cross a
boundary either way. Each non-empty boundary carries a stable `Phase N` label anchor and encloses
its cards. Aggregate quotient feedback is explicitly reversed for ranking and stays distinct from
an edge inside a canonical task-cycle component. Any other reversal remains a typed kernel cycle
disposition rather than silently changing the arrow. Parallel edges receive stable ordinals and
all crossing, shared-segment and junction records point back to exact edge identities and
provenance.

Card measurement wraps extended grapheme clusters by terminal display columns before geometry is
requested. The default Readability policy lets the crossing reducer choose placement with identity
as the total tie-break; explicit identity, title and status sorts supply total peer keys but cannot
change dependency rank. A scene reports that active policy, crossings, bends, area and a quality
score. Auto lays out both supported directions and compares, in order of cost, crossings and node
hits, degraded edges, bends, prior displacement and area. Ties prefer left-to-right. A prior
direction within 12 percent of the best score is retained to prevent resize/refresh oscillation;
an actual direction change retains the semantic selection and marks it for recentering.

The structural geometry key excludes status and input-snapshot churn. A status-only refresh
therefore reuses the prior cells exactly. A structural, size, orientation or ordering change runs a
seeded layout, measures Manhattan displacement and reports typed causes without promising that a
seed is a constraint. A failure or over-budget answer carries one omission per admitted node and
edge. It may retain the last valid scene only as input to a later recovery attempt; failed current
geometry is never painted or hit-tested. The existing `GraphScene` lane adapter remains compiled
and byte-compatible behind explicit `--layout lanes`; A1-678 by itself changed no CLI text or live
TUI painter.

The scale harness covers three approximately 10,000-node shapes: an eight-lane sparse graph with
9,992 edges, an eight-way repeated fan-in graph with 17,768 edges, and a crossing-heavy graph with
13,997 edges. It asserts every edge has geometry, then separately measures construction, 1,000
40-by-40 viewport queries and about 10,000 node hit tests. The 2026-10-15 debug baselines on a
12-core/24-thread Ryzen 9 3900X with Rust 1.99, reported as the median of three warm runs, are:

| shape | construction | 1,000 viewports | all node hits |
|---|---:|---:|---:|
| sparse | 756 ms | 4.6 ms | 9.6 ms |
| fan-in | 791 ms | 4.7 ms | 12.5 ms |
| crossing-heavy | 605 ms | 2.3 ms | 9.5 ms |

These are comparison anchors, not cross-machine budgets; `A1-425` measures the release-profile and
end-to-end foundation before ratcheting a budget. They remain the lane compatibility baseline, not
permission to lay out a complete 10,000-node source with the layered adapter. Live layered layout
operates on the semantic overview or current bounded task window.

### Live layered painter

`A1-679` replaces the Graph destination's Paragraph of lane rows with a custom Ratatui
`StatefulWidget`. The painter receives the immutable `CellScene` and coordinate-free explorer
projection; it does not parse text or infer an identity from a glyph. One transform maps scene
cells to the clipped terminal viewport and is shared by paint and hit testing. Phase headers,
cards, routes, ports, arrows and scrollbars therefore remain pointer-addressable when labels are
clipped. Grapheme-safe cell slicing replaces byte slicing at the viewport boundary.

Paint order carries meaning. Phase bands establish grouping first. Every admitted edge is then
painted beneath cards with distinct feedback and bundled forms, visible crossings/junctions and
continuation arrows where a route crosses the viewport. Cards restore their borders above routes;
source ports and target arrows are restored last so a connection is not erased by the card.
Selection uses a stronger border and marker, direct relationships use the accent style, and other
structure uses the normal or muted style. None of those meanings depends on color. Relations Only
suppresses unrelated routes plus crossing/junction marks derived from them, dims unrelated cards,
and leaves semantic scope, accounting, selection and geometry unchanged. Returning to All Edges
reproduces the original bytes.

The selected card is one continuous visual surface, not a collection of styled strings. Before
the border and semantic labels are restored, the painter fills every viewport-visible interior
cell with the selected style. That includes unused suffix cells after a short id, title or
aggregate progress value and any intentionally blank row. Clipping uses the same scene transform,
so the fill neither escapes the card nor changes its hit region. Dark and light themes get a
coherent background; low-color and monochrome keep the equivalent reverse-video surface.

Compact cards keep identity and status. Comfortable cards add a display-column-measured title and,
where meaningful, exact aggregate completion/readiness counts. Auto changes from Comfortable to
Compact at 88 columns or 16 rows and back only at 104 columns and 20 rows, so resize inside the
transition band retains its prior decision. Compact and Comfortable can be pinned for the running
session. A density change is an explicit geometry-policy change and gets a fresh deterministic
layout; this prevents expanded spacing from contaminating a later compact scene. Selection-only
changes retain exact boxes and routes. Detail stays in the synchronized inspector instead of
resizing the selected card.

The first three graph chrome rows prioritize selected identity, semantic phase/rank, exact
visible/source node and edge accounting, viewport row/column extents, directional off-screen
counts and available actions. Narrow views keep the same truth and overlay exact adjacent-phase
markers on the graph edge rather than squeezing all phase bands into columns. The inspector names
why an anchor was chosen and explicitly says that the choice is presentation only, not persisted
plan priority or evidence. A failed or over-budget cell scene renders the complete admitted nodes
as a scrollable/selectable list with the same accounting; it never presents a clipped picture as a
complete graph and never returns to the old width-refusal paragraph.

Ordinary paint scans only the already bounded scene records and expands cell work only for
viewport-intersecting rectangles and orthogonal segments. Fallback caches its exact horizontal
extent when the semantic projection changes and formats only the visible node slice. Pan and
Relations Only do no layout or semantic graph traversal. Plan/phase selection updates the current
selection and transition subjects without repeating complete-source projection; bounded detail
still reprojects around an anchor when membership can change. Resize and refresh rebuild only the
visible source-sized destination, and route entry reconciles any deferred destination from the
adopted snapshot. Fixed-buffer tests cover full and clipped pictures, styles,
ports, semantic hit regions, wide and combining graphemes, every built-in theme and glyph set,
reduced motion, density hysteresis and the fallback path.

Ratatui Canvas was evaluated and rejected for this release. Its sub-cell markers could make a
free-form curve smoother, but this scene deliberately owns integer orthogonal routes, exact
display-cell labels and cell-accurate pointer identities. Mixing coordinate systems would weaken
those contracts and would not add dependency information. The standard `Buffer`, `Block`, text,
style and `Scrollbar` APIs are sufficient and add no dependency. The earlier minimap decision also
stands: exact extents and directional counts provide the location answer without permanently
removing useful card cells.

### Static layered CLI adapter

`A1-681` exposes the same semantic projection and validated `CellScene` outside the alternate
screen. It is deliberately an adapter, not another graph implementation. The application boundary
retains each node's semantic identity, represented raw tasks, dependency rank and peer order; each
phase band's typed members; each edge's endpoints, ordinal and canonical provenance; and the
projection's admitted, visible, level-omitted, window-omitted and boundary accounting. Adapter
drift returns an invariant error. Parity tests compare these typed records to the source scene and
projection rather than comparing a Ratatui buffer to a text picture.

An attended bare `aplan graph` chooses text; a redirected bare command remains Mermaid so existing
scripts and document generation do not change. Explicit `--format` always wins. Within text,
explicit `--layout layered` is equivalent to the omitted layout, while `--layout lanes` selects
the compatibility row document. `--level` selects `plan`, `phase:N`,
`milestone:ID` or `task:ID`; selectors, reach and membership remain structural scope;
`--focus` changes emphasis only. Auto/left-to-right/top-to-bottom orientation, readability and
five explicit peer orders, compact/comfortable cards, exact width, ASCII/Unicode and global color
are separate choices. The shared card and ordering policies keep the TUI and CLI from acquiring
similar but unequal copies. Horizontal phase labels stay on the top border in both orientations;
the audit found that a left-border anchor in top-to-bottom mode overwrote the border and first
card.

Plan and phase levels apply a source-bound semantic window before layout: at most 10 aggregate
cards and 20 visible aggregate edges, or any tighter caller budget. The window anchors on the
contextual, remembered or work-aware subject and exposes exact raw-task/dependency omissions plus
continuations. Milestone/task detail keeps the general graph window. This distinction lets
aplan's 68-card overview open as a real layered TUI scene and a bounded static document without
raising the shared 64-node/96-edge cell-scene ceiling or pretending a partial source is complete.

A redirected layered document must name `--width`, is unstyled and is independent of terminal
state. An attended terminal may use its actual stdout width; `--width` always overrides it for
review, snapshots and agents. The painter draws every node and route admitted to the semantic
window or returns a width/resource refusal with exact omitted identities and narrowing/export
commands. It never labels a window or clipped viewport as the complete source graph. Static notes
retain aggregate-internal dependency
counts, phase boundary provenance, window continuations, typed feedback/degradation and critical
route availability. A1-683 passed the frozen text-layout default gate and retained Lanes as an
explicit byte-compatible surface; A1-688 keeps the matched crossing/bend comparison executable.
A1-696 makes that text view the attended entry while retaining redirected Mermaid, and repairs a
route which shares an unrelated card border after terminal-cell quantization instead of discarding
the entire current scene. The repair preserves the original ports and monotone direction;
unrepairable geometry still uses the typed fallback.

### Layered graph exploration

`A1-680` makes the layered picture an explorer without adding a second graph model. Selection is a
raw `GraphExplorerIdentity`; movement reads `CellScene` ranks, centers, direction and semantic edge
identities. Along-flow arrows choose the nearest visible predecessor or successor. Cross-flow
arrows prefer a sibling in the same dependency rank, then the nearest node in that half-plane. A
geometry fallback remains a complete selectable list: up/down traverse its explicit list and
left/right follow typed relationships, so degradation never revives lane rows or parses glyphs.

`[` and `]` retain the original semantic subject while it has several direct relations and cycle
them in stable identity order with an exact `n/total` cue. A subject with exactly one relation
advances to that neighbour, so repeated keys can still follow a chain. Boundary relations in a
phase projection open the exact remote phase before selecting the raw subject. Separate jumps
visit the nearest previous/next dependency rank, cycle source, sink, critical, milestone or
diagnostic-bearing nodes, and open the exact previous/next ordered phase. Phase order is never
treated as a causal edge.

The hierarchy is Overview, Phase, Milestone where present, Task graph, then the synchronized task
inspector. Enter drills and Escape reverses. A bounded in-memory history retains semantic location,
raw selection, source-bound continuation, viewport origin and follow/manual mode; no navigation
state is persisted. Overview is always an explicit action. The first direct Graph visit starts at
Overview and explains its deterministic work-aware anchor: in-progress first, then derived ready
rank, then phase/id. A later direct visit restores the valid session. "Show in Graph" from another
destination is a temporary contextual route: it opens the exact raw task or milestone, uses a
visible origin breadcrumb, widens an excluding saved graph to a temporary whole/default-selector
view when necessary, and restores both the originating selection and prior Graph session.

Panning, Home focus, compact-card fit, hierarchy level and structural continuation are deliberately
different commands. Chrome reports exact breadcrumb, complete/windowed state, visible/admitted
node and edge accounting, viewport ranges, directional off-screen counts, active structural scope,
edge presentation, order, direction and layout movement. Search does not narrow scope; selecting an
off-view match opens typed task detail, and bounded detail pages retain their source-bound cursor.
Relations Only remains a paint filter: selection, direct relation counts, accounting, continuation
and structural scope are byte-for-byte the same when unrelated routes are hidden.

Readability is the default peer policy and lets the layout kernel minimize crossings. Explicit
plan-priority, id, title, status/readiness and criticality policies supply total stable hints only
inside a phase/rank peer slot; none can invent dependency order. The UI labels readability as
crossing-minimized and warns that explicit peer sorts may add crossings. Auto direction keeps its
12 percent hysteresis; left-to-right and top-to-bottom are session pins. Compact and Comfortable
density are independent pins, while Fit explicitly chooses Compact and the scene origin. Refresh
and resize retain selection, semantic level, scope, order, direction, density and prior geometry
where valid. The scene reports fresh, exactly reused, or relaid-out state with moved-node count,
Manhattan distance and structural/card-size/direction/order/input causes, so motion is inspectable
rather than random.

The shared command catalogue drives keys, contextual help, the palette and every painted pointer
accelerator. Reducer tests prove geometry rather than row-order movement, relation cycling,
hierarchy/history, temporary contextual scope and resize pins. Fixed buffers cover full, narrow,
ASCII, Unicode, monochrome and fallback presentation. The real PTY journey drills
phase-to-milestone-to-task, follows an off-screen/cross-phase relation, changes order and direction,
moves between exact phases, enters from Tasks, returns to the saved session, resizes and verifies
terminal restoration. The adversarial corpus, release measurements and frozen human protocol are
recorded in [`graph-layout-evaluation.md`](graph-layout-evaluation.md) and
[`tui-evaluation.md`](tui-evaluation.md).

### Test dependencies

Ratatui's official testing recipe recommends fixed-size `TestBackend` snapshots and a small number
of PTY journeys because a buffer cannot cover the event loop, key handling, terminal setup,
teardown or exit codes. `A1-424` keeps the buffer fixtures as ordinary checked-in ASCII text rather
than admitting `insta`: a normal repository diff is the approval mechanism, and each picture is
paired with assertions over focus, route, selection, labels and selected-cell style.

`termlens 0.11.6` is admitted only as a development dependency, with default features disabled so
its optional `insta` edge stays absent. Its core grid, style, cursor, mode, resize, typed-input,
focus, `wait_until`, `wait_stable` and `snapshot_after` surface is tested upstream on Linux, macOS
and Windows ConPTY. Unix process signals are intentionally not part of the Windows surface, and
aplan's portable journey uses content predicates rather than synchronized-frame waits. Every wait
has a twenty-second contention ceiling; there is no correctness sleep. The journey is an explicit
module in the crate's one consolidated integration binary; the binary gates its historical
Unix-only fixture modules off Windows, and the portability job selects only this module. It launches
the immutable package binary with a cleared environment and proves focus delivery, resize bursts, minimum-size refusal/recovery,
ordinary and interrupt exit, external-editor release/reacquisition, input-identity refresh,
alternate-screen exit, bracketed-paste disable and cursor restoration. Raw-mode, Unix job control
and every partial-acquisition failure remain covered by the Linux PTY and in-process mode-control
fault tests because a screen emulator cannot observe the child PTY's termios flags after exit. No
testing convenience enters the runtime graph.

- [Ratatui snapshot and PTY testing recipe](https://ratatui.rs/recipes/testing/snapshots/)
- [insta](https://github.com/mitsuhiko/insta)
- [termlens](https://github.com/vyncint/termlens)

### Patterns borrowed, not copied whole

GitUI demonstrates contextual command enumeration, a quick key bar and explicit focus/event
forwarding. bottom demonstrates dense responsive panels, focused expansion, search and themeable
visual hierarchy. Both are actively maintained and prove that Ratatui can support the intended
quality. `aplan` should borrow those interaction patterns, not bottom's continuous data-collector
loop or GitUI's repository-specific component graph.

- [GitUI source](https://github.com/gitui-org/gitui)
- [bottom source](https://github.com/ClementTsang/bottom)
- [Ratatui Labs command-palette design](https://github.com/ratatui/ratatui-labs/blob/main/docs/prds/command-palette.md)

## Architecture

### Dependency direction

`aplan` currently composes the CLI while some orchestration is duplicated by MCP. Making
`aplan-tui` depend on `aplan` while `aplan` depends on `aplan-tui` would create a cycle. Foundation
work therefore proves one typed query boundary before choosing between:

1. a small application-service crate shared by CLI, MCP and TUI; or
2. an explicit host interface implemented by `aplan`, if that stays typed and does not duplicate
   setup or policy.

The extraction demonstrated value with `check` and dependency editing, so the result is the small
`aplan-app` crate rather than a host interface implemented by the composing CLI. Both `aplan` and
`aplan-mcp` depend downward on it, and `aplan-tui` can do the same without a dependency cycle.
`PlanContext` owns one identified `Plan`, resolved validation and verdict policy, repository
identity, selected configuration location and an explicit trust mode. The identified loader reads
each Markdown candidate once and retains those exact bytes, including files which are not yet
tasks; the working-tree digest combines them with the resolved policy inputs. It never reopens a
task or configuration file after parsing and labels an old answer with newer bytes. An asynchronous
client can therefore identify what an answer describes without calling the CLI or inventing
another plan model. Read operations answer from that immutable load. A mutation preview now retains
the complete input identity, exact before/after file content, semantic effects, findings, denial
verdict and evidence resolution state in one immutable `ChangeSet` (`A1-436`). Commit consumes that
reviewed value, rechecks the input identity and publishes the same bytes through the guarded batch
writer. A stale preview is a typed non-publication, as are denied and no-op previews; a failed
publication reports whether the observed files are unchanged, complete, partial or diverged.

The two deliberately concrete operations establish the pattern rather than a generic command
framework: `CheckRequest` returns ordered diagnostics, the complete denial verdict and unresolved
evidence count; `DependencyRequest` is the first mutation with separate preview and commit
operations. Its existing immediate edit wrapper, used by CLI and MCP, invokes that same preview,
policy and commit implementation. A direct application client inspects the typed file delta before
commit and proves the writer publishes those exact bytes. Transport rendering, exit codes and retry
wording remain in their adapters.

The boundary continues to consume existing `Plan`, `Graph`, `Ready`, `Why`, `GraphView`, task rows
and diagnostics rather than define parallel projections. New operations are admitted one concrete
request/response pair at a time when another caller needs them.

### State and events

The intended shape is immediate-mode model/update/view:

- `Model`: current identified plan snapshot, route/focus, selection, filters, modal state, pending
  effects and last completed results;
- `Event`: key, mouse, paste, resize, refresh request, worker completion and external-change notice;
- `Action`: semantic intent such as select-next, open-task, run-check or preview-close;
- `Effect`: repository/config reads, verification and mutation previews or commits;
- `View`: a deterministic render from model and terminal area.

Key decoding is separate from actions, so tests drive semantic actions and keymaps can evolve
without rewriting behavior. Effects carry a generation or input digest; a result from an older
plan snapshot is marked stale and cannot overwrite current state. Rendering has no repository or
process side effects.

### Evidence verification workbench

`A1-431` makes verification a typed application operation rather than a CLI subprocess. Before a
run, the sheet names the repository and selected configuration, the effective effort, trace,
warning and drift policy, configured test, URL and version providers, the resulting repository-read
or configured-process trust boundary, and an input digest that includes resolver configuration.
The context and provider options come from one configuration read, so a concurrent edit cannot
silently combine check policy from one revision with commands from another. The `execute` choice
is deliberately outside that digest: it is a trust-bearing request made for one run, not a hidden
change to the inputs being identified.
Resolve-only is not side-effect-free - it may run `test.list` and configured URL or version
commands - so the sheet says so before any is scheduled. The completed workbench carries this
verification digest and actual trust boundary as its own identity; it does not inherit the
hermetic label of the plan-loading step that preceded it.

The digest is also a pre-execution capability boundary, not merely a stale-result check. The
worker reloads plan and configuration, recomputes the sheet, and refuses a mismatch before it
enters the evidence engine. Consequently a configuration edit made after review cannot replace a
listed command and have that command run on the theory that its eventual answer will be discarded.
The typed application operation has no independent repository argument: evidence resolution uses
the repository root carried by the identified context, so one repository cannot be displayed and
hashed while another is opened.

The same evidence resolvers that produce verification diagnostics also produce a per-citation
ledger. Each row carries task, kind, reference, actual resolver, strongest evidence rung reached,
terminal outcome and optional detail. Outcomes distinguish resolved, failed, refused, timed out,
unavailable and output-limited work. Configured-command measurements remain provider-level costs:
a memoised listing or URL check may serve several citations, so copying its cost onto every row
would falsely multiply what the run spent. Every measured provider cost is independently
inspectable, including a provider that failed before resolving any citation, and a citation links
to its provider where that attribution is exact. Exact machine-applicable evidence replacements
are projected beside their source citation. `A1-441` supplies the shared previewable mutation path
that stages all replacements as one guarded batch; shell composition still keeps the read-only
result and its separately confirmed write action distinct.

The result retains the complete structural and evidence finding set as well as the ledger. A
denied count is therefore always inspectable down to code, subject, location, message, instruction
and detail; a run with no citations does not lose the whole-plan diagnostics that denied it.

Verification uses the existing one-running/one-queued effect worker. Browsing and help remain on
the input thread. An equivalent repeat request coalesces; a different execute choice is visibly
refused while a verification is already pending rather than silently inheriting either choice. A
queued request can be cancelled before it starts; a running configured process is not
detached, and quit restores terminal state before joining it. The interface reports activity only
because the verifier exposes no honest incremental denominator. Completion is accepted only when
both generation and the verification-input digest still match; otherwise it remains a visible
stale result and cannot replace the last complete answer. Long run-sheet fields, evidence details,
repairs and costs wrap and scroll rather than disappear at the minimum supported terminal size.

### Terminal lifecycle

Terminal state has two non-overlapping owners. The outer session guard owns raw mode, alternate
screen, focus-change reporting, mouse capture and bracketed paste; Ratatui owns the cursor it hides
for rendering and may later show and position for focused fields. Normal exit, error, panic and
supported signals restore the terminal state each owner acquired. On Unix, `SIGINT`, `SIGTERM`,
`SIGHUP` and `SIGQUIT` are
queued by a signal-safe handler and consumed by the terminal's input thread. That thread restores
the renderer cursor and every acquired mode, then re-raises the same signal so the parent retains
the real exit reason. Before re-raising, the configured-command seam refuses new spawns and kills
every registered process group (and its cgroup, where one was acquired). It does not join a running
effect first: waiting on an uncooperative configured provider would preserve the terminal
corruption this boundary exists to prevent, while exiting without the command registry would
orphan that provider.

The lifecycle remains active after a graceful `q` has restored the terminal and while the process
waits for a running worker to finish. Catchable termination during that interval takes the same
provider-kill and signal re-raise path; it is not queued behind a blocking join or discarded with
the signal iterator. Suspend stops with the terminal already restored, and continue resumes the
shutdown wait without reacquiring modes for an interface that has already closed.

Unix `SIGTSTP` follows the same cleanup path before the process stops. `SIGCONT` then reacquires all
five modes, rebuilds the terminal size and repaints the retained model. Suspension also crosses the
external-observation boundary before terminal ownership is released: it advances the focus epoch
and labels retained bytes unconfirmed. Resume requests a current-epoch probe immediately. If a
deliberate operation still owns the bounded worker, the deadline remains due and starts that probe
as soon as capacity returns rather than certifying the retained snapshot or waiting five seconds.
The signal iterator
coalesces a burst and does not retain delivery order, so a batch containing both stop and continue
is treated as continued; consuming the continue and then stopping would strand the process until a
second external continue. The same arbitration runs across the screen-to-stop handoff: a continue
observed after screen teardown cancels the pending stop decision before the default handler can
re-raise `SIGTSTP`. That cancelled handoff still enters the observation boundary before the next
screen session. More generally, every continue which resumes interactive work first marks retained
bytes unconfirmed and requests a current-epoch observation. This includes a standalone `SIGCONT`,
which may be the first event the process can observe after uncatchable `SIGSTOP`; an already
inactive model does not advance its epoch a second time. A terminating signal or separately
observed suspend remains dominant. The handler does no terminal I/O, allocation or model work, and
an idle event loop polls the queued boundary at most 100 ms later.

Resize and input readiness have their own proved ordering boundary. The default edge-triggered
Unix source could report `SIGWINCH` while leaving a simultaneously queued key unread, after which
no second edge woke the loop. The selected level-triggered `/dev/tty` source consumes input first.
The deterministic PTY regression stops the process, queues resize and quit, resumes it and requires
clean exit; repeated longest-journey runs cover the ordinary schedule. Quit also cancels an
in-flight repository observation before worker join, so terminal restoration never waits for the
full observation timeout.

`SIGKILL` and `SIGSTOP` cannot be caught, so no child can restore terminal state after either one.
After `SIGSTOP`, the subsequent catchable continue can and does repair the repository-freshness
claim before the resumed interface certifies retained state; that is distinct from repairing modes
before the stop, which remains impossible.
The shell recovery for an unavoidable hard kill is `reset` (or `stty sane` when only termios is
damaged), not a launcher that changes every ordinary command. Non-Unix builds retain ordinary
guarded cleanup but do not claim Unix job-control behavior. Non-TTY invocation is refused before
acquiring modes. Plans cannot inject terminal controls: existing sanitization remains in the path.
Fatal errors restore modes before emitting diagnostics, and no persistent log captures plan text,
the private editor draft, configured-command output or inherited environment values.

Background effect panics cross the same boundary without writing through it. One process hook
delegates every ordinary panic to the hook the host installed first, but recognizes the
thread-local effect-worker marker and lets `catch_unwind` turn only that panic into typed model
state. The hook is installed once rather than swapped around each execution, so concurrent
non-worker panics cannot encounter a temporary silent process hook.

### Platform support and evidence

Support is a three-part statement, not one check mark. Compile coverage proves the source and its
platform branches build. Automated PTY coverage proves the application and one terminal transport
agree on observable modes and input. Manual smoke evidence exercises a real emulator, multiplexer
or remote transport and records what automation cannot identify. An unavailable column never
inherits another column's pass.

| platform | compile and unit coverage | automated real PTY | Unix signals and job control | latest observation |
|---|---|---|---|---|
| Linux x86_64 | ordinary workspace suite plus `aplan-tui` library | four portable cases plus the full `tui_journey` corpus | supported and automated | public hosted matrix passed 2026-11-03; complete Unix suite also passed in the local full profile |
| macOS arm64 | public `macos-latest` portability job | four portable cases, including attended CLI interruption | supported by the Unix implementation; Linux owns the complete signal matrix | public hosted matrix passed 2026-11-03 |
| Windows x86_64 | public `windows-latest` portability job | three applicable cases through ConPTY | unavailable: Windows has no `SIGTSTP`/`SIGCONT` contract | public hosted matrix passed 2026-11-03; no claim is made for a console-control event the harness cannot generate |

The weekly/manual `TUI portability` workflow is the executable matrix. Public runs use standard
GitHub-hosted `ubuntu-latest`, `macos-latest` and `windows-latest` systems. ADR-0059 records why this
is a narrow exception to the otherwise Ubuntu-only public CI decision. The 2026-11-03 qualification
ran the shared library suite and every platform-applicable real-PTY case on all three systems.

The lifecycle cases and their load-bearing proofs are:

| boundary | automated proof |
|---|---|
| focus, resize burst, 48x14 floor, below-floor refusal, recovery, ordinary exit | `shared_real_pty_lifecycle_handles_focus_resize_and_tiny_terminals` |
| portable interrupt and mode restoration | `portable_control_c_restores_terminal_modes` |
| Unix INT, TERM, HUP and QUIT with original exit reason | `tui_journey::terminating_signals_restore_the_terminal_and_keep_the_exit_reason` |
| Unix suspend, exact termios baseline, resume and repaint | `tui_journey::suspend_restores_exact_termios_and_resume_repaints` |
| blocked startup cancellation after terminal restoration | `tui_journey::blocked_startup_quit_restores_the_terminal_before_joining` |
| partial acquisition, draw, input, cursor cleanup and panic fault injection | the `ModeGuard` and `with_modes` fault matrix in `aplan-tui` |
| CLI progress completion, failure and terminal-control allowlist | the `progress` and `terminal_integrity` integration modules |
| attended CLI progress interruption and row cleanup | `portable_cli_progress_interrupt_cleans_its_terminal_row` |

Closing a PTY delivers HUP on Unix and is covered by the terminating-signal row. Windows ConPTY
does not expose a Unix EOF signal; ordinary quit and Ctrl-C exercise its supported exit paths.
`SIGKILL`, `SIGSTOP` before its later continue, power loss and emulator crashes are explicitly
outside any in-process restoration promise.

### External editor handoff

The Add form exposes `External editor (Ctrl-E)` for a long Markdown body. The handoff has one
ownership order:

1. invalidate repository freshness and leave the renderer;
2. show the cursor and disable bracketed paste, mouse/focus reporting, alternate screen and raw
   mode in the ordinary reverse cleanup order;
3. parse `VISUAL`, then `EDITOR`, as arguments and launch the executable directly, never through a
   shell; fall back to `vi` on Unix and `notepad.exe` on Windows;
4. append one private, uniquely created draft as the final argument, bound reads to 4 MiB, verify
   that the opened regular file still owns the name, and remove it on success or failure;
5. accept successful UTF-8 bytes into the retained Add form, where they visibly override the short
   Body field but still pass through ordinary preview, confirmation and compare-and-swap publish;
6. request a current input observation before the reacquired interface can call the repository
   snapshot current.

Configuration is an argv string, not shell source. Complex setup belongs in an explicit wrapper
script named by `VISUAL`; pipes, redirections and substitutions are never interpreted by aplan.
The configured process must remain alive until editing is complete (use the editor's ordinary
`--wait` option where required), because its successful exit is the hand-back boundary.
An editor spawn, exit, replacement, encoding or size failure returns to the form with a visible
error and no accepted body. On Unix, catchable termination stops the active direct child before
preserving the parent's signal exit; Windows console termination has no equivalent TUI lifecycle
event, but terminal modes have already been restored before the editor starts. The PTY proof holds
the editor open, observes every mode restored, edits
the plan during that interval, then proves both the draft note and refreshed task identity after
reacquisition. No editor bytes or environment values enter a persistent log.

### Manual transport and emulator smoke protocol

Manual evidence records date, aplan source and binary digests, OS/architecture, emulator and
version, `$TERM`, local/SSH/tmux layers, glyph/theme choice and pass/fail notes. Run the exact
`cargo test -p aplan --test integration tui_portability::` selection first, then in each environment:

1. launch `aplan tui` at 100x24, navigate all six destinations and open/close help;
2. resize through 80x20, 49x14, 47x13, 64x16 and 100x24, checking the explicit too-small screen
   and preserved route/selection on recovery;
3. paste into a focused field, focus away/back and confirm no input or cursor leaks to the shell;
4. open Add, use Ctrl-E, edit the body, return, review without publishing and cancel;
5. on Unix press Ctrl-Z, change one task from another process, run `fg`, and confirm the retained
   frame is unconfirmed until refresh completes;
6. exit once with `q` and once with Ctrl-C, then verify cursor, echo, bracketed paste and shell
   history remain normal without `reset`.

Run the protocol in a direct local session, inside current tmux, and through an allocated TTY such
as `ssh -t host`. The named emulator coverage set is Kitty and GNOME Terminal on Linux, iTerm2 on
macOS, and Windows Terminal on Windows. At least two real emulators plus tmux and SSH must have a
current recorded pass at release audit. Hosted PTY coverage does not identify a GUI emulator or an
SSH transport, so those manual observations remain `not observed`, not `pass`.

The A1-448 development observation record is deliberately narrower than that release protocol:

| environment | observation on 2026-10-27 |
|---|---|
| Linux PTY through termlens | passed the complete automated `tui_portability::` module locally and on the public hosted matrix |
| tmux 3.7c on Linux, detached 100x24 session | passed launch, Overview/Tasks navigation, 47x13 refusal, 100x24 recovery, ordinary quit and restored shell modes; the remaining manual-protocol steps are not claimed |
| SSH allocated TTY | not observed: the local SSH service accepted connections but this host had no usable non-interactive account authentication |
| Kitty, GNOME Terminal, iTerm2 and Windows Terminal | not observed on this non-GUI Linux host |

### Foundation audit and baselines

`A1-425` audited the implemented foundation on 2026-10-15 before feature panels could normalize a
wrong boundary. The executable path is one directional pipeline:

| step | owner | state or side effect |
|---|---|---|
| terminal refusal | CLI adapter | checks stdin and stdout before reading repository-controlled input; redirected use acquires nothing |
| identified load | CLI adapter and `aplan-app` | reads the plan/configuration once into `PlanContext`; no terminal state is held |
| session acquisition | `aplan-tui` mode guard | enables raw mode, alternate screen, focus-change reporting, mouse capture and bracketed paste in that order |
| decode | sole Crossterm input owner | converts one polled terminal event into at most one semantic `Action`; it cannot mutate the model |
| update | reducer | changes in-memory UI state and may return one typed `EffectRequest`; it performs no repository, terminal or process I/O |
| execute | bounded worker | runs at most one typed effect while one waits; an identity probe loads context without report projections, refresh builds every projection and confirms identity with a second load, and verification uses its explicit trust choice |
| complete | input-thread dispatcher | validates effect id, kind, generation and input digest before changing the last complete snapshot; stale, failed and mismatched results remain visible |
| draw | Ratatui renderer | deterministically paints dirty model state to stdout; plan-controlled text crosses the sanitization and grapheme-width boundary first |
| leave | renderer, mode guard, then runtime | restores the renderer-owned cursor, disables the five modes in reverse order, and only then waits for or discards bounded worker work |

The complete external side-effect inventory is therefore terminal mode/output, plan/configuration
and repository reads, one worker thread per active effect and the once-installed process
panic-hook dispatcher. The
dispatcher suppresses only panics on an effect thread while its typed catch boundary is active and
delegates every other panic to the previously installed hook. The foundation performs no
repository write, child-process execution, network access, clipboard access or persistent logging.

The restoration audit injects a failure after every operation which can have acquired terminal
state, rather than proving cleanup with one representative error:

- each of the five mode enables, including the failed mode because a terminal write may fail after
  changing state, and every reverse-order disable without short-circuiting later repair;
- Ratatui's construction-size query and aplan's initial-size query;
- first-frame autoresize, backend draw, cursor hide and backend flush;
- input poll and input read after a successful frame;
- explicit cursor restoration alone and together with primary draw and mode-cleanup failures;
- unwinding before a frame and after Ratatui has hidden the cursor;
- a signal at every partial mode-acquisition boundary, including a coalesced stop/continue burst;
- real-PTY `q`, Ctrl-C, `SIGINT`, `SIGTERM`, `SIGHUP` and `SIGQUIT` exits, including exact termios,
  alternate-screen, bracketed-paste and cursor state after the child exits;
- real-PTY stop/continue with exact non-default termios restoration while stopped and repaint after
  resume, standalone continue after uncatchable `SIGSTOP` with a coherent external-edit refresh,
  plus termination while a configured provider is actively blocked; and
- every non-TUI command from the executable command census, including MCP receiving EOF, with exact
  before/after termios equality and no control sequences outside the declared SGR palette.

The named tests are
`every_partial_initialization_failure_restores_every_mode_that_may_have_changed`,
`every_renderer_initialization_boundary_restores_all_acquired_modes`,
`every_first_frame_failure_restores_the_cursor_and_all_acquired_modes`,
`every_input_failure_restores_the_cursor_and_all_acquired_modes`,
`primary_cursor_and_mode_failures_are_all_reported_and_restored`,
`unwinding_after_a_frame_restores_the_renderer_cursor_and_every_mode`,
`launch_input_resize_and_clean_quit_restore_terminal_modes`,
`control_c_exits_cleanly_and_restores_terminal_modes`,
`terminating_signals_restore_the_terminal_and_keep_the_exit_reason`,
`suspend_restores_exact_termios_and_resume_repaints`,
`standalone_continue_after_sigstop_refreshes_the_snapshot`,
`termination_does_not_wait_for_an_active_provider` and
`every_non_tui_command_preserves_terminal_state_and_emits_only_sgr`.

The pre-cut repeat audit found that the final runtime step was guaranteed only on an ordinary
return. If the terminal body unwound, the mode and renderer guards still repaired terminal state,
but unwinding skipped the explicit `WorkerRuntime::shutdown` call and dropping its `JoinHandle`
detached the active repository read. `WorkerRuntime::Drop` now performs the same operation as
explicit shutdown: it cancels the one queued successor and joins the one active worker. Explicit
shutdown drains the handle first, so ordinary return does not join twice. The controlled
`dropping_the_runtime_joins_running_work_and_drops_work_that_never_started` test holds an active
worker while dropping the runtime on another thread, proves drop cannot finish early, releases the
worker and proves the queued request never starts. This closes the unwind side-effect lifetime as
well as the terminal-state lifetime; staged builds can assert it with
`tui.worker_cleanup_is_unwind_safe`.

The performance and packaging snapshot below used Rust 1.99.0 on Linux 7.2, a Ryzen 9 3900X
(12 cores, 24 threads), the stripped thin-LTO release profile and this repository's 565-task plan.
Values are comparison anchors for `A1-447`, not cross-machine thresholds.

| foundation measure | result | method |
|---|---:|---|
| process start to terminal modes | 128.197 ms median (112.454-169.857 ms) | 10 warm PTY runs after discarding two warmups, 100 by 24 cells |
| process start to first recognizable frame | 128.602 ms median (113.160-170.416 ms) | same runs, ending when the Overview title arrived |
| terminal modes to first recognizable frame | 0.555 ms median (0.041-0.839 ms) | difference between the two markers above |
| idle redraws | 0 after the initial frame | `an_idle_loop_draws_once_and_blocks_on_its_single_input_owner` counts one draw and one input poll capped by the five-second external-probe deadline |
| observed idle CPU/output | less than one 10 ms scheduler tick and 0 output bytes in 5 seconds | release PTY after the first frame was quiescent |

Both supported source shapes and the official static artifact shape were built from the audited
tree. The musl pair was cross-built in a clean Debian Bookworm environment and the standard result
was confirmed as a stripped static PIE. The headless binary returned 1 for
`facts --require build.tui`, returned usage status 2 with enablement advice for `aplan tui`, and
contained none of `aplan-tui`, Ratatui or Crossterm in its normal/build dependency graph. The
standard binary made the positive capability assertion.

| release shape | headless | TUI | TUI cost |
|---|---:|---:|---:|
| native GNU/Linux stripped binary | 12,444,008 bytes | 12,850,648 bytes | 406,640 bytes (3.27%) |
| official x86_64 musl static binary | 13,617,592 bytes | 14,023,192 bytes | 405,600 bytes (2.98%) |
| unique normal/build packages | 158 | 189 | 31 packages |

At that packaging snapshot, `aplan-app` independently resolved 94 normal/build packages and no
TUI, Ratatui or Crossterm package. It now also consumes `aplan-output`'s typed `GraphView`; that
crate points down at `aplan-core` and has no terminal-backend dependency. `aplan-tui` points down
at both and neither points back. The current graph therefore still proves that non-TUI consumers
and the supported headless source build pay no Ratatui or Crossterm dependency cost.

The graph-scene audit reran CLI byte parity, deterministic shuffle, display-width, hit-test,
malformed-input and exhaustive five-node DAG invariants. No presentation-text parser is exported or
used by the TUI. Three warm optimized scale runs produced these medians:

| approximately 10,000-node scene | construction | 1,000 40-by-40 viewports | all node hits |
|---|---:|---:|---:|
| sparse | 141.9 ms | 0.392 ms | 5.03 ms |
| fan-in | 148.3 ms | 0.422 ms | 8.00 ms |
| crossing-heavy | 104.7 ms | 0.151 ms | 4.41 ms |

Finally, the two high-level testing layers were mutation-checked rather than merely observed green.
Changing one checked-in shell snapshot cell made
`fixed_size_buffers_pin_every_layout_focus_and_minimum_refusal` fail with an exact buffer diff;
inverting the post-exit alternate-screen assertion made
`launch_input_resize_and_clean_quit_restore_terminal_modes` fail. Both mutations were reverted and
the tests passed again.

### Public dependency and binary re-audit

`A2-002` repeated the audit from the public repository on 2026-11-03. The executable policy remains
split deliberately: `deny.toml` owns licenses, registries, advisories, duplicate exceptions and
exact selected features. `ci/dependency-policy.json` owns direct admissions, package ceilings and
the measured default/headless binary ceiling. `cargo xtask dependency-audit` derives declarations
and the lock census from Cargo metadata, then derives both product shapes from Cargo's selected
package tree.

Every direct `aplan-tui` dependency is mapped exactly. The product crate edges provide typed
operations and domain/output contracts; Ratatui and the Unicode crates provide terminal layout and
text boundaries; shell-words and thiserror provide editor parsing and typed errors. Unix-only
Crossterm, libc and signal-hook edges own terminal input and process lifecycle. The aplan-config and
tempfile dev edges support isolated fixtures. The three selected supporting admissions outside the
TUI are also still used: graphlay supplies layered coordinates in `aplan-app`, shell-words and
tempfile implement CLI capture editing, and termlens drives real-PTY integration tests. No stale,
missing or target-mismatched admission was found.

The selected terminal stack remains one Ratatui/Crossterm backend line. Ratatui 0.30.2 resolves
only `crossterm` and `std`; `ratatui-crossterm` 0.1.2 supplies the Crossterm 0.29 bridge and
underline-color support; the direct Crossterm edge adds `use-dev-tty`. Graphlay and termlens retain
empty selected feature sets. Headless `aplan` resolves no `aplan-tui`, Ratatui or Crossterm package.
The internal progress renderer keeps `indicatif`, `console` and `unit-prefix` out of both product
shapes.

The locked graph uses crates.io only and passes the admitted MIT, MIT-0, Apache-2.0,
Apache-2.0 WITH LLVM-exception, Unicode-3.0 and Zlib license set. The advisory check is clean.
Duplicate versions remain denied except for six public, exact exceptions: getrandom 0.3 and its
r-efi edge remain under jsonschema's ahash while the workspace uses getrandom 0.4; Ratatui's
kasuari remains on hashbrown 0.16 while the general graph uses 0.17; stable derive dependencies
still require syn 2 while current clap and darling use syn 3; and Crossterm's filedescriptor keeps
the thiserror 1 and thiserror-impl 1 pair while the workspace uses version 2. Each inverse tree was
rechecked and none admits a second terminal backend.

| public dependency measure | 2026-11-03 |
|---|---:|
| complete lockfile packages | 288 |
| default normal/build product packages | 200 |
| headless normal/build product packages | 169 |
| TUI-only product increment | 31 |
| native stripped headless binary | 19,286,904 bytes |
| native stripped default binary | 21,684,712 bytes |
| native stripped TUI increment | 2,397,808 bytes (12.43%) |

The binaries were built separately from public control revision `51a5c95` with Rust 1.99.1 using
locked release builds for `x86_64-unknown-linux-gnu`, first with default features and then with
`--no-default-features`. GNU strip 2.47 applied `--strip-all`. The resulting SHA-256 digests are
`b0f9d3701bbef296ba4449bf71172c99604eccb571e5f2b1ca4bd06a41684dd3` for the default binary and
`88606ad35b690e0e73806a9667455d2d49c19a46ea6b02a2ecf9c3072711d873` for the headless binary. The
increment remains below both committed ceilings: 2.5 MB and 15 percent. A2-002 changes audit code,
policy and documentation rather than the shipped `aplan` product source, so the public control is
the exact product measured. The policy records only measurements that participate in that verdict;
two unused historical foundation fields were removed rather than retained as inert evidence.

The maintained YAML bridge has no current advisory, but its engine is still C source translated to
unsafe Rust. That is independently changeable hardening work, not an exception to hide inside this
admission review. `A2-009` now owns a public cutover once an immutable, licensed safe Rust backend
passes the compatibility, hostile-input, performance and complete product gates in
`docs/design/yaml-parser.md`.

### Mutations

Every mutation is preview first:

1. capture the identified input snapshot;
2. construct a typed request;
3. compute a `ChangeSet` containing affected files, reviewable before/after bytes or an equivalent
   exact delta, semantic effects, findings and denial verdict;
4. render the preview and any evidence resolution still required;
5. commit only on explicit confirmation and only if the input identity still matches;
6. reload and show the actual result.

This contract extends the existing guarded mutation gate. It is not a TUI-specific writer. A stale
preview is discarded and recomputed, never applied optimistically. Locks, complete preflight and
compare-and-swap make ordinary refusals pre-publication, and each file replacement is atomic; an OS
failure between several file renames can still leave a typed partial result, which the client must
reload and present rather than describe as all-or-none.

The reusable form scales confirmation from one focused fresh press to an arm, safe-control refocus
and second fresh press whenever a preview affects more than one task or file. Its impact projection
derives task identities from typed semantic effects and enumerates every file from the immutable
`ChangeSet`, covering cut drops, broad tag renames, imports and rekeys without an operation-specific
shortcut. A confirmation consumes the reviewed value, but the form retains all safe input when the
typed commit result returns. `Applied` means `RELOAD REQUIRED`, not success. A fresh application
snapshot must itself contain every reviewed after-state, and the filesystem must still match it,
before the form can say `COMMITTED`; an old in-memory context cannot impersonate a reload.

Publication errors carry a per-file report as well as the whole-batch state. Unchanged means a
fresh review may be retried; complete means the reviewed result is already present and must not be
repeated; partial names every completed and untouched path and requires completing the reviewed
after-state or restoring the reviewed before-state; diverged prohibits blind retry until every
third-party, missing or unreadable path is reconciled. This is recovery guidance over individually
atomic renames, not a claim that a multi-file filesystem transaction exists. The same observation
primitive gives an interrupted post-commit session an unambiguous answer after reload, while
interrupting a preview has issued no write at all.

The first work-action slice applies that contract to `claim`, `release`, `start` and `stop` at the
application boundary. One exhaustive catalogue always contains all four actions with either a typed
request or a contextual disabled reason. Eligibility uses the graph's selected-task readiness
predicate, task lifecycle, milestone role, effective actor and the current worktree-shared claim;
it does not derive availability from a static menu or compute the whole ranked ready list. A task
must be claimed by the effective actor before the catalogue offers Start, while the underlying
typed operation deliberately retains the CLI transition rules so parity is independently testable.
Milestones are checkpoints rather than work and are never offered for claim or start; release and
stop remain available as recovery actions when existing state calls for them.

Claim identity uses the CLI precedence and retains the source on the exact proposed record. A
takeover names the displayed holder, and confirmation compares the complete prior claim record so
even a same-holder reclaim with a new time or origin makes the preview stale. Claim confirmation
also rechecks the complete plan/configuration/repository input identity. Status changes use the
shared `ChangeSet` publisher and carry exact before/after bytes, a typed status effect, newly
introduced findings and the effective denial verdict. Release remains usable as local recovery
when an unrelated task file no longer loads. Both stale outcome families return the freshly
observed input or claim snapshot for the shell to display after reloading.

Reusable terminal-independent forms and review rows cover this slice now. Claim exposes `--as`
and `--from`; release, start and stop have no fake editable fields. The rows render exact holder,
identity source, timestamp, origin, scope, file and status deltas, warnings and complete findings
with instructions/details after making untrusted text inert. `A1-443` composed every workflow into
the bounded production worker, command palette, visible review and guarded publisher. Its
closed-world catalogue and clap-derived option audit cover every task/plan mutation action, while
an exhaustive CLI-leaf classification prevents a new command from silently falling outside that
scope. Real-PTY journeys cross coordination, status, planning, authoring and evidence writes.
`A1-444` then closed discovery over that write path: palette, full help and contextual footer
consume one action catalogue, search accepts CLI and human-facing boundary language, and facts,
clap leaves and behavior-bearing options are executable audits rather than separate remembered
lists.

Manifest and proposal-import forms capture the selected source bytes before preview so review and
confirmation cannot observe two versions of the file. Capture is limited to regular UTF-8 files no
larger than 4 MiB. Type is checked before opening, then checked again on the opened descriptor; on
Unix that open is non-blocking so replacing the approved path with a FIFO cannot occupy the bounded
mutation worker. Streams, devices, directories, oversized input, growth past the bound and invalid
UTF-8 are refused before a typed authoring request is constructed.

## Information architecture

The application shell has six primary destinations. Each owns one question; links may project an
answer into another destination but must not create another version of it.

| Destination | Question it owns | Primary content | Boundary |
|---|---|---|---|
| **Overview** | Is this plan healthy, and where should attention go next? | plan/build/trust identity, whole-plan check and verification state, progress by cut, ready and due work, current critical path | an orienting summary, not a task browser or generic widget dashboard |
| **Tasks** | What is true about this work item, and what may I do to it? | searchable and filterable task table, body and metadata inspector, blockers/releases and contextual actions | the canonical task detail and mutation entry; it links to topology and findings rather than redrawing them poorly |
| **Diagnostics** | Why does this plan fail or warn, and what would resolve the finding? | whole-plan `check`, `verify` and `check-docs` results grouped by severity/rule/task, exact subject/location/instruction, rule explanation and permitted repair | filters never turn a partial list into a clean gate verdict; runner failure is not an empty result |
| **Graph** | How is work connected, scoped and sequenced? | dependency lanes, neighbourhood/tree, cut scope, grouping edges and critical path with synchronized task inspector | owns topology and omissions, not task editing or aggregate programme health |
| **Reports** | What aggregate or historical signal needs a planning decision? | roadmap, version, stale work, path specificity, vacuity, deprecation runway and summary rollups | named analyses rather than a dumping ground for commands that lack a screen |
| **Inbox** | Which retained intake needs review or disposition? | bounded open and terminal captures, deferred/kind/label/state filters, provenance/defaults inspector and typed lifecycle/promotion actions | intake remains outside the task graph until one atomic promotion; open-count cues are not a pressure metric |

Selection is application state, not panel state. Following a task from a finding into Graph and
then Tasks keeps the same task selected; returning restores the prior finding and scroll position.
Every destination defines loading, empty, partial, stale and error states. "No matches" is distinct
from "no tasks", and both are distinct from "the effect failed".

Two conditional surfaces sit outside those six destinations. When discovery cannot load a plan,
an onboarding route distinguishes a missing plan/config/root from an invalid or unreadable one and
shows the exact safe `aplan init` or repair route without overwriting anything. An about/capability
view renders the running build's typed `facts` and identity rather than maintaining another list.
These are the promised guided dispositions for `init` and `facts`; `A1-453` owns them.

The implemented launch adapter retains the classification as application data. `no plan`,
`selected configuration missing`, `plan root missing`, `configuration unreadable` and
`configuration invalid` are separate causes; none becomes an empty Overview. Configuration
discovery follows the CLI's two-stage contract: discovery from the invocation directory chooses
the root, then discovery from that root chooses repository policy. `--config` never falls back and
`--no-config` never discovers. The screen names both selections. Suggested commands are quoted for
copying but inert: the TUI neither invokes `init` nor creates a directory. `r` runs the same loader
again; another classified failure replaces the guidance, while success installs one complete
identified snapshot and enables ordinary probing.

The init suggestion is conditional, not a generic response to every absent file. It is offered
only when its real output path is the missing conventional `.aplan.yml` and that file's parent
already exists. An arbitrary path selected with `--config` must be created at that exact path (or
the TUI relaunched with a conventional selection), because `init` cannot create the filename the
caller chose. A missing plan root is likewise a separate directory-creation step.

About is available from onboarding and loaded sessions with `i`. Its version, build source and
digests, exit codes, counts, commands and command formats, output formats, rules, evidence kinds,
behaviours, MCP tools, configuration keys and build capabilities are a projection of the typed
payload serialized by `aplan facts`. An executable audit derives the payload's collection keys
from serialization and compares them with the view groups, so adding or removing a collection
without a deliberate disposition fails rather than silently narrowing About.

### Semantic task inspector

The task inspector is a typed presentation, not a list of strings that happen to contain colons.
`InspectorItem` carries a stable group, one presentation role and an optional typed task or section
link. The deliberately small vocabulary is:

| role | meaning | rendering obligation |
|---|---|---|
| identity | the exact task id and title | first and visually strongest; controls are made inert before painting |
| labelled value | one named exact fact | align label/value in a roomy pane; stack both without truncating the label when space is tight |
| badge | a compact state such as status, readiness or claim | brackets preserve the badge boundary without color; semantic tone may add color/modifiers |
| metric | a number or compact value plus optional explanatory detail | emphasize the value while retaining the complete qualification |
| callout | success, warning, error, unavailable or empty-state guidance | `+`, `!`, `x`, `*`, `i` or `-` remains visible in monochrome and ASCII |
| link | a task or inspector-section identity attached to any role | append a visible `[Enter]` cue; activation follows the typed identity and never parses its label |

Headings are emitted from group changes, so they organize the page without becoming fake keyboard
rows. Summary orders the facts by the questions a reader asks: identity; status and readiness;
durable owner, ephemeral claim and coordination; phase/cut/milestone/tags; effort, revisit and
version intent; then the exact counts and links to deeper sections. The other sections use the same
vocabulary rather than falling back to flat prose:

| section | semantic groups |
|---|---|
| Work | verdict, query controls, derived priority, preferences, revisit schedule, blockers and dependents |
| Body | exact body text, including its source line structure |
| Relations | blockers, dependents, preferences, ratification and membership, with availability on each identity |
| Paths | every declared path as an exact labelled value |
| Traces | declared trace/fingerprint pairs and separately warned orphaned stamps |
| Evidence | live citations and separately warned orphaned stamps, retaining captured values and fingerprints |
| Findings | evidence-observation empty state or severity/code callouts with exact message, instruction and repair |

At 38 cells or more the renderer derives one bounded label column from typed labels. A label longer
than that column stacks in full rather than ending in an ellipsis that erases its meaning. Below
that width every label stacks above its value. Values wrap at grapheme and terminal-cell boundaries;
control bytes are visible replacements, and the model keeps the unsplit exact value for navigation
and inspection. Resizing changes only the wrapped-line projection: selected item, linked identity,
section, back-stack and explicit scroll state remain semantic model state. Up/down continuation
markers disclose content outside the pane, including when a minimum-height viewport can show only
the selected item and not its group heading.

Dark, light, low-color and monochrome palettes style the same roles. Status, readiness, terminal,
claimed, missing and finding states retain brackets, labels and callout cues, so color is redundant.
The fixed-buffer matrix covers sparse and dense summaries, warning and claimed states, hostile
controls, wide/combining text, all appearance modes, aligned and stacked widths, link activation and
reflow. A real-PTY journey scans identity/state/responsibility, narrows to 48 by 14, follows the Work
link and returns without losing the selected row. A1-449 asks participants the same product
questions without exposing these implementation labels: what is selected, whether it can start, who
owns and holds it, where it belongs, and what they would inspect or do next.

## Interaction contract

### Navigation and focus

When a modal is active, it owns all four list-movement families: arrows, `j`/`k`, Tab and
Shift-Tab. Each moves and wraps the modal selection; none reaches through to the obscured route or
task list. Enter and Space act on the visibly selected row. The same movement catalogue drives the
help text and decoder, so an advertised key cannot silently target a different layer.

The normal-mode baseline is deliberately conventional and small:

| Intent | Keyboard | Mouse |
|---|---|---|
| choose destination | `1` through `6`, or the palette | click a named destination tab |
| move within a list or viewport | arrows; `j`/`k` aliases outside text input | click a row, use the wheel, or drag across selectable rows |
| move focus between visible regions | `Tab` / `Shift-Tab` | click the region |
| open or activate | `Enter` | click a labelled control; clicking a row selects it |
| go back or cancel one layer | `Esc` | click the labelled back/cancel control |
| search this destination | `/` | click search |
| find an action | `Ctrl-P` | click the command affordance |
| contextual help | `?` | click help |
| running build and capabilities | `i` | click About |
| refresh | `r` when no text field owns input | click refresh |
| quit from the base shell | `q` | click quit |

The mouse column is the release-quality contract. Base-shell and overlay controls have semantic
click targets, while selectable modal rows, form fields and repeated form items use the same typed
visible-row projection and clipping geometry that paints them. Fixed action rows remain visible
below scrollable overlay content. Pointer use is always an accelerator: every action remains
keyboard reachable, and a hidden, clipped or illegal action has no pointer target.

Only the focused region receives navigation. Focus is always visible without relying on color,
modal surfaces trap focus, and closing one restores the exact prior focus. `Esc` unwinds search,
palette, form, preview, help and route layers one at a time; it never confirms. `q` is text inside
an editor and quits only in the base shell. Destructive actions have no single unmodified-letter
shortcut: they are invoked contextually or through the palette and confirmed from a preview.
When a text field owns input, printable keys including `/`, `?`, `1` through `6`, `r` and `q` are
text rather than global shortcuts; only the form's displayed control keys retain their meanings.

Crossterm key `Press` events invoke actions. Navigation may honor `Repeat`; confirmation and
mutation actions do not. `Release` never invokes. Bracketed paste is enabled and disabled with the
terminal guard and a reported paste is accepted only by the focused text field, with the same size
and control-byte sanitization as typed input. A legacy terminal may deliver a paste as ordinary key
events, which cannot be identified reliably. Safety therefore does not depend on paste detection:
`Enter` in a text field never commits or confirms, opening a preview is a separate action, and its
confirmation accepts only a new non-repeat `Press` while the labelled Confirm control has focus.
The portability matrix records terminals where bracketed paste could not be exercised.

Mouse support is an optional accelerator while the alternate screen is owned, and the terminal
guard owns capture whenever it is enabled. Click changes focus before acting; wheel behavior
follows the pointed topmost scrollable region; graph clicks use semantic hit targets rather than
painted glyphs. A left-button drag may change a visible row selection but cannot activate footer,
route or mutation controls. Button release, pointer movement, non-left buttons and modified pointer
events are inert, so terminals that report both button lifecycle edges cannot invoke one press
twice. An overlay owns wheel input only inside its scrollable content and blocks click-through to
the obscured shell. Modal rows, labelled modal controls, mutation fields/items, review controls and
actions legal in the exact mutation state are semantic targets; loading exposes only Cancel and a
publishing state exposes none. Pointer confirmation still requires a fresh press, keeps the
two-step broad-change arm, and refuses pending work. Modal-list presses select a row; an explicit
labelled control performs its action, so a double-click remains selection-only. After a broad
change is armed, an inert guard covers the old Arm coordinates and Confirm moves elsewhere, so a
rapid second press cannot turn one double-click into proportional confirmation. List and row
wheels stop at their boundaries rather than inheriting cyclic keyboard navigation. Activation
never requires a double-click, hover, gesture or mouse-only action.

Where the terminal reports focus-gained/lost events, they mark the display inactive and request an
immediate identity probe on return; a full refresh follows only when identified inputs moved.
Correctness and keyboard handling do not depend on those events being available. Every probe and
refresh records the focus epoch that requested it. Losing focus advances the epoch, so returning
before old work completes cannot turn that pre-loss answer into a current-state claim: a
crossed-epoch probe is discarded and replaced immediately, while a coherent crossed-epoch refresh
remains unconfirmed until the replacement probe succeeds. Process suspension enters this same
epoch boundary before the process stops, even when the terminal never reports an ordinary
focus-loss event.

### Search, filters, palette and help

These four concepts stay separate:

- `/` searches entities inside the current destination. Overview searches tasks/cuts and routes to
  the canonical detail; Tasks searches task id/title/body and composes with structured filters;
  Diagnostics searches finding code/text/subject; Graph finds a node without changing graph scope;
  Reports finds a report name from the primary list or a result key, label or summary from detail;
  Inbox searches capture id, title and body while composing with its structured filters.
  The Stale work report's as-of date is a parameter, not a search term, and has a separate
  contextual `d` editor.
- filters are visible, structured constraints such as cut, phase, tag, owner, status, unclaimed,
  reachability and time. Active filters remain visible in narrow layouts and are cleared
  individually or together. A filter never mutates underlying results.
- `Ctrl-P` searches the semantic action catalogue. Results show shortcut, context, availability
  and a reason for every disabled action. Argument collection produces the same typed invocation
  as a form; it is not a command-line parser or shell.
- `?` shows the complete action catalogue in the current context, including global actions and
  explicit reasons for unavailable ones. The footer shows a compact subset from the same action
  catalogue. Each command's key label, accepted chords and repeat policy are one binding value;
  key handling, footer, help and palette therefore cannot maintain conflicting copies.

The catalogue design follows the useful boundary in Ratatui Labs' experimental command-palette
work: semantic action identity and availability are application data, while key labels and visual
rendering are context-specific. The experiment is a design reference, not a dependency decision;
`A1-419` still has to earn any crate.

### Accessibility review and linear CLI fallback

The TUI does not claim a screen-reader accessibility tree. A terminal application paints cells in
an alternate screen and updates them in place; terminal emulators and assistive technologies differ
in whether those changes are announced, in what order, and whether the current focus is exposed.
Ratatui and Crossterm provide no portable semantic node, live-region or accessible-name channel.
A plain-text capture of a frame is useful for deterministic review, but it is not evidence that
Orca, VoiceOver, NVDA or another screen reader can operate the live interface. `A1-449` therefore
keeps an accessibility session as human evaluation rather than treating snapshots or a language
model as a screen-reader user.

Within that limitation, the interactive contract removes avoidable barriers:

- every native workflow has a keyboard route, a visible text focus marker and one-layer `Esc`
  cancellation; mouse capture adds accelerators but no required gesture;
- status, severity, selection, graph roles and continuation use words, punctuation or modifiers in
  addition to color; common color-vision deficiencies therefore cannot erase the semantic fact,
  and monochrome, ASCII and reduced-motion modes preserve the same facts;
- fresh key presses are required for activation, while repeats are limited to text editing and
  explicitly repeatable navigation; paste is accepted only by the focused editor;
- modal and mutation layers trap pointer and keyboard input, derive visible row and labelled
  control targets from the geometry that paints them, retain the prior shell focus, and restore it
  exactly when cancelled; and
- `?`, `Ctrl-P` and the footer derive from the same binding catalogue, so discovering an action
  does not require memorizing an undocumented shortcut.

The honest assistive fallback is the ordinary CLI, not a claim that an alternate-screen grid is
linear. Use `--color never` (or non-empty `NO_COLOR`) and commands such as `aplan ready`,
`aplan tasks`, `aplan show ID --body`, `aplan why ID`, `aplan check`, `aplan verify` and
`aplan graph` in the terminal, a pager or redirected text. Their `--format json` forms are the
programmable fallback where offered. Mutations remain available through their named CLI commands
and retain the same typed preview, evidence and publication gates as the TUI. This fallback loses
the TUI's persistent spatial context and is not itself a universal screen-reader certification,
but it provides sequential output without cursor-addressed redraws. `--ascii`,
`--theme monochrome` and `--reduced-motion` improve the TUI presentation when the live terminal is
usable; they do not repair an emulator or assistive technology that cannot interpret an alternate
screen.

`A1-427` established the Tasks slice of this contract. The typed application boundary projects the
whole plan once into browser rows, then applies `Selector::matches_fields`; the existing
command-line `Selector::matches` delegates to that same predicate. Values repeated within one
dimension widen it, dimensions compose by intersection, a cut may be named by label or milestone
id, and durable owner plus ephemeral unclaimed compose as distinct dimensions: team-owned work can
still be locally free to pick up. Claim state is projected from the same Git-common snapshot as the
CLI; a held row carries its holder and the inspector keeps owner, holder, claim age, identity source,
origin and unavailable coordination visibly distinct. Search began as a deterministic
Unicode-lowercase substring over id and title; `A1-428` extended the same field matcher to body
text when it added the complete inspector projection. Matches return UTF-8-safe source spans so
the terminal can emphasize source text without searching presentation text. Search in the other
four destinations belongs to their owning tasks.

The default sort is the exact `ready` order, including declared preferences, followed by non-ready
task ids. Each ready row retains both its one-based rank and the core rank explanation. Alternate
id, title, status, cut, phase and owner sorts use id as their final tie-break. The header always
names the visible/total count, sort, query and structured constraints, including in the narrow
layout. Empty plan, filters matching nothing and text matching nothing are separate states. A
selected id survives harmless reorder or metadata refresh; narrowing says
that it is hidden and offers `x` to reveal it, while refresh says when it was removed. Neither case
silently selects another task.

Text editing is local because the required behavior is small: extended-grapheme insertion,
backspace, forward delete, left/right/home/end, bounded bracketed paste, control sanitization and
display-cell-aware cursor placement. The existing `unicode-segmentation` and `unicode-width`
dependencies cover those rules. A generated 10,000-row corpus measured 13.5 ms for a combined
two-status, tag, body-query and title-sort pass in the debug test profile on the research host on
2026-10-15. That leaves ample interactive margin without a fuzzy matcher, index or new dependency.
The `task_browser_large_plan_latency_baseline` test keeps the corpus and measurement
procedure executable; a fuzzy dependency remains admissible only after a representative corpus
shows materially better discovery, not merely a different matching style.

### Forms, previews and long operations

Forms name required, optional and derived values separately. Validation is inline but submission
still goes through the whole post-state gate. Defaults such as `close` deriving `commit:HEAD` are
shown as concrete candidate values before confirmation. Secret environment values and arbitrary
configured-command output are never inserted into a form or log; the explicitly requested,
normalized configured-test catalogue is the deliberate exception used by the test selector.

Evidence forms are generated from the evidence-kind inventory rather than a parallel handwritten
list. Commit, file, test and URL fields each state their resolver boundary. The `test list` palette
action searches the configured runner catalogue on the bounded worker, keeps partial-runner and
ambiguous-name advice visible, and inserts only an unambiguous exact reference into Close for live
work or Amend Add for terminal work. The mutation preview resolves that reference again; selector
output is not proof. A note
requires the author to supply its `captured` calendar date and is labelled non-resolving. Close,
amend-add, amend-replace, amend-remove and restamp are separate workflows. `verify --fix` consumes
only typed repairs from the retained verification answer and stages every applicable replacement
in one `ChangeSet`; an empty answer is a no-op and an incomplete or stale answer cannot write.
Resolver work begins only after the form emits a preview request, so composition can schedule it on
the bounded worker while navigation remains responsive.

Every asynchronous form preview consumes the form's monotonically increasing request token before
it can replace editable state. That includes claim's `WorkActionPreview`, which is not a
`ChangeSet`: its action and review rows come only from the immutable typed preview, and its selected
task and identified plan input must match the still-open form. Editing while a claim preview runs
therefore invalidates the answer just as it does for planning, authoring and evidence forms; a corrupt
cross-task, cross-action or cross-input worker handoff remains visible as a failed effect and keeps
the current form intact.

Dismissing a running mutation layer does not cancel work which has already started. Until that
effect finishes, opening another mutation is refused before a new layer is created; the late
answer is then recorded and discarded against the closed layer. This prevents a reopened copy of
the same action from adopting an older prepare, preview or refusal merely because its local form
token began at the same value.

Planning forms use the same boundary for all four ways a graph is maintained. Blocking and
preference edges are separate typed modes; the selected task is excluded from its own candidate
list, and every candidate carries status and cut context. The preview shows the complete ready set
before and after the edit, plus the tasks that enter or leave it. Tag forms show every declared
family with its cardinality and scope alongside tags observed outside those families; a broad
rename enumerates every affected task and file. A cut/phase move reports each task's source cuts,
resolved destination and phase transition, and separately lists every milestone file in the one
guarded batch. A cut drop distinguishes the live tasks it will change from terminal members it
will retain, while a directly named terminal task or cut remains a refusal rather than an armed
confirmation. Each preview supplies the first affected task or requested cut as its post-commit
selection, and focused tests reload the published snapshot and prove that selection still exists;
because drop records a status instead of deleting a task, it remains meaningful for every
delivered planning operation. The composing shell consumes that contract in the end-to-end parity
cut rather than inventing selection policy in a terminal widget.

Every effect has an identity, state (`queued`, `running`, `cancelling`, `complete`, `failed` or
`stale`) and owning action. One foreground status region shows current work; completed background
work goes to a bounded session inbox and a non-modal notice. Navigation and already loaded data
remain available. Repeating refresh/check/verify coalesces by equivalent input or visibly replaces
the older request; it never creates an unbounded queue.

The five-second external-observation deadline yields worker capacity to deliberate user work but
not the freshness claim. If verification or another effect still owns the worker when a periodic
or resumed-session probe is due, the retained snapshot becomes or remains unconfirmed without a
notice or repeated redraw, the deadline remains due, and one probe starts immediately when the
worker becomes free. It is not queued over, used to cancel or run concurrently with the user
operation, and it does not defer itself for a second full interval.

Observation and refresh are separate host capabilities. Observation resolves the complete plan,
configuration, configured-document and repository identity but does not construct a verification
run sheet or report projections. Refresh constructs those once, then uses observation for its
post-projection identity confirmation. This keeps the timed probe equal to the path the latency
measurement exercises and prevents refresh-only work from being hidden inside a discarded result.

Cancellation is advertised only when the provider can honor it. A cancelled or failed effect
cannot replace the last complete result. Completion from an old plan/config/repository identity is
marked stale, may be inspected, and is never installed as current. Mutations cannot be committed
in the background: their preview remains modal at confirmation, input identity is rechecked, and
the guarded publisher re-observes repository HEAD/status plus selected configuration bytes and
semantics in addition to plan bytes. A changed commit, dirty-path set/status, policy or byte-only
selected-config edit therefore invalidates the preview even when the config is outside Git and its
resolved meaning is unchanged. A semantic-only embedding exposes that its bytes were not observed
and fails closed at publication. Provider output and traced or cited material are still shown as
observations, not folded into a stronger snapshot claim than their resolver supplies. A late
publication failure is reconciled per path, and success reloads the plan before reporting
completion. Quit asks whether to cancel or wait for work that cannot be abandoned safely, then
restores the terminal exactly once.

Progress uses exact counts when the service supplies them and an activity indicator otherwise.
Animation stops when hidden, when reduced motion is selected, and after completion; elapsed time
plus the operation name always conveys that work continues.

### Responsive layout and presentation

Layout is derived from available cells and content minima, not terminal-family detection. A wide
layout shows navigation, list and inspector together. A normal layout shows navigation plus a
list/inspector split. A narrow layout shows one routed pane with a breadcrumb and an explicit back
action. Below the tested minimum, a stable message names the current size, required minimum, quit
and retry behavior; it does not render clipped controls behind the message.

Resize preserves route, entity selection, filters and logical scroll anchor. When a selected row
is filtered or removed, the UI says so and offers to reveal it; it never silently selects a
different task and leaves an action armed. Long lines wrap by display width in detail views and
truncate only in tables with an explicit continuation/detail route. ASCII and low-color modes
retain borders, focus, status and graph meaning with text/symbol redundancy.

The foundation shell implements this boundary at explicit, content-derived test points: wide
begins at 120 by 24 cells, normal at 80 by 18, narrow remains usable down to 48 by 14, and anything
smaller renders only its current/required size plus resize and quit guidance. Wide uses a destination
rail plus primary and detail regions, normal uses destination tabs plus the two content regions, and
narrow makes primary/detail a routed layer rather than hiding one. Resize retains route and task
selection. At the foundation checkpoint, Overview, Tasks, Diagnostics and Graph had their
task-owned data projections while Reports, onboarding and mutation operations remained assigned to
later feature milestones; those surfaces are now delivered in the sections below.

Width chooses pane composition while height independently chooses chrome density. At 22 rows and
above, the bordered header, navigation and two-row command/status footer remain useful context. A
shorter supported terminal removes those decorative borders and spends one row each on health and
navigation plus two on commands/status. It does not remove a semantic region. The measured support
matrix is:

| terminal | composition | route work retained |
|---|---|---|
| 140x30 | wide, full chrome | destination rail, primary and detail; at least 21 inner primary rows |
| 120x24 | minimum wide, full chrome | rail plus primary/detail; at least 15 inner primary rows |
| 100x21 | short normal, compact chrome | tabs plus primary/detail; at least 15 inner primary rows |
| 80x24 | common normal, full chrome | tabs plus primary/detail; at least 12 inner primary rows |
| 120x17 | short despite its width, compact chrome | one routed pane; at least 11 inner rows |
| 60x24 | narrow, full chrome | one routed pane; at least 12 inner rows |
| 60x16 | short narrow, compact chrome | one routed pane; at least 10 inner rows |
| 48x14 | supported floor, compact chrome | one routed pane; at least 8 inner rows |
| below 48 columns or 14 rows | refusal | exact current/required size and `Esc`, `q` and `Ctrl-C` exit paths |

The route work in that matrix is asserted rather than inferred from an absence of overlap. Overview
keeps every headline metric; Tasks keeps scope/count plus selected rows; Diagnostics keeps the run
verdict and findings; Graph keeps completeness/window accounting and its scene; Reports keeps every
report destination. Narrow detail borders carry `Esc`, the destination to return to, the current
route and the inspected subject. Diagnostic detail repeats the exact run scope, narrowing,
one-run-policy, verdict and counts in its content because a compact title is not a safe place for
semantic state.

Graph applies the same rule to traversal truth (`A1-689`). The `g` scope control is retained before
presentation controls at every supported width and appends the relation class the shared
projection actually walked: `dep`, `member`, `all`, or `dep+contain` for critical path. `+nF`
counts active structured selectors separately from the one structural scope. The context line
always reports `omit nM/nG`, where `M` is a cut-membership edge hidden by dependency-only
presentation and `G` is a task which has only those grouping relationships. Membership
presentation reports explicit zeros rather than silently removing the disclosure. These compact
facts stay ahead of selection detail so the 48x14 floor cannot crop away the admission boundary.
Staged consumers can require the correction with
`tui.graph_context_discloses_scope_and_grouping_omissions`.

Footer actions are ordered by immediate recovery and discovery value instead of catalogue order.
Back, the command palette and complete help survive at the supported floor; compact Overview folds
all three exit chords into its first hint. A contextual action is retained when it fits, while every
omitted action remains in the same availability-aware palette and help catalogue. The label and
pointer target use the exact final segment, inset and cell width that were painted.

Overlays also spend margin before content: narrow and short forms, previews, result/error screens,
search, help and onboarding use the enlarged bounded overlay. An editing form derives its viewport
from the focused field, selected repeated item and exact wrapped cursor row. Page and wheel scrolling
begin at that painted offset; the next focus or cursor edit resumes focus-following. Paint and
pointer rows consume the same resolved viewport, including wide and combining graphemes.

Resize is a presentation transition only. It clamps effective list, inspector, graph and overlay
viewports but retains task identity, inspector section and back stack, graph anchor/manual pan and
window history, filters, report selection and uncommitted form bytes/cursor. A deterministic matrix
tests every boundary and every route in-process; a real PTY crosses wide, short split, narrow and
minimum layouts in one session, edits a form across both resize directions and proves terminal
restoration on exit. Staged consumers can require this complete contract with
`tui.shell_adapts_by_route_and_available_rows`.

The semantic shell commands form one catalogue that grows with each destination. Key dispatch,
compact footer hints, contextual help and the filterable `Ctrl-P` palette all read its labels and
availability, so a disabled command has one reason instead of four drifting representations. `t`
cycles the themes the terminal can safely represent; `u` changes Unicode presentation tokens and
borders to ASCII. Focused borders add explicit
`> ... <` markers and health always includes `OK`, `WARN` or `ERROR`, so color is redundant.
Repository text and palette input cross the existing sanitization boundary before grapheme-aware
cell measurement; truncation, cursor placement and label hit targets therefore share the same
wide/combining-character geometry. Reviewable fixed-buffer fixtures pin every layout class, all
focus states, the supported minimum and the below-minimum refusal, with semantic state and style
assertions beside them. One action declaration now generates both the production enum and the
test-only action inventory, so the reducer-driving table fails when a new action has no case.
The PTY journey above covers the boundary these in-process render and reducer contracts cannot see.

### Visual language and appearance policy

The implemented visual system has seven semantic roles, and every destination consumes them from
one palette: normal body text, muted provenance or unavailable context, accent for focus and
critical structure, selected for the current actionable row, good for ready/dependent/success,
warning for blockers/due work/continuation, and error for denied or failed state. Bold headings
create the primary typographic level; ordinary labels and exact values form the second; dimmed
provenance forms the third. One-cell borders separate regions, `> title <` names keyboard focus,
reverse video plus `> ` names selection, and cleared bordered overlays are the only modal layer.
Dense lists keep one row per entity while explanations wrap in inspectors and overlays. These
rules apply equally to Overview, Tasks, Diagnostics, Graph, Reports, Inbox and mutation review.

Four built-in themes map those roles without changing meaning:

| theme | intended terminal | selection and safety |
|---|---|---|
| low-color | background unknown or fewer than 256 reported colors | background-neutral normal text, reverse-video selection, standard ANSI semantic colors plus text and modifiers |
| dark | explicitly selected dark background with at least 256 reported colors | black-on-cyan selection and light normal text |
| light | explicitly selected light background with at least 256 reported colors | white-on-blue selection and dark normal text |
| monochrome | color disabled or explicitly selected | reverse, bold, underline and text badges carry every distinction |

Startup never infers a background from terminal environment strings. Low-color is the default;
`aplan tui --theme dark|light|low-color|monochrome` is the explicit override. A non-empty
`NO_COLOR` has highest precedence and fixes the session to monochrome. With fewer than 256 reported
colors, requested dark or light falls back to low-color; otherwise the requested theme is used.
The `t` command cycles all four themes with extended color, low-color and monochrome with basic
color, and is visibly unavailable under `NO_COLOR`. `--ascii` selects ASCII borders and tokens on
the first frame; `u` remains the in-session toggle. Both modes carry the same text badges, exact
values and continuation cues. All shell-owned Unicode markers are measured through the same
grapheme/cell-width boundary as repository text; the review matrix also asserts that every fixed
one-cell marker is one terminal cell and that ASCII mode emits only ASCII cells.

Overview retains its exact counts and adds fixed-width, eight-cell scan aids where the available
row can hold them without truncating the answer. `#` is completed cut work, `+` is ready work, `!`
and `?` are errors and warnings, `*` is critical-path work, and `.` is the remainder. The exact
fractions, counts, verdict, days and task count always precede the meter and remain the authority;
at a narrow width the meter disappears before any exact value does. Graph badges (`S`, `B`, `R`
and `C`) keep selected, blocker, dependent and critical roles explicit, including combined roles;
their styles are the same selected, warning, good and accent roles used by lists and inspectors.
Unrelated work uses normal text, while off-screen continuation uses the shared warning role.

Decorative motion is absent. A running bounded operation is the sole animated surface, and it
always states the operation plus any exact provider progress. `--reduced-motion` replaces its
spinner with a static `*` and stops the animation clock without hiding queued/running/completed
state; transient notices still expire. Animation stops when the operation completes or the screen
is inactive.

The visual review matrix covers wide (140x30), normal (100x24), narrow (60x16), supported-minimum
(48x14) and refusal (47x13) buffers under dark, light, low-color and monochrome themes in both
Unicode and ASCII. Review questions are executable beside the fixed snapshots: is the route and
focus identifiable without color; is selection distinct from normal text; do ready, warning and
error remain distinct; are exact Overview values retained beside their meters; do Graph roles map
to the shared semantics; does ASCII stay ASCII; and does each layout retain its identifying
content instead of clipping it? The A1-445 review answered each question affirmatively and updated
the fixed ASCII snapshots to make the compact meters reviewable in diffs.

The `A1-540` audit correction makes that geometry one explicit viewport contract. Selection moves
the task window before paint, and the same visible range plus border-free content rectangle maps
mouse rows; wheel input follows its pointed list rather than stale keyboard focus and is inert when
narrow detail occupies that pane. Table identifiers and titles, the header and footer status use
explicit continuation markers. Palette storage is bounded independently by cells, graphemes and
bytes, while its narrow renderer keeps the editable tail and cursor visible. If resize hides an
open modal behind the below-minimum refusal, the refusal's displayed `q` and `Ctrl-C` recovery still
wins. Staged consumers can require `tui.selection_is_visible_and_pointer_safe`.

The second shell audit (`A1-541`) closes the remaining implicit-clipping routes. Destination
explanations, modal command rows and repository-controlled detail fields now fit their actual pane
with a visible continuation marker; this also bounds zero-width content by graphemes and bytes
rather than cells alone. Tests exercise the exact minimum and both layout-class edges in ASCII and
Unicode modes, and assert that navigation and footer hit targets remain inside the content that
painted them. Pointer-wheel movement is deliberately clamped at list ends while the explicit
keyboard next/previous commands retain their cyclic navigation. Staged consumers can require
`tui.shell_content_is_explicitly_bounded`.

`A1-426` replaces the Overview placeholder with one application-level projection. It computes
cut progress, live and completed effort, ready rank, due work at an explicit epoch, structural
findings and the whole-plan critical path once, beside the exact task or finding identities behind
each aggregate. Renderers do not walk the graph again. Enter opens that carried membership as a
Tasks scope or the ordered Diagnostics list, and a successful refresh re-projects a retained scope
from the new snapshot rather than keeping obsolete ids. The snapshot has no duplicate task or
finding counters for the header to drift from.

The `A1-547` audit correction applies that contract to both halves of the composite revisit row.
Due and future revisits carry distinct timestamps and signed-direction offsets, while opening the
row contains every identity counted by `DUE` and `LATER`, once. A refresh rebuilds both sets; a
count without the identities behind it is not a complete interactive projection. Staged consumers
can require `tui.overview_revisit_membership_is_exact`.

`A1-429` joins that Overview entry to the task inspector through a typed work-discovery workspace.
The Work section exposes a ready task's final position after preferences, all four derived rank
factors and both active and currently inert declared preferences. Its `why` answer is cut-bounded
by default, gives each folded cut's exact unmet count, labels missing and unloadable blockers, and
switches to the complete transitive closure with `w`. An undefined blocker below loaded work stays
folded behind a cut in the bounded answer and appears in the complete answer; every adapter obtains
that boundary from the same graph implementation. Every loaded blocker and direct dependent is a
task link through the inspector's identity-preserving back stack. An inactive preference says
whether the inspected task or the preferred task is the row absent from the ready offer. `d` edits
a distinct due-work clock as RFC 3339 or signed epoch seconds; blank or whitespace-only input is
canonical snapshot-following state across refresh, while an explicit value remains pinned. An
invalid draft cannot replace the current answer. The due list and detail re-project from the same
explicit instant, and every due sentence says that the revisit is worth re-reading while the
external condition remains unasserted. Rendering reads a cached selected-task projection rather
than walking the graph per frame. Staged consumers can require
`tui.work_discovery_is_one_typed_flow`,
`tui.work_discovery_state_is_semantically_exact` and
`why.complete_retains_nested_undefined_blockers`.

The identity panel names the plan root and digest, selected configuration and semantic digest,
repository root, HEAD, exact Git-status digest and clean/modified/untracked counts, combined input
digest/source/trust, and the running build's version, source, facts digest and binary digest.
`current`, `refreshing (stale possible)` and `unconfirmed (stale possible)` describe what the
displayed bytes can support; a failed, refused or focus-lost probe or refresh never relabels
retained bytes as current. The loaded snapshot itself is immutable: guarded mutations live in a
separate preview until confirmation and reload. `memory dirty: no` therefore means the snapshot has
no unpublished in-process edit and is deliberately separate from the displayed Git worktree state.

`A1-434` resolved the external-change decision by measurement, and the `A1-575` audit corrected
what was measured. On 2026-10-17, the committed harness loaded this repository's 597-task-file
plan 15 times on Linux 7.2 and a Ryzen 9 3900X through the exact production CLI loader. That path
includes resolved validation and report policy, configured-document bytes, HEAD and exact Git
status. A context-only observation measured 78.104 ms median and 106.849 ms p95 in the release
profile; a complete refresh adding the verification run sheet, every TUI report projection and
the post-projection confirmation read measured 1.266 s median and 1.596 s p95. The debug-profile
comparison was 2.149 s versus 8.513 s median (3.805 s and 11.566 s p95). The deliberately ignored
test prints host-dependent observations rather than enforcing timing thresholds.

The 16.2x release median difference (4.0x in the debug profile) earns five-second polling while
the terminal is active, plus an immediate probe on focus gain or process resume and the
always-available `r` refresh. Filesystem notification is
not admitted: it would add a platform backend and runtime dependency, still require identity
rechecks for coalescing, rename and overflow races, and would not remove the manual/focus fallback.
The probe runs through the existing bounded worker and never queues behind user work. A changed
identity schedules a full refresh; an unchanged one is silent. Refresh retains the old complete
snapshot while building the replacement, then reloads identity again. If that second identity
differs, the replacement is rejected as raced. Otherwise plan/config/repository and load-state
transitions are named explicitly. Stable task, finding and report-row identities, route, filters,
modal state and scroll survive; removed selections and unloadable inputs remain explicit.

Load, work and estimate completeness are independent states. Empty, wholly malformed and partially
unloadable inputs are not clean zeroes; active, all-done, only-dropped and no-loaded-work plans are
distinct; complete, partial, absent and inapplicable effort totals are distinct. Six compact rows
retain exact numeric text and severity/verdict words without relying on color at 80 by 18. Narrow
lists and details expose overflow markers and deterministic scrolling, including the minimal
finding detail needed to keep an Overview drill honest. Staged consumers can require the projection with
`tui.overview_is_one_typed_projection`.

`A1-430` turns that drill into the complete Diagnostics workbench. Structural `check` and
configured-input `check-docs` are separate typed runs with independent coverage, counts and
verdicts. A scope can be complete, partial, unavailable or vacuous; an empty filtered list never
substitutes for any of those states. Search and severity/rule/task/file filters retain validator
order, and grouping adds labels without sorting the rows. The title continues to show the complete
run verdict and labels a narrowed view. Each finding retains a stable semantic identity, exact
subject and location where one exists, message, agent instruction, verdict contribution, detail
and an inert source line from the identified input. Refresh follows that identity where possible
and announces added and removed findings.

Exact rule lookup calls the same core catalogue as `aplan explain rule`; it neither normalizes an
unknown spelling nor copies catalogue prose into the TUI. A contextual lookup begins at the
selected finding, while direct lookup accepts an exact code. A finding with a loaded task subject
can open that canonical task inspector and Escape returns to the originating finding. The
advanced check sheet displays the inherited effort, trace and warning policy and attaches any
override to one refresh request. A relaxed result says `ONE-RUN RELAXED`; the draft is cleared as
soon as the request is made, and an ordinary refresh always reverts to repository policy. Staged
consumers can require `tui.diagnostic_workbench_retains_complete_runs` and
`tui.check_overrides_are_one_run`.

The adversarial audit found one split between the typed result and its rendering (`A1-549`). A
complete denied run retained its verdict when a query hid every finding, but the empty-list branch
still printed `Clean complete run`; an active filter matching every row also lost its `FILTERED`
label. The empty branch now says that no row matches and repeats the retained run verdict and
counts. `FILTERED` and `ONE-RUN RELAXED` are derived from state rather than row-count change and
precede truncatable metadata, including at the minimum supported width. Builds carrying this
correction advertise `tui.narrowed_diagnostic_state_is_truthful`.

`A1-428` builds task detail from the canonical typed `Show` projection rather than another TUI
view model. The application snapshot derives every body from the exact bytes already identified by
the load, so an edit racing the renderer cannot pair old metadata with new prose. Summary, body,
relationships, paths, traces, evidence and findings are separate sections; bracket keys cycle
them, arrows select items, page keys scroll wrapped content, and Enter follows section or loaded
task links. Escape unwinds the stored task/section/item/scroll location before it leaves detail.
Refreshing clears link history and resets scroll while clamping the selected item to the new
immutable snapshot, so a removed item cannot strand the inspector at old coordinates.
Missing and unloadable relationship targets remain distinct rows, and trace/evidence fingerprints
include unstamped and orphaned cases rather than normalizing either away. Body text also joins id
and title search, labelled on a result where neither compact field contains the match. The
inspector adds no dependency and sanitizes before display-width wrapping. A staged build can
require it with `tui.task_inspector_uses_show_projection`; require the shared complete projection
alone with `show.carries_complete_task_projection`.

`A1-432` replaced the Graph placeholder with one typed application projection over the canonical
whole-plan `GraphView`. Whole-plan, upstream, downstream, between and unreachable scopes delegate
to core reachability; neighbourhood scopes delegate to the same shared walk as `aplan tree`; and
critical scope delegates to core critical-path analysis. Cut, tag, phase, owner, unclaimed and
status selectors use the task browser's shared predicate and intersect the structural scope.
Dependency-only and dependency-plus-membership presentations use the same filtering policy as
CLI graph output, including exact hidden-edge and grouping omission counts. Scope notes name the
edges each algorithm actually walked: neighbourhood includes every `blocked_by` edge,
unreachable is defined by cut membership, and critical-path analysis includes containment.

The compatibility TUI originally laid out only the current bounded projection's shared
`GraphScene`. That historical lane painter selected nodes in stable scene order and used `h`/`l`
for horizontal pan. `A1-679` replaced its live paint with `CellScene`, and `A1-680` replaced that
row-order interaction with the layered explorer described above. The compatibility lane remains
only as explicit `--layout lanes` CLI text; it is not a live navigation source.
`g` cycles structural scopes, `b` captures the two endpoints of a between scope, `m` toggles cut
membership, `/` finds a node in the semantic projection without changing scope, `f` applies shared task filters and
`x` restores the whole dependency view. In a neighbourhood, `+` and `-` expand or contract its
finite depth. Manual pan begins at the currently centred viewport rather than jumping to an old
stored origin.

Five role letters in fixed text badges keep selection, direct blocker, direct dependent,
cut-membership and critical-route roles distinct without color, including combined roles such as
selected-critical.
The synchronized inspector follows node and edge selection; when the selected task is excluded,
it states whether filters, structural scope or hidden membership caused that absence. The header
reports exact visible/total row and column ranges, active scope, edge policy and search. The
context line reports projection state, hidden crossings and exact grouping omissions as
`omit nM/nG`; the scope control names the actual traversal edge class and active-filter count.
These exact breadcrumbs were retained instead of a minimap: a minimap would consume the
very cells needed to make a small graph usable while providing less precise orientation. At 48 by
14 cells, automatic centering and explicit scroll coordinates retain the selected node and a path
to narrow. A complete scene that exceeds the interactive node, edge or lane budget becomes an
explicit `WINDOWED` answer with visible/source and omitted counts; it is neither clipped nor
described as complete.

The behavior is declared as `tui.graph_explorer_uses_semantic_scene`; the later disclosure
correction is independently assertable as
`tui.graph_context_discloses_scope_and_grouping_omissions`. Reducer and fixed-backend
proofs cover every scope, selector, role, search, filter, keyboard and pointer path in ASCII and
small terminals. Shared integration fixtures construct fan-in, fan-out, cyclic, disconnected,
scoped hidden-edge and wide-Unicode shapes, compare the application/TUI `GraphView` with CLI JSON,
and compare painting the same scene with CLI ASCII text. No presentation-text parser, private TUI
graph walk, second layout or new external runtime dependency was admitted. A real pseudo-terminal
journey drives relationship following, membership, scope, search, ASCII mode, minimum-size resize
and terminal restoration through the shipped binary.

`A1-668` adds a bounded semantic layer between that complete application projection and scene
layout. `GraphWorkspace::project_window` first resolves the unchanged selector, structural scope
and membership request, then applies independent node, edge and peak-lane budgets. Peak lanes come
from the same canonical ordering and cycle-omission pass as `GraphScene`; edge count is not used as
a proxy for geometry. The answer carries source and visible counts, exact omitted node and edge
counts, the actual lane requirement and one of complete, windowed, empty, cyclic or failed states.
The original `GraphView` and CLI renderers are untouched, so text retains its complete-or-refuse
contract and Mermaid/DOT remain complete.

The selected task is the anchor when it survives the semantic projection. If it disappeared, the
answer reports the requested identity and reason, then deterministically prefers live critical-path
work, other live work and finally the lowest id. Windows admit a stable blocker-first,
dependent-second traversal, then membership relationships and disconnected components; resizing
changes only budgets and recomputes that presentation subset. Every crossing is grouped by raw
boundary identity, incoming/outgoing direction and `EdgeKind`. High-degree groups expose bounded,
non-overlapping cursor ranges with previous/next pages; components with no boundary edge expose a
component continuation. Exact global omission totals deliberately do not equal a sum of
continuation counts: edges wholly beyond the current boundary become crossing edges only after the
reader follows toward them. This distinction prevents double-counting while still making every
admitted node and edge reachable. Every cursor carries the immutable input digest that emitted it;
presenting it after refresh is a typed invalid-continuation failure rather than an ordinal range
silently selecting different relationships. Malformed cursors and budgets that cannot retain an
anchor or one requested relationship are likewise typed failures, not empty graphs. `A1-669` owns
adopting these answers in the Graph destination and presenting their controls.

`A1-669` completes that adoption. A graph that fits remains labelled `COMPLETE`; otherwise Graph
opens immediately on an anchored `WINDOWED` scene and states visible/source node and edge totals,
exact omissions and the selected continuation. `,` and `.` select a typed incoming, outgoing,
membership, component, earlier-page or later-page continuation, and `n` opens it. The same controls
have labelled semantic pointer targets. `Esc` returns to the exact prior semantic window before it
returns to the prior route. Relationship following or a literal graph search recentres through a
new anchored window when the target is outside the current one; panning changes only row/column
presentation inside it. Window history is bounded. A refresh which changes input identity drops
all cursors and history before rebuilding around the selected raw id, while an excluded or removed
anchor remains visibly explained rather than silently substituted. Scope, filters and membership
rebuild the window over their already-resolved complete projection, and clearing them keeps the
selected id as the preferred anchor.

Scene work is bounded to the current window (64 nodes, 96 edges and 20 peak lanes by default), so
painting and ordinary pan/selection never lay out the complete oversized graph. The complete
projection remains in memory for typed relationship and exclusion queries; continuation changes
reuse it instead of resolving selectors and structural scope again. Reducer, fixed-buffer and real
PTY proofs cover high-degree paging, history, source changes, normal/narrow/minimum rendering,
semantic mouse activation, resize storms and terminal restoration. Staged consumers require all
three independently assertable contracts: `tui.graph_windows_are_bounded_and_accounted`,
`tui.graph_window_cursors_are_source_bound` and
`tui.graph_explorer_windows_oversized_inputs`.

## Structural buffer prototype

The wide prototype tests hierarchy and routes, not typography or final decoration:

```text
+ aplan | root: demo/aplan | config: .aplan.yml | CHECK DENIED: 2 findings --+
| [1 Overview] [2 Tasks] [3 Diagnostics] [4 Graph] [5 Reports] [6 Inbox] |
+-------------------------------+--------------------------------------------+
| READY                         | SELECTED: P-101                            |
| > P-101 repair evidence       | Repair stale evidence                      |
|   P-119 improve onboarding    | status: in_progress  owner: sam            |
|                               | waits on: none  releases: P-117            |
| ATTENTION                     |                                            |
|   1 error   1 warning         | Why / Graph / Claim / Start / More...      |
|   2 due     critical: 8.5 d   |                                            |
+-------------------------------+--------------------------------------------+
| Tab focus  / search  Enter open  r refresh  C-p actions  ? help  q/C-c quit |
+----------------------------------------------------------------------------+
```

Tasks, Diagnostics and Graph keep the shell and replace the content question. A mutation overlays
the selected context instead of navigating to an unrelated wizard:

```text
+ Close P-127 ---------------------------------------------------------------+
| Evidence                                                                   |
|   test:validate::tests::citations_resolve      resolved / required         |
|   commit:abc123                                touches declared paths      |
|                                                                            |
| Candidate change: status in_progress -> done; add 2 citations; 0 traces    |
| Preview: STALE - the input plan changed; recompute before confirmation     |
|                                                                            |
| [Esc Cancel]                                      [Enter Confirm disabled] |
+----------------------------------------------------------------------------+
```

The narrow prototype tests that collapse is navigation rather than deletion:

```text
+ aplan > Diagnostics > APLAN013 ------------------------------+
| APLAN013 error                                   task P-099  |
| The cited test did not resolve in one configured runner.     |
|                                                              |
| Subject: test:validate::tests::citation_resolve              |
| Location: demo/aplan/tasks/P-099.md                          |
|                                                              |
| [Explain rule] [Open task] [Evidence]                        |
+--------------------------------------------------------------+
| Esc back  / search  Ctrl-P actions  ? help                   |
+--------------------------------------------------------------+
```

The remaining prototype states are summarized here. The runnable review kit in
[`tui-structure-review.md`](tui-structure-review.md) fixes the synthetic fixture, buffers,
transitions, driver wording, intervention policy and evidence record so actor sessions do not
quietly test different structures:

| State | Primary region | Inspector / next routes |
|---|---|---|
| onboarding | cause (`no plan`, `missing root`, `unreadable`, `invalid config`), paths searched, trust choice | exact `aplan init` or repair command, retry, About, quit |
| Overview | health, progress by cut, ready and due lists, critical-path summary | selected task/cut explanation; Tasks, Diagnostics or Graph |
| Tasks | id/title/status/owner table plus visible structured filters | complete task/body/evidence/blockers/releases; Why, Graph and available typed actions |
| Diagnostics | whole-plan verdict plus severity/rule/task/source groups | exact finding subject/location/instruction, Explain rule, Open task, typed repair |
| Graph | semantic lane viewport, scope chips, extents and omission counts | selected node, blockers/dependents, critical route; Open task and scope controls |
| Reports | named report list and its parameters | provenance-bearing result rows; open the task/cut represented by a row |
| Inbox | bounded retained captures plus explicit state/revisit/kind/label narrowing | provenance, defaults and disposition; typed amend/discard/restore/promotion and promoted-task link |
| action palette | action query and category, shortcut and availability per row | disabled reason or typed arguments; invocation returns to the originating context |
| mutation preview | semantic change, affected files, exact delta, findings and policy verdict | cancel, recompute stale input, or confirm only when allowed |

The implemented Reports state makes the provenance rule concrete. Its application snapshot owns
the resolved configuration and captures recent-commit breadth plus task-file last-touch times once
under repository-read trust. Moving among reports, editing the stale as-of date and applying a
selector are pure projections over that identified input. A refresh replaces the workspace as one
snapshot; a report never combines old Git observations with a newer plan or configuration.

Roadmap, version and deprecations are whole-plan questions, matching their CLI contracts. Their
scope line says that shared filters do not change the computation. Summary, paths, vacuity and
stale work apply the shared selector to rows while retaining whole-graph readiness. Each visible
row carries the exact contributor ids used to produce it; Enter opens that frozen set in Tasks and
does not re-run the report under a newly inferred scope. A version configuration fault has no task
to invent and is labelled as configuration instead of offering a false drill-down.

The edge states are product states rather than prose conventions. Paths distinguishes no sampled
history, a task measured at zero hits, a task with no declared paths, and an invalid path pattern
that was not measured. Vacuity renders a switched-off rule and a rule with zero subjects instead
of treating either as a pass. Stale work validates an explicit `YYYY-MM-DD` before accessing Git,
uses the end of that day, refuses dates before HEAD, and labels every result advisory. Repository
failure is `unavailable`, malformed input is `invalid`, and an empty successful result remains
`ready`. The version view names `version.current.command` but does not run it: opening a report is
not consent to execute repository-configured code. Its argv is displayed as an argument array,
preserving whitespace, quotes and empty elements rather than flattening a different-looking shell
command.

For the static review, every row is drawn with the same header, destination strip and contextual
footer as the wide prototype. A route changes the primary/inspector regions, not plan identity or
selection. The narrow version renders the primary region first and makes the inspector a named
next route.

### Formative structure review gate

The screen structure remains provisional until isolated language-model actors complete the fixed
review in [`tui-structure-review.md`](tui-structure-review.md). This is a structural preflight, not
a human usability study: it can expose missing routes, contradictory labels, hidden write
boundaries and internally inconsistent states, but it cannot establish accessibility, trust,
motor behavior or that a person finds the interface intuitive. `A1-449` retains the later
five-participant evaluation of the real implementation and remains a deferred release gate.

The cohort uses one identified prototype revision and the same neutral prompts. Both unfamiliar
observable-strategy profiles receive at least three fresh complete sessions; counted runs span at
least two provider families and include every adapter whose configured default passes conformance.
Unavailable and failed attempts stay visible. Neutral canaries qualify transports without showing
an evaluation screen. Actors see only the current goal, rendered buffer, bounded prior action
labels and primitive actions - never internal state labels, driver effects, oracles, this design,
expected routes, source or hidden state. An invalid first response ends the attempt; repair and
replacement runs cannot turn it into a completion.

The paper driver records routes and checks named structural and safety invariants. A separately
blinded cross-family critic sees only exact screen/action pairs under an open-ended rubric; each
finding must quote visible text that the driver resolves. Actor statements and critic
interpretations remain candidate observations. Every observation is incorporated,
reduced to an independently reproducible failure and filed, or declined with a reason. If a
disposition materially changes a tested state, all affected scenarios repeat at the new revision;
an old run cannot accept a screen no actor saw. The normalized cohort and one-to-one dispositions
are tracked, and a replay audit recomputes fixed scheduling, prompt/response bindings, grounding,
route facts, diversity and eligibility. A clean cohort means only that those actors completed those
routes without a deterministic failure.

[`tui-ai-evaluation.md`](tui-ai-evaluation.md) defines adapter isolation, evidence classes, drift,
artifacts and failure handling. The research summarized there supports synthetic actors as a
prototyping aid and warns against treating them as people; narrowing this early gate preserves that
boundary rather than relabeling synthetic output as participant evidence.

## Current-command coverage

The command inventory was re-derived from `aplan facts` during `A1-444`. It has 43 members; the
earlier 42-command snapshot predated `test`, the 41-command snapshot predated the `tui` entry
point, and the 40-command snapshot before it had drifted when `explain` shipped. The executable audit is the live
source after implementation. This disposition is a product boundary, not a second hand-maintained
implementation checklist, and counts below are assertions about this snapshot rather than a gate.

| TUI disposition | Commands | Reason |
|---|---|---|
| Native plan understanding | `check`, `verify`, `test`, `check-docs`, `ready`, `due`, `why`, `version`, `graph`, `show`, `explain`, `tree`, `tasks`, `critical-path`, `roadmap`, `summary` | Daily interactive questions and the core evidence-gated workflow; configured test search belongs to evidence authoring, while document findings and rule explanations remain contextual rather than separate artifacts |
| Native analysis reports | `paths`, `vacuity`, `stale`, `deprecations` | Valuable inspection views that share selection and task detail |
| Native guided operations | `claim`, `release`, `start`, `stop`, `add`, `close`, `amend`, `supersede`, `dep`, `tag`, `mv`, `drop`, `rekey`, `import` | Human-guided changes that benefit from forms, previews and immediate validation; several are deliberately destructive rather than generically "safe" |
| Guided setup/about | `init`, `facts` | Missing-plan onboarding and build/capability identity, not raw document rendering |
| CLI-only batch artifacts | `attest`, `export`, `export-agents-md`, `export-roadmap-json`, `schema` | Produce named external artifacts; a full-screen UI adds no value and complicates stdout/file contracts |
| CLI-only transport | `mcp` | A long-running stdio server, not an interactive plan action |

This gives native interactive treatment to 34 of the 43 commands in this snapshot and guided
treatment to two more. Six stay discoverable in help with their exact CLI spelling, and `tui`
itself is the entry point rather than an action inside its own interface. Every other new command must
receive a disposition in the executable coverage audit; a historical count never excuses a new
member. Native mutation entries point to the complete `MutationAction` union rather than held
tasks, and their complete-state parity drivers fail if an action disappears.

Command-level coverage is necessary and not sufficient. The executable audit also enumerates
behavior-bearing global and per-command options: root/config/trust selection, selectors,
reachability, traversal, as-of time, verification execution, mutation modes and policy overrides.
Each gets an interactive equivalent or an explicit boundary visible to the user. Machine-output
formats, color switches and other presentation-only CLI flags can be inapplicable to a full-screen
surface, but `native` cannot be used as one checkbox that hides an unimplemented semantic mode.

### Behavior-bearing option disposition

This table records the interaction decision, including optional positional modes. `A1-435` made
the read-only census derive its membership from clap. `A1-443` added the closed-world task/plan
mutation catalogue and derives every such command's argument inventory from clap, so a new
behavior mode cannot disappear silently. Every other CLI leaf is classified outside that scope,
so a new command also cannot evade the census. `A1-444` completed how those actions and boundaries
are exposed across palette, contextual help and the final feature-coverage audit.

| CLI behavior | Interactive equivalent or explicit boundary |
|---|---|
| global `--root`, `--config`, `--no-config`; repository `--repo` | launch/session inputs, always visible in identity/help and immutable for the loaded session; onboarding gives exact relaunch commands. Repository and plan root remain separate. |
| global `--color`; report output `--format` values | rendering choices, not hidden domain modes. Theme/contrast/ASCII controls replace color selection. Machine, Markdown, Mermaid, DOT and other serialized projections stay exact CLI alternatives where the output artifact is the purpose. `import --format` is excluded from this row because it selects input semantics. |
| shared `--cut`, `--phase`, `--tag`, `--owner`, `--unclaimed`, `--status` | structured filter editor shared by Tasks, Graph and Reports. Diagnostics may filter a completed whole-plan verdict; `check`/`verify` are never scoped into a partial green gate. |
| `graph --reach`, `--membership`; `tree --depth`, `--live`; `why --all`; `critical-path [TO]` | Graph scope controls and named projections. Current scope, hidden satisfied ancestry, grouping-edge choice, endpoint and every omission count remain visible. |
| `ready --explain`; `show --body`, `--no-resolve`; `version [ID]`; `explain rule NAME` | inspector toggles and contextual routes. Evidence resolution state is labelled; rule explanation opens from a finding; version offers plan-wide next release or a named task/cut/release. |
| `due --as-of`; `stale --older-than`, `--as-of`; `paths --sample`; `summary --at`; report `--repo` | report-local controls whose non-default values stay in the title/filter bar. Revision/time/repository provenance is part of the result, never hidden state. |
| `check --no-require-effort`, `--no-traces`, `--deny-warnings`, `--no-deny-warnings` | the advanced structural-check run sheet shows effective configuration and these one-run overrides before execution. A relaxed override is conspicuous in the result and never becomes an unlabeled session default. Selecting an alternate `--traces-root` remains an exact visible CLI boundary because the TUI is bound to the loaded root. |
| verification `--repo`, policy overrides and receipt import/refresh modes | the interactive verifier uses its loaded repository, trace and warning policy. Palette/help expose an inert exact CLI alternative for each override and for receipt-backed, incremental, working-tree and shadow workflows. `--execute` remains the native explicit trust-bearing toggle. A classification in an audit table is not credited unless its boundary is present in the product catalogue. |
| `verify --execute`, `--fix` | separate trust-bearing actions. Execute names configured programs and requires explicit invocation. Fix consumes typed repairs, shows one guarded `ChangeSet`, and never turns prose advice into a write. |
| `check-docs [PATHS]` | Diagnostics defaults to configured document inputs and offers an explicit path-set control; the result names whether configured or one-off inputs were checked. |
| common mutation `--deny-warnings`, `--no-deny-warnings`; `--dry-run` | preview is the native dry run and shows the complete denied/allowed result under the loaded plan policy. One-run warning-policy overrides remain CLI-only and the form says so; the TUI does not imply that a per-preview override exists. |
| `claim --as`, `--from`; `drop --cut`; `mv --cut`, `--phase`; `rekey --allow-terminal` | typed fields in the contextual action. Takeover names both holders; multi-task/cut scope and terminal-id override receive destructive emphasis and exact affected-file previews. |
| `test list [query]`; `close --evidence`, `--restamp`, `--no-resolve`; `amend --replace-evidence`, `--add-evidence`, `--remove-evidence`, `--no-resolve` | a bounded-worker configured-test selector plus dedicated evidence forms. Search retains typed partial and ambiguous answers, and an unambiguous exact reference seeds Close or Amend Add before the ordinary preview resolves it again. Close, restamp and each amend mode remain distinct claims; unchecked mode says exactly what resolution it skips, and a default `commit:HEAD` is materialized before confirmation. Alternate `--repo` selection remains a visible CLI-only boundary because the TUI uses its loaded repository. |
| `supersede --cut`, `--effort`, `--owner`, `--tag`; `dep add/rm --after`, `--prefer`; `tag add/rm/rename` | typed successor, edge and tag forms using current task/cut/family choices. Preference and blocking edges are visually distinct; rename previews every affected task. |
| single `add` fields (`--id`, `--phase`, `--blocked-by`, `--tag`, `--effort`, `--owner`, `--paths`, `--traces`, `--cut`) | one native form with derived/default values labelled and the complete candidate plan gated. `--dry-run` is its preview. |
| `add --from`, `--out`; `import` source plus `--format`, `--out`, `--phase`, `--implemented-as`, `--as-of`, `--cut`, `--id-prefix`, `--tag` | Batch add and every currently accepted import shape get a native review/preview inside the identified plan, including their semantic options and loss report. `--out` destinations outside that plan remain CLI-only boundaries with exact examples; no TUI action writes outside its identified plan. |
| configuration, artifact and repository writers (`init`, exports, `attest --attach-note`) | Explicitly outside the task/plan mutation catalogue. They remain CLI workflows; the exhaustive leaf audit keeps that boundary visible when the CLI grows. |
| `init [DIR]` | onboarding explains target and exact safe CLI command, then retries discovery. It stays guided rather than becoming a special pre-`ChangeSet` writer. |
| `facts --identity`, `--require` | About renders typed build identity and capability inventory. The process-exit assertion remains an inert, searchable `aplan facts --require ...` boundary with an exact example rather than being hidden by the native About route. |
| roadmap `--regenerate-with`; `attest --repo`, `--execute`, `--attach-note`; export/schema write/check/output and format options; `mcp` | intentionally CLI-only generation, batch, artifact, git-note and transport contracts. Help shows exact invocations and never spawns them or embeds a shell. |

The explicit boundaries are part of parity. For example, native `add` does not imply arbitrary
`--out`, and native `verify` does not imply that configured programs may execute on opening the
screen. Conversely, presentation flags do not create fake feature gaps: the TUI need not render
JSON to claim the interactive question it already answers.

### Read-only release audit and real-plan evaluation

`A1-435` completed the v0.11 read-only audit on 2026-10-18, and `A1-444` replaced its local command
table with the product catalogue in `crates/aplan-tui/src/discovery.rs`. The live audit in
`crates/aplan/tests/tui_read_surface.rs` compares that catalogue with both clap's complete command
tree and the command collection serialized by `aplan facts`. Each of the 20 native read commands
runs against a complete Git-backed fixture, proves that it
did not write the plan, and invokes the typed application projection named by its disposition. The
fixture carries the configured document bytes and verification providers used by the production TUI
loader; the audit asserts that `check-docs` inspected its document and `verify` inspected its test
citation, rather than crediting an unconfigured projection that merely had the right type.
Fourteen mutation commands map to the exact 25-member typed action union proved by the mutation
parity suite; `verify` additionally maps its repair action alongside its read surface. The two
guided commands, six batch/transport boundaries and the `tui` entry point each state why they are
different and carry a copyable exact command where the boundary is non-interactive. Nested
boundaries keep `test receipt`, capability assertions, alternate check/verification roots and
policy, and receipt import/refresh workflows from being hidden beneath a command-level native
label.

The same audit walks every argument clap exposes on the global and 20 read-command surfaces.
Launch trust, shared selectors, reachability, traversal, explicit clocks, verification execution
and policy overrides each have a successful command case. Every behavior-bearing argument is
claimed by exactly one such case; a missing, duplicate, stale or merely named claim fails. Output-
only presentation flags and the separately proved `verify --fix` mutation are explicit
dispositions and are not credited as read executions. An option classified as CLI-only must map
to a searchable product-catalogue boundary that names the exact flag and parses as a real command;
a test-only label is not evidence. A recursive leaf pass covers nested `test list`/`test receipt`,
guided commands including the `facts --require` boundary, complete CLI-only commands and the TUI
entry point; a new nested native leaf is refused until it receives a deliberate audit. The table
therefore cannot credit a command while silently omitting a semantic mode. These cases prove
parse-to-command reachability; semantic value parity remains at
the typed boundaries:
task/overview tests cover empty, malformed and large catalogues; report tests compare every report
family with CLI JSON and keep empty, unavailable and invalid results distinct; the graph test
compares the complete `GraphView` and shared scene across fan-in, fan-out, crossing, disconnected,
cyclic, wide-Unicode and scoped-out shapes.

The ordinary suite now drives 19 real-PTY journeys through the built binary, plus a nonterminal
identified-fixture check. It covers invalid
configuration to missing-plan repair, the displayed non-overwriting `init` command, a real external
`init`, retry and About; Overview health to a location-bearing finding; ready work through folded
blockers; literal task search; and Tasks/Graph round trips. The graph journey uses real keyboard and
mouse input, follows both relation directions, selects a membership-only task, enters critical-path
scope where that selection is excluded, recovers to a visible task, switches glyphs, resizes and
pans a larger scene. Structural and document findings both expose source locations in the PTY.

The same opt-in real-PTY driver was also exercised against plans independent of aplan's fixture.
Each run painted identifying markers for Overview, Tasks, Diagnostics, Graph, Reports, and Inbox,
restored the terminal on exit, and retained both plan bytes and observable Git state. Version-skew
findings remained visible as plan findings rather than being rewritten by the TUI. Those private
plan inputs and measurements are not part of the public test corpus; the generated and synthetic
fixtures below carry the reproducible release claims.

Responsiveness is measured at both boundaries. An in-process dispatch must navigate in under 100ms
while refresh owns the only worker, and the real PTY must repaint Tasks in under 500ms while an
executing verifier is blocked. These budgets measure input-to-state and input-to-frame respectively;
neither waits for the background operation. Status-only and diagnostic-only refresh tests compare
every node marker and connector cell, and preserve the selected id, manual viewport and viewport
mode.

### Release performance budgets

`A1-447` replaced the observational foundation timings with two executable release gates. Both are
ignored in ordinary tests and run serially from `ci/scripts/scale.sh` on the nightly/manual Linux
lane. That keeps optimized linking, real PTYs and generated 10,000-task plans out of development
handoffs while requiring the lane before a release. Every gate prints its samples and budgets even
when it passes.

The real-terminal gate uses stable generated fixtures rather than this repository's changing task
count. A forced `refresh` cache mode is the cold case; the following `auto` process is the warm
generation-hit case. Three cold/warm pairs are interleaved so host drift does not favor one state.
The reported p95 is conservatively the maximum of the three. First feedback means the visible
loading title after alternate-screen ownership with the cursor safely hidden; usable means the
identified Overview frame. The timing receipt after each process exits must also prove zero cache
hits for forced refresh and at least one generation hit for `auto`, so a mislabeled series fails
instead of merely looking fast without coupling the gate to the number of internal plan reads. RSS
is Linux `VmRSS` sampled at usable content. Search includes every typed character and its painted
result; refresh includes an identified task edit, reload and repaint.

| fixture/state | observed feedback / usable | observed action / search / refresh | observed RSS | enforced feedback / usable / action / search / refresh | RSS ceiling |
|---|---:|---:|---:|---:|---:|
| 32 tasks, cold | 17ms / 117ms | 1ms / 11ms / 103ms | 25.0 MiB | 250ms / 750ms / 100ms / 250ms / 1,000ms | 64 MiB |
| 32 tasks, warm | 16ms / 119ms | 1ms / 14ms / 103ms | 22.8 MiB | 250ms / 500ms / 100ms / 250ms / 1,000ms | 64 MiB |
| 700 tasks, cold | 10ms / 222ms | 7ms / 46ms / 214ms | 33.7 MiB | 250ms / 1,000ms / 150ms / 750ms / 2,000ms | 128 MiB |
| 700 tasks, warm | 8ms / 217ms | 4ms / 52ms / 211ms | 31.5 MiB | 250ms / 750ms / 150ms / 750ms / 2,000ms | 128 MiB |
| 10,000 tasks, cold | 10ms / 2,057ms | 140ms / 897ms / 2,745ms | 159.8 MiB | 250ms / 6,000ms / 250ms / 2,500ms / 6,000ms | 320 MiB |
| 10,000 tasks, warm | 9ms / 2,249ms | 93ms / 961ms / 2,256ms | 156.0 MiB | 250ms / 4,500ms / 250ms / 2,500ms / 4,500ms | 320 MiB |

These 2026-10-27 measurements are release thin-LTO runs on the shared Linux host used by the
private runner. The ceilings round upward to about twice the largest observation for full-plan work
and at least 1.75 times it for input-to-frame work, with more fixed headroom on sub-millisecond
paints where scheduler noise dominates. That admits the private runner's observed contention and
the future four-vCPU hosted class without turning a several-fold regression into a pass. Warm cache
is proven from the timing counters but is not assumed to beat a forced refresh in every wall-clock
sample on a shared host. The first measured large cold launch was 26.35s and refresh was 29.41s:
complete inspector construction rebuilt the same reverse dependency index for every task. Building
that immutable index once reduced them below three seconds; the gate would fail if the quadratic
path returned.

The in-process gate uses one warmup and seven release samples. It keeps separate ceilings for the
work a PTY total would blur:

| operation | ceiling |
|---|---:|
| build the 10,000-task model and bounded graph | 1,500ms |
| filter, search and paint 10,000 task rows | 400ms |
| paint Tasks at 32 / 700 / 10,000 rows | 4ms / 6ms / 12ms |
| summary resize and reflow at any supported class | 12ms |
| construct sparse, fan-in, crossing or too-wide graph window | 250ms each |
| lay out scene / paint viewport / pan | 10ms / 4ms / 25ms |
| hit-test / focus / continuation traversal | 0.25ms / 50ms / 100ms |
| accept and project a 10,000-task status-only refresh | 750ms |
| paint loading progress / reduce one progress tick | 5ms / 0.1ms |

The list-paint gate additionally limits 10,000 hidden rows to eight times the 32-row cost plus
0.75ms, rather than trusting an absolute number alone. Every graph fixture retains at most 64 scene
nodes and separately times semantic window construction, layout and viewport painting. Progress
painting is repeated 2,000 times and may retain at most 16 MiB on Linux; idle has zero dirty frames
and no clock, transient ticks expire, and reduced-motion startup now repaints only on progress
events. Redirected, disabled and machine progress stay outside the TUI and remain covered by the
shared progress-policy tests.

The checked-in shell snapshots were reviewed semantically at 140x30, 100x24, 60x16, 48x14 and the
47x13 refusal, not accepted because their bytes happened to match. Health precedes navigation and
content; the focused region and selected row have non-color markers; wide and normal layouts keep
identity beside the decision surface; narrow layout shows one named focused pane instead of two
ambiguous panes; truncation and vertical overflow have visible markers; and the below-minimum state
names both the current and required size plus recovery. No snapshot hides an error behind a clean
filtered view, and plan text remains inert. The deliberately sparse minimum view omits secondary
identity detail rather than clipping it into a false value; focus navigation reaches the complete
narrow pane.

## Release-quality field findings

The safe foundation is not yet the final public experience. A current-plan field pass after the
v0.12 operations cut established four gaps and one interaction-contract correction. The launch gap
is closed by A1-666, the graph gap by A1-668/A1-669 and the pointer correction by A1-672; two
presentation gaps remain scheduled by the tasks named below:

- aplan's own whole graph is 51 lanes wide and needs 102 gutter columns before titles. The shared
  scene honestly refuses its 95-column cap, but the TUI then has no scene for its existing pan,
  follow or hit-test controls to operate on;
- the task Summary inspector carries complete facts but flattens almost all of them into equally
  weighted `label: value` strings;
- the 48x14 shell is bounded and escapable, but fixed header/navigation/footer allocations leave
  only four rows for the bordered work pane.

These are not grounds to weaken the existing truth and safety contracts. They refine how those
contracts are presented:

1. **Feedback is a typed engine fact before it is animation.** Time to first truthful feedback is
   measured separately from time to usable state. Known totals may drive a gauge; unknown work
   names its phase without a fake percentage or ETA. TUI startup uses the existing worker/reducer
   boundary. Ratatui is immediate-mode and leaves event/task architecture to the application, so
   adding an async runtime only to paint loading would add weight without solving ownership
   ([Ratatui application pattern](https://ratatui.rs/templates/component/)). Built-in Gauge and
   LineGauge widgets are sufficient if measurement earns a bar
   ([Ratatui widgets](https://docs.rs/ratatui/latest/ratatui/widgets/)).
   A1-666 now acquires an eligible terminal and commits a stable shell-shaped loading frame before
   build identification, configuration discovery or plan input begins. One bounded worker owns the
   complete launch generation and moves the same frame through source/cache work, repository
   observation, verification preparation, structural validation and projection. It adopts exactly
   one identified snapshot or onboarding state. Quit requests cooperative cancellation, restores
   terminal modes first and then joins the worker; failures follow the same restoration boundary
   before their diagnostic is returned.
2. **CLI progress is a protocol policy.** A1-665 implements the shared model and policy internally;
   A1-666 and A1-667 complete the TUI-loading and CLI adapters. `--progress` governs stream output;
   it never erases the TUI's in-frame loading state, whose animation remains controlled by reduced
   motion. One CLI owner spans the command and every plan/provider child generation, validates the
   typed stream and serializes its row around ordinary output. A late worker, duplicate terminal or
   concurrent provider therefore cannot redraw a different run merely because its phase has the
   same name.
3. **A graph window is data, not clipping.** A bounded projection keeps an anchor, admits a
   deterministic subset, reports exact directional omissions and provides continuation identities.
   Pan moves within one scene; follow/expand/page changes the semantic window. Complete CLI text and
   Mermaid/DOT exports retain their current whole-graph contract. A1-668 now supplies projection,
   accounting and source-bound continuation cursors; A1-669 now supplies interactive traversal,
   reversible history, semantic pointer controls and bounded-scene presentation.
4. **Hierarchy belongs in the model.** Summary headings, labelled values, badges, callouts and links
   carry typed roles. The renderer must not recover structure by splitting display strings. Wide
   panes may align values; narrow panes stack the same values with non-color meaning. A1-670 owns
   this vocabulary and its representative review fixtures.
5. **Responsive means useful work, not merely no overlap.** Width and height both allocate chrome;
   route identity, focus and a back path survive when secondary header, navigation and footer detail
   collapses. Each route and overlay owns a narrow composition, and resize preserves semantic state.
   A1-671 owns that integration. A1-672 now supplies typed overlay pointer geometry shared by paint
   and hit testing, so later presentation changes cannot recreate click-through or infer actions
   from prose.

### Operation progress foundation

The application contract is an ordered stream, not rendered text. An event carries an operation
id, a per-operation sequence, an operation kind, an operation or phase subject, an optional child
id, a lifecycle and a measure. Lifecycles are `queued`, `started`, `advanced`, `completed`,
`failed` and `cancelled`. A measure is either indeterminate or a completed/total pair with an
explicit unit. The consumer reducer refuses a different operation or kind, an out-of-sequence or
post-terminal event, an invalid lifecycle, a changed total/unit and a decreasing count. A dropped
non-panicking producer emits cancellation; an unwinding producer invokes no callback and leaves
the host panic boundary to own the terminal outcome.

The initial operation vocabulary is CLI command, plan load, verification and test catalogue. The
CLI command generation is the presentation lifetime and contributes the projection phase around
the command's native child operations; it does not replace their more precise phases. Interactive plan
startup additionally reports build identification and configuration discovery before plan work,
then verification preparation and projection around the resulting application snapshot. Plan load
reports cache lookup, source enumeration, source parsing, snapshot confirmation and cache publication.
Each bounded snapshot attempt has a child identity, so a retry starts a new count instead of making
one bar move backwards. Verification adds structural validation, repository observation and
configured evidence-provider phases. Providers use stable one-based declaration ordinals as child
ids. Paths, task text, configured labels, provider output and environment values never enter the
event; they belong in the final typed result or diagnostic. The TUI reducer validates the operation
id derived from its effect id before changing visible detail.

`PlanStore` and the verification/test-catalogue operations are the emitters. They retain quiet
entry points for MCP, library and fast CLI calls that have no consumer, so disabled instrumentation
does not allocate a channel, start a thread or walk the sources a second time. The CLI command
enum has one exhaustive mode-aware progress disposition: plan load, plan plus configured
providers, measured fast, long-lived protocol or interactive frame. The output format is taken
from the resolved mode. `graph` is the one conditional omitted format: stdout terminal presence
resolves it to human text, while a redirect resolves it to Mermaid; an explicit format always
wins. The source distinguishes plan-only,
plan-plus-repository, plan-plus-provider and provider-only work from interactive,
long-lived-protocol and inapplicable commands. Modes matter: plain `show` observes repository
evidence, `show --no-resolve` does not, and `version <id>` avoids the configured process and Git
work required by bare `version`. Adding a command therefore fails compilation until it has a
disposition; the representative census test drives every category and these mode boundaries.

The stream policy is exact:

| choice and surface | result |
|---|---|
| `auto`, human format, stdout and stderr both terminals, no active `CI` signal | delayed stderr progress |
| `auto`, either stream redirected, machine/generated format, or active `CI` | no stream progress |
| `always`, any non-interactive mode | delayed stderr progress; stdout is unchanged |
| `never`, any non-interactive mode | no progress bytes on either stream |
| any choice, TUI | progress belongs in-frame; stream rendering is disabled |

`CI` is active for any non-empty value except ASCII-case-insensitive `0`, `false`, `no` or `off`;
a non-Unicode value fails closed as active. Non-empty `NO_COLOR` removes progress color,
`--color always|never` remains authoritative for human output, machine-format progress is always
unstyled, `TERM=dumb` selects the non-animated stream form, and the portable form is ASCII. The
TUI's explicit `--reduced-motion` appearance setting independently governs its in-frame animation;
it is not an undocumented environment variable.

The complete CLI renderer (`A1-667`) owns one command-lifetime grace clock and thread. It waits 120
ms before its first row and admits semantic updates or animation frames no faster than every 100
ms. At a capable attended terminal it draws one grapheme- and cell-width-fitted Braille activity
row, erasing it before stdout/stderr writes and at the terminal event. It never changes cursor
visibility. The static fallback emits newline-terminated ASCII only on reveal, meaningful changes
and the command terminal, so a captured log is bounded rather than a repeated spinner transcript.
Concurrent provider children are represented by their aggregate completed/total provider measure;
configured labels, child ordinals and output remain private. A consumer state is created only by
sequence-zero `queued`, then validates kind, sequence, lifecycle and monotonic measure. Late,
duplicate, cross-generation and post-terminal events are inert. A blocking phase is revealed by a
channel deadline, not polling; the command terminal is acknowledged before the renderer joins, and
ordinary writes use the same pause/restore boundary. Redirected-signal tests exercise cancellation
without control bytes, while PTY tests prove erase-before-output and that cursor visibility is
untouched.

### First-frame launch ownership

Terminal eligibility remains the first check: a redirected `tui` invocation refuses before reading
configuration or repository-controlled input. For an eligible terminal, the TUI resolves appearance,
enters its guarded modes and draws a shell-shaped loading frame before it submits the startup effect.
The placeholder carries no plan facts and therefore never displays a false zero-task or clean-plan
answer. Its stable header, destination, work and command geometry avoids a transient splash on a warm
load; normal and wide layouts reserve the eventual inspector pane, while constrained layouts retain
the current phase and an exact bounded count when one exists. Dark, light, low-color, monochrome,
Unicode, ASCII and reduced-motion paths all carry the same text state. An indeterminate phase says
that no fixed total exists and never displays a percentage or ETA.

Build-facts construction and the digest of the running executable are part of the worker-owned
`identifying this build` phase. This matters on a cold filesystem: hashing the executable before the
first draw would merely move the apparent hang ahead of plan loading. Configuration selection, both
stages of discovery, onboarding classification, plan context construction and verification-sheet
preparation share the same resolver as quiet retry/refresh paths; progress is an observer on that
resolver, not a second copy of launch policy. Snapshot validation and presentation projection then
complete the same operation inside `aplan-tui` before one generation-safe reducer transition replaces
the placeholder. `--progress never` suppresses stream adapters only and cannot erase this in-frame
state.

Cancellation is cooperative and monotonic. The source loader checks it before opening, refreshing or
publishing a cache and between candidate reads. Work already inside a blocking operating-system read
is not detached: quit restores the terminal, requests cancellation and joins the owned thread at the
next safe boundary. Resize and focus events remain reducer input during loading; suspend tears down and
reacquires modes around the same owned worker. A result is accepted only for the startup effect's exact
operation and snapshot generation. Onboarding is a successful typed launch, while a fatal startup
error requests exit and is surfaced only after mode and cursor restoration.

The dependency review selected the internal renderer. `indicatif` 0.18.6 has the useful pieces:
stderr draw targets rate-limited to 20 Hz, hidden non-terminal targets, bounded bars,
`MultiProgress`, and a steady-tick thread. It still does not decide aplan's both-stream rule,
machine-mode and CI silence, explicit override, privacy vocabulary, grace threshold, generation
rejection or TUI ownership. Its manifest has MSRV 1.85, below aplan's 1.88, but on Linux it would
add `indicatif`, `console`, `unit-prefix` and `portable-atomic` to the normal linked graph
(`unicode-width` and `libc` are already present), introduce a second terminal abstraction beside
Anstream/Crossterm and retain a background-thread lifecycle to audit. The internal implementation
adds no package or lockfile entry and shares the same reducer with the TUI.

Measured after implementation on the shared development host, the normal linked graph remained
184 packages and the debug binary was 71,284,736 bytes. Twenty interleaved warm runs of
`show A1-665 --no-resolve --format json` measured 67.5 ms mean with `--progress never`, 69.5 ms
under redirected `auto`, and 67.4 ms with `always`; the spreads overlapped, so no measurable
inactive or enabled-fast-path regression was established. These are dependency and regression
observations; the release-class budgets above own the interactive boundary.

The production `--timings` counters separate the work rather than attributing the whole command to
progress. On the 695-source/694-task repository, one cache-off sample took 144.9 ms total (74.8 ms
load, 43.8 ms validate, 12.8 ms project); an explicit cache-refresh sample took 225.2 ms (75.4 ms
load, 124.3 ms validate, 13.2 ms project, plus a 1,056,295-byte cache publication). Five subsequent
warm cache-hit samples took 55.5-65.5 ms total, with medians of 26.3 ms load, 7.1 ms validate and
14.0 ms project; all 695 sources were reused and no source or trace bytes were read. The shared-host
spread is why these remain observations. It also exposes the next adapter obligation: a plan-load
generation can finish within the grace period while later validation makes the complete command
slow. A1-667 therefore owns one command-level presentation spanning all native phase sources,
rather than treating a short child generation as proof the invocation is short.

All five lanes remain in v0.13 and therefore block the initial release audit. A1-447 through
A1-452 measure, port, evaluate, document and adversarially re-audit the assembled result rather than
evaluating the older defensive shell in isolation.

## Functional release sequence

### v0.10: foundation

Settle the product contract and dependency budget, prove the shared application boundary, create
the crate and command, make terminal restoration reliable, and ship a responsive navigable shell
with deterministic tests. This release may be useful only as an architecture skeleton; it must be
safe and measurable.

### v0.11: plan explorer

Deliver the read-only daily driver: overview, task search/detail, ready/why/due, diagnostics,
verification, graph exploration and analytical reports. External edits and long-running work are
handled without losing context or freezing input. Missing-plan onboarding and the build-capability
view fulfill the two guided command dispositions without adding an early writer. Evaluate real
plans before adding writes.

### v0.12: plan operations

Put the ordinary work loop behind previewed typed mutations: ownership/status actions, dependencies,
tags, cut moves, add/import/supersede/rekey, and evidence-gated close/amend/restamp. Destructive or
broad changes receive explicit previews and confirmations. CLI and TUI parity tests prove that both
surfaces accept and refuse the same operations.

### v0.13: release quality

Finish visual design, keyboard/mouse accessibility, performance, terminal portability, recovery,
documentation and usability evaluation. Audit dependency and binary-size changes again. This is the
quality gate for the original TUI scope. Field use also requires prompt truthful startup/CLI
progress, a traversable bounded graph when the whole graph is too wide, semantic summary hierarchy,
adaptive narrow/short layouts and complete overlay pointer accelerators before this cut can close.

### v0.14: workflow ergonomics

Integrate the later completion-history, terminal-Markdown and quick-capture requirements into the
feature-complete interface. Recent completions gain a native history view, task bodies use the
shared semantic Markdown renderer, and a dedicated Inbox supports capture through guarded task
promotion. `A1-509`, `A1-523` and `A1-518` own those three TUI adapters respectively; none waits
on its sibling CLI or MCP adapter merely to claim parity.
[`workflow-ergonomics.md`](workflow-ergonomics.md) owns the cross-surface contract and
`M-public` waits on this milestone. Its interactive tasks depend on the exact v0.11/v0.12
foundations they extend, and its final audit waits on the completed v0.13 milestone; independent
domain work does not. The public gate therefore covers the complete interaction surface without
freezing the earlier command census in time or serializing unrelated work.

## Quality and evaluation gates

The milestones turn exact thresholds into measured baselines before implementation claims them.
The release-quality floor is:

- no redraw while idle and no busy polling;
- measured startup, refresh, search, render and memory behavior on small, repository-sized and
  generated large plans, with time-to-first-feedback separate from time-to-usable and committed
  regression budgets derived from the baseline;
- attended slow operations expose truthful phase progress while disabled, redirected and machine
  modes remain byte-stable and consume no animation work;
- responsive input while `verify --execute` or another bounded effect runs;
- deterministic snapshots at representative wide, normal, narrow and minimum sizes;
- reducer tests for every action/state transition and parity tests for every native mutation;
- executable censuses for command dispositions and evidence-kind inputs, where each entry drives a
  case and a future operation remains a held gap with an owning task rather than a passing `none`;
- a behavior-option census prevents command-level coverage from passing while a meaningful CLI
  mode silently disappears;
- PTY tests for launch, navigation, resize, interruption, panic/error restoration and clean exit;
- keyboard-complete operation, visible focus, non-color status cues, ASCII/low-color behavior and
  no mouse-only action;
- Linux, macOS and Windows build coverage, plus real smoke tests where CI can provide a PTY;
- a support matrix that distinguishes a compile check, automated PTY evidence and a manual terminal
  smoke test instead of treating them as interchangeable;
- task text and findings remain sanitized before reaching the terminal;
- a first-use study and expert dogfood sessions complete representative workflows without coaching,
  with observed failures filed before the public gate closes;
- graph evaluation asks participants to locate a task, trace both edge directions, identify the
  critical route, explain omitted work and recover from an off-screen selection; at least one graph
  is too wide for a complete scene and requires continuation traversal; the same answer is compared
  with CLI text where both surfaces support it;
- summary evaluation asks people to find identity, status/readiness, ownership/claim, placement and
  next actions without reading an undifferentiated block, including at narrow and short sizes;
- graph scale tests enforce the 10,000-node scene construction, viewport, hit-test, status-only
  refresh, pan and resize budgets on sparse, high-fan-in, high-crossing and too-wide fixtures.

## Non-goals before public

- a daemon, remote dashboard or network service;
- a second configuration language for layouts or workflows;
- arbitrary plugins or user code inside the process;
- automatic mutation on file changes;
- animation as a requirement for comprehension;
- embedded shell, editor, git client or replacement for batch export commands;
- a general undo stack: previews, guarded atomic file writes, explicit batch-failure state and
  version control are the recovery contract;
- exact UI parity with every CLI flag where the underlying workflow is intentionally CLI-only.

## Decisions deliberately left to measured tasks

- whether layout caching improves representative plans enough to enable it;
- whether fuzzy matching, text input or a PTY harness earns an additional crate;
- whether a later minimap can earn permanent cells after the measured first candidate failed its
  navigation-benefit and useful-card gate;
- whether custom keymaps and themes belong in the first public release or follow the built-in
  accessible choices.
