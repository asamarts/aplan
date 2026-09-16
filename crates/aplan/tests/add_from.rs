//! A written plan becomes tasks in one change (`A1-241`).
//!
//! Two pilots arrived with a document and no way in. `p00016` turned a
//! 1,400-line research document into 153 tasks, 11 cuts and 3 releases with a
//! generator that emitted frontmatter directly, which makes the plan's own file
//! format a second implementation nothing gates (pre-public adoption evidence item
//! 3). `p00002` shelled out to `aplan add` 64 times and every task landed, which
//! is the same finding without the damage (pre-public adoption evidence item 2).
//!
//! The tests below are about the two properties that make this `add` rather than
//! an importer: an entry is resolved against what the entries before it produce,
//! and nothing is written unless all of it can be.

use std::path::Path;
use std::process::Command;

mod common;

fn repo() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("add-from-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    let out = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .output()
        .expect("git");
    assert!(out.status.success());
    dir
}

fn config(dir: &Path, text: &str) {
    std::fs::write(dir.join(".aplan.yml"), text).expect("config");
}

fn manifest(dir: &Path, text: &str) -> String {
    std::fs::write(dir.join("plan.yml"), text).expect("manifest");
    "plan.yml".to_string()
}

fn tasks_in(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir.join("aplan/tasks"))
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// **An entry is resolved against what the entries before it produce.**
///
/// The property that makes a manifest a plan rather than a list: a cut declared
/// here holds tasks declared after it, an id is minted from the convention the
/// entries themselves establish, and a task waits on one the same file creates.
/// None of that is expressible as N independent invocations without the author
/// working out the ids first, which is what drove a pilot to write frontmatter.
/// Proves the named behaviour `add.reads_a_task_list` (`A1-241`).
#[test]
fn a_later_entry_sees_what_an_earlier_one_created() {
    let dir = repo();
    config(dir.path(), "root: aplan\n");
    let m = manifest(
        dir.path(),
        "- title: The first cut\n  id: M-s0\n  tags: [milestone, s0]\n\
         - title: Research the corpus\n  id: A-001\n  cut: M-s0\n  effort: 2\n\
         - title: Build the table\n  cut: s0\n  effort: 3\n  blocked_by: [A-001]\n",
    );

    let out = common::run(dir.path(), &["add", "--from", &m]);
    assert_eq!(out.code, 0, "{}", out.text());
    assert_eq!(
        tasks_in(dir.path()),
        ["A-001.md", "A-002.md", "M-s0.md"],
        "the third id is minted from the convention the second one set"
    );

    let cut = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s0.md")).expect("read");
    assert!(
        cut.contains("- A-001") && cut.contains("- A-002"),
        "a cut created here holds the tasks that joined it, including by label: {cut}"
    );
    let second = std::fs::read_to_string(dir.path().join("aplan/tasks/A-002.md")).expect("read");
    assert!(
        second.contains("blocked_by:\n- A-001"),
        "and an entry may wait on one an earlier entry created: {second}"
    );

    let check = common::run(dir.path(), &["check"]);
    assert_eq!(
        check.code,
        0,
        "the plan it wrote is clean: {}",
        check.text()
    );
}

/// **One bad entry writes nothing at all.**
///
/// The reason to batch rather than loop: a hundred and fifty invocations that
/// fail at the eighty-seventh leave a half-authored plan and no record of where
/// it stopped.
/// Proves the named behaviour `add.reads_a_task_list` (`A1-241`).
#[test]
fn a_batch_that_cannot_finish_writes_none_of_it() {
    let dir = repo();
    config(dir.path(), "root: aplan\n");
    let m = manifest(
        dir.path(),
        "- title: A good one\n  id: A-001\n  effort: 1\n\
         - title: Another good one\n  effort: 1\n\
         - title: One that cannot work\n  blocked_by: [NOPE]\n",
    );

    let out = common::run(dir.path(), &["add", "--from", &m]);
    assert_eq!(out.code, 2, "{}", out.text());
    assert!(
        out.text().contains("entry 3") && out.text().contains("NOPE"),
        "the refusal names the entry and what is wrong with it: {}",
        out.text()
    );
    assert!(
        tasks_in(dir.path()).is_empty(),
        "and the two entries before it were not written: {:?}",
        tasks_in(dir.path())
    );
}

/// **The rules run once, on the plan the whole file would produce.**
///
/// Not once per entry: a manifest that ends clean passes through states no
/// author intends, and judging each of them would refuse a plan that is correct
/// the moment it is finished.
#[test]
fn the_gate_judges_the_finished_plan_and_refuses_the_batch() {
    let dir = repo();
    config(dir.path(), "root: aplan\nrequire_cut_membership: true\n");
    let m = manifest(
        dir.path(),
        "- title: The cut\n  id: M-s0\n  tags: [milestone]\n\
         - title: In the cut\n  id: A-001\n  cut: M-s0\n  effort: 1\n\
         - title: In no cut at all\n  id: A-002\n  effort: 1\n",
    );

    let out = common::run(dir.path(), &["add", "--from", &m]);
    assert_eq!(out.code, 2, "{}", out.text());
    assert!(
        out.text().contains("APLAN022"),
        "the refusal is the rule's, with its own remedy: {}",
        out.text()
    );
    assert!(
        tasks_in(dir.path()).is_empty(),
        "and nothing was written: {:?}",
        tasks_in(dir.path())
    );
}

/// **`--dry-run` says what it would write, and writes nothing**, including the
/// cut edits, which is the half a loop of `add --dry-run` cannot show.
#[test]
fn a_dry_run_reports_the_whole_change_and_makes_none_of_it() {
    let dir = repo();
    config(dir.path(), "root: aplan\n");
    let m = manifest(
        dir.path(),
        "- title: The cut\n  id: M-s0\n  tags: [milestone]\n\
         - title: In it\n  id: A-001\n  cut: M-s0\n  effort: 1\n",
    );

    let out = common::run(dir.path(), &["add", "--from", &m, "--dry-run"]);
    assert_eq!(out.code, 0, "{}", out.text());
    assert!(
        out.stdout.contains("would write M-s0") && out.stdout.contains("would write A-001"),
        "{}",
        out.stdout
    );
    assert!(
        tasks_in(dir.path()).is_empty(),
        "nothing was written: {:?}",
        tasks_in(dir.path())
    );
}

/// **An id the plan already has refuses the batch**, rather than overwriting a
/// task somebody wrote by hand.
#[test]
fn an_id_that_is_taken_refuses_before_anything_is_written() {
    let dir = repo();
    config(dir.path(), "root: aplan\n");
    std::fs::write(
        dir.path().join("aplan/tasks/A-001.md"),
        "---\nid: A-001\ntitle: hand written\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\n## Why\n\nx\n",
    )
    .expect("existing");
    let m = manifest(
        dir.path(),
        "- title: A new one\n  effort: 1\n- title: Collides\n  id: A-001\n  effort: 1\n",
    );

    let out = common::run(dir.path(), &["add", "--from", &m]);
    assert_eq!(out.code, 2, "{}", out.text());
    assert!(out.text().contains("A-001"), "{}", out.text());
    assert!(
        std::fs::read_to_string(dir.path().join("aplan/tasks/A-001.md"))
            .expect("read")
            .contains("hand written"),
        "the file that was there is untouched"
    );
    assert_eq!(
        tasks_in(dir.path()),
        ["A-001.md"],
        "and the entry before it was not written either"
    );
}

/// **A manifest and the arguments are two ways to say one thing, so they are not
/// combined**, which clap refuses before the command runs.
#[test]
fn a_title_and_a_manifest_together_are_refused() {
    let dir = repo();
    config(dir.path(), "root: aplan\n");
    let m = manifest(dir.path(), "- title: A task\n  effort: 1\n");

    let out = common::run(dir.path(), &["add", "a title", "--from", &m]);
    assert_eq!(out.code, 2, "{}", out.text());
    assert!(
        out.text().contains("cannot be used with"),
        "clap says which two: {}",
        out.text()
    );
}

/// **An id that could name a file outside the plan is refused** (`A1-241`,
/// audit).
///
/// `emit::write_task` has always refused these, so `aplan add` was safe; this
/// path writes the file itself and skipped the check, and a manifest id of
/// `../../elsewhere/x` wrote there. A manifest is generated from a document, so
/// its ids are as trustworthy as that document.
#[test]
fn an_id_that_escapes_the_plan_directory_is_refused() {
    for bad in ["../../outside/pwned", "a/b", "..", ".", "  spaced  ", ""] {
        let dir = repo();
        config(dir.path(), "root: aplan\n");
        std::fs::create_dir_all(dir.path().join("outside")).expect("sibling");
        let m = manifest(
            dir.path(),
            &format!("- title: escapes\n  id: '{bad}'\n  effort: 1\n"),
        );

        let out = common::run(dir.path(), &["add", "--from", &m]);
        assert_eq!(out.code, 2, "`{bad}` must be refused: {}", out.text());
        assert!(
            out.text().contains("cannot name a file"),
            "`{bad}`: {}",
            out.text()
        );
        assert!(
            std::fs::read_dir(dir.path().join("outside"))
                .expect("read")
                .next()
                .is_none(),
            "`{bad}` wrote outside the plan"
        );
        assert!(tasks_in(dir.path()).is_empty(), "`{bad}` wrote a task");
    }
}

/// **The first bulk add in a repository with no plan directory works**, exactly
/// as the first `aplan add` does.
///
/// It did not: a lock file lives in the target's directory, so a creation into a
/// directory that is not there failed at the lock, reporting "No such file or
/// directory" about a path the command was about to create. This is the shape
/// every adoption starts in (`A1-241`, audit).
#[test]
fn the_first_batch_creates_the_plan_directory_it_writes_into() {
    let dir = repo();
    std::fs::remove_dir_all(dir.path().join("aplan")).expect("start with nothing");
    config(dir.path(), "root: aplan\n");
    let m = manifest(
        dir.path(),
        "- title: The very first\n  id: A-001\n  effort: 1\n",
    );

    let out = common::run(dir.path(), &["add", "--from", &m]);
    assert_eq!(out.code, 0, "{}", out.text());
    assert!(
        dir.path().join("aplan/A-001.md").exists(),
        "the task landed: {}",
        out.text()
    );
    let check = common::run(dir.path(), &["check"]);
    assert_eq!(check.code, 0, "{}", check.text());
}

/// **Order is for reading, not for resolving** (`A1-293`).
///
/// Membership has two spellings and they used to pull in opposite directions:
/// `blocked_by` on the milestone wanted its members written first, `cut:` on
/// each task wanted the milestone written first, and a file cannot do both. The
/// natural order -- the cut, then the work under it, the way a roadmap reads --
/// was the one both refused for one of the two spellings.
#[test]
fn a_milestone_may_be_written_above_the_work_it_lists() {
    for (shape, body) in [
        (
            "blocked_by on the milestone",
            "- id: M-a\n  title: Alpha\n  tags: [milestone]\n  blocked_by: [T-001, T-002]\n\
             - id: T-001\n  title: First\n  effort: 1\n\
             - id: T-002\n  title: Second\n  effort: 1\n",
        ),
        (
            "cut on each task, with the milestone last",
            "- id: T-001\n  title: First\n  cut: M-a\n  effort: 1\n\
             - id: T-002\n  title: Second\n  cut: M-a\n  effort: 1\n\
             - id: M-a\n  title: Alpha\n  tags: [milestone]\n",
        ),
    ] {
        let dir = repo();
        config(dir.path(), "root: aplan\n");
        std::fs::write(dir.path().join("plan.yml"), body).expect("manifest");

        let (out, code) = common::aplan(dir.path(), &["add", "--from", "plan.yml"]);
        assert_eq!(code, 0, "{shape} was refused: {out}");

        // And the membership really landed, in both spellings. The edge used to
        // be written inside the loop, where a cut declared later had no slot to
        // write into, so `cut:` forwards would have reported success and dropped
        // it.
        let milestone =
            std::fs::read_to_string(dir.path().join("aplan/tasks/M-a.md")).expect("M-a");
        assert!(milestone.contains("- T-001"), "{shape}: {milestone}");
        assert!(milestone.contains("- T-002"), "{shape}: {milestone}");

        let (out, code) = common::aplan(dir.path(), &["check"]);
        assert_eq!(code, 0, "{shape}: {out}");
    }
}

/// An id nothing declares is still refused, in the message that says where it
/// was looked for.
#[test]
fn an_id_neither_the_plan_nor_the_file_has_is_still_refused() {
    let dir = repo();
    config(dir.path(), "root: aplan\n");
    std::fs::write(
        dir.path().join("plan.yml"),
        "- id: M-a\n  title: Alpha\n  tags: [milestone]\n  blocked_by: [T-999]\n",
    )
    .expect("manifest");
    let (out, code) = common::aplan(dir.path(), &["add", "--from", "plan.yml"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("T-999"), "{out}");
    assert!(out.contains("this file does not declare"), "{out}");
}

/// A cut the file declares is named by its `id`, and a name that is neither an
/// id nor a milestone says which of the two it failed to be.
#[test]
fn a_cut_this_file_declares_is_named_by_its_id() {
    let dir = repo();
    config(dir.path(), "root: aplan\n");

    // A later entry that is not a milestone.
    std::fs::write(
        dir.path().join("plan.yml"),
        "- id: T-001\n  title: Work\n  cut: T-002\n  effort: 1\n\
         - id: T-002\n  title: Not a milestone\n  effort: 1\n",
    )
    .expect("manifest");
    let (out, code) = common::aplan(dir.path(), &["add", "--from", "plan.yml"]);
    assert_ne!(code, 0, "{out}");
    assert!(
        out.contains("carries no `milestone` tag"),
        "the reader is not told which of the two it is: {out}"
    );

    // A label, where the file declares a cut under a different id.
    std::fs::write(
        dir.path().join("plan.yml"),
        "- id: T-001\n  title: Work\n  cut: Alpha\n  effort: 1\n\
         - id: M-a\n  title: Alpha\n  tags: [milestone]\n",
    )
    .expect("manifest");
    let (out, code) = common::aplan(dir.path(), &["add", "--from", "plan.yml"]);
    assert_ne!(code, 0, "{out}");
    assert!(
        out.contains("M-a") && out.contains("named by its `id`"),
        "the ids that would have worked are not named: {out}"
    );
}

/// **A cut the plan cannot identify stays unidentified, whatever the file
/// declares** (`A1-293`, audit).
///
/// `resolve_id` refuses an ambiguous label on purpose: taking one silently sends
/// a reader who typed what `roadmap` printed into a cut they did not mean. The
/// first version of the forward-reference lookup read past *any* failure to the
/// ids the file declares, so a label naming two existing cuts was answered with
/// a third one the manifest happened to declare under that name.
#[test]
fn an_ambiguous_cut_is_refused_even_where_the_file_declares_that_id() {
    let dir = repo();
    config(dir.path(), "root: aplan\n");
    for id in ["M-a", "M-b"] {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: Cut {id}\nphase: 1\nstatus: todo\ntags:\n\
                 - milestone\n- v1\n---\n\n## Why\n\nx\n"
            ),
        )
        .expect("cut");
    }
    std::fs::write(
        dir.path().join("plan.yml"),
        "- id: T-new\n  title: New work\n  cut: v1\n  effort: 1\n\
         - id: v1\n  title: A third cut\n  tags: [milestone]\n",
    )
    .expect("manifest");

    let (out, code) = common::aplan(dir.path(), &["add", "--from", "plan.yml"]);
    assert_ne!(code, 0, "an ambiguous label was resolved anyway: {out}");
    assert!(
        out.contains("is the label of 2 cuts") && out.contains("M-a") && out.contains("M-b"),
        "the ambiguity was answered with a third cut instead of refused: {out}"
    );
    assert!(
        !dir.path().join("aplan/tasks/T-new.md").exists(),
        "a refusal wrote a task"
    );
}
