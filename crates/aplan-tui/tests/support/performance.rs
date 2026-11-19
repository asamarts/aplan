//! Release-profile performance gate for the interactive reducer and renderer.
//!
//! Wall-clock measurements do not belong in every development test run. This
//! module is ignored by default and run in a release profile by the scheduled
//! scale job and, on the reference host, by `ci/scripts/reference-tui-perf.sh`.
//! The reference host owns the wall-clock budgets (`A2-164`, ADR-0067); a hosted
//! run fails a metric only well past them. Ratios and semantic assertions catch
//! work that silently grows with hidden plan content (`A1-447`) on every host.

use std::cell::RefCell;
use std::hint::black_box;
use std::time::{Duration, Instant};

use aplan_app::{CellSceneJob, CellSceneRequest, CellSceneResult};
use aplan_core::Status;
use aplan_output::MAX_WIDTH;
use aplan_output::graph_scene::GraphScene;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::{Position, Rect};

use crate::graph_widget::{GraphEdgePresentation, GraphHit, GraphViewport, hit_test};
use crate::model::{Action, Appearance, EffectCompletion, EffectOutput, Glyphs, Model, Route};
use crate::shell::render;
use crate::testing::{ModelBuilder, graph_task};

const SAMPLES: usize = 7;

/// The budget a wall-clock metric is held to, chosen by `APLAN_TUI_BUDGETS` (`A2-164`,
/// ADR-0067). The budget written beside each metric below is the hosted budget of 2026-11-18;
/// the reference host's own budgets are in `REFERENCE_BUDGETS_NS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Budgets {
    /// A hosted runner, the default: 1.5 times the hosted budget. The hosted machines differ by
    /// about 35%, so this fails a gross regression and not a slower machine; the reference host
    /// judges the rest.
    Hosted,
    /// The reference host, a quiet machine that runs one benchmark at a time, with the gate pinned
    /// to one CPU (`ci/scripts/reference-tui-perf.sh`): each metric's budget in
    /// `REFERENCE_BUDGETS_NS`, or the hosted rule for one `HOSTED_RULE_ON_REFERENCE` names.
    Reference,
    /// Print every metric and hold none to a budget: how the reference budgets are measured.
    Measure,
}

impl Budgets {
    fn from_env() -> Self {
        Self::named(std::env::var("APLAN_TUI_BUDGETS").ok().as_deref())
    }

    fn named(name: Option<&str>) -> Self {
        match name {
            None | Some("" | "hosted") => Self::Hosted,
            Some("reference") => Self::Reference,
            Some("measure") => Self::Measure,
            Some(other) => {
                panic!("APLAN_TUI_BUDGETS={other} is none of hosted, reference and measure")
            }
        }
    }

    /// The budget `metric` is held to, given its hosted budget, or none when measuring.
    fn of(self, metric: &str, hosted: Duration) -> Option<Duration> {
        match self {
            Self::Hosted => Some(hosted * 3 / 2),
            Self::Reference => {
                if let Some(&(_, budget)) = REFERENCE_BUDGETS_NS
                    .iter()
                    .find(|(name, _)| *name == metric)
                {
                    return Some(Duration::from_nanos(budget));
                }
                assert!(
                    HOSTED_RULE_ON_REFERENCE.contains(&metric),
                    "{metric} has no reference budget: measure it on the reference host and add \
                     it to REFERENCE_BUDGETS_NS (A2-164)"
                );
                Some(hosted * 3 / 2)
            }
            Self::Measure => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Hosted => "hosted",
            Self::Reference => "reference",
            Self::Measure => "measure",
        }
    }
}

/// Each wall-clock metric's budget on the reference host, in nanoseconds (`A2-164`). Five runs
/// there of source `58025c81b7ad-0d81d7890265-32f79219ed53`, each in a queue slot of its own and
/// pinned to one CPU, gave it through `ci/scripts/reference-tui-budgets.py`: the worst p95 times
/// 1.25, and at least the worst p95 plus 0.1 ms. `docs/design/read-path-performance.md` records
/// the runs.
const REFERENCE_BUDGETS_NS: &[(&str, u64)] = &[
    ("layered_cell_scene_layered_sparse", 613_068),
    ("layered_hit_layered_sparse", 100_087),
    ("layered_cell_scene_layered_fan_in", 735_119),
    ("layered_hit_layered_fan_in", 100_076),
    ("layered_cell_scene_layered_crossing", 980_245),
    ("layered_hit_layered_crossing", 100_099),
    ("list_paint_small", 641_047),
    ("list_paint_repository", 633_920),
    ("model_and_graph_10000", 2_074_955_648),
    ("list_paint_10000", 641_702),
    ("search_and_render_10000", 154_596_550),
    ("graph_search_and_render_10000", 820_648),
    ("summary_resize_and_reflow_10000", 736_390),
    ("graph_window_sparse", 227_092_245),
    ("graph_scene_sparse", 790_042),
    ("graph_paint_sparse", 652_688),
    ("graph_pan_sparse", 1_553_063),
    ("graph_hit_sparse", 100_085),
    ("graph_window_traversal_sparse", 135_309_037),
    ("graph_window_fan_in", 24_104_887),
    ("graph_scene_fan_in", 629_177),
    ("graph_paint_fan_in", 659_983),
    ("graph_pan_fan_in", 1_664_993),
    ("graph_hit_fan_in", 100_085),
    ("graph_window_traversal_fan_in", 12_771_440),
    ("graph_window_crossing", 73_062_200),
    ("graph_scene_crossing", 1_796_918),
    ("graph_paint_crossing", 674_190),
    ("graph_pan_crossing", 1_631_754),
    ("graph_hit_crossing", 100_126),
    ("graph_window_traversal_crossing", 27_521_593),
    ("graph_window_too_wide", 13_101_084),
    ("graph_scene_too_wide", 492_376),
    ("graph_paint_too_wide", 666_740),
    ("graph_pan_too_wide", 1_597_797),
    ("graph_hit_too_wide", 100_085),
    ("graph_focus_too_wide", 1_073_735),
    ("graph_window_traversal_too_wide", 6_946_563),
    ("status_only_refresh_10000", 1_154_092_254),
    ("loading_progress_paint", 439_045),
    ("loading_progress_tick", 100_009),
];

/// The metrics the reference host holds to the hosted rule, because their p95 spread across the
/// five runs by more than the agreed 5%, or 0.1 ms under 2 ms (maintainer decision, 2026-11-19).
/// `A2-180` makes them measurable there.
const HOSTED_RULE_ON_REFERENCE: &[&str] = &[
    "graph_focus_sparse",
    "graph_focus_fan_in",
    "graph_focus_crossing",
];

#[derive(Debug, Clone, Copy)]
struct Measurement {
    p50: Duration,
    p95: Duration,
    maximum: Duration,
}

thread_local! {
    /// The metrics this gate measured past their budget. The gate reports them together when it
    /// ends, so that one slow metric does not hide the measurements after it.
    static EXCEEDED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

impl Measurement {
    /// Print the measurement, and record it if its p95 is past the budget of this host (see
    /// [`Budgets`]); [`assert_within_budgets`] fails the gate. `hosted` is the metric's hosted
    /// budget of 2026-11-18.
    fn check_budget(self, name: &str, hosted: Duration) {
        self.check_budget_of(Budgets::from_env(), name, hosted);
    }

    fn check_budget_of(self, budgets: Budgets, name: &str, hosted: Duration) {
        let budget = budgets.of(name, hosted);
        eprintln!(
            "TUI_PERF metric={name} p50_us={} p95_us={} max_us={} budget_us={} budgets={} p95_ns={}",
            self.p50.as_micros(),
            self.p95.as_micros(),
            self.maximum.as_micros(),
            budget.map_or_else(|| "-".to_owned(), |budget| budget.as_micros().to_string()),
            budgets.label(),
            self.p95.as_nanos()
        );
        if let Some(budget) = budget
            && self.p95 > budget
        {
            EXCEEDED.with_borrow_mut(|exceeded| {
                exceeded.push(format!(
                    "{name} p95 {:?} exceeded the {} budget {budget:?}",
                    self.p95,
                    budgets.label()
                ));
            });
        }
    }
}

/// Fail the gate if any metric it measured was past its budget, naming every one.
fn assert_within_budgets() {
    let exceeded = EXCEEDED.take();
    assert!(exceeded.is_empty(), "{}", exceeded.join("\n"));
}

fn measure(mut operation: impl FnMut()) -> Measurement {
    let mut samples = Vec::with_capacity(SAMPLES);
    operation();
    for _ in 0..SAMPLES {
        let started = Instant::now();
        operation();
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    Measurement {
        p50: samples[SAMPLES / 2],
        p95: samples[SAMPLES - 1],
        maximum: samples[SAMPLES - 1],
    }
}

fn per_operation(total: Measurement, operations: u32) -> Measurement {
    Measurement {
        p50: total.p50 / operations,
        p95: total.p95 / operations,
        maximum: total.maximum / operations,
    }
}

fn sparse_tasks(count: usize, lanes: usize) -> Vec<aplan_core::task::Task> {
    (0..count)
        .map(|index| {
            let blocker = (index >= lanes).then(|| format!("T-{:05}", index - lanes));
            graph_task(
                &format!("T-{index:05}"),
                &format!("representative task {index}"),
                &blocker.as_deref().into_iter().collect::<Vec<_>>(),
            )
        })
        .collect()
}

fn fan_in_tasks(count: usize) -> Vec<aplan_core::task::Task> {
    let mut tasks = (0..count.saturating_sub(1))
        .map(|index| graph_task(&format!("F-{index:04}"), "fan-in source", &[]))
        .collect::<Vec<_>>();
    let blockers = tasks
        .iter()
        .map(|task| task.id.as_str())
        .collect::<Vec<_>>();
    tasks.push(graph_task("F-SINK", "fan-in sink", &blockers));
    tasks
}

fn crossing_tasks(layers: usize, width: usize) -> Vec<aplan_core::task::Task> {
    let mut tasks = Vec::with_capacity(layers * width);
    for layer in 0..layers {
        for lane in 0..width {
            let first = (layer > 0).then(|| format!("C-{:03}-{:03}", layer - 1, width - lane - 1));
            let second =
                (layer > 0).then(|| format!("C-{:03}-{:03}", layer - 1, (lane + 1) % width));
            let blockers = [first.as_deref(), second.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            tasks.push(graph_task(
                &format!("C-{layer:03}-{lane:03}"),
                "crossing task",
                &blockers,
            ));
        }
    }
    tasks
}

fn wide_tasks(count: usize) -> Vec<aplan_core::task::Task> {
    let mut tasks = vec![graph_task("W-ROOT", "wide root", &[])];
    tasks.extend(
        (0..count).map(|index| graph_task(&format!("W-{index:04}"), "wide dependent", &["W-ROOT"])),
    );
    tasks
}

fn graph_model(tasks: Vec<aplan_core::task::Task>) -> Model {
    let mut model = ModelBuilder::default()
        .graph(tasks)
        .glyphs(Glyphs::Ascii)
        .build();
    model.update(Action::Route(Route::Graph));
    model.update(Action::Resize {
        columns: 100,
        rows: 24,
    });
    model
}

fn paint(model: &Model, width: u16, height: u16, times: u32) -> Measurement {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("performance terminal");
    per_operation(
        measure(|| {
            for _ in 0..times {
                terminal
                    .draw(|frame| render(frame, black_box(model)))
                    .expect("performance draw");
            }
        }),
        times,
    )
}

fn update_and_paint(model: &mut Model, action: Action, width: u16, height: u16) {
    model.update(action);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("action terminal");
    terminal
        .draw(|frame| render(frame, black_box(model)))
        .expect("action draw");
}

#[cfg(target_os = "linux")]
fn resident_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")?
                .split_ascii_whitespace()
                .next()?
                .parse()
                .ok()
        })
}

fn assert_browser_and_summary_budgets() {
    let small_tasks = sparse_tasks(32, 4);
    let repository_tasks = sparse_tasks(700, 8);
    let large_tasks = sparse_tasks(10_000, 8);

    let mut small = graph_model(small_tasks);
    small.update(Action::Route(Route::Tasks));
    let small_list = paint(&small, 100, 24, 50);
    small_list.check_budget("list_paint_small", Duration::from_millis(4));

    let mut repository = graph_model(repository_tasks);
    repository.update(Action::Route(Route::Tasks));
    let repository_list = paint(&repository, 100, 24, 50);
    repository_list.check_budget("list_paint_repository", Duration::from_millis(6));

    let large_construction = measure(|| {
        black_box(graph_model(large_tasks.clone()));
    });
    large_construction.check_budget("model_and_graph_10000", Duration::from_millis(1_500));

    let mut large = graph_model(large_tasks);
    large.update(Action::Route(Route::Tasks));
    let large_list = paint(&large, 100, 24, 50);
    large_list.check_budget("list_paint_10000", Duration::from_millis(12));
    assert!(
        large_list.p95 <= small_list.p95.saturating_mul(8) + Duration::from_micros(750),
        "list painting grew with hidden rows: small {:?}, large {:?}",
        small_list.p95,
        large_list.p95
    );

    let task_search = measure(|| {
        update_and_paint(
            &mut large,
            Action::SearchInput("representative task 999".to_string()),
            100,
            24,
        );
        large.update(Action::SearchClear);
    });
    task_search.check_budget("search_and_render_10000", Duration::from_millis(400));

    large.update(Action::Route(Route::Graph));
    let graph_search = measure(|| {
        update_and_paint(
            &mut large,
            Action::SearchInput("representative task 999".to_string()),
            100,
            24,
        );
        large.update(Action::SearchClear);
    });
    graph_search.check_budget("graph_search_and_render_10000", Duration::from_millis(500));

    large.update(Action::SearchClear);
    large.update(Action::Route(Route::Overview));
    let resize_storm = per_operation(
        measure(|| {
            for (columns, rows) in [(140, 30), (100, 24), (60, 16), (48, 14)] {
                update_and_paint(&mut large, Action::Resize { columns, rows }, columns, rows);
            }
        }),
        4,
    );
    resize_storm.check_budget("summary_resize_and_reflow_10000", Duration::from_millis(12));
}

fn assert_graph_budgets() {
    for (name, tasks, construction_budget) in [
        ("sparse", sparse_tasks(2_000, 8), Duration::from_millis(250)),
        ("fan_in", fan_in_tasks(256), Duration::from_millis(250)),
        (
            "crossing",
            crossing_tasks(24, 16),
            Duration::from_millis(250),
        ),
        ("too_wide", wide_tasks(120), Duration::from_millis(250)),
    ] {
        let construction = measure(|| {
            black_box(graph_model(tasks.clone()));
        });
        construction.check_budget(&format!("graph_window_{name}"), construction_budget);

        let mut model = graph_model(tasks);
        assert!(model.graph_window.view.nodes.len() <= 64);
        let scene = measure(|| {
            let _ = black_box(GraphScene::layout(&model.graph_window.view, MAX_WIDTH));
        });
        scene.check_budget(&format!("graph_scene_{name}"), Duration::from_millis(10));
        paint(&model, 100, 24, 50)
            .check_budget(&format!("graph_paint_{name}"), Duration::from_millis(4));

        let pan = measure(|| {
            update_and_paint(
                &mut model,
                Action::SetGraphViewport { row: 3, column: 5 },
                100,
                24,
            );
            update_and_paint(
                &mut model,
                Action::SetGraphViewport { row: 0, column: 0 },
                100,
                24,
            );
        });
        pan.check_budget(&format!("graph_pan_{name}"), Duration::from_millis(25));

        if let Ok(scene) = &model.graph_scene
            && let Some(node) = scene.nodes.first()
        {
            let point = node.marker;
            per_operation(
                measure(|| {
                    for _ in 0..1_000 {
                        black_box(model.graph_hit(point));
                    }
                }),
                1_000,
            )
            .check_budget(&format!("graph_hit_{name}"), Duration::from_micros(250));
            let id = node.id.clone();
            measure(|| {
                model.update(Action::SelectTask(id.clone()));
            })
            .check_budget(&format!("graph_focus_{name}"), Duration::from_millis(50));
        }

        if !model.graph_window.continuations.is_empty() {
            let traversal = measure(|| {
                model.update(Action::OpenGraphContinuation);
                model.update(Action::PreviousGraphWindow);
            });
            traversal.check_budget(
                &format!("graph_window_traversal_{name}"),
                Duration::from_millis(100),
            );
        }
    }

    let refresh_tasks = sparse_tasks(10_000, 8);
    let mut refresh = graph_model(refresh_tasks.clone());
    let mut replacement_tasks = refresh_tasks;
    replacement_tasks[0].status = Status::Done;
    let replacement = graph_model(replacement_tasks).snapshot;
    let mut replacements = (0..=SAMPLES).map(|_| replacement.clone());
    let status_refresh = measure(|| {
        let request = refresh
            .update(Action::Refresh)
            .effect
            .expect("refresh request");
        refresh.update(Action::EffectCompleted(Box::new(EffectCompletion {
            request,
            outcome: Ok(EffectOutput::snapshot(
                replacements.next().expect("one snapshot per sample"),
            )),
        })));
    });
    status_refresh.check_budget("status_only_refresh_10000", Duration::from_millis(750));
}

fn assert_layered_graph_budgets() {
    for (name, tasks, layout_budget) in [
        (
            "layered_sparse",
            sparse_tasks(64, 8),
            Duration::from_millis(50),
        ),
        (
            "layered_fan_in",
            fan_in_tasks(64),
            Duration::from_millis(100),
        ),
        (
            "layered_crossing",
            crossing_tasks(4, 8),
            Duration::from_millis(50),
        ),
    ] {
        let model = graph_model(tasks);
        let projection = model.graph_explorer.clone();
        let layout = measure(|| {
            let completion =
                CellSceneJob::new(1, CellSceneRequest::new(black_box(projection.clone()))).run();
            assert!(matches!(completion.result, CellSceneResult::Ready(_)));
        });
        layout.check_budget(&format!("layered_cell_scene_{name}"), layout_budget);
        let CellSceneResult::Ready(scene) = CellSceneJob::new(2, CellSceneRequest::new(projection))
            .run()
            .result
        else {
            panic!("bounded layered performance scene must be complete")
        };
        let node = scene
            .nodes
            .iter()
            .find(|node| {
                let center = node.bounds.center();
                center.x.saturating_sub(scene.bounds.x) < 99
                    && center.y.saturating_sub(scene.bounds.y) < 23
            })
            .expect("one node intersects the performance viewport");
        let center = node.bounds.center();
        let position = Position::new(
            u16::try_from(center.x.saturating_sub(scene.bounds.x)).unwrap_or_default(),
            u16::try_from(center.y.saturating_sub(scene.bounds.y)).unwrap_or_default(),
        );
        assert_eq!(
            hit_test(
                &scene,
                Rect::new(0, 0, 100, 24),
                GraphViewport { row: 0, column: 0 },
                position,
                GraphEdgePresentation::All,
            ),
            Some(GraphHit::Node(node.identity.clone())),
            "the performance sample must measure a real semantic hit"
        );
        per_operation(
            measure(|| {
                for _ in 0..1_000 {
                    black_box(hit_test(
                        &scene,
                        Rect::new(0, 0, 100, 24),
                        GraphViewport { row: 0, column: 0 },
                        position,
                        GraphEdgePresentation::All,
                    ));
                }
            }),
            1_000,
        )
        .check_budget(&format!("layered_hit_{name}"), Duration::from_micros(250));
    }
}

fn assert_progress_budgets() {
    let build = aplan_app::BuildIdentity::default();
    let capabilities = aplan_app::BuildFacts::default().capability_view(&build);
    let (mut loading, request) = Model::loading(&build, capabilities, Appearance::default());
    loading.update(Action::EffectStarted(request.id));
    #[cfg(target_os = "linux")]
    let resident_before = resident_kib().expect("Linux resident memory");
    let progress = paint(&loading, 100, 24, 250);
    progress.check_budget("loading_progress_paint", Duration::from_millis(5));
    #[cfg(target_os = "linux")]
    {
        let resident_after = resident_kib().expect("Linux resident memory after progress paint");
        assert!(
            resident_after.saturating_sub(resident_before) <= 16 * 1_024,
            "progress painting retained more than 16 MiB: {resident_before} -> {resident_after} KiB"
        );
    }
    let progress_ticks = per_operation(
        measure(|| {
            for _ in 0..1_000 {
                loading.update(Action::Tick);
            }
        }),
        1_000,
    );
    progress_ticks.check_budget("loading_progress_tick", Duration::from_micros(100));

    let reduced_capabilities = aplan_app::BuildFacts::default().capability_view(&build);
    let (mut reduced, reduced_request) = Model::loading(
        &build,
        reduced_capabilities,
        Appearance {
            reduced_motion: true,
            ..Appearance::default()
        },
    );
    reduced.update(Action::EffectStarted(reduced_request.id));
    assert!(
        !reduced.needs_tick(),
        "reduced motion scheduled animation work"
    );
    let mut idle = ModelBuilder::default().build();
    assert!(idle.take_dirty(), "first frame is dirty");
    assert!(!idle.take_dirty(), "idle requested a redraw");
    assert!(!idle.needs_tick(), "idle requested a timer");
}

/// The portable release gate. It is intentionally absent from ordinary test
/// selection and runs only in the scheduled/manual scale job.
#[test]
#[ignore = "release performance gate; run by ci/scripts/scale.sh"]
/// Release-profile proof for `tui.graph_fallback_paint_is_viewport_bounded`
/// and `tui.refresh_rebuilds_only_the_visible_projection`.
fn tui_performance_gate_render_interaction_and_graph_work() {
    assert_browser_and_summary_budgets();
    assert_graph_budgets();
    assert_progress_budgets();
    assert_within_budgets();
}

/// The A1-682 layered-scene gate is independently runnable so an unrelated
/// browser or progress regression cannot hide its measurements in a failed
/// nightly log. The complete scale job runs both gates.
#[test]
#[ignore = "release layered-graph performance gate; run by ci/scripts/scale.sh"]
fn layered_graph_performance_gate_layout_and_semantic_hits() {
    assert_layered_graph_budgets();
    assert_within_budgets();
}

/// Not a measurement: the budget each host is held to, which an ordinary test run checks.
#[test]
fn a_wall_clock_metric_is_held_to_the_budget_of_its_host() {
    let hosted = Duration::from_millis(4);
    assert_eq!(Budgets::named(None), Budgets::Hosted);
    assert_eq!(Budgets::named(Some("hosted")), Budgets::Hosted);
    assert_eq!(Budgets::named(Some("reference")), Budgets::Reference);
    assert_eq!(Budgets::named(Some("measure")), Budgets::Measure);
    assert_eq!(
        Budgets::Hosted.of("list_paint_small", hosted),
        Some(Duration::from_millis(6))
    );
    assert_eq!(Budgets::Measure.of("list_paint_small", hosted), None);
    for (metric, budget) in REFERENCE_BUDGETS_NS {
        assert!(*budget >= 100_000, "{metric} is under the 0.1 ms floor");
        assert!(!HOSTED_RULE_ON_REFERENCE.contains(metric), "{metric}");
        assert_eq!(
            Budgets::Reference.of(metric, hosted),
            Some(Duration::from_nanos(*budget))
        );
    }
    for metric in HOSTED_RULE_ON_REFERENCE {
        assert_eq!(
            Budgets::Reference.of(metric, hosted),
            Some(Duration::from_millis(6))
        );
    }
    assert_eq!(
        ["hosted", "reference", "measure"],
        [Budgets::Hosted, Budgets::Reference, Budgets::Measure].map(Budgets::label)
    );
}

#[test]
fn every_metric_past_its_budget_is_reported_when_the_gate_ends() {
    let slow = Measurement {
        p50: Duration::from_millis(9),
        p95: Duration::from_millis(10),
        maximum: Duration::from_millis(10),
    };
    let fast = Measurement {
        p50: Duration::from_millis(1),
        p95: Duration::from_millis(1),
        maximum: Duration::from_millis(1),
    };
    for (name, measurement) in [("first", slow), ("within", fast), ("second", slow)] {
        measurement.check_budget_of(Budgets::Hosted, name, Duration::from_millis(4));
    }
    let report = std::panic::catch_unwind(assert_within_budgets)
        .expect_err("two metrics are past their budgets");
    let message = report
        .downcast_ref::<String>()
        .expect("a formatted message");
    assert_eq!(
        message.lines().collect::<Vec<_>>(),
        [
            "first p95 10ms exceeded the hosted budget 6ms",
            "second p95 10ms exceeded the hosted budget 6ms",
        ]
    );
    assert_within_budgets();
}

#[test]
#[should_panic(expected = "none of hosted, reference and measure")]
fn an_unknown_budget_profile_is_refused() {
    Budgets::named(Some("fast"));
}

#[test]
#[should_panic(expected = "has no reference budget")]
fn a_metric_without_a_reference_budget_is_refused_on_the_reference_host() {
    Budgets::Reference.of("not_a_metric", Duration::from_millis(1));
}
