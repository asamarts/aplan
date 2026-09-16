//! `close` refuses evidence `verify` would reject (`A1-109`).
//!
//! The first pilot closed a task citing three integration tests, was told
//! `closed`, and `verify` rejected all three seconds later: the citations used
//! `guider::name` where the runner lists them bare
//! (pre-public adoption evidence, item 10). **The tool contradicted itself about its
//! own central claim**, and the task sat `done` on evidence the project's own
//! gate rejects until a separate command was run.
//!
//! Driven through the built binary rather than through `resolve_proposed`, for
//! `A1-107`'s reason: a unit test on the resolver proves the resolver. If `close`
//! stopped calling it tomorrow, every such test would still pass. The runner here
//! is a shell script rather than Cargo, which also makes these the
//! non-Cargo-runner exercise `A1-034` asks for, in passing.
//!
//! Not a `trycmd` case: closing writes, and the corpus runs in its own fixture
//! directory with no sandbox, so a writing case edits the fixture it is pinned
//! against.

use std::path::Path;

mod common;

/// A git repository whose runner lists exactly one test, bare, and whose task
/// owns `tests/**`.
///
/// The listed name is the pilot's, so the fixture reproduces the mistake that was
/// actually made rather than a shorter one invented for the test.
fn fixture(dir: &Path) -> String {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("tests")).expect("mkdir tests");
    std::fs::write(
        dir.join(".aplan.yml"),
        "root: aplan\ntest:\n  list: [\"sh\", \"-c\", \
         \"echo a_settledone_that_overtakes_the_reply_is_not_lost\"]\n  \
         run: [\"sh\", \"-c\", \"true\", \"{test}\"]\n",
    )
    .expect("config");
    std::fs::write(
        dir.join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: a task\nphase: 1\nstatus: todo\nowner: t\n\
         effort_days: 1\npaths:\n- tests/**\n---\n\nbody\n",
    )
    .expect("task");
    std::fs::write(dir.join("tests/t.rs"), "fn main() {}\n").expect("test file");
    git(dir, &["init", "-q", "."]);
    git(dir, &["config", "user.email", "t@example.com"]);
    git(dir, &["config", "user.name", "t"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "initial"]);
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .expect("git rev-parse");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?} failed");
}

fn run(dir: &Path, args: &[&str]) -> (String, String, bool) {
    let out = common::output(dir, args);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

fn status_of(dir: &Path) -> String {
    let text = std::fs::read_to_string(dir.join("aplan/tasks/A-1.md")).expect("read");
    text.lines()
        .find(|l| l.starts_with("status:"))
        .expect("a status line")
        .to_string()
}

/// `APLAN019` got quieter and the close gate did not (`A1-138`).
///
/// The rule now fires only where a citation is judged against the task's paths,
/// which silences the 140 findings the fourth pilot met on `todo` tasks whose
/// globs name a tree a later cut creates. The moment a citation *appears* is the
/// moment the glob starts mattering, and this is that moment: closing a task
/// whose `paths` reach nothing is still refused, with the same message the pilot
/// quoted approvingly.
#[test]
fn closing_a_task_whose_paths_bind_nothing_is_still_refused() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let sha = fixture(dir.path());
    // A forward-looking glob, of exactly the shape the rule stopped reporting.
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: a task\nphase: 1\nstatus: todo\nowner: t\n\
         effort_days: 1\npaths:\n- crates/not-yet/**\n---\n\nbody\n",
    )
    .expect("task");

    // Silent while nothing is cited: this is what p00020 needed.
    let (_, err, ok) = run(dir.path(), &["verify"]);
    assert!(
        ok,
        "a forward-looking path is not a finding on its own: {err}"
    );

    // And refused the moment a citation would bind to it.
    let (out, err, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", &format!("commit:{sha}")],
    );
    let said = format!("{out}{err}");
    assert!(!ok, "the closure was accepted: {said}");
    assert!(said.contains("matches no tracked file"), "{said}");
    assert!(
        said.contains("binds to nothing through it"),
        "the message names what is unbound: {said}"
    );
    assert_eq!(status_of(dir.path()), "status: todo", "it wrote anyway");
}

#[test]
fn the_pilots_citation_is_refused_and_the_right_form_is_named() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (stdout, _, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:guider::a_settledone_that_overtakes_the_reply_is_not_lost",
        ],
    );

    assert!(!ok, "the close must fail; it printed: {stdout}");
    assert!(
        stdout.contains("APLAN013"),
        "the refusal must cite the rule that would have rejected it later: {stdout}"
    );
    // The near match is the whole difference between a typo and a dead end.
    assert!(
        stdout.contains("but `a_settledone_that_overtakes_the_reply_is_not_lost` is"),
        "the refusal must name what the runner does list: {stdout}"
    );
    assert!(
        stdout.contains("--no-resolve"),
        "a refusal must say how to proceed deliberately: {stdout}"
    );
    // The point of refusing before writing.
    assert_eq!(
        status_of(dir.path()),
        "status: todo",
        "a refused close must not have written anything"
    );
}

#[test]
fn the_citation_the_runner_lists_closes() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (stdout, stderr, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:a_settledone_that_overtakes_the_reply_is_not_lost",
        ],
    );

    assert!(ok, "a resolvable citation must close: {stdout}{stderr}");
    assert_eq!(status_of(dir.path()), "status: done");
    // A gate that refused everything would pass the test above and be useless.
    assert!(
        !stdout.contains("APLAN013"),
        "a good citation must not be reported: {stdout}"
    );
}

/// Proves the named behaviour `close.evidence_resolves_before_write` (`A1-211`).
#[test]
fn no_resolve_records_it_and_says_that_it_did() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (stdout, _, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:guider::a_settledone_that_overtakes_the_reply_is_not_lost",
            "--no-resolve",
        ],
    );

    assert!(ok, "the escape hatch must work: {stdout}");
    assert_eq!(status_of(dir.path()), "status: done");
    // A closure that skipped its checks must not read like one that passed.
    assert!(
        stdout.contains("--no-resolve") && stdout.contains("without being resolved"),
        "the escape hatch must say what it skipped: {stdout}"
    );
}

/// A stale sibling glob does not stop a close whose citation is bound
/// (`A1-141`).
///
/// `close` refuses what `verify` would fail, so under `deny_warnings` the fourth
/// pilot could not close a task whose evidence *was* bound because a second glob
/// named a directory the task had not created yet. 28 of its tasks are that
/// shape, including all twelve of the cut it was starting
/// (pre-public adoption evidence item 26).
/// Proves the named behaviour `paths.unmatched_ignores_a_bound_sibling` (`A1-211`).
#[test]
fn a_dead_sibling_glob_does_not_refuse_a_bound_citation() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // The pilot's policy, which is what turns the warning into a refusal.
    let config = std::fs::read_to_string(dir.path().join(".aplan.yml")).expect("read");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!("deny_warnings: true\n{config}"),
    )
    .expect("config");
    // `tests/**` is live; `fixtures/**` is a directory this task has not made.
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: a task\nphase: 1\nstatus: todo\nowner: t\n\
         effort_days: 1\npaths:\n- tests/**\n- fixtures/**\n---\n\nbody\n",
    )
    .expect("task");

    let (out, err, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", "file:tests/t.rs"],
    );
    assert!(
        ok,
        "a bound citation was refused for a stale sibling: {out}{err}"
    );
    assert_eq!(status_of(dir.path()), "status: done");

    // And the case the rule is for still refuses: with no live glob at all,
    // nothing the task cites binds.
    std::fs::write(
        dir.path().join("aplan/tasks/A-2.md"),
        "---\nid: A-2\ntitle: another\nphase: 1\nstatus: todo\nowner: t\n\
         effort_days: 1\npaths:\n- fixtures/**\n---\n\nbody\n",
    )
    .expect("task");
    let (out, err, ok) = run(
        dir.path(),
        &["close", "A-2", "--evidence", "file:tests/t.rs"],
    );
    let said = format!("{out}{err}");
    assert!(!ok, "{said}");
    assert!(
        said.contains("APLAN019") || said.contains("APLAN031"),
        "{said}"
    );
}

/// The same refusal for the other kind that names an artefact (`A1-045`).
///
/// The fourth pilot closed a task about ADR authorship with a Containerfile, in
/// one command, and `check` and `verify` said `ok` afterwards. The commit
/// citation for the same work was refused by `APLAN012`; the file citation was
/// not, and its conventions prefer `file:` precisely because a large commit
/// names a Tuesday (pre-public adoption evidence item 25).
#[test]
fn a_file_outside_the_tasks_paths_is_refused_at_the_same_moment() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // The fixture's task owns `tests/**`; this file exists and is not its.
    std::fs::write(dir.path().join("elsewhere.md"), "hi\n").expect("write");
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &["commit", "-q", "-m", "a file the task does not own"],
    );

    let (out, err, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", "file:elsewhere.md"],
    );
    let said = format!("{out}{err}");
    assert!(!ok, "the pilot's closure was accepted: {said}");
    assert!(said.contains("APLAN031"), "{said}");
    assert!(said.contains("outside this task's"), "{said}");
    assert_eq!(status_of(dir.path()), "status: todo", "it wrote anyway");

    // And the citation the task does own closes it, so the refusal is about
    // relevance rather than about `file:` evidence.
    let (out, err, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", "file:tests/t.rs"],
    );
    assert!(ok, "{out}{err}");
    assert_eq!(status_of(dir.path()), "status: done");
}

#[test]
fn a_commit_that_touches_none_of_the_tasks_paths_is_refused() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(dir.path().join("NOTES.md"), "unrelated\n").expect("notes");
    git(dir.path(), &["add", "NOTES.md"]);
    git(dir.path(), &["commit", "-q", "-m", "unrelated work"]);
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git");
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();

    let (stdout, _, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", &format!("commit:{sha}")],
    );

    // `APLAN012` is the rung that made `paths` load-bearing; a close that ignored
    // it would certify a commit about something else.
    assert!(!ok, "an irrelevant commit must be refused: {stdout}");
    assert!(stdout.contains("APLAN012"), "{stdout}");
    assert_eq!(status_of(dir.path()), "status: todo");
}

#[test]
fn without_a_runner_a_test_citation_is_recorded_as_unchecked_not_as_passed() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // Take the runner away; `test` evidence now has no L1 at all.
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");

    let (stdout, _, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", "test:nothing_lists_this"],
    );

    assert!(
        ok,
        "a repository with no runner must still be able to close: {stdout}"
    );
    assert_eq!(status_of(dir.path()), "status: done");
    // The criterion this task turns on: silence about a check nothing ran reads
    // exactly like a check that passed, which is the defect one level up.
    assert!(
        stdout.contains("unresolved") && stdout.contains("test"),
        "it must say the citation was not checked: {stdout}"
    );
}

#[test]
fn an_already_closed_task_is_told_that_and_not_something_true_but_irrelevant() {
    // Found auditing `A1-109`. The gate ran before the structural refusals, so
    // closing an already-`done` task spawned the runner and answered "this
    // evidence would fail `aplan verify`" -- true, and not the reason. The reason
    // is that the task is closed, and a tool that reports a real problem in place
    // of the actual one sends the reader to fix the wrong thing.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let (_, _, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:a_settledone_that_overtakes_the_reply_is_not_lost",
        ],
    );
    assert!(ok, "the first close must succeed");

    let (stdout, stderr, ok) = run(dir.path(), &["close", "A-1", "--evidence", "test:ghost"]);
    let out = format!("{stdout}{stderr}");
    assert!(!ok, "closing a closed task must fail: {out}");
    assert!(
        out.contains("already `done`"),
        "it must give the structural reason: {out}"
    );
    assert!(
        !out.contains("APLAN013"),
        "and must not report the citation instead, which is true but not why: {out}"
    );
}

#[test]
fn a_refusal_does_not_claim_the_evidence_was_recorded() {
    // Also found auditing `A1-109`. The unchecked-kinds note printed before the
    // refusal, so a refused close said `url` evidence "was recorded unresolved"
    // while recording nothing. `A1-091`: a message may name only what happened.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (stdout, _, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:ghost",
            "--evidence",
            "url:https://example.com",
        ],
    );
    assert!(!ok, "the close must be refused: {stdout}");
    assert!(
        !stdout.contains("was recorded"),
        "a refusal must not claim anything was recorded: {stdout}"
    );
    assert_eq!(status_of(dir.path()), "status: todo");

    // The note is still owed when the closure goes ahead, or the fix would have
    // traded a false statement for a missing one.
    let (stdout, _, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:a_settledone_that_overtakes_the_reply_is_not_lost",
            "--evidence",
            "url:https://example.com",
        ],
    );
    assert!(ok, "{stdout}");
    assert!(
        stdout.contains("url evidence was recorded unresolved"),
        "a successful close must still name what nothing checked: {stdout}"
    );
}

#[test]
fn deny_warnings_decides_whether_a_ladder_warning_refuses() {
    // The design decision this task turns on: `close` refuses exactly what would
    // fail the build, by reusing `Report::denied`, rather than inventing a second
    // severity policy for closing. Proved both ways, because a gate that always
    // refused and a gate that never did would each pass one half of this.
    //
    // `APLAN025` is the warning used because it is on the `verify` ladder.
    // Hermetic warnings like `APLAN023` are deliberately *not* checked here: the
    // rule engine over a mutation's post-state is `A1-113`'s, and this task owns
    // the evidence ladder. Stated so the boundary is a decision rather than a gap
    // nobody noticed.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::create_dir_all(dir.path().join("other")).expect("mkdir");
    std::fs::write(dir.path().join("other/o.rs"), "fn main() {}\n").expect("other");
    // Written line by line: a wrapped string literal here once produced fifteen
    // spaces of indent for two keys and two for a third, which is invalid YAML,
    // and the test failed with an empty stdout rather than a wrong verdict.
    let config = |deny: bool| {
        [
            "root: aplan".to_string(),
            format!("deny_warnings: {deny}"),
            "test:".to_string(),
            "  list: [\"printf\", \"a_settledone_that_overtakes_the_reply_is_not_lost\\n\"]"
                .to_string(),
            "  run: [\"true\", \"{test}\"]".to_string(),
            "  covers: [\"other/**\"]".to_string(),
        ]
        .join("\n")
    };
    let cite = [
        "close",
        "A-1",
        "--evidence",
        "test:a_settledone_that_overtakes_the_reply_is_not_lost",
    ];

    std::fs::write(dir.path().join(".aplan.yml"), config(true)).expect("config");
    let (stdout, _, ok) = run(dir.path(), &cite);
    assert!(!ok, "with deny_warnings, a warning must refuse: {stdout}");
    assert!(stdout.contains("APLAN025"), "{stdout}");
    assert_eq!(status_of(dir.path()), "status: todo");

    std::fs::write(dir.path().join(".aplan.yml"), config(false)).expect("config");
    let (stdout, _, ok) = run(dir.path(), &cite);
    assert!(ok, "without it, the same warning must not refuse: {stdout}");
    assert!(
        stdout.contains("APLAN025"),
        "it must still be reported, not hidden: {stdout}"
    );
    assert_eq!(status_of(dir.path()), "status: done");
}

#[test]
fn a_plan_outside_a_repository_still_closes_and_says_what_was_skipped() {
    // `close` was hermetic before this task, and a plan kept outside git is a
    // real arrangement. Demanding a repository to close would be a regression
    // dressed as a gate.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: a task\nphase: 1\nstatus: todo\nowner: t\n\
         effort_days: 1\n---\n\nbody\n",
    )
    .expect("task");

    let (stdout, _, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", "commit:0123456789abcdef"],
    );

    assert!(ok, "no repository must not mean no closing: {stdout}");
    assert!(
        stdout.contains("unresolved"),
        "it must say the citation went unchecked: {stdout}"
    );
}
