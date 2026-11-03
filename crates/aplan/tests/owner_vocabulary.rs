//! A declared owner vocabulary reaches `check` from a real `.aplan.yml` (`A1-190`).
//!
//! Core tests exercise the predicate and config tests exercise the declaration.
//! This is the seam between them: the binary has to map the selected config
//! into validator options without changing the owner strings reports expose.

use crate::common;

fn repo(config: &str, owner: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan directory");
    std::fs::write(dir.path().join(".aplan.yml"), config).expect("configuration");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        format!(
            "---\nid: T-1\ntitle: A task\nphase: 1\nstatus: todo\nowner: {owner}\n\
             effort_days: 1\n---\n\n## Why\n\nSomething.\n"
        ),
    )
    .expect("task");
    dir
}

#[test]
fn a_declared_owner_typo_is_reported_end_to_end() {
    let dir = repo("owners: [kaminsod]\n", "kaminsdo");
    let run = common::run(dir.path(), &["check"]);
    assert!(run.ok(), "the advisory should not fail by default: {run:?}");
    assert!(run.stdout.contains("warning[APLAN056] T-1"), "{run:?}");
    assert!(run.stdout.contains("owner `kaminsdo`"), "{run:?}");
    assert!(run.stdout.contains("did you mean `kaminsod`?"), "{run:?}");
    assert!(
        run.stdout.contains("aplan/tasks/T-1.md:6:1"),
        "the rule did not point at the owner field: {run:?}"
    );
}

#[test]
fn an_exact_owner_and_a_repository_with_no_vocabulary_stay_silent() {
    let exact = repo("owners: [kaminsod]\n", "kaminsod");
    let run = common::run(exact.path(), &["check"]);
    assert!(run.ok(), "{run:?}");
    assert!(!run.stdout.contains("APLAN056"), "{run:?}");

    let open = repo("require_effort: true\n", "any-spelling");
    let run = common::run(open.path(), &["check"]);
    assert!(run.ok(), "{run:?}");
    assert!(!run.stdout.contains("APLAN056"), "{run:?}");
}

#[test]
fn claim_identity_is_independent_of_the_durable_owner_vocabulary() {
    let dir = repo("owners: [ada]\ndeny_warnings: true\n", "");
    git2::Repository::init(dir.path()).expect("git init");
    // Turn the fixture's empty scalar into an actually unassigned task.
    let path = dir.path().join("aplan/tasks/T-1.md");
    let before = std::fs::read_to_string(&path).expect("task");
    let before = before.replace("owner: \n", "");
    std::fs::write(&path, &before).expect("unassigned task");

    let claimed = common::run(dir.path(), &["claim", "T-1", "--as", "grace"]);
    assert!(claimed.ok(), "{claimed:?}");
    assert_eq!(
        std::fs::read_to_string(&path).expect("task after claim"),
        before,
        "an ephemeral identity was written as durable ownership"
    );
    let state =
        std::fs::read_to_string(dir.path().join(".git/aplan/claims-v1.json")).expect("claim state");
    assert!(state.contains("grace"), "{state}");
}

#[test]
fn add_cannot_write_an_owner_the_repository_does_not_declare() {
    let dir = repo("owners: [ada]\ndeny_warnings: true\n", "ada");
    let added = dir.path().join("aplan/tasks/T-2.md");

    let refused = common::run(
        dir.path(),
        &["add", "Second task", "--owner", "grace", "--effort", "1"],
    );
    assert_eq!(refused.code, 2, "{refused:?}");
    assert!(refused.stderr.contains("APLAN056"), "{refused:?}");
    assert!(!added.exists(), "a refused add wrote {}", added.display());

    let accepted = common::run(
        dir.path(),
        &["add", "Second task", "--owner", "ada", "--effort", "1"],
    );
    assert!(accepted.ok(), "{accepted:?}");
    let written = std::fs::read_to_string(&added).expect("added task");
    assert!(written.contains("\nowner: ada\n"), "{written}");
}
