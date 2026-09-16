//! `aplan paths`, breadth measured against history (`A1-151`).
//!
//! The first pilot's `AL1-34` declared `crates/p0000004-core/**`, where nearly
//! all that project's work happens, so `APLAN012` on it was close to a coin
//! flip. Breadth against the *file tree* does not separate that case; breadth
//! against *commits* does (pre-public adoption evidence item 11).
//!
//! What these pin is that the measurement is a measurement: it agrees with git,
//! it divides by what it actually sampled, it never fails a build, and it prints
//! the plan's own median beside every row because a share means nothing without
//! it.

use std::path::Path;
use std::process::Command;

mod common;
use common::aplan;

#[test]
fn the_share_is_the_fraction_of_sampled_commits_that_touch_the_paths() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (out, code) = aplan(dir.path(), &["paths", "--format", "json"]);
    assert_eq!(code, 0, "{out}");
    let doc: serde_json::Value = serde_json::from_str(&out).expect("json");

    // Four commits: three touch `src/`, one touches `docs/`.
    assert_eq!(doc["sampled"], 4, "{out}");
    let by_id = |id: &str| -> u64 {
        doc["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .find(|t| t["id"] == id)
            .unwrap_or_else(|| panic!("{id} is missing from {out}"))["percent"]
            .as_u64()
            .expect("a percentage")
    };
    assert_eq!(by_id("T-WIDE"), 100, "`**` matches every commit: {out}");
    assert_eq!(by_id("T-SRC"), 75, "three of four: {out}");
    assert_eq!(by_id("T-DOCS"), 25, "one of four: {out}");
    assert_eq!(doc["median_percent"], 75, "{out}");
}

/// The denominator is what was sampled, not what was asked for: a repository
/// shorter than the sample is the common case, and the other denominator would
/// understate every row in it.
#[test]
fn a_short_history_is_divided_by_what_there_was() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (out, code) = aplan(
        dir.path(),
        &["paths", "--sample", "500", "--format", "json"],
    );
    assert_eq!(code, 0, "{out}");
    let doc: serde_json::Value = serde_json::from_str(&out).expect("json");
    assert_eq!(doc["asked_for"], 500, "{out}");
    assert_eq!(doc["sampled"], 4, "{out}");
    // Still 100%, not 4/500.
    let wide = doc["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|t| t["id"] == "T-WIDE")
        .expect("the wide task");
    assert_eq!(wide["percent"], 100, "{out}");
}

/// A smaller sample sees a different history, which is the reason the default is
/// not smaller: at 30 commits one quiet week decides the answer (`A1-151`, the
/// measurement across five repositories).
#[test]
fn the_sample_is_a_window_and_moving_it_changes_the_answer() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (out, _) = aplan(dir.path(), &["paths", "--sample", "1", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&out).expect("json");
    assert_eq!(doc["sampled"], 1, "{out}");
    // The newest commit touches `docs/`, so a one-commit window says `docs` is
    // everything and `src` is nothing -- the failure mode a short sample has.
    let by_id = |id: &str| -> u64 {
        doc["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .find(|t| t["id"] == id)
            .expect("the task")["percent"]
            .as_u64()
            .expect("a percentage")
    };
    assert_eq!(by_id("T-DOCS"), 100, "{out}");
    assert_eq!(by_id("T-SRC"), 0, "{out}");
}

#[test]
fn it_measures_and_never_fails_a_build() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (out, code) = aplan(dir.path(), &["paths"]);
    assert_eq!(code, 0, "a measurement is not a verdict: {out}");
    // The median is printed with the rows, and the sentence that says how to
    // read one against it.
    assert!(
        out.contains("the median task's `paths` are touched by"),
        "{out}"
    );
    assert!(out.contains("Read a row against that median"), "{out}");
    // No task is called wrong.
    assert!(!out.to_lowercase().contains("too broad"), "{out}");
    assert!(!out.contains("APLAN"), "no rule code is implied: {out}");
}

/// A task with no `paths` is counted rather than measured: there is nothing to
/// match, and its `commit` evidence is the weaker kind this report exists to
/// talk about.
#[test]
fn tasks_without_paths_are_counted_not_measured() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/T-NONE.md"),
        "---\nid: T-NONE\ntitle: no paths\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
    )
    .expect("task");

    let (out, code) = aplan(dir.path(), &["paths", "--format", "json"]);
    assert_eq!(code, 0, "{out}");
    let doc: serde_json::Value = serde_json::from_str(&out).expect("json");
    assert_eq!(doc["without_paths"], 1, "{out}");
    assert!(
        !doc["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .any(|t| t["id"] == "T-NONE"),
        "{out}"
    );
    // And a milestone is not work, as in every other rollup.
    assert!(
        !doc["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .any(|t| t["id"] == "M-s1"),
        "{out}"
    );

    let (human, _) = aplan(dir.path(), &["paths"]);
    assert!(human.contains("1 task(s) declare no `paths`"), "{human}");
}

/// Cross-checked against git rather than against itself: for each task, what
/// `git log --name-only` says, computed independently here.
#[test]
fn every_share_agrees_with_git() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (out, _) = aplan(dir.path(), &["paths", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&out).expect("json");

    // Parse the log the way git writes it: a 40-hex line opens a commit, and
    // the file names follow until the next one. Splitting on blank lines looked
    // right and counted the wrong commits, which is why this test asserts
    // against a number derived here rather than one written down by hand.
    let (log, _) = git(dir.path(), &["log", "--format=%H", "--name-only"]);
    let mut commits: Vec<Vec<String>> = Vec::new();
    for line in log.lines() {
        let line = line.trim();
        if line.len() == 40 && line.chars().all(|c| c.is_ascii_hexdigit()) {
            commits.push(Vec::new());
        } else if !line.is_empty()
            && let Some(current) = commits.last_mut()
        {
            current.push(line.to_string());
        }
    }
    assert_eq!(commits.len(), 4, "the fixture's history: {log}");

    for (id, prefix) in [("T-SRC", "src/"), ("T-DOCS", "docs/")] {
        let touching = commits
            .iter()
            .filter(|files| files.iter().any(|f| f.starts_with(prefix)))
            .count();
        let expected = touching * 100 / 4;
        let got = doc["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .find(|t| t["id"] == id)
            .expect("the task")["percent"]
            .as_u64()
            .expect("a percentage");
        assert_eq!(
            got, expected as u64,
            "{id}: aplan says {got}%, git says {expected}%"
        );
    }
}

/// A repository with no commits sampled nothing, and a table of `0%` rows would
/// be this command saying "no recent commit touches these paths" about a history
/// it never read. The same false green `A1-020`'s audit found in `stale`.
/// Proves the named behaviour `paths.no_sample_is_not_a_zero` (`A1-211`).
#[test]
fn a_repository_with_no_history_reports_no_measurement() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture_without_commits(dir.path());

    let (out, code) = aplan(dir.path(), &["paths"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("no commit was sampled"), "{out}");
    assert!(
        !out.contains("T-SRC"),
        "no row may be printed for a measurement that did not happen: {out}"
    );
    assert!(!out.contains('%'), "and no share either: {out}");

    let (json, _) = aplan(dir.path(), &["paths", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(doc["sampled"], 0, "{json}");
    assert!(
        doc["median_percent"].is_null(),
        "a median of nothing is null, not 0: {json}"
    );
    assert_eq!(doc["tasks"].as_array().expect("tasks").len(), 0, "{json}");
}

/// `--sample 0` is the same emptiness asked for on purpose, and it is refused at
/// the flag rather than explained in the output.
#[test]
fn a_sample_of_zero_is_refused() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (out, code) = aplan(dir.path(), &["paths", "--sample", "0"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("measures nothing"), "{out}");
}

/// A `paths` entry that will not compile is not measured, and is certainly not
/// `0%`: that is the narrowest row the report can print, so a broken glob would
/// read as the best-scoped task in the plan. `APLAN032` is where the glob itself
/// is reported; this only declines to invent a number.
/// Proves the named behaviour `paths.no_sample_is_not_a_zero` (`A1-211`).
#[test]
fn a_path_that_does_not_compile_is_not_reported_as_zero() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/T-BAD.md"),
        "---\nid: T-BAD\ntitle: broken glob\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths:\n  - \"src/[a\"\n---\n\nbody\n",
    )
    .expect("task");

    let (json, code) = aplan(dir.path(), &["paths", "--format", "json"]);
    assert_eq!(code, 0, "still a measurement, not a verdict: {json}");
    let doc: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(doc["unmeasurable"], 1, "{json}");
    assert!(
        !doc["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .any(|t| t["id"] == "T-BAD"),
        "a task nothing was measured for must not carry a share: {json}"
    );

    let (human, _) = aplan(dir.path(), &["paths"]);
    assert!(human.contains("do not compile to a glob"), "{human}");
    assert!(human.contains("APLAN032"), "{human}");
    assert!(
        !human.contains("T-BAD"),
        "no row, because there is no number: {human}"
    );
}

/// One commit in two hundred and no commit at all both round to `0%`, and they
/// are opposite findings: the first is the tightest scope in the plan, the
/// second is a `paths` entry history has never touched.
#[test]
fn a_share_too_small_to_round_is_not_printed_as_none() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // 4 commits, one of which touches `docs/`. Ask for a hundred and the share
    // is 25%; make the history long enough and it rounds away.
    for n in 0..200 {
        std::fs::write(dir.path().join(format!("src/gen{n}.rs")), "// x\n").expect("write");
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", &format!("gen {n}")]);
    }

    let (json, _) = aplan(
        dir.path(),
        &["paths", "--sample", "300", "--format", "json"],
    );
    let doc: serde_json::Value = serde_json::from_str(&json).expect("json");
    let row = |id: &str| -> serde_json::Value {
        doc["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .find(|t| t["id"] == id)
            .unwrap_or_else(|| panic!("{id} is missing from {json}"))
            .clone()
    };
    let docs = row("T-DOCS");
    assert_eq!(docs["percent"], 0, "it does round to zero: {json}");
    assert_eq!(docs["hits"], 1, "and the count says it is not zero: {json}");

    let (human, _) = aplan(dir.path(), &["paths", "--sample", "300"]);
    assert!(
        human.contains("<1%"),
        "a share that rounds to nothing says so: {human}"
    );
    // And it outranks a task nothing touches. The id sorts *first*
    // alphabetically on purpose: ordering by the truncated percent leaves these
    // two tied, and the tie-break would put the untouched task on top.
    std::fs::write(
        dir.path().join("aplan/tasks/T-AAA.md"),
        "---\nid: T-AAA\ntitle: never touched\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths:\n  - nowhere/**\n---\n\nbody\n",
    )
    .expect("task");
    let (ordered, _) = aplan(dir.path(), &["paths", "--sample", "300"]);
    let at = |id: &str| {
        ordered
            .find(id)
            .unwrap_or_else(|| panic!("{id}: {ordered}"))
    };
    assert!(
        at("T-DOCS") < at("T-AAA"),
        "one commit in two hundred outranks none at all: {ordered}"
    );
}

/// The same fixture, before anything is committed.
fn fixture_without_commits(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "t@example.com"]);
    git(dir, &["config", "user.name", "Test"]);
    std::fs::write(
        dir.join("aplan/tasks/T-SRC.md"),
        "---\nid: T-SRC\ntitle: T-SRC\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths:\n- src/**\n---\n\nbody\n",
    )
    .expect("task");
}

/// Four commits: three touching `src/`, the newest touching `docs/`.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("src")).expect("mkdir");
    std::fs::create_dir_all(dir.join("docs")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "t@example.com"]);
    git(dir, &["config", "user.name", "Test"]);

    for (n, (path, body)) in [
        ("src/a.rs", "// a\n"),
        ("src/b.rs", "// b\n"),
        ("src/c.rs", "// c\n"),
        ("docs/guide.md", "# Guide\n"),
    ]
    .into_iter()
    .enumerate()
    {
        std::fs::write(dir.join(path), body).expect("write");
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", &format!("commit {n}")]);
    }

    for (id, paths) in [
        ("T-WIDE", "- '**'"),
        ("T-SRC", "- src/**"),
        ("T-DOCS", "- docs/**"),
    ] {
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: todo\neffort_days: 1\npaths:\n\
                 {paths}\n---\n\nbody\n"
            ),
        )
        .expect("task");
    }
    std::fs::write(
        dir.join("aplan/tasks/M-s1.md"),
        "---\nid: M-s1\ntitle: A cut\nphase: 1\nstatus: todo\ntags: [milestone, s1]\n\
         blocked_by: [T-WIDE, T-SRC, T-DOCS]\n---\n\nbody\n",
    )
    .expect("milestone");
}

fn git(dir: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("running git");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}
