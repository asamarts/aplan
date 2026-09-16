//! `aplan stale --as-of`, the clock said out loud (`A1-162`).
//!
//! Ages are measured from `HEAD`'s own commit time, so two runs at one commit
//! agree. That is right for a gate and wrong for a repository that has stopped
//! moving: every age freezes with the last commit, and a plan abandoned six
//! months ago reports nothing as idle. Naming a date is the explicit way to ask
//! "how does this look now" without putting a wall clock inside a tool whose
//! answers are otherwise a function of the commit.

use std::path::Path;
use std::process::Command;

mod common;
use common::aplan;

/// The case the pilot found: a repository whose history stopped, where the
/// default clock cannot see the age at all.
#[test]
fn a_repository_that_stopped_moving_reports_nothing_until_a_date_is_named() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), "2026-06-01T12:00:00");

    let (default, code) = aplan(dir.path(), &["stale", "--older-than", "7"]);
    assert_eq!(code, 0, "{default}");
    assert!(
        default.contains("nothing live has sat longer than 7 day(s)"),
        "every age froze with the last commit: {default}"
    );

    let (dated, code) = aplan(
        dir.path(),
        &["stale", "--older-than", "7", "--as-of", "2026-08-19"],
    );
    assert_eq!(code, 0, "a measurement is not a verdict: {dated}");
    assert!(dated.contains("T-1"), "{dated}");
    assert!(dated.contains("79d") || dated.contains("78d"), "{dated}");
}

/// The report says which clock it used, always. "Idle 42 days" is unreadable
/// without knowing what it is 42 days before, and the default is the surprising
/// one.
#[test]
fn the_report_names_the_clock_it_measured_from() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), "2026-06-01T12:00:00");

    let (default, _) = aplan(dir.path(), &["stale"]);
    assert!(default.contains("`HEAD`'s commit time"), "{default}");
    assert!(default.contains("--as-of"), "and the way out: {default}");

    let (dated, _) = aplan(dir.path(), &["stale", "--as-of", "2026-08-19"]);
    assert!(dated.contains("Ages measured from 2026-08-19"), "{dated}");
    assert!(
        !dated.contains("`HEAD`'s commit time"),
        "one clock, named once: {dated}"
    );

    let (json, _) = aplan(dir.path(), &["stale", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert!(
        doc["as_of"].is_null(),
        "null is `HEAD`, which is the default: {json}"
    );
    let (json, _) = aplan(
        dir.path(),
        &["stale", "--as-of", "2026-08-19", "--format", "json"],
    );
    let doc: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(doc["as_of"], "2026-08-19", "{json}");
}

/// Two runs at one commit agree, which is the property `--as-of` must not cost:
/// the same date is the same report, and that is what a wall clock inside the
/// tool could not have promised.
#[test]
fn the_same_date_at_one_commit_is_the_same_report() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), "2026-06-01T12:00:00");

    let (first, _) = aplan(dir.path(), &["stale", "--as-of", "2026-08-19"]);
    let (second, _) = aplan(dir.path(), &["stale", "--as-of", "2026-08-19"]);
    assert_eq!(first, second);

    // And a different date is a different question, visibly.
    let (later, _) = aplan(dir.path(), &["stale", "--as-of", "2026-09-19"]);
    assert_ne!(first, later);
}

/// A date before the last commit would make every age negative, and the report
/// would then say nothing has sat -- the false green this command's own audit
/// was about.
#[test]
fn a_date_before_the_last_commit_is_refused() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), "2026-06-01T12:00:00");

    let (out, code) = aplan(dir.path(), &["stale", "--as-of", "2026-01-01"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("before `HEAD`'s own commit"), "{out}");
    assert!(
        out.contains("not yet written"),
        "and what it would have reported: {out}"
    );
}

#[test]
fn a_date_that_is_not_a_date_is_refused_as_one() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), "2026-06-01T12:00:00");

    for bad in [
        "last tuesday",
        "2026-13-01",
        "2026-8-19",
        "2026-08-19T00:00:00Z",
    ] {
        let (out, code) = aplan(dir.path(), &["stale", "--as-of", bad]);
        assert_ne!(code, 0, "{bad}: {out}");
        assert!(out.contains("is not an ISO-8601 date"), "{bad}: {out}");
    }
}

/// `--as-of $(date +%F)` in a weekly review is the flag's whole point, and
/// measuring from midnight refused it on any day the repository saw a commit --
/// which is most of them. A date names a whole day.
#[test]
fn today_works_on_a_day_that_already_has_a_commit() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), "2026-08-19T18:00:00+0000");

    let (out, code) = aplan(dir.path(), &["stale", "--as-of", "2026-08-19"]);
    assert_eq!(code, 0, "the commit is earlier the same day: {out}");
    assert!(out.contains("Ages measured from 2026-08-19"), "{out}");

    // The day before that commit is still refused: it is genuinely before.
    let (out, code) = aplan(dir.path(), &["stale", "--as-of", "2026-08-18"]);
    assert_ne!(code, 0, "{out}");

    // And the age counts whole days from the commit, so a task committed on the
    // 19th has sat eight days by the 27th rather than seven.
    let (out, _) = aplan(
        dir.path(),
        &["stale", "--older-than", "3", "--as-of", "2026-08-27"],
    );
    assert!(out.contains("8d"), "{out}");
}

/// One live task, committed at the date given, and nothing since.
fn fixture(dir: &Path, when: &str) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    git(dir, &["init", "-q"], None);
    git(dir, &["config", "user.email", "t@example.com"], None);
    git(dir, &["config", "user.name", "Test"], None);
    std::fs::write(
        dir.join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: An old task\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
    )
    .expect("task");
    git(dir, &["add", "-A"], None);
    git(dir, &["commit", "-q", "-m", "one"], Some(when));
}

fn git(dir: &Path, args: &[&str], when: Option<&str>) {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(dir);
    if let Some(when) = when {
        cmd.env("GIT_AUTHOR_DATE", when)
            .env("GIT_COMMITTER_DATE", when);
    }
    let out = cmd
        .env(
            "GIT_CEILING_DIRECTORIES",
            aplan_core::fixture::fixture_root(),
        )
        .output()
        .expect("running git");
    // Checked, because `expect` proves only that `git` started: a setup that
    // carries on from a failed `init` runs the rest against whatever repository
    // it can find, which is the checkout the fixture sits in (`A1-257`).
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
