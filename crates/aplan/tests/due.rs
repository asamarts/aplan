//! `aplan due`, the hand-off a parked task can give a scheduler (`A1-264`).
//!
//! `p00013`'s `DN0-29` waits on GitHub creating a real weekday `schedule`
//! event. The earliest useful revisit is computable from a committed cron and a
//! measured delay, and it lived in prose: a human had to copy it into another
//! system, and a missed copy leaves correctly parked work invisible forever.
//!
//! **Reaching the instant never means the wait is over.** The event may still be
//! absent. Everything here holds that line: no status changes, `check` and
//! `ready` read no clock, and the report says what it is in as many words.

use std::path::Path;

use crate::common;
use common::aplan;

/// Two parked tasks, one revisit in the past and one ahead of it.
fn plan() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    let task = |id: &str, at: &str, title: &str| {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: {title}\nphase: 1\nstatus: blocked\n\
                 owner: kaminsod\neffort_days: 1\nrevisit_at: {at}\n---\n\n\
                 ## Why\n\nWaits on something outside the plan.\n"
            ),
        )
        .expect("task");
    };
    task(
        "DN0-29",
        "2026-09-10T05:00:00Z",
        "Retire the persistent runner",
    );
    task("DN0-30", "2027-01-01T00:00:00Z", "Something later");
    dir
}

fn due(dir: &Path, args: &[&str]) -> (String, i32) {
    let mut invocation = vec!["due"];
    invocation.extend_from_slice(args);
    aplan(dir, &invocation)
}

/// The measured case: before the instant nothing is due, after it one is.
#[test]
fn a_revisit_becomes_due_when_the_instant_passes_and_not_before() {
    let dir = plan();

    let (before, code) = due(dir.path(), &["--as-of", "2026-09-10T04:59:59Z"]);
    assert_eq!(code, 0, "{before}");
    assert!(!before.contains("DN0-29"), "due a second early:\n{before}");
    assert!(before.contains("no parked task is due"), "{before}");
    // The denominator, so an empty answer is not read as "nothing is parked".
    assert!(before.contains("2 carry a revisit"), "{before}");

    let (after, code) = due(dir.path(), &["--as-of", "2026-09-10T05:00:00Z"]);
    assert_eq!(code, 0, "{after}");
    assert!(
        after.contains("DN0-29"),
        "not due at its own instant:\n{after}"
    );
    assert!(!after.contains("DN0-30"), "{after}");
    assert!(after.contains("1 more parked task(s)"), "{after}");
}

/// **The sentence the whole feature turns on.**
///
/// A scheduler that read this as "the wait is over" would unpark work on a
/// clock, which is the one thing the field must never be able to say.
#[test]
fn the_report_says_it_is_due_for_a_look_and_not_unblocked() {
    let dir = plan();
    let (out, _) = due(dir.path(), &["--as-of", "2026-09-10T06:00:00Z"]);
    assert!(
        out.contains("due for a look, not unblocked"),
        "the caveat is the feature: {out}"
    );
    assert!(out.contains("Nothing here changes a status"), "{out}");

    // And it did not: the task is still parked, and `ready` still will not
    // offer it.
    let (ready, _) = aplan(dir.path(), &["ready"]);
    assert!(
        !ready.contains("DN0-29"),
        "a clock offered parked work: {ready}"
    );
    let text = std::fs::read_to_string(dir.path().join("aplan/tasks/DN0-29.md")).expect("read");
    assert!(text.contains("status: blocked"), "the status moved: {text}");
}

/// Which clock, always: a reader told a task is due has to know due by when.
#[test]
fn the_report_names_the_clock_it_used() {
    let dir = plan();
    let (asked, _) = due(dir.path(), &["--as-of", "2026-09-10T06:00:00Z"]);
    assert!(
        asked.contains("Answered as of 2026-09-10T06:00:00Z"),
        "{asked}"
    );

    let (default, _) = due(dir.path(), &[]);
    assert!(
        default.contains("Answered against this machine's clock"),
        "an answer nobody can reproduce, and it did not say so: {default}"
    );
    assert!(default.contains("--as-of"), "{default}");
}

/// An `--as-of` the tool cannot parse is refused rather than guessed at.
#[test]
fn an_as_of_that_is_not_an_instant_is_refused() {
    let dir = plan();
    for bad in ["2026-09-10", "next tuesday", "2026-09-10T05:00:00"] {
        let (out, code) = due(dir.path(), &["--as-of", bad]);
        assert_eq!(code, 2, "{bad}: {out}");
        assert!(out.contains("not an RFC 3339 instant"), "{bad}: {out}");
    }
}

/// A task that is not parked is never due, whatever it carries.
///
/// `APLAN042` reports the field there; acting on it here would make the command
/// disagree with the rule.
#[test]
fn a_revisit_on_an_unparked_task_is_never_due() {
    let dir = plan();
    let path = dir.path().join("aplan/tasks/DN0-29.md");
    let text = std::fs::read_to_string(&path).expect("read");
    std::fs::write(&path, text.replace("status: blocked", "status: todo")).expect("write");

    let (out, _) = due(dir.path(), &["--as-of", "2026-09-10T06:00:00Z"]);
    assert!(
        !out.contains("DN0-29"),
        "a `todo` task was called due: {out}"
    );

    // And the rule says why, rather than the command silently ignoring it.
    let (check, _) = aplan(dir.path(), &["check"]);
    assert!(check.contains("APLAN042"), "{check}");
}

/// `check` and `ready` read no clock, which is what keeps this out of the
/// scheduler business: their answers are the same at any instant.
#[test]
fn the_ordinary_commands_do_not_depend_on_the_time() {
    let dir = plan();
    let (check, _) = aplan(dir.path(), &["check"]);
    assert!(check.contains("ok: 2 tasks"), "{check}");
    assert!(
        !check.contains("APLAN042"),
        "a usable revisit was reported: {check}"
    );
    let (ready, _) = aplan(dir.path(), &["ready"]);
    assert!(
        !ready.contains("DN0-29") && !ready.contains("DN0-30"),
        "{ready}"
    );
}

/// The JSON a scheduler reads, and the field on the task surface it could read
/// instead.
#[test]
fn both_machine_surfaces_carry_the_revisit() {
    let dir = plan();
    let (out, _) = due(
        dir.path(),
        &["--as-of", "2026-09-10T06:00:00Z", "--format", "json"],
    );
    let v: serde_json::Value = serde_json::from_str(&out).expect("json");
    assert_eq!(v["schema"], "aplan/due/v1");
    assert_eq!(v["as_of"], "2026-09-10T06:00:00Z");
    assert_eq!(v["waiting"], 1);
    let first = &v["tasks"][0];
    assert_eq!(first["id"], "DN0-29");
    assert_eq!(first["revisit_at"], "2026-09-10T05:00:00Z");
    assert_eq!(first["overdue_seconds"], 3600);

    // And `tasks` carries it on every task, `null` where there is none, as that
    // document's every other key does.
    let (tasks, _) = aplan(dir.path(), &["tasks", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&tasks).expect("json");
    let rows = v["tasks"].as_array().expect("tasks");
    assert!(
        rows.iter().all(|t| t.get("revisit_at").is_some()),
        "a key that comes and goes hands a consumer `undefined`: {tasks}"
    );
}

/// An offset is read, not ignored: `07:00+02:00` is 05:00 UTC, so it is due at
/// the same instant the `Z` form is.
#[test]
fn an_offset_names_the_same_instant_as_its_utc_spelling() {
    let dir = plan();
    let path = dir.path().join("aplan/tasks/DN0-29.md");
    let text = std::fs::read_to_string(&path).expect("read");
    std::fs::write(
        &path,
        text.replace("2026-09-10T05:00:00Z", "2026-09-10T07:00:00+02:00"),
    )
    .expect("write");

    let (early, _) = due(dir.path(), &["--as-of", "2026-09-10T04:59:59Z"]);
    assert!(
        !early.contains("DN0-29"),
        "the offset was ignored:\n{early}"
    );
    let (on_time, _) = due(dir.path(), &["--as-of", "2026-09-10T05:00:00Z"]);
    assert!(on_time.contains("DN0-29"), "{on_time}");
}

/// The four things this task's own audit found (`A1-264`).
mod audit {
    use super::*;

    /// **A validator must not crash on the plan it was asked to judge.**
    ///
    /// The parser slices by byte offset, and a multi-byte character straddling
    /// one of them panicked: `aplan check` exited 101 on a task file, which is
    /// the worst answer a tool like this can give.
    #[test]
    fn a_revisit_with_a_multi_byte_character_is_refused_not_a_crash() {
        let dir = plan();
        std::fs::write(
            dir.path().join("aplan/tasks/DN0-29.md"),
            "---\nid: DN0-29\ntitle: Non-ASCII in the instant\nphase: 1\nstatus: blocked\n\
             effort_days: 1\nrevisit_at: \"2026-09-10T05:00:0\u{e9}\"\n---\n\n## Why\n\nx\n",
        )
        .expect("task");

        let (out, code) = aplan(dir.path(), &["check"]);
        assert_ne!(code, 101, "the validator crashed on a task file:\n{out}");
        assert!(out.contains("APLAN042"), "{out}");

        let (out, code) = due(dir.path(), &["--as-of", "2026-09-10T06:00:00Z"]);
        assert_ne!(code, 101, "the command crashed:\n{out}");
    }

    /// `show` is what a reader opens to see one task whole, so a field that is
    /// on the task and not in the answer is one they have to open the file for.
    #[test]
    fn show_carries_the_revisit_on_both_surfaces() {
        let dir = plan();
        let (human, _) = aplan(dir.path(), &["show", "DN0-29", "--no-resolve"]);
        assert!(human.contains("revisit at 2026-09-10T05:00:00Z"), "{human}");
        // Said as what it is: an instant alone reads as a deadline, and it is
        // the opposite.
        assert!(human.contains("not due"), "{human}");

        let (json, _) = aplan(
            dir.path(),
            &["show", "DN0-29", "--no-resolve", "--format", "json"],
        );
        let v: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(v["revisit_at"], "2026-09-10T05:00:00Z");
    }

    /// **`why` was saying something this task made false.**
    ///
    /// *No field carries it* was `A1-175`'s sentence and was true when written.
    /// A task that names a revisit now has a field carrying *when to look
    /// again*, and a reader told otherwise would not go looking for it.
    #[test]
    fn why_names_the_revisit_and_keeps_the_old_sentence_without_one() {
        let dir = plan();
        let (with, _) = aplan(dir.path(), &["why", "DN0-29"]);
        assert!(with.contains("2026-09-10T05:00:00Z"), "{with}");
        assert!(
            !with.contains("no field carries it"),
            "said something this task made false:\n{with}"
        );
        // And it is still not a claim that the wait is over.
        assert!(
            with.contains("rather than a claim the wait is over"),
            "{with}"
        );

        let path = dir.path().join("aplan/tasks/DN0-29.md");
        let text = std::fs::read_to_string(&path).expect("read");
        let kept: Vec<&str> = text
            .lines()
            .filter(|l| !l.starts_with("revisit_at:"))
            .collect();
        std::fs::write(&path, kept.join("\n")).expect("write");

        let (without, _) = aplan(dir.path(), &["why", "DN0-29"]);
        assert!(without.contains("no field carries it"), "{without}");
    }

    /// The agent surface says the same thing as the human one, or the two
    /// disagree about one task.
    #[test]
    fn the_agent_surface_carries_the_revisit_too() {
        let dir = plan();
        let (json, _) = aplan(dir.path(), &["why", "DN0-29", "--format", "json"]);
        let v: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(v["parked"], true);
        assert_eq!(v["revisit_at"], "2026-09-10T05:00:00Z");
    }

    /// Every command that rewrites a task file keeps the field.
    ///
    /// `add --from` writes it and five other commands rewrite the file; a
    /// serialiser that dropped it would lose a hand-off silently.
    #[test]
    fn the_field_survives_the_commands_that_rewrite_the_file() {
        let dir = plan();
        for argv in [vec!["mv", "DN0-29", "--phase", "2"], vec!["drop", "DN0-29"]] {
            let (out, code) = aplan(dir.path(), &argv);
            assert_eq!(code, 0, "{argv:?}: {out}");
            let text =
                std::fs::read_to_string(dir.path().join("aplan/tasks/DN0-29.md")).expect("read");
            assert!(
                text.contains("revisit_at: 2026-09-10T05:00:00Z"),
                "{argv:?} dropped the field:\n{text}"
            );
        }
    }
}
