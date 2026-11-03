//! Operation progress stays useful to a person and inert to automation.

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use crate::common;

fn fixture(root: &Path) {
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan directory");
    std::fs::create_dir_all(root.join("runner")).expect("runner directory");
    let runner = root.join("runner/list.sh");
    std::fs::write(
        &runner,
        "#!/bin/sh\nsleep 0.30\nprintf '%s\\n' 'suite::proof'\n",
    )
    .expect("runner");
    let mut permissions = runner.metadata().expect("runner metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&runner, permissions).expect("executable runner");
    std::fs::write(
        root.join(".aplan.yml"),
        "root: aplan\ntest:\n  name: private-label-must-not-leak\n  dir: runner\n  list: [sh, list.sh]\n  run: ['true', '{test}']\n",
    )
    .expect("config");
    std::fs::write(
        root.join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: task\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/**]\ntags: [cross-cutting]\n---\n",
    )
    .expect("task");
}

fn progress_lines(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .filter(|line| line.starts_with("progress: "))
        .collect()
}

#[test]
fn progress_reducer_refusal_neighbours_are_deterministic() {
    use aplan_app::{
        OperationId, OperationKind, ProgressEvent, ProgressLifecycle, ProgressMeasure,
        ProgressPhase, ProgressRejection, ProgressState, ProgressSubject,
    };

    let operation = OperationId::new(41);
    let event = |sequence, subject, lifecycle| ProgressEvent {
        operation,
        sequence,
        kind: OperationKind::PlanLoad,
        subject,
        lifecycle,
        measure: ProgressMeasure::Indeterminate,
    };
    let phase = ProgressSubject::Phase {
        phase: ProgressPhase::SourceParsing,
        child: None,
    };

    let mut state = ProgressState::new(operation, OperationKind::PlanLoad);
    assert_eq!(
        state.accept(event(
            0,
            ProgressSubject::Operation,
            ProgressLifecycle::Completed,
        )),
        Err(ProgressRejection::InvalidLifecycle)
    );

    state
        .accept(event(
            0,
            ProgressSubject::Operation,
            ProgressLifecycle::Queued,
        ))
        .expect("operation queues");
    state
        .accept(event(
            1,
            ProgressSubject::Operation,
            ProgressLifecycle::Started,
        ))
        .expect("operation starts");
    assert_eq!(
        state.accept(event(2, phase, ProgressLifecycle::Queued)),
        Err(ProgressRejection::InvalidLifecycle)
    );
    state
        .accept(event(2, phase, ProgressLifecycle::Started))
        .expect("phase starts");
    state
        .accept(event(3, phase, ProgressLifecycle::Completed))
        .expect("phase completes");
    assert_eq!(
        state.accept(event(4, phase, ProgressLifecycle::Advanced)),
        Err(ProgressRejection::InvalidLifecycle)
    );
    assert_eq!(
        state.accept(event(4, phase, ProgressLifecycle::Completed)),
        Err(ProgressRejection::InvalidLifecycle)
    );
}

fn multi_runner_fixture(root: &Path) {
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan directory");
    std::fs::create_dir_all(root.join("runner")).expect("runner directory");
    for (file, delay, proof) in [
        ("one.sh", "0.30", "suite::one"),
        ("two.sh", "0.35", "suite::two"),
    ] {
        let path = root.join("runner").join(file);
        std::fs::write(
            &path,
            format!("#!/bin/sh\nsleep {delay}\nprintf '%s\\n' '{proof}'\n"),
        )
        .expect("runner");
        let mut permissions = path.metadata().expect("runner metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("executable runner");
    }
    std::fs::write(
        root.join(".aplan.yml"),
        "root: aplan\ntest:\n  schedule: {capacity: 2}\n  runners:\n    - name: private-one\n      dir: runner\n      list: [sh, one.sh]\n      run: ['true', '{test}']\n    - name: private-two\n      dir: runner\n      list: [sh, two.sh]\n      run: ['true', '{test}']\n",
    )
    .expect("config");
    std::fs::write(
        root.join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: task\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/**]\ntags: [cross-cutting]\n---\n",
    )
    .expect("task");
}

fn failing_runner_fixture(root: &Path) {
    fixture(root);
    let runner = root.join("runner/list.sh");
    std::fs::write(&runner, "#!/bin/sh\nsleep 0.30\nexit 9\n").expect("runner");
    let mut permissions = runner.metadata().expect("runner metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(runner, permissions).expect("executable runner");
}

fn indeterminate_fixture(root: &Path) {
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan directory");
    std::fs::write(
        root.join(".aplan.yml"),
        "root: aplan\nversion:\n  current:\n    command: [sh, ./version.sh]\n",
    )
    .expect("config");
    std::fs::write(
        root.join("version.sh"),
        "sleep 0.30\nprintf '%s\\n' 0.3.0\n",
    )
    .expect("version provider");
    std::fs::write(
        root.join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: task\nphase: 1\nstatus: todo\neffort_days: 1\ntags: [cross-cutting]\n---\n",
    )
    .expect("task");
    common::git(root, &["init", "-q", "."]);
    common::git(root, &["config", "user.email", "t@example.invalid"]);
    common::git(root, &["config", "user.name", "t"]);
    common::git(root, &["add", "-A"]);
    common::git(root, &["commit", "-q", "-m", "release"]);
    common::git(root, &["tag", "v0.3.0"]);
}

/// Proof for `progress.stream_output_is_automation_safe`.
#[test]
fn explicit_progress_is_stderr_only_and_never_changes_structured_stdout() {
    let temporary = tempfile::Builder::new()
        .prefix("aplan-progress-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    fixture(temporary.path());

    let always = common::run(
        temporary.path(),
        &["--progress", "always", "test", "list", "--format", "json"],
    );
    let never = common::run(
        temporary.path(),
        &["--progress", "never", "test", "list", "--format", "json"],
    );
    assert_eq!(always.code, 0, "{}", always.text());
    assert_eq!(never.code, 0, "{}", never.text());
    assert_eq!(always.stdout, never.stdout, "progress changed stdout bytes");
    serde_json::from_str::<serde_json::Value>(&always.stdout).expect("structured stdout");
    let lines = progress_lines(&always.stderr);
    assert!(!lines.is_empty(), "explicit progress was not emitted");
    assert!(
        lines.iter().any(|line| line.contains("0/1 providers")),
        "the known bound was not rendered: {lines:?}"
    );
    assert!(
        lines.iter().all(|line| {
            line.is_ascii()
                && !line.contains('\u{1b}')
                && !line.contains("private-label-must-not-leak")
        }),
        "progress was not the privacy-safe ASCII fallback: {lines:?}"
    );
    assert!(
        never.stderr.is_empty(),
        "--progress never emitted stderr bytes: {}",
        never.stderr
    );
}

#[test]
fn command_lifetime_progress_respects_grace_or_reports_a_real_slow_run() {
    let temporary = tempfile::Builder::new()
        .prefix("aplan-progress-short-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    fixture(temporary.path());

    // `check` is classified as plan-loading rather than inapplicable, so this
    // exercises the command-lifetime grace gate rather than a census shortcut.
    let started = std::time::Instant::now();
    let result = common::run(temporary.path(), &["--progress", "always", "check"]);
    let elapsed = started.elapsed();
    assert_eq!(result.code, 0, "{}", result.text());
    let lines = progress_lines(&result.stderr);
    if lines.is_empty() {
        return;
    }

    // Instrumentation or a saturated shared host can make even this tiny
    // fixture genuinely outlive the grace period. That is not a flash: the
    // deterministic gate proof in `aplan-output` establishes the before-grace
    // boundary. This process-level proof establishes that visible progress is
    // reserved for a run whose observed lifetime could have crossed it and
    // still terminates cleanly.
    assert!(
        elapsed >= aplan_output::progress::ProgressTiming::default().grace,
        "progress appeared before the grace period ({elapsed:?}): {}",
        result.stderr
    );
    // The scheduler decides which phase is active when the renderer first
    // gets to draw. This command-lifetime proof owns the active/terminal
    // lifecycle; deterministic tests below own the phase-specific wording.
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("progress: ") && line.ends_with(": active")),
        "a slow run omitted its active state: {lines:?}"
    );
    assert_eq!(
        lines.last().copied(),
        Some("progress: command: complete"),
        "a slow run omitted its terminal state: {lines:?}"
    );
}

#[test]
fn a_long_indeterminate_phase_is_named_without_a_fake_percentage() {
    let temporary = tempfile::Builder::new()
        .prefix("aplan-progress-indeterminate-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    indeterminate_fixture(temporary.path());

    let result = common::run(temporary.path(), &["--progress", "always", "version"]);
    assert_eq!(result.code, 0, "{}", result.text());
    let lines = progress_lines(&result.stderr);
    assert!(
        lines
            .iter()
            .any(|line| line == &"progress: building the requested view: active"),
        "the indeterminate wait was not named: {lines:?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains('%')),
        "indeterminate progress invented a percentage: {lines:?}"
    );
}

/// Proof for `progress.cli_renderer_is_identity_safe_and_terminal_safe`.
#[test]
fn a_slow_command_error_emits_one_failed_terminal_before_the_diagnostic() {
    let temporary = tempfile::Builder::new()
        .prefix("aplan-progress-error-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    indeterminate_fixture(temporary.path());
    std::fs::write(temporary.path().join("version.sh"), "sleep 0.30\nexit 9\n")
        .expect("failing version provider");

    let result = common::run(temporary.path(), &["--progress", "always", "version"]);
    assert_eq!(result.code, 2, "{}", result.text());
    let lines = progress_lines(&result.stderr);
    assert_eq!(
        lines.last().copied(),
        Some("progress: command: failed"),
        "the failed command left no terminal progress state: {lines:?}"
    );
    let terminal = result
        .stderr
        .find("progress: command: failed")
        .expect("terminal progress");
    let diagnostic = result.stderr.rfind("aplan:").expect("final diagnostic");
    assert!(
        terminal < diagnostic,
        "the diagnostic was written before progress terminated: {}",
        result.stderr
    );
}

/// Proof for `progress.stream_output_is_automation_safe` and
/// `progress.cli_renderer_is_identity_safe_and_terminal_safe`.
#[test]
fn auto_is_hidden_in_pipes_and_machine_modes_but_visible_for_an_attended_human_mode() {
    let temporary = tempfile::Builder::new()
        .prefix("aplan-progress-auto-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    fixture(temporary.path());

    let piped = common::run(temporary.path(), &["test", "list"]);
    assert_eq!(piped.code, 0, "{}", piped.text());
    assert!(progress_lines(&piped.stderr).is_empty());

    let Some(machine_terminal) = common::on_a_terminal_raw(
        temporary.path(),
        100,
        &[("CI", "0")],
        &["test", "list", "--format", "json"],
    ) else {
        return;
    };
    assert!(
        !machine_terminal.contains("progress: "),
        "{machine_terminal}"
    );

    let attended = common::on_a_terminal_raw(
        temporary.path(),
        100,
        &[("CI", "0"), ("NO_COLOR", "1")],
        &["test", "list"],
    )
    .expect("script(1) is available after the preceding invocation");
    assert!(attended.contains("progress: "), "{attended}");
    assert!(
        attended
            .lines()
            .filter(|line| line.starts_with("progress: "))
            .all(|line| !line.contains("private-label-must-not-leak")),
        "{attended}"
    );
    assert!(
        attended.contains("\u{1b}[2K"),
        "the animated row was never cleared: {attended:?}"
    );
    assert!(
        !attended.contains("\u{1b}[?25l") && !attended.contains("\u{1b}[?25h"),
        "stream progress changed cursor visibility: {attended:?}"
    );
}

/// Proof for `progress.cli_renderer_is_identity_safe_and_terminal_safe`.
#[test]
fn concurrent_runners_are_aggregated_without_identity_or_escape_leaks() {
    let temporary = tempfile::Builder::new()
        .prefix("aplan-progress-multi-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    multi_runner_fixture(temporary.path());

    let result = common::run(
        temporary.path(),
        &["--progress", "always", "test", "list", "--format", "json"],
    );
    assert_eq!(result.code, 0, "{}", result.text());
    serde_json::from_str::<serde_json::Value>(&result.stdout).expect("structured stdout");
    let lines = progress_lines(&result.stderr);
    assert!(
        lines.iter().any(|line| line.contains("0/2 providers")),
        "concurrent work was not aggregated: {lines:?}"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.ends_with("command: complete"))
            .count(),
        1,
        "the command did not have one terminal row: {lines:?}"
    );
    assert!(
        lines.iter().all(|line| {
            line.is_ascii()
                && !line.contains('\u{1b}')
                && !line.contains("private-one")
                && !line.contains("private-two")
        }),
        "progress leaked an identity or terminal sequence: {lines:?}"
    );
}

#[test]
fn a_slow_provider_failure_terminates_progress_before_the_result() {
    let temporary = tempfile::Builder::new()
        .prefix("aplan-progress-failure-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    failing_runner_fixture(temporary.path());

    let result = common::run(
        temporary.path(),
        &["--progress", "always", "test", "list", "--format", "json"],
    );
    assert_eq!(result.code, 1, "{}", result.text());
    let document = serde_json::from_str::<serde_json::Value>(&result.stdout)
        .expect("failure still has structured output");
    assert_eq!(document["catalogue"]["state"], "partial");
    let lines = progress_lines(&result.stderr);
    assert!(
        lines.iter().any(|line| line.contains("0/1 providers")),
        "the wait was not visible: {lines:?}"
    );
    assert_eq!(
        lines.last().copied(),
        Some("progress: command: complete"),
        "a denied provider result left stale progress: {lines:?}"
    );
}
