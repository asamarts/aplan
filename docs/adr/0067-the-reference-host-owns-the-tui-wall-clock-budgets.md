# ADR-0067: The reference host owns the TUI's wall-clock budgets

Date: 2026-11-18

Status: Accepted

Supersedes nothing. Keeps ADR-0065: no runner is registered for this repository. Applies the
measurement contract of `docs/design/read-path-performance.md`, which already gave the wall-time
release budget to a stable reference host, to the TUI's three release gates from `A1-447`.

## Context

The TUI has three wall-clock gates, all ignored in ordinary tests:
- the in-process render gate;
- the layered-graph gate;
- the real-terminal startup and interaction gate.

The scheduled `scale` workflow runs them on a GitHub-hosted runner. Their budgets were set on
2026-10-27 from runs on a shared host, at about twice the largest observation.

From 2026-11-11 to 2026-11-14 the scheduled run failed four times with no source change. Each time
one metric, `status_only_refresh_10000` or `model_and_graph_10000`, crossed its budget by a few
percent. On those days every in-process metric was about 35% slower than on the days before,
uniformly. That is the spread between the machines GitHub assigns, not a regression (`A2-164`).

A budget that a slower machine exceeds by chance makes the run red for no reason in aplan, and
hides a real failure among the false ones. A budget wide enough for every machine in the fleet
misses a regression smaller than the fleet's spread.

The first remedy considered was to calibrate each run, scaling the budgets by a reference workload
timed on the same machine. It was dropped. To tell a slower machine from a regression, the workload
would have to measure the machine's speed more precisely than the 35% it varies by, and no workload
was shown to.

## Decision

**1. The budgets are measured on the reference host.** The reference host is a quiet machine that
runs one benchmark at a time through a queue. `ci/scripts/reference-tui-perf.sh` does the whole
run in one slot of that queue:
- builds the gates in release;
- waits until the one-minute load is at or under 0.25 before each gate;
- runs each gate pinned to one CPU.

The pinned Rust release is the one `rust-toolchain.toml` names. Two settings in the operator's
environment name the host and its queue; this repository does not.

**2. Budgets come from five runs, for each metric whose runs agree.** Five runs with `--budgets
measure`, each in a slot of its own, give the budgets through `ci/scripts/reference-tui-budgets.py`.
A metric's runs agree when its p95 spreads across them by at most 5% of the smallest, or by at
most 0.1 ms for a metric whose worst p95 is under 2 ms. These numbers were agreed before anything
was measured.

Its budget is then the worst p95 times 1.25, and at least the worst p95 plus 0.1 ms. The floor was
added after the first measurement, on 2026-11-19. Without it, a hit test of about 100 ns would
have had a budget of 125 ns, which an unrelated change in code layout can exceed.

A metric whose runs do not agree gets no reference budget. The reference host holds it to the
hosted rule of decision 3 until it can be measured there (`HOSTED_RULE_ON_REFERENCE`, `A2-180`).
The maintainer decided this on 2026-11-19 for the six metrics that did not agree in the first
measurement.

The same day, the maintainer extended it to every real-terminal first feedback. Each is about
7 ms and the slowest of three launches, and one that agreed failed a passing revision once in
three reference runs (`HOSTED_RULE_BY_DECISION` in the script). The script exits non-zero
whenever any other metric does not agree. So a later re-basing that leaves metrics on the hosted
rule is a decision someone makes, not a default.

The budgets are kept in nanoseconds beside each gate (`REFERENCE_BUDGETS_NS`), because some
metrics take a few nanoseconds per operation. `docs/design/read-path-performance.md` records the
runs and each budget's basis.

Two further runs prove the budgets before they are relied on:
- a run with `--budgets reference` passes;
- a deliberate doubling of `status_only_refresh_10000` and `model_and_graph_10000` fails.

**3. A hosted run fails only a gross regression.** Without `APLAN_TUI_BUDGETS`, a gate holds each
wall-clock metric to 1.5 times its hosted budget of 2026-11-18. The hosted machines' spread stays
inside that, and a several-fold regression does not.

Resident memory, the ratio between the 32-row and the 10,000-row list paint, and the semantic and
work-count assertions are not wall-clock. Every host holds them to the same limits as before.

Every gate now prints every metric before it fails, and names every metric past its budget. One
slow metric no longer hides the ones measured after it.

**4. The reference gate runs before a release and for a change to the TUI's render or model
code.** The release task requires it to pass on the release revision. It is reached by a script,
not by a workflow, so ADR-0065 stands.

**5. A new reference host, or a new pinned compiler, is a re-basing.** The budgets belong to one
host, one CPU and one compiler. Changing any of them takes five new agreeing runs, recorded as in
decision 2, never an edit of the numbers.

## Consequences

- A regression of more than a quarter, or of 0.1 ms for the smallest metrics, fails the reference
  gate. Before, a regression had to roughly double a metric to fail, or happen to land on a slow
  hosted machine. Nine metrics are still judged only by the hosted rule until `A2-180`.
- The hosted run can no longer go red on a slow machine. It also no longer catches a regression of
  less than about half until someone runs the reference gate. That run is required before a
  release, so a regression cannot ship unmeasured.
- Only someone with access to the reference host can run the reference gate. A contributor's
  change to the TUI is measured by a maintainer before release.
- The budgets describe one machine. Losing it means re-basing on another, as decision 5 says, not
  carrying its numbers over.

## Alternatives considered

| Alternative | Disposition |
|---|---|
| Calibrate each hosted run by a reference workload | Rejected: the workload would have to be more precise than the 35% fleet spread, and none was shown to be. |
| Widen the hosted budgets until every machine passes | Rejected as the only gate: it misses every regression smaller than the fleet's spread. Kept, at 1.5 times, as the hosted run's check for gross ones. |
| Register a self-hosted runner for the reference host | Rejected by ADR-0065, and a workflow on a shared benchmark host would bypass its queue. |
| Pin the hosted runner class more narrowly | Not available: GitHub assigns the machine within a class. |

## References

- `A2-164`, which measured the hosted failures and delivered this decision.
- `A1-447`, which introduced the three gates and their first budgets.
- `docs/design/read-path-performance.md`, "Requirement and measurement contract".
- `docs/design/tui.md`, "Release performance budgets".
