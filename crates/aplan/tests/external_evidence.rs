//! Evidence stops at the repository boundary, and says so (`A1-249`, ADR-0028).
//!
//! `traces_external` exists because a task may *answer to* a document in another
//! repository (ADR-0021). Evidence has no counterpart, and the ninth pilot hit
//! the asymmetry in the case this directory was invented for: work whose
//! deliverable is a friction log upstream (pre-public adoption evidence item 6).
//!
//! The decision is that `file` cannot cross and `url` is how a deliverable
//! elsewhere is cited. What was wrong was not the refusal but the sentence: a
//! sibling path was reported as not existing at HEAD, which the author can see is
//! false, and the remedy offered was to correct a path that is already correct.

use std::path::Path;
use std::process::Command;

mod common;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}");
}

/// A repository, with a sibling beside it holding a real file.
fn repo_beside_a_sibling() -> tempfile::TempDir {
    let top = tempfile::Builder::new()
        .prefix("external-evidence-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");

    let sibling = top.path().join("sibling/docs");
    std::fs::create_dir_all(&sibling).expect("sibling");
    std::fs::write(sibling.join("log.md"), "# upstream\n").expect("upstream file");

    let main = top.path().join("main");
    std::fs::create_dir_all(main.join("aplan")).expect("plan");
    std::fs::write(main.join("f.txt"), "x\n").expect("file");
    git(&main, &["init", "-q"]);
    git(&main, &["config", "user.email", "t@t"]);
    git(&main, &["config", "user.name", "t"]);
    git(&main, &["add", "-A"]);
    git(&main, &["commit", "-qm", "init"]);
    top
}

fn task(dir: &Path, evidence: &str) {
    std::fs::write(
        dir.join("aplan/T-1.md"),
        format!(
            "---\nid: T-1\ntitle: work whose deliverable landed elsewhere\nphase: 1\n\
             status: done\neffort_days: 1\npaths: [\"f.txt\"]\nevidence:\n{evidence}---\n\n\
             ## Why\n\nx\n"
        ),
    )
    .expect("task");
}

/// **A citation that leaves the repository is told what is wrong with it.**
///
/// "does not exist at HEAD" is false in the plain reading: the file is on the
/// author's disk and they can open it. The fault is that a `file` citation
/// resolves against this repository's history, where no rev holds that path.
#[test]
fn a_file_citation_outside_the_repository_says_so() {
    let top = repo_beside_a_sibling();
    let main = top.path().join("main");
    std::fs::write(main.join(".aplan.yml"), "root: aplan\n").expect("config");

    for reference in ["../sibling/docs/log.md", "/etc/passwd"] {
        task(&main, &format!("  - kind: file\n    ref: {reference}\n"));
        let out = common::run(&main, &["verify"]);
        assert_eq!(out.code, 1, "{}", out.text());
        assert!(
            out.text().contains("is outside this repository"),
            "`{reference}`: {}",
            out.text()
        );
        assert!(
            !out.text().contains("does not exist at HEAD"),
            "`{reference}` is not a missing file, and saying so sends the author to check a \
             path that is right: {}",
            out.text()
        );
        assert!(
            out.text().contains("url"),
            "and the remedy names what does cross the boundary: {}",
            out.text()
        );
    }
}

/// **A path inside the repository is judged exactly as before.**
///
/// The guard is lexical and narrow: it fires on a path that leaves, and on
/// nothing else. A citation that is simply wrong still gets the sentence it
/// always got.
#[test]
fn an_ordinary_citation_is_untouched() {
    let top = repo_beside_a_sibling();
    let main = top.path().join("main");
    std::fs::write(main.join(".aplan.yml"), "root: aplan\n").expect("config");

    task(&main, "  - kind: file\n    ref: f.txt\n");
    let out = common::run(&main, &["verify"]);
    assert_eq!(out.code, 0, "a real file still resolves: {}", out.text());

    task(&main, "  - kind: file\n    ref: gone.txt\n");
    let out = common::run(&main, &["verify"]);
    assert!(
        out.text().contains("does not exist at HEAD"),
        "a missing file is still a missing file: {}",
        out.text()
    );
}

/// **The remedy works**, which is the half a message is not allowed to promise
/// without.
///
/// `url` evidence naming the artefact on its forge resolves through the
/// repository's configured checker, and closes a task whose deliverable is not
/// in this repository at all.
#[test]
fn the_url_the_message_recommends_actually_closes_such_a_task() {
    let top = repo_beside_a_sibling();
    let main = top.path().join("main");
    // `true` stands in for the adopter's checker: this is about the citation
    // resolving through the configured command, not about reaching a network.
    std::fs::write(
        main.join(".aplan.yml"),
        "root: aplan\nurl:\n  check: [\"true\", \"{url}\"]\n",
    )
    .expect("config");
    task(
        &main,
        "  - kind: url\n    ref: https://forge.example/o/r/blob/abc123/docs/log.md\n",
    );

    let out = common::run(&main, &["verify"]);
    assert_eq!(
        out.code,
        0,
        "the documented way to cite a deliverable elsewhere has to work: {}",
        out.text()
    );
}

/// **`close` refuses it too, with the same sentence.**
///
/// The surface the pilot actually used: `aplan close AM-001 --evidence
/// file:../aplan/pre-public adoption evidence`. `close` resolves a proposed citation
/// before it writes, through the same resolvers, so the message an author meets
/// first is this one -- and a test that only drove `verify` would not have said
/// so (`A1-249`, audit).
#[test]
fn close_refuses_a_citation_that_leaves_the_repository() {
    let top = repo_beside_a_sibling();
    let main = top.path().join("main");
    std::fs::write(main.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        main.join("aplan/T-1.md"),
        "---\nid: T-1\ntitle: deliverable lands upstream\nphase: 1\nstatus: todo\n\
         effort_days: 1\npaths: [\"f.txt\"]\n---\n\n## Why\n\nx\n",
    )
    .expect("task");

    let out = common::run(
        &main,
        &["close", "T-1", "--evidence", "file:../sibling/docs/log.md"],
    );
    assert_ne!(out.code, 0, "{}", out.text());
    assert!(
        out.text().contains("is outside this repository"),
        "{}",
        out.text()
    );
    assert!(
        !out.text().contains("was recorded"),
        "a refusal must not claim anything was recorded: {}",
        out.text()
    );
    let after = std::fs::read_to_string(main.join("aplan/T-1.md")).expect("read");
    assert!(
        after.contains("status: todo") && !after.contains("evidence:"),
        "and the task is untouched: {after}"
    );
}

/// **A file whose name begins with two dots is not a climb.**
///
/// The edge the guard turns on. `Path::starts_with` compares components, so
/// `..foo.txt` is one component named `..foo.txt` and stays inside; a check
/// written against the string would have refused a legal filename and sent the
/// author looking for a sibling repository they do not have.
#[test]
fn a_filename_beginning_with_two_dots_is_still_inside() {
    let top = repo_beside_a_sibling();
    let main = top.path().join("main");
    std::fs::write(main.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(main.join("..odd.txt"), "y\n").expect("oddly named file");
    git(&main, &["add", "-A"]);
    git(&main, &["commit", "-qm", "odd"]);
    task(&main, "  - kind: file\n    ref: \"..odd.txt\"\n");

    let out = common::run(&main, &["verify"]);
    assert!(
        !out.text().contains("is outside this repository"),
        "a component named `..odd.txt` is inside the repository: {}",
        out.text()
    );
}
