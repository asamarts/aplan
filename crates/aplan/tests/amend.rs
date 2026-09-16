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

use std::path::Path;

mod common;

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
         list: [\"printf\", \"each_rail_appears_exactly_once_named_by_its_vector\\n\"]\n  \
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

const OLD: &str = "test:each_rail_appears_exactly_once";
const NEW: &str = "test:each_rail_appears_exactly_once_named_by_its_vector";

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
