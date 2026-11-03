//! A release records the version it produced when it closes (`A1-339`,
//! ADR-0038).
//!
//! Driven through the built binary rather than through `close`, for `A1-107`'s
//! reason: the unit test beside `close` proves the rewrite, and if `cmd_close`
//! stopped deriving a number tomorrow that test would still pass. What is under
//! test here is the seam -- the declared command actually spawned, the window
//! actually walked, and the closure that goes through without a number when one
//! cannot be had.
//!
//! Not a `trycmd` case: closing writes, and the corpus runs in its own fixture
//! directory (`A1-109`'s note).

use std::path::Path;

use crate::common;

fn tmp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir")
}

/// A repository that shipped `v1.0.0`, with an open release over one breaking
/// task and a tree that has been bumped to `2.0.0`.
fn fixture(dir: &Path, config: &str) -> String {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), config).expect("config");
    std::fs::write(dir.join("version.sh"), "echo 1.0.0\n").expect("version.sh");
    std::fs::write(
        dir.join("aplan/tasks/W-1.md"),
        "---\nid: W-1\ntitle: a break\nphase: 1\nstatus: done\neffort_days: 1\n\
         version_impact: breaking\nevidence:\n  - kind: note\n    ref: shipped\n    \
         captured: 2026-09-24\n---\n\nbody\n",
    )
    .expect("task");
    std::fs::write(
        dir.join("aplan/tasks/M-r.md"),
        "---\nid: M-r\ntitle: the release\nphase: 1\nstatus: todo\n\
         tags:\n  - milestone\n  - release\nblocked_by:\n  - W-1\n---\n\nbody\n",
    )
    .expect("release");
    common::git(dir, &["init", "-q", "."]);
    common::git(dir, &["config", "user.email", "t@example.invalid"]);
    common::git(dir, &["config", "user.name", "t"]);
    common::git(dir, &["add", "-A"]);
    common::git(dir, &["commit", "-q", "-m", "the plan"]);
    common::git(dir, &["tag", "v1.0.0"]);
    std::fs::write(dir.join("version.sh"), "echo 2.0.0\n").expect("bump");
    common::git(dir, &["add", "-A"]);
    common::git(dir, &["commit", "-q", "-m", "bump"]);
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .expect("git rev-parse");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const WITH_READER: &str = "root: aplan\nversion:\n  current:\n    command: [sh, ./version.sh]\n  \
                           levels:\n    cut: none\n";

fn recorded(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("aplan/tasks/M-r.md")).expect("read");
    text.lines()
        .find_map(|l| l.strip_prefix("version: "))
        .map(str::to_string)
}

/// **Closing a release writes the derived number into the node**, and asking
/// afterwards gives the record rather than a fresh derivation (`A1-339`).
#[test]
fn closing_a_release_records_what_it_produced_and_the_record_stands() {
    let dir = tmp();
    let sha = fixture(dir.path(), WITH_READER);
    let out = common::run(
        dir.path(),
        &["close", "M-r", "--evidence", &format!("commit:{sha}")],
    );
    assert_eq!(out.code, 0, "{}", out.text());
    assert_eq!(
        recorded(dir.path()).as_deref(),
        Some("2.0.0"),
        "{}",
        out.text()
    );

    // The record answers, and it is the record rather than a derivation.
    let asked = common::run(dir.path(), &["version", "M-r"]);
    assert_eq!(asked.stdout, "2.0.0\n", "{}", asked.text());
    assert!(
        asked.stderr.contains("recorded when it closed"),
        "{}",
        asked.stderr
    );

    // **And it does not move when the work it closed is edited**, which is the
    // whole argument for writing it down: a closed release whose answer changes
    // when somebody edits a task it closed is not a record.
    let task = dir.path().join("aplan/tasks/W-1.md");
    let text = std::fs::read_to_string(&task).expect("read");
    std::fs::write(
        &task,
        text.replace(
            "version_impact: breaking",
            "version_impact: none\nversion_impact_reason: edited after the fact",
        ),
    )
    .expect("write");
    let again = common::run(dir.path(), &["version", "M-r", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&again.stdout).expect("json");
    assert_eq!(v["recorded"], "2.0.0", "the record moved: {}", again.stdout);
    assert_eq!(v["owes"], "none", "a fresh derivation should have moved");
}

/// A closure is never refused for a version that cannot be derived: it says so
/// and goes through, because the field is a record rather than a gate.
#[test]
fn a_release_closes_without_a_number_when_none_can_be_derived() {
    let dir = tmp();
    let sha = fixture(dir.path(), "root: aplan\n");
    let out = common::run(
        dir.path(),
        &["close", "M-r", "--evidence", &format!("commit:{sha}")],
    );
    assert_eq!(out.code, 0, "the closure must go through: {}", out.text());
    assert_eq!(recorded(dir.path()), None);
    assert!(
        out.stderr
            .contains("without recording the version it produced"),
        "it has to say so: {}",
        out.stderr
    );

    // And an ordinary task says nothing at all, because the field is a
    // release's.
    let dir = tmp();
    let sha = fixture(dir.path(), WITH_READER);
    let task = dir.path().join("aplan/tasks/W-2.md");
    std::fs::write(
        &task,
        "---\nid: W-2\ntitle: ordinary work\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
    )
    .expect("task");
    let out = common::run(
        dir.path(),
        &["close", "W-2", "--evidence", &format!("commit:{sha}")],
    );
    assert_eq!(out.code, 0, "{}", out.text());
    assert!(
        !out.stderr.contains("version it produced"),
        "a task is not a release: {}",
        out.stderr
    );
}

/// A release already recording a different number stops the closure, naming
/// both: one of the two is wrong and which is not this tool's to decide.
#[test]
fn a_release_recording_a_different_number_is_refused_with_both_named() {
    let dir = tmp();
    let sha = fixture(dir.path(), WITH_READER);
    let release = dir.path().join("aplan/tasks/M-r.md");
    let text = std::fs::read_to_string(&release).expect("read");
    std::fs::write(
        &release,
        text.replace("status: todo", "status: todo\nversion: 1.5.0"),
    )
    .expect("write");
    let out = common::run(
        dir.path(),
        &["close", "M-r", "--evidence", &format!("commit:{sha}")],
    );
    assert_ne!(out.code, 0, "the closure went through: {}", out.text());
    let said = out.text();
    assert!(said.contains("1.5.0") && said.contains("2.0.0"), "{said}");
    assert_eq!(
        recorded(dir.path()).as_deref(),
        Some("1.5.0"),
        "a refused closure writes nothing"
    );
}

/// Proves `close.refuses_empty_version_fields` through the public command.
/// Presence belongs to the source mapping rather than to `Option<String>`: an
/// authored null cannot be mistaken for permission to append another key.
#[test]
fn empty_version_fields_are_refused_without_writing() {
    for (name, field) in [
        ("bare", "version:\n"),
        ("null", "version: null\n"),
        ("tilde", "version: ~\n"),
        ("single quoted key", "'version': null\n"),
        ("double quoted key", "\"version\": null\n"),
        ("escaped key", "\"vers\\u0069on\": null\n"),
        ("explicit key", "? version\n: null\n"),
        ("tagged key", "!!str version: null\n"),
        ("anchored key", "&version_key version: null\n"),
        (
            "aliased key",
            "owner: &version_key version\n*version_key: null\n",
        ),
    ] {
        let dir = tmp();
        fixture(dir.path(), WITH_READER);
        let release = dir.path().join("aplan/tasks/M-r.md");
        let source = std::fs::read_to_string(&release).expect("release");
        let source = source.replace("status: todo\n", &format!("status: todo\n{field}"));
        std::fs::write(&release, &source).expect("empty version field");

        let out = common::run(
            dir.path(),
            &[
                "close",
                "M-r",
                "--evidence",
                "file:.aplan.yml",
                "--no-resolve",
            ],
        );
        assert_eq!(out.code, 2, "{name} reached the wrong exit: {out:?}");
        assert!(
            out.text().contains("no value")
                && out.text().contains("2.0.0")
                && out.text().contains("remove the empty field"),
            "{name} was not actionable: {out:?}"
        );
        assert!(!out.text().contains("bug in aplan"), "{name}: {out:?}");
        assert_eq!(
            std::fs::read_to_string(&release).expect("release after refusal"),
            source,
            "{name} changed the release before refusing"
        );
    }
}

#[test]
fn agreeing_version_records_keep_their_exact_source_on_close() {
    for (name, field) in [
        ("ordinary", "version : 2.0.0 # authored record\n"),
        ("quoted key", "\"version\": 2.0.0 # authored record\n"),
    ] {
        let dir = tmp();
        fixture(dir.path(), WITH_READER);
        let release = dir.path().join("aplan/tasks/M-r.md");
        let source = std::fs::read_to_string(&release).expect("release");
        let source = source.replace("status: todo\n", &format!("status: todo\n{field}"));
        std::fs::write(&release, source).expect("agreeing version record");

        let out = common::run(
            dir.path(),
            &[
                "close",
                "M-r",
                "--evidence",
                "file:.aplan.yml",
                "--no-resolve",
            ],
        );
        assert_eq!(out.code, 0, "{name} did not close: {out:?}");
        let updated = std::fs::read_to_string(&release).expect("closed release");
        assert!(updated.contains(field), "{name} record moved: {updated}");
        let (frontmatter, _) = aplan_core::load::split_frontmatter(&updated).expect("frontmatter");
        let task: aplan_core::Task = yaml_compat::from_str(frontmatter).expect("one valid record");
        assert_eq!(task.version.as_deref(), Some("2.0.0"), "{name}");
        assert_eq!(task.status, aplan_core::Status::Done, "{name}");
    }
}

#[test]
fn an_empty_version_field_does_not_block_close_without_a_derivation() {
    let dir = tmp();
    let sha = fixture(dir.path(), "root: aplan\n");
    let release = dir.path().join("aplan/tasks/M-r.md");
    let source = std::fs::read_to_string(&release).expect("release");
    let source = source.replace("status: todo\n", "status: todo\nversion: null\n");
    std::fs::write(&release, source).expect("empty version record");

    let out = common::run(
        dir.path(),
        &["close", "M-r", "--evidence", &format!("commit:{sha}")],
    );
    assert_eq!(
        out.code, 0,
        "an optional record became a closure gate: {out:?}"
    );
    let updated = std::fs::read_to_string(&release).expect("closed release");
    assert!(
        updated.contains("status: done\nversion: null\n"),
        "{updated}"
    );
    assert_eq!(updated.matches("version: null").count(), 1, "{updated}");
}

/// **A window that declined to be measured records nothing** (`A1-339`, audit).
///
/// Two open releases: `aplan version` refuses -- "which one ships next is its
/// question rather than this tool's" -- and the closure recorded a number
/// anyway, derived from that same refused window. A record the tool will not
/// stand behind when asked directly is worse than no record, because the whole
/// point of the field is that a later run trusts it.
#[test]
fn an_ambiguous_plan_records_nothing_and_says_why() {
    let dir = tmp();
    let sha = fixture(dir.path(), WITH_READER);
    std::fs::write(
        dir.path().join("aplan/tasks/M-r2.md"),
        "---\nid: M-r2\ntitle: a second release\nphase: 1\nstatus: todo\n\
         tags:\n  - milestone\n  - release\nblocked_by:\n  - W-1\n---\n\nbody\n",
    )
    .expect("release");
    let out = common::run(
        dir.path(),
        &["close", "M-r", "--evidence", &format!("commit:{sha}")],
    );
    assert_eq!(out.code, 0, "the closure must go through: {}", out.text());
    assert_eq!(
        recorded(dir.path()),
        None,
        "a refused window recorded a number"
    );
    assert!(
        out.stderr.contains("open releases"),
        "it has to say which refusal it was: {}",
        out.stderr
    );
}

/// The same for a checkout that cannot answer the window at all: the command
/// refuses, so the closure must not write down what it would have said.
#[test]
fn a_shallow_clone_records_nothing_and_says_why() {
    let dir = tmp();
    fixture(dir.path(), WITH_READER);
    // A citation that resolves without history, so the closure reaches the
    // derivation rather than stopping at the evidence gate.
    let release = dir.path().join("aplan/tasks/M-r.md");
    let text = std::fs::read_to_string(&release).expect("read");
    std::fs::write(
        &release,
        text.replace("blocked_by:", "paths:\n- version.sh\nblocked_by:"),
    )
    .expect("write");
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    std::fs::write(
        dir.path().join(".git/shallow"),
        String::from_utf8_lossy(&head.stdout).trim(),
    )
    .expect("shallow");

    let out = common::run(
        dir.path(),
        &["close", "M-r", "--evidence", "file:version.sh"],
    );
    assert_eq!(out.code, 0, "{}", out.text());
    assert_eq!(recorded(dir.path()), None);
    assert!(
        out.stderr.contains("shallow clone"),
        "it has to say which refusal it was: {}",
        out.stderr
    );
}

/// **A record this tool cannot read back is not handed on** (`A1-339`, audit).
///
/// `APLAN050` refuses it in `check`, and printing it anyway would put it
/// straight into a `cargo release "$(aplan version <release>)"` that `check` was
/// about to refuse.
#[test]
fn an_unreadable_record_is_refused_rather_than_printed() {
    let dir = tmp();
    fixture(dir.path(), WITH_READER);
    let release = dir.path().join("aplan/tasks/M-r.md");
    let text = std::fs::read_to_string(&release).expect("read");
    std::fs::write(
        &release,
        text.replace("status: todo", "status: todo\nversion: next"),
    )
    .expect("write");
    let out = common::run(dir.path(), &["version", "M-r"]);
    assert_ne!(out.code, 0, "it handed on `next`: {}", out.text());
    assert!(
        out.stdout.is_empty(),
        "stdout must stay clean: {}",
        out.stdout
    );
    assert!(
        out.stderr.contains("read back") && out.stderr.contains("major.minor.patch"),
        "{}",
        out.stderr
    );
}

/// A version the arithmetic cannot read is reported, and the release still
/// closes (ADR-0034's refusal, one layer along).
#[test]
fn a_scheme_the_arithmetic_cannot_read_closes_without_a_number() {
    let dir = tmp();
    let sha = fixture(dir.path(), WITH_READER);
    std::fs::write(dir.path().join("version.sh"), "echo 2026.09.1\n").expect("calver");
    let out = common::run(
        dir.path(),
        &["close", "M-r", "--evidence", &format!("commit:{sha}")],
    );
    assert_eq!(out.code, 0, "{}", out.text());
    assert_eq!(recorded(dir.path()), None);
    assert!(
        out.stderr.contains("arithmetic"),
        "it must say which of the reasons it was: {}",
        out.stderr
    );
}

/// A declared command that will not run is reported, and the release still
/// closes: the field is a record and a broken key is not a reason to refuse a
/// finished release.
#[test]
fn a_reader_that_cannot_run_closes_without_a_number() {
    let dir = tmp();
    let sha = fixture(
        dir.path(),
        "root: aplan\nversion:\n  current:\n    command: [no-such-reader-4f2a]\n",
    );
    let out = common::run(
        dir.path(),
        &["close", "M-r", "--evidence", &format!("commit:{sha}")],
    );
    assert_eq!(out.code, 0, "{}", out.text());
    assert_eq!(recorded(dir.path()), None);
    assert!(
        out.stderr.contains("version.current"),
        "it must name the key: {}",
        out.stderr
    );
}
