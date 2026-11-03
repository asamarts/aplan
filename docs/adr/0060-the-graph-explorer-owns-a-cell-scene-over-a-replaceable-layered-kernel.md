# ADR-0060: The graph explorer owns a cell scene over a replaceable layered kernel

Date: 2026-10-27
Status: Accepted
Tasks: `A1-676`

## Context

ADR-0048 retained aplan's lane renderer until a released native candidate met Rust and terminal
compatibility requirements and real use showed that lanes could not express a needed view. Both
revisit conditions now hold. The Graph destination's bounded lane document is truthful but does
not provide readable dependency ranks, spatial node cards, crossing reduction, compound phase and
milestone views or spatial navigation. The complete current plan and a bounded high-fan fixture
also exceed its lane-width limit.

The current candidate audit compared exact releases at Rust 1.88 and measured the lane renderer,
a small aplan-owned ranked prototype and graphlay on the real plan and hostile fixtures. Detailed
inputs, commands and measurements are in `docs/design/graph-layout-evaluation.md`.

## Decision

**1. Aplan owns graph meaning, scene identity and terminal-cell validity.** The application
projection owns semantic level, nodes, edges, grouping, aggregate provenance, omissions and
continuations. A typed cell scene owns boxes, ranks, ports, routes, bounds, hit regions,
degradation and quality metrics. Neither contains rendered text or requires Ratatui.

**2. A replaceable graphlay adapter supplies layered geometry.** `A1-678` may admit exact graphlay
0.3.1 with default features disabled. The adapter uses stable semantic keys, strictly bounded
inputs and effort, an internal geometry scale and a validated cell quantization pass. Graphlay
does not own selection, projection, grouping meaning, Unicode labels, interaction, painting or
fallback policy.

**3. Live layout operates only on bounded semantic projections.** Plan overview lays out milestone
aggregates. Milestone and task detail lay out bounded task windows with exact continuations. The
complete task source is never laid out in response to interactive paint, pan or selection.

**4. Aggregate cycles are layout feedback, not plan invalidity.** Collapsing an acyclic task graph
into milestone aggregates can produce a cyclic quotient. The projection retains exact underlying
edge provenance and the scene marks feedback routing. Only the canonical task graph owns cycle
diagnostics.

**5. Both product painters consume the same cell scene.** Ratatui paints an interactive viewport;
the CLI paints a complete deterministic static scene or reports exact omissions. Hit targets come
from the geometry that is painted. The current lane scene remains a separate compatibility path
through the evaluation and rollout in `A1-683`.

**6. Runtime subprocess layout is rejected.** Graphviz and ELK remain reference and export tools,
not live dependencies. Their installation, startup, trust, portability, cancellation and failure
boundaries conflict with the one-native-binary product. No browser or JavaScript runtime is added.

**7. The first layered release has no minimap.** In the measured wide prototype, reserving a
26-column semantic map reduced none of the predeclared modeled navigation action counts and
reduced useful cards by 25 percent on crossing-heavy fixtures. This was an interaction-model and
capacity screen, not an observed-user result. Breadcrumbs, extents, directional counts, search,
focus and recenter provide orientation without that permanent cell cost.

## Consequences

- The semantic projection, navigation and painters survive a future kernel replacement.
- A new normal dependency enters only when `A1-678` implements and tests behavior that uses it;
  this decision does not add unused production code.
- The adapter must turn resource limits, graphlay failures and quantization failures into typed
  degradation with an honest usable fallback. It may not silently omit geometry.
- Graphlay has no cooperative cancellation token. Work is bounded outside paint and tagged by
  generation; stale completion is discarded. A measured over-budget bounded call is a failure to
  fix or degrade, not permission to freeze input.
- Identical input must remain byte-stable. Status-only refresh reuses geometry; structural refresh
  measures displacement and preserves the selected anchor where possible rather than promising
  that graphlay seeds never move nodes.
- `A1-450` re-audits the exact supply chain, feature graph and binary cost before public release.

## Alternatives

| option | why not |
|---|---|
| Keep lanes as the only graph | Real and high-fan windows exceed its width model, and row lanes cannot express multiscale spatial hierarchy |
| Build the complete layered kernel in aplan now | The prototype loses badly on crossings and lacks orthogonal routing, compounds, feedback handling, stability and metrics; completing it recreates a general engine |
| `ascii-dag` 0.11.0 | Its declared Rust 1.92 fails the workspace floor, and aplan still needs its own semantic projection and painter |
| `rust-sugiyama` or `gen-sugiyama` | Useful algorithmic prior art but missing the selected compound, orthogonal route, incremental and terminal adapter surface |
| `dagre` 0.1.1 | The no-default exact build fails and the young archived-lineage port has no terminal or incremental contract |
| `abstracttui-graph` | Imports a second TUI/application framework across the Ratatui and aplan-owned application boundary |
| Graphviz, ELK or JavaScript at runtime | Adds an external runtime and process/trust/failure boundary to a single-binary interactive path |
| Permanent wide minimap | The measured prototype loses useful cards without reducing the modeled action count; observed evaluation may revisit it later |
