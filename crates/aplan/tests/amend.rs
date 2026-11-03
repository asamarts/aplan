//! Repairing a closed task's evidence (`A1-110`).
//!
//! `test` evidence is cited by name, so a rename leaves every citing task
//! pointing at nothing, including tasks closed weeks earlier that nobody is
//! editing. One refactor in the first pilot did it to two closed tasks at once:
//! seven citations, suite green, clippy clean, and `verify` red. The tool's
//! answer was `close` refusing twice and then telling the author to "edit the
//! rest by hand", so the repair for a tool-detected problem was to hand-edit the
//! files the tool owns (pre-public adoption evidence, item 9).
//!
//! Driven through the built binary, for `A1-107`'s reason: a unit test on
//! `replace_evidence` proves the rewrite. It does not prove that the command
//! resolves the replacement, and that is the half this task could most easily
//! have got wrong.

use std::path::{Path, PathBuf};

use crate::common;

fn source_delta_plan(source: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    let path = dir.path().join("aplan/tasks/W-1.md");
    std::fs::write(&path, source).expect("task");
    common::git(dir.path(), &["init", "-q", "."]);
    common::git(dir.path(), &["config", "user.email", "t@example.com"]);
    common::git(dir.path(), &["config", "user.name", "t"]);
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-q", "-m", "fixture"]);
    (dir, path)
}

/// **A citation is named by `kind:ref`, and that is what removal matches**
/// (`A1-361`). This also proves
/// `amend.policy_reads_rewritten_evidence_candidate`: replacement policy is
/// evaluated against what the core actually rewrote, including when the
/// source citation carries a date the command syntax cannot name (`A1-717`).
///
/// Two defects met here. The shared `kind:ref` parser refused `note` for every
/// mode, though only the modes that *write* a citation have anything to say
/// about a kind `close` will not record; and the match compared the whole
/// `Evidence`, including a `captured` date a caller naming a citation cannot
/// supply. Together they made a dated `note` unmaintainable: `close` says to add
/// one by hand, and `amend` would not take it away.
///
/// Every citation in this repository carries no date, so the two comparisons
/// agreed here and would have parted on an imported plan -- `import
/// --implemented-as done` records note-level provenance *with* a date.
#[test]
fn a_dated_note_citation_can_be_removed_and_replaced() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/W-1.md"),
        "---\nid: W-1\ntitle: closed on notes\nphase: 1\nstatus: done\n\
         effort_days: 1\nevidence:\n  - kind: note\n    ref: reviewed\n\
         \x20   captured: 2026-08-14\n  - kind: note\n    ref: second\n\
         \x20   captured: 2026-08-15\n---\n\n## Why\n\nBecause.\n",
    )
    .expect("task");
    common::git(dir.path(), &["init", "-q", "."]);
    common::git(dir.path(), &["config", "user.email", "t@example.com"]);
    common::git(dir.path(), &["config", "user.name", "t"]);
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-q", "-m", "fixture"]);

    // Removal names what is already there, so the kind `close` refuses to write
    // is still a kind this can take away.
    let out = common::run(
        dir.path(),
        &["amend", "W-1", "--remove-evidence", "note:reviewed"],
    );
    assert_eq!(
        out.code,
        0,
        "a dated note could not be removed: {}",
        out.text()
    );

    // The replacement's candidate, not a separately reconstructed vector, is
    // what the resolver must judge. Full-record equality does not match this
    // dated note to the undated `kind:ref` flag and used to make the delta look
    // empty, allowing the missing file to bypass resolution.
    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/W-1.md")).expect("task");
    let out = common::run(
        dir.path(),
        &[
            "amend",
            "W-1",
            "--replace-evidence",
            "note:second",
            "file:missing.md",
        ],
    );
    assert_ne!(
        out.code,
        0,
        "a missing replacement was accepted: {}",
        out.text()
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("aplan/tasks/W-1.md")).expect("task"),
        before,
        "the unresolved replacement was written"
    );

    // The `from` side of a replacement is the same: nothing is recorded by it.
    let out = common::run(
        dir.path(),
        &[
            "amend",
            "W-1",
            "--replace-evidence",
            "note:second",
            "file:.aplan.yml",
        ],
    );
    assert_eq!(
        out.code,
        0,
        "a dated note could not be replaced: {}",
        out.text()
    );

    let body = std::fs::read_to_string(dir.path().join("aplan/tasks/W-1.md")).expect("task");
    assert!(
        !body.contains("ref: reviewed"),
        "the removal did not land:\n{body}"
    );
    assert!(
        !body.contains("ref: second"),
        "the replacement did not land:\n{body}"
    );
    assert!(
        body.contains("ref: .aplan.yml"),
        "the new citation is missing:\n{body}"
    );

    // And the mode that *writes* still refuses it, which is the half the rule
    // was right about.
    let out = common::run(
        dir.path(),
        &["amend", "W-1", "--add-evidence", "note:third"],
    );
    assert_ne!(
        out.code,
        0,
        "adding a note should still be refused: {}",
        out.text()
    );
    assert!(
        out.text().contains("needs a `captured` date"),
        "refused for some other reason: {}",
        out.text()
    );
}

/// Proves `amend.preserves_evidence_sequence_source_deltas` through the real
/// command: add, remove and replace keep every unaddressed citation byte,
/// authored comment and line ending instead of merely round-tripping the same
/// typed values (`A1-722`).
#[test]
fn ordinary_evidence_amendments_are_exact_source_deltas() {
    let source = concat!(
        "---\n",
        "id: W-1\n",
        "title: source-preserving evidence\n",
        "phase: 1\n",
        "status: done\n",
        "effort_days: 1\n",
        "evidence : # evidence boundary\n",
        "- kind : 'test' # first kind\n",
        "  ref : \"alpha\" # first ref\n",
        "# target follows\n",
        "- ref : 'docs/old.md' # target ref\n",
        "  kind : file # target kind\n",
        "  captured: 2026-10-30 # target date\n",
        "# retained citation follows\n",
        "- kind: note # retained kind\n",
        "  ref: reviewed # retained ref\n",
        "  captured : '2026-10-31' # retained date\n",
        "# after evidence\n",
        "owner: alice\n",
        "---\n\n",
        "Body stays exact.\n",
    );

    for crlf in [false, true] {
        let source = if crlf {
            source.replace('\n', "\r\n")
        } else {
            source.to_string()
        };
        let nl = if crlf { "\r\n" } else { "\n" };

        let (dir, path) = source_delta_plan(&source);
        let out = common::run(
            dir.path(),
            &[
                "amend",
                "W-1",
                "--add-evidence",
                "commit:abc123",
                "--no-resolve",
            ],
        );
        assert_eq!(out.code, 0, "add failed: {}", out.text());
        let expected = source.replace(
            &format!("  captured : '2026-10-31' # retained date{nl}"),
            &format!(
                "  captured : '2026-10-31' # retained date{nl}- kind: commit{nl}  ref: abc123{nl}"
            ),
        );
        assert_eq!(std::fs::read_to_string(path).expect("added task"), expected);

        let (dir, path) = source_delta_plan(&source);
        let out = common::run(
            dir.path(),
            &[
                "amend",
                "W-1",
                "--replace-evidence",
                "file:docs/old.md",
                "test:beta",
                "--no-resolve",
            ],
        );
        assert_eq!(out.code, 0, "replace failed: {}", out.text());
        let expected = source.replace(
            &format!(
                "- ref : 'docs/old.md' # target ref{nl}  kind : file # target kind{nl}  captured: 2026-10-30 # target date{nl}"
            ),
            &format!("- ref : 'beta' # target ref{nl}  kind : test # target kind{nl}"),
        );
        assert_eq!(
            std::fs::read_to_string(path).expect("replaced task"),
            expected
        );

        let (dir, path) = source_delta_plan(&source);
        let out = common::run(
            dir.path(),
            &["amend", "W-1", "--remove-evidence", "file:docs/old.md"],
        );
        assert_eq!(out.code, 0, "remove failed: {}", out.text());
        let expected = source.replace(
            &format!(
                "- ref : 'docs/old.md' # target ref{nl}  kind : file # target kind{nl}  captured: 2026-10-30 # target date{nl}"
            ),
            "",
        );
        assert_eq!(
            std::fs::read_to_string(path).expect("removed task"),
            expected
        );
    }
}

/// The third shape: proof that was never proof (`A1-137`).
///
/// `A1-123` wrote a citation for `A1-120` that would not have failed if that
/// task's work were reverted, noticed, and deleted it in a text editor because
/// nothing here could. `A1-045` then found five more on this plan, each a
/// `file:` naming the test that proves the task, written before a `test:`
/// citation could resolve at all.
#[test]
fn a_citation_that_was_never_proof_can_be_removed() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (stdout, stderr, ok) = run(
        dir.path(),
        &["amend", "AL1-25", "--remove-evidence", "file:tests/t.rs"],
    );
    assert!(ok, "{stdout}{stderr}");
    assert!(stdout.contains("- file:tests/t.rs"), "{stdout}");

    let text = std::fs::read_to_string(dir.path().join("aplan/tasks/AL1-25.md")).expect("read");
    assert!(!text.contains("tests/t.rs"), "{text}");
    // What remains is untouched, including the block's shape and everything
    // outside it.
    assert!(text.contains("- kind: test"), "{text}");
    assert!(text.contains("status: done"), "{text}");
    assert!(text.ends_with("body\n"), "{text}");

    // And the neighbour is not edited: a repair names one task.
    let other = std::fs::read_to_string(dir.path().join("aplan/tasks/AL1-26.md")).expect("read");
    assert!(other.contains("tests/t.rs"), "{other}");

    let (out, err, ok) = run(dir.path(), &["check"]);
    assert!(ok, "{out}{err}");
}

/// Removal cannot take a task below the bar its repository set (`A1-137`).
///
/// That was the argument against building it: removal is how a closed task
/// quietly loses its proof, and unlike the other two shapes there is nothing for
/// the gate to resolve. `A1-113`'s post-state check is the answer, and this is
/// what says so.
#[test]
fn a_removal_that_would_leave_the_task_unproven_is_refused() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // Take the file citation first, leaving the test citation alone.
    let (_, _, ok) = run(
        dir.path(),
        &["amend", "AL1-25", "--remove-evidence", "file:tests/t.rs"],
    );
    assert!(ok);

    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/AL1-25.md")).expect("read");
    let (stdout, stderr, ok) = run(
        dir.path(),
        &[
            "amend",
            "AL1-25",
            "--remove-evidence",
            "test:each_rail_appears_exactly_once",
        ],
    );
    let said = format!("{stdout}{stderr}");
    assert!(!ok, "a `done` task with no evidence was written: {said}");
    assert!(said.contains("APLAN005"), "{said}");
    assert_eq!(
        before,
        std::fs::read_to_string(dir.path().join("aplan/tasks/AL1-25.md")).expect("read"),
        "the refusal wrote anyway"
    );

    // A citation the task does not carry is told apart from one it does.
    let (stdout, stderr, ok) = run(
        dir.path(),
        &["amend", "AL1-25", "--remove-evidence", "test:not_cited"],
    );
    let said = format!("{stdout}{stderr}");
    assert!(!ok, "{said}");
    assert!(said.contains("cites no `test:not_cited`"), "{said}");

    // And a live task is not repaired: `close` is how a live task gets evidence.
    std::fs::write(
        dir.path().join("aplan/tasks/AL1-27.md"),
        "---\nid: AL1-27\ntitle: still open\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
    )
    .expect("task");
    let (stdout, stderr, ok) = run(
        dir.path(),
        &["amend", "AL1-27", "--remove-evidence", "test:whatever"],
    );
    let said = format!("{stdout}{stderr}");
    assert!(!ok, "{said}");
    assert!(
        said.contains("todo"),
        "it names the status it found: {said}"
    );
}

/// The pilot's shape: two tasks closed on a test that has since been renamed.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("tests")).expect("mkdir");
    std::fs::write(
        dir.join(".aplan.yml"),
        "root: aplan\ntest:\n  \
         list: [\"printf\", \"each_rail_appears_exactly_once_named_by_its_vector\\nsuite::each_rail_appears_exactly_once\\n\"]\n  \
         run: [\"true\", \"{test}\"]\n",
    )
    .expect("config");
    for id in ["AL1-25", "AL1-26"] {
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: closed before the rename\nphase: 1\nstatus: done\n\
                 owner: t\neffort_days: 1\npaths:\n- tests/**\nevidence:\n  \
                 - kind: test\n    ref: each_rail_appears_exactly_once\n  \
                 - kind: file\n    ref: tests/t.rs\n---\n\nbody\n"
            ),
        )
        .expect("task");
    }
    std::fs::write(dir.join("tests/t.rs"), "fn main() {}\n").expect("test file");
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "initial"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }
}

fn run(dir: &Path, args: &[&str]) -> (String, String, bool) {
    let out = common::output(dir, args);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

fn task(dir: &Path, id: &str) -> String {
    std::fs::read_to_string(dir.join(format!("aplan/tasks/{id}.md"))).expect("read")
}

/// Evidence freshness follows the citation through every amend shape. A new or
/// replacement citation gets a baseline only after resolution; removal cannot
/// leave an orphaned baseline, and `--no-resolve` cannot claim content was
/// reviewed when the command deliberately did not read it (`A1-196`).
#[test]
fn amend_maintains_only_resolved_mutable_evidence_fingerprints() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(dir.path().join("docs")).expect("docs");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndetect_drift: true\n",
    )
    .expect("config");
    std::fs::write(dir.path().join("docs/one.md"), "# Proof\n\none\n").expect("proof one");
    std::fs::write(dir.path().join("docs/two.md"), "# Proof\n\ntwo\n").expect("proof two");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: mutable proof\nphase: 1\nstatus: done\neffort_days: 1\n\
         paths: [docs/**]\ntags: [cross-cutting]\nevidence:\n  - kind: note\n    ref: reviewed\n\
         \x20   captured: 2026-10-10\n---\n\nbody\n",
    )
    .expect("task");
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "fixture"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }

    let (out, err, ok) = run(
        dir.path(),
        &["amend", "T-1", "--add-evidence", "file:docs/one.md"],
    );
    assert!(ok, "{out}{err}");
    let added = task(dir.path(), "T-1");
    assert!(added.contains("evidence_fingerprints:"), "{added}");
    assert!(added.contains("file:docs/one.md"), "{added}");

    // Turning the detector off controls findings and new capture; it must not
    // erase a baseline an unrelated edit did not supersede.
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndetect_drift: false\n",
    )
    .expect("disable drift");
    let (out, err, ok) = run(
        dir.path(),
        &[
            "amend",
            "T-1",
            "--add-evidence",
            "test:deferred_test",
            "--no-resolve",
        ],
    );
    assert!(ok, "{out}{err}");
    let disabled = task(dir.path(), "T-1");
    assert!(
        disabled.contains("evidence_fingerprints:") && disabled.contains("file:docs/one.md"),
        "disabling the detector erased an unrelated baseline: {disabled}"
    );
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndetect_drift: true\n",
    )
    .expect("restore drift");

    let (out, err, ok) = run(
        dir.path(),
        &[
            "amend",
            "T-1",
            "--replace-evidence",
            "file:docs/one.md",
            "file:docs/two.md",
        ],
    );
    assert!(ok, "{out}{err}");
    let replaced = task(dir.path(), "T-1");
    let fingerprint_block = replaced
        .split("evidence_fingerprints:")
        .nth(1)
        .expect("fingerprint block");
    assert!(
        !fingerprint_block.contains("file:docs/one.md"),
        "{replaced}"
    );
    assert!(fingerprint_block.contains("file:docs/two.md"), "{replaced}");

    let (out, err, ok) = run(
        dir.path(),
        &["amend", "T-1", "--remove-evidence", "file:docs/two.md"],
    );
    assert!(ok, "{out}{err}");
    assert!(
        !task(dir.path(), "T-1").contains("evidence_fingerprints:"),
        "removal left an orphaned freshness claim"
    );

    let (out, err, ok) = run(
        dir.path(),
        &[
            "amend",
            "T-1",
            "--add-evidence",
            "file:docs/one.md",
            "--no-resolve",
        ],
    );
    assert!(ok, "{out}{err}");
    let unresolved = task(dir.path(), "T-1");
    assert!(unresolved.contains("ref: docs/one.md"), "{unresolved}");
    assert!(
        !unresolved.contains("evidence_fingerprints:"),
        "an unresolved citation was presented as fingerprinted: {unresolved}"
    );
}

fn runner_mutates_sibling(mutation: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(dir.path().join("src")).expect("src");
    std::fs::write(dir.path().join("src/a.rs"), "pub fn a() {}\n").expect("source");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: repairable\nphase: 1\nstatus: done\neffort_days: 1\n\
         paths: [src/**]\ntags: [cross-cutting]\nevidence:\n  - kind: test\n    ref: known\n\
         ---\n\nbody\n",
    )
    .expect("repairable task");
    std::fs::write(
        dir.path().join("aplan/tasks/T-2.md"),
        "---\nid: T-2\ntitle: runner changes this sibling\nphase: 1\nstatus: todo\n\
         effort_days: 1\npaths: [src/**]\ntags: [cross-cutting]\n---\n\nbody\n",
    )
    .expect("sibling task");
    std::fs::write(
        dir.path().join("mutate-list.sh"),
        format!("#!/bin/sh\n{mutation}\nprintf 'suite::known\\n'\n"),
    )
    .expect("runner");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ntest:\n  list: [sh, mutate-list.sh]\n  run: ['true', '{test}']\n",
    )
    .expect("config");
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "fixture"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }
    dir
}

fn runner_mutates_sibling_on_second_list(mutation: &str) -> tempfile::TempDir {
    let dir = runner_mutates_sibling("");
    std::fs::write(
        dir.path().join("mutate-list.sh"),
        format!(
            "#!/bin/sh\ncount=0\n[ ! -f list-count ] || count=$(cat list-count)\n\
             count=$((count + 1))\nprintf '%s\\n' \"$count\" > list-count\n\
             if [ \"$count\" -eq 2 ]; then\n  {mutation}\nfi\nprintf 'suite::known\\n'\n"
        ),
    )
    .expect("runner");
    dir
}

fn runner_rewrites_config(config_name: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(dir.path().join("src")).expect("src");
    std::fs::write(dir.path().join("src/a.rs"), "pub fn a() {}\n").expect("source");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: repairable\nphase: 1\nstatus: done\neffort_days: 1\n\
         paths: [src/**]\ntags: [cross-cutting]\nevidence:\n  - kind: test\n    ref: known\n\
         ---\n\nbody\n",
    )
    .expect("task");
    std::fs::write(
        dir.path().join("mutate-list.sh"),
        format!(
            "#!/bin/sh\nprintf '%s\\n' 'root: aplan' 'test:' \
             \x20 '  list: [printf, suite::different\\n]' \
             \x20 \x20 '  run: ['\"'\"'true'\"'\"', '\"'\"'{{test}}'\"'\"']' > {config_name}\n\
             printf 'suite::known\\n'\n"
        ),
    )
    .expect("runner");
    std::fs::write(
        dir.path().join(config_name),
        "root: aplan\ntest:\n  list: [sh, mutate-list.sh]\n  run: ['true', '{test}']\n",
    )
    .expect("config");
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "fixture"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }
    dir
}

const OLD: &str = "test:each_rail_appears_exactly_once";
const NEW: &str = "test:each_rail_appears_exactly_once_named_by_its_vector";
const COMPUTED: &str = "suite::each_rail_appears_exactly_once";

#[test]
fn the_pilots_case_is_repaired_and_verify_goes_green() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // The state the pilot was left in: a closed task that `verify` rejects.
    let (before, _, ok) = run(dir.path(), &["verify"]);
    assert!(!ok, "the fixture must start red: {before}");
    assert!(before.contains("APLAN013"), "{before}");

    for id in ["AL1-25", "AL1-26"] {
        let (stdout, stderr, ok) = run(dir.path(), &["amend", id, "--replace-evidence", OLD, NEW]);
        assert!(ok, "the repair must succeed: {stdout}{stderr}");
        // Both sides named, so the change is legible here and in the diff.
        assert!(
            stdout.contains(OLD) && stdout.contains(NEW),
            "it must name what it replaced and what with: {stdout}"
        );
    }

    let (after, _, ok) = run(dir.path(), &["verify"]);
    assert!(ok, "verify must be green afterwards: {after}");
    // The task stays closed. Reopening to repair would launder the closure.
    assert!(task(dir.path(), "AL1-25").contains("status: done"));
    // The untouched citation survives: this replaces one, not the block.
    assert!(task(dir.path(), "AL1-25").contains("ref: tests/t.rs"));
}

/// Proves the named behaviour `diagnostics.machine_applicable_repairs`: the
/// ordinary report exposes exact typed data, and the explicit flag applies all
/// of it as one mutation before verifying the state that now exists (`A1-199`).
#[test]
fn computed_repairs_are_reviewable_and_applied_as_one_guarded_batch() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (human, stderr, ok) = run(dir.path(), &["verify"]);
    assert!(!ok, "the fixture must start red: {human}{stderr}");
    assert_eq!(
        human.matches("repair: replace evidence").count(),
        2,
        "each computed edit remains reviewable even when prose fixes are folded: {human}"
    );
    assert!(human.contains("aplan verify --fix"), "{human}");

    let (json, stderr, ok) = run(dir.path(), &["verify", "--format", "json"]);
    assert!(
        !ok,
        "the machine report must retain the finding: {json}{stderr}"
    );
    let report: serde_json::Value = serde_json::from_str(&json).expect("report JSON");
    let repairs: Vec<&serde_json::Value> = report["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .filter_map(|d| d.get("repair"))
        .collect();
    assert_eq!(repairs.len(), 2, "{json}");
    for repair in repairs {
        assert_eq!(repair["operation"], "replace_evidence");
        assert_eq!(repair["from"]["kind"], "test");
        assert_eq!(repair["from"]["ref"], "each_rail_appears_exactly_once");
        assert_eq!(repair["to"]["kind"], "test");
        assert_eq!(repair["to"]["ref"], COMPUTED);
    }

    let (agent, stderr, ok) = run(dir.path(), &["verify", "--format", "agent"]);
    assert!(!ok, "the agent report lost the finding: {agent}{stderr}");
    let agent: serde_json::Value = serde_json::from_str(&agent).expect("agent JSON");
    let agent_repairs: Vec<&serde_json::Value> = agent["diagnostics"]
        .as_array()
        .expect("agent diagnostics")
        .iter()
        .filter_map(|diagnostic| diagnostic.get("repair"))
        .collect();
    assert_eq!(agent_repairs.len(), 2, "{agent}");
    assert!(
        agent_repairs
            .iter()
            .all(|repair| repair["operation"] == "replace_evidence"),
        "agent repairs are not typed: {agent}"
    );

    let (shown, stderr, ok) = run(
        dir.path(),
        &["show", "AL1-25", "--format", "json", "--refresh-evidence"],
    );
    assert!(ok, "show could not expose the repair: {shown}{stderr}");
    let shown: serde_json::Value = serde_json::from_str(&shown).expect("show JSON");
    let shown_repair = shown["findings"]
        .as_array()
        .expect("show findings")
        .iter()
        .find_map(|finding| finding.get("repair"))
        .expect("typed repair in show");
    assert_eq!(shown_repair["operation"], "replace_evidence");
    assert_eq!(shown_repair["to"]["ref"], COMPUTED);

    let (stdout, stderr, ok) = run(dir.path(), &["verify", "--fix"]);
    assert!(ok, "the guarded repair must finish green: {stdout}{stderr}");
    assert_eq!(stdout.matches("repaired ").count(), 2, "{stdout}");
    assert!(
        stdout.contains("ok: 2 tasks"),
        "post-state was not verified: {stdout}"
    );
    for id in ["AL1-25", "AL1-26"] {
        let repaired = task(dir.path(), id);
        assert!(repaired.contains("status: done"), "{repaired}");
        assert!(repaired.contains("ref: tests/t.rs"), "{repaired}");
        assert!(repaired.contains(&format!("ref: {COMPUTED}")), "{repaired}");
        assert!(!repaired.contains("ref: each_rail_appears_exactly_once\n"));
    }
}

/// Proves `close.refuses_structural_evidence_sequences` for a computed batch:
/// the same structural-source refusal as explicit `amend`. A safe repair
/// ordered before the structural task remains only a candidate in memory when
/// the later rewrite is refused, so no prefix reaches disk (`A1-717`).
#[test]
fn computed_repairs_refuse_structural_evidence_as_one_unwritten_batch() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let path = dir.path().join("aplan/tasks/AL1-26.md");
    let structural = task(dir.path(), "AL1-26")
        .replace("owner: t\n", "")
        .replacen(
            "ref: each_rail_appears_exactly_once",
            "ref: &shared each_rail_appears_exactly_once",
            1,
        )
        .replace(
            "    ref: tests/t.rs\n---",
            "    ref: tests/t.rs\nowner: *shared\n---",
        );
    std::fs::write(&path, structural).expect("structural repair task");

    let before: Vec<(String, String, aplan_core::Task)> = ["AL1-25", "AL1-26"]
        .into_iter()
        .map(|id| {
            let source = task(dir.path(), id);
            let (frontmatter, _) =
                aplan_core::load::split_frontmatter(&source).expect("frontmatter");
            let parsed = yaml_compat::from_str(frontmatter).expect("task loads before repair");
            (id.to_string(), source, parsed)
        })
        .collect();

    let (stdout, stderr, ok) = run(dir.path(), &["verify", "--fix"]);
    let said = format!("{stdout}{stderr}");
    assert!(!ok, "a structural batch was applied: {said}");
    assert!(said.contains("evidence"), "{said}");
    assert!(said.contains("simplify that field by hand"), "{said}");
    assert!(!said.contains("bug in aplan"), "{said}");

    for (id, original, parsed_before) in before {
        let after = task(dir.path(), &id);
        assert_eq!(after, original, "the batch partly wrote {id}");
        let (frontmatter, _) =
            aplan_core::load::split_frontmatter(&after).expect("frontmatter after refusal");
        let parsed_after: aplan_core::Task =
            yaml_compat::from_str(frontmatter).expect("task loads after refusal");
        assert_eq!(
            parsed_after, parsed_before,
            "the parsed task changed for {id}"
        );
    }
}

/// A candidate spelling is not machine-applicable when two runners claim it.
/// The useful suggestion remains prose, while an unrelated exact proposal in
/// the same report can still be reviewed and applied (`A1-458`,
/// `diagnostics.repair_requires_unique_resolver`).
#[test]
fn a_shared_near_match_is_not_published_as_a_typed_repair() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(dir.path().join("tests")).expect("tests");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ntest:\n  - name: libs\n    \
         list: [printf, 'suite::known_name\\nsuite::other_name\\n']\n    \
         run: ['true', '{test}']\n  - name: web\n    \
         list: [printf, 'suite::known_name\\n']\n    run: ['true', '{test}']\n",
    )
    .expect("config");
    for (id, cited) in [("T-ambiguous", "known_name"), ("T-exact", "other_name")] {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: near match\nphase: 1\nstatus: done\neffort_days: 1\n\
                 paths: [tests/**]\nevidence:\n  - kind: test\n    ref: {cited}\n---\n\nbody\n"
            ),
        )
        .expect("task");
    }
    std::fs::write(dir.path().join("tests/t.rs"), "fn main() {}\n").expect("test file");
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "fixture"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}");
    }

    let (json, stderr, ok) = run(dir.path(), &["verify", "--format", "json"]);
    assert!(
        !ok,
        "the misspellings unexpectedly resolved: {json}{stderr}"
    );
    let report: serde_json::Value = serde_json::from_str(&json).expect("report JSON");
    let diagnostics = report["diagnostics"].as_array().expect("diagnostics");
    let ambiguous = diagnostics
        .iter()
        .find(|finding| finding["task"] == "T-ambiguous")
        .expect("ambiguous finding");
    assert!(
        ambiguous["message"]
            .as_str()
            .is_some_and(|message| message.contains("suite::known_name")),
        "the suggestion disappeared: {ambiguous}"
    );
    assert!(
        ambiguous.get("repair").is_none(),
        "two runners supplied one spelling: {ambiguous}"
    );
    let exact = diagnostics
        .iter()
        .find(|finding| finding["task"] == "T-exact")
        .expect("exact finding");
    assert_eq!(exact["repair"]["to"]["ref"], "suite::other_name");

    let (stdout, stderr, ok) = run(dir.path(), &["verify", "--fix"]);
    assert!(
        !ok,
        "the unresolved runner ambiguity must remain visible: {stdout}{stderr}"
    );
    assert!(stdout.contains("repaired T-exact"), "{stdout}");
    assert!(stdout.contains("APLAN013"), "{stdout}");
    assert!(
        task(dir.path(), "T-ambiguous").contains("ref: known_name\n"),
        "the ambiguous proposal was applied"
    );
    assert!(
        task(dir.path(), "T-exact").contains("ref: suite::other_name\n"),
        "the independent exact repair was not applied"
    );
}

/// The other `A1-199` producer through the shipped command: a binary blob has
/// no anchor to resolve, and the exact edit removes only that anchor while
/// retaining the historical revision (`A1-461`).
#[test]
fn a_binary_file_anchor_is_repaired_end_to_end_without_losing_its_revision() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(dir.path().join("docs")).expect("docs");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.path().join("docs/scan.bin"), [0xff, 0xfe, 0x00, 0x01]).expect("binary");
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["add", ".aplan.yml", "docs/scan.bin"],
        vec!["commit", "-q", "-m", "binary"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }
    let rev = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    assert!(rev.status.success());
    let rev = String::from_utf8(rev.stdout)
        .expect("UTF-8 revision")
        .trim()
        .to_string();
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        format!(
            "---\nid: T-1\ntitle: binary evidence\nphase: 1\nstatus: done\neffort_days: 1\n\
             paths: [docs/**]\ntags: [cross-cutting]\nevidence:\n  - kind: file\n    \
             ref: docs/scan.bin#result@{rev}\n---\n\nbody\n"
        ),
    )
    .expect("task");
    for args in [
        vec!["add", "aplan/tasks/T-1.md"],
        vec!["commit", "-q", "-m", "task"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }

    let (before, stderr, ok) = run(dir.path(), &["verify"]);
    assert!(
        !ok,
        "the anchored binary unexpectedly resolved: {before}{stderr}"
    );
    assert!(before.contains("repair: replace evidence"), "{before}");

    let (fixed, stderr, ok) = run(dir.path(), &["verify", "--fix"]);
    assert!(ok, "the exact binary repair failed: {fixed}{stderr}");
    assert!(fixed.contains("repaired T-1"), "{fixed}");
    assert!(fixed.contains("ok: 1 tasks"), "{fixed}");
    let repaired = task(dir.path(), "T-1");
    assert!(repaired.contains("status: done"), "{repaired}");
    assert!(
        repaired.contains(&format!("ref: docs/scan.bin@{rev}\n")),
        "the revision was lost or changed: {repaired}"
    );
    assert!(!repaired.contains("#result"), "{repaired}");

    let (verified, stderr, ok) = run(dir.path(), &["verify"]);
    assert!(
        ok,
        "the repaired citation did not stay green: {verified}{stderr}"
    );
}

/// A computed evidence replacement is the same mutation as explicit `amend`,
/// including the freshness baseline a newly introduced mutable file citation
/// needs (`A1-196`).
#[test]
fn verify_fix_stamps_the_mutable_file_citation_it_repairs() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(dir.path().join("docs")).expect("docs");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndetect_drift: true\n",
    )
    .expect("config");
    std::fs::write(dir.path().join("docs/scan.bin"), [0xff, 0xfe, 0x00, 0x01]).expect("binary");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: mutable binary evidence\nphase: 1\nstatus: done\neffort_days: 1\n\
         paths: [docs/**]\ntags: [cross-cutting]\nevidence:\n  - kind: file\n    \
         ref: docs/scan.bin#result\n---\n\nbody\n",
    )
    .expect("task");
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "fixture"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }

    let (fixed, stderr, ok) = run(dir.path(), &["verify", "--fix"]);
    assert!(ok, "repair failed: {fixed}{stderr}");
    let repaired = task(dir.path(), "T-1");
    assert!(repaired.contains("ref: docs/scan.bin\n"), "{repaired}");
    assert!(repaired.contains("evidence_fingerprints:"), "{repaired}");
    assert!(repaired.contains("file:docs/scan.bin"), "{repaired}");

    std::fs::write(dir.path().join("docs/scan.bin"), [0xff, 0xfe, 0x00, 0x02]).expect("rewrite");
    for args in [
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "rewrite proof"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }
    let (verified, stderr, ok) = run(dir.path(), &["verify"]);
    assert!(ok, "a warning is advisory by default: {verified}{stderr}");
    assert!(verified.contains("APLAN054"), "{verified}{stderr}");
}

/// The typed edit is a proposal, not authority. If applying it reveals a
/// relevance failure, the same gate as `amend` refuses the whole batch and no
/// earlier repair leaks onto disk. This proves
/// `diagnostics.machine_applicable_repairs` fails closed (`A1-199`).
#[test]
fn a_computed_repair_the_evidence_gate_refuses_writes_none_of_the_batch() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::create_dir_all(dir.path().join("docs")).expect("docs");
    std::fs::create_dir_all(dir.path().join("src")).expect("src");
    std::fs::write(dir.path().join("docs/scan.bin"), [0xff, 0xfe, 0x00, 0x01])
        .expect("binary proof");
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn kept() {}\n").expect("owned source");
    std::fs::write(
        dir.path().join("aplan/tasks/AL1-27.md"),
        "---\nid: AL1-27\ntitle: binary proof outside the owned path\nphase: 1\nstatus: done\n\
         effort_days: 1\npaths:\n- src/**\nevidence:\n  - kind: file\n    ref: docs/scan.bin#result\n\
         ---\n\nbody\n",
    )
    .expect("task");
    for args in [
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "binary proof"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }
    let before: Vec<(String, String)> = ["AL1-25", "AL1-26", "AL1-27"]
        .into_iter()
        .map(|id| (id.to_string(), task(dir.path(), id)))
        .collect();

    let (stdout, stderr, ok) = run(dir.path(), &["verify", "--fix"]);
    assert!(
        !ok,
        "an irrelevant replacement was accepted: {stdout}{stderr}"
    );
    assert!(stdout.contains("APLAN031"), "{stdout}");
    assert!(stdout.contains("nothing was written"), "{stdout}");
    for (id, original) in before {
        assert_eq!(
            task(dir.path(), &id),
            original,
            "the batch partly wrote {id}"
        );
    }
}

/// A runner is external code and can race the task file while `verify` is
/// computing its report. The repair must be judged against the bytes it is
/// actually about to replace, not the stale task loaded before that command
/// ran; otherwise changed `paths` can make irrelevant evidence land and only
/// the post-write verification notices (`A1-199`, audit).
#[test]
fn a_computed_repair_rechecks_the_task_context_changed_by_its_runner() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(dir.path().join("src")).expect("src");
    std::fs::create_dir_all(dir.path().join("docs")).expect("docs");
    std::fs::write(dir.path().join("src/a.rs"), "pub fn a() {}\n").expect("source");
    std::fs::write(dir.path().join("docs/a.md"), "# Other ownership\n").expect("doc");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        concat!(
            "---\n",
            "id: T-1\n",
            "title: runner races the task\n",
            "phase: 1\n",
            "status: done\n",
            "effort_days: 1\n",
            "paths:\n",
            "- src/**\n",
            "tags: [cross-cutting]\n",
            "evidence:\n",
            "  - kind: test\n",
            "    ref: known\n",
            "---\n\n",
            "body\n",
        ),
    )
    .expect("task");
    std::fs::write(
        dir.path().join("mutate-list.sh"),
        "#!/bin/sh\nsed -i 's|- src/\\*\\*|- docs/**|' aplan/tasks/T-1.md\n\
         printf 'suite::known\\n'\n",
    )
    .expect("runner");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ntest:\n  list: [sh, mutate-list.sh]\n  run: ['true', '{test}']\n  \
         covers: [src/**]\n",
    )
    .expect("config");
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "fixture"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }

    let (stdout, stderr, ok) = run(dir.path(), &["verify", "--fix"]);
    assert!(!ok, "the stale task context was accepted: {stdout}{stderr}");
    assert!(stdout.contains("APLAN025"), "{stdout}");
    assert!(stdout.contains("nothing was written"), "{stdout}");
    let after = task(dir.path(), "T-1");
    assert!(
        after.contains("- docs/**"),
        "the runner's edit was lost: {after}"
    );
    assert!(
        after.contains("ref: known\n"),
        "the repair partly landed: {after}"
    );
    assert!(!after.contains("ref: suite::known"), "{after}");
}

/// Re-reading only the task that carries a repair is not a plan snapshot. A
/// configured runner can change a sibling before the mutation gate runs; the
/// candidate must include that state or the command writes first and discovers
/// the new finding only in its post-write verification (`A1-459`,
/// `verify_fix.post_runner_plan_is_gated`).
#[test]
fn a_computed_repair_gates_the_sibling_state_changed_by_its_runner() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(dir.path().join("src")).expect("src");
    std::fs::write(dir.path().join("src/a.rs"), "pub fn a() {}\n").expect("source");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: repairable\nphase: 1\nstatus: done\neffort_days: 1\n\
         paths: [src/**]\ntags: [cross-cutting]\nevidence:\n  - kind: test\n    ref: known\n\
         ---\n\nbody\n",
    )
    .expect("repairable task");
    std::fs::write(
        dir.path().join("aplan/tasks/T-2.md"),
        "---\nid: T-2\ntitle: runner changes this sibling\nphase: 1\nstatus: todo\n\
         effort_days: 1\npaths: [src/**]\ntags: [cross-cutting]\n---\n\nbody\n",
    )
    .expect("sibling task");
    std::fs::write(
        dir.path().join("mutate-list.sh"),
        "#!/bin/sh\nsed -i 's/status: todo/status: done/' aplan/tasks/T-2.md\n\
         printf 'suite::known\\n'\n",
    )
    .expect("runner");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ntest:\n  list: [sh, mutate-list.sh]\n  run: ['true', '{test}']\n",
    )
    .expect("config");
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "fixture"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }

    let (stdout, stderr, ok) = run(dir.path(), &["verify", "--fix"]);
    assert!(
        !ok,
        "the invalid sibling state was accepted: {stdout}{stderr}"
    );
    let said = format!("{stdout}{stderr}");
    assert!(said.contains("APLAN005"), "{said}");
    assert!(
        !said.contains("repaired T-1"),
        "the gate ran after the write: {said}"
    );
    let repaired = task(dir.path(), "T-1");
    assert!(
        repaired.contains("ref: known\n"),
        "the repair landed before the sibling was judged: {repaired}"
    );
    assert!(!repaired.contains("ref: suite::known"), "{repaired}");
}

/// The post-run snapshot is not a blanket concurrency refusal. A valid sibling
/// edit is part of the candidate, survives the repair, and does not make the
/// command write that sibling (`A1-459`,
/// `verify_fix.post_runner_plan_is_gated`).
#[test]
fn a_computed_repair_preserves_a_valid_sibling_edit_from_its_runner() {
    let dir = runner_mutates_sibling(
        "sed -i 's/runner changes this sibling/updated by runner/' aplan/tasks/T-2.md",
    );

    let (stdout, stderr, ok) = run(dir.path(), &["verify", "--fix"]);
    assert!(ok, "the valid combined state was refused: {stdout}{stderr}");
    assert!(stdout.contains("repaired T-1"), "{stdout}");
    assert!(
        task(dir.path(), "T-1").contains("ref: suite::known\n"),
        "the exact repair did not land"
    );
    assert!(
        task(dir.path(), "T-2").contains("title: updated by runner"),
        "the sibling edit was lost"
    );
}

/// A task that stops loading cannot be represented in a mutation candidate at
/// all. The reload therefore refuses before applying a repair instead of
/// dropping that task from the plan the gate sees (`A1-459`).
#[test]
fn a_computed_repair_refuses_a_sibling_its_runner_makes_unloadable() {
    let dir = runner_mutates_sibling(
        "printf '%s\\n' '---' 'id: T-2' 'title: [' '---' > aplan/tasks/T-2.md",
    );

    let (stdout, stderr, ok) = run(dir.path(), &["verify", "--fix"]);
    assert!(!ok, "the incomplete plan was accepted: {stdout}{stderr}");
    assert!(stdout.contains("no longer load"), "{stdout}{stderr}");
    assert!(stdout.contains("nothing was written"), "{stdout}{stderr}");
    assert!(
        task(dir.path(), "T-1").contains("ref: known\n"),
        "the repair landed before the load failure was judged"
    );
}

/// The selected configuration is part of the post-run snapshot too. A runner
/// that removes the route which resolved its suggested name cannot have the
/// edit accepted under the stale pre-run runner and rejected only after the
/// write (`A1-460`, `verify_fix.post_runner_config_is_gated`).
#[test]
fn a_computed_repair_reloads_configuration_changed_by_its_runner() {
    for config_name in [".aplan.yml", "policy.yml"] {
        let dir = runner_rewrites_config(config_name);
        let args: Vec<&str> = if config_name == ".aplan.yml" {
            vec!["verify", "--fix"]
        } else {
            vec![
                "--root",
                "aplan",
                "--config",
                config_name,
                "verify",
                "--fix",
            ]
        };
        let (stdout, stderr, ok) = run(dir.path(), &args);
        assert!(
            !ok,
            "the stale configuration was accepted: {stdout}{stderr}"
        );
        let said = format!("{stdout}{stderr}");
        assert!(said.contains("suite::known"), "{said}");
        assert!(
            !said.contains("repaired T-1"),
            "the current config was loaded only after the write: {said}"
        );
        let task = task(dir.path(), "T-1");
        assert!(task.contains("ref: known\n"), "{task}");
        assert!(!task.contains("ref: suite::known"), "{task}");
    }
}

/// `verify` discovers its policy from the repository it was asked to resolve.
/// The mutation half must not start a second discovery at the plan root: that
/// would make one invocation use two policies (`A1-462`,
/// `verify_fix.one_post_runner_config_source`).
#[test]
fn a_computed_repair_keeps_the_repositorys_configuration_through_its_mutation_gate() {
    let fixture = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let plan_repo = fixture.path().join("plan-repo");
    let evidence_repo = fixture.path().join("evidence-repo");
    let neutral = fixture.path().join("neutral");
    std::fs::create_dir_all(plan_repo.join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(plan_repo.join("src")).expect("source directory");
    std::fs::create_dir_all(&evidence_repo).expect("evidence repository");
    std::fs::create_dir_all(&neutral).expect("neutral working directory");

    // This file is deliberately invalid. It belongs to neither the resolver
    // policy discovered from `--repo` nor the subsequent post-verification.
    std::fs::write(plan_repo.join(".aplan.yml"), "not_a_config_key: true\n")
        .expect("hostile plan-side config");
    std::fs::write(plan_repo.join("src/a.rs"), "pub fn a() {}\n").expect("source");
    std::fs::write(
        plan_repo.join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: repairable\nphase: 1\nstatus: done\neffort_days: 1\n\
         paths: [src/**]\ntags: [cross-cutting]\nevidence:\n  - kind: test\n    ref: known\n\
         ---\n\nbody\n",
    )
    .expect("task");
    std::fs::write(
        evidence_repo.join(".aplan.yml"),
        "test:\n  list: [printf, 'suite::known\\n']\n  run: ['true', '{test}']\n",
    )
    .expect("repository policy");
    for repo in [&plan_repo, &evidence_repo] {
        common::git(repo, &["init", "-q", "."]);
        common::git(repo, &["config", "user.email", "t@example.com"]);
        common::git(repo, &["config", "user.name", "t"]);
        common::git(repo, &["add", "-A"]);
        common::git(repo, &["commit", "-q", "-m", "fixture"]);
    }

    let root = plan_repo.join("aplan").display().to_string();
    let repo = evidence_repo.display().to_string();
    let out = common::output(
        &neutral,
        &["--root", &root, "verify", "--repo", &repo, "--fix"],
    );
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "the policy root changed mid-command: {said}"
    );
    assert!(said.contains("repaired T-1"), "{said}");
    assert!(said.contains("ok: 1 tasks"), "{said}");
    let repaired = task(&plan_repo, "T-1");
    assert!(repaired.contains("ref: suite::known\n"), "{repaired}");
}

/// Replacement validation invokes the configured runner after the first
/// post-run snapshot. A sibling it invalidates on that later invocation must
/// still reach the mutation gate (`A1-463`,
/// `verify_fix.revalidation_side_effects_are_gated`).
#[test]
fn a_computed_repair_gates_a_sibling_changed_during_revalidation() {
    let dir = runner_mutates_sibling_on_second_list(
        "sed -i 's/status: todo/status: done/' aplan/tasks/T-2.md",
    );

    let (stdout, stderr, ok) = run(dir.path(), &["verify", "--fix"]);
    let said = format!("{stdout}{stderr}");
    assert!(!ok, "the later invalid sibling state was accepted: {said}");
    assert!(said.contains("APLAN005"), "{said}");
    assert!(
        !said.contains("repaired T-1"),
        "the later resolver change was gated only after writing: {said}"
    );
    assert!(task(dir.path(), "T-1").contains("ref: known\n"));
}

/// The later snapshot is complete-or-nothing: an unloadable sibling cannot
/// disappear from the candidate merely because there is no task value to put
/// into it (`A1-463`, `verify_fix.revalidation_side_effects_are_gated`).
#[test]
fn a_computed_repair_refuses_a_sibling_made_unloadable_during_revalidation() {
    let dir = runner_mutates_sibling_on_second_list(
        "printf '%s\\n' '---' 'id: T-2' 'title: [' '---' > aplan/tasks/T-2.md",
    );

    let (stdout, stderr, ok) = run(dir.path(), &["verify", "--fix"]);
    let said = format!("{stdout}{stderr}");
    assert!(!ok, "the incomplete later plan was accepted: {said}");
    assert!(said.contains("being resolved and no longer load"), "{said}");
    assert!(said.contains("nothing was written"), "{said}");
    assert!(!said.contains("repaired T-1"), "{said}");
    assert!(task(dir.path(), "T-1").contains("ref: known\n"));
}

/// A valid edit from the same later resolver remains on disk and the repaired
/// state verifies. Reloading is a snapshot, not an assertion that external
/// resolvers are read-only (`A1-463`).
#[test]
fn a_computed_repair_preserves_a_valid_sibling_change_during_revalidation() {
    let dir = runner_mutates_sibling_on_second_list(
        "sed -i 's/runner changes this sibling/updated on the second list/' aplan/tasks/T-2.md",
    );

    let (stdout, stderr, ok) = run(dir.path(), &["verify", "--fix"]);
    assert!(
        ok,
        "the valid post-resolution state was refused: {stdout}{stderr}"
    );
    assert!(stdout.contains("repaired T-1"), "{stdout}");
    assert!(
        task(dir.path(), "T-2").contains("title: updated on the second list"),
        "the later sibling edit was lost"
    );
}

/// A runner can also change the policy while `replacement_resolves` is using
/// it. That proposal was never checked by the new policy, so the safe result is
/// a refusal and a fresh run, not a write followed by a red post-verification
/// (`A1-463`, `verify_fix.revalidation_side_effects_are_gated`).
#[test]
fn a_computed_repair_refuses_configuration_changed_during_revalidation() {
    let dir = runner_mutates_sibling("");
    std::fs::write(
        dir.path().join("mutate-list.sh"),
        "#!/bin/sh\ncount=0\n[ ! -f list-count ] || count=$(cat list-count)\n\
         count=$((count + 1))\nprintf '%s\\n' \"$count\" > list-count\n\
         if [ \"$count\" -eq 2 ]; then\n\
           printf '%s\\n' 'root: aplan' 'test:' \\
             '  list: [printf, suite::different\\n]' \\
             '  run: ['\"'\"'true'\"'\"', '\"'\"'{test}'\"'\"']' > .aplan.yml\n\
         fi\nprintf 'suite::known\\n'\n",
    )
    .expect("runner");

    let (stdout, stderr, ok) = run(dir.path(), &["verify", "--fix"]);
    let said = format!("{stdout}{stderr}");
    assert!(!ok, "the changing policy was accepted: {said}");
    assert!(said.contains("configuration changed"), "{said}");
    assert!(said.contains("nothing was written"), "{said}");
    assert!(!said.contains("repaired T-1"), "{said}");
    assert!(task(dir.path(), "T-1").contains("ref: known\n"));
}

#[test]
fn a_repair_refuses_exactly_what_close_refuses() {
    // The anti-pattern this task set out not to build: an amend that accepts any
    // citation would be `A1-109`'s defect with a second door.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (stdout, _, ok) = run(
        dir.path(),
        &["amend", "AL1-25", "--replace-evidence", OLD, "test:ghost"],
    );
    assert!(!ok, "a citation the runner does not list must be refused");
    assert!(stdout.contains("APLAN013"), "{stdout}");
    // The refusal names the operation performing it. It said "refusing to close"
    // during an amend, which is the wrong-verb defect this project keeps finding.
    assert!(
        stdout.contains("refusing to amend"),
        "the refusal must name the operation: {stdout}"
    );
    assert!(
        task(dir.path(), "AL1-25").contains("ref: each_rail_appears_exactly_once\n"),
        "a refused repair must not have written anything"
    );
}

#[test]
fn a_repair_names_what_it_cannot_find_rather_than_guessing() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (_, stderr, ok) = run(
        dir.path(),
        &[
            "amend",
            "AL1-25",
            "--replace-evidence",
            "test:never_cited",
            NEW,
        ],
    );
    assert!(!ok, "a `from` that matches nothing is a mistake");
    assert!(stderr.contains("cites no `test:never_cited`"), "{stderr}");

    // Replacing with something already cited would collapse two citations into
    // one and silently drop proof.
    let (_, stderr, ok) = run(
        dir.path(),
        &[
            "amend",
            "AL1-25",
            "--replace-evidence",
            OLD,
            "file:tests/t.rs",
        ],
    );
    assert!(!ok, "a duplicate must be refused");
    assert!(stderr.contains("already cites"), "{stderr}");
}

#[test]
fn only_a_closed_task_is_repaired_and_the_message_says_so() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let path = dir.path().join("aplan/tasks/AL1-25.md");
    let live = std::fs::read_to_string(&path)
        .expect("read")
        .replace("status: done", "status: todo");
    std::fs::write(&path, live).expect("write");

    let (_, stderr, ok) = run(
        dir.path(),
        &["amend", "AL1-25", "--replace-evidence", OLD, NEW],
    );
    assert!(!ok, "a live task's evidence is `close`'s business");
    // The message named `--restamp`, a flag this command does not have, because
    // the error was written for the other repair.
    assert!(
        stderr.contains("only a `done` task can be repaired"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("--restamp"),
        "it must not name a flag this command has no notion of: {stderr}"
    );
}

#[test]
fn a_task_with_two_rotted_citations_can_be_repaired_one_at_a_time() {
    // Found dogfooding `A1-120` on this repository's own plan. The gate first
    // judged the whole task, so repairing one citation was refused because a
    // *different* citation was also rotten: the repair was blamed for the defect
    // it was not fixing. That is the pilot's own case -- p0000004 item 9 was
    // seven citations across two tasks -- and the "right about a citation and
    // wrong about the task" shape `p00025` item 14 names.
    //
    // A repair is answerable for what it introduces, which is `A1-113`'s delta
    // principle applied where it was needed first.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // A second rotted citation beside the first.
    let path = dir.path().join("aplan/tasks/AL1-25.md");
    let two = std::fs::read_to_string(&path).expect("read").replace(
        "  - kind: file\n",
        "  - kind: test\n    ref: also_renamed\n  - kind: file\n",
    );
    std::fs::write(&path, two).expect("write");

    let (stdout, stderr, ok) = run(
        dir.path(),
        &["amend", "AL1-25", "--replace-evidence", OLD, NEW],
    );
    assert!(
        ok,
        "a repair must not be refused for a citation it is not touching: {stdout}{stderr}"
    );
    assert!(
        task(dir.path(), "AL1-25").contains("ref: also_renamed"),
        "the other one is untouched"
    );

    // And the gate still refuses a replacement that is itself unresolvable, which
    // is the property the delta must not have thrown away.
    let (stdout, _, ok) = run(
        dir.path(),
        &[
            "amend",
            "AL1-25",
            "--replace-evidence",
            "test:also_renamed",
            "test:still_not_a_test",
        ],
    );
    assert!(!ok, "a bad replacement is still refused: {stdout}");
    assert!(stdout.contains("APLAN013"), "{stdout}");
}

#[test]
fn a_repair_is_exactly_as_strict_as_close_and_no_stricter() {
    // The criterion is "refuses exactly what `close` refuses". Measured in both
    // directions with `require_min_strength: [test]`: `amend` lets a repair
    // downgrade `test` to `file`, and `close` lets a task be closed on `file` in
    // the first place. Neither runs the hermetic rules, so `APLAN017` catches it
    // on the next `check`.
    //
    // That is the same boundary `A1-109` recorded: the evidence ladder is the
    // gate's business and the rule engine over a mutation's post-state is
    // `A1-113`'s. Written down so the gap is a decision with an owner rather
    // than something a later audit reports as new.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let config = std::fs::read_to_string(dir.path().join(".aplan.yml")).expect("read");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!("evidence:\n  require_min_strength: [test]\n{config}"),
    )
    .expect("config");

    // A weaker kind, one the task does not already cite, and *inside* its
    // `paths`: since `A1-045` a cited file outside them is refused by the ladder
    // (`APLAN031`), which would refuse this repair for a different reason than
    // the one under test and prove nothing about strength.
    std::fs::write(dir.path().join("tests/other.rs"), "fn other() {}\n").expect("test file");
    for args in [
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "another test file"],
    ] {
        std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git runs");
    }
    let repair = [
        "amend",
        "AL1-26",
        "--replace-evidence",
        OLD,
        "file:tests/other.rs",
    ];

    // The repair itself still does not judge strength: `APLAN017` is not a
    // citation that failed to resolve. What refuses is `A1-113`'s post-state
    // check, which asks the rules about the plan this would produce, and it
    // names the rule.
    let (stdout, stderr, ok) = run(dir.path(), &repair);
    assert!(!ok, "a plan `check` rejects was written: {stdout}{stderr}");
    assert!(stderr.contains("APLAN017"), "{stderr}");

    // And the escape is the one `A1-109` chose, which says what it skipped.
    let mut forced = repair.to_vec();
    forced.push("--no-resolve");
    let (stdout, _, ok) = run(dir.path(), &forced);
    assert!(ok, "{stdout}");
    assert!(stdout.contains("were not checked"), "{stdout}");
    assert!(stdout.contains("APLAN017"), "{stdout}");

    let (check, _, ok) = run(dir.path(), &["check"]);
    assert!(!ok, "and `check` objects, as it always did: {check}");
    assert!(check.contains("APLAN017"), "{check}");
}

#[test]
fn a_citation_can_be_added_to_proof_that_was_never_whole() {
    // The other half of the repair surface (`A1-123`). `--replace-evidence` fixes
    // proof that rotted; this completes proof that was incomplete, which is the
    // shape 52 of this plan's 90 closed tasks were in: the tests were written and
    // never cited, so a fix's machine-checkable proof was a commit.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (stdout, stderr, ok) = run(dir.path(), &["amend", "AL1-25", "--add-evidence", NEW]);
    assert!(ok, "the addition must succeed: {stdout}{stderr}");
    assert!(stdout.contains(NEW), "it must name what it added: {stdout}");
    let file = task(dir.path(), "AL1-25");
    // The file stores the citation as YAML, not in the `kind:ref` form the
    // command takes.
    assert!(
        file.contains("ref: each_rail_appears_exactly_once_named_by_its_vector"),
        "the new citation is recorded: {file}"
    );
    // Additive: nothing already recorded is disturbed.
    assert!(file.contains("ref: each_rail_appears_exactly_once\n"));
    assert!(file.contains("ref: tests/t.rs"));

    // Gated exactly as a replacement is: this is not a door round `A1-109`.
    let (stdout, _, ok) = run(
        dir.path(),
        &["amend", "AL1-25", "--add-evidence", "test:not_a_real_test"],
    );
    assert!(!ok, "an unresolvable addition must be refused: {stdout}");
    assert!(stdout.contains("APLAN013"), "{stdout}");

    // And adding what is already cited is refused rather than growing the block.
    let (_, stderr, ok) = run(dir.path(), &["amend", "AL1-25", "--add-evidence", NEW]);
    assert!(!ok, "a duplicate must be refused");
    assert!(stderr.contains("already cites"), "{stderr}");
}

#[test]
fn a_repair_needs_exactly_one_of_the_three_shapes() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // Neither: there is no default repair, because guessing which one was meant is
    // the kind of guess this command exists not to make.
    let (_, stderr, ok) = run(dir.path(), &["amend", "AL1-25"]);
    assert!(!ok, "one of the two is required");
    assert!(
        stderr.contains("--replace-evidence") || stderr.contains("--add-evidence"),
        "{stderr}"
    );
    // Both at once is ambiguous about what the diff should show.
    let (_, stderr, ok) = run(
        dir.path(),
        &[
            "amend",
            "AL1-25",
            "--replace-evidence",
            OLD,
            NEW,
            "--add-evidence",
            "file:.aplan.yml",
        ],
    );
    assert!(!ok, "both at once must be refused: {stderr}");
}

#[test]
fn the_flag_given_twice_is_refused_rather_than_half_applied() {
    // `--replace-evidence` is `num_args = 2`, and clap allows the flag more than
    // once, so four citations reach the command. Naming both sides is the whole
    // safety property, and "which two did you mean" has no good guess.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (_, stderr, ok) = run(
        dir.path(),
        &[
            "amend",
            "AL1-25",
            "--replace-evidence",
            OLD,
            NEW,
            "--replace-evidence",
            "file:tests/t.rs",
            "file:tests/other.rs",
        ],
    );
    assert!(!ok, "two repairs at once must be refused");
    assert!(stderr.contains("exactly two citations"), "{stderr}");
    assert!(
        task(dir.path(), "AL1-25").contains("ref: each_rail_appears_exactly_once\n"),
        "nothing may be written"
    );
}

#[test]
fn a_repair_does_not_touch_trace_fingerprints() {
    // Repairing a citation is not re-reviewing a traced section. Doing both here
    // would let an amendment silently clear a drift warning it never looked at,
    // so `close --restamp` stays the other repair.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let path = dir.path().join("aplan/tasks/AL1-25.md");
    let with_stamp = std::fs::read_to_string(&path).expect("read").replace(
        "---\n\nbody",
        "trace_fingerprints:\n  DOC.md#x: abc123\n---\n\nbody",
    );
    std::fs::write(&path, with_stamp).expect("write");

    let (_, _, ok) = run(
        dir.path(),
        &[
            "amend",
            "AL1-25",
            "--replace-evidence",
            OLD,
            NEW,
            "--no-resolve",
        ],
    );
    assert!(ok);
    assert!(
        task(dir.path(), "AL1-25").contains("DOC.md#x: abc123"),
        "the fingerprint must survive the repair untouched"
    );
}

#[test]
fn no_resolve_records_the_repair_and_says_that_it_skipped_the_check() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (stdout, _, ok) = run(
        dir.path(),
        &[
            "amend",
            "AL1-25",
            "--replace-evidence",
            OLD,
            "test:ghost",
            "--no-resolve",
        ],
    );
    assert!(ok, "the escape hatch must work: {stdout}");
    assert!(task(dir.path(), "AL1-25").contains("ref: ghost"));
    assert!(
        stdout.contains("without being resolved"),
        "a repair that skipped its check must not read like one that passed: {stdout}"
    );
}

// --- a repair says what it could not check (`A1-238`) ------------------------

/// A closed task in a directory that is deliberately **not** a repository, and
/// with nothing configured to run a test.
///
/// Both halves are the point: with no repository a `commit` citation cannot be
/// resolved, and with no runner a `test` one cannot either. What the tool does
/// then is the question.
fn plan_nothing_can_check(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.join("aplan/tasks/T-2.md"),
        "---\nid: T-2\ntitle: closed\nphase: 0\nstatus: done\nowner: k\neffort_days: 1\n\
         paths:\n- f.txt\nevidence:\n  - kind: commit\n    ref: \"1111111\"\n---\n\nbody\n",
    )
    .expect("task");
}

const UNRESOLVED: &str = "evidence was recorded unresolved";
const NO_REPOSITORY: &str = "there is no repository here to resolve it against";
const NOTHING_CONFIGURED: &str = "nothing is configured to check it";

/// **A repair says what it could not check, in the words `close` uses.**
///
/// `close` ends with that note and explains why beside it: silence about a check
/// nothing ran reads exactly like a check that passed. `amend` ran the same
/// resolution and printed no such line, so outside a repository it recorded a
/// citation `verify` rejects and said nothing at all -- a repair recording a
/// claim *because* it could not check it, in the tool whose argument is that a
/// status is a claim and evidence is the proof (`A1-238`, found by `A1-208`
/// while exercising `--repo`).
#[test]
fn a_repair_that_nothing_could_check_says_so() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan_nothing_can_check(dir.path());

    let (out, _, ok) = run(
        dir.path(),
        &["amend", "T-2", "--add-evidence", "commit:8888888"],
    );
    assert!(ok, "the repair was refused: {out}");
    assert!(
        out.contains(UNRESOLVED) && out.contains("commit"),
        "the repair recorded a citation nothing checked and did not say so:\n{out}"
    );
    // **The cause a reader can act on.** A missing `test` runner is a config
    // question and standing outside a repository is not; both reached the
    // surface as one empty result, and the sentence named the wrong one of them
    // here (`A1-238`, audit).
    assert!(
        out.contains(NO_REPOSITORY),
        "it blamed configuration for there being no repository:\n{out}"
    );
    assert!(
        !out.contains(NOTHING_CONFIGURED),
        "both causes were claimed at once:\n{out}"
    );
    assert!(
        out.contains("amended T-2: + commit:8888888"),
        "it said what it could not check and not what it did:\n{out}"
    );

    // The other mode, which had the same silence.
    let (out, _, ok) = run(
        dir.path(),
        &[
            "amend",
            "T-2",
            "--replace-evidence",
            "commit:8888888",
            "commit:7777777",
        ],
    );
    assert!(ok, "{out}");
    assert!(
        out.contains(UNRESOLVED),
        "`--replace-evidence` was silent:\n{out}"
    );
}

/// **A repair that only removes says nothing**, because it introduces nothing.
///
/// The note is keyed on the citations a repair *adds*, not on a delta of the
/// unchecked kinds: a task already carrying an unchecked `commit` would absorb a
/// second one in silence under the coarser reading, which is the case that found
/// this.
#[test]
fn a_repair_that_only_removes_evidence_reports_nothing_unchecked() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan_nothing_can_check(dir.path());
    // Two citations, so removing one leaves the task proven.
    let (out, _, ok) = run(
        dir.path(),
        &["amend", "T-2", "--add-evidence", "commit:2222222"],
    );
    assert!(ok, "{out}");

    let (out, _, ok) = run(
        dir.path(),
        &["amend", "T-2", "--remove-evidence", "commit:2222222"],
    );
    assert!(ok, "{out}");
    assert!(
        !out.contains(UNRESOLVED),
        "a removal reported something unchecked:\n{out}"
    );
}

/// And where a repository *can* answer, it still decides: the citation that does
/// not resolve in the named `--repo` is refused, the one that does is written,
/// and neither is called unchecked.
#[test]
fn a_named_repo_answers_and_the_note_is_not_printed() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan_nothing_can_check(dir.path());
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");
    std::fs::write(elsewhere.join("f.txt"), "x\n").expect("file");
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "one"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(&elsewhere)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }
    let sha = String::from_utf8_lossy(
        &std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(&elsewhere)
            .output()
            .expect("git runs")
            .stdout,
    )
    .trim()
    .to_string();
    let named = elsewhere.display().to_string();

    let (out, _, ok) = run(
        dir.path(),
        &[
            "amend",
            "T-2",
            "--add-evidence",
            "commit:1234567",
            "--repo",
            &named,
        ],
    );
    assert!(
        !ok,
        "a commit absent from the named repository was accepted:\n{out}"
    );
    assert!(out.contains("APLAN011"), "{out}");

    let citation = format!("commit:{sha}");
    let (out, _, ok) = run(
        dir.path(),
        &[
            "amend",
            "T-2",
            "--add-evidence",
            &citation,
            "--repo",
            &named,
        ],
    );
    assert!(ok, "a commit that exists was refused:\n{out}");
    assert!(
        !out.contains(UNRESOLVED),
        "a citation the named repository resolved was called unchecked:\n{out}"
    );
}

/// **A refused repair says nothing about recording**, because nothing was.
///
/// The note above sat before the refusal in its first version, so a repair that
/// was about to be turned down announced that evidence "was recorded
/// unresolved" and then recorded none of it. `close` puts the same line after
/// its refusal and says why: a message may name only what happened (`A1-091`).
///
/// Reachable whenever the added citation is both uncheckable and enough to wake
/// a rule. A `test` citation is exactly that for `APLAN019`, which is silent
/// about a dead `paths` glob until the task cites something that would bind
/// through it (`A1-238`, audit).
#[test]
fn a_refused_repair_does_not_claim_it_recorded_anything() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/T-2.md"),
        "---\nid: T-2\ntitle: closed on a dead glob\nphase: 0\nstatus: done\nowner: k\n\
         effort_days: 1\npaths:\n- nope/**\nevidence:\n  - kind: note\n    ref: reviewed\n    \
         captured: 2026-08-14\n---\n\nbody\n",
    )
    .expect("task");
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "one"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }

    // Nothing runs a test here, so the citation is uncheckable *and* it wakes
    // `APLAN019` about the dead glob. Both halves are needed, or this passes
    // for the wrong reason.
    let (out, _, ok) = run(dir.path(), &["amend", "T-2", "--add-evidence", "test:foo"]);
    assert!(
        !ok,
        "the repair was accepted, so there is no refusal to order:\n{out}"
    );
    assert!(out.contains("APLAN019"), "the rule did not fire:\n{out}");
    assert!(
        out.contains("refusing to amend"),
        "it did not refuse:\n{out}"
    );
    assert!(
        !out.contains(UNRESOLVED),
        "a refused repair claimed evidence was recorded:\n{out}"
    );
    assert!(
        !task(dir.path(), "T-2").contains("test:foo") && !task(dir.path(), "T-2").contains("foo"),
        "and it wrote the citation it refused"
    );
}

/// The other cause, in the same words' place: inside a repository with no test
/// runner, a `test` citation is unchecked because nothing is configured to run
/// it -- which is a question about `.aplan.yml`, not about git.
#[test]
fn the_note_names_a_missing_runner_as_a_missing_runner() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // The fixture configures a runner; take it away, leaving the repository.
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");

    let (out, _, ok) = run(
        dir.path(),
        &["amend", "AL1-25", "--add-evidence", "test:some::other"],
    );
    assert!(ok, "{out}");
    assert!(
        out.contains(NOTHING_CONFIGURED),
        "a missing runner was reported as a missing repository:\n{out}"
    );
    assert!(
        !out.contains(NO_REPOSITORY),
        "there is a repository here:\n{out}"
    );
}
