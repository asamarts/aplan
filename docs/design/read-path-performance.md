# Read-path performance and exact incremental snapshots

Status: Accepted for phased implementation; A1-588, A1-322, A1-589, A2-135 and A2-136 delivered

Owning tasks: `A1-588`, `A1-322`, `A1-589`, `A1-590`-`A1-594`, `A2-135`, `A2-136`

Date: 2026-10-18

## Decision in one page

Ordinary aplan reads should feel immediate. The target is an end-to-end p95 below 100 ms for a
bounded human or machine response over a current local plan, including process startup, plan and
configuration discovery, validation, projection and rendering. The target applies to both the
installed optimized binary and the ordinary development binary. It is not met today.

The fix is a sequence, not one cache bolted around the current path:

1. Remove work that is already known to be redundant: retain source bytes only when a caller needs
   them, index `PlanContext` inputs instead of searching them once per task, reuse a selected
   configuration within the same repository frame, build graph state once per operation, parse each
   traced document once, and make `vacuity` observe rule activation in one validation pass. Trace
   parsing has landed in `A1-322`; vacuity remains a separate task so its correctness and
   performance evidence cannot hide behind the baseline refactor.
2. Introduce one shared `PlanStore` that owns an immutable plan snapshot and provides a
   per-operation view that builds the existing borrowing `Graph` once for validation and report
   projections. CLI, MCP and TUI must ask this boundary rather than each reconstructing a
   plan-shaped context.
3. Back `PlanStore` with a disposable, versioned, content-validated index. It stores parsed task
   records and exact-keyed typed structural reports, not authority or rendered answers. Markdown
   and configuration remain the only source of truth. A miss or corrupt index falls back to the
   ordinary loader and can never change a verdict.
4. Separate plan inspection from refresh of arbitrary evidence providers. `show` must consult a
   current, snapshot-bound evidence observation without launching opaque commands on its default
   latency path. If no sound current observation exists, it must finish promptly and say
   `not_observed` or `stale`; it must never call stale data resolved. An explicit refresh operation
   may wait, show progress and populate the observation for later reads.
5. Gate the work performed as well as measuring wall time. Wall-clock samples prove the user
   experience; deterministic counters prove that a shared runner did not hide a regression.

A persistent daemon, SQLite, Salsa and a new binary serialization dependency are not required for
the first implementation. A single atomically replaced JSON index reuses dependencies the workspace
already carries and is enough to test the design. Its internal format is deliberately not a public
contract. Admit a different encoding, a file watcher or a daemon only when the benchmark matrix
shows the simpler design missing its budget.

An earlier architecture proposal deferred both a serialized cache and a daemon until file watching,
invalidation and lifecycle ownership were demonstrably needed. The measurements below are evidence
for a disposable cache: optimized structural reads already exceed the interaction budget,
development reads exceed it by multiples, and every new process repeats unchanged parsing. They are
not evidence for a daemon, so that half of the earlier decision remains deferred. Acceptance of this
proposal produced ADR-0049 for cache authority/location/invalidation and ADR-0050 for its measured
typed-report refinement. The observable `show` evidence-state change still requires its own ADR
before implementation; existing decisions change only through supersession.

There is one hard limit the interface must state honestly: a fresh invocation cannot both execute
an arbitrary configured program and guarantee completion in 100 ms. Such a program can compile,
contact a network, wait on a lock or never return. The current repository's test listing alone takes
seconds. The fast default therefore means "read an exact current observation if one exists, otherwise
report that resolution is not current". It cannot mean "silently weaken ambiguity checks" or
"declare an old listing current".

## Requirement and measurement contract

"Under 100 ms" is meaningless without the state, corpus, output and percentile. Use these classes:

| Class | Examples | Required contract |
|---|---|---|
| Local bounded read | `ready`, `why`, `due`, `tree`, scoped `graph`, `summary`, `critical-path`, `roadmap`, `deprecations`, `version ID` | current plan, p95 below 100 ms |
| Full structural read | `check`, `show --no-resolve`, `vacuity`, configured-document checks | index hit or one ordinary file changed, p95 below 100 ms |
| Evidence-backed inspection | default `show` | task plus current/stale/not-observed evidence state, p95 below 100 ms; no provider process on the latency path |
| Repository read | `paths`, `stale`, `summary --at`, bare `version`, commit/file evidence | plan overhead below 100 ms; separate budget for bounded Git work |
| External refresh | `verify` when it lists/runs tests, URL checking, version commands, explicit evidence refresh | no 100 ms completion claim; bounded, cancellable, observable and reusable |
| Output-dominated read | an unscoped 10,000-row table or graph | first bounded response below 100 ms, then streaming; output transfer is reported separately |

The release corpus has these required states:

- clean checkout with no index;
- unchanged plan with a valid index;
- one task added, changed, renamed or removed;
- a shared trace document or relevant configuration changed.
- an unreadable, corrupt or unavailable cache location, proving caching is not required.

Report p50, p95, maximum, CPU time, bytes read, files statted, files hashed, task files parsed, trace
documents parsed, provider processes spawned, bytes rendered and time to first byte. Run the matrix
with release and development profiles on a named local SSD host and, when available, a standard
GitHub-hosted runner. Record host load and retain the best-of-N diagnostic number beside the
percentile; neither number may be substituted for the other. The stable reference host owns the
wall-time release budget. Hosted-runner times are published observations, not a flaky CI threshold;
deterministic work counts are the portable gate.

The initial release budgets should be:

- the current aplan plan and the representative 1,000-task corpus: all local bounded and structural
  reads below 100 ms p95 on an exact index hit and after one ordinary task edit;
- the 10,000-task corpus: bounded reads below 100 ms p95 on an index hit, with a separately published
  clean-checkout parse budget;
- no local bounded read may parse unchanged YAML, parse one trace document more than once, or spawn a
  configured command;
- `A2-136` (superseding `A1-594`) established the **256 KiB** (`BOUNDED_RESPONSE_THRESHOLD_BYTES`)
  bounded-response threshold and **16 KiB** (`STREAM_BUFFER_CAPACITY`) streaming buffer from
  measured human, JSON, Markdown/graph, throttled-pipe and MCP protocol behavior. Larger outputs
  begin promptly and stream with O(1) output buffering.

The existing HLD sentence, "`check` on 10,000 tasks in under 100 ms, cold", is not currently an
enforced or achieved contract. The existing scale gate intentionally ratchets complexity ratios,
not milliseconds, and its own recorded 6,000-task load phase is about 0.9 to 1.1 seconds before
validation. Replace the ambiguous word `cold` with the states above when this proposal is accepted;
do not lower a target silently or continue publishing one the product does not meet.

### The TUI's wall-clock budgets on the reference host (`A2-164`)

The TUI's three release gates (the in-process render gate, the layered-graph gate and the
real-terminal gate of `A1-447`) hold their wall-clock metrics to budgets measured on the reference
host (ADR-0067). The hosted `scale` run holds them only to 1.5 times their hosted budgets, the
ceilings `docs/design/tui.md` lists, because GitHub's machines differ by about 35%.

**The host and the setting.** The reference host has a four-core, eight-thread Intel Core
i7-6700HQ at 2.6 GHz, with turbo off and the performance governor. It runs one benchmark at a time
through a queue. `ci/scripts/reference-tui-perf.sh` builds the gates in release with Rust 1.99.1,
inside one slot of that queue. Before each gate it waits for the one-minute load to fall to 0.25
or less, then runs the gate pinned to CPU 2. It prints every metric's p95: the slowest of seven
samples in process, or of three launches in a real terminal. The real terminal's first feedback,
usable frame, action, search and refresh are separate metrics, each cold and warm, for 32, 700 and
10,000 tasks.

**The runs.** Five runs with `--budgets measure`, each in its own slot, measured source
`58025c81b7ad-0d81d7890265-32f79219ed53` on 2026-11-19 between 06:00 and 06:54 UTC. That id names
the trees of `crates/`, `Cargo.toml` and `Cargo.lock`. The load before each gate was 0.20 to 0.25,
and after each run 0.60 to 0.74.

`ci/scripts/reference-tui-budgets.py` compared the runs:
- 68 of the 74 wall-clock metrics agreed within the spread agreed in advance: 5% of the smallest
  p95, or 0.1 ms for a metric whose worst p95 is under 2 ms.
- 65 of them have a reference budget: the worst p95 times 1.25, and at least the worst p95 plus
  0.1 ms. The maintainer decided the floor on 2026-11-19, after this measurement. It sets the
  budget of ten metrics under 0.4 ms, among them hit tests of 76 to 126 ns and a 9 ns progress
  tick, which an unrelated change in code layout could otherwise fail.
- Nine metrics keep the hosted rule on the reference host too, by the maintainer's decisions of the
  same day, until `A2-180` makes them measurable:
  - Three graph-focus metrics vary up to 11% between runs but agree to 1% within one, which points
    at something fixed per process.
  - All six real-terminal first-feedback metrics. Each is about 7 ms and the slowest of three
    launches. Three of them varied by up to 6.4%, and one that agreed failed a passing revision
    once (see the proof below).

The two metrics whose hosted budgets failed the scheduled run from 2026-11-11 to 2026-11-14
agreed well:

| metric | p95 spread across five runs | reference budget |
|---|---:|---:|
| `model_and_graph_10000` | 1.4% | 2.075 s |
| `status_only_refresh_10000` | 2.7% | 1.154 s |

The reference host is slower than most hosted runners at some of these. So a reference budget
can be above the hosted one, and only the reference host's own runs can set it.

**The proof.** Four runs with `--budgets reference` followed, each in its own slot:
- **The first run, on the commit that added the budgets, failed one metric.**
  `pty_small_cold_feedback` read 9.70 ms against its 9.17 ms budget. One of its three launches was
  slow, and the others took about 7.3 ms. That is why every first feedback keeps the hosted rule.
- **The first doubling run passed, which exposed a bug in the script.** The doubling commit did the
  measured work of `model_and_graph_10000` and `status_only_refresh_10000` twice. `git archive`
  stamps files with their commit's time, and that commit was older than the previous run's build,
  so cargo measured the previous run's binary. The script now extracts with the time of
  extraction.
- **With that fix, the commit with the budgets passed twice.** The doubling commit failed exactly
  the two doubled metrics, at 3.26 s against 2.075 s and at 1.85 s against 1.154 s, and still
  printed all 80 metrics.

**Re-basing.** A new host, or a new pinned compiler, takes five new runs and the same comparison,
never an edit of the numbers.

In-process gates:

| metric | p95, fastest to slowest run | spread | reference budget |
|---|---:|---:|---:|
| `layered_cell_scene_layered_sparse` | 488.8 us to 490.5 us | 0.002 ms | 613.1 us |
| `layered_hit_layered_sparse` | 81 ns to 87 ns | 0.000 ms | 100.1 us |
| `layered_cell_scene_layered_fan_in` | 583.6 us to 588.1 us | 0.004 ms | 735.1 us |
| `layered_hit_layered_fan_in` | 76 ns to 76 ns | 0.000 ms | 100.1 us |
| `layered_cell_scene_layered_crossing` | 781.9 us to 784.2 us | 0.002 ms | 980.2 us |
| `layered_hit_layered_crossing` | 93 ns to 99 ns | 0.000 ms | 100.1 us |
| `list_paint_small` | 511.9 us to 512.8 us | 0.001 ms | 641.0 us |
| `list_paint_repository` | 504.7 us to 507.1 us | 0.002 ms | 633.9 us |
| `model_and_graph_10000` | 1.638 s to 1.660 s | 1.4% | 2.075 s |
| `list_paint_10000` | 511.0 us to 513.4 us | 0.002 ms | 641.7 us |
| `search_and_render_10000` | 121.95 ms to 123.68 ms | 1.4% | 154.60 ms |
| `graph_search_and_render_10000` | 636.7 us to 656.5 us | 0.020 ms | 820.6 us |
| `summary_resize_and_reflow_10000` | 574.5 us to 589.1 us | 0.015 ms | 736.4 us |
| `graph_window_sparse` | 180.47 ms to 181.67 ms | 0.7% | 227.09 ms |
| `graph_scene_sparse` | 622.8 us to 632.0 us | 0.009 ms | 790.0 us |
| `graph_paint_sparse` | 519.9 us to 522.1 us | 0.002 ms | 652.7 us |
| `graph_pan_sparse` | 1.23 ms to 1.24 ms | 0.016 ms | 1.55 ms |
| `graph_hit_sparse` | 80 ns to 85 ns | 0.000 ms | 100.1 us |
| `graph_focus_sparse` | 17.19 ms to 19.10 ms | 11.1% | hosted rule (`A2-180`) |
| `graph_window_traversal_sparse` | 107.02 ms to 108.25 ms | 1.1% | 135.31 ms |
| `graph_window_fan_in` | 19.15 ms to 19.28 ms | 0.7% | 24.10 ms |
| `graph_scene_fan_in` | 487.7 us to 503.3 us | 0.016 ms | 629.2 us |
| `graph_paint_fan_in` | 527.0 us to 528.0 us | 0.001 ms | 660.0 us |
| `graph_pan_fan_in` | 1.26 ms to 1.33 ms | 0.071 ms | 1.66 ms |
| `graph_hit_fan_in` | 80 ns to 85 ns | 0.000 ms | 100.1 us |
| `graph_focus_fan_in` | 1.62 ms to 1.83 ms | 0.215 ms | hosted rule (`A2-180`) |
| `graph_window_traversal_fan_in` | 10.18 ms to 10.22 ms | 0.3% | 12.77 ms |
| `graph_window_crossing` | 57.86 ms to 58.45 ms | 1.0% | 73.06 ms |
| `graph_scene_crossing` | 1.41 ms to 1.44 ms | 0.023 ms | 1.80 ms |
| `graph_paint_crossing` | 535.4 us to 539.4 us | 0.004 ms | 674.2 us |
| `graph_pan_crossing` | 1.28 ms to 1.31 ms | 0.024 ms | 1.63 ms |
| `graph_hit_crossing` | 121 ns to 126 ns | 0.000 ms | 100.1 us |
| `graph_focus_crossing` | 3.92 ms to 4.28 ms | 9.2% | hosted rule (`A2-180`) |
| `graph_window_traversal_crossing` | 21.76 ms to 22.02 ms | 1.2% | 27.52 ms |
| `graph_window_too_wide` | 10.42 ms to 10.48 ms | 0.5% | 13.10 ms |
| `graph_scene_too_wide` | 385.7 us to 392.4 us | 0.007 ms | 492.4 us |
| `graph_paint_too_wide` | 531.5 us to 533.4 us | 0.002 ms | 666.7 us |
| `graph_pan_too_wide` | 1.27 ms to 1.28 ms | 0.008 ms | 1.60 ms |
| `graph_hit_too_wide` | 80 ns to 85 ns | 0.000 ms | 100.1 us |
| `graph_focus_too_wide` | 764.5 us to 859.0 us | 0.095 ms | 1.07 ms |
| `graph_window_traversal_too_wide` | 5.48 ms to 5.56 ms | 1.4% | 6.95 ms |
| `status_only_refresh_10000` | 898.69 ms to 923.27 ms | 2.7% | 1.154 s |
| `loading_progress_paint` | 337.7 us to 339.0 us | 0.001 ms | 439.0 us |
| `loading_progress_tick` | 9 ns to 9 ns | 0.000 ms | 100.0 us |

Real-terminal gate:

| metric | p95, fastest to slowest run | spread | reference budget |
|---|---:|---:|---:|
| `pty_small_cold_feedback` | 7.20 ms to 7.34 ms | 1.9% | hosted rule (`A2-180`) |
| `pty_small_cold_usable` | 308.71 ms to 310.20 ms | 0.5% | 387.75 ms |
| `pty_small_cold_action` | 1.99 ms to 2.08 ms | 4.7% | 2.60 ms |
| `pty_small_cold_search` | 16.48 ms to 17.23 ms | 4.6% | 21.54 ms |
| `pty_small_cold_refresh` | 102.15 ms to 103.94 ms | 1.8% | 129.92 ms |
| `pty_small_warm_feedback` | 7.20 ms to 7.32 ms | 1.6% | hosted rule (`A2-180`) |
| `pty_small_warm_usable` | 308.89 ms to 310.27 ms | 0.4% | 387.83 ms |
| `pty_small_warm_action` | 1.99 ms to 2.03 ms | 2.1% | 2.54 ms |
| `pty_small_warm_search` | 17.02 ms to 17.51 ms | 2.9% | 21.88 ms |
| `pty_small_warm_refresh` | 102.14 ms to 102.19 ms | 0.1% | 127.74 ms |
| `pty_repository_cold_feedback` | 6.89 ms to 7.26 ms | 5.4% | hosted rule (`A2-180`) |
| `pty_repository_cold_usable` | 423.29 ms to 424.21 ms | 0.2% | 530.26 ms |
| `pty_repository_cold_action` | 5.20 ms to 5.30 ms | 2.1% | 6.63 ms |
| `pty_repository_cold_search` | 52.17 ms to 52.94 ms | 1.5% | 66.18 ms |
| `pty_repository_cold_refresh` | 206.97 ms to 207.38 ms | 0.2% | 259.23 ms |
| `pty_repository_warm_feedback` | 6.84 ms to 7.25 ms | 5.9% | hosted rule (`A2-180`) |
| `pty_repository_warm_usable` | 323.59 ms to 323.96 ms | 0.1% | 404.95 ms |
| `pty_repository_warm_action` | 5.17 ms to 5.31 ms | 2.6% | 6.63 ms |
| `pty_repository_warm_search` | 51.32 ms to 52.57 ms | 2.4% | 65.72 ms |
| `pty_repository_warm_refresh` | 207.03 ms to 208.56 ms | 0.7% | 260.70 ms |
| `pty_large_cold_feedback` | 6.90 ms to 7.35 ms | 6.4% | hosted rule (`A2-180`) |
| `pty_large_cold_usable` | 2.335 s to 2.451 s | 5.0% | 3.064 s |
| `pty_large_cold_action` | 56.97 ms to 59.62 ms | 4.6% | 74.52 ms |
| `pty_large_cold_search` | 646.49 ms to 669.31 ms | 3.5% | 836.64 ms |
| `pty_large_cold_refresh` | 2.683 s to 2.789 s | 4.0% | 3.487 s |
| `pty_large_warm_feedback` | 7.08 ms to 7.26 ms | 2.6% | hosted rule (`A2-180`) |
| `pty_large_warm_usable` | 1.937 s to 1.945 s | 0.4% | 2.432 s |
| `pty_large_warm_action` | 56.94 ms to 58.53 ms | 2.8% | 73.16 ms |
| `pty_large_warm_search` | 630.66 ms to 654.71 ms | 3.8% | 818.38 ms |
| `pty_large_warm_refresh` | 2.087 s to 2.088 s | 0.0% | 2.610 s |

## What the implementation did at proposal time

The pre-A1-588 one-shot path was approximately:

```text
argv/clap
  -> discover and parse configuration to find the plan root
  -> walk every Markdown path under the root
  -> read every candidate in full
  -> derive and compile the JSON Schema validator
  -> parse each frontmatter as YAML -> JSON
  -> validate the JSON value against the schema
  -> parse the same YAML again into Task
  -> scan the body for criteria and field locations
  -> often retain every candidate's bytes
  -> often rediscover and parse configuration
  -> build Graph and/or PlanContext
  -> validate, reopening traced documents per trace
  -> project and buffer output
  -> for default show, run repository and configured evidence providers
  -> render anything visible
```

The measurement snapshot had 610 tasks in 611 Markdown files and about 2.74 MB of Markdown. There
were 44 non-empty trace references into 15 distinct documents. The live plan has since grown, so
future reports record their exact task/file/byte counts instead of treating these numbers as a
fixture identity. This is not a large plan; it is a useful release corpus precisely because an
interactive read is already visibly slow at this size.

### Measurements

Measurements were taken on 2026-10-18 from the same checkout. The host was concurrently running
other Rust, WebAssembly and Python work, so means and wall-time variance are not portable budgets.
CPU time, syscall counts, minima and the relative shapes still identify real work.

| Command | optimized binary | development binary |
|---|---:|---:|
| `ready` | about 65 ms | 357 ms |
| `why A1-588` | about 56 ms | 344 ms |
| `show A1-587 --no-resolve` | 180-390 ms across samples | 1.69 s |
| `check` | 239-254 ms | 4.08 s |
| `check --no-traces` | 110-155 ms | 0.67 s in a later sample |
| `vacuity` | about 461 ms | not rerun; its optimized result already fails the target |

The default development `show A1-587` was separately observed at 7.2 seconds with redirected output
and 11.1 seconds through a PTY. It emitted nothing while waiting. The delay was evidence resolution,
not a terminal deadlock: its two test citations caused all twelve configured Cargo runners to list
their tests before ambiguity could be excluded.

Scale probes separated fixed startup from per-file work:

| `ready --no-config` corpus | optimized | development |
|---|---:|---:|
| empty | 12 ms | 35 ms |
| one file | 12 ms | 29 ms |
| 100 copied task files | 20 ms | 79 ms |
| current plan | 64 ms | 379 ms |

`strace -c` found about 4,118 syscalls for `ready`, 7,372 for `show --no-resolve` and 8,033 for
`check`. The latter two made roughly 1,930 `read` calls, 1,750 `statx` calls, 970 `openat` calls and
1,170 mostly unsuccessful `readlink` calls. The system-call time was only about 11-32 ms; most of the
CPU cost is parsing, validation, path handling and derived work above the kernel.

### A1-588 measured delivery baseline

`ci/scripts/read-performance.py` is the committed command harness. It launches the real subprocess,
drains both streams without deadlock, measures startup-through-exit min/p50/p95/max wall and child CPU
time, timestamps the first stdout byte, hashes and counts stdout, preserves per-phase timing medians, parses deterministic
`--timings` work, and refuses nonzero commands or samples whose output or work changes. It fingerprints
the complete Git source state and executable before and after sampling, so a concurrent edit or rebuild
cannot produce a mixed-state report. It also terminates a timed-out sample's process group instead of
leaving descendants alive. Its generated corpus
uses 1,000 independent task files. Run it once for each freshly built profile:

```sh
python3 ci/scripts/read-performance.py --binary target/release/aplan --profile release
python3 ci/scripts/read-performance.py --binary target/debug/aplan --profile development-opt1
```

The following 20-sample run was recorded on `kbox` from clean revision `cb6bfa49647ceda87a57051d45adfc9aaa9130e0`
after two warm-ups. The release executable was 16,615,848 bytes with SHA-256
`070b66603fe6c464c2490925ddeb4ecb6caa81d9368f70449ce50d215a407395`; the development executable
was 59,474,824 bytes with SHA-256 `4c4374ca06fb153c07f4e2b9dec0c62c02393540895503377434f28bed12a09e`.
The machine was deliberately not represented as quiet: its recorded one-minute load average was
18.19-19.14 while other work ran. Those wall percentiles are therefore observations, not a release
gate. CPU and deterministic work still describe the process, and the harness records the host, load,
binary hash, source state and maximum in its full report. A dirty run is marked `+dirty` so it cannot
masquerade as its base revision.

| corpus / command | release min / p50 / p95 | development min / p50 / p95 | release CPU p50 | development CPU p50 |
|---|---:|---:|---:|---:|
| current / `ready` | 55.4 / 60.7 / 68.4 ms | 68.4 / 75.9 / 102.4 ms | 59.2 ms | 73.6 ms |
| current / `why A1-588` | 51.9 / 61.4 / 69.0 ms | 68.7 / 80.1 / 96.3 ms | 59.7 ms | 76.9 ms |
| current / `show A1-588 --no-resolve` | 163.9 / 171.5 / 197.8 ms | 270.5 / 326.7 / 361.9 ms | 169.5 ms | 319.6 ms |
| current / `check --no-traces` | 82.3 / 91.2 / 97.2 ms | 193.3 / 208.0 / 224.1 ms | 90.2 ms | 202.9 ms |
| current / `check` | 181.7 / 203.5 / 212.1 ms | 354.5 / 427.4 / 754.8 ms | 201.5 ms | 417.1 ms |
| generated 1,000 / `why` | 27.7 / 29.1 / 31.0 ms | 35.5 / 38.9 / 50.7 ms | 28.2 ms | 38.2 ms |
| generated 1,000 / `show --no-resolve` | 35.3 / 36.5 / 40.0 ms | 46.6 / 56.1 / 121.8 ms | 35.7 ms | 54.7 ms |
| generated 1,000 / `check` | 33.0 / 35.3 / 37.7 ms | 43.7 / 46.6 / 108.8 ms | 34.3 ms | 44.4 ms |

The same samples attribute the internal CLI median. Each cell is release/development milliseconds;
configuration was below 1 ms on the current plan and rounded to zero with `--no-config`:

| corpus / command | load | validate | project | render | unattributed |
|---|---:|---:|---:|---:|---:|
| current / `ready` | 48.4 / 59.7 | 0.0 / 0.0 | 3.3 / 4.6 | 0.2 / 0.3 | 2.1 / 2.6 |
| current / `why A1-588` | 51.4 / 68.0 | 0.0 / 0.0 | 0.5 / 0.8 | 0.0 / 0.1 | 2.4 / 2.9 |
| current / `show A1-588 --no-resolve` | 43.9 / 71.6 | 111.8 / 218.0 | 9.0 / 17.9 | 0.1 / 0.2 | 1.4 / 3.4 |
| current / `check --no-traces` | 45.6 / 71.2 | 13.4 / 26.4 | 24.8 / 96.3 | 0.0 / 0.1 | 1.7 / 3.5 |
| current / `check` | 46.7 / 83.4 | 114.2 / 232.0 | 27.1 / 106.8 | 0.0 / 0.1 | 2.1 / 4.1 |
| generated 1,000 / `why` | 23.0 / 30.9 | 0.0 / 0.0 | 0.2 / 0.3 | 0.0 / 0.0 | 1.0 / 1.6 |
| generated 1,000 / `show --no-resolve` | 22.9 / 32.8 | 3.4 / 6.1 | 4.3 / 8.1 | 0.0 / 0.1 | 1.1 / 1.9 |
| generated 1,000 / `check` | 22.9 / 29.6 | 3.7 / 5.5 | 2.1 / 2.6 | 0.0 / 0.1 | 1.2 / 1.9 |

Every row loaded one configuration, loaded the plan once and built one graph. Current-plan local reads
read 617 candidates / 2,764,027 bytes and parsed 616 tasks. `ready`, `why` and body-free `show` retained
zero source files; `check` retained the identified input set because its shared application response
carries an exact snapshot identity. Generated rows read and parsed exactly 1,000 candidates. No row
spawned a provider. Output bytes ranged from 134 to 19,593 across these bounded rows;
TTFB tracked completion for buffered responses, which is evidence for `A1-594` rather than a reason to
pretend output cost is parsing cost.

The baseline counters covered selected configuration loads, plan candidate reads/bytes, task parses,
retained input bytes and graph constructions. `A1-322` added successful trace reads and bytes plus
Markdown and structured parse attempts. They do not pretend that filesystem stats, content hashes,
provider spawns or rendered bytes already have counters at the shared store boundary. `A1-589`,
`A1-590`, `A1-594` and the final ratchet in `A1-591` extend this same diagnostic and harness with
those counters; actual stdout bytes already come from the harness rather than a self-reported
rendering estimate.

The current plan is slower than the generated corpus despite having fewer files because its 2.76 MB of
real bodies, policy, edges and traces exercise substantially more semantic work. Both `why` profiles
and release `ready` meet the target at p95. Development `ready` misses by 2.4 ms, so the baseline does
not promote its median into a guarantee.
Structural current-plan reads remain above budget with cache disabled: in the median phase sample,
traces add about 101 ms of release validation, the repository's generated agent contract and exact
snapshot projection cost about 25-27 ms, and body-free `show` spends 112 ms validating before it
assembles inspection state. `A1-322` removes the named trace-document residual; the remaining named
work belongs to `A1-589` and `A1-593`. The baseline no longer contains the earlier duplicate config,
source copy or graph passes for these commands.

The state matrix is executable for the authoritative loader and persistent store and remains
explicit about later observation/output rows:

| State | Corpus / operation | Current result | Owner of remaining row |
|---|---|---|---|
| no index / authoritative load | current and generated 1,000 bounded + structural reads | measured above | complete |
| exact valid index hit | current, 1,000 and 10,000 bounded + structural reads | delivered and measured; 10,000 remains above target | `A1-591` scale residual |
| one task add/change/rename/remove | same | one edit measured; set changes and renames covered by deterministic tests | complete |
| trace or relevant config change | structural reads | trace/config invalidation covered; exact report hit performs zero trace parses | complete |
| unavailable/read-only/corrupt index | same bytes, findings and exit as no index | semantic equivalence and safe miss covered | complete |
| evidence observation current/stale/absent | default `show` | delivered and measured; provider spawn count is zero | complete |
| output-dominated | unscoped 10,000-row output | delivered and measured (`A2-136`): 16 KiB bounded streaming, O(1) output buffering, flat RSS vs `check`, and exhaustive CLI+MCP disposition inventory | complete |

This prevents an absent cache row from being reported as a hit and prevents a provider refresh from
being compared to a local read. `A1-589` extended the same harness rather than creating a second
matrix.

### A1-589 measured persistent snapshot

`PlanStore` now enumerates the authoritative candidate set and reuses exact parsed artifacts across
processes. Structural operations can also reuse a typed `Report` under the semantic build and trace-
input key defined by ADR-0050. The CLI, MCP and TUI enter through this application boundary; a
short-lived operation still constructs the borrowing graph. `--cache=off` bypasses persistence and
`--cache=refresh` ignores the prior generation. The harness isolates its cache and names `clean`,
`hit` and `one-edit` states rather than inferring them.

The following p95 wall observations were recorded on `kbox` under load averages from 3.45 to 8.28.
Current and 1,000-task exact hits use 15 samples after two warm-ups; the less stable development
one-edit row uses 30, and the other one-edit rows use 15. All include process startup and buffered
output. They are observational timing evidence; deterministic work is the portable assertion.

| profile / state | current `ready` | current `why` | current `show --no-resolve` | current `check` | 1,000 `why` | 1,000 `show --no-resolve` | 1,000 `check` |
|---|---:|---:|---:|---:|---:|---:|---:|
| development / hit | 36.2 ms | 33.6 ms | 58.1 ms | 46.2 ms | 34.8 ms | 47.6 ms | 39.4 ms |
| development / one edit | 52.1 ms | 53.2 ms | 75.3 ms | 72.3 ms | 52.4 ms | 62.7 ms | 56.5 ms |
| release / hit | 28.0 ms | 26.2 ms | 41.4 ms | 51.4 ms | 29.5 ms | 33.3 ms | 36.8 ms |
| release / one edit | 42.1 ms | 38.2 ms | 52.6 ms | 48.8 ms | 42.1 ms | 47.4 ms | 41.3 ms |

Every hit row observed one generation hit, zero task bytes read, zero task parses and zero trace
parses. Referenced trace bytes are content-hashed because no prior trace observation carries a safe
racy-clean horizon; hashing does not build their Markdown/YAML indexes. Every one-edit row reused all
but one candidate, read and parsed exactly the changed file, and parsed zero trace documents. The
current generation is about 907 KiB and the 1,000-task one is about 864 KiB.

The required 10,000-task publication used seven samples after two warm-ups. Every clean sample
deleted the isolated index, used `--cache=auto`, parsed the authoritative source and included
generation/report publication; it was not the cheaper `--cache=off` path. The matrix proves the
cache state and exposes, rather than hides, the remaining linear candidate enumeration, metadata,
JSON decode and projection cost:

| profile / state | `why` p95 | `show --no-resolve` p95 | `check` p95 | deterministic task parses |
|---|---:|---:|---:|---:|
| development / hit | 307.1 ms | 453.7 ms | 377.6 ms | 0 |
| release / hit | 235.0 ms | 345.3 ms | 349.8 ms | 0 |
| development / clean | 615.1 ms | 831.2 ms | 724.7 ms | 10,000 |
| release / clean | 570.7 ms | 753.6 ms | 667.7 ms | 10,000 |

The 10,000-task JSON generation is about 8.64 MB (8.24 MiB). It is below the private-format bound but
does not meet the target, so A1-591 owns the measured O(n) residual and the JSON-versus-binary
decision after the independent A1-590/A1-592/A1-593/A1-594 lanes land.

No new package entered `Cargo.lock`: `serde_json` and `sha2` were existing workspace dependencies
made direct where needed. Private cache directories and files were observed as mode 0700 and 0600.
The measured dirty-tree binaries were 58,958,408 bytes in development and 17,198,256 bytes in
release, compared with A1-588's clean 59,474,824 and 16,615,848 byte observations; the source states
differ, so those deltas are disclosed rather than attributed. The final development and release
rebuilds took 14.20 and 46.86 seconds; a no-op mbx build took 0.09 seconds. JSON remains the encoding
because current/1,000 hits meet their budget and the 10,000-task bottleneck is broader than encoding
alone.

### A1-590 measured evidence observation

Default `show` now reads a private, checksummed provider observation and recomputes the task-specific
answer without starting a configured provider. An explicit refresh remains outside the read budget:
it names runner start and finish events, permits at most two concurrent catalogue commands, and
publishes only a complete result whose declared material inputs remained stable. The executable
regression test observes process creation directly; the timing result is not being asked to prove a
negative that the test can assert deterministically.

The final development-binary measurements on `kbox` used the live 678-task plan and task `A1-587`,
whose evidence includes one commit and two tests. Each row includes process startup, exact snapshot
reuse, evidence projection and JSON rendering. A fixed minimal inherited environment was used for
both refresh and reads because this evaluation host injects a different `CODEX_TOOL_CALL_ID` into
each command invocation. That variable correctly changes the provider material key: an unrestricted
environment is an input to configured commands, so silently excluding it for a benchmark would have
tested weaker freshness semantics than the product ships.

| observation state / command | samples | min | p50 | p95 | max | host one-minute load |
|---|---:|---:|---:|---:|---:|---:|
| current / default `show` | 80 | 78.3 ms | 81.8 ms | 86.4 ms | 99.6 ms | 10.74 |
| stale / default `show` | 40 | 79.6 ms | 83.7 ms | 91.5 ms | 93.9 ms | 7.61 |
| skipped / `show --no-resolve` | 40 | 59.1 ms | 61.9 ms | 86.0 ms | 92.5 ms | 15.50 |

The current row's mean was 82.5 ms; Hyperfine reported 59.0 ms user and 33.7 ms system time per
sample. Every citation and the aggregate were `current` before sampling. The earlier unrestricted
host-harness read became `stale`, rather than resolved, immediately after refresh because its
injected environment token changed; that is a fail-closed invalidation observation, not a discarded
timing sample. Warm explicit refresh listed all twelve runners in 8.0 seconds and never had more than
two active. A prior cold refresh took about 53 seconds, which is why provider work has a progress and
resource contract rather than a false sub-100 ms promise.

### A1-322 measured trace snapshot

`A1-322` shares one read and one parsed Markdown index or YAML/JSON value per distinct traced
document during a validation run. The work counters are cumulative on the operation's thread and the
CLI reports the delta. They count successful UTF-8 reads and bytes, Markdown index construction
attempts, and structured parse attempts including failures. Missing or non-UTF-8 documents remain
findings owned by the existing rules rather than being relabelled as successful work.

The 200-closed-task shapes that exposed the issue were rerun on `kbox` from clean revision
`d88d79ec234435f8d78510de49b5e375c1a2a6a5`, with 120 records in one shared document, 20 samples
after two warm-ups. Every sample exited cleanly, retained stable output and reported exactly one
trace read and parse:

| profile / resolver | full-process min / p50 / p95 / max | validation p50 / p95 |
|---|---:|---:|
| release / Markdown | 16.8 / 18.0 / 23.0 / 23.6 ms | 3.5 / 3.6 ms |
| release / YAML | 16.9 / 18.5 / 19.3 / 19.3 ms | 3.9 / 4.1 ms |
| development opt-1 / Markdown | 20.3 / 21.8 / 25.0 / 25.7 ms | 4.1 / 4.8 ms |
| development opt-1 / YAML | 21.3 / 22.1 / 24.0 / 26.2 ms | 4.5 / 4.7 ms |

The earlier 517/1205 ms observations predate the current profile and harness and are not treated as
like-for-like ratios. The rerun states both profiles, the complete corpus shape, full-process and
validation percentiles, and deterministic work. A separate 200-reference unit shape alternates
lexical aliases across Markdown and YAML and proves exactly two reads and parses. Twenty-five
references to one malformed YAML source still produce 25 task findings but only one parse attempt.

On the complete current-plan matrix, 15 samples after two warm-ups, the release `show --no-resolve`
p95 fell from the A1-588 baseline's 197.8 to 90.6 ms and `check` from 212.1 to 107.8 ms. Development
`show --no-resolve` fell from 361.9 to 125.3 ms and `check` from 754.8 to 201.0 ms. The less
load-sensitive validation medians moved from 114.2 to 24.5 ms in release and 232.0 to 34.5 ms in
development. Each structural validation reports 38 reads, 1,846,273 bytes, 38 Markdown parses and
zero structured parses for the current snapshot; commands that do not validate traces report zero.
Release `show --no-resolve` now meets the p95 target, while release `check` narrowly misses and the
development structural reads remain above it. That residual is evidence for `A1-589`, not a reason to
weaken or prematurely claim the budget.

### A2-135 measured single-pass vacuity

`A2-135` replaces `vacuity`'s repeated validation passes with one exhaustive validation pass that
emits typed per-rule `CheckActivation` observations (`Inapplicable`, `EnabledNoFindings`,
`EnabledWithFindings`, `Disabled`) driven by mandatory `SwitchDisposition` rows on `Rule` in
`define_rules!`. `aplan vacuity` builds one borrowed `Graph` shared across optional selection
(`--cut`, `--status`, etc.) and `validate_with_graph_and_observations`, and parses each distinct
traced document at most once through the run-scoped `TraceDocuments` cache.

Cache-disabled (`--cache=off`) before/after measurements were recorded on `kbox` (Linux x86_64,
recorded load average `5.12 4.66 3.68`), 15 samples after two warm-ups, comparing baseline commit
`b55ca36` (release `39c8b6a7975daf5a`, development `fac51257eb35ff73`) against the single-pass
implementation (release `432e403e5c72c941`, development `2c1bb8a200e9f9a4`). Every row produced
byte-identical stdout and exit `0` across both builds:

| corpus / command (`--cache=off`) | release p50 / p95 (before -> after) | dev p50 / p95 (before -> after) | release CPU p50 (before -> after) | graphs (before -> after) | trace reads / MD / struct (before -> after) |
|---|---:|---:|---:|---:|---:|
| current (205 tasks) / `vacuity` (human) | 41.3 / 43.9 -> 28.2 / 34.5 ms | 60.4 / 70.5 -> 40.3 / 43.9 ms | 41.0 -> 27.9 ms | 3 -> 1 | 9 / 9 / 0 -> 3 / 3 / 0 |
| current (205 tasks) / `vacuity --format json` | 41.9 / 49.9 -> 28.3 / 29.0 ms | 61.0 / 69.9 -> 40.7 / 45.5 ms | 41.3 -> 27.9 ms | 3 -> 1 | 9 / 9 / 0 -> 3 / 3 / 0 |
| current (205 tasks) / `vacuity --status done --format json` | 44.3 / 45.9 -> 31.4 / 32.5 ms | 64.0 / 67.3 -> 44.2 / 45.4 ms | 43.9 -> 31.0 ms | 4 -> 1 | 9 / 9 / 0 -> 3 / 3 / 0 |
| generated-1000 / `vacuity --no-config --format json` | 62.6 / 64.2 -> 51.0 / 56.5 ms | 82.0 / 86.0 -> 63.6 / 71.1 ms | 62.1 -> 50.5 ms | 5 -> 1 | 0 / 0 / 0 -> 0 / 0 / 0 |
| generated-1000-traced (switches off, shared MD + YAML) / `vacuity --format json` | 246.3 / 254.6 -> 89.3 / 98.6 ms | 284.8 / 288.7 -> 111.7 / 113.4 ms | 244.9 -> 88.7 ms | 6 -> 1 | 12 / 6 / 6 -> 2 / 1 / 1 |

Every case dropped to `graph_builds=1` (including scoped `--status` / `--cut` runs) and at most one
read and parse per distinct referenced trace document, cutting release CPU on the 1,000-task
shared-trace corpus by 2.76x (244.9 ms to 88.7 ms) and bringing its cache-disabled release p95
under 100 ms. A mutation test restores the pre-`A2-135` repeated-validator path and verifies that it
produces the same map while failing the deterministic graph-build and trace-parse proof.

### A2-136 measured output-dominated streaming and bounded-response threshold

`A2-136` measures representative human, JSON, Markdown, graph and MCP responses before and after
streaming, establishes a data-backed bounded-response threshold, streams preflighted output-dominated
surfaces through a bounded 16 KiB `StreamingWriter` (`crates/aplan-output/src/stream.rs`), extends
`--timings` and `ci/scripts/read-performance.py --output-matrix` with `output_bytes`,
`peak_output_buffer_bytes`, `transfer_p50_ms` and per-child `max_rss_p50_kib`, hardens early
`BrokenPipe` and interrupted MCP consumer shutdown (including terminating active configured providers
on client disconnect with zero orphaned descendants), and enforces an exhaustive derived CLI+MCP
read-surface disposition inventory (`aplan::read_surface_inventory()`).

**How the 256 KiB threshold and 16 KiB streaming buffer were established.** Pre-streaming
measurements on `kbox` (baseline `68f43e1`, release profile, warm index hit, `NO_COLOR=1`) exposed
two concrete failure modes once output crossed roughly 256 KiB:

1. **Unbuffered per-token writes turned transfer into the dominant latency phase.** Pre-streaming
   `tasks --format json` wrote unbuffered `serde_json::to_writer_pretty` directly to a locked stdout
   adapter, causing 37.9 ms transfer on the current 206-task plan (301 KiB), 96.2 ms transfer on
   1,000 tasks (830 KiB), and 596.2 ms transfer on 10,000 tasks (8.24 MiB, 1,451.7 ms total wall).
2. **Whole-document JSON `Value` and `String` buffers caused multi-MiB RSS spikes before byte 0.**
   Pre-streaming `roadmap --format json` (2.31 MiB) and `graph --format json` (2.27 MiB) spiked
   10,000-task process RSS to 94,240 KiB (+19.3 MiB over `roadmap` human) and 91,716 KiB (+16.5 MiB
   over `graph` dot) before emitting byte 0.

Meanwhile every response at or below 256 KiB transferred in 1-15 ms (completing well inside the
100 ms read budget across both current and 1,000-task corpora). Across Linux pipe capacity (64 KiB;
4x = 256 KiB), terminal PTY buffers (4 KiB), and MCP protocol caps (64 KiB paginated resources,
512 KiB hard tool ceiling), **256 KiB (`BOUNDED_RESPONSE_THRESHOLD_BYTES`)** is the empirical
boundary above which output volume becomes its own latency and memory class, and **16 KiB
(`STREAM_BUFFER_CAPACITY`)** is the streaming buffer size (one quarter of a Linux pipe buffer: large
enough to coalesce millions of small formatting writes while capping peak resident output buffering
at O(1) 16 KiB and keeping early `BrokenPipe` stop responsive).

**Reproducible post-streaming measurements (`ci/scripts/read-performance.py --output-matrix`).**
Recorded on `kbox` (`Linux-7.2.8-arch1-2-x86_64`, one-minute load 7.15-13.50 while other host work
ran) comparing pre-streaming release baseline (`68f43e1`) against post-streaming release
(`target/release/aplan`, 31,798,336 bytes, SHA-256 `6ab1b32890fd5bf6`) and development
(`target/debug/aplan`, 108,011,416 bytes, SHA-256 `6384ad543d9f64e6`) on warm index hits (5 samples
after 2 warm-ups on current and 1,000-task corpora; 3 samples after 1 warm-up on release 10,000 and
2 samples after 1 warm-up on dev 10,000):

| corpus / command | bytes | release wall p50 (before -> after) | release TTFB p50 (before -> after) | release xfer p50 (before -> after) | dev wall p50 / p95 (after) | peak output buf (after) | release max RSS (before -> after) |
|---|---:|---:|---:|---:|---:|---:|---:|
| current / `ready` human | 7,095 | 41.1 -> 19.8 ms | 37.5 -> 18.5 ms | 3.3 -> 1.3 ms | 26.1 / 30.9 ms | 7,095 B | 26,008 -> 26,404 KiB |
| current / `ready --format json` | 27,058 | 45.5 -> 22.0 ms | 40.9 -> 19.7 ms | 4.5 -> 1.6 ms | 25.9 / 34.6 ms | 16,384 B | 25,644 -> 26,276 KiB |
| current / `tasks` human | 25,623 | 53.2 -> 22.6 ms | 46.8 -> 20.9 ms | 7.4 -> 1.6 ms | 28.8 / 34.6 ms | 16,370 B | 26,156 -> 26,728 KiB |
| current / `tasks --format json` | 300,941 | 71.6 -> 23.0 ms | 35.2 -> 20.5 ms | 37.9 -> 2.4 ms | 32.7 / 35.4 ms | 16,384 B | 25,796 -> 26,420 KiB |
| current / `roadmap --format json` | 76,797 | 29.2 -> 12.8 ms | 26.4 -> 11.6 ms | 3.4 -> 1.2 ms | 18.7 / 49.9 ms | 16,384 B | 20,216 -> 25,224 KiB |
| generated-1000 / `tasks` human | 90,522 | 107.5 -> 58.3 ms | 93.1 -> 54.4 ms | 14.7 -> 3.8 ms | 84.8 / 95.4 ms | 16,380 B | 27,660 -> 28,344 KiB |
| generated-1000 / `tasks --format json` | 847,347 | 199.4 -> 57.7 ms | 94.6 -> 52.4 ms | 96.2 -> 5.3 ms | 80.1 / 82.7 ms | 16,384 B | 27,328 -> 28,444 KiB |
| generated-1000 / `roadmap --format json` | 221,620 | 78.0 -> 40.5 ms | 73.2 -> 36.8 ms | 5.2 -> 3.1 ms | 53.8 / 56.5 ms | 16,384 B | 23,508 -> 27,636 KiB |
| generated-1000 / `graph --format json` | 207,452 | 69.2 -> 35.2 ms | 62.5 -> 32.6 ms | 6.8 -> 2.6 ms | 44.7 / 48.3 ms | 16,384 B | 22,472 -> 27,636 KiB |
| generated-10000 / `ready --format json` | 570,082 | 631.5 -> 391.5 ms | 609.1 -> 377.1 ms | 22.4 -> 14.4 ms | 548.6 / 551.0 ms | 16,384 B | 79,724 -> 83,944 KiB |
| generated-10000 / `tasks` human | 909,523 | 1,055.2 -> 575.6 ms | 968.5 -> 542.5 ms | 91.6 -> 33.5 ms | 1,011.2 / 1,060.6 ms | 16,382 B | 83,876 -> 87,940 KiB |
| generated-10000 / `tasks --format json` | 8,413,749 | 1,451.7 -> 596.9 ms | 867.4 -> 535.3 ms | 596.2 -> 58.4 ms | 960.1 / 966.5 ms | 16,384 B | 84,244 -> 87,628 KiB |
| generated-10000 / `roadmap` human | 679,819 | 605.4 -> 387.7 ms | 567.8 -> 364.2 ms | 37.5 -> 24.5 ms | 635.7 / 688.6 ms | 16,380 B | 74,976 -> 79,484 KiB |
| generated-10000 / `roadmap --format json` | 2,178,252 | 672.9 -> 374.8 ms | 642.7 -> 355.1 ms | 29.2 -> 19.7 ms | 627.0 / 642.5 ms | 16,384 B | 94,240 -> 79,168 KiB |
| generated-10000 / `roadmap --format markdown` | 738,740 | 615.7 -> 361.7 ms | 591.3 -> 346.6 ms | 24.4 -> 15.0 ms | 625.8 / 639.5 ms | 16,384 B | 76,384 -> 80,112 KiB |
| generated-10000 / `graph --format mermaid` | 831,093 | 500.9 -> 306.8 ms | 477.9 -> 289.7 ms | 22.9 -> 16.7 ms | 496.7 / 527.6 ms | 16,384 B | 74,916 -> 78,904 KiB |
| generated-10000 / `graph --format dot` | 1,073,121 | 434.7 -> 319.6 ms | 417.9 -> 299.4 ms | 16.8 -> 18.4 ms | 477.5 / 512.8 ms | 16,384 B | 75,172 -> 79,412 KiB |
| generated-10000 / `graph --format json` | 2,083,054 | 391.7 -> 323.4 ms | 367.3 -> 302.9 ms | 25.1 -> 19.0 ms | 438.1 / 449.3 ms | 16,384 B | 91,716 -> 78,868 KiB |
| generated-10000 / `check --format json` (control) | 136 | 386.5 -> 361.6 ms | 375.1 -> 348.7 ms | 14.1 -> 11.9 ms | 493.9 / 509.5 ms | 136 B | 75,136 -> 79,308 KiB |

Key observations from the matrix and complementary consumer probes:

- **Bounded responses complete inside 100 ms in both profiles.** Every current-plan and 1,000-task
  surface completes below 100 ms p95 in both release (12.6-79.9 ms) and development (19.5-95.4 ms).
- **Transfer speedup and flat RSS on 10,000 tasks.** Coalesced 16 KiB streaming cut `tasks --format
  json` transfer 10.2x on 10,000 tasks (596.2 ms to 58.4 ms) and 18.2x on 1,000 tasks (96.2 ms to
  5.3 ms). Eliminating intermediate `serde_json::Value` trees dropped 10,000-task RSS on `roadmap
  json` by 15.1 MiB (94,240 KiB to 79,168 KiB) and `graph json` by 12.8 MiB (91,716 KiB to
  78,868 KiB), matching the 136-byte `check json` control (79,308 KiB) within 0.5 MiB. Every
  streamed 10,000-task surface stays pinned at `peak_output_buffer_bytes <= 16,384` (up to 513x
  lower output buffering on the 8.41 MiB `tasks json` payload). The remaining 10,000-task
  load/projection latency (~270 ms load, ~75-248 ms projection) belongs to `A2-134` (`A1-591`).
- **Throttled-pipe separation of TTFB from transfer.** Draining stdout at 16 KiB per 5 ms (simulating
  a slow pager/terminal consumer), release `current / tasks json` (301 KiB) emits byte 0 at
  **22.9 ms TTFB** while transferring over 189.7 ms (212.6 ms wall, `peak_buf=16,384`);
  `generated-1000 / tasks json` (847 KiB) emits byte 0 at **64.8 ms TTFB** while transferring over
  544.2 ms (609.1 ms wall, `peak_buf=16,384`).
- **MCP protocol framing bounds.** Single JSON-RPC tool frames cannot stream incrementally, so
  `aplan_ready`, `aplan_check`, `aplan_verify`, `aplan_show`, `aplan_why` and `aplan_identity`
  enforce a 512 KiB `MAX_TOOL_RESULT_BYTES` ceiling on both legacy text and structured envelopes and
  return a bounded refusal directing callers to paginated `aplan://` resources (`<= 64 KiB` per
  page). Measured cold-session stdio release frames: `current / aplan://plan/summary` 1,029 B,
  `current / aplan_ready` 20,234 B, `generated-1000 / aplan_ready` 40,917 B, and
  `generated-1000 / aplan://collections/ready?limit=20` 5,563 B.

**Derived read-surface output dispositions (`aplan::read_surface_inventory()`).** Every CLI read
command/format and every MCP read tool/resource is derived from the live `clap` command tree,
`COMMAND_SURFACES`, `TOOLS`, `RESOURCES` and `RESOURCE_TEMPLATES` and assigned one of four explicit
dispositions, audited executably in `crates/aplan/tests/read_streaming.rs`:

| Disposition | Surfaces | Atomic error / size boundary |
|---|---|---|
| `streamed` | CLI `check`, `verify` (`human`, `json`, `sarif`, `github`, `agent`), `ready`, `tasks`, `roadmap` (`human`, `json`, `markdown`), `graph` (`mermaid`, `dot`, `json`), `paths`, `stale`, `due`, `deprecations`, `test list` (`human`, `json`, `agent`) | Validation, projection and finite-float preflights complete before byte 0; 16 KiB `StreamingWriter` strips ANSI before counting and stops promptly with exit 0 on `BrokenPipe` |
| `paginated` | CLI `completed` (`human`, `json`); MCP `aplan://collections/ready{?limit,cursor}`, `aplan://collections/completed{?cut,phase,owner,tag,since,until,oldest,history,limit,cursor}` | Bounded page limit (`<= 100` CLI, `<= 50` and `<= 64 KiB` MCP) with snapshot-bound continuation cursor |
| `explicitly_capped` | CLI `graph --format text`, `inbox` (`human`, `json`), `paths match` (`human`, `json`); MCP read tools (`aplan_identity`, `aplan_ready`, `aplan_why`, `aplan_show`, `aplan_check`, `aplan_verify`), `aplan://plan/summary`, `aplan://tasks/{id}` | Hard ceiling enforced before emission (`MAX_CANVAS_CELLS` 8,000,000, `inbox` 100 rows, `paths match` default 200 per entry with `--all` streaming, MCP 512 KiB tool / 64 KiB resource caps) |
| `buffered` | CLI `show`, `why`, `summary`, `critical-path`, `vacuity`, `version`, `check-docs`, `tree`, `facts`, `schema`, `explain`, `attest`, `export-agents-md`, `export-roadmap-json`, `export`, `inbox show`, `test receipt` | Focused single-task / fixed-cardinality views or atomic `--check`/template/digest contracts that must assemble completely before emitting bytes |

### A1-588 implementation choices

- **Development optimization accepted.** Global development `opt-level = 1` kept debuginfo and
  incremental compilation while reducing representative CPU by roughly 4-10x from the original
  unoptimized readings. A selective trial (`aplan-core`, output/MCP and parser dependencies optimized)
  left unoptimized orchestration dominant: current `ready`/`why` still consumed about 170 ms CPU and
  current `check` remained around 2.4-2.6 seconds. The measured clean-revision development binary is
  59,474,824 bytes, versus 76,299,840 bytes for that selective trial and 16,615,848 bytes for release. Profile-transition
  builds took 31-62 seconds on the contended host; these are not clean-build claims, but they expose the
  compile-time cost instead of omitting it.
- **In-process schema reuse accepted.** One `OnceLock` compiles the derived validator once in a long-lived
  MCP/TUI process and adds no dependency or persisted state. It cannot improve a one-shot process's first
  compile, so none of the CLI win above is attributed to it.
- **Parallel cold parsing rejected for this increment.** The generated 1,000-task release check already
  has 34.3 ms median CPU and 37.7 ms p95 wall time under host load, while current-plan CPU shows semantic
  content and traces, not candidate count, as the residual. Spending multiple threads would compete with
  configured builds, increase peak live source storage and complicate deterministic assembly for a row
  already inside budget. It adds neither compile time nor binary size because it was not admitted.
  Reconsider only on the published 10,000-task clean-checkout row after `A1-589`; cap concurrency and
  measure peak memory if that row proves parsing, rather than validation or output, dominant.

### Confirmed sources of avoidable cost and their disposition

1. **Every ordinary load was an identified load (removed by A1-588/A1-589).** Typed-only commands
   use non-retaining snapshots, and `show` retains candidate sources only for `--body`. A reused
   candidate now carries its exact SHA-256 in the identified input instead of retaining its body
   merely for hashing; byte-backed and digest-only inputs produce the same canonical identity.
2. **The input is parsed twice by design, but unchanged input is parsed on every process.** The first
   YAML parse feeds JSON Schema validation; the second typed parse catches duplicate fields that the
   JSON value would lose. That safety property must remain, but a content-valid parsed artifact can
   be reused across processes.
3. **Schema compilation was repeated in-process (removed by A1-588).** One `OnceLock` now helps
   MCP/TUI but, as measured, cannot remove a one-shot CLI's first compilation.
4. **`PlanContext` copied and searched (removed by A1-588).** It now consumes one path map, indexes
   the first source for each task id in one pass, and lends body text from the owned byte buffer.
5. **Trace validation repeated document work (removed by A1-322/A1-589).** One validation context now
   canonicalizes aliases into a run-scoped document cache, reads each distinct target once, builds
   one Markdown heading/table index or parses one YAML/JSON value, and shares it across address
   resolution, stampability and drift comparison. Failed structured parses are cached too. The
   deterministic counters distinguish successful reads/bytes from Markdown and structured parse
   attempts. An exact semantic report hit persists that typed result across commands and parses zero
   trace documents; a trace observation change recomputes through the same run-scoped parser.
6. **`vacuity` validated repeatedly (removed by A2-135).** Each rule row in `define_rules!` now
   carries a mandatory `SwitchDisposition`, and one validation pass emits typed `CheckActivation`
   observations while sharing a single borrowed `Graph` and run-scoped `TraceDocuments` cache.
   Missing, duplicate and stale switch dispositions fail an executable exhaustive audit across
   `Rule::ALL`, `.aplan.yml`'s derived schema and the end-to-end rule corpus.
7. **Configuration was read twice (same-frame duplication removed by A1-588).** Root resolution and
   the command now share one selected value only when discovery identifies the same frame. A later
   security read after configured-provider execution remains a real read by design.
8. **Graph/report work was rebuilt independently (removed by A1-588/A1-589).**
   `show` and the interactive task snapshot now share one borrowed graph across validation and every
   projection. CLI, MCP and TUI share `PlanStore`; exact semantic builds can reuse structural reports
   across operations without persisting a borrowing graph.
9. **Default `show` crosses an unbounded cost class before rendering.** Bare test names require every
   runner to be considered to reject ambiguity. The per-run cache correctly lists each runner once,
   but a one-shot process starts empty. Each runner defaults to a 600-second timeout and a configured
   zero removes the limit. No amount of YAML optimization makes that a 100 ms operation.

## Delivered and remaining architecture

### One immutable `PlanStore`

The delivered application-layer `PlanStore`, not a second domain model, owns:

- the selected configuration value and its exact semantic identity;
- one plan snapshot plus only the identified source bytes the requested features need, shared rather
  than cloned;
- a stable plan snapshot digest;
- a per-operation `PlanView` that borrows the stored plan and builds the existing `Graph` once for
  every projection that operation needs;
- typed structural reports keyed by canonical rule inputs and trace-document observations;
- optional evidence observations keyed by the same plan and provider identities.

`Plan`, `Task`, `Graph`, diagnostics and typed application responses remain the domain types.
`PlanStore` is the lifecycle and reuse boundary around them. `Graph` currently borrows `Task`, so the
store must not become an unsafe/self-referential owner merely to memoize it: a short-lived
`PlanView<'_>` builds one borrowed graph and passes it through validation and projection. Refactor to
an owned graph index only if that construction is measured as material. A CLI invocation constructs
one store; MCP and TUI keep one between requests and replace it atomically after refresh. No adapter
may create its own loader, trace or provider memo.

The store must expose work counters in tests. Assertions such as "zero task parses on an exact hit",
"one task parse after one edit", "one parse per trace document" and "zero provider spawns for
default show" are more stable than timing and directly guard the optimization's correctness.

### A disposable persistent plan index

The on-disk index is an implementation detail of `PlanStore`. Put it under the operating system's
conventional per-user cache location in an aplan namespace, keyed by a digest of the canonical plan
root and configuration selection. Do not put it in the plan, commit it, or require Git. A moved
checkout safely gets a miss. Provide an environment override for hermetic tests and global
`--cache=off`/`--cache=refresh` diagnostics, but no repository configuration key: a repository must
not choose where a reader writes host-local data. If the location is absent, read-only, unsafe or
over quota, continue without persistence. Apply bounded age/size eviction so old checkout keys
cannot grow without limit, and make eviction best-effort so it never changes a command verdict.

Version 1 is one checksummed JSON generation because `serde`, `serde_json` and SHA-256 were already
dependencies. A versioned binary encoding remains admissible only if representative measurements
isolate JSON as the reason a required row misses budget; the format is private, so replacement needs
no migration promise.

The generation contains:

- magic, platform tag, format epoch, cache-semantic epoch, exact validator/schema identity, writer
  build identity and checksum;
- canonical plan root and selected configuration identity;
- a sorted complete candidate manifest;
- for each candidate: relative path, file type/link facts, size, high-resolution mtime, ctime/file
  identity where available, content SHA-256 and either the parsed `LoadedTask` artifact or its load
  diagnostics;
- exact candidate digests; authoritative bodies stay in memory only where an application feature
  requests them;
- typed structural diagnostics keyed by canonical options, task semantics, trace observations and
  validator build identity;
- no authoritative status, no mutable task state and no command output.

The file is host-platform-specific rather than portable: path identities preserve native path
units instead of silently applying lossy UTF-8 conversion. The JSON representation uses an explicit
tag plus hexadecimal Unix bytes or Windows wide units; decoding a different platform tag is a miss.
Create its directory and files user-only, reject symlink substitution at every cache-owned path,
write through a unique create-new sibling and atomically replace the generation. A checksum detects
truncation and accidental corruption; it is not authentication against another process running as
the same user. Do not persist complete task bodies merely to make `show --body` cheaper, and bound
all decoded counts, string lengths and aggregate allocation before allocation. A read command
remains read-only with respect to the plan and repository but may refresh this derived user cache.
`--cache=off` is the strictly no-write mode for sandboxes and for proving the uncached loader.

Package version or Git `HEAD` alone is not an exact writer identity: an uncommitted development
rebuild can change parser semantics while both stay fixed. Persisted validation results therefore
require an explicit cache-semantic epoch and derived schema/rule identities in addition to the build
identity. A development binary that cannot prove its implementation identity may reuse safe parsed
source artifacts only when their format identity is exact; it must recompute semantic validation or
disable persistent reuse. This conservative miss is preferable to a fast answer produced by old
code.

Do not persist rendered command responses or invent a parallel `PlanIndex`. Reconstruct projections
from the cached `Plan` and existing `Graph`. Measurement justified persisting the existing typed
structural `Report`; ADR-0050 records its narrower semantic key and why trace parse trees remain
run-scoped rather than becoming another persisted model.

#### Exact invalidation

The fast path follows the same conservative pattern as Git's index and untracked cache:

1. Record every known file's size, mtime and available ctime/file identity. Stat known candidates on
   the next invocation.
2. Enumerate the candidate set every time with the authoritative loader's ignore, glob, symlink and
   file-type rules. Without a trusted directory monitor, directory metadata alone cannot safely
   prove that no task was added.
3. Never trust a racily clean file. Metadata-only reuse requires a strong file/change identity and
   both mtime and ctime strictly older than the snapshot horizon. If either timestamp overlaps the
   generation, precision is insufficient, or metadata is unavailable, hash its contents before
   reuse. Git documents this exact same-size/same-timestamp failure and resolves it with a content
   check.
4. For a changed metadata tuple, hash once. Reuse the parsed artifact when the content digest is
   unchanged; otherwise parse only that candidate.
5. Read with stat-before/read/stat-after. If an input changes during capture, retry a bounded number
   of times and then report that the plan changed while it was being read. Never combine an old
   task with a new identity and call the result current.
6. Include every input that can change the answer: selected config semantics, trace documents,
   validator/build epoch, relevant command flags and configured document bytes. Absence is an input.
   Use a canonical typed encoding for semantic digests, never Rust `Debug` output or map iteration.

Write a new generation to a unique create-new sibling temporary file, validate its checksum, then
atomically rename it. Readers use the last complete immutable generation without waiting for a
writer. A writer that cannot acquire a short advisory lock computes in memory and leaves the
existing generation alone; the lock is an efficiency device, not a correctness premise. Two racing
complete writers are safe because the next reader still validates the winning generation against
current inputs. A malformed, oversized, wrong-version or wrong-root cache is a miss, never a plan
diagnostic. Apply strict allocation bounds before parsing because a cache file is not trusted merely
because it is outside the repository.

Git's index documentation supports the design without making Git a dependency of the cache: Git
uses cached stat data, directory metadata for its untracked cache and an optional filesystem monitor
to avoid wider scans, while its racy-Git rules show why size and mtime alone are insufficient.

### Evidence observations, not an unsound provider cache

Plan parsing and arbitrary provider execution have different invalidation laws. Keep them in
separate stores.

The delivered observation envelope records:

- a provider identity over protocol, runner configuration and the input declaration;
- a material identity over every matched input byte, inherited environment and configured
  executable byte;
- exact runner/name pairs and listed, empty or failed catalogue state;
- observation time, completeness and any refresh failure;
- a distinct derived identity over the current typed plan, evidence configuration, repository
  observation and provider observation; and
- typed citation outcomes derived afresh rather than persisted as a verdict.

`verify`, explicit refresh and TUI background verification may publish a complete observation
atomically. A mutating `close` can reuse an independently keyed provider catalogue while checking
the pre-state, but its successful write changes the plan snapshot: it cannot publish that pre-state
result as a complete post-state observation unless it verifies the newly written snapshot. Default
`show` may reuse an observation only when all keys still match. Its state is a typed enum such as
`current`, `stale`, `not_observed`, `refresh_failed` or `unsupported`; the existing boolean
`resolved` remains true only for a current observation, whether its resolved citation succeeded or
failed. Human and JSON output name the state and the observation identity. Stale data may be
displayed as historical context but may not contribute a current verdict.

Freshness is resolver-specific, not one global TTL. Commit and file evidence are recomputed from the
selected repository on every inspection. A test catalogue is keyed to its declared material inputs.
The current format deliberately does not retain URL or version-command answers because neither has
an immutable subject or expiry/input contract; an explicit refresh can report the immediate URL
answer, while an ordinary read reports it as `unsupported`. `show` aggregates per-citation states
and is `current` only when every citation it claims to resolve is current.

Generic configured commands are not automatically cacheable. They may read the network, clock,
ambient environment or files they never declared. Add an opt-in provider input contract only after
testing it adversarially:

- `evidence.observation_inputs` globs define the exhaustive repository files that can change every
  configured test catalogue;
- the complete inherited environment, configured executable bytes and, for configured Rust tools,
  Rustup settings and installed-toolchain manifest hashes join the key without spawning a command;
- omission means `unsupported`, not an inferred subset;
- task files can select only typed runner/test identities, never commands or cache policy;
- a bare test name still consults all applicable runner catalogues to reject ambiguity;
- the runner-qualified evidence considered by `A1-478` can narrow a lookup but cannot make one slow
  runner complete in 100 ms.

An explicit input declaration is a trust claim by the repository, not something aplan can prove
complete for an arbitrary process. The UI and machine schema must say that `current` means current
under that declared contract. Providers that require stronger assurance need a self-authenticating
catalogue or an independently checkable build-system manifest; they must not receive a stronger
label merely because a TTL has not expired.

An optional generated catalogue file can make a runner independently fast if it carries its own
input digest and runner identity. A plain list of names with no provenance is not sufficient. A
long-lived TUI/MCP process may refresh observations asynchronously, but its old result is installed
only after the ordinary stale-input check. A future daemon can offer the same service across CLI
processes; it is not needed for the first release and cannot repair an incomplete invalidation key.
Persistent observations live in their own bounded namespace in the user cache, with the same
permission, corruption and atomic-replacement rules as the plan index. Deleting that namespace
changes only speed and whether the state is `not_observed`; it never changes plan facts.

The delivered user-facing command contract is:

```text
aplan show ID                    # below 100 ms; consult current observation, never spawn
aplan show ID --no-resolve       # skip even stored observations, preserving today's escape hatch
aplan show ID --refresh-evidence # explicit slow path with progress, bounds and cancellation
aplan show ID --require-current-evidence # fail unless every displayed observation is current
aplan verify                     # authoritative whole-plan refresh and gate
```

This is an existing-command behavior change and therefore needs a named `Behaviour`, proof, schema
compatibility decision, changelog entry and upgrade guidance. Plain `show` remains an inspection and
therefore keeps its existing success exit when the task can be shown; the typed state and
`--require-current-evidence` make freshness enforceable without turning a missing disposable cache
into a plan failure. `verify` remains the whole-plan evidence gate. The default becomes faster by
making freshness visible, not by redefining stale as fresh.

### Immediate changes before the persistent index

The first five items landed in `A1-588`; the remaining items keep independent owners so their value
and regressions stay visible:

1. **Delivered:** change non-identified callers to use `load_plan`; add selective body/source retention
   for consumers that need it.
2. **Delivered:** replace `PlanContext`'s per-task input search with a path map and duplicate body
   `String` copies with body slices borrowed from one owned byte set.
3. **Delivered:** carry one selected configuration through root resolution and the application context.
   Preserve commands whose explicit repository frame intentionally selects another config.
4. **Delivered:** build one borrowed `Graph` per operation and share it across validation, selection and
   projection. Do not put a borrowing graph inside its owning store. Audit repeated `task_rows`,
   roadmap and reachability construction only after measuring them.
5. **Delivered:** add opt-in timing diagnostics on stderr with phase durations and work counters. Machine
   stdout and exit semantics stay stable, and counters never expose source or provider secrets.
6. **Delivered by A1-322:** canonicalize, read and parse each trace document once per validation
   snapshot, then use the same parsed index or node for resolution, stampability and drift
   fingerprinting.
7. **Delivered by A1-592:** install configured-command process-group ownership for every CLI entry
   point, not only TUI termination.
8. **Delivered by A2-135:** rule evaluation emits typed activation observations in one pass so
   `vacuity` performs one graph construction and at most one parse per trace document while keeping
   switch dispositions exhaustive on `Rule` in `define_rules!`.
9. **Delivered by A2-136:** stream preflighted human, Markdown, graph and machine JSON/SARIF
   read outputs through a bounded 16 KiB `StreamingWriter` above the measured 256 KiB
   bounded-response threshold, enforce hard MCP tool/resource ceilings and orphan-free disconnect
   cleanup, and maintain an exhaustive executable CLI+MCP read-surface disposition inventory.

Also measure a narrowly optimized development profile for parser/validator dependencies. The empty
development-process baseline is only about 30 ms, while per-task work grows much faster than the
optimized build, so selective dependency optimization may improve clean misses. It is a supplement,
not the contract: reject it if incremental compile time or binary size rises materially, and do not
use it to excuse repeated work. An in-process schema `OnceLock` similarly helps MCP/TUI but does not
remove a one-shot CLI's first compilation. Parallel cold parsing is a later measured option; it must
retain deterministic diagnostics and stay within the machine-wide resource budget.

These changes should make the optimized current-plan cold path approach 100 ms and greatly reduce
index rebuild time. They do not replace the index: development-profile parsing is still several
times slower, and a 10,000-file clean checkout cannot honestly depend on parsing every unchanged
YAML document for each command.

## Alternatives considered

| Approach | Decision | Reason |
|---|---|---|
| Optimize only, no persistent state | Insufficient | Worth doing, but debug and 10,000-file repeated reads still pay all parsing and validation work. |
| One private snapshot file | Adopt | Matches the whole-plan consistency boundary, has cheap startup and can be atomically replaced. |
| SQLite/WAL | Do not adopt initially | Strong transactions and concurrent readers are useful, but this workload reads one immutable snapshot and has one occasional writer. The dependency, recovery, WAL/shm files and query model buy little over atomic generation replacement. |
| Salsa/red-green incremental queries | Defer | Excellent for a long-lived in-memory dependency graph, but it does not by itself supply cross-process persistence or filesystem/provider identities. Add it only if fine-grained in-process recomputation remains dominant. |
| Mandatory daemon/filesystem watcher | Defer | It can avoid stat scans and keep providers warm, but introduces discovery, lifecycle, version skew, crash recovery and security boundaries. The file index should meet the first target without it. |
| Git index/HEAD as the cache key | Reject | Misses untracked and dirty task/config bytes, excludes non-Git plans and conflates source control state with aplan's exact inputs. Git may accelerate repository-specific providers, not define plan identity. |
| Cache rendered command output | Reject initially | Multiplies keys and compatibility state. Cache normalized parsed/validated inputs and derive output. |
| Memory-map every Markdown file | Reject initially | It does not remove directory walks, schema/YAML work or provider execution, complicates mutation and small-file portability, and needs measurement before adding unsafe/platform-specific machinery. |
| New zero-copy/binary cache encoding | Defer | It can reduce decoding and allocation but adds a dependency and format complexity before JSON has been shown to miss the exact-hit budget. |
| Run fewer runners for bare test evidence | Reject | A skipped runner can contain the same name; accepting it would weaken fail-closed ambiguity detection. |
| Keep default `show` synchronous but print progress | Useful but insufficient | Fixes the apparent hang and remains required for explicit refresh, but does not meet completion below 100 ms. |

SQLite's own documentation confirms that WAL improves reader/writer concurrency but brings a WAL,
shared-memory index and checkpoint lifecycle. Bazel's Skyframe and Salsa both reinforce the more
important rule: incremental reuse is sound only when every input dependency is declared. That rule
is why plan artifacts, trace documents and evidence-provider observations have separate keys.

## Correctness, security and compatibility tests

The performance work is complete only when adversarial invalidation tests cover:

- same-size rewrite within timestamp granularity (the racy-clean case);
- atomic editor rename, add, delete, case-only rename, symlink retarget and file/directory swap;
- unreadable and non-UTF-8 candidates, incomplete frontmatter and duplicate YAML keys;
- config selection changing between discovered, explicit, absent and an external path;
- a trace target changing while task files do not, including two anchors in one document;
- index truncation, bit corruption, old/new format, hostile length fields, wrong root and wrong
  writer build, including a dirty rebuild at unchanged Git `HEAD`;
- unavailable/read-only cache roots, cache-path symlink substitution, bounded eviction, simultaneous
  readers and writers, a killed writer before rename and lock contention;
- task mutation through aplan and through an external editor;
- clean Git HEAD with dirty task, config, source and untracked test files;
- provider input declaration that omits a real dependency: the provider remains non-cacheable or
  the explicit trust claim is surfaced; it must not silently become current;
- duplicate test names in different runners, runner rename/reorder, toolchain change, timeout,
  cancellation and a stale observation after any key changes;
- successful `close` invalidating the pre-mutation plan observation before default `show` reads it;
- release and development binaries reading an index written by a different build;
- stdout snapshots with cache hit, miss and corruption proving that caching cannot change content,
  order, diagnostics or exit code.

Mutation tests should deliberately bypass each invalidation component and prove a stale answer is
not accepted. Performance tests must assert the work counters before asserting elapsed time. Delete
the cache and rerun the full correctness corpus in CI so a warm index can never hide a loader or
validator regression.

## Delivery sequence

### P0: define and expose the budget

- `A1-588` delivered the executable command matrix and published baseline;
- `A1-588` delivered phase/work instrumentation and a repeatable release procedure that records
  host load instead of assuming quiet hardware;
- `A1-588` reconciled the HLD target and distinguished exact hit, incremental edit and clean checkout;
- the existing complexity-ratio gate from `A1-229` remains unchanged.

### P0: remove deterministic waste

- `A1-588`: selective input retention, one config value and one graph;
- `A1-588`: linear `PlanContext` construction with shared source storage;
- `A1-322`: delivered run-scoped trace parsing and deterministic trace-work counters;
- `A1-592` delivered process-wide cleanup for the non-interactive configured-command orphan
  window;
- `A2-135`: delivered single-pass vacuity activation observations and exhaustive switch-catalogue
  audits;
- `A2-136`: delivered bounded 16 KiB output streaming above the measured 256 KiB threshold,
  preflighted machine serialization, orphan-free `BrokenPipe`/MCP disconnect cleanup, and the
  exhaustive derived CLI+MCP read-surface disposition inventory.

Re-run the matrix here. If every required current-plan command already meets the target in both
profiles, the persistent index can narrow to the 10,000-task and cross-process cases; it should not
be justified by old numbers.

### P0: exact persistent `PlanStore`

- `A1-589` delivered: first record the cache authority/location/invalidation ADR, then implement the
  private generation format, conservative invalidation, atomic replacement and corruption fallback;
- CLI, MCP and TUI use the same store boundary;
- successful mutations and external edits are observed conservatively on the next load;
- clean, hit and one-change equivalence and current/1,000 budgets are proved; the published 10,000
  residual is handed to `A1-591`.

### P0: fast evidence-backed inspection

- `A1-590` delivered: typed per-citation and aggregate evidence observations, provider-free default
  `show`, explicit bounded refresh and `--require-current-evidence`;
- raw catalogues reuse `A1-478`'s resolver normalization and preserve all-runner ambiguity checks;
- ADR-0053 records freshness, JSON v3, exit and compatibility semantics;
- private atomic observation publication and A1-592's process cleanup reject incomplete, mutated,
  cancelled and late results;
- CLI, MCP and TUI share the application operation, and interactive refresh reports exact runner
  activity.

### P1: broaden and harden

- `A1-591`: audit repository and large-output commands with separate budgets;
- use in-process watching for TUI/MCP only if polling/stat validation consumes the measured margin;
- evaluate a cross-process daemon only if the persistent index cannot meet the CLI target;
- ratchet the best demonstrated work counts and update release evidence.

These headings describe priority lanes, not extra dependency edges. The task metadata is the source
of truth: after `A1-588`, independent cleanup can proceed in parallel; `A1-589` waits only for the
baseline and trace snapshot it consumes, `A1-590` additionally waits for process lifecycle, and the
final audit waits for every implementation lane.

## Research basis

- [Git index format](https://git-scm.com/docs/index-format.html) describes cached stat data,
  untracked-directory state and filesystem-monitor tokens.
- [Git's racy-clean analysis](https://git-scm.com/docs/racy-git) explains why matching size and mtime
  can still hide a changed file and when content must be checked.
- [Git update-index](https://git-scm.com/docs/git-update-index) documents directory-mtime assumptions,
  the untracked cache and filesystem-monitor tradeoffs.
- [Cargo fingerprints](https://doc.rust-lang.org/stable/nightly-rustc/cargo/compiler/fingerprint/index.html)
  demonstrate persisted dependency fingerprints and dirty/fresh propagation, while also showing
  why opaque runner invocation is a build-system problem rather than a fast read.
- [Bazel Skyframe](https://bazel.build/reference/skyframe) documents dependency-driven
  invalidation and change pruning.
- [Salsa's red-green algorithm](https://salsa-rs.github.io/salsa/reference/algorithm.html)
  documents revisioned inputs and memoized derived queries.
- [SQLite WAL](https://www.sqlite.org/wal.html) documents its concurrent-reader benefits and the
  WAL/shared-memory/checkpoint lifecycle weighed above.

