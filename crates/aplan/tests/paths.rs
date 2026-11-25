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

use crate::common;
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

#[test]
#[allow(clippy::too_many_lines)]
fn paths_match_agrees_with_evidence_verification_across_all_glob_semantics() {
    let dir = tempfile::Builder::new()
        .prefix("paths-match-semantics-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();

    // Seed tracked files covering bare directories, `/**`, `/**/`, `*`, `?`,
    // dot-prefixed paths, and separator boundaries. Deliberately no `aplan/`
    // directory yet to prove standalone custom-gate usage needs no plan.
    for rel in [
        ".env.example",
        ".github/workflows/ci.yml",
        ".hidden/nested/file.txt",
        "docs/guide.md",
        "src/a.rs",
        "src/ab.rs",
        "src/mod.rs",
        "src/sub/deep.rs",
        "src/sub/mod.rs",
        "src/a/b/mod.rs",
    ] {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(&p, "x\n").expect("write");
    }
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "seed tracked tree"]);

    // 1. Standalone glob evaluation (no plan directory) covering all semantics.
    let (json, code) = aplan(
        root,
        &[
            "paths",
            "match",
            "--format",
            "json",
            "src",
            "src/**",
            "src/**/mod.rs",
            "src/*",
            "src/?.rs",
            ".github/workflows/*",
            ".hidden/**",
            "./src/?.rs",
            "src/../src/a.rs",
            "nowhere/**",
        ],
    );
    assert_eq!(
        code, 0,
        "all valid globs (including empty) must exit 0: {json}"
    );
    let doc: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(doc["schema"], "aplan/paths-match/v1");
    assert_eq!(doc["mode"], "globs");
    assert_eq!(doc["tracked_files"], 10);
    assert_eq!(doc["summary"]["entries"], 10);
    assert_eq!(doc["summary"]["matched"], 8);
    assert_eq!(doc["summary"]["empty"], 2);
    assert_eq!(doc["summary"]["invalid"], 0);

    let entry = |pat: &str| -> &serde_json::Value {
        doc["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .find(|e| e["pattern"] == pat)
            .unwrap_or_else(|| panic!("missing entry for {pat}: {json}"))
    };
    let matches_of = |pat: &str| -> Vec<String> {
        entry(pat)["matches"]
            .as_array()
            .expect("matches")
            .iter()
            .map(|v| v.as_str().expect("str").to_string())
            .collect()
    };

    // Bare directory: valid empty match with explicit bare_tracked_directory flag.
    assert_eq!(entry("src")["status"], "empty");
    assert_eq!(entry("src")["bare_tracked_directory"], true);
    assert_eq!(entry("nowhere/**")["status"], "empty");
    assert_eq!(entry("nowhere/**")["bare_tracked_directory"], false);

    // Wildcards, separator boundaries, dot-prefixed paths, lexical normalization.
    assert_eq!(
        matches_of("src/**"),
        vec![
            "src/a.rs",
            "src/a/b/mod.rs",
            "src/ab.rs",
            "src/mod.rs",
            "src/sub/deep.rs",
            "src/sub/mod.rs",
        ]
    );
    assert_eq!(
        matches_of("src/**/mod.rs"),
        vec!["src/a/b/mod.rs", "src/mod.rs", "src/sub/mod.rs"]
    );
    assert_eq!(
        matches_of("src/*"),
        vec!["src/a.rs", "src/ab.rs", "src/mod.rs"]
    );
    assert_eq!(matches_of("src/?.rs"), vec!["src/a.rs"]);
    assert_eq!(
        matches_of(".github/workflows/*"),
        vec![".github/workflows/ci.yml"]
    );
    assert_eq!(matches_of(".hidden/**"), vec![".hidden/nested/file.txt"]);
    assert_eq!(entry("./src/?.rs")["normalised"], "src/?.rs");
    assert_eq!(matches_of("./src/?.rs"), vec!["src/a.rs"]);
    assert_eq!(entry("src/../src/a.rs")["normalised"], "src/a.rs");
    assert_eq!(matches_of("src/../src/a.rs"), vec!["src/a.rs"]);

    // 2. Mixed matched + empty + invalid batch distinguishes all three states
    //    without parsing prose, and exits 1 because an entry is invalid.
    let (mixed_json, mixed_code) = aplan(
        root,
        &[
            "paths", "match", "--format", "json", "src/?.rs", "src", "src/[",
        ],
    );
    assert_eq!(mixed_code, 1, "invalid glob must exit 1: {mixed_json}");
    let mixed: serde_json::Value = serde_json::from_str(&mixed_json).expect("mixed json");
    assert_eq!(mixed["summary"]["matched"], 1);
    assert_eq!(mixed["summary"]["empty"], 1);
    assert_eq!(mixed["summary"]["invalid"], 1);
    assert_eq!(mixed["entries"][0]["status"], "matched");
    assert_eq!(mixed["entries"][1]["status"], "empty");
    assert_eq!(mixed["entries"][1]["bare_tracked_directory"], true);
    assert_eq!(mixed["entries"][2]["status"], "invalid");
    assert!(
        mixed["entries"][2]["error"]
            .as_str()
            .is_some_and(|e| e.contains('[')),
        "{mixed_json}"
    );

    let (mixed_human, mixed_human_code) =
        aplan(root, &["paths", "match", "src/?.rs", "src", "src/["]);
    assert_eq!(mixed_human_code, 1, "{mixed_human}");
    assert!(mixed_human.contains("[matched]"), "{mixed_human}");
    assert!(mixed_human.contains("[empty]"), "{mixed_human}");
    assert!(mixed_human.contains("[invalid]"), "{mixed_human}");
    assert!(mixed_human.contains("src/**"), "{mixed_human}");
    assert!(
        mixed_human.contains("Neither proves semantic relevance"),
        "{mixed_human}"
    );

    // 3. Cross-check `--task <ID>` against `aplan verify` on the exact same tasks:
    //    every task whose `paths match` is matched passes `APLAN053`/`APLAN063`,
    //    bare directory `src` fails `verify`, and invalid glob fails `APLAN032`.
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("tasks dir");
    std::fs::write(root.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        root.join("aplan/tasks/T-OK.md"),
        "---\nid: T-OK\ntitle: Valid task\nphase: 1\nstatus: in_progress\neffort_days: 1\n\
         paths:\n- src/?.rs\n- .github/workflows/*\n---\n\nbody\n",
    )
    .expect("T-OK");
    std::fs::write(
        root.join("aplan/tasks/T-BARE.md"),
        "---\nid: T-BARE\ntitle: Bare directory task\nphase: 1\nstatus: in_progress\neffort_days: 1\n\
         paths:\n- src\n---\n\nbody\n",
    )
    .expect("T-BARE");
    std::fs::write(
        root.join("aplan/tasks/T-EMPTY-LIST.md"),
        "---\nid: T-EMPTY-LIST\ntitle: No paths\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
    )
    .expect("T-EMPTY-LIST");

    let (task_ok_json, task_ok_code) = aplan(
        root,
        &["paths", "match", "--task", "T-OK", "--format", "json"],
    );
    assert_eq!(task_ok_code, 0, "{task_ok_json}");
    let task_ok: serde_json::Value = serde_json::from_str(&task_ok_json).expect("task json");
    assert_eq!(task_ok["mode"], "task");
    assert_eq!(task_ok["task"]["id"], "T-OK");
    assert_eq!(task_ok["summary"]["matched"], 2);
    assert_eq!(task_ok["summary"]["unique_matched_files"], 2);

    let (task_bare_json, task_bare_code) = aplan(
        root,
        &["paths", "match", "--task", "T-BARE", "--format", "json"],
    );
    assert_eq!(task_bare_code, 0, "{task_bare_json}");
    let task_bare: serde_json::Value = serde_json::from_str(&task_bare_json).expect("bare json");
    assert_eq!(task_bare["entries"][0]["status"], "empty");
    assert_eq!(task_bare["entries"][0]["bare_tracked_directory"], true);

    // Verify agrees: T-OK is clean, T-BARE is flagged for unmatched/bare directory.
    let (verify_json, _) = aplan(root, &["verify", "--format", "json"]);
    let verify_doc: serde_json::Value = serde_json::from_str(&verify_json).expect("verify json");
    let diags = verify_doc["diagnostics"].as_array().expect("diagnostics");
    assert!(
        !diags.iter().any(|d| d["task"] == "T-OK"),
        "T-OK matched in `paths match` and must have no verify path findings: {verify_json}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d["task"] == "T-BARE" && (d["code"] == "APLAN053" || d["code"] == "APLAN063")),
        "T-BARE was empty bare dir in `paths match` and must be flagged by verify: {verify_json}"
    );

    // Empty `paths` list on a task exits 0 cleanly; missing args & unknown task refuse.
    let (empty_task, empty_code) = aplan(root, &["paths", "match", "--task", "T-EMPTY-LIST"]);
    assert_eq!(empty_code, 0, "{empty_task}");
    assert!(empty_task.contains("declares no `paths`"), "{empty_task}");

    assert_ne!(aplan(root, &["paths", "match"]).1, 0);
    assert_ne!(aplan(root, &["paths", "match", "--task", "NO-SUCH"]).1, 0);
    assert_ne!(
        aplan(root, &["paths", "match", "--limit", "0", "src/**"]).1,
        0
    );
}

#[test]
fn paths_match_enforces_deterministic_order_and_explicit_bounded_contract() {
    let dir = tempfile::Builder::new()
        .prefix("paths-match-bounds-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("pkg")).expect("pkg dir");
    for i in 0..230 {
        std::fs::write(root.join(format!("pkg/f{i:03}.rs")), "// f\n").expect("write");
    }
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "seed 230 files"]);

    // Default limit (200) truncates deterministically and reports exact totals.
    let (def_json, code) = aplan(root, &["paths", "match", "pkg/**", "--format", "json"]);
    assert_eq!(code, 0, "{def_json}");
    let def: serde_json::Value = serde_json::from_str(&def_json).expect("def json");
    assert_eq!(def["limit"], 200);
    assert_eq!(def["summary"]["truncated_entries"], 1);
    assert_eq!(def["entries"][0]["total_matched"], 230);
    assert_eq!(def["entries"][0]["returned"], 200);
    assert_eq!(def["entries"][0]["truncated"], true);
    let def_matches = def["entries"][0]["matches"].as_array().expect("matches");
    assert_eq!(def_matches.len(), 200);
    assert_eq!(def_matches.first().unwrap(), "pkg/f000.rs");
    assert_eq!(def_matches.last().unwrap(), "pkg/f199.rs");

    // Human output explicitly reports the omitted count.
    let (human, human_code) = aplan(root, &["paths", "match", "pkg/**"]);
    assert_eq!(human_code, 0, "{human}");
    assert!(human.contains("showing 200 of 230"), "{human}");
    assert!(human.contains("30 more matched file(s)"), "{human}");

    // Custom `--limit 5` bounds cleanly.
    let (lim_json, lim_code) = aplan(
        root,
        &[
            "paths", "match", "pkg/**", "--limit", "5", "--format", "json",
        ],
    );
    assert_eq!(lim_code, 0, "{lim_json}");
    let lim: serde_json::Value = serde_json::from_str(&lim_json).expect("lim json");
    assert_eq!(lim["limit"], 5);
    assert_eq!(lim["entries"][0]["returned"], 5);
    assert_eq!(lim["entries"][0]["total_matched"], 230);
    assert_eq!(lim["entries"][0]["truncated"], true);
    assert_eq!(
        lim["entries"][0]["matches"],
        serde_json::json!([
            "pkg/f000.rs",
            "pkg/f001.rs",
            "pkg/f002.rs",
            "pkg/f003.rs",
            "pkg/f004.rs",
        ])
    );

    // `--all` emits every matched tracked file in deterministic lexicographical order.
    let (all_json, all_code) = aplan(
        root,
        &["paths", "match", "pkg/**", "--all", "--format", "json"],
    );
    assert_eq!(all_code, 0, "{all_json}");
    let all: serde_json::Value = serde_json::from_str(&all_json).expect("all json");
    assert!(all["limit"].is_null());
    assert_eq!(all["summary"]["truncated_entries"], 0);
    assert_eq!(all["entries"][0]["returned"], 230);
    assert_eq!(all["entries"][0]["total_matched"], 230);
    assert_eq!(all["entries"][0]["truncated"], false);
    let all_matches = all["entries"][0]["matches"].as_array().expect("matches");
    assert_eq!(all_matches.first().unwrap(), "pkg/f000.rs");
    assert_eq!(all_matches.last().unwrap(), "pkg/f229.rs");
}
