//! Release-profile, real-terminal performance gate for the TUI (`A1-447`).
//!
//! This suite is deliberately ignored during ordinary development. The
//! scheduled/manual scale workflow runs it serially against a release binary,
//! where it measures the process boundary that reducer microbenchmarks cannot:
//! startup feedback, usable content, resident memory and visible interaction.

#![cfg(all(feature = "tui", target_os = "linux"))]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use termlens::Key;

use crate::common;

const SAMPLES: usize = 3;

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

fn assert_samples(name: &str, state: &str, samples: &[Sample], budget: Budget) {
    let feedback = p95_duration(samples.iter().map(|sample| sample.feedback).collect());
    let usable = p95_duration(samples.iter().map(|sample| sample.usable).collect());
    let action = p95_duration(samples.iter().map(|sample| sample.action).collect());
    let search = p95_duration(samples.iter().map(|sample| sample.search).collect());
    let refresh = p95_duration(samples.iter().map(|sample| sample.refresh).collect());
    let rss_kib = p95_u64(samples.iter().map(|sample| sample.rss_kib).collect());
    eprintln!(
        "TUI_PERF fixture={name} cache={state} samples={} feedback_ms={} usable_ms={} action_ms={} search_ms={} refresh_ms={} rss_kib={} budgets_ms={}/{}/{}/{}/{} budget_rss_kib={}",
        samples.len(),
        feedback.as_millis(),
        usable.as_millis(),
        action.as_millis(),
        search.as_millis(),
        refresh.as_millis(),
        rss_kib,
        budget.feedback.as_millis(),
        budget.usable.as_millis(),
        budget.action.as_millis(),
        budget.search.as_millis(),
        budget.refresh.as_millis(),
        budget.rss_kib,
    );
    assert!(
        feedback <= budget.feedback,
        "{name}/{state} first feedback {feedback:?}"
    );
    assert!(usable <= budget.usable, "{name}/{state} usable {usable:?}");
    assert!(action <= budget.action, "{name}/{state} action {action:?}");
    assert!(search <= budget.search, "{name}/{state} search {search:?}");
    assert!(
        refresh <= budget.refresh,
        "{name}/{state} refresh {refresh:?}"
    );
    assert!(
        rss_kib <= budget.rss_kib,
        "{name}/{state} RSS {rss_kib} KiB"
    );
}

fn exercise_fixture(name: &str, tasks: usize, cold: Budget, warm: Budget) -> termlens::Result<()> {
    let fixture = Fixture::new(tasks);
    let mut cold_samples = Vec::with_capacity(SAMPLES);
    let mut warm_samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        cold_samples.push(run_sample(&fixture, "refresh")?);
        warm_samples.push(run_sample(&fixture, "auto")?);
    }
    assert_samples(name, "cold-refresh", &cold_samples, cold);
    assert_samples(name, "warm-hit", &warm_samples, warm);
    Ok(())
}

/// End-to-end startup, interaction and memory budgets. The fixture sizes are
/// stable representatives rather than copies of the changing aplan plan.
#[test]
#[ignore = "release performance gate; run by ci/scripts/scale.sh"]
fn tui_performance_gate_startup_interaction_and_memory() -> termlens::Result<()> {
    exercise_fixture(
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
    )?;
    exercise_fixture(
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
    )?;
    exercise_fixture(
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
    )
}
