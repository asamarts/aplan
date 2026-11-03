# Layered graph layout evaluation

Date: 2026-10-27; implementation hardening updated 2026-10-28

Tasks: `A1-676`, `A1-682`, `A1-686`

Decision: use an aplan-owned semantic projection and terminal-cell scene over a replaceable
`graphlay` 0.3.1 geometry adapter. Lay out only a bounded semantic window, never the complete task
source during interaction. Keep the existing lane renderer through the explicit post-hardening
compatibility decision in `A1-683`.

This document records the evidence behind ADR-0060 and the implemented-candidate audit in
`A1-682`. The renderer is implemented and mechanically hardened; it is not yet human-validated.
`A1-683` owns the compatibility/default decision and `A1-449` owns observed human evaluation.

## Question and hard constraints

The current Graph destination is truthful and navigable but remains a linear lane document. The
question is whether aplan should build a layered kernel, use a native dependency or embed an
external renderer to obtain phase, milestone and task views that read as a directed graph.

The live implementation has these hard constraints:

- Rust 1.88 and the existing Ratatui/Crossterm product boundary;
- deterministic output from canonical semantic identities, independent of input iteration order;
- exact terminal display columns, grapheme-safe labels and cell-accurate hit targets;
- typed accounting for every admitted and omitted node and edge;
- bounded interactive work, stale-result rejection and no layout work during paint;
- left-to-right and top-to-bottom layout, stable refresh, compound phase grouping and honest cycle
  disposition;
- one native binary without Graphviz, a browser, JavaScript or JVM availability at runtime.

## Current architecture audit

The current path is:

```text
Plan and Graph
  -> GraphWorkspace / GraphProjection
  -> GraphWindowProjection
  -> GraphScene::layout
  -> TUI model viewport
  -> Ratatui Paragraph paint and semantic hit dispatch
```

The first two stages are reusable. `GraphWorkspace` owns selectors, structural scope, critical
route and membership policy. `GraphWindowProjection` owns the identified bounded subset, exact
source/admitted/omitted counts and continuation tokens. Selection and navigation already use raw
semantic identity rather than parsing display text.

At the original research commit, the remaining stages were lane-specific. `GraphScene`
topologically orders task rows, gives each
active dependency a vertical gutter lane and caps the static scene width at 95 columns. Nodes are
row markers rather than spatial boxes. The TUI then slices scene rows and paints styled spans.
Scene identities and same-geometry hit targets are useful concepts, but its row coordinates,
connector cells, width refusal, scene-order navigation and Paragraph painter are not a reusable
layered layout.

The implemented replacement boundary is therefore below semantic projection and above painting.
It must not
make `graphlay`, Ratatui, glyphs or rendered rows owners of plan meaning.

## Reproducible method

The checked-in harness is
[`prototypes/a1-676-layout-spike`](prototypes/a1-676-layout-spike/README.md). It ran from commit
`bd8f701a62dbc79b1b585ff2d238f1689229f945` on Linux 7.2, an AMD Ryzen 9 3900X with 24 logical
CPUs, Rust 1.88.0 and an unoptimized development build. The host was not quiesced, so times are
comparative medians rather than cross-machine budgets.

Each native candidate also received an isolated exact-version `mbx +1.88 check`. The graphlay
probe disabled default features. The layout harness used a ten-unit internal scale per terminal
cell, then quantized and checked node overlap and collapsed route segments. A first one-unit probe
was rejected: pixel-oriented tolerances consumed terminal-sized clearances and produced misleading
geometry. The scale is an adapter concern to tune and test, not a unit conversion that the domain
projection may observe.

The compile probes used a minimal Rust 2021 package with `rust-version = "1.88"` and one exact
dependency, then ran `mbx +1.88 check`. The graphlay dependency was
`{ version = "=0.3.1", default-features = false }`; the equivalent exact versions for the other
probes are in the audit table below. `mbx tree --edges normal -p graphlay` resolved only `libm` and
Serde in graphlay's no-default normal graph. The checked harness lockfile preserves the full
transitive resolution used for the measured run.

Three renderers were compared:

1. Lanes: the current `GraphScene` and its exact width/refusal behavior.
2. Ranked cards: an aplan-owned longest-rank prototype with median sweeps and straight edges. This
   is intentionally the smallest credible in-house kernel, not a straw-man row renderer.
3. Layered cards: graphlay with stable semantic keys, orthogonal routes, both orientations, fixed
   spacing and `Fast`, `Balanced`, and a bounded custom interactive effort profile.

The real inputs were the complete aplan task graph (334 nodes, 383 dependency edges), the v0.13
cut (29, 59), and a phase-grouped milestone quotient (20, 45). Generated fixtures covered a chain
(12, 11), diamond (8, 9), fan (14, 24), bounded high fan (50, 96), crossing-heavy graphs (40, 60
and 56, 90), a two-phase cross-cut graph (12, 8), disconnected components (18, 7), an invalid
cycle (6, 6) and hostile Unicode labels (8, 8). The real views also include cross-cut and
cross-phase relationships. Every renderer was sampled at 48x14, 80x24, 120x36 and 160x50.

Measured outputs were scene bounds, cards intersecting the anchor-centred viewport, crossings,
bends, overlap, routes through nodes, collapsed quantized segments, deterministic displacement
after reversed input order, prior-scene displacement and warm layout time. For path finding, the
spike reports how many distinct edge identities participate in an unrelated-edge intersection or
a collapsed cell segment, alongside graphlay's unrelated-node hit count. This conservative
structural risk proxy distinguishes one hard-to-follow edge from many pairwise intersection
events. It is not a human comprehension or accuracy result. `A1-682` owns executable mechanical
traceability, while `A1-449` owns blinded human path-tracing evaluation.

## Results

### Baseline and bounded scenes

| view | current lanes | small ranked prototype | graphlay bounded profile |
|---|---|---|---|
| complete task graph, 334/383 | refuses every tested width: 50 lanes need 100 columns and the scene caps at 95 | 2,058 crossings in left-to-right; 536x448 | 200 crossings left-to-right, about 0.88 s; still too large and too slow for interactive use |
| v0.13, 29/59 | refuses at 48; 80x99 or 95x78 otherwise | 95 crossings; 476x18 left-to-right | 31 crossings and about 30 ms left-to-right; 27 and about 31 ms top-to-bottom |
| milestone overview, 20/45 | 74x28 at useful widths but still a task-row lane view | detects 14 nodes in quotient feedback cycles; 30 crossings left-to-right | 25 crossings and about 17 ms left-to-right; 22 and about 18 ms top-to-bottom |
| bounded fan, 50/96 | refuses every width: 48 lanes need 96 columns | no straight-line crossings but routes share an unreadable fan | 173 crossings and about 207 ms left-to-right; bounded but close to the existing 250 ms debug interaction ceiling |
| bounded crossings, 56/90 | 53x98 at useful widths | 50 crossings | 18 crossings and about 19 ms left-to-right |

The current lane view remains compact and exact on small graphs, but it fails the real full graph
and high-fan window by construction. The small ranked prototype is attractive on a fan yet loses
badly on real and crossing-heavy graphs, provides no orthogonal routing or compound handling and
cannot correctly lay out a cyclic aggregate quotient. Building those missing pieces recreates the
general layered engine.

Graphlay produced no geometric node overlap, route-through-node result or quantized node overlap
on the bounded corpus. Reversing input enumeration produced zero displacement because stable keys
and canonical input order were used. It had no collapsed route segment on bounded fixtures. The
two collapsed segments observed on the forbidden complete source reinforce the no-full-layout
rule; they are not an acceptable scene to paint.

The audited harness also reports route identities, not just intersection totals. On the real
v0.13 left-to-right view, 34 of 59 edges in both lanes and the small prototype participated in at
least one unrelated-edge crossing, compared with 23 in graphlay. On the 56-node crossing fixture,
the counts were 64 of 90 for lanes, 52 for the small prototype and 36 for graphlay. The explicit
cross-cut fixture found no risky lane or small-prototype edge and four graphlay edges. These are
structural warning counts, not path-tracing scores. The selected kernel therefore required the
implemented hardening below and still requires the observed human evaluation in `A1-449`.

`Balanced` commonly spent four to ten times longer for little or no reduction in crossings and
even introduced one reported overlap on the v0.13 left-to-right candidate. It is not the live
default. A fixed custom interactive effort profile is the starting point, with exact hard input
and work limits. Higher effort may be evaluated for background static export only.

### Stability and cycles

Identical layouts were byte-for-byte stable and shuffled inputs had zero positional delta. A
prior-scene seed was exact on most synthetic graphs but moved the real v0.13 view by about 9.8
cells left-to-right and 12.2 cells top-to-bottom. Stable keys are therefore not a promise of
incremental stability. Status-only refresh must reuse cached geometry. Structural refresh must
measure displacement, preserve the selected anchor and explain forced movement.

The milestone quotient exposed an important semantic edge case: collapsing tasks into milestones
can produce a cycle among milestone aggregates even when the canonical task graph is acyclic. The
14 quotient nodes involved are not evidence of an invalid plan. Aggregate projection must retain
the exact underlying task edges and mark a layout feedback relationship; cycle diagnostics remain
owned by the canonical task graph.

### Minimap admission result

Before the capacity comparison, the admission bar was: reduce the modeled action count for at
least two of three scripted relocation/path tasks by 30 percent, lose no more than 15 percent of
useful cards in the wide viewport and add no keyboard-inaccessible action. The scripts were
relocate to a known identity, follow one exact dependency and return to overview. This was an
interaction-model and capacity screen, not an observed-user timing or accuracy study.

A 26x12 semantic minimap reserved from a 160x50 viewport reduced none of the modeled action
counts. Known-identity focus and Overview are already direct semantic actions; the candidate map
offered no exact task-dependency action. The reserved width reduced visible cards from 24 to 18 on
both crossing-heavy left-to-right fixtures, a 25 percent loss. Other fixtures sometimes lost
nothing, but no demonstrated interaction benefit offsets the worst representative loss. The first
release therefore omits the minimap. Breadcrumbs, viewport extents, directional counts, search,
focus and recenter remain mandatory. A later minimap proposal needs observed task-performance
evidence rather than reopening this choice by decoration alone.

## Candidate audit

Registry and repository state was inspected on 2026-10-27. Download counts and stars are weak
signals, so the decision uses source, exact builds and capability evidence instead.

| candidate | license and compatibility | dependency and maintenance shape | result |
|---|---|---|---|
| `ascii-dag` 0.11.0 | MIT OR Apache-2.0; declares Rust 1.92; exact Rust 1.88 resolution fails | no normal dependencies; active single-project maintenance; terminal cells, ports, groups and custom painters | reject: hard MSRV failure, released Unicode width limitations, no incremental/cancellation contract and duplicates a painter aplan must own |
| `graphlay` 0.3.1 | MIT OR Apache-2.0; declares Rust 1.88; exact no-default build and spike pass | pure Rust; no-default normal graph adds `libm` and Serde; very new and lightly adopted | select behind adapter: deterministic keys, cycle removal, ranks, compounds/group lanes, orthogonal routes, seeds, limits and metrics; aplan must own quantization, bounded work, cache and degradation |
| `dagre` 0.1.1 | Apache-2.0; no declared MSRV | young Rust port of an archived/moved upstream; general floating geometry | reject: exact no-default Rust 1.88 build fails because JSON code imports optional Serde unconditionally; no terminal or incremental contract |
| `rust-sugiyama` 0.4.0 | MIT; no declared MSRV; exact Rust 1.88 build passes | `petgraph` and `log`; limited recent activity | reject: useful ranking/crossing kernel but no compounds, orthogonal terminal routing, stable incremental scene, cancellation or cell contract |
| `gen-sugiyama` 0.5.1 | MIT; edition 2024, no declared MSRV; exact Rust 1.88 build passes | `petgraph` and `log`; active and released days before the audit | reject for now: current release adds cycle removal and deterministic tie work, but lacks compound, orthogonal terminal, incremental and cancellation contracts |
| `abstracttui-graph` 0.5.0 | MIT; declares Rust 1.87; exact Rust 1.88 build passes | depends on the AbstractTUI application/rendering framework; small and active | reject: importing a second TUI framework violates the selected Ratatui and aplan application boundary |
| Graphviz `dot` / ELK / browser Dagre | mature reference engines; separate executable, JVM or JavaScript runtime | runtime installation, startup, trust, portability and failure boundary | reject for live TUI; retain DOT/Mermaid export and optional differential reference testing |
| Graph::Easy | Perl static terminal renderer | external process; static layout can be slow and has no aplan identity model | reject for live TUI; retain interaction ideas only |

The capability review separates geometry inputs from terminal correctness. A kernel marked
"caller-sized" does not understand Unicode; it can be used only when aplan computes grapheme-safe
card dimensions and validates the quantized result.

| candidate | deterministic contract | Unicode/cells | compound groups | incremental/stability | cancellation/bounds |
|---|---|---|---|---|---|
| `ascii-dag` | deterministic scene API | terminal-native but released width model fails required graphemes | groups and ports | no retained-layout contract found | no cancellation contract found |
| `graphlay` | stable keys, total comparisons and fixed seed; reversed-input spike had zero delta | caller-sized floating geometry; aplan adapter required | compounds and strict group lanes | prior positions and comparison metrics, but real seed displacement was non-zero | input/resource/effort limits; no cooperative cancellation or deadline |
| `dagre` | no release-level stable-output contract found | caller-sized floating geometry | compound graph model | no interactive retained-layout contract found | no cancellation contract found |
| `rust-sugiyama` | no release-level stable-output contract found | caller-sized coordinates | none found | none found | no cancellation contract found |
| `gen-sugiyama` | current release documents deterministic tie work | caller-sized coordinates | none found | none found | no cancellation contract found |
| `abstracttui-graph` | not established by this audit | terminal-oriented scene | grouping support advertised | not established by this audit | explicit honesty markers, but no suitable aplan worker contract established |
| Graphviz / ELK | deterministic modes depend on engine/options/version | external geometric output | mature clusters/compounds | mature incremental options in ELK, not a native aplan state contract | process termination is possible but creates the rejected runtime boundary |

Graphlay has no cooperative cancellation token or time deadline. The selected boundary does not
pretend otherwise: projection size and effort are strictly bounded, layout runs outside paint,
and a generation mismatch discards a stale result. If a bounded call still exceeds the committed
budget, the application shows an honest focus/list fallback. It must not leave an unkillable
subprocess or a frozen input loop.

## Selected boundary

The product owns these conceptual values; exact Rust names may be refined by `A1-677` and
`A1-678`:

```text
ExplorerProjection {
  input_identity, semantic_level, nodes, edges, groups,
  aggregate_provenance, admitted_counts, omissions, continuations
}

LayoutRequest {
  projection, cell_viewport, card_sizes, orientation, density,
  peer_order, selected_identity, optional_prior_scene, work_limits
}

CellScene {
  input_identity, semantic_level, node_boxes, group_boxes,
  edge_routes, ranks, ports, bounds, semantic_hit_regions,
  continuations, degradation, quality_metrics
}
```

`ExplorerProjection` contains no coordinates, Ratatui types or graphlay ids. The adapter converts
canonical raw identities to stable kernel keys, lays out at an internal geometric scale, converts
to display columns and rows, then validates overlap, ports, routes, bounds and total accounting.
Every scene element keeps the raw semantic identity and provenance needed by navigation. A
graphlay error, resource limit, quantization failure or unsupported geometry becomes a typed
degradation; it cannot become missing paint.

Both the Ratatui widget and static CLI painter consume `CellScene`. They may choose glyphs, styles,
clipping and viewport chrome, but may not recompute rank, group membership, omissions or edge
identity. Hit targets are emitted from the same cell geometry as paint. The lane scene remains a
separate compatibility adapter rather than rendered text parsed back into this model.

## Decision and implementation constraints

`A1-678` may admit exact `graphlay` 0.3.1 with default features disabled. The dependency does not
land in this research task as unused production code. The implementation must isolate it behind
the boundary above, pin the reviewed feature graph, enforce canonical stable keys and hard window
limits, and retain independent scene invariants and differential fixtures. `A1-450` re-audits the
final dependency and binary cost before public release.

No interactive path may lay out all 334 current task nodes merely because the source contains
them. Overview lays out milestone aggregates; milestone/task detail lays out the bounded
projection. Aggregate quotient cycles use typed feedback edges and never produce a false plan-cycle
finding. Status-only refresh reuses geometry; structural refresh is generation-tagged and reports
movement metrics. Auto orientation compares bounded candidates under a deterministic score and
hysteresis rather than rerunning layout on paint.

The final terminal adapter deliberately overrides graphlay's layered facing-port preference with
`PortSides::Any`. The facing preference traded bends for crossings: on the matched hostile graph
it exceeded A1-683's frozen bend limit. Aplan still accepts only ports on the source/target bounds
and only complete monotone orthogonal routes, then measures the same graph against the explicit
Lanes compatibility scene. This is routing policy behind the adapter, not a public graphlay type.

The A1-696 post-delivery audit found a second adapter boundary: a floating-point route may touch an
unrelated box border without entering it, while quantization makes that border an occupied painted
cell. Rejecting the route was truthful but replaced the ordinary v0.13 milestone graph with the
fallback list in both clients. The adapter now detects this after quantization, asks graphlay to
reroute over the fixed placement when useful, and deterministically diverts any remaining
orthogonal border segment by terminal cells. Ports, monotone flow, orthogonality, bounds and every
unrelated card are revalidated afterward. The frozen release-candidate topology runs in both
orientations; a repair that cannot reduce collisions is rejected rather than painted.

The result is deliberately replaceable. If adversarial implementation work reveals a graphlay
cell, correctness, latency or supply-chain defect, aplan can replace the geometry adapter without
changing semantic projection, interaction, painters or formats. The small in-house prototype is
not the fallback by default; it becomes viable only with a separately measured task that owns the
missing routing, compound, cycle, stability and metric behavior.

## Implemented-candidate hardening

`A1-682` froze the code candidate at
`0d9d2a86a561d47da5725c5fa4868d75a5e5c4cb`. The audit treats a scene as complete only if every
admitted identity, dependency route and non-empty phase band has validated current geometry.
Missing points, ports, arrows, orthogonality, containment or safe quantization produce a typed
fallback. A prior scene may seed recovery, but it is never painted or hit-tested as the current
answer after layout failed.

The discoverable hardening corpus covers chain, diamond, repeated fan-in/out, crossing plus a long
edge and hostile graphemes, disconnected components, and cycle/self/parallel edges. Separate
fixtures cover cross-phase and cross-cut boundaries, empty groups, milestone nesting and cyclic
milestone quotient over an acyclic task graph. Every one of the 64 labelled four-node DAGs runs in
both orientations and in reversed input order. The shared assertion checks exact node and edge
identity, canonical provenance and ordinals, phase containment and non-overlap, card separation,
ports, arrows, orthogonal routes, bounds, hit agreement and total dependency accounting. Aggregate
feedback retains every contributing task edge without raising a canonical task-cycle condition;
the invalid task-cycle fixture remains a distinct typed state.

The debug corpus emitted these stable candidate metrics. `Nodes` is also the useful-card count:
these bounded fixtures admit and paint every node. Status-only and selection-only reuse have zero
cell displacement; reversed enumeration is byte-identical. Structural relayout reports its actual
Manhattan displacement and cause rather than asserting stability.

| fixture | nodes/edges | LTR crossings/bends/area | TTB crossings/bends/area |
|---|---:|---:|---:|
| chain | 6/5 | 0 / 0 / 455 | 0 / 0 / 605 |
| diamond | 4/4 | 0 / 4 / 516 | 0 / 4 / 600 |
| repeated fan | 7/8 | 0 / 10 / 1,121 | 0 / 10 / 1,295 |
| crossing, long edge, hostile text | 8/10 | 2 / 14 / 1,952 | 2 / 12 / 1,848 |
| disconnected | 5/2 | 0 / 0 / 513 | 0 / 0 / 555 |
| cycle, self and parallel | 3/5 | 0 / 6 / 387 | 0 / 4 / 450 |

Those are the frozen A1-682 facing-port numbers, not the final default candidate. The first matched
A1-683 calculation exposed that left-to-right's 14 bends exceeded 125 percent of Lanes' 10 even
though its 2 crossings were below Lanes' 6. A1-688 changed only the private port-choice policy and
reran every invariant. The final `6857c302` figures are:

| fixture | nodes/edges | LTR crossings/bends/area | TTB crossings/bends/area |
|---|---:|---:|---:|
| chain | 6/5 | 0 / 0 / 455 | 0 / 0 / 605 |
| diamond | 4/4 | 0 / 2 / 516 | 0 / 2 / 600 |
| repeated fan | 7/8 | 0 / 5 / 1,121 | 0 / 5 / 1,295 |
| crossing, long edge, hostile text | 8/10 | 3 / 10 / 1,952 | 2 / 7 / 1,848 |
| disconnected | 5/2 | 0 / 0 / 513 | 0 / 0 / 555 |
| cycle, self and parallel | 3/5 | 0 / 5 / 387 | 0 / 4 / 450 |

The executable matched calculation derives crossings and bends from typed Lanes edge ownership,
not painted characters. Both final orientations remain below Lanes' 6 crossings and no worse than
its 10 bends, while every provenance, port, arrow and containment invariant remains unchanged.

The A1-676 lane and in-house prototype measurements remain the frozen structural baseline above.
They use the old complete-task and generated fixtures, while the shipped explorer deliberately
uses semantic overview or bounded detail, so pretending their area totals are directly comparable
would reward different admitted work. The comparable conclusions did not reverse: lanes remain
exact and compact on small inputs but refuse wide real views; the small prototype still lacks
compound groups, orthogonal routes and quotient-cycle handling; the selected adapter retains all
admitted identities and markedly reduces the measured crossing-heavy baseline. Path traceability
is protected mechanically by exact provenance, direction, arrow, hit and relation-cycle checks;
whether people interpret it faster or more accurately remains the blinded A1-449 question rather
than a synthetic score.

Fixed-buffer tests span 120x30, 80x24 and 48x14 layouts, every built-in theme, Unicode and ASCII,
compact and comfortable cards, both directions, manual pan, continuation and fallback. The visual
mutation guard fails when an arrow, edge, node, label, selection, continuation or phase boundary is
removed. Entry-policy tests name contextual, remembered, in-progress, ready, unfinished,
all-done and no-milestone anchors, exercise deterministic ready ties and prove no plan or claim
mutation.

### Terminal and cancellation finding

Repeated PTY stress found an idle final frame with `q` already queued. The cause was not layout or
teardown: Crossterm's default Unix mio source could surface `SIGWINCH` before consuming terminal
input from the same readiness batch, then wait forever because edge-triggered readiness produced
no second wake-up for the queued byte. The Unix dependency now feature-unifies Crossterm 0.29 with
`use-dev-tty`, whose level-triggered poll consumes terminal input before reporting resize. A
deterministic `SIGSTOP`, queue resize plus quit, `SIGCONT` negative control reproduces the old
collision and proves clean exit. The longest graph journey passed 30 consecutive resize/quit runs
and the forced collision passed 50 consecutive runs. After the performance fixes, both journeys
passed 10 more consecutive ordinary runs. Two identical full coverage-instrumented workspace runs
then completed on 2026-10-28 with every crate at or above its committed floor and no recurrence;
`aplan-output` measured 98.0% and `aplan-tui` 92.1% in both runs.

Repository observation was another shutdown boundary: a probe could start immediately before
quit and ignore worker cancellation. Periodic and focus observations now receive the same
cancellation token as refresh, check it at identified load boundaries and use an identity-only
loader instead of constructing refresh-only verification projections. The negative control holds
an observation in flight, quits and proves cancellation precedes worker join and terminal
restoration.

### Release performance result

The A1-683 preflight subsequently found that aggregate plan/phase levels bypassed the semantic
window and sent all 68 current overview cards to the 64-node cell-scene ceiling. Raising only the
static budget was rejected: Compact still needed 290 columns, optimized output measured 131-162
ms p95 against the frozen 100 ms gate, and the TUI remained a fallback list. `A1-686` instead
applies a source-bound 10-node/20-edge aggregate window before either painter, while honoring a
tighter request and retaining exact raw-task/dependency omissions and continuations. The existing
64/96 cell-scene and milestone/task detail limits remain unchanged.

The frozen post-A1-686 command matrix then found a narrower static-selection defect before the
default decision: on the bounded aplan plan overview with Compact cards, Auto refused at 80 and
120 columns after selecting left-to-right even though the separately validated top-to-bottom
scene fit at both widths. This failed the predeclared rule that Auto must not refuse while one of
its complete candidates fits, so `A1-687` blocked `A1-683` instead of averaging those cells into
the comparison. Static Auto now retains both validated candidates through exact-width painting
and ranks only the candidates which fit. The 70-task aggregate regression passes at both widths;
the explicit left-to-right control still refuses at 80 with its exact 145-column requirement.
Application and TUI Auto selection remain independent of viewport width.

The release gate separates complete-source projection/search from bounded layout, paint, hit and
navigation. On the shared 12-core/24-thread host, without quiescing other projects, the final warm
p95 values were:

| operation | p95 | budget |
|---|---:|---:|
| construct model and graph, 10,000 tasks | 1,162.1 ms | 1,500 ms |
| task search and render, 10,000 tasks | 84.5 ms | 400 ms |
| graph search and render, 10,000 tasks | 2.6 ms | 500 ms |
| hidden-Graph resize/reflow, 10,000 tasks | 0.30 ms | 12 ms |
| sparse bounded graph projection | 123.7 ms | 250 ms |
| sparse fallback paint | 0.71 ms | 4 ms |
| sparse aggregate focus | 21.0 ms | 50 ms |
| status-only refresh, 10,000 tasks | 705.6 ms | 750 ms |
| layered sparse / fan-in / crossing layout | 3.1 / 38.6 / 3.4 ms | 50 / 100 / 50 ms |
| layered semantic hit | less than 1 microsecond | 250 microseconds |
| layered default journeys, neutral / v0.13 / whole plan | 17.1 / 81.4 / 80.3 ms | 100 ms each |

The A1-686 prerequisite whole-plan run was a clean release-binary, process-and-output measurement
at 240 columns under a 14.65 one-minute host load. Its first invocation against a fresh cache took
142.2 ms, parsed all 709 candidates and published 1,083,362 bytes; the next process reused all
candidates and parsed none. Twenty warm runs produced 9,498 bytes in Auto and 9,423 bytes
top-to-bottom. The initial candidate had deferred publication for a structural report the graph
command never computed, so repeated graph reads stayed cold. The semantic-only context now
publishes its own parsed artifacts immediately; an executable two-process test prevents the result
from depending on an unrelated `check` invocation.

A1-683 then repeated the promotion default journeys against clean release commit `6857c302`, including
process startup and output redirected to disk. Under load 11.04, 14.61, 19.74, the 20-run p95s were
17.10 ms for the neutral 80-column view, 80.82 and 81.39 ms for v0.13 at 120 and 160 columns, and
80.26 ms for the complete plan at 240. The post-promotion `1c813efe` audit restored exact
membership and reach-edge scope disclosure, then repeated the functional matrix and stability
digests against the frozen source. Its largest document was 17,140 bytes. No promotion sample or
failure was discarded; pre-public technical evaluation
contains the complete matrix, digests and blinded formative results.

The full rerun found four work leaks rather than relaxing budgets: hidden Graph reprojected during
Overview resize; fallback paint formatted every source node and recomputed its maximum width twice
per frame; aggregate selection repeated complete-source projection; and refresh rebuilt every
hidden destination. The fixes defer hidden work to route entry, cache fallback extent at projection
time, slice fallback rows before formatting, update selection-independent aggregate focus in place
and rebuild only the visible destination. During the same run fallback paint fell from 9.24 ms to
0.71 ms, graph search from about 52 ms to 2.6 ms, aggregate focus from 57.9 ms to 21.0 ms and
status refresh from 829 ms to 706 ms. No release threshold was widened.

### Evaluation boundary and remaining work

[`tui-evaluation.md`](tui-evaluation.md) freezes A1-449's candidate identity, synthetic fixtures,
fixed terminal classes, blinded Lanes/Layered crossover, neutral task wording, answer oracle,
measurement fields and predeclared failure rules. Automated actors may rehearse it only as
formative evidence. This audit does not claim human comprehension, accessibility or preference.
A1-683 used this mechanical record and that evaluation boundary to promote Layered for text while
retaining explicit Lanes compatibility. A1-449 must still execute and disposition the human
sessions; the formative automated actors are not human evidence. A1-450 still owns final
supply-chain, feature-graph and binary-size review.

## Downstream ownership

- `A1-677`: semantic levels, provenance and feedback-aware aggregate projection.
- `A1-678`: graphlay adapter, cell quantization, bounded effort, scene invariants and fallback.
- `A1-679`: Ratatui painter over cell geometry, without a minimap in the initial release.
- `A1-680`: semantic and spatial navigation, refresh history and orientation/density controls.
- `A1-681`: deterministic static CLI painter and lane compatibility spelling.
- `A1-682`: completed hostile corpus, differential checks, performance ratchets and blinded
  evaluation preparation at candidate `0d9d2a86a561d47da5725c5fa4868d75a5e5c4cb`.
- `A1-683`: passed the evidence gate, promoted the Layered text default and retained explicit
  Lanes compatibility; the audited renderer boundary is
  `1c813efe924f3453dc3ed91c3b1b1c1dcfd4d4bb`.
- `A1-686`: bounded aggregate semantic windows and graph-owned parsed-cache publication.
- `A1-687`: static Auto chooses the best validated direction which fits exact output width.
- `A1-688`: matched Lanes/Layered geometry gate and low-bend port policy.
- `A1-450`: final supply-chain, feature-graph, build and binary-size audit.

## Primary sources

- [Graphviz dot](https://graphviz.org/docs/layouts/dot/)
- [ELK Layered](https://eclipse.dev/elk/reference/algorithms/org-eclipse-elk-layered.html)
- [`ascii-dag` 0.11.0 manifest](https://docs.rs/crate/ascii-dag/0.11.0/source/Cargo.toml.orig)
- [`graphlay` 0.3.1 source](https://docs.rs/crate/graphlay/0.3.1/source/)
- [`dagre` 0.1.1](https://docs.rs/dagre/0.1.1/dagre/layout/index.html)
- [`rust-sugiyama` 0.4.0](https://docs.rs/crate/rust-sugiyama/0.4.0)
- [`gen-sugiyama` 0.5.1](https://docs.rs/gen-sugiyama/0.5.1/gen_sugiyama/)
- [`abstracttui-graph` 0.5.0](https://docs.rs/abstracttui-graph/0.5.0/abstracttui_graph/)
- [Cytoscape.js graph and viewport API](https://js.cytoscape.org/)
- [Ratatui Canvas](https://docs.rs/ratatui/0.30.2/ratatui/widgets/canvas/struct.Canvas.html)
- [Crossterm Unix mio event source](https://github.com/crossterm-rs/crossterm/blob/0.29.0/src/event/source/unix/mio.rs)
- [Crossterm Unix `/dev/tty` event source](https://github.com/crossterm-rs/crossterm/blob/0.29.0/src/event/source/unix/tty.rs)
