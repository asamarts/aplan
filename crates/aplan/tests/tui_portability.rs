//! Portable real-PTY lifecycle checks for the interactive interface (`A1-448`).
//!
//! Keep this module independent of Unix-only fixture machinery, but route every
//! binary launch through `tests/common` so the workspace-wide invocation census
//! observes these journeys on every supported CI system.

#![cfg(feature = "tui")]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use termlens::Key;

use crate::common;

const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug)]
struct Fixture {
    directory: tempfile::TempDir,
    plan_root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("tui-portability-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("portable TUI fixture");
        let plan_root = directory.path().join("plan");
        std::fs::create_dir_all(plan_root.join("tasks")).expect("task directory");
        std::fs::create_dir_all(directory.path().join("src")).expect("declared path");
        std::fs::write(directory.path().join("src/lib.rs"), "pub fn fixture() {}\n")
            .expect("declared file");
        std::fs::write(
            plan_root.join("tasks/T-001.md"),
            "---\n\
             id: T-001\n\
             title: Inspect portable lifecycle\n\
             phase: 1\n\
             status: todo\n\
             effort_days: 1.0\n\
             blocked_by: []\n\
             paths: [src/**]\n\
             tags: [cross-cutting]\n\
             ---\n\n\
             ## Why\n\n\
             Exercise one identified task.\n",
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

    fn task(&self) -> PathBuf {
        self.plan_root.join("tasks/T-001.md")
    }
}

fn terminal(
    fixture: &Fixture,
    environment: impl IntoIterator<Item = (String, String)>,
) -> termlens::Result<termlens::Terminal> {
    let environment: Vec<(String, String)> = environment.into_iter().collect();
    common::tui_terminal_with_environment(
        fixture.directory(),
        100,
        24,
        &["tui", "--no-config", "--root", "plan"],
        &environment,
    )
}

fn assert_owned(screen: &termlens::Screen) {
    assert!(screen.alternate_screen());
    assert!(screen.bracketed_paste());
    assert!(screen.focus_events());
    assert!(!screen.cursor_visible());
}

fn assert_restored(screen: &termlens::Screen) {
    assert!(!screen.alternate_screen());
    assert!(!screen.bracketed_paste());
    // Crossterm documents focus events as permanently enabled by the Windows
    // console API. Its disable command is consequently a no-op there: aplan
    // did not acquire that state and cannot restore it. On terminals where
    // mode 1004 is owned by the application, the disable sequence is part of
    // the observable restoration contract.
    #[cfg(not(windows))]
    assert!(!screen.focus_events());
    assert!(screen.cursor_visible());
}

fn frame_border_reaches_right_edge(screen: &termlens::Screen) -> bool {
    let right = screen.cols().saturating_sub(1);
    (0..screen.rows()).any(|row| {
        screen
            .cell(row, right)
            .is_some_and(|cell| matches!(cell.contents(), "+" | "\u{2510}"))
    })
}

fn open_add_form(terminal: &mut termlens::Terminal) -> termlens::Result<()> {
    terminal.send(Key::Ctrl('p'))?;
    terminal.wait_until(|screen| screen.contains("Action:"))?;
    for character in "Add task (CLI: aplan add)".chars() {
        terminal.send(Key::Char(character))?;
    }
    terminal.wait_until(|screen| screen.contains("Add task (CLI: aplan add)"))?;
    terminal.send(Key::Enter)?;
    terminal.wait_until_for(
        |screen| screen.contains("FORM: Add task"),
        Duration::from_secs(20),
    )
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + TIMEOUT;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn portable_process_environment() -> Vec<(String, String)> {
    [
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
    ]
    .into_iter()
    .filter_map(|key| {
        std::env::var_os(key)
            .and_then(|value| value.into_string().ok().map(|value| (key.into(), value)))
    })
    .collect()
}

#[cfg(unix)]
fn editor_environment(directory: &Path, marker: &Path, gate: &Path) -> Vec<(String, String)> {
    use std::os::unix::fs::PermissionsExt as _;

    let script = directory.join("portable-editor.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         : > \"$APLAN_EDITOR_MARKER\"\n\
         while test -e \"$APLAN_EDITOR_GATE\"; do sleep 0.01; done\n\
         printf '%s' '## Why\n\nEdited outside the TUI.\n' > \"$1\"\n",
    )
    .expect("editor script");
    let mut permissions = std::fs::metadata(&script)
        .expect("editor metadata")
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&script, permissions).expect("private editor script");
    let mut environment = portable_process_environment();
    environment.extend([
        ("VISUAL".to_string(), script.display().to_string()),
        (
            "APLAN_EDITOR_MARKER".to_string(),
            marker.display().to_string(),
        ),
        ("APLAN_EDITOR_GATE".to_string(), gate.display().to_string()),
    ]);
    environment
}

#[cfg(windows)]
fn editor_environment(directory: &Path, marker: &Path, gate: &Path) -> Vec<(String, String)> {
    let script = directory.join("portable-editor.cmd");
    std::fs::write(
        &script,
        "@echo off\r\n\
         type nul > \"%APLAN_EDITOR_MARKER%\"\r\n\
         :wait\r\n\
         if exist \"%APLAN_EDITOR_GATE%\" (\r\n\
           ping -n 2 127.0.0.1 > nul\r\n\
           goto wait\r\n\
         )\r\n\
         > \"%~1\" (\r\n\
           echo ## Why\r\n\
           echo(\r\n\
           echo Edited outside the TUI.\r\n\
         )\r\n",
    )
    .expect("editor script");
    let command = "cmd.exe /D /Q /C portable-editor.cmd".to_string();
    let mut environment = portable_process_environment();
    environment.extend([
        ("VISUAL".to_string(), command),
        (
            "APLAN_EDITOR_MARKER".to_string(),
            marker.display().to_string(),
        ),
        ("APLAN_EDITOR_GATE".to_string(), gate.display().to_string()),
    ]);
    environment
}

#[cfg(unix)]
fn blocked_provider(fixture: &Fixture, marker: &Path, gate: &Path) -> Vec<(String, String)> {
    let script = fixture.directory().join("blocked-provider.sh");
    std::fs::write(
        &script,
        ": > \"$APLAN_PROVIDER_MARKER\"\n\
         while test -e \"$APLAN_PROVIDER_GATE\"; do sleep 0.01; done\n\
         printf '%s\\n' 'suite::proof'\n",
    )
    .expect("provider script");
    std::fs::write(
        fixture.directory().join(".aplan.yml"),
        "root: plan\ntest:\n  list: [/bin/sh, blocked-provider.sh]\n  run: ['true', '{test}']\n",
    )
    .expect("provider configuration");
    let mut environment = portable_process_environment();
    environment.extend([
        (
            "APLAN_PROVIDER_MARKER".to_string(),
            marker.display().to_string(),
        ),
        (
            "APLAN_PROVIDER_GATE".to_string(),
            gate.display().to_string(),
        ),
    ]);
    environment
}

#[cfg(windows)]
fn blocked_provider(fixture: &Fixture, marker: &Path, gate: &Path) -> Vec<(String, String)> {
    let script = fixture.directory().join("blocked-provider.cmd");
    std::fs::write(
        &script,
        "@echo off\r\n\
         type nul > \"%APLAN_PROVIDER_MARKER%\"\r\n\
         :wait\r\n\
         if exist \"%APLAN_PROVIDER_GATE%\" (\r\n\
           ping -n 2 127.0.0.1 > nul\r\n\
           goto wait\r\n\
         )\r\n\
         echo suite::proof\r\n",
    )
    .expect("provider script");
    std::fs::write(
        fixture.directory().join("successful-run.cmd"),
        "@exit /B 0\r\n",
    )
    .expect("successful runner script");
    std::fs::write(
        fixture.directory().join(".aplan.yml"),
        "root: plan\ntest:\n  list: [cmd.exe, /D, /Q, /C, blocked-provider.cmd]\n  run: [cmd.exe, /D, /Q, /C, successful-run.cmd, '{test}']\n",
    )
    .expect("provider configuration");
    let mut environment = portable_process_environment();
    environment.extend([
        (
            "APLAN_PROVIDER_MARKER".to_string(),
            marker.display().to_string(),
        ),
        (
            "APLAN_PROVIDER_GATE".to_string(),
            gate.display().to_string(),
        ),
    ]);
    environment
}

/// Proof for `tui.terminal_portability_is_explicit`: the same executable
/// acquires its complete terminal contract, survives focus and resize bursts,
/// renders the minimum-size refusal, recovers, and restores every visible
/// mode on an ordinary quit. This test is the public three-OS CI matrix's
/// shared real-PTY smoke.
#[test]
fn shared_real_pty_lifecycle_handles_focus_resize_and_tiny_terminals() -> termlens::Result<()> {
    let fixture = Fixture::new();
    let mut terminal = terminal(&fixture, [])?;
    terminal.wait_until(|screen| {
        screen.contains("Overview - where attention goes next") && screen.focus_events()
    })?;
    assert_owned(&terminal.screen());

    terminal.focus_out()?;
    terminal.focus_in()?;
    for (columns, rows) in [(80, 20), (49, 14), (47, 13), (64, 16), (100, 24)] {
        terminal.resize(columns, rows)?;
        terminal.wait_until(|screen| {
            screen.cols() == columns
                && screen.rows() == rows
                && if columns < 48 || rows < 14 {
                    screen.contains("TERMINAL TOO SMALL")
                } else {
                    frame_border_reaches_right_edge(screen)
                }
        })?;
    }
    terminal.wait_until(|screen| {
        screen.cols() == 100
            && screen.rows() == 24
            && screen.contains("Overview - where attention goes next")
    })?;
    assert_owned(&terminal.screen());

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_restored(&terminal.screen());
    Ok(())
}

/// Proof for `tui.external_editor_handoff_is_safe_and_identity_bound`: the
/// real application restores terminal ownership before starting the editor,
/// accepts its private draft only after success, then re-observes a plan edit
/// made while the editor owned the terminal before returning to normal input.
#[test]
fn external_editor_handoff_restores_modes_and_refreshes_identity() -> termlens::Result<()> {
    let fixture = Fixture::new();
    let marker = fixture.directory().join("editor-started");
    let gate = fixture.directory().join("editor-gate");
    std::fs::write(&gate, "held\n").expect("editor gate");
    let environment = editor_environment(fixture.directory(), &marker, &gate);
    let mut terminal = terminal(&fixture, environment)?;
    terminal.wait_until(|screen| screen.contains("Overview - where attention goes next"))?;
    open_add_form(&mut terminal)?;

    terminal.send(Key::Ctrl('e'))?;
    wait_for_file(&marker);
    terminal.wait_until(|screen| {
        !screen.alternate_screen()
            && !screen.bracketed_paste()
            && !screen.focus_events()
            && screen.cursor_visible()
    })?;
    assert_restored(&terminal.screen());

    let task = fixture.task();
    let before = std::fs::read_to_string(&task).expect("task before editor-side change");
    std::fs::write(
        &task,
        before.replace(
            "title: Inspect portable lifecycle",
            "title: Observe editor-side refresh",
        ),
    )
    .expect("editor-side plan change");
    std::fs::remove_file(&gate).expect("release editor");

    terminal.wait_until_for(
        |screen| {
            screen.alternate_screen()
                && screen.contains("External body: 32 byte(s)")
                && screen.contains("Ctrl-E reopens it")
        },
        TIMEOUT,
    )?;
    terminal.wait_until_for(
        |screen| screen.contains("Refresh complete") && screen.contains("plan input changed"),
        TIMEOUT,
    )?;
    terminal.send(Key::Esc)?;
    terminal.wait_until(|screen| !screen.contains("FORM: Add task"))?;
    terminal.send(Key::Char('2'))?;
    terminal.wait_until(|screen| screen.contains("Observe editor-side refresh"))?;

    terminal.send(Key::Char('q'))?;
    assert!(terminal.wait_exit()?.success());
    assert_restored(&terminal.screen());
    Ok(())
}

#[test]
fn portable_control_c_restores_terminal_modes() -> termlens::Result<()> {
    let fixture = Fixture::new();
    let mut terminal = terminal(&fixture, [])?;
    terminal.wait_until(|screen| screen.contains("Overview - where attention goes next"))?;
    terminal.send(Key::Ctrl('c'))?;
    assert!(terminal.wait_exit()?.success());
    assert_restored(&terminal.screen());
    Ok(())
}

/// Cross-platform process proof for
/// `progress.cli_renderer_is_identity_safe_and_terminal_safe`: interruption of
/// a real slow provider removes the attended progress row, leaves the cursor
/// visible and returns a non-success process result without acquiring TUI
/// modes.
#[test]
fn portable_cli_progress_interrupt_cleans_its_terminal_row() -> termlens::Result<()> {
    let fixture = Fixture::new();
    let marker = fixture.directory().join("provider-started");
    let gate = fixture.directory().join("provider-gate");
    std::fs::write(&gate, "held\n").expect("provider gate");
    let environment = blocked_provider(&fixture, &marker, &gate);
    let mut terminal = common::tui_terminal_with_environment(
        fixture.directory(),
        100,
        24,
        &["--progress", "always", "test", "list"],
        &environment,
    )?;
    wait_for_file(&marker);
    terminal.wait_until(|screen| screen.contains("progress:"))?;
    terminal.send(Key::Ctrl('c'))?;
    assert!(!terminal.wait_exit()?.success());
    let screen = terminal.screen();
    assert_restored(&screen);
    assert!(
        !screen.contains("progress:"),
        "interruption left a partial progress row: {screen:?}"
    );
    Ok(())
}
