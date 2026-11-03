//! Executable terminal-state census for every non-interactive command
//! (`A1-548`).

#![cfg(unix)]

use crate::common;

use common::census::{RUNS, Start};

const MARKER: &str = "__APLAN_TERMIOS__|";

#[derive(Debug)]
struct TerminalResult {
    before: String,
    after: String,
    code: i32,
    output_before_marker: Vec<u8>,
}

fn fixture(start: Start) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture directory");
    if start == Start::Populated {
        common::census::a_plan_to_act_on(dir.path());
        common::git(dir.path(), &["init", "-q", "."]);
        common::git(dir.path(), &["add", "-A"]);
        common::git(
            dir.path(),
            &[
                "-c",
                "user.email=terminal@example.invalid",
                "-c",
                "user.name=terminal census",
                "commit",
                "-qm",
                "fixture",
            ],
        );
    }
    dir
}

fn run(name: &str, args: &[&str], start: Start) -> TerminalResult {
    let dir = fixture(start);
    let output = common::on_terminal_with_state(dir.path(), args);
    assert!(
        output.status.success(),
        "terminal wrapper failed for {name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let marker = output
        .stdout
        .windows(MARKER.len())
        .rposition(|window| window == MARKER.as_bytes())
        .unwrap_or_else(|| {
            panic!(
                "terminal wrapper produced no state marker for {name}: {}",
                String::from_utf8_lossy(&output.stdout)
            )
        });
    let state = String::from_utf8_lossy(&output.stdout[marker + MARKER.len()..]);
    let mut fields = state.trim().split('|');
    let before = fields.next().expect("before termios").to_string();
    let after = fields.next().expect("after termios").to_string();
    let code = fields
        .next()
        .expect("inner exit code")
        .parse()
        .expect("numeric inner exit code");
    assert_eq!(fields.next(), None, "unexpected terminal marker fields");
    TerminalResult {
        before,
        after,
        code,
        output_before_marker: output.stdout[..marker].to_vec(),
    }
}

fn allowed_terminal_control(sequence: &[u8]) -> bool {
    sequence.last() == Some(&b'm') || sequence == b"\x1b[2K"
}

fn assert_only_owned_controls(name: &str, bytes: &[u8]) {
    let mut cursor = 0;
    while let Some(relative) = bytes[cursor..].iter().position(|byte| *byte == 0x1b) {
        let start = cursor + relative;
        assert_eq!(
            bytes.get(start + 1),
            Some(&b'['),
            "{name} emitted a non-CSI terminal control at byte {start}: {:?}",
            &bytes[start..bytes.len().min(start + 24)]
        );
        let body = start + 2;
        let end = (body..bytes.len())
            .find(|index| (0x40..=0x7e).contains(&bytes[*index]))
            .unwrap_or_else(|| panic!("{name} emitted an unterminated CSI sequence"));
        assert!(
            allowed_terminal_control(&bytes[start..=end]),
            "{name} emitted an unowned CSI control {:?}",
            &bytes[start..=end]
        );
        cursor = end + 1;
    }
}

/// Proof for `tui.terminal_lifecycle_handles_signals`: the shared command
/// census drives every non-TUI command in a real PTY. Exact termios equality
/// and an escape-sequence allowlist prove ordinary CLI output cannot acquire
/// interactive terminal state. The historical test name remains stable for
/// evidence citations; the allowlist also admits the exact erase-line control
/// owned by the attended progress renderer.
#[test]
fn every_non_tui_command_preserves_terminal_state_and_emits_only_sgr() {
    let mut cases = RUNS
        .iter()
        .map(|(name, args, start)| (*name, *args, *start))
        .collect::<Vec<_>>();
    cases.push(("mcp", &["mcp"][..], Start::Populated));

    assert_eq!(
        common::census::NOT_RUN
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        ["mcp", "tui"],
        "the terminal census must disposition every census exemption"
    );

    for (name, args, start) in cases {
        let result = run(name, args, start);
        assert_ne!(result.before, "STTY_FAILED", "{name} had no PTY baseline");
        assert_eq!(
            result.before, result.after,
            "{name} changed the caller's termios state"
        );
        assert!(
            matches!(result.code, 0 | 1) || (name == "mcp" && result.code == 2),
            "{name} did not reach a normal command outcome: {}\n{}",
            result.code,
            String::from_utf8_lossy(&result.output_before_marker)
        );
        assert_only_owned_controls(name, &result.output_before_marker);
    }
}

#[test]
fn terminal_control_allowlist_admits_only_sgr_and_the_exact_progress_erase() {
    assert!(allowed_terminal_control(b"\x1b[m"));
    assert!(allowed_terminal_control(b"\x1b[1;32m"));
    assert!(allowed_terminal_control(b"\x1b[2K"));
    for unowned in [
        &b"\x1b[0K"[..],
        &b"\x1b[1K"[..],
        &b"\x1b[3K"[..],
        &b"\x1b[H"[..],
        &b"\x1b[?25l"[..],
        &b"\x1b[?1049h"[..],
    ] {
        assert!(!allowed_terminal_control(unowned), "{unowned:?}");
    }
}
