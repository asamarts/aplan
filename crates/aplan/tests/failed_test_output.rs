//! What a failed cited test is allowed to say in a finding (`A1-365`).
//!
//! The pilot's complaint, in one line: `verify --execute` rejected two cited
//! tests that failed only inside a hosted runner, and the whole actionable
//! output was `test ... failed under --execute` with the instruction to make
//! the test pass. Both streams were captured and then dropped, so the cause
//! could only be found by entering the runner and invoking each selector by
//! hand -- which exposed five independent boundary faults, none of which that
//! instruction could distinguish.
//!
//! These tests drive real runners rather than a mocked outcome, because what is
//! being checked is a *contract with a command this tool did not write*: that
//! its output reaches the reader bounded, readable and on every surface.

use std::path::Path;
use std::process::Command;

mod common;
use common::aplan;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env(
            "GIT_CEILING_DIRECTORIES",
            aplan_core::fixture::fixture_root(),
        )
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A plan with one closed task citing `known`, and a runner whose `run` fails
/// with `script` as its shell body.
fn plan_with_failing_runner(script: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::create_dir_all(dir.path().join("tests")).expect("tests dir");
    std::fs::write(dir.path().join("tests/suite"), "x").expect("file");
    std::fs::write(
        dir.path().join("aplan/tasks/T-001.md"),
        "---\nid: T-001\ntitle: proved by a test\nphase: 1\nstatus: done\n\
         paths:\n  - 'tests/**'\nevidence:\n  - kind: test\n    ref: known\n---\n\nbody\n",
    )
    .expect("task");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!(
            "test:\n  name: shell-shaped\n  list: [\"printf\", \"known\\\\n\"]\n  \
             run: [\"sh\", \"-c\", \"{script}\", \"--\", \"{{test}}\"]\n"
        ),
    )
    .expect("config");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "t@t"]);
    git(dir.path(), &["config", "user.name", "t"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "init"]);
    dir
}

/// **A runner that fails on stdout alone, and one that fails on stderr alone,
/// both leave the reason in the finding** (`A1-365`, criterion 1).
///
/// Both halves, because the pilot's own runner used one of them and a design
/// that served only stderr would have left it exactly where it started. Driven
/// through `sh` rather than a mocked outcome: the contract is with a command
/// this tool did not write.
#[test]
fn either_stream_reaches_the_reader_without_rerunning_the_command() {
    for (where_, script, needle) in [
        (
            "stdout",
            "echo 'FAIL known: inner cgroup undelegated'; exit 3",
            "inner cgroup undelegated",
        ),
        (
            "stderr",
            "echo 'panicked: rootless user namespace mismatch' >&2; exit 1",
            "rootless user namespace mismatch",
        ),
    ] {
        let dir = plan_with_failing_runner(script);
        let (out, code) = aplan(dir.path(), &["verify", "--execute"]);
        assert_ne!(code, 0, "a failing cited test must fail the run: {out}");
        assert!(
            out.contains("APLAN014"),
            "the failure was not reported at all ({where_}): {out}"
        );
        assert!(
            out.contains(needle),
            "the reason the test failed never reached the reader ({where_}), which is the \
             whole of what this task is about:\n{out}"
        );
        // And the stream is named, so a reader knows where to look when they do
        // go to the runner.
        assert!(
            out.contains(where_),
            "the stream is not named ({where_}):\n{out}"
        );
    }
}

/// **Every surface carries it, or says why it carries less** (`A1-365`,
/// criterion 4).
///
/// `detail` is a field rather than newlines inside the message precisely so
/// each surface can present it in the shape its consumer expects. JSON hands a
/// consumer an array it does not have to split a sentence to recover; SARIF
/// puts it in `message.text`, which is where a reviewer reads; the terminal
/// prints the lines unfolded, because reflowing an assertion to the screen
/// width rearranges the thing being read.
#[test]
fn json_sarif_and_the_terminal_all_carry_the_captured_output() {
    let dir = plan_with_failing_runner("echo 'assertion failed: 1 == 2' >&2; exit 1");

    let (human, _) = aplan(dir.path(), &["verify", "--execute"]);
    assert!(
        human.contains("assertion failed: 1 == 2"),
        "human:\n{human}"
    );

    let (json, _) = aplan(dir.path(), &["verify", "--execute", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&json).expect("json parses");
    let found = doc["diagnostics"]
        .as_array()
        .or_else(|| doc["findings"].as_array())
        .unwrap_or_else(|| panic!("no diagnostics array in:\n{json}"))
        .iter()
        .find(|d| d["code"] == "APLAN014")
        .unwrap_or_else(|| panic!("no APLAN014 in:\n{json}"));
    let detail = found["detail"]
        .as_array()
        .unwrap_or_else(|| panic!("`detail` is not an array:\n{found}"));
    assert!(
        detail.iter().any(|l| l
            .as_str()
            .is_some_and(|s| s.contains("assertion failed: 1 == 2"))),
        "json carries no captured output: {found}"
    );

    let (sarif, _) = aplan(dir.path(), &["verify", "--execute", "--format", "sarif"]);
    let doc: serde_json::Value = serde_json::from_str(&sarif).expect("sarif parses");
    let results = doc["runs"][0]["results"]
        .as_array()
        .unwrap_or_else(|| panic!("no results in:\n{sarif}"));
    let found = results
        .iter()
        .find(|r| r["ruleId"] == "APLAN014")
        .unwrap_or_else(|| panic!("no APLAN014 result in:\n{sarif}"));
    let text = found["message"]["text"].as_str().unwrap_or_default();
    assert!(
        text.contains("assertion failed: 1 == 2"),
        "sarif carries no captured output: {text}"
    );
}

/// **One failing test cited by many tasks shows its output once** (`A1-365`,
/// criterion 5).
///
/// The test runs once already (`A1-243`), and before this its finding carried
/// nothing, so repetition cost nothing. Now it carries lines, and on one
/// adopter's plan there are 86 citations over nine names: a copy per citation
/// is the page nobody reads. Two distinct failing tests stay distinguishable,
/// because every finding still names its test.
#[test]
fn a_failure_cited_twice_prints_its_output_once() {
    let dir = plan_with_failing_runner("echo 'the one explanation' >&2; exit 1");
    // A second task citing the same test.
    std::fs::write(
        dir.path().join("aplan/tasks/T-002.md"),
        "---\nid: T-002\ntitle: also proved by it\nphase: 1\nstatus: done\n\
         paths:\n  - 'tests/**'\nevidence:\n  - kind: test\n    ref: known\n---\n\nbody\n",
    )
    .expect("task");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "second"]);

    let (out, code) = aplan(dir.path(), &["verify", "--execute"]);
    assert_ne!(code, 0, "{out}");
    assert_eq!(
        out.matches("the one explanation").count(),
        1,
        "the captured output is repeated per citation:\n{out}"
    );
    // Both citations are still reported, and both still name the test.
    assert_eq!(
        out.matches("APLAN014").count(),
        2,
        "a citation stopped being judged:\n{out}"
    );
    // Counted on the finding lines, not on the whole output: the `fix:` text
    // names the test too, so a raw count is three and says nothing.
    let naming = out
        .lines()
        .filter(|l| l.contains("APLAN014") && l.contains("`known`"))
        .count();
    assert_eq!(
        naming, 2,
        "both findings must name the test they are about, so two distinct failing tests stay \
         distinguishable:\n{out}"
    );
}

/// **`output_lines: 0` suppresses it, and says the runner printed nothing**
/// (`A1-365`, criterion 2).
///
/// The escape hatch that makes the default bearable: a repository whose tests
/// print things it would rather not publish answers this in one key. The
/// finding then says the output is absent rather than implying the runner was
/// silent, because those are different facts and only one of them is the
/// reader's to fix.
#[test]
fn a_repository_can_suppress_the_output_entirely() {
    let dir = plan_with_failing_runner("echo 'a secret-shaped thing' >&2; exit 1");
    let config = std::fs::read_to_string(dir.path().join(".aplan.yml")).expect("read");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!("{config}  output_lines: 0\n"),
    )
    .expect("config");

    let (out, code) = aplan(dir.path(), &["verify", "--execute"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("APLAN014"), "{out}");
    assert!(
        !out.contains("a secret-shaped thing"),
        "`output_lines: 0` did not suppress the output:\n{out}"
    );
    assert!(
        out.contains("output_lines") || out.contains("printed nothing"),
        "the finding does not say why there is no output:\n{out}"
    );
}

/// **Five outcomes, still five** (`A1-365`, criterion 6).
///
/// A failed test now carries what it printed, and the risk in that is
/// collapsing distinctions the evidence ladder depends on: a command that could
/// not be *launched* proves nothing, a command the kernel *stopped* neither
/// passed nor failed, a test the runner *skips* was never run, and only the
/// last of these is a test that ran and failed. Reading an excerpt into any of
/// the other three would turn "unverified" into "disproved", which is the
/// misreading `A1-300` exists to prevent.
///
/// The cap case lives in `aplan-evidence/tests/cgroup_caps.rs`, which already
/// owns the cgroup environment question; what is checked here is the three that
/// need no kernel feature, and that only the real failure carries output.
#[test]
fn a_launch_failure_a_skip_and_a_real_failure_stay_three_different_things() {
    // 1. Could not be launched: the command does not exist.
    let dir = plan_with_failing_runner("exit 1");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "test:\n  name: shell-shaped\n  list: [\"printf\", \"known\\\\n\"]\n  \
         run: [\"no-such-program-anywhere\", \"{test}\"]\n",
    )
    .expect("config");
    let (out, _) = aplan(dir.path(), &["verify", "--execute"]);
    assert!(
        out.contains("could not run the tests cited against it"),
        "a command that cannot be launched must not read as a failing test:\n{out}"
    );
    assert!(
        !out.contains("failed under `--execute`"),
        "a launch failure was reported as a test that ran and failed:\n{out}"
    );

    // 2. The runner *claims* it skips the test. That claim is about the proof
    // the gate executes, not a gate on execution: the command still runs, so
    // both statements are made about the same citation and both are true. What
    // criterion 6 asks is that they stay two statements -- carrying output must
    // not fold "no test this task cites was run" into "it ran and failed".
    let dir = plan_with_failing_runner("exit 1");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "test:\n  name: shell-shaped\n  list: [\"printf\", \"known\\\\n\"]\n  \
         skipped: [\"printf\", \"known\\\\n\"]\n  \
         run: [\"sh\", \"-c\", \"echo nope >&2; exit 1\", \"--\", \"{test}\"]\n",
    )
    .expect("config");
    let (out, _) = aplan(dir.path(), &["verify", "--execute"]);
    assert!(
        out.contains("among the tests it skips"),
        "the skip claim stopped being reported:\n{out}"
    );
    assert!(
        out.contains("failed under `--execute`"),
        "the run that happened stopped being reported:\n{out}"
    );
    assert_eq!(
        out.matches("APLAN014").count(),
        2,
        "the two statements collapsed into one:\n{out}"
    );

    // 3. Ran and failed: the one outcome that carries what it printed.
    let dir = plan_with_failing_runner("echo 'the real reason' >&2; exit 1");
    let (out, _) = aplan(dir.path(), &["verify", "--execute"]);
    assert!(out.contains("failed under `--execute`"), "{out}");
    assert!(out.contains("the real reason"), "{out}");
}

/// **Hostile bytes survive the whole pipeline, not just the excerpt**
/// (`A1-365`, criterion 3).
///
/// The excerpt's own tests pin the sanitizing; this pins that the result still
/// *parses* once it has travelled through a renderer. A control byte that
/// reached a JSON string raw, or a line long enough to grow the document
/// without limit, would be a defect no unit test on the excerpt alone could
/// see.
#[test]
fn a_runner_printing_hostile_bytes_cannot_break_the_machine_formats() {
    // Escape sequences, an invalid byte, a carriage return, a credential shape
    // and a very long line, all from one failing command.
    // **The script lives in a file.** Writing these escapes into `.aplan.yml`
    // means YAML parses them first and refuses `\033` as an unknown escape --
    // which is the config loader doing its job, and nothing to do with what is
    // under test here.
    let dir = plan_with_failing_runner("exit 1");
    let script = dir.path().join("hostile.sh");
    std::fs::write(
        &script,
        "printf 'TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123 \\033[41mred\\033[0m\\r\\n' >&2\n\
         printf 'x%.0s' $(seq 1 4000) >&2; echo >&2\n\
         printf '\\377\\376 invalid\\n' >&2\n\
         exit 1\n",
    )
    .expect("script");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!(
            "test:\n  name: shell-shaped\n  list: [\"printf\", \"known\\\\n\"]\n  \
             run: [\"sh\", \"{}\", \"{{test}}\"]\n",
            script.display()
        ),
    )
    .expect("config");

    let (json, _) = aplan(dir.path(), &["verify", "--execute", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("hostile output broke the json document: {e}\n{json}"));
    let found = doc["diagnostics"]
        .as_array()
        .or_else(|| doc["findings"].as_array())
        .unwrap_or_else(|| panic!("no diagnostics in:\n{json}"))
        .iter()
        .find(|d| d["code"] == "APLAN014")
        .unwrap_or_else(|| panic!("no APLAN014 in:\n{json}"));
    let detail: Vec<&str> = found["detail"]
        .as_array()
        .unwrap_or_else(|| panic!("no detail array:\n{found}"))
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();

    assert!(!detail.is_empty(), "nothing was carried: {found}");
    for line in &detail {
        assert!(
            !line.contains('\u{1b}') && !line.contains('\r'),
            "a control byte reached the document: {line:?}"
        );
        // Bounded: the excerpt's width plus its marker, with room for the
        // stream indent the renderer adds.
        assert!(
            line.chars().count() < 400,
            "a line grew the document without limit: {} chars",
            line.chars().count()
        );
    }
    assert!(
        !detail
            .iter()
            .any(|l| l.contains("ghp_abcdefghijklmnopqrstuvwxyz0123")),
        "a credential shape reached the document: {detail:?}"
    );
    assert!(
        detail
            .iter()
            .any(|l| l.contains("not shown") || l.contains("[cut]"))
            || detail.len() <= 8,
        "truncation is invisible: {detail:?}"
    );

    // And SARIF, which carries it inside `message.text`.
    let (sarif, _) = aplan(dir.path(), &["verify", "--execute", "--format", "sarif"]);
    let doc: serde_json::Value = serde_json::from_str(&sarif)
        .unwrap_or_else(|e| panic!("hostile output broke the sarif document: {e}\n{sarif}"));
    assert!(
        doc["runs"][0]["results"].is_array(),
        "sarif lost its results"
    );
}

/// **The captured lines fold, and the width gate could not see that they did
/// not** (`A1-365`, audit).
///
/// `human_output_width` measures `check`, and `verify` is held to be covered by
/// it because "its findings are the same renderer". `APLAN014` is a `verify`
/// rule, so a finding carrying captured output never reaches that census: these
/// lines were the widest thing the tool emits and the one thing that gate was
/// structurally unable to measure. It passed vacuously.
///
/// The first version printed them raw, up to two hundred columns, which is
/// exactly the defect `A1-206` was filed about. They fold now, and this
/// measures what the renderer *contributes* -- the line minus its longest
/// unbreakable token -- which is the same property that census measures, so a
/// long path still survives whole and only prose wraps.
#[test]
fn the_captured_lines_fold_to_the_terminal_width() {
    const COLS: usize = 80;
    // A failure whose message is long foldable prose, which is the shape that
    // would overflow; a single long token is allowed to stay whole.
    let dir = plan_with_failing_runner(
        "echo 'assertion failed: the left value and the right value differ in a way that \
         takes a great many words to explain and therefore runs well past eighty columns' >&2; \
         exit 1",
    );

    let Some(out) = common::on_a_terminal(dir.path(), COLS, &["verify", "--execute"]) else {
        // No pty available: the width question cannot be asked here, and
        // failing would report the environment rather than the renderer.
        return;
    };
    assert!(out.contains("APLAN014"), "nothing was reported:\n{out}");
    assert!(
        out.contains("assertion failed"),
        "the captured output did not reach the terminal:\n{out}"
    );

    for line in out.lines() {
        let longest = line
            .split_whitespace()
            .map(aplan_output::fold::display_width)
            .max()
            .unwrap_or(0);
        let contributed = aplan_output::fold::display_width(line).saturating_sub(longest);
        assert!(
            contributed <= COLS,
            "the renderer contributed {contributed} columns past {COLS}, which is a line it \
             chose not to fold:\n{line}\n--- whole output ---\n{out}"
        );
    }
}
