//! Two fields carry a path, and they agree about how it may be written
//! (`A1-254`).
//!
//! `traces` resolves on the filesystem and has always been normalised, so every
//! spelling of one path worked. `file` evidence is a lookup in a git tree, which
//! stores one literal path and had no entry for `./f.txt`. Both readings are
//! defensible alone and the pair was not, because an adopter learns the first
//! and writes the second -- and the message they got was `does not exist at
//! HEAD` about a file they could open, which is the shape `A1-249` fixed one
//! case of.
//!
//! A path is a locator, not an identity: `./src/a.rs` and `src/a.rs` name one
//! blob, and accepting both proves nothing extra. Ids are the opposite and stay
//! refused rather than rewritten (`emit::is_usable_id`).

use std::path::Path;
use std::process::Command;

use crate::common;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}");
}

/// A repository with one source file and one traceable document.
fn repo() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("path-spelling-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan")).expect("plan");
    std::fs::create_dir_all(dir.path().join("src")).expect("src");
    std::fs::write(dir.path().join("src/a.rs"), "fn main() {}\n").expect("source");
    std::fs::write(
        dir.path().join("DOC.md"),
        "# Doc\n\n## The rail\n\nFixed at three points.\n",
    )
    .expect("doc");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "t@t"]);
    git(dir.path(), &["config", "user.name", "t"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "init"]);
    dir
}

/// The spellings that name one path, for each of the three fields that carry
/// one. `sub/../x` is the interesting one: it is what a generator produces when
/// it joins a directory it later backs out of.
const SPELLINGS: &[(&str, &str, &str)] = &[
    ("src/a.rs", "DOC.md#the-rail", "src/**"),
    ("./src/a.rs", "./DOC.md#the-rail", "./src/**"),
    ("src/../src/a.rs", "src/../DOC.md#the-rail", "src/../src/**"),
];

/// **One spelling, two fields, one answer.**
///
/// Pins the pair rather than one side, which is the point: either field passing
/// alone is what the plan had before, and the failure was that they disagreed.
/// Proves the named behaviour `evidence.file_path_is_normalised` (`A1-254`).
#[test]
fn a_path_spelled_any_way_resolves_in_both_fields() {
    for (evidence, trace, globs) in SPELLINGS {
        let dir = repo();
        std::fs::write(
            dir.path().join("aplan/T-1.md"),
            format!(
                "---\nid: T-1\ntitle: t\nphase: 1\nstatus: done\neffort_days: 1\n\
                 paths: [\"{globs}\"]\ntraces:\n- \"{trace}\"\nevidence:\n  - kind: file\n    \
                 ref: \"{evidence}\"\n---\n\n## Why\n\nx\n"
            ),
        )
        .expect("task");

        let check = common::run(dir.path(), &["check"]);
        assert_eq!(
            check.code,
            0,
            "`traces: {trace}` must resolve: {}",
            check.text()
        );
        let verify = common::run(dir.path(), &["verify"]);
        assert_eq!(
            verify.code,
            0,
            "`file: {evidence}` and `paths: {globs}` must resolve, exactly as the trace \
             beside them does: {}",
            verify.text()
        );
    }
}

/// **Relevance is judged on the same reading.**
///
/// `paths: [src/**]` does not match the literal `./src/a.rs`, so a citation that
/// only started resolving would have failed the rung after it instead. The
/// normalisation has to reach the comparison too, or the fix moves the error
/// rather than removing it.
/// Proves the named behaviour `evidence.file_path_is_normalised` (`A1-254`).
#[test]
fn an_oddly_spelled_citation_is_judged_against_paths_the_same_way() {
    let dir = repo();
    std::fs::write(
        dir.path().join("aplan/T-1.md"),
        "---\nid: T-1\ntitle: t\nphase: 1\nstatus: done\neffort_days: 1\npaths: [\"src/**\"]\n\
         evidence:\n  - kind: file\n    ref: \"./src/a.rs\"\n---\n\n## Why\n\nx\n",
    )
    .expect("task");

    let out = common::run(dir.path(), &["verify"]);
    assert_eq!(out.code, 0, "{}", out.text());
    assert!(
        !out.text().contains("APLAN031"),
        "the glob and the citation must meet in one frame: {}",
        out.text()
    );
}

/// **A file that is genuinely absent still says so, in the spelling the author
/// used.**
///
/// The normalisation is for the lookup, not for the message: a reader has to
/// find the citation in their own file, and naming a spelling they did not write
/// is the same defect one step along.
#[test]
fn a_missing_file_is_reported_as_the_author_wrote_it() {
    let dir = repo();
    std::fs::write(
        dir.path().join("aplan/T-1.md"),
        "---\nid: T-1\ntitle: t\nphase: 1\nstatus: done\neffort_days: 1\npaths: [\"src/**\"]\n\
         evidence:\n  - kind: file\n    ref: \"./src/gone.rs\"\n---\n\n## Why\n\nx\n",
    )
    .expect("task");

    let out = common::run(dir.path(), &["verify"]);
    assert_eq!(out.code, 1, "{}", out.text());
    assert!(
        out.text().contains("`./src/gone.rs` does not exist"),
        "the message names what the author typed: {}",
        out.text()
    );
}

/// **A path that leaves the repository is still refused**, which normalising
/// must not undo: `..` at the front survives it, and `A1-249`'s guard runs
/// first.
#[test]
fn normalising_does_not_open_a_way_out_of_the_repository() {
    let dir = repo();
    std::fs::write(
        dir.path().join("aplan/T-1.md"),
        "---\nid: T-1\ntitle: t\nphase: 1\nstatus: done\neffort_days: 1\npaths: [\"src/**\"]\n\
         evidence:\n  - kind: file\n    ref: \"src/../../elsewhere/x.md\"\n---\n\n## Why\n\nx\n",
    )
    .expect("task");

    let out = common::run(dir.path(), &["verify"]);
    assert!(
        out.text().contains("is outside this repository"),
        "{}",
        out.text()
    );
}

/// **Every message names the written spelling, including the anchor's.**
///
/// The claim was made in the task, the commit and `docs/upgrading.md`, and it
/// was true of one message. `unresolved_anchor` was handed the normalised path
/// and used it for the lookup *and* for its three sentences, so a citation of
/// `./DOC.md#nope` was answered about `DOC.md` -- a spelling the author would
/// not find in their file (`A1-254`, audit).
#[test]
fn an_anchor_that_does_not_resolve_is_reported_as_written() {
    let dir = repo();
    std::fs::write(
        dir.path().join("aplan/T-1.md"),
        "---\nid: T-1\ntitle: t\nphase: 1\nstatus: done\neffort_days: 1\npaths: [\"src/**\"]\n\
         evidence:\n  - kind: file\n    ref: \"./DOC.md#nope\"\n---\n\n## Why\n\nx\n",
    )
    .expect("task");

    let out = common::run(dir.path(), &["verify"]);
    assert_eq!(out.code, 1, "{}", out.text());
    assert!(
        out.text().contains("file `./DOC.md` exists at HEAD"),
        "the anchor message names what the author typed: {}",
        out.text()
    );
    assert!(
        out.text().contains("a section `./DOC.md` has"),
        "and so does its remedy: {}",
        out.text()
    );
}

/// **An empty anchor is reported as written too**, which is the same function's
/// other early return and would have been missed by a test that only drove the
/// mismatch.
#[test]
fn an_empty_anchor_is_reported_as_written() {
    let dir = repo();
    std::fs::write(
        dir.path().join("aplan/T-1.md"),
        "---\nid: T-1\ntitle: t\nphase: 1\nstatus: done\neffort_days: 1\npaths: [\"src/**\"]\n\
         evidence:\n  - kind: file\n    ref: \"./DOC.md#\"\n---\n\n## Why\n\nx\n",
    )
    .expect("task");

    let out = common::run(dir.path(), &["verify"]);
    assert!(
        out.text().contains("`./DOC.md#` ends in an empty anchor"),
        "{}",
        out.text()
    );
}
