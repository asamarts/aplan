//! `aplan stale`, the question a tracker cannot ask (`A1-020`).
//!
//! FIELD-RESEARCH 7.3 found "how long has this sat unchanged" the highest-signal
//! drift metric in the wild, and these pin the three things that make it honest
//! here: it is advice and never fails a build, it leads with the task whose
//! blockers cleared long ago, and it stays out of the hermetic `check`.
//!
//! Dates are made by committing with `GIT_COMMITTER_DATE`, so the history is a
//! fixture rather than a wait.

use std::path::Path;
use std::process::Command;

mod common;
use common::aplan;

#[test]
fn a_live_task_that_has_sat_is_reported_and_the_exit_code_stays_zero() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (out, code) = aplan(dir.path(), &["stale", "--older-than", "10"]);
    assert_eq!(code, 0, "advice never fails a build: {out}");
    assert!(out.contains("A-OLD"), "the task that sat: {out}");
    assert!(
        !out.contains("A-NEW"),
        "and not the one that did not: {out}"
    );
    assert!(
        out.contains("advice, not a verdict"),
        "the report says what it is: {out}"
    );

    // A window nothing exceeds is an answer, not an empty page.
    let (out, code) = aplan(dir.path(), &["stale", "--older-than", "900"]);
    assert_eq!(code, 0);
    assert!(out.contains("nothing live has sat"), "{out}");
}

/// The least-proxy case: every blocker is done, so nothing but a decision is
/// stopping it, and it has sat anyway.
#[test]
fn a_task_whose_blockers_cleared_long_ago_is_called_out() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (out, code) = aplan(
        dir.path(),
        &["stale", "--older-than", "10", "--format", "json"],
    );
    assert_eq!(code, 0, "{out}");
    let doc: serde_json::Value = serde_json::from_str(&out).expect("json");
    let tasks = doc["tasks"].as_array().expect("tasks");

    let old = tasks
        .iter()
        .find(|t| t["id"] == "A-OLD")
        .expect("the idle task");
    assert_eq!(old["ready"], true, "its blocker is done: {out}");
    assert!(old["idle_days"].as_i64().expect("days") >= 30, "{out}");

    // A task still waiting on unfinished work is idle for a reason, and the
    // report says so by not calling it ready.
    let waiting = tasks
        .iter()
        .find(|t| t["id"] == "A-WAIT")
        .expect("the blocked task");
    assert_eq!(waiting["ready"], false, "{out}");

    // Readiest first, then idlest: what a reader can pick up now leads the
    // table, and among those the longest-neglected.
    assert_eq!(tasks[0]["id"], "A-OLD", "{out}");
    assert_eq!(tasks[1]["id"], "A-BLOCK", "ready, but idle for less: {out}");
    assert_eq!(
        tasks.last().expect("a row")["id"],
        "A-WAIT",
        "not ready, so last whatever its age: {out}"
    );

    // Milestones are checkpoints, not neglected work.
    assert!(
        !tasks.iter().any(|t| t["id"] == "M-s1"),
        "a cut sits by design: {out}"
    );
}

/// `in_progress` is live too, and a task that has been "in progress" for months
/// is the report's best customer (`A1-020`, its audit: the acceptance criterion
/// names both statuses and only one had a test).
#[test]
fn a_task_in_progress_counts_as_live() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    write(
        dir.path(),
        "A-DOING",
        "in_progress",
        "Started, then nothing",
        None,
        "",
    );
    commit(dir.path(), "started long ago", "2026-06-15T12:00:00");
    // Move HEAD on without touching it.
    write(dir.path(), "A-OTHER", "todo", "Something else", None, "");
    commit(dir.path(), "later work", "2026-08-18T12:00:00");

    let (out, code) = aplan(
        dir.path(),
        &["stale", "--older-than", "10", "--format", "json"],
    );
    assert_eq!(code, 0, "{out}");
    let doc: serde_json::Value = serde_json::from_str(&out).expect("json");
    let doing = doc["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|t| t["id"] == "A-DOING")
        .expect("an in_progress task is live");
    assert_eq!(doing["status"], "in_progress", "{out}");
    assert!(doing["idle_days"].as_i64().expect("days") >= 60, "{out}");
}

/// A report that can see nothing must not read as a clean bill of health
/// (`A1-020`, found by its audit: pointed at an unrelated repository it said
/// "nothing has sat", which is the false green this project exists to refuse).
#[test]
fn a_repository_that_holds_none_of_the_plan_says_so() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let elsewhere = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    git(elsewhere.path(), &["init", "-q"]);
    git(elsewhere.path(), &["config", "user.email", "t@example.com"]);
    git(elsewhere.path(), &["config", "user.name", "Test"]);
    std::fs::write(
        elsewhere.path().join("f.txt"),
        "x
",
    )
    .expect("write");
    commit(elsewhere.path(), "unrelated", "2026-08-18T12:00:00");

    let repo = elsewhere.path().display().to_string();
    let (out, code) = aplan(dir.path(), &["stale", "--repo", &repo, "--older-than", "0"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("no live task has any history in this repository"),
        "it must not report a clean bill of health: {out}"
    );
    assert!(
        !out.contains("nothing live has sat"),
        "and must not say the reassuring thing: {out}"
    );

    // The machine surface can tell the two apart on the counts alone.
    let (json, _) = aplan(
        dir.path(),
        &[
            "stale",
            "--repo",
            &repo,
            "--older-than",
            "0",
            "--format",
            "json",
        ],
    );
    let doc: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(doc["fresh"], 0, "{json}");
    assert!(doc["untracked"].as_u64().expect("count") >= 4, "{json}");
}

/// A task file git has never seen has no history to have sat idle in, and
/// inventing a date for it would be the report making something up.
#[test]
fn an_untracked_task_is_counted_rather_than_dated() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/A-NEVER.md"),
        "---\nid: A-NEVER\ntitle: Never committed\nphase: 1\nstatus: todo\neffort_days: 1\n\
         ---\n\nbody\n",
    )
    .expect("task");

    let (out, code) = aplan(
        dir.path(),
        &["stale", "--older-than", "10", "--format", "json"],
    );
    assert_eq!(code, 0, "{out}");
    let doc: serde_json::Value = serde_json::from_str(&out).expect("json");
    assert_eq!(doc["untracked"], 1, "{out}");
    assert!(
        !doc["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .any(|t| t["id"] == "A-NEVER"),
        "no invented date: {out}"
    );
}

/// `check` stays hermetic: it is the fast path that runs on every keystroke, and
/// it may not open a repository. The structural proof lives in `aplan-e2e`
/// (`git2` is forbidden in the crates `check` runs through); this is the
/// behavioural half -- `check` works where there is no git at all, and `stale`
/// says why it cannot.
#[test]
fn check_needs_no_repository_and_stale_says_so_when_there_is_none() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::remove_dir_all(dir.path().join(".git")).expect("rm .git");

    let (out, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "check must not need git: {out}");
    assert!(out.contains("DAG acyclic"), "{out}");

    let (out, code) = aplan(dir.path(), &["stale"]);
    assert_ne!(code, 0, "stale reads history and has none: {out}");
    assert!(
        out.to_lowercase().contains("repository") || out.to_lowercase().contains("git"),
        "and says which: {out}"
    );
}

/// A repository whose history the plan predates: `stale_after_days` is the
/// window when no flag is given.
#[test]
fn the_window_comes_from_the_config_when_no_flag_is_given() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\nstale_after_days: 5\n",
    )
    .expect("config");

    let (out, code) = aplan(dir.path(), &["stale"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("A-OLD"), "the 5-day window catches it: {out}");

    // And the flag still wins.
    let (out, _) = aplan(dir.path(), &["stale", "--older-than", "900"]);
    assert!(
        out.contains("nothing live has sat longer than 900"),
        "{out}"
    );
}

/// A plan, a history, and three shapes: a task that sat with its blocker done,
/// one that sat while still blocked, and one touched at `HEAD`.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "t@example.com"]);
    git(dir, &["config", "user.name", "Test"]);

    write(
        dir,
        "A-DONE",
        "done",
        "Finished long ago",
        None,
        "evidence:\n  - kind: note\n    ref: done long ago\n    captured: 2026-06-01\n",
    );
    write(dir, "A-OLD", "todo", "Could start, has not", None, "");
    write(dir, "A-WAIT", "todo", "Still waiting", Some("A-BLOCK"), "");
    std::fs::write(
        dir.join("aplan/tasks/M-s1.md"),
        "---\nid: M-s1\ntitle: A cut\nphase: 1\nstatus: todo\ntags: [milestone, s1]\n\
         blocked_by: [A-OLD, A-WAIT, A-BLOCK, A-DONE]\n---\n\nbody\n",
    )
    .expect("milestone");
    commit(dir, "the plan, as it stood", "2026-06-01T12:00:00");

    // The blocker arrives later, so `A-OLD` is the idlest ready task and the
    // ordering the report claims -- readiest, then idlest -- is observable.
    write(dir, "A-BLOCK", "todo", "The blocker", None, "");
    commit(dir, "the blocker", "2026-07-01T12:00:00");

    write(dir, "A-NEW", "todo", "Touched at HEAD", None, "");
    commit(dir, "a task touched at HEAD", "2026-08-18T12:00:00");
}

/// The same plan with one task parked long ago.
///
/// Its own builder rather than an edit to [`fixture`], because the parked task
/// has to be written *before* the last commit to have an age at all -- and
/// because every other test in this file counts the tasks that fixture holds.
fn parked_fixture(dir: &Path) {
    fixture(dir);
    // Rewind: write the parked task, then re-commit the whole plan at a date
    // after it but before `HEAD`, so the park has held for something measurable.
    write(dir, "A-PARK", "blocked", "Waiting on a vendor", None, "");
    commit(dir, "park a task", "2026-06-15T12:00:00");
    write(dir, "A-NEW", "todo", "Touched at HEAD, again", None, "");
    commit(dir, "move HEAD forward", "2026-08-18T12:00:00");
}

fn write(dir: &Path, id: &str, status: &str, title: &str, blocker: Option<&str>, extra: &str) {
    let edge = blocker.map_or(String::new(), |b| format!("blocked_by: [{b}]\n"));
    std::fs::write(
        dir.join(format!("aplan/tasks/{id}.md")),
        format!(
            "---\nid: {id}\ntitle: {title}\nphase: 1\nstatus: {status}\neffort_days: 1\n\
             {edge}{extra}---\n\nbody\n"
        ),
    )
    .expect("task");
}

fn commit(dir: &Path, message: &str, date: &str) {
    git(dir, &["add", "-A"]);
    let out = Command::new("git")
        .args(["commit", "-q", "-m", message])
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .current_dir(dir)
        .output()
        .expect("running git");
    assert!(
        out.status.success(),
        "commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("running git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// **A parked task is live work, and nothing was tracking it** (`A1-275`).
///
/// `Status::Blocked` is *explicitly parked for a reason outside the dependency
/// graph*. `ready` excludes it, correctly -- it is not startable -- and `stale`
/// excluded it too, so between them the only work nothing tracked was the work
/// nobody was doing. `p00002` hit this and rightly did nothing, because the
/// alternative was reimplementing an age calculation against a tag convention.
#[test]
fn a_parked_task_is_reported_with_its_age_and_why_it_is_not_ready() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    // Parked before the fixture's last commit, so it has an age at `HEAD`:
    // committing it afterwards would make `HEAD` its own commit and every age
    // would read as zero.
    parked_fixture(dir.path());

    let (out, code) = aplan(dir.path(), &["stale", "--older-than", "10"]);
    assert_eq!(code, 0, "advice never fails a build: {out}");
    assert!(
        out.contains("A-PARK"),
        "the parked task is invisible: {out}"
    );
    assert!(
        out.contains("blocked"),
        "its status is what says why it is not ready: {out}"
    );
    // Not sold as startable: the marker belongs to what a reader can pick up.
    let row = out
        .lines()
        .find(|l| l.starts_with("A-PARK"))
        .unwrap_or_else(|| panic!("no row for A-PARK:\n{out}"));
    assert!(!row.contains("(ready)"), "offered as startable: {row}");
    assert!(
        out.contains("parked (`blocked`): set aside by a decision"),
        "a reader is left to guess what a `blocked` row means: {out}"
    );
}

/// A task that is parked **and** lists unfinished blockers is described
/// truthfully (`A1-275`, audit).
///
/// Nothing refuses that shape, and the first wording of the explanation --
/// *not waiting on the plan* -- was simply false for it. What holds either way
/// is that the *status* keeps it out of `ready`: clearing every blocker would
/// not offer it.
#[test]
fn a_task_parked_while_it_also_waits_on_work_is_still_described_truthfully() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // Parked, and waiting on a task that is not done.
    write(
        dir.path(),
        "A-BOTH",
        "blocked",
        "Parked while also blocked",
        Some("A-BLOCK"),
        "",
    );
    commit(
        dir.path(),
        "park something that also waits",
        "2026-06-15T12:00:00",
    );
    write(
        dir.path(),
        "A-NEW",
        "todo",
        "Touched at HEAD, again",
        None,
        "",
    );
    commit(dir.path(), "move HEAD forward", "2026-08-18T12:00:00");

    let (out, _) = aplan(dir.path(), &["stale", "--older-than", "10"]);
    assert!(out.contains("A-BOTH"), "{out}");
    assert!(
        !out.contains("not waiting on the plan"),
        "said something false about a task that is waiting on the plan too:\n{out}"
    );
    assert!(
        out.contains("clearing blockers does not make one ready"),
        "the one thing true of every parked row:\n{out}"
    );
}

/// One parked task reads as one, not as "1 of these are".
///
/// A single park is the common case, and a plural verb over a count of one is
/// the sort of thing a reader notices instead of the report.
#[test]
fn the_explanation_reads_correctly_for_a_single_parked_task() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    parked_fixture(dir.path());
    let (out, _) = aplan(dir.path(), &["stale", "--older-than", "10"]);
    assert!(out.contains("1 parked (`blocked`)"), "{out}");
    assert!(!out.contains("1 of these are parked"), "{out}");
}

/// The selector reaches the rows this task added.
///
/// `--status blocked` is how somebody asks the question directly, and a filter
/// that could not see parked work would leave the new rows unreachable by the
/// one selection that names them.
#[test]
fn selecting_by_status_reaches_a_parked_task() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    parked_fixture(dir.path());
    let (out, code) = aplan(
        dir.path(),
        &["stale", "--older-than", "10", "--status", "blocked"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("A-PARK"), "{out}");
    assert!(!out.contains("A-OLD"), "the selection was ignored: {out}");
}

/// And a plan with none of them answers exactly as it did before.
///
/// The guarantee an existing adopter needs: this widened what `stale` reports,
/// so a plan that parks nothing must not see a single byte change -- otherwise
/// a gate somebody wired moves under them (`A1-275`).
#[test]
fn a_plan_that_parks_nothing_reads_exactly_as_before() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (out, _) = aplan(dir.path(), &["stale", "--older-than", "10"]);
    assert!(
        !out.contains("parked"),
        "a plan with nothing parked was told about parking: {out}"
    );
    // The sentence is present only where it is earned, so its absence here is
    // the whole assertion: everything else in this file pins the rest of the
    // report unchanged.
    let (json, _) = aplan(
        dir.path(),
        &["stale", "--older-than", "10", "--format", "json"],
    );
    let v: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert!(
        v["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .all(|t| t["status"] != "blocked"),
        "{json}"
    );
}

/// A parked *milestone* stays out, like every other milestone.
///
/// A cut sits `todo` by design and is excluded so a report about neglect does
/// not carry a permanent row; parking one does not make it work.
#[test]
fn a_parked_milestone_is_still_not_work() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // Written *before* the last commit, so it has an age: parked at `HEAD` it
    // would be inside any window and this would pass without the milestone rule
    // doing anything (found by mutating that rule).
    std::fs::write(
        dir.path().join("aplan/tasks/M-s2.md"),
        "---\nid: M-s2\ntitle: A parked cut\nphase: 1\nstatus: blocked\n\
         tags: [milestone, s2]\nblocked_by: [A-OLD]\n---\n\nbody\n",
    )
    .expect("milestone");
    commit(dir.path(), "park a cut", "2026-06-15T12:00:00");
    write(
        dir.path(),
        "A-NEW",
        "todo",
        "Touched at HEAD, again",
        None,
        "",
    );
    commit(dir.path(), "move HEAD forward", "2026-08-18T12:00:00");

    let (out, _) = aplan(dir.path(), &["stale", "--older-than", "10"]);
    // The control: something else *is* reported, so the absence below is the
    // milestone rule rather than an empty report.
    assert!(out.contains("A-OLD"), "nothing was reported at all: {out}");
    assert!(
        !out.contains("M-s2"),
        "a parked cut is reported as work: {out}"
    );
}
