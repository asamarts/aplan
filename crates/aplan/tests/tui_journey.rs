//! Real-terminal boundary checks for the interactive interface (`A1-424`).

#![cfg(feature = "tui")]

use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Command;
#[cfg(unix)]
use std::time::{Duration, Instant};

use aplan_app::{ContextInputs, PlanContext, Trust};
use aplan_core::{Options, Verdict};
use termlens::Key;

use crate::common;

fn frame_border_reaches_right_edge(screen: &termlens::Screen) -> bool {
    let right = screen.cols().saturating_sub(1);
    (0..screen.rows()).any(|row| {
        screen
            .cell(row, right)
            .is_some_and(|cell| matches!(cell.contents(), "+" | "\u{2510}"))
    })
}

fn repainted_at(screen: &termlens::Screen, columns: u16, rows: u16) -> bool {
    screen.cols() == columns && screen.rows() == rows && frame_border_reaches_right_edge(screen)
}

fn send_down(terminal: &mut termlens::Terminal, count: usize) -> termlens::Result<()> {
    for _ in 0..count {
        terminal.send(Key::Down)?;
    }
    Ok(())
}

#[derive(Debug)]
struct IdentifiedPlanFixture {
    directory: tempfile::TempDir,
    plan_root: PathBuf,
}

impl IdentifiedPlanFixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture directory");
        let plan_root = directory.path().join("plan");
        let tasks = plan_root.join("tasks");
        std::fs::create_dir_all(&tasks).expect("task directory");
        std::fs::create_dir_all(directory.path().join("src")).expect("declared source path");
        std::fs::write(directory.path().join("src/lib.rs"), "pub fn fixture() {}\n")
            .expect("declared source file");
        std::fs::write(
            tasks.join("T-001.md"),
            "---\n\
             id: T-001\n\
             title: Inspect the plan\n\
             phase: 1\n\
             status: todo\n\
             effort_days: 1.0\n\
             blocked_by: []\n\
             paths: [src/**]\n\
             tags: [cross-cutting]\n\
             ---\n\n\
             ## Why\n\n\
             Give the terminal journey one identified task.\n",
        )
        .expect("task file");
        Self {
            directory,
            plan_root,
        }
    }

    fn directory(&self) -> &Path {
        self.directory.path()
    }

    fn context(&self) -> PlanContext {
        PlanContext::new(
            aplan_core::load_plan_identified(&self.plan_root),
            ContextInputs {
                repository_root: self.directory().to_path_buf(),
                configuration_file: None,
                trust: Trust::Hermetic,
                options: Options::default(),
                verdict: Verdict::default(),
            },
        )
    }

    fn enable_diagnostic_workbench(&self) {
        let task = self.plan_root.join("tasks/T-001.md");
        let body = std::fs::read_to_string(&task).expect("task");
        std::fs::write(&task, body.replace("effort_days: 1.0\n", "")).expect("remove effort");
        std::fs::write(
            self.directory().join("PLAN.md"),
            "T-999 is not a task in this plan.\n",
        )
        .expect("document");
        std::fs::write(
            self.directory().join(".aplan.yml"),
            "root: plan\ndocs:\n  paths: [PLAN.md]\n",
        )
        .expect("config");
    }

    fn enable_graph_explorer(&self) {
        for (id, title, phase, blocked_by) in [
            ("T-002", "Follow the dependency", 1, "[T-001]"),
            ("T-003", "Finish the dependency", 2, "[T-002]"),
        ] {
            std::fs::write(
                self.plan_root.join(format!("tasks/{id}.md")),
                format!(
                    "---\nid: {id}\ntitle: {title}\nphase: {phase}\nstatus: todo\n\
                     effort_days: 1.0\nblocked_by: {blocked_by}\npaths: [src/**]\n\
                     tags: [cross-cutting]\n---\n"
                ),
            )
            .expect("graph task");
        }
        std::fs::write(
            self.plan_root.join("tasks/M-v1.md"),
            "---\nid: M-v1\ntitle: graph release\nphase: 1\nstatus: todo\n\
             blocked_by: [T-001, T-002]\ntags: [milestone, v1]\n---\n",
        )
        .expect("graph milestone");
        std::fs::write(
            self.plan_root.join("tasks/M-v2.md"),
            "---\nid: M-v2\ntitle: second graph release\nphase: 2\nstatus: todo\n\
             blocked_by: [T-003]\ntags: [milestone, v2]\n---\n",
        )
        .expect("second graph milestone");
    }

    fn enable_work_discovery(&self) {
        std::fs::write(
            self.plan_root.join("tasks/T-002.md"),
            "---\n\
             id: T-002\n\
             title: Follow the ready task\n\
             phase: 1\n\
             status: todo\n\
             effort_days: 1.0\n\
             blocked_by: [T-001, M-work]\n\
             paths: [src/**]\n\
             tags: [cross-cutting]\n\
             ---\n",
        )
        .expect("dependent task");
        std::fs::write(
            self.plan_root.join("tasks/M-work.md"),
            "---\n\
             id: M-work\n\
             title: Folded work cut\n\
             phase: 1\n\
             status: todo\n\
             blocked_by: [PARKED]\n\
             tags: [milestone, work]\n\
             ---\n",
        )
        .expect("work cut");
        std::fs::write(
            self.plan_root.join("tasks/PARKED.md"),
            "---\n\
             id: PARKED\n\
             title: Revisit the parked member\n\
             phase: 1\n\
             status: blocked\n\
             effort_days: 1.0\n\
             blocked_by: []\n\
             paths: [src/**]\n\
             revisit_at: 1970-01-01T00:00:10Z\n\
             tags: []\n\
             ---\n",
        )
        .expect("parked cut member");
    }

    fn enable_mutation_journey(&self) {
        std::fs::write(
            self.plan_root.join("tasks/T-002.md"),
            "---\n\
             id: T-002\n\
             title: Follow the first task\n\
             phase: 1\n\
             status: todo\n\
             effort_days: 1.0\n\
             blocked_by: []\n\
             paths: [src/**]\n\
             tags: [cross-cutting]\n\
             ---\n",
        )
        .expect("second mutation task");
        for args in [
            &["init", "-q"][..],
            &["config", "user.email", "test@example.com"][..],
            &["config", "user.name", "Fixture User"][..],
            &["add", "."][..],
            &["commit", "-qm", "fixture"][..],
        ] {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(self.directory())
                .output()
                .expect("git setup");
            assert!(output.status.success(), "git {args:?}: {output:?}");
        }
    }

    fn enable_capture_journey(&self) {
        std::fs::write(
            self.directory().join(".aplan.yml"),
            "root: plan\ncapture:\n  profiles:\n    triage:\n      kind: idea\n      labels: [terminal]\n      body: Captured from the terminal journey.\n      promotion:\n        phase: 1\n        effort_days: 1.0\n        tags: [cross-cutting]\n        paths: [src/**]\n",
        )
        .expect("capture configuration");
        for args in [
            &["init", "-q"][..],
            &["config", "user.email", "test@example.com"][..],
            &["config", "user.name", "Fixture User"][..],
            &["add", "."][..],
            &["commit", "-qm", "fixture"][..],
        ] {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(self.directory())
                .output()
                .expect("git setup");
            assert!(output.status.success(), "git {args:?}: {output:?}");
        }
    }

    fn enable_test_catalogue(&self) {
        std::fs::write(
            self.directory().join(".aplan.yml"),
            "root: plan\ntest:\n  name: selector\n  list: [printf, 'suite::proof\\n']\n  run: ['true', '{test}']\n",
        )
        .expect("test catalogue config");
    }
}

fn open_mutation(
    terminal: &mut termlens::Terminal,
    query: &str,
    visible: &str,
) -> termlens::Result<()> {
    terminal.send(Key::Ctrl('p'))?;
    terminal.wait_until(|screen| screen.contains("Action:"))?;
    for character in query.chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until(|screen| screen.contains(query))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until_for(|screen| screen.contains(visible), Duration::from_secs(20))
}

fn review_and_confirm(terminal: &mut termlens::Terminal, _title: &str) -> termlens::Result<()> {
    terminal.send(Key::BackTab)?;
    terminal.send(Key::Enter)?;
    terminal.wait_until_for(
        |screen| screen.contains("PREVIEW:"),
        Duration::from_secs(20),
    )?;
    terminal.send(Key::BackTab)?;
    terminal.send(Key::Enter)?;
    terminal.wait_until_for(
        |screen| screen.contains("Verdict: COMMITTED"),
        Duration::from_secs(20),
    )?;
    terminal.send(Key::Esc)?;
    terminal
        .wait_until(|screen| !screen.contains("PREVIEW:") && !screen.contains("Verdict: COMMITTED"))
}

fn assert_capture_promoted(fixture: &IdentifiedPlanFixture) {
    let task =
        std::fs::read_to_string(fixture.plan_root.join("tasks/T-002.md")).expect("promoted task");
    assert!(task.contains("title: Terminal intake"), "{task}");
    assert!(task.contains("tags:\n- cross-cutting"), "{task}");
    assert!(task.contains("paths:\n- src/**"), "{task}");
    let inbox = std::fs::read_dir(fixture.plan_root.join(".inbox"))
        .expect("capture inbox")
        .filter_map(Result::ok)
        .map(|entry| std::fs::read_to_string(entry.path()).expect("capture record"))
        .collect::<Vec<_>>();
    assert_eq!(inbox.len(), 1, "{inbox:?}");
    assert!(inbox[0].contains("state: promoted"), "{}", inbox[0]);
    assert!(inbox[0].contains("promoted_to: T-002"), "{}", inbox[0]);
    assert!(inbox[0].contains("origin: tui"), "{}", inbox[0]);
}

fn quick_capture_from_overview(terminal: &mut termlens::Terminal) -> termlens::Result<()> {
    terminal.send(Key::Ctrl('n'))?;
    terminal.wait_until(|screen| screen.contains("FORM: Quick capture"))?;
    for character in "Terminal intake".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.send(Key::Tab)?;
    terminal.send(Key::Tab)?;
    for character in "triage".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.send(Key::BackTab)?;
    terminal.send(Key::BackTab)?;
    terminal.send(Key::BackTab)?;
    terminal.send(Key::Enter)?;
    terminal.wait_until_for(
        |screen| screen.contains("PREVIEW: Quick capture") && screen.contains("origin: tui"),
        Duration::from_secs(20),
    )?;
    terminal.send(Key::BackTab)?;
    terminal.send(Key::Enter)?;
    terminal.wait_until_for(
        |screen| screen.contains("Verdict: COMMITTED"),
        Duration::from_secs(20),
    )?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| {
        screen.contains("Overview - where attention goes next")
            && !screen.contains("Verdict: COMMITTED")
    })
}

#[test]
fn tui_fixture_builder_constructs_an_identified_plan_without_a_terminal() {
    let fixture = IdentifiedPlanFixture::new();
    let context = fixture.context();
    assert_eq!(context.plan().tasks.len(), 1);
    assert_eq!(context.plan().tasks[0].task.id, "T-001");
    assert_eq!(context.identity().source, "working_tree");
    assert_eq!(context.identity().sha256.len(), 64);
    assert_eq!(context.identity().trust, Trust::Hermetic);
}

#[derive(Debug, PartialEq, Eq)]
struct RepositoryState {
    head: Vec<u8>,
    status: Vec<u8>,
    tracked_diff: Vec<u8>,
}

fn git_observation(repository: &Path, args: &[&str]) -> Vec<u8> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repository)
        .output()
        .expect("git observation runs");
    assert!(
        output.status.success(),
        "git {} failed in {}: {}",
        args.join(" "),
        repository.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn repository_state(repository: &Path) -> RepositoryState {
    RepositoryState {
        head: git_observation(repository, &["rev-parse", "HEAD"]),
        status: git_observation(
            repository,
            &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
        ),
        tracked_diff: git_observation(
            repository,
            &["diff", "--no-ext-diff", "--binary", "HEAD", "--"],
        ),
    }
}

/// Reproducible dogfood driver for `A1-435`. The default suite cannot assume
/// sibling repositories exist, so the evaluator supplies a platform path list
/// and runs this ignored test explicitly. Each repository is opened through a
/// real PTY and every read-only top-level destination must paint and quit
/// cleanly; neither plan bytes nor observable repository state may move.
#[test]
#[ignore = "set APLAN_TUI_REAL_PLANS to at least three plan repository paths"]
fn configured_real_plans_open_every_read_destination_in_a_real_terminal() -> termlens::Result<()> {
    let configured = std::env::var_os("APLAN_TUI_REAL_PLANS")
        .expect("APLAN_TUI_REAL_PLANS must name the repositories to evaluate");
    let repositories = std::env::split_paths(&configured).collect::<Vec<_>>();
    assert!(
        repositories.len() >= 3,
        "real-plan evaluation requires at least three repositories"
    );
    let mut distinct = std::collections::BTreeSet::new();

    for repository in repositories {
        let canonical = repository.canonicalize().expect("real-plan repository");
        assert!(distinct.insert(canonical.clone()), "duplicate real plan");
        let plan_before = common::census::plan_bytes(&canonical);
        let repository_before = repository_state(&canonical);
        let started = Instant::now();
        let mut terminal = common::tui_terminal(&canonical, 120, 30, &["tui"])?;
        terminal.wait_until_for(
            |screen| screen.contains("Overview - where attention goes next"),
            Duration::from_secs(20),
        )?;
        for (key, required, corroborating) in [
            ('2', "Tasks - what is true about this work", None),
            ('3', "Finding ", Some("Esc back to overview")),
            ('4', "Graph - how work is connected", None),
            ('5', "Reports - which aggregate needs a decision", None),
            ('6', "Inbox - which captured intake needs review", None),
            ('1', "Overview - where attention goes next", None),
        ] {
            terminal.send(Key::Char(key))?;
            terminal.wait_until_for(
                |screen| {
                    screen.contains(required)
                        && corroborating.is_none_or(|marker| screen.contains(marker))
                },
                Duration::from_secs(20),
            )?;
        }
        terminal.send(Key::Char('q'))?;
        assert!(terminal.wait_exit()?.success());
        assert_terminal_restored(&terminal.screen());
        assert_eq!(
            common::census::plan_bytes(&canonical),
            plan_before,
            "read-only TUI journey wrote {}",
            canonical.display()
        );
        assert_eq!(
            repository_state(&canonical),
            repository_before,
            "read-only TUI journey changed repository state in {}",
            canonical.display()
        );
        eprintln!(
            "real-plan TUI smoke {}: {:?}",
            canonical.display(),
            started.elapsed()
        );
    }
    Ok(())
}

#[test]
fn launch_input_resize_and_clean_quit_restore_terminal_modes() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 100, 24, &args)?;

    terminal.wait_until(|screen| {
        screen.contains("Overview - where attention goes next")
            && screen.alternate_screen()
            && screen.bracketed_paste()
            && !screen.cursor_visible()
    })?;
    let initial = terminal.screen();
    // Crossterm emits SGR 59 to reset underline colour. Termlens records that
    // harmless style reset as unsupported, so keep the allow-list exact: a
    // second sequence is evidence that the emulated picture may be incomplete.
    let unsupported = initial.unsupported();
    assert!(
        unsupported.into_iter().all(|sequence| sequence == "^[[59m"),
        "the foundation shell emitted sequence(s) the PTY emulator could not interpret: \
         {:?}",
        initial.unsupported()
    );

    terminal.send(Key::Char('2'))?;
    terminal.wait_until(|screen| screen.contains("Tasks - what is true about this work"))?;

    terminal.send(Key::Char('/'))?;
    for character in "inspect".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until(|screen| {
        screen.contains("Search tasks")
            && screen.contains("Search: inspect")
            && screen.contains("1/1 tasks shown")
    })?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| {
        !screen.contains("Search tasks") && screen.contains("Inspect the plan")
    })?;

    terminal.send(Key::Ctrl('p'))?;
    terminal.wait_until(|screen| screen.contains("Action:"))?;
    terminal.send(Key::Char('q'))?;
    terminal.wait_until(|screen| screen.contains("Action: q"))?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| {
        !screen.contains("Action:") && screen.contains("Tasks - what is true about this work")
    })?;

    terminal.resize(47, 13)?;
    terminal.wait_until(|screen| {
        screen.cols() == 47
            && screen.rows() == 13
            && screen.contains("TERMINAL TOO SMALL")
            && screen.contains("Current: 47x13. Required: 48x14.")
    })?;
    terminal.resize(100, 24)?;
    terminal.wait_until(|screen| {
        screen.cols() == 100
            && screen.rows() == 24
            && screen.contains("Tasks - what is true about this work")
    })?;

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    let restored = terminal.screen();
    assert!(!restored.alternate_screen());
    assert!(!restored.bracketed_paste());
    assert!(restored.cursor_visible());
    assert!(
        restored
            .unsupported()
            .into_iter()
            .all(|sequence| sequence == "^[[59m"),
        "unexpected terminal sequence during the journey: {:?}",
        restored.unsupported()
    );
    Ok(())
}

/// Proof for `tui.simultaneous_resize_and_input_are_not_lost`: stop the child
/// before making both its SIGWINCH source and terminal descriptor ready. The
/// first event poll after SIGCONT must consume the queued `q` even when it also
/// reports the resize; an edge-triggered source which returns after SIGWINCH
/// strands that already-readable byte and never exits.
#[cfg(target_os = "linux")]
#[test]
fn simultaneous_resize_and_quit_never_strand_terminal_input() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 100, 24, &args)?;

    terminal.wait_until(|screen| {
        screen.contains("Overview - where attention goes next")
            && screen.alternate_screen()
            && !screen.cursor_visible()
    })?;
    let pid = terminal.pid().expect("Linux child pid");
    send_signal(pid, "STOP");
    wait_for_stopped_process(pid);

    terminal.resize(99, 23)?;
    terminal.send(Key::Char('q'))?;
    send_signal(pid, "CONT");
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    Ok(())
}

/// Proof for `tui.visual_language_is_semantic_and_deterministic`: launch-time
/// appearance controls reach the real binary before its first PTY frame.
#[test]
fn appearance_controls_apply_from_the_first_real_terminal_frame() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    let args = [
        "tui",
        "--no-config",
        "--root",
        "plan",
        "--theme",
        "monochrome",
        "--ascii",
        "--reduced-motion",
    ];
    let mut terminal = common::tui_terminal(fixture.directory(), 100, 24, &args)?;
    terminal.wait_until(|screen| {
        screen.contains("Overview - where attention goes next")
            && screen.cell(0, 0).is_some_and(|cell| cell.contents() == "+")
    })?;
    assert!(
        terminal.screen().to_string().is_ascii(),
        "--ascii must apply before the first complete frame"
    );

    terminal.send(Key::Ctrl('p'))?;
    for character in "theme".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until(|screen| screen.contains("theme (monochrome)"))?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| {
        !screen.contains("Action:") && screen.contains("Overview - where attention goes next")
    })?;
    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    Ok(())
}

/// Proof for `tui.action_discovery_is_closed_and_contextual`: the built binary
/// searches product command aliases, keeps CLI-only examples inert, routes the
/// guided facts command to About and restores the terminal without writes.
#[test]
fn action_discovery_guides_boundaries_and_native_routes_in_a_real_terminal() -> termlens::Result<()>
{
    let fixture = IdentifiedPlanFixture::new();
    let before = common::census::plan_bytes(fixture.directory());
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 120, 34, &args)?;
    terminal.wait_until_for(
        |screen| screen.contains("Overview - where attention goes next"),
        Duration::from_secs(20),
    )?;

    terminal.send(Key::Ctrl('p'))?;
    for character in "export-roadmap-json".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal
        .wait_until(|screen| screen.contains("aplan export-roadmap-json --check roadmap.json"))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| {
        screen.contains("Command palette")
            && screen.contains("aplan export-roadmap-json --check roadmap.json")
            && !screen.contains("Preparing")
    })?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| !screen.contains("Command palette"))?;

    terminal.send(Key::Ctrl('p'))?;
    for character in "--no-traces".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until(|screen| screen.contains("aplan verify --no-traces"))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| {
        screen.contains("Command palette")
            && screen.contains("aplan verify --no-traces")
            && !screen.contains("RUN VERIFY")
    })?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| !screen.contains("Command palette"))?;

    terminal.send(Key::Ctrl('p'))?;
    for character in "facts".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until(|screen| screen.contains("build capabilities"))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| {
        screen.contains("About this build") && screen.contains("Facts digest:")
    })?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| !screen.contains("About this build"))?;

    terminal.send(Key::Ctrl('p'))?;
    for character in "test list".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until(|screen| screen.contains("search configured test evidence"))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until_for(
        |screen| {
            screen.contains("Configured tests")
                && screen.contains("Unconfigured: 0/0 runner(s) answered")
        },
        Duration::from_secs(20),
    )?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| !screen.contains("Configured tests"))?;

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    assert_eq!(
        common::census::plan_bytes(fixture.directory()),
        before,
        "discovery journey wrote the plan"
    );
    Ok(())
}

/// Proof for `tui.configured_test_selector_is_reachable`: the built binary,
/// real PTY, bounded worker and configured provider carry one exact test name
/// into the live task's Close form without writing before review.
#[test]
fn configured_test_selector_reaches_a_seeded_close_form_in_a_real_terminal() -> termlens::Result<()>
{
    let fixture = IdentifiedPlanFixture::new();
    fixture.enable_test_catalogue();
    let before = common::census::plan_bytes(fixture.directory());
    let mut terminal = common::tui_terminal(fixture.directory(), 120, 34, &["tui"])?;
    terminal.wait_until_for(
        |screen| screen.contains("Overview - where attention goes next"),
        Duration::from_secs(20),
    )?;
    terminal.send(Key::Ctrl('p'))?;
    for character in "test list".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until(|screen| screen.contains("search configured test evidence"))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until_for(
        |screen| {
            screen.contains("Configured tests")
                && screen.contains("Complete: 1/1 runner(s) answered")
                && screen.contains("test:suite::proof")
        },
        Duration::from_secs(20),
    )?;
    terminal.send(Key::Enter)?;
    terminal.wait_until_for(
        |screen| {
            screen.contains("Close task with evidence - Esc closes")
                && screen.contains("suite::proof|")
        },
        Duration::from_secs(20),
    )?;
    // The inserted value may still own text input when this frame arrives. The
    // first Escape either leaves editing or closes the form; observe which
    // transition happened before deciding whether there is a form to close.
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| !screen.contains("suite::proof|"))?;
    if terminal
        .screen()
        .contains("Close task with evidence - Esc closes")
    {
        terminal.send(Key::Esc)?;
    }
    terminal.wait_until(|screen| !screen.contains("Close task with evidence"))?;
    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    assert_eq!(
        common::census::plan_bytes(fixture.directory()),
        before,
        "test selection wrote before review"
    );
    Ok(())
}

/// Proof for `tui.mutation_surface_has_cross_adapter_parity` and
/// `tui.planning_forms_render_typed_choice_context`: the real binary, PTY
/// decoder, worker, guarded publisher and post-write reload complete the
/// representative coordination, status, planning, authoring and evidence
/// journeys through visible typed context, review and confirmation boundaries.
#[test]
fn mutations_claim_start_edit_dependencies_add_and_close_in_a_real_terminal() -> termlens::Result<()>
{
    let fixture = IdentifiedPlanFixture::new();
    fixture.enable_mutation_journey();
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 120, 34, &args)?;
    terminal.wait_until_for(
        |screen| screen.contains("Overview - where attention goes next"),
        Duration::from_secs(20),
    )?;

    open_mutation(&mut terminal, "Claim task", "FORM: Claim task")?;
    for character in "Fixture User".chars() {
        terminal.send(Key::Char(character))?;
    }
    review_and_confirm(&mut terminal, "Claim task")?;

    open_mutation(&mut terminal, "Start task", "PREVIEW: start `T-001`")?;
    terminal.send(Key::Enter)?;
    terminal.wait_until_for(
        |screen| screen.contains("Verdict: COMMITTED"),
        Duration::from_secs(20),
    )?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| !screen.contains("Verdict: COMMITTED"))?;

    terminal.send(Key::Char('2'))?;
    terminal.wait_until(|screen| screen.contains("Tasks - what is true about this work"))?;
    terminal.send(Key::Down)?;
    terminal.wait_until(|screen| screen.contains("T-002"))?;
    open_mutation(
        &mut terminal,
        "Add blocking dependency",
        "FORM: Add blocking dependency",
    )?;
    terminal.wait_until(|screen| {
        screen.contains("Candidates (1; selected task excluded)")
            && screen.contains("T-001 [in_progress] cut(s): none")
            && !screen.contains("  T-002 [")
    })?;
    terminal.send(Key::Tab)?;
    for character in "T-001".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.send(Key::Tab)?;
    terminal.send(Key::Tab)?;
    terminal.send(Key::Enter)?;
    terminal.wait_until_for(
        |screen| screen.contains("PREVIEW: Add blocking dependency"),
        Duration::from_secs(20),
    )?;
    terminal.send(Key::BackTab)?;
    terminal.send(Key::Enter)?;
    terminal.wait_until_for(
        |screen| screen.contains("Verdict: COMMITTED"),
        Duration::from_secs(20),
    )?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| !screen.contains("Verdict: COMMITTED"))?;

    open_mutation(&mut terminal, "Add task (CLI: aplan add)", "FORM: Add task")?;
    for character in "Created interactively".chars() {
        terminal.send(Key::Char(character))?;
    }
    review_and_confirm(&mut terminal, "Add task")?;

    terminal.send(Key::Char('2'))?;
    terminal.send(Key::Char('/'))?;
    for character in "T-001".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.send(Key::Enter)?;
    terminal.send(Key::Down)?;
    terminal
        .wait_until(|screen| screen.contains("1/3 shown") && screen.contains("T-001 / Summary"))?;
    open_mutation(
        &mut terminal,
        "Close task with evidence",
        "FORM: Close task with evidence",
    )?;
    review_and_confirm(&mut terminal, "Close task with evidence")?;

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());

    let first =
        std::fs::read_to_string(fixture.plan_root.join("tasks/T-001.md")).expect("first task");
    let second =
        std::fs::read_to_string(fixture.plan_root.join("tasks/T-002.md")).expect("second task");
    assert!(first.contains("status: done"), "{first}");
    assert!(second.contains("blocked_by: [T-001]"), "{second}");
    assert!(
        std::fs::read_dir(fixture.plan_root.join("tasks"))
            .expect("tasks")
            .filter_map(Result::ok)
            .any(|entry| {
                std::fs::read_to_string(entry.path())
                    .is_ok_and(|body| body.contains("title: Created interactively"))
            }),
        "the authoring confirmation did not publish its task"
    );
    Ok(())
}

/// Proof for the native capture workflow: the global shortcut preserves its
/// destination, Inbox exposes the retained record, promotion publishes the
/// task and disposition together, and the terminal state visibly refuses a
/// second promotion.
#[test]
fn capture_promotes_atomically_and_refuses_repromotion_in_a_real_terminal() -> termlens::Result<()>
{
    let fixture = IdentifiedPlanFixture::new();
    fixture.enable_capture_journey();
    let args = ["tui"];
    let mut terminal = common::tui_terminal(fixture.directory(), 120, 34, &args)?;
    terminal.wait_until_for(
        |screen| screen.contains("Overview - where attention goes next"),
        Duration::from_secs(20),
    )?;

    quick_capture_from_overview(&mut terminal)?;

    terminal.send(Key::Char('6'))?;
    terminal.wait_until_for(
        |screen| {
            screen.contains("Inbox - which captured intake needs review")
                && screen.contains("Terminal intake")
                && screen.contains("OPEN")
        },
        Duration::from_secs(20),
    )?;
    open_mutation(
        &mut terminal,
        "Promote capture to task",
        "FORM: Promote capture to task",
    )?;
    terminal.send(Key::BackTab)?;
    terminal.send(Key::Enter)?;
    terminal.wait_until_for(
        |screen| {
            screen.contains("PREVIEW: Promote capture to task")
                && screen.contains("TASK 1/1: T-002")
                && screen.contains("Arm broad change")
        },
        Duration::from_secs(20),
    )?;
    terminal.send(Key::BackTab)?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("Confirm broad change"))?;
    terminal.send(Key::BackTab)?;
    terminal.send(Key::Enter)?;
    terminal.wait_until_for(
        |screen| screen.contains("Verdict: COMMITTED"),
        Duration::from_secs(20),
    )?;
    terminal.send(Key::Esc)?;
    terminal.wait_until_for(
        |screen| {
            screen.contains("Inbox - which captured intake needs review")
                && screen.contains("PROMOTED")
                && screen.contains("Promoted task: T-002")
        },
        Duration::from_secs(20),
    )?;

    terminal.resize(180, 34)?;
    terminal.wait_until(|screen| repainted_at(screen, 180, 34))?;
    terminal.send(Key::Ctrl('p'))?;
    terminal.wait_until(|screen| screen.contains("Action:"))?;
    for character in "Promote capture to task".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until(|screen| {
        screen.contains("disabled: Promote capt")
            && !screen.contains("PREVIEW: Promote capture to task")
    })?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| {
        screen.contains("Action:") && screen.contains("disabled: Promote capt")
    })?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| !screen.contains("Command palette"))?;
    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());

    assert_capture_promoted(&fixture);
    Ok(())
}

/// Proof for `tui.overlay_pointer_activation_is_semantic_and_guarded`: the
/// real decoder and renderer agree on modal rows, fixed controls, editable
/// form fields and guarded review/confirmation across a responsive resize.
#[test]
fn overlay_pointer_controls_run_end_to_end_in_a_real_terminal() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    fixture.enable_mutation_journey();
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 48, 14, &args)?;
    terminal.wait_until_for(
        |screen| screen.contains("Overview - where attention goes next"),
        Duration::from_secs(20),
    )?;
    terminal.wait_idle(Duration::from_millis(20))?;

    terminal.send(Key::Char('2'))?;
    terminal.wait_until(|screen| screen.contains("Tasks - what is true about this work"))?;
    terminal.wait_idle(Duration::from_millis(20))?;
    terminal.send(Key::Char('/'))?;
    terminal.wait_until(|screen| screen.contains("Search tasks"))?;
    terminal.wait_idle(Duration::from_millis(20))?;
    for character in "missing".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until(|screen| screen.contains("missing"))?;
    terminal.wait_idle(Duration::from_millis(20))?;
    let (row, column) = terminal.screen().find("[Clear]").expect("clear control");
    terminal.click(column, row)?;
    terminal.wait_until(|screen| screen.contains("Search tasks") && !screen.contains("missing"))?;
    terminal.wait_idle(Duration::from_millis(20))?;
    let (row, column) = terminal.screen().find("[Apply]").expect("apply control");
    terminal.click(column, row)?;
    terminal.wait_until(|screen| !screen.contains("Search tasks"))?;
    terminal.wait_idle(Duration::from_millis(20))?;

    terminal.send(Key::Char('f'))?;
    terminal.wait_until(|screen| screen.contains("Task filters") && screen.contains("[Close]"))?;
    terminal.wait_idle(Duration::from_millis(20))?;
    let (instruction_row, instruction_column) = terminal
        .screen()
        .find("Enter/space toggles")
        .expect("filter instructions");
    terminal.click(instruction_column, instruction_row.saturating_add(1))?;
    terminal.wait_idle(Duration::from_millis(20))?;
    let (row, column) = terminal.screen().find("[Toggle]").expect("toggle control");
    terminal.click(column, row)?;
    terminal.wait_until(|screen| screen.contains("[x]"))?;
    terminal.wait_idle(Duration::from_millis(20))?;
    let (row, column) = terminal.screen().find("[Close]").expect("close control");
    terminal.click(column, row)?;
    terminal.wait_until(|screen| !screen.contains("Task filters"))?;
    terminal.wait_idle(Duration::from_millis(20))?;

    terminal.resize(100, 30)?;
    terminal.wait_until(|screen| repainted_at(screen, 100, 30))?;
    terminal.wait_idle(Duration::from_millis(20))?;
    open_mutation(&mut terminal, "Claim task", "FORM: Claim task")?;
    terminal.wait_idle(Duration::from_millis(20))?;
    let (row, column) = terminal.screen().find("Act as").expect("form field");
    terminal.click(column, row)?;
    terminal.wait_idle(Duration::from_millis(20))?;
    for character in "Pointer User".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until(|screen| screen.contains("Pointer User|"))?;
    // `wait_until` may match a screen while Ratatui is still repainting it.
    // Pointer coordinates need a settled whole-screen snapshot so the cells
    // searched here are the cells the application's hit map just painted.
    terminal.wait_idle(Duration::from_millis(20))?;
    let (row, column) = terminal.screen().find("[Review]").expect("review control");
    terminal.click(column, row)?;
    terminal.wait_until_for(
        |screen| screen.contains("PREVIEW: claim `T-001`") && screen.contains("[Confirm]"),
        Duration::from_secs(20),
    )?;
    terminal.wait_idle(Duration::from_millis(20))?;
    let (row, column) = terminal
        .screen()
        .find("[Confirm]")
        .expect("confirm control");
    terminal.click(column, row)?;
    terminal.wait_until_for(
        |screen| screen.contains("Verdict: COMMITTED") && screen.contains("[Close]"),
        Duration::from_secs(20),
    )?;
    terminal.wait_idle(Duration::from_millis(20))?;
    let (row, column) = terminal
        .screen()
        .find("[Close]")
        .expect("result close control");
    terminal.click(column, row)?;
    terminal.wait_until(|screen| !screen.contains("Verdict: COMMITTED"))?;
    terminal.wait_idle(Duration::from_millis(20))?;

    open_mutation(&mut terminal, "Add task (CLI: aplan add)", "FORM: Add task")?;
    terminal.wait_idle(Duration::from_millis(20))?;
    let (row, column) = terminal
        .screen()
        .find("[Cancel]")
        .expect("form cancel control");
    terminal.click(column, row)?;
    terminal.wait_until(|screen| !screen.contains("FORM: Add task"))?;
    terminal.wait_idle(Duration::from_millis(20))?;

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    let claim = std::fs::read_to_string(fixture.directory().join(".git/aplan/claims-v1.json"))
        .expect("published claim");
    assert!(claim.contains("Pointer User"), "{claim}");
    Ok(())
}

/// Proof for `tui.startup_feedback_precedes_plan_io`: a selected configuration
/// deliberately blocked before its first byte cannot prevent the initial
/// frame, resize or cancellation controls from working. Replacing the gate
/// then transitions in the same terminal session to the identified snapshot.
#[cfg(unix)]
#[test]
fn loading_frame_precedes_blocked_configuration_and_transitions_in_place() -> termlens::Result<()> {
    use std::io::Write as _;

    let fixture = IdentifiedPlanFixture::new();
    let task = fixture.plan_root.join("tasks/T-001.md");
    let before = std::fs::read(&task).expect("task before startup");
    let gate = fixture.directory().join("startup-gate.yml");
    let status = Command::new("mkfifo")
        .arg(&gate)
        .status()
        .expect("mkfifo runs");
    assert!(status.success(), "mkfifo failed");
    let gate_arg = gate.to_string_lossy().into_owned();
    let args = [
        "--progress",
        "never",
        "tui",
        "--ascii",
        "--reduced-motion",
        "--config",
        gate_arg.as_str(),
        "--root",
        "plan",
    ];
    let mut terminal = common::tui_terminal(fixture.directory(), 48, 14, &args)?;
    terminal.wait_until_for(
        |screen| {
            screen.contains("aplan - loading identified inputs")
                && screen.contains("selecting configuration")
                && screen.contains("indeterminate phase; no fake ETA")
        },
        Duration::from_secs(10),
    )?;
    terminal.resize(64, 16)?;
    terminal.wait_until_for(
        |screen| {
            screen.cols() == 64 && screen.rows() == 16 && screen.contains("selecting configuration")
        },
        Duration::from_secs(10),
    )?;

    let replacement = fixture.directory().join("startup-config-replacement.yml");
    std::fs::write(&replacement, "{}\n").expect("replacement config");
    let writer_gate = gate.clone();
    let writer = std::thread::spawn(move || {
        let mut fifo = std::fs::OpenOptions::new()
            .write(true)
            .open(&writer_gate)
            .expect("open startup gate");
        std::fs::rename(&replacement, &writer_gate).expect("publish regular config path");
        fifo.write_all(b"{}\n").expect("release configuration read");
    });
    terminal.wait_until_for(
        |screen| screen.contains("Overview - where attention goes next"),
        Duration::from_secs(20),
    )?;
    writer.join().expect("configuration writer");
    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    assert_eq!(std::fs::read(&task).expect("task after startup"), before);
    Ok(())
}

/// Proof for `tui.startup_feedback_precedes_plan_io`: quitting while a real
/// configuration read is blocked restores the terminal before shutdown waits
/// for that owned loader, then cancellation prevents its result from adoption.
#[cfg(unix)]
#[test]
fn blocked_startup_quit_restores_the_terminal_before_joining() -> termlens::Result<()> {
    use std::io::Write as _;
    use std::sync::mpsc::channel;

    let fixture = IdentifiedPlanFixture::new();
    let task = fixture.plan_root.join("tasks/T-001.md");
    let before = std::fs::read(&task).expect("task before startup");
    let gate = fixture.directory().join("startup-quit-gate.yml");
    let status = Command::new("mkfifo")
        .arg(&gate)
        .status()
        .expect("mkfifo runs");
    assert!(status.success(), "mkfifo failed");
    let gate_arg = gate.to_string_lossy().into_owned();
    let args = [
        "--progress",
        "never",
        "tui",
        "--config",
        gate_arg.as_str(),
        "--root",
        "plan",
    ];
    let mut terminal = common::tui_terminal(fixture.directory(), 80, 24, &args)?;
    terminal.wait_until_for(
        |screen| screen.contains("selecting configuration"),
        Duration::from_secs(10),
    )?;

    let replacement = fixture.directory().join("startup-quit-replacement.yml");
    std::fs::write(&replacement, "{}\n").expect("replacement config");
    let (opened_tx, opened_rx) = channel();
    let (release_tx, release_rx) = channel();
    let writer_gate = gate.clone();
    let writer = std::thread::spawn(move || {
        let mut fifo = std::fs::OpenOptions::new()
            .write(true)
            .open(&writer_gate)
            .expect("open startup gate");
        std::fs::rename(&replacement, &writer_gate).expect("publish regular config path");
        opened_tx.send(()).expect("opened notification");
        release_rx.recv().expect("release notification");
        fifo.write_all(b"{}\n").expect("release configuration read");
    });
    opened_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("loader reached blocked read");
    terminal.send(Key::Char('q'))?;
    terminal.wait_until_for(
        |screen| {
            !screen.alternate_screen()
                && !screen.bracketed_paste()
                && !screen.focus_events()
                && screen.cursor_visible()
        },
        Duration::from_secs(5),
    )?;
    release_tx.send(()).expect("release blocked loader");
    writer.join().expect("configuration writer");
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    assert_eq!(std::fs::read(&task).expect("task after startup"), before);
    Ok(())
}

/// Proof for `tui.graph_explorer_uses_semantic_scene` and
/// `tui.layered_graph_navigation_is_semantic_and_reversible`: drive hierarchy,
/// geometry, repeated relations, order/direction pins, exact phase movement,
/// contextual entry/return, resize and teardown through the real terminal.
#[test]
fn graph_explorer_runs_end_to_end_in_a_real_terminal() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    fixture.enable_graph_explorer();
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 120, 30, &args)?;

    terminal.wait_until(|screen| screen.contains("Overview - where attention goes next"))?;
    terminal.send(Key::Char('4'))?;
    terminal.wait_until(|screen| {
        screen.contains("Graph - how work is connected")
            && screen.contains("5/5N 2/2E")
            && screen.contains("T-001 / Summary")
            && screen.contains("Overview | COMPLETE")
    })?;

    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("Overview > Phase 1 |"))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("Overview > Phase 1 > M-v1 |"))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("Overview > Phase 1 > M-v1 > T-001"))?;

    terminal.send(Key::Char(']'))?;
    terminal.wait_until(|screen| {
        screen.contains("T-002 / Summary") && screen.contains("Dependent 1/1")
    })?;
    terminal.send(Key::Char(']'))?;
    terminal.wait_until(|screen| screen.contains("T-003 / Summary"))?;
    terminal.send(Key::Char('['))?;
    terminal.wait_until(|screen| screen.contains("T-002 / Summary"))?;

    terminal.send(Key::Char('s'))?;
    terminal.wait_until(|screen| screen.contains("Graph peer order: plan priority"))?;
    terminal.send(Key::Char('d'))?;
    terminal.wait_until(|screen| screen.contains("Graph direction: left-to-right"))?;

    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| screen.contains("Overview > Phase 1 > M-v1 |"))?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| screen.contains("Overview > Phase 1 |"))?;
    terminal.send(Key::Char('N'))?;
    terminal.wait_until(|screen| screen.contains("Overview > Phase 2 |"))?;
    terminal.send(Key::Char('P'))?;
    terminal.wait_until(|screen| screen.contains("Overview > Phase 1 |"))?;

    terminal.send(Key::Char('2'))?;
    terminal.wait_until(|screen| screen.contains("Tasks - what is true about this work"))?;
    terminal.send(Key::Char('G'))?;
    terminal.wait_until(|screen| {
        screen.contains("Tasks > Overview") && screen.contains("Graph - how work is connected")
    })?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| screen.contains("Tasks - what is true about this work"))?;
    terminal.send(Key::Char('4'))?;
    terminal.wait_until(|screen| screen.contains("Overview > Phase 1 |"))?;

    terminal.resize(48, 14)?;
    // `resize` changes the emulator's dimensions immediately, before the TUI
    // has handled SIGWINCH. These markers also existed in the wider frame, so
    // dimensions plus content can accept that stale clipped frame and race the
    // following key with the resize. The top border reaches the new last
    // column only after the TUI has laid out and painted the narrower frame.
    terminal.wait_until(|screen| {
        repainted_at(screen, 48, 14)
            && screen.contains("Overview > Phase 1 |")
            && screen.contains("M-v1")
    })?;

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    Ok(())
}

/// Proof for `tui.graph_explorer_windows_oversized_inputs` and
/// `tui.layered_graph_navigation_is_semantic_and_reversible`: a real terminal
/// opens an over-wide task level, reverses a continuation, cycles every direct
/// relation including an omitted target, pans, resizes and restores its modes.
#[test]
fn graph_manual_pan_and_resize_run_end_to_end_in_a_real_terminal() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    for index in 2..=36 {
        let id = format!("T-{index:03}");
        std::fs::write(
            fixture.plan_root.join(format!("tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: a deliberately wide graph leaf {index} {}\nphase: 1\n\
                 status: todo\neffort_days: 1.0\nblocked_by: [T-001]\npaths: [src/**]\n\
                 tags: [cross-cutting]\n---\n",
                "wide".repeat(12)
            ),
        )
        .expect("large graph task");
    }
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 80, 18, &args)?;

    terminal.wait_until(|screen| screen.contains("Overview - where attention goes next"))?;
    terminal.send(Key::Char('4'))?;
    terminal.wait_until(|screen| {
        screen.contains("Graph - how work is connected")
            && screen.contains("10/36N 9/35E")
            && screen.contains("Overview | WINDOWED")
            && screen.contains("n open")
    })?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("Overview > Phase 1"))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| {
        screen.contains("Overview > Phase 1 > T-001")
            && screen.contains("WINDOWED")
            && screen.contains("n open")
    })?;
    terminal.send(Key::Char('n'))?;
    terminal.wait_until(|screen| {
        screen.contains("Overview > Phase 1 > T-001")
            && screen.contains("WINDOWED 16/")
            && !screen.contains("n open")
    })?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| {
        screen.contains("Overview > Phase 1 > T-001")
            && screen.contains("WINDOWED 21/")
            && screen.contains("n open")
    })?;
    for index in 1..=35 {
        terminal.send(Key::Char(']'))?;
        let target = format!("T-{:03}", index + 1);
        let progress = format!("Dependent {index}/35: {target}");
        terminal.wait_until(|screen| screen.contains(&target) && screen.contains(&progress))?;
    }
    terminal.wait_until(|screen| {
        screen.contains("T-036 / Summary") && screen.contains("Dependent 35/35: T-036")
    })?;
    terminal.send(Key::PageDown)?;
    terminal.send(Key::Char('L'))?;
    terminal.resize(60, 16)?;
    terminal.wait_until(|screen| {
        repainted_at(screen, 60, 16)
            && screen.contains("Graph - how work is connected")
            && screen.contains("Overview > Phase 1 > T-036")
    })?;

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    Ok(())
}

/// Proof for `tui.inspectors_use_semantic_information_hierarchy`: the real
/// binary exposes the task's identity, state and responsibility as scannable
/// groups, keeps exact non-colour badges through minimum-width reflow and
/// activates the same typed section link after that reflow.
#[test]
fn semantic_task_summary_is_scannable_and_navigable_in_a_real_terminal() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 120, 34, &args)?;

    terminal.wait_until(|screen| screen.contains("Overview - where attention goes next"))?;
    terminal.send(Key::Char('2'))?;
    terminal.wait_until(|screen| screen.contains("Tasks - what is true about this work"))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| {
        screen.contains("T-001 / Summary")
            && screen.contains("Inspect the plan")
            && screen.contains("-- STATE --")
            && screen.contains("Status")
            && screen.contains("[todo]")
            && screen.contains("Readiness")
            && screen.contains("ready to start")
            && screen.contains("-- RESPONSIBILITY --")
            && screen.contains("Owner")
            && screen.contains("Claim")
    })?;

    terminal.resize(48, 14)?;
    terminal.wait_until(|screen| {
        repainted_at(screen, 48, 14)
            && screen.contains("T-001 / Summary")
            && screen.contains("Inspect the plan")
    })?;
    terminal.send(Key::Down)?;
    terminal.wait_until(|screen| {
        screen.contains("-- STATE --") && screen.contains("Status") && screen.contains("[todo]")
    })?;
    send_down(&mut terminal, 9)?;
    terminal.wait_until(|screen| {
        screen.contains("Work")
            && screen.contains("[Enter]")
            && (screen.contains("^") || screen.contains("\u{2191}"))
    })?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| {
        screen.contains("T-001 / Work")
            && screen.contains("Ready")
            && screen.contains("[ordinary work can start")
            && screen.contains("Esc back | Tasks > T-001 / Work")
    })?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| screen.contains("T-001 / Summary"))?;

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    Ok(())
}

/// Proof for `tui.shell_adapts_by_route_and_available_rows`: the real binary
/// crosses wide, short split, narrow and minimum compositions without losing
/// route/task orientation or editable form input, then restores the terminal.
#[test]
fn adaptive_shell_preserves_orientation_and_work_across_real_resizes() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 140, 30, &args)?;

    terminal.wait_until(|screen| screen.contains("Overview - where attention goes next"))?;
    terminal.send(Key::Char('2'))?;
    terminal.wait_until(|screen| {
        screen.contains("Tasks - what is true about this work") && screen.contains("T-001")
    })?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("T-001 / Summary"))?;

    terminal.resize(100, 21)?;
    terminal.wait_until(|screen| {
        repainted_at(screen, 100, 21)
            && screen.contains("Tasks - what is true about this work")
            && screen.contains("T-001 / Summary")
    })?;
    terminal.resize(60, 16)?;
    terminal.wait_until(|screen| {
        repainted_at(screen, 60, 16)
            && screen.contains("Esc tasks | Tasks > T-001 / Summary")
            && screen.contains("Inspect the plan")
    })?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| {
        screen.contains("Tasks - what is true about this work")
            && screen.contains("T-001")
            && screen.contains("C-p commands")
            && screen.contains("? help")
    })?;
    terminal.resize(48, 14)?;
    terminal.wait_until(|screen| {
        repainted_at(screen, 48, 14)
            && screen.contains("T-001")
            && screen.contains("C-p commands")
            && screen.contains("? help")
    })?;

    open_mutation(
        &mut terminal,
        "Add task (CLI: aplan add)",
        "Title [required; one value]",
    )?;
    let form = terminal.wait_stable(Duration::from_millis(100))?;
    assert!(form.contains("Add task - Esc closes"), "{form:?}");
    assert!(form.contains("Title"), "{form:?}");
    assert!(form.contains("* 1:"), "{form:?}");
    assert!(form.contains("[Review]"), "{form:?}");
    terminal.send(Key::Char('r'))?;
    terminal.wait_until(|screen| screen.contains("* 1: r"))?;
    for character in "esponsive draft".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until(|screen| screen.contains("responsive draft"))?;
    terminal.resize(80, 24)?;
    terminal.snapshot_after(|screen| {
        screen.cols() == 80
            && screen.rows() == 24
            && screen.contains("Title")
            && screen.contains("responsive draft")
    })?;
    terminal.resize(48, 14)?;
    terminal.snapshot_after(|screen| {
        screen.cols() == 48
            && screen.rows() == 14
            && screen.contains("Title")
            && screen.contains("responsive draft")
    })?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| !screen.contains("Title [required; one value]"))?;

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    Ok(())
}

/// Proof for `tui.work_discovery_is_one_typed_flow` and
/// `tui.work_discovery_state_is_semantically_exact`: drive rank, a real folded
/// cut and member, an exact scheduled revisit, a release link and exact Back
/// state through the real binary and terminal protocol, including minimum
/// layout and clean teardown.
#[test]
fn work_discovery_runs_end_to_end_in_a_real_terminal() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    fixture.enable_work_discovery();
    let context = fixture.context();
    let graph = aplan_core::Graph::new(context.plan().tasks.iter().map(|task| &task.task));
    assert_eq!(
        graph
            .ready()
            .into_iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>(),
        vec!["T-001"],
        "T-001 blockers: {:?}",
        graph.blockers_of("T-001")
    );
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 120, 30, &args)?;

    terminal.wait_until(|screen| {
        screen.contains("Overview - where attention goes next") && screen.contains("READY 1")
    })?;
    send_down(&mut terminal, 2)?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| {
        screen.contains("Tasks - what is true about this work")
            && screen.contains("ready work")
            && screen.contains("1/4 shown")
            && screen.contains("T-001")
    })?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("> T-001 / Summary <"))?;
    terminal.send(Key::Char(']'))?;
    terminal.wait_until(|screen| {
        screen.contains("T-001 / Work")
            && screen.contains("Ready rank")
            && screen.contains("#1 - releases 0")
            && screen.contains("Why traversal")
            && screen.contains("cut-bounded")
    })?;

    terminal.send(Key::Char('d'))?;
    terminal.wait_until(|screen| screen.contains("Work discovery as-of instant"))?;
    terminal.send(Key::Char('1'))?;
    terminal.send(Key::Char('0'))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("Due as-of") && screen.contains("epoch 10"))?;

    send_down(&mut terminal, 7)?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("T-002 / Summary"))?;

    terminal.send(Key::Char(']'))?;
    terminal.wait_until(|screen| {
        screen.contains("T-002 / Work")
            && screen.contains("Why traversal")
            && screen.contains("cut-bounded")
    })?;
    send_down(&mut terminal, 7)?;
    terminal.wait_until(|screen| {
        screen.contains("T-002 / Work")
            && screen.contains("Blocked by")
            && screen.contains("M-work")
            && screen.contains("folds")
            && screen.contains("1 unmet task(s)")
            && !screen.contains("PARKED")
    })?;
    terminal.send(Key::Char('w'))?;
    terminal
        .wait_until(|screen| screen.contains("Why traversal") && screen.contains("complete"))?;

    send_down(&mut terminal, 8)?;
    terminal.wait_until(|screen| {
        screen.contains("Blocked by") && screen.contains("PARKED") && !screen.contains("folds")
    })?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("PARKED / Summary"))?;
    terminal.send(Key::Char(']'))?;
    send_down(&mut terminal, 5)?;
    terminal.wait_until(|screen| {
        screen.contains("PARKED / Work")
            && screen.contains("Revisit")
            && screen.contains("DUE since 1970-01-01T00")
            && screen.contains(":00:10Z")
    })?;

    // Back restores the complete-why row and its exact PARKED selection: Enter
    // must reopen the same identity without moving the cursor again.
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| {
        screen.contains("T-002 / Work")
            && screen.contains("Blocked by")
            && screen.contains("PARKED")
    })?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("PARKED / Summary"))?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| screen.contains("T-002 / Work"))?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| {
        screen.contains("T-001 / Work") && screen.contains("Releases") && screen.contains("T-002")
    })?;

    terminal.resize(48, 14)?;
    terminal
        .wait_until(|screen| repainted_at(screen, 48, 14) && screen.contains("T-001 / Work"))?;
    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    Ok(())
}

/// Proof for `tui.report_hub_preserves_cli_report_semantics` and
/// `tui.report_discovery_is_literal_and_parameter_safe`: drive a typed
/// report through the real binary and terminal protocol, including report
/// selection, row focus, contributor drill-down and clean teardown.
#[test]
fn report_hub_runs_end_to_end_in_a_real_terminal() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 120, 30, &args)?;

    terminal.wait_until(|screen| screen.contains("Overview - where attention goes next"))?;
    terminal.send(Key::Char('5'))?;
    terminal.wait_until(|screen| {
        screen.contains("Reports - which aggregate needs a decision")
            && screen.contains("Roadmap report")
            && screen.contains("Cross-cutting work")
    })?;
    terminal.send(Key::Char('/'))?;
    for character in "summary".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until(|screen| {
        screen.contains("Search report names") && screen.contains("1/7 report names shown")
    })?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("Summary report"))?;
    terminal.send(Key::Tab)?;
    terminal.wait_until(|screen| {
        screen.contains("> Summary report <")
            && screen.contains("0/1 done; 1 ready; 1.0d")
            && screen.contains("[Enter: tasks] Phase 1")
    })?;
    terminal.send(Key::Char('/'))?;
    for character in "phase 1".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until(|screen| {
        screen.contains("Search report rows") && screen.contains("1/1 rows shown")
    })?;
    terminal.send(Key::Enter)?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| {
        screen.contains("Tasks - what is true about this work")
            && screen.contains("report: Summary / Phase 1")
            && screen.contains("T-001")
    })?;

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    Ok(())
}

/// Proof for `tui.external_refresh_is_identity_safe`: an edit made outside the
/// running process is detected by the timed identity probe and appears only as
/// a complete replacement snapshot in the real terminal.
#[test]
fn external_plan_edit_refreshes_in_a_real_terminal() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 120, 30, &args)?;

    terminal.wait_until(|screen| screen.contains("Overview - where attention goes next"))?;
    terminal.send(Key::Char('2'))?;
    terminal.wait_until(|screen| {
        screen.contains("Tasks - what is true about this work")
            && screen.contains("Inspect the plan")
    })?;

    let task = fixture.plan_root.join("tasks/T-001.md");
    let before = std::fs::read_to_string(&task).expect("task before external edit");
    std::fs::write(
        &task,
        before.replace(
            "title: Inspect the plan",
            "title: Observe the refreshed plan",
        ),
    )
    .expect("external plan edit");

    terminal.wait_until_for(
        |screen| {
            screen.contains("Observe the refreshed plan")
                && screen.contains("Refresh complete")
                && screen.contains("plan input changed")
        },
        Duration::from_secs(15),
    )?;
    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    Ok(())
}

/// Proof for `tui.external_refresh_is_identity_safe` and
/// `tui.focus_epoch_invalidates_external_results`: the session actually
/// enables focus reporting, keeps an inactive snapshot unconfirmed, and uses
/// focus gain to observe an edit without waiting for the polling interval.
#[test]
fn focus_gain_probes_an_external_edit_in_a_real_terminal() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 120, 30, &args)?;

    terminal.wait_until(|screen| {
        screen.contains("Overview - where attention goes next") && screen.focus_events()
    })?;
    terminal.send(Key::Char('2'))?;
    terminal.wait_until(|screen| {
        screen.contains("Tasks - what is true about this work")
            && screen.contains("Inspect the plan")
    })?;
    terminal.focus_out()?;
    terminal.wait_until(|screen| screen.contains("INACTIVE"))?;

    let task = fixture.plan_root.join("tasks/T-001.md");
    let before = std::fs::read_to_string(&task).expect("task before external edit");
    std::fs::write(
        &task,
        before.replace(
            "title: Inspect the plan",
            "title: Observe the focus refresh",
        ),
    )
    .expect("external plan edit");
    terminal.focus_in()?;
    terminal.wait_until_for(
        |screen| {
            screen.contains("Observe the focus refresh")
                && screen.contains("Refresh complete")
                && screen.contains("plan input changed")
        },
        Duration::from_secs(10),
    )?;

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    Ok(())
}

/// Proof for `tui.overdue_external_probe_is_fail_closed`: a configured
/// verifier may own the sole worker across the polling deadline, but the real
/// terminal immediately stops calling its snapshot current and observes an
/// external edit as soon as the provider releases capacity.
#[cfg(unix)]
#[test]
fn overdue_probe_recovers_after_a_long_provider_in_a_real_terminal() -> termlens::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = IdentifiedPlanFixture::new();
    let task = fixture.plan_root.join("tasks/T-001.md");
    let task_body = std::fs::read_to_string(&task).expect("task");
    std::fs::write(
        &task,
        task_body.replace("status: todo", "status: done").replace(
            "paths: [src/**]\n",
            "paths: [src/**]\nevidence:\n- kind: test\n  ref: suite::proof\n",
        ),
    )
    .expect("task with test evidence");
    let started = fixture.directory().join("overdue-provider.started");
    let gate = fixture.directory().join("overdue-provider.gate");
    let script = fixture.directory().join("block-overdue-list.sh");
    std::fs::write(&gate, "held\n").expect("provider gate");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n: > '{}'\nwhile test -e '{}'; do :; done\nprintf '%s\\n' 'suite::proof'\n",
            started.display(),
            gate.display()
        ),
    )
    .expect("provider script");
    let mut permissions = script.metadata().expect("script metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).expect("script executable");
    std::fs::write(
        fixture.directory().join(".aplan.yml"),
        "root: plan\ntest:\n  list: [./block-overdue-list.sh]\n  run: ['true', '{test}']\n",
    )
    .expect("config");
    common::git(fixture.directory(), &["init", "-q", "."]);

    let args = ["tui", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 120, 30, &args)?;
    terminal
        .wait_until(|screen| screen.contains("Overview - where attention goes next"))
        .expect("initial overview");
    terminal.send(Key::Char('V'))?;
    terminal
        .wait_until(|screen| screen.contains("fix: exact repairs are a separate guarded"))
        .expect("verification run sheet");
    terminal.send(Key::Down)?;
    terminal.wait_until(|screen| screen.contains("> RUN VERIFY"))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("verify in progress"))?;
    wait_for_file(&started);

    let response_started = Instant::now();
    terminal.send(Key::Char('2'))?;
    terminal.wait_until_for(
        |screen| {
            screen.contains("Tasks - what is true about this work")
                && screen.contains("verify in progress")
        },
        Duration::from_millis(500),
    )?;
    assert!(
        response_started.elapsed() < Duration::from_millis(500),
        "navigation was not responsive while verification owned the worker"
    );
    terminal.send(Key::Char('1'))?;
    terminal.wait_until(|screen| screen.contains("Overview - where attention goes next"))?;

    let before = std::fs::read_to_string(&task).expect("task before external edit");
    std::fs::write(
        &task,
        before.replace(
            "title: Inspect the plan",
            "title: Observe the overdue refresh",
        ),
    )
    .expect("external plan edit");
    terminal.wait_until_for(
        |screen| {
            screen.contains("verify in progress") && screen.contains("unconfirmed (stale possible)")
        },
        Duration::from_secs(10),
    )?;

    std::fs::remove_file(&gate).expect("release provider");
    terminal.wait_until_for(
        |screen| {
            screen.contains("Refresh complete")
                && screen.contains("snapshot: current")
                && screen.contains("plan input changed")
                && !screen.contains("Verification - arrows inspect")
        },
        Duration::from_secs(15),
    )?;
    terminal.send(Key::Char('2'))?;
    terminal.wait_until(|screen| screen.contains("Observe the overdue refresh"))?;

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    Ok(())
}

#[test]
fn control_c_exits_cleanly_and_restores_terminal_modes() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 100, 24, &args)?;

    terminal.wait_until(|screen| {
        screen.contains("Overview - where attention goes next")
            && screen.alternate_screen()
            && screen.bracketed_paste()
            && !screen.cursor_visible()
    })?;
    terminal.send(Key::Ctrl('c'))?;
    assert!(terminal.wait_exit()?.success());

    let restored = terminal.screen();
    assert!(!restored.alternate_screen());
    assert!(!restored.bracketed_paste());
    assert!(restored.cursor_visible());
    assert!(
        restored
            .unsupported()
            .into_iter()
            .all(|sequence| sequence == "^[[59m"),
        "unexpected terminal sequence during Ctrl-C exit: {:?}",
        restored.unsupported()
    );
    Ok(())
}

fn assert_terminal_restored(screen: &termlens::Screen) {
    assert!(!screen.alternate_screen());
    assert!(!screen.bracketed_paste());
    assert!(!screen.focus_events());
    assert!(screen.cursor_visible());
}

/// Proof for `tui.terminal_lifecycle_handles_signals`: each catchable
/// terminating signal reaches the process out of band, restoration is visible
/// before EOF, and the parent still observes the original signal rather than a
/// fabricated success code.
#[cfg(unix)]
#[test]
fn terminating_signals_restore_the_terminal_and_keep_the_exit_reason() -> termlens::Result<()> {
    for signal in [
        termlens::Signal::Int,
        termlens::Signal::Term,
        termlens::Signal::Hup,
        termlens::Signal::Quit,
    ] {
        let expected = match signal {
            termlens::Signal::Int => "Interrupt",
            termlens::Signal::Term => "Terminated",
            termlens::Signal::Hup => "Hangup",
            termlens::Signal::Quit => "Quit",
            _ => unreachable!("the case table contains only handled termination signals"),
        };
        let fixture = IdentifiedPlanFixture::new();
        let args = ["tui", "--no-config", "--root", "plan"];
        let mut terminal = common::tui_terminal(fixture.directory(), 100, 24, &args)?;
        terminal.wait_until(|screen| {
            screen.contains("Overview - where attention goes next")
                && screen.alternate_screen()
                && screen.bracketed_paste()
                && !screen.cursor_visible()
        })?;

        terminal.signal(signal)?;
        let status = terminal.wait_exit()?;
        assert!(
            status
                .signal()
                .is_some_and(|observed| observed.contains(expected)),
            "{signal:?} did not preserve its exit reason: {status}"
        );
        assert_terminal_restored(&terminal.screen());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn send_signal(pid: u32, name: &str) {
    common::send_signal(pid, name);
}

#[cfg(target_os = "linux")]
fn process_state(pid: u32) -> Option<String> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("State:\t"))
        .map(ToString::to_string)
}

#[cfg(target_os = "linux")]
fn wait_for_stopped_process(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if process_state(pid).is_some_and(|state| state.starts_with('T')) {
            return;
        }
        assert!(Instant::now() < deadline, "process {pid} never stopped");
        std::thread::yield_now();
    }
}

#[cfg(target_os = "linux")]
fn wait_for_resumed_process(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if process_state(pid).is_some_and(|state| !state.starts_with('T')) {
            return;
        }
        assert!(Instant::now() < deadline, "process {pid} never resumed");
        std::thread::yield_now();
    }
}

#[cfg(unix)]
fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        std::thread::yield_now();
    }
}

#[cfg(target_os = "linux")]
fn wait_for_process_exit(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Path::new(&format!("/proc/{pid}")).exists() {
        assert!(
            Instant::now() < deadline,
            "configured provider {pid} survived TUI termination"
        );
        std::thread::yield_now();
    }
}

#[cfg(target_os = "linux")]
fn terminal_settings(pid: u32) -> String {
    let terminal =
        std::fs::read_link(format!("/proc/{pid}/fd/0")).expect("read the child terminal path");
    let output = Command::new("stty")
        .args([
            "-F",
            terminal.to_str().expect("terminal path is UTF-8"),
            "-g",
        ])
        .output()
        .expect("stty runs");
    assert!(output.status.success(), "stty failed: {output:?}");
    String::from_utf8(output.stdout)
        .expect("stty output is UTF-8")
        .trim()
        .to_string()
}

/// Proof for `tui.terminal_lifecycle_handles_signals` and
/// `tui.suspension_invalidates_snapshot_freshness`: suspend restores the exact
/// pre-existing termios state and invalidates freshness before the process
/// stops; resume repaints retained bytes as unconfirmed and observes an edit
/// made while stopped before presenting it as current.
#[cfg(target_os = "linux")]
#[test]
fn suspend_restores_exact_termios_and_resume_repaints() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    let task = fixture.plan_root.join("tasks/T-001.md");
    let baseline = fixture.directory().join("termios.baseline");
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal =
        common::tui_terminal_with_termios_baseline(fixture.directory(), 120, 30, &args, &baseline)?;
    wait_for_file(&baseline);
    terminal.wait_until(|screen| {
        screen.contains("Overview - where attention goes next")
            && screen.alternate_screen()
            && !screen.cursor_visible()
    })?;
    let pid = terminal.pid().expect("Unix child pid");
    let expected = std::fs::read_to_string(&baseline)
        .expect("termios baseline")
        .trim()
        .to_string();
    assert_ne!(
        terminal_settings(pid),
        expected,
        "TUI never entered raw mode"
    );
    send_signal(pid, "TSTP");
    terminal.wait_until(|screen| {
        !screen.alternate_screen() && !screen.bracketed_paste() && screen.cursor_visible()
    })?;
    wait_for_stopped_process(pid);
    assert_eq!(
        terminal_settings(pid),
        expected,
        "suspend did not restore the exact caller termios state"
    );
    let task_body = std::fs::read_to_string(&task).expect("task before suspended edit");
    std::fs::write(
        &task,
        task_body.replace("Inspect the plan", "Inspect the resumed plan"),
    )
    .expect("edit plan while the TUI is suspended");

    send_signal(pid, "CONT");
    terminal.wait_until(|screen| {
        screen.contains("Overview - where attention goes next")
            && screen.alternate_screen()
            && screen.bracketed_paste()
            && !screen.cursor_visible()
    })?;
    let resumed = terminal.screen();
    assert!(
        resumed.contains("snapshot: unconfirmed (stale possible)")
            || resumed.contains("snapshot: refreshing (stale possible)"),
        "the first resumed frame falsely certified the pre-suspension snapshot: {resumed:?}"
    );
    terminal.send(Key::Char('2'))?;
    terminal.wait_until_for(
        |screen| screen.contains("Inspect the resumed plan"),
        Duration::from_secs(10),
    )?;
    assert_ne!(
        terminal_settings(pid),
        expected,
        "resume did not reacquire raw mode"
    );
    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    Ok(())
}

/// Proof for `tui.continue_establishes_freshness_boundary`: an uncatchable
/// SIGSTOP has no preceding lifecycle event, so SIGCONT itself must invalidate
/// retained bytes and drive the edited plan to a coherent replacement.
#[cfg(target_os = "linux")]
#[test]
fn standalone_continue_after_sigstop_refreshes_the_snapshot() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    let task = fixture.plan_root.join("tasks/T-001.md");
    let args = ["tui", "--no-config", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 120, 30, &args)?;
    terminal.wait_until(|screen| {
        screen.contains("Overview - where attention goes next")
            && screen.contains("snapshot: current")
            && screen.alternate_screen()
    })?;
    let pid = terminal.pid().expect("Unix child pid");

    send_signal(pid, "STOP");
    wait_for_stopped_process(pid);
    let task_body = std::fs::read_to_string(&task).expect("task before SIGSTOP edit");
    std::fs::write(
        &task,
        task_body.replace("Inspect the plan", "Inspect after SIGSTOP"),
    )
    .expect("edit plan while the TUI is stopped");

    send_signal(pid, "CONT");
    wait_for_resumed_process(pid);
    terminal.wait_until_for(
        |screen| {
            screen.contains("snapshot: unconfirmed (stale possible)")
                || screen.contains("snapshot: refreshing (stale possible)")
                || screen.contains("Inspect after SIGSTOP")
        },
        Duration::from_secs(10),
    )?;
    terminal.send(Key::Char('2'))?;
    terminal.wait_until_for(
        |screen| screen.contains("Inspect after SIGSTOP"),
        Duration::from_secs(10),
    )?;
    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    Ok(())
}

/// Proof for `tui.terminal_lifecycle_handles_signals` and
/// `tui.modal_navigation_is_consistent`: the advertised movement key reaches
/// the second modal action, and a termination request does not join its
/// uncooperative verification worker before restoring the terminal and
/// preserving the signal exit.
#[cfg(target_os = "linux")]
#[test]
fn termination_does_not_wait_for_an_active_provider() -> termlens::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = IdentifiedPlanFixture::new();
    let task = fixture.plan_root.join("tasks/T-001.md");
    let task_body = std::fs::read_to_string(&task).expect("task");
    std::fs::write(
        &task,
        task_body.replace("status: todo", "status: done").replace(
            "paths: [src/**]\n",
            "paths: [src/**]\nevidence:\n- kind: test\n  ref: suite::proof\n",
        ),
    )
    .expect("task with test evidence");
    let started = fixture.directory().join("provider.started");
    let provider_pid = fixture.directory().join("provider.pid");
    let gate = fixture.directory().join("provider.gate");
    let script = fixture.directory().join("block-list.sh");
    std::fs::write(&gate, "held\n").expect("provider gate");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$$\" > '{}'\n: > '{}'\nwhile test -e '{}'; do :; done\nprintf 'suite::proof\\n'\n",
            provider_pid.display(),
            started.display(),
            gate.display()
        ),
    )
    .expect("provider script");
    let mut permissions = script.metadata().expect("script metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).expect("script executable");
    std::fs::write(
        fixture.directory().join(".aplan.yml"),
        "root: plan\ntest:\n  list: [./block-list.sh]\n  run: ['true', '{test}']\n",
    )
    .expect("config");
    common::git(fixture.directory(), &["init", "-q", "."]);

    let args = ["tui", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 100, 24, &args)?;
    terminal.wait_until(|screen| screen.contains("Overview - where attention goes next"))?;
    terminal.send(Key::Char('V'))?;
    terminal.wait_until(|screen| screen.contains("fix: exact repairs are a separate guarded"))?;
    terminal.wait_idle(Duration::from_millis(20))?;
    terminal.send(Key::Down)?;
    terminal.wait_until(|screen| screen.contains("> RUN VERIFY"))?;
    terminal.wait_idle(Duration::from_millis(20))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("verify in progress"))?;
    wait_for_file(&started);
    wait_for_file(&provider_pid);
    let provider_pid = std::fs::read_to_string(&provider_pid)
        .expect("provider pid")
        .trim()
        .parse::<u32>()
        .expect("numeric provider pid");

    terminal.signal(termlens::Signal::Term)?;
    let status = terminal.wait_exit()?;
    assert!(
        status
            .signal()
            .is_some_and(|observed| observed.contains("Terminated")),
        "termination became {status}"
    );
    assert_terminal_restored(&terminal.screen());
    wait_for_process_exit(provider_pid);
    assert!(
        gate.exists(),
        "test released the provider instead of killing it"
    );
    std::fs::remove_file(&gate).expect("remove provider gate");
    Ok(())
}

/// Proof for `tui.termination_preempts_worker_shutdown`: graceful `q` first
/// restores the terminal, then an out-of-band termination delivered while the
/// provider still holds worker shutdown is consumed, re-raised and propagated
/// to the registered provider group.
#[cfg(target_os = "linux")]
#[test]
fn termination_after_terminal_restoration_preempts_worker_shutdown() -> termlens::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = IdentifiedPlanFixture::new();
    let task = fixture.plan_root.join("tasks/T-001.md");
    let task_body = std::fs::read_to_string(&task).expect("task");
    std::fs::write(
        &task,
        task_body.replace("status: todo", "status: done").replace(
            "paths: [src/**]\n",
            "paths: [src/**]\nevidence:\n- kind: test\n  ref: suite::proof\n",
        ),
    )
    .expect("task with test evidence");
    let started = fixture.directory().join("shutdown-provider.started");
    let provider_pid = fixture.directory().join("shutdown-provider.pid");
    let gate = fixture.directory().join("shutdown-provider.gate");
    let script = fixture.directory().join("block-shutdown-list.sh");
    std::fs::write(&gate, "held\n").expect("provider gate");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$$\" > '{}'\n: > '{}'\nwhile test -e '{}'; do :; done\nprintf 'suite::proof\\n'\n",
            provider_pid.display(),
            started.display(),
            gate.display()
        ),
    )
    .expect("provider script");
    let mut permissions = script.metadata().expect("script metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).expect("script executable");
    std::fs::write(
        fixture.directory().join(".aplan.yml"),
        "root: plan\ntest:\n  list: [./block-shutdown-list.sh]\n  run: ['true', '{test}']\n",
    )
    .expect("config");
    common::git(fixture.directory(), &["init", "-q", "."]);

    let args = ["tui", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 100, 24, &args)?;
    terminal.wait_until(|screen| screen.contains("Overview - where attention goes next"))?;
    terminal.send(Key::Char('V'))?;
    terminal.wait_until(|screen| screen.contains("fix: exact repairs are a separate guarded"))?;
    terminal.send(Key::Down)?;
    terminal.wait_until(|screen| screen.contains("> RUN VERIFY"))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| screen.contains("verify in progress"))?;
    wait_for_file(&started);
    wait_for_file(&provider_pid);
    let provider_pid = std::fs::read_to_string(&provider_pid)
        .expect("provider pid")
        .trim()
        .parse::<u32>()
        .expect("numeric provider pid");

    terminal.send(Key::Char('q'))?;
    terminal.wait_until(|screen| {
        !screen.alternate_screen() && !screen.bracketed_paste() && screen.cursor_visible()
    })?;
    assert!(
        process_state(terminal.pid().expect("Unix child pid")).is_some(),
        "graceful quit did not wait for its active worker"
    );

    let tui_pid = terminal.pid().expect("Unix child pid");
    send_signal(tui_pid, "TSTP");
    wait_for_stopped_process(tui_pid);
    assert_terminal_restored(&terminal.screen());
    send_signal(tui_pid, "CONT");
    wait_for_resumed_process(tui_pid);
    assert_terminal_restored(&terminal.screen());

    terminal.signal(termlens::Signal::Term)?;
    let status = terminal.wait_exit()?;
    assert!(
        status
            .signal()
            .is_some_and(|observed| observed.contains("Terminated")),
        "termination during worker shutdown became {status}"
    );
    assert_terminal_restored(&terminal.screen());
    wait_for_process_exit(provider_pid);
    assert!(
        gate.exists(),
        "test released the provider instead of terminating it"
    );
    std::fs::remove_file(&gate).expect("remove provider gate");
    Ok(())
}

/// Proof for `tui.diagnostic_workbench_retains_complete_runs` and
/// `tui.check_overrides_are_one_run`: exercise both scopes, contextual rule and
/// task navigation, then prove a relaxed run is labelled and an ordinary
/// refresh restores repository policy in a real pseudo-terminal.
#[test]
fn diagnostic_workbench_runs_end_to_end_in_a_real_terminal() -> termlens::Result<()> {
    let fixture = IdentifiedPlanFixture::new();
    fixture.enable_diagnostic_workbench();
    let args = ["tui", "--root", "plan"];
    let mut terminal = common::tui_terminal(fixture.directory(), 120, 30, &args)?;

    terminal.wait_until(|screen| {
        screen.contains("Overview - where attention goes next")
            && screen.contains("FINDINGS E:0 W:1")
    })?;
    for _ in 0..4 {
        terminal.send(Key::Down)?;
    }
    terminal.send(Key::Enter)?;
    terminal
        .wait_until(|screen| screen.contains("check PASS E0 W1") && screen.contains("APLAN009"))?;

    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| {
        screen.contains("Finding APLAN009")
            && screen.contains("location:")
            && screen.contains("plan/tasks/T-001.md")
    })?;
    terminal.send(Key::Char('e'))?;
    terminal.wait_until(|screen| screen.contains("Rule explanation"))?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| {
        screen.contains("Finding APLAN009") && !screen.contains("Rule explanation")
    })?;
    terminal.send(Key::Char('o'))?;
    terminal
        .wait_until(|screen| screen.contains("T-001 / Findings") && screen.contains("APLAN009"))?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| screen.contains("Finding APLAN009"))?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| {
        screen.contains("check PASS E0 W1") && screen.contains("Esc back to overview")
    })?;

    terminal.send(Key::Char('v'))?;
    terminal.wait_until(|screen| {
        screen.contains("check-docs DENIED E1 W0") && screen.contains("DOC-REFERENCE")
    })?;
    terminal.send(Key::Enter)?;
    // `wait_until` may observe the sentinel during a repaint. Settle the
    // completed detail view before asserting the rest or sending Esc; the
    // coverage suite exposed the otherwise tiny input/repaint race (A2-003).
    let detail =
        terminal.snapshot_after(|screen| screen.contains("source: T-999 is not a task"))?;
    assert!(detail.contains("scope: check-docs"), "{detail:?}");
    assert!(detail.contains("subject: reference T-999"), "{detail:?}");
    assert!(detail.contains("location: /"), "{detail:?}");
    assert!(
        !detail.contains("location: no source location"),
        "{detail:?}"
    );
    assert!(detail.contains("Correct or remove the"), "{detail:?}");
    assert!(detail.contains("PLAN.md"), "{detail:?}");
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| {
        screen.contains("check-docs DENIED E1 W0") && screen.contains("Esc back to overview")
    })?;
    terminal.send(Key::Char('v'))?;
    terminal.wait_until(|screen| screen.contains("check PASS E0 W1"))?;

    terminal.send(Key::Char('a'))?;
    terminal.wait_until(|screen| screen.contains("Advanced check"))?;
    terminal.send(Key::Enter)?; // effort becomes optional for this request
    terminal.send(Key::Down)?;
    terminal.send(Key::Down)?;
    terminal.send(Key::Down)?;
    terminal.send(Key::Enter)?;
    terminal.wait_until(|screen| {
        screen.contains("check | ONE-RUN RELAXED PASS E0 W0") && !screen.contains("Advanced check")
    })?;

    terminal.send(Key::Char('r'))?;
    terminal.wait_until(|screen| {
        screen.contains("check PASS E0 W1") && !screen.contains("ONE-RUN RELAXED")
    })?;
    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    Ok(())
}

#[test]
fn invalid_configuration_guides_init_about_and_retry_in_a_real_terminal() -> termlens::Result<()> {
    let directory = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture directory");
    std::fs::write(directory.path().join(".aplan.yml"), "root: [\n").expect("invalid config");
    let mut terminal = common::tui_terminal(directory.path(), 120, 30, &["tui"])?;

    terminal.wait_until(|screen| {
        screen.contains("Configuration is invalid")
            && screen.contains("existing configuration")
            && screen.contains("r retry")
    })?;
    std::fs::remove_file(directory.path().join(".aplan.yml")).expect("remove invalid config");
    terminal.send(Key::Char('r'))?;
    terminal.wait_until(|screen| {
        screen.contains("No plan discovered")
            && screen.contains("Suggested commands (shown only; never run)")
            && screen.contains("aplan init")
    })?;

    terminal.send(Key::Char('i'))?;
    terminal.wait_until(|screen| {
        screen.contains("About this build")
            && screen.contains("Version:")
            && screen.contains("Counts:")
    })?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| screen.contains("No plan discovered"))?;

    let init = common::run(directory.path(), &["init"]);
    assert!(init.ok(), "guided init failed: {init:?}");
    std::fs::create_dir_all(directory.path().join("aplan/tasks")).expect("task directory");
    std::fs::write(
        directory.path().join("aplan/tasks/T-001.md"),
        "---\nid: T-001\ntitle: first task\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths: [src/**]\ntags: [cross-cutting]\n---\n",
    )
    .expect("first task");
    terminal.send(Key::Char('r'))?;
    terminal.wait_until_for(
        |screen| {
            screen.contains("Overview - where attention goes next")
                && screen.contains("snapshot: current")
        },
        Duration::from_secs(10),
    )?;

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_terminal_restored(&terminal.screen());
    Ok(())
}
