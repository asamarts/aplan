//! Release-profile, real-terminal performance gate for the TUI (`A1-447`).
//!
//! This suite is deliberately ignored during ordinary development. The
//! scheduled/manual scale workflow, and `ci/scripts/reference-tui-perf.sh` on the
//! reference host, run it serially against a release binary, where it measures
//! the process boundary that reducer microbenchmarks cannot: startup feedback,
//! usable content, resident memory and visible interaction. The reference host
//! owns the wall-clock budgets (`A2-164`, ADR-0067); a hosted run fails a
//! wall-clock metric only well past them, and holds memory to the same budget
//! everywhere.

#![cfg(all(feature = "tui", target_os = "linux"))]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use termlens::Key;

use crate::common;

const SAMPLES: usize = 3;

/// The budget a wall-clock metric is held to, chosen by `APLAN_TUI_BUDGETS` (`A2-164`,
/// ADR-0067), by the same rule as `aplan-tui`'s in-process gate. The budgets written in
/// [`tui_performance_gate_startup_interaction_and_memory`] are the hosted budgets of 2026-11-18;
/// the reference host's own are in `REFERENCE_BUDGETS_NS`. Memory is not wall-clock and is held to
/// its written budget on every host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Budgets {
    /// A hosted runner, the default: 1.5 times the hosted budget, which fails a gross regression
    /// and not a slower machine.
    Hosted,
    /// The reference host, with the gate pinned to one CPU (`ci/scripts/reference-tui-perf.sh`):
    /// each metric's budget in `REFERENCE_BUDGETS_NS`, or the hosted rule for one
    /// `HOSTED_RULE_ON_REFERENCE` names.
    Reference,
    /// Print every metric and hold no wall-clock metric to a budget: how the reference budgets
    /// are measured.
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
    ("pty_small_cold_usable", 387_749_357),
    ("pty_small_cold_action", 2_604_364),
    ("pty_small_cold_search", 21_543_524),
    ("pty_small_cold_refresh", 129_922_065),
    ("pty_small_warm_usable", 387_831_859),
    ("pty_small_warm_action", 2_538_803),
    ("pty_small_warm_search", 21_882_282),
    ("pty_small_warm_refresh", 127_739_808),
    ("pty_repository_cold_usable", 530_263_253),
    ("pty_repository_cold_action", 6_630_772),
    ("pty_repository_cold_search", 66_175_815),
    ("pty_repository_cold_refresh", 259_229_703),
    ("pty_repository_warm_usable", 404_952_382),
    ("pty_repository_warm_action", 6_633_438),
    ("pty_repository_warm_search", 65_717_684),
    ("pty_repository_warm_refresh", 260_698_635),
    ("pty_large_cold_usable", 3_064_003_363),
    ("pty_large_cold_action", 74_521_870),
    ("pty_large_cold_search", 836_637_035),
    ("pty_large_cold_refresh", 3_486_573_022),
    ("pty_large_warm_usable", 2_431_845_185),
    ("pty_large_warm_action", 73_163_518),
    ("pty_large_warm_search", 818_382_982),
    ("pty_large_warm_refresh", 2_609_985_594),
];

/// The metrics the reference host holds to the hosted rule (maintainer decisions, 2026-11-19): every
/// first feedback, about 7 ms and the slowest of three launches, of which three spread across the
/// five runs by more than the agreed 5% and one failed a passing revision once in three reference
/// runs. `A2-180` makes them measurable there.
const HOSTED_RULE_ON_REFERENCE: &[&str] = &[
    "pty_small_cold_feedback",
    "pty_small_warm_feedback",
    "pty_repository_cold_feedback",
    "pty_repository_warm_feedback",
    "pty_large_cold_feedback",
    "pty_large_warm_feedback",
];

#[derive(Debug)]
struct Fixture {
    directory: tempfile::TempDir,
    cache: PathBuf,
    tasks: usize,
}

impl Fixture {
    fn new(tasks: usize) -> Self {
        let directory = tempfile::Builder::new()
            .prefix("aplan-tui-performance-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("performance fixture");
        let plan = directory.path().join("plan/tasks");
        std::fs::create_dir_all(&plan).expect("task directory");
        std::fs::create_dir_all(directory.path().join("src")).expect("source directory");
        std::fs::write(directory.path().join("src/lib.rs"), "pub fn fixture() {}\n")
            .expect("source file");
        for index in 0..tasks {
            std::fs::write(
                plan.join(format!("T-{index:05}.md")),
                format!(
                    "---\nid: T-{index:05}\ntitle: Generated task {index:05}\nphase: 1\n\
                     status: todo\neffort_days: 1.0\nblocked_by: []\npaths: [src/**]\n\
                     tags: [cross-cutting]\n---\n\n## Why\n\nPerformance fixture {index}.\n"
                ),
            )
            .expect("task file");
        }
        let cache = directory.path().join("cache");
        std::fs::create_dir_all(&cache).expect("cache directory");
        Self {
            directory,
            cache,
            tasks,
        }
    }

    fn root(&self) -> &Path {
        self.directory.path()
    }
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    feedback: Duration,
    usable: Duration,
    action: Duration,
    search: Duration,
    refresh: Duration,
    rss_kib: u64,
}

#[derive(Debug, Clone, Copy)]
struct Budget {
    feedback: Duration,
    usable: Duration,
    action: Duration,
    search: Duration,
    refresh: Duration,
    rss_kib: u64,
}

fn rss_kib(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .expect("read TUI process status")
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")?
                .split_ascii_whitespace()
                .next()?
                .parse()
                .ok()
        })
        .expect("VmRSS in process status")
}

fn timing_counter(receipt: &str, name: &str) -> Option<usize> {
    let prefix = format!("{name}=");
    receipt.split_ascii_whitespace().find_map(|field| {
        field
            .strip_prefix(&prefix)
            .and_then(|value| value.parse().ok())
    })
}

fn run_sample(fixture: &Fixture, cache_mode: &str) -> termlens::Result<Sample> {
    let args = [
        "--timings",
        "--progress",
        "never",
        "--cache",
        cache_mode,
        "tui",
        "--no-config",
        "--ascii",
        "--reduced-motion",
        "--root",
        "plan",
    ];
    let started = Instant::now();
    let mut terminal =
        common::tui_terminal_with_cache(fixture.root(), &fixture.cache, 100, 24, &args)?;
    terminal.wait_until_for(
        |screen| {
            screen.alternate_screen()
                && !screen.cursor_visible()
                && screen.contains("aplan - loading identified inputs")
        },
        Duration::from_secs(10),
    )?;
    let feedback = started.elapsed();
    terminal.wait_until_for(
        |screen| screen.contains("Overview - where attention goes next"),
        Duration::from_secs(30),
    )?;
    let usable = started.elapsed();
    let pid = terminal.pid().expect("Linux PTY child pid");
    let rss_kib = rss_kib(pid);

    let action_started = Instant::now();
    terminal.send(Key::Char('2'))?;
    terminal.wait_until_for(
        |screen| screen.contains("Tasks - what is true about this work"),
        Duration::from_secs(5),
    )?;
    let action = action_started.elapsed();

    let query = format!("task {:05}", fixture.tasks - 1);
    let task = fixture
        .root()
        .join(format!("plan/tasks/T-{:05}.md", fixture.tasks - 1));
    let body = std::fs::read_to_string(&task).expect("search task");
    let generated = format!("Generated task {:05}", fixture.tasks - 1);
    let refreshed = format!("Refreshed task {:05}", fixture.tasks - 1);
    let (expected, replacement) = if body.contains(&generated) {
        (generated, refreshed)
    } else {
        (refreshed, generated)
    };
    let search_started = Instant::now();
    terminal.send(Key::Char('/'))?;
    terminal.wait_until(|screen| screen.contains("Search tasks"))?;
    for character in query.chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until_for(
        |screen| {
            screen.contains(&format!("Search: {query}"))
                && screen.contains(&format!("1/{} tasks shown", fixture.tasks))
        },
        Duration::from_secs(10),
    )?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| !screen.contains("Search tasks") && screen.contains(&expected))?;
    let search = search_started.elapsed();

    std::fs::write(&task, body.replace(&expected, &replacement)).expect("edit refresh task");
    let refresh_started = Instant::now();
    terminal.send(Key::Char('r'))?;
    terminal.wait_until_for(
        |screen| screen.contains(&replacement),
        Duration::from_secs(30),
    )?;
    let refresh = refresh_started.elapsed();

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    let receipt = terminal.screen().logical_text();
    let hits = timing_counter(&receipt, "cache_hits")
        .unwrap_or_else(|| panic!("{cache_mode} sample emitted no cache counter: {receipt}"));
    if cache_mode == "auto" {
        assert!(hits > 0, "warm sample had no cache hit: {receipt}");
    } else {
        assert_eq!(hits, 0, "forced-refresh sample used a cache hit: {receipt}");
    }
    Ok(Sample {
        feedback,
        usable,
        action,
        search,
        refresh,
        rss_kib,
    })
}

fn p95_duration(mut values: Vec<Duration>) -> Duration {
    values.sort_unstable();
    values[values.len() - 1]
}

fn p95_u64(mut values: Vec<u64>) -> u64 {
    values.sort_unstable();
    values[values.len() - 1]
}

fn p50_duration(mut values: Vec<Duration>) -> Duration {
    values.sort_unstable();
    values[values.len() / 2]
}

/// Reads one wall-clock measurement from a sample.
type Field = fn(&Sample) -> Duration;

/// Print one wall-clock metric, as the in-process gate prints its own, and record it in
/// `exceeded` if its p95 is past the budget of this host.
fn check_duration(
    budgets: Budgets,
    metric: &str,
    values: Vec<Duration>,
    hosted: Duration,
    exceeded: &mut Vec<String>,
) {
    let budget = budgets.of(metric, hosted);
    let p50 = p50_duration(values.clone());
    let p95 = p95_duration(values);
    eprintln!(
        "TUI_PERF metric={metric} p50_us={} p95_us={} max_us={} budget_us={} budgets={} p95_ns={}",
        p50.as_micros(),
        p95.as_micros(),
        p95.as_micros(),
        budget.map_or_else(|| "-".to_owned(), |budget| budget.as_micros().to_string()),
        budgets.label(),
        p95.as_nanos()
    );
    if let Some(budget) = budget
        && p95 > budget
    {
        exceeded.push(format!(
            "{metric} p95 {p95:?} exceeded the {} budget {budget:?}",
            budgets.label()
        ));
    }
}

fn check_samples(
    budgets: Budgets,
    name: &str,
    state: &str,
    samples: &[Sample],
    budget: Budget,
    exceeded: &mut Vec<String>,
) {
    let metric = |field: &str| format!("pty_{name}_{state}_{field}");
    let fields: [(&str, Field, Duration); 5] = [
        ("feedback", |sample| sample.feedback, budget.feedback),
        ("usable", |sample| sample.usable, budget.usable),
        ("action", |sample| sample.action, budget.action),
        ("search", |sample| sample.search, budget.search),
        ("refresh", |sample| sample.refresh, budget.refresh),
    ];
    for (field, value, hosted) in fields {
        let values = samples.iter().map(value).collect();
        check_duration(budgets, &metric(field), values, hosted, exceeded);
    }
    let rss_kib = p95_u64(samples.iter().map(|sample| sample.rss_kib).collect());
    eprintln!(
        "TUI_PERF metric={} p95_kib={rss_kib} budget_kib={}",
        metric("rss"),
        budget.rss_kib
    );
    if rss_kib > budget.rss_kib {
        exceeded.push(format!(
            "{} {rss_kib} KiB exceeded the budget {} KiB",
            metric("rss"),
            budget.rss_kib
        ));
    }
}

fn exercise_fixture(
    budgets: Budgets,
    name: &str,
    tasks: usize,
    cold: Budget,
    warm: Budget,
    exceeded: &mut Vec<String>,
) -> termlens::Result<()> {
    let fixture = Fixture::new(tasks);
    let mut cold_samples = Vec::with_capacity(SAMPLES);
    let mut warm_samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        cold_samples.push(run_sample(&fixture, "refresh")?);
        warm_samples.push(run_sample(&fixture, "auto")?);
    }
    check_samples(budgets, name, "cold", &cold_samples, cold, exceeded);
    check_samples(budgets, name, "warm", &warm_samples, warm, exceeded);
    Ok(())
}

/// End-to-end startup, interaction and memory budgets. The fixture sizes are
/// stable representatives rather than copies of the changing aplan plan.
#[test]
#[ignore = "release performance gate; run by ci/scripts/scale.sh"]
fn tui_performance_gate_startup_interaction_and_memory() -> termlens::Result<()> {
    let budgets = Budgets::from_env();
    let mut exceeded = Vec::new();
    exercise_fixture(
        budgets,
        "small",
        32,
        Budget {
            feedback: Duration::from_millis(250),
            usable: Duration::from_millis(750),
            action: Duration::from_millis(100),
            search: Duration::from_millis(250),
            refresh: Duration::from_millis(1_000),
            rss_kib: 64 * 1_024,
        },
        Budget {
            usable: Duration::from_millis(500),
            ..Budget {
                feedback: Duration::from_millis(250),
                usable: Duration::from_millis(750),
                action: Duration::from_millis(100),
                search: Duration::from_millis(250),
                refresh: Duration::from_millis(1_000),
                rss_kib: 64 * 1_024,
            }
        },
        &mut exceeded,
    )?;
    exercise_fixture(
        budgets,
        "repository",
        700,
        Budget {
            feedback: Duration::from_millis(250),
            usable: Duration::from_secs(1),
            action: Duration::from_millis(150),
            search: Duration::from_millis(750),
            refresh: Duration::from_secs(2),
            rss_kib: 128 * 1_024,
        },
        Budget {
            usable: Duration::from_millis(750),
            ..Budget {
                feedback: Duration::from_millis(250),
                usable: Duration::from_secs(1),
                action: Duration::from_millis(150),
                search: Duration::from_millis(750),
                refresh: Duration::from_secs(2),
                rss_kib: 128 * 1_024,
            }
        },
        &mut exceeded,
    )?;
    exercise_fixture(
        budgets,
        "large",
        10_000,
        Budget {
            feedback: Duration::from_millis(250),
            usable: Duration::from_secs(6),
            action: Duration::from_millis(250),
            search: Duration::from_millis(2_500),
            refresh: Duration::from_secs(6),
            rss_kib: 320 * 1_024,
        },
        Budget {
            usable: Duration::from_millis(4_500),
            refresh: Duration::from_millis(4_500),
            ..Budget {
                feedback: Duration::from_millis(250),
                usable: Duration::from_secs(6),
                action: Duration::from_millis(250),
                search: Duration::from_millis(2_500),
                refresh: Duration::from_secs(6),
                rss_kib: 320 * 1_024,
            }
        },
        &mut exceeded,
    )?;
    assert!(exceeded.is_empty(), "{}", exceeded.join("\n"));
    Ok(())
}

/// Not a measurement: the budget each host holds a wall-clock metric to, which an ordinary test
/// run checks.
#[test]
fn a_wall_clock_metric_is_held_to_the_budget_of_its_host() {
    let hosted = Duration::from_millis(250);
    assert_eq!(Budgets::named(None), Budgets::Hosted);
    assert_eq!(Budgets::named(Some("")), Budgets::Hosted);
    assert_eq!(Budgets::named(Some("reference")), Budgets::Reference);
    assert_eq!(Budgets::named(Some("measure")), Budgets::Measure);
    assert_eq!(
        Budgets::Hosted.of("pty_small_cold_feedback", hosted),
        Some(Duration::from_millis(375))
    );
    assert_eq!(Budgets::Measure.of("pty_small_cold_feedback", hosted), None);
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
            Some(Duration::from_millis(375))
        );
    }
    let mut exceeded = Vec::new();
    for (metric, millis) in [("first", 400), ("within", 300), ("second", 500)] {
        check_duration(
            Budgets::Hosted,
            metric,
            vec![Duration::from_millis(millis); SAMPLES],
            hosted,
            &mut exceeded,
        );
    }
    check_duration(
        Budgets::Measure,
        "measured",
        vec![Duration::from_secs(9)],
        hosted,
        &mut exceeded,
    );
    assert_eq!(
        exceeded,
        [
            "first p95 400ms exceeded the hosted budget 375ms",
            "second p95 500ms exceeded the hosted budget 375ms",
        ]
    );
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

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// One reference run as `ci/scripts/reference-tui-perf.sh` writes it, with each metric's p95.
fn reference_run(directory: &Path, name: &str, source: &str, metrics: &[(&str, u64)]) -> PathBuf {
    let mut text = format!(
        "REFERENCE commit={name} source={source} budgets=measure cpu=2 max_load=0.25 \
         rustc=rustc_1.99.1\n"
    );
    for (metric, p95_ns) in metrics {
        writeln!(
            text,
            "TUI_PERF metric={metric} p50_us=1 p95_us=1 max_us=1 budget_us=- budgets=measure \
             p95_ns={p95_ns}"
        )
        .expect("write to a String");
    }
    text.push_str("TUI_PERF metric=pty_small_cold_rss p95_kib=1 budget_kib=2\n");
    let path = directory.join(name);
    std::fs::write(&path, text).expect("write a reference run");
    path
}

fn summarize(runs: &[PathBuf]) -> (bool, String) {
    let output = std::process::Command::new("python3")
        .arg("-I")
        .arg(workspace().join("ci/scripts/reference-tui-budgets.py"))
        .arg("--rust")
        .args(runs)
        .output()
        .expect("run the reference summarizer");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

/// Reference runs give budgets only when they measure one source, and a metric only when its runs
/// agree within the spread agreed for `A2-164`: 5% of the smallest p95, or 0.1 ms for a metric
/// under 2 ms. A budget is the worst p95 times 1.25, and at least the worst plus 0.1 ms. Each run
/// here has a commit of its own, as runs on either side of a rebase do.
#[test]
fn reference_runs_give_budgets_only_when_they_agree() {
    let directory = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("directory");
    let run = |name: &str, source: &str, slow: u64, small: u64| {
        reference_run(
            directory.path(),
            name,
            source,
            &[
                ("model_and_graph_10000", slow),
                ("graph_hit_sparse", small),
                ("pty_small_cold_usable", 40_000_000),
                ("pty_small_cold_feedback", small),
            ],
        )
    };
    let agreeing = [
        run("1", "abc", 700_000_000, 1_000),
        run("2", "abc", 734_000_000, 99_000),
    ];
    let (agreed, report) = summarize(&agreeing);
    assert!(agreed, "{report}");
    assert!(
        report.contains("(\"model_and_graph_10000\", 917_500_000),")
            && report.contains("(\"graph_hit_sparse\", 199_000),")
            && report.contains("(\"pty_small_cold_usable\", 50_000_000),")
            && !report.contains("rss"),
        "{report}"
    );
    // A first feedback keeps the hosted rule by the maintainer's decision, however its runs agree.
    assert!(
        report.contains(
            "const HOSTED_RULE_ON_REFERENCE: &[&str] = &[\n    \"pty_small_cold_feedback\",\n];"
        ) && !report.contains("(\"pty_small_cold_feedback\","),
        "{report}"
    );

    let (agreed, report) = summarize(&[agreeing[0].clone(), run("3", "abc", 736_000_000, 1_000)]);
    assert!(
        !agreed && report.contains("WIDE model_and_graph_10000"),
        "{report}"
    );
    let (agreed, report) = summarize(&[agreeing[0].clone(), run("4", "abc", 700_000_000, 102_000)]);
    assert!(
        !agreed && report.contains("WIDE graph_hit_sparse"),
        "{report}"
    );
    assert!(
        report.contains(
            "const HOSTED_RULE_ON_REFERENCE: &[&str] = &[\n    \"graph_hit_sparse\",\n];"
        ) && !report.contains("(\"graph_hit_sparse\","),
        "a metric that does not agree keeps the hosted rule: {report}"
    );

    let partial = reference_run(directory.path(), "5", "abc", &[("graph_hit_sparse", 1_000)]);
    let (agreed, report) = summarize(&[agreeing[0].clone(), partial]);
    assert!(
        !agreed && report.contains("model_and_graph_10000: missing from run 2"),
        "{report}"
    );
    assert!(!report.contains("REFERENCE_BUDGETS_NS"), "{report}");
    let (agreed, report) = summarize(&[agreeing[0].clone(), run("6", "abd", 700_000_000, 1_000)]);
    assert!(!agreed && report.contains("differ in source"), "{report}");
}

/// The reference script needs the host and its queue named, and knows two budget profiles.
#[test]
fn the_reference_script_refuses_to_run_without_its_host_or_with_an_unknown_profile() {
    let script = workspace().join("ci/scripts/reference-tui-perf.sh");
    for (arguments, expected) in [
        (
            &[][..],
            "set APLAN_REFERENCE_HOST and APLAN_REFERENCE_QUEUE",
        ),
        (
            &["--budgets", "hosted"][..],
            "usage: ci/scripts/reference-tui-perf.sh",
        ),
    ] {
        let output = std::process::Command::new("bash")
            .arg(&script)
            .args(arguments)
            .env_remove("APLAN_REFERENCE_HOST")
            .env_remove("APLAN_REFERENCE_QUEUE")
            .output()
            .expect("run the reference script");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert!(stderr.contains(expected), "{stderr}");
    }
}
