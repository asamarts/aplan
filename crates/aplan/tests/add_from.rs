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

use crate::common;

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

fn commit_all(dir: &Path) {
    let add = Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .output()
        .expect("git add");
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let commit = Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ])
        .current_dir(dir)
        .output()
        .expect("git commit");
    assert!(
        commit.status.success(),
        "{}",
        String::from_utf8_lossy(&commit.stderr)
    );
}

/// Proves `add.accepts_creation_time_model_fields` and
/// `add_from.resolves_terminal_ledger_entries_atomically` end to end.
#[test]
fn manifest_creation_fields_and_terminal_ledger_are_atomic() {
    let dir = repo();
    config(dir.path(), "root: aplan\ndetect_drift: true\n");
    let live = manifest(
        dir.path(),
        "- title: Parked experiment\n  id: A-001\n  status: blocked\n  effort: 1\n  \
         paths: [src/one.rs]\n  blocked_by: [A-002]\n  prefer_after: [A-003]\n  \
         ratified_by: [A-004]\n  revisit_at: 2030-01-02T03:04:05Z\n  \
         version_impact: feature\n- title: Later blocker\n  id: A-002\n  effort: 1\n  \
         paths: [src/two.rs]\n- title: Later preference\n  id: A-003\n  effort: 1\n  \
         paths: [src/three.rs]\n- title: Later reviewer\n  id: A-004\n  effort: 1\n  \
         paths: [src/four.rs]\n- title: Container\n  id: M-container\n  tags: [milestone]\n  \
         blocked_by: [M-later]\n  contains: [M-later]\n- title: Later contained cut\n  id: M-later\n  \
         tags: [milestone]\n  blocked_by: [A-001]\n",
    );
    let out = common::run(dir.path(), &["add", "--from", &live]);
    assert_eq!(out.code, 0, "{}", out.text());
    let parked = std::fs::read_to_string(dir.path().join("aplan/tasks/A-001.md")).expect("parked");
    for expected in [
        "status: blocked",
        "blocked_by:\n- A-002",
        "prefer_after:\n- A-003",
        "ratified_by:\n- A-004",
        "revisit_at: 2030-01-02T03:04:05Z",
        "version_impact: feature",
    ] {
        assert!(parked.contains(expected), "missing {expected:?}:\n{parked}");
    }
    let container =
        std::fs::read_to_string(dir.path().join("aplan/tasks/M-container.md")).expect("container");
    assert!(container.contains("contains:\n- M-later"), "{container}");

    std::fs::write(dir.path().join("proof.txt"), "historical delivery\n").expect("proof");
    std::fs::write(
        dir.path().join("DOC.md"),
        "# Contract\n\n## Ledger\n\nHistorical record.\n",
    )
    .expect("doc");
    std::fs::write(
        dir.path().join("aplan/tasks/OLD.md"),
        "---\nid: OLD\ntitle: old work\nphase: 1\nstatus: done\nevidence:\n- kind: note\n  ref: reviewed\n  captured: 2026-10-30\npaths: [proof.txt]\n---\n",
    )
    .expect("old");
    let cut_path = dir.path().join("aplan/tasks/M-done.md");
    std::fs::write(
        &cut_path,
        "---\nid: M-done\ntitle: historical cut\nphase: 1\nstatus: done\ntags: [milestone]\nblocked_by: [OLD]\n---\n",
    )
    .expect("cut");
    commit_all(dir.path());

    let ledger = manifest(
        dir.path(),
        "- title: Reconstructed delivery\n  id: HIST-001\n  phase: 1\n  status: done\n  \
         cut: M-done\n  paths: [proof.txt]\n  traces: [DOC.md#ledger]\n  evidence:\n  \
         - kind: file\n    ref: proof.txt\n",
    );
    let out = common::run(dir.path(), &["add", "--from", &ledger]);
    assert_eq!(out.code, 0, "{}", out.text());
    let historical =
        std::fs::read_to_string(dir.path().join("aplan/tasks/HIST-001.md")).expect("history");
    assert!(historical.contains("status: done"), "{historical}");
    assert!(historical.contains("trace_fingerprints:"), "{historical}");
    assert!(
        historical.contains("evidence_fingerprints:"),
        "{historical}"
    );
    let cut_after = std::fs::read_to_string(&cut_path).expect("cut after");
    assert!(cut_after.contains("HIST-001"), "{cut_after}");

    let refused = manifest(
        dir.path(),
        "- title: Live sibling that must not leak\n  id: A-005\n  effort: 1\n  paths: [src/five.rs]\n\
         - title: Broken historical entry\n  id: HIST-002\n  phase: 1\n  status: done\n  cut: M-done\n  \
         paths: [missing.txt]\n  evidence:\n  - kind: file\n    ref: missing.txt\n",
    );
    let before_cut = std::fs::read_to_string(&cut_path).expect("before refusal");
    let out = common::run(dir.path(), &["add", "--from", &refused]);
    assert_eq!(out.code, 1, "{}", out.text());
    assert!(out.text().contains("fail `aplan verify`"), "{}", out.text());
    assert!(!dir.path().join("aplan/tasks/A-005.md").exists());
    assert!(!dir.path().join("aplan/tasks/HIST-002.md").exists());
    assert_eq!(
        std::fs::read_to_string(&cut_path).expect("unchanged cut"),
        before_cut
    );
}

#[test]
fn later_declared_terminal_cuts_obey_the_same_ledger_boundary() {
    let dir = repo();
    config(dir.path(), "root: aplan\n");
    let valid = manifest(
        dir.path(),
        "- title: Historical member\n  id: HIST-001\n  status: done\n  cut: M-history\n  \
         evidence:\n  - kind: note\n    ref: historical review\n    captured: 2026-10-30\n\
         - title: Historical cut\n  id: M-history\n  status: done\n  tags: [milestone]\n  \
         evidence:\n  - kind: note\n    ref: release review\n    captured: 2026-10-30\n",
    );
    let out = common::run(dir.path(), &["add", "--from", &valid]);
    assert_eq!(out.code, 0, "{}", out.text());
    let cut = std::fs::read_to_string(dir.path().join("aplan/tasks/M-history.md"))
        .expect("historical cut");
    assert!(cut.contains("HIST-001"), "{cut}");

    let refused_dir = repo();
    config(refused_dir.path(), "root: aplan\n");
    let invalid = manifest(
        refused_dir.path(),
        "- title: Live member\n  id: A-001\n  cut: M-history\n\
         - title: Historical cut\n  id: M-history\n  status: done\n  tags: [milestone]\n  \
         evidence:\n  - kind: note\n    ref: release review\n    captured: 2026-10-30\n",
    );
    let out = common::run(refused_dir.path(), &["add", "--from", &invalid]);
    assert_eq!(out.code, 2, "{}", out.text());
    assert!(
        out.text().contains("cannot gain this member"),
        "{}",
        out.text()
    );
    assert!(
        tasks_in(refused_dir.path()).is_empty(),
        "settled cut leaked files"
    );

    let dropped_dir = repo();
    config(dropped_dir.path(), "root: aplan\n");
    let dropped = manifest(
        dropped_dir.path(),
        "- title: Invented disposition\n  id: A-001\n  status: dropped\n",
    );
    let out = common::run(dropped_dir.path(), &["add", "--from", &dropped]);
    assert_eq!(out.code, 2, "{}", out.text());
    assert!(out.text().contains("guarded `drop`"), "{}", out.text());
    assert!(tasks_in(dropped_dir.path()).is_empty());
}

#[test]
fn manifest_creation_state_neighbours_are_refused_atomically() {
    for (entry, expected) in [
        (
            "title: unproved history\n  id: A-001\n  status: done",
            "needs evidence",
        ),
        (
            "title: premature evidence\n  id: A-001\n  evidence:\n  - kind: note\n    ref: not done\n    captured: 2026-10-30",
            "live work",
        ),
        (
            "title: wrongly scheduled\n  id: A-001\n  status: todo\n  revisit_at: 2030-01-02T03:04:05Z",
            "initially parked",
        ),
        (
            "title: invalid schedule\n  id: A-001\n  status: blocked\n  revisit_at: next Thursday",
            "RFC 3339",
        ),
        (
            "title: unexplained field\n  id: A-001\n  version_impact_reason: no impact",
            "requires the `version_impact`",
        ),
        (
            "title: impossible effort\n  id: A-001\n  effort: .nan",
            "finite, non-negative",
        ),
    ] {
        let dir = repo();
        config(dir.path(), "root: aplan\n");
        let input = manifest(
            dir.path(),
            &format!("- title: valid sibling\n  id: A-000\n  effort: 1\n- {entry}\n"),
        );
        let out = common::run(dir.path(), &["add", "--from", &input]);
        assert_eq!(out.code, 2, "{entry:?}: {}", out.text());
        assert!(out.text().contains(expected), "{entry:?}: {}", out.text());
        assert!(tasks_in(dir.path()).is_empty(), "{entry:?} leaked files");
    }
}

/// Proves `authoring.refuses_visually_empty_creation_values` on the batch
/// surface, including that one bad entry publishes none of its valid sibling.
#[test]
fn manifest_refuses_every_visually_empty_model_value_atomically() {
    let invisible = "\u{200b}";
    for entry in [
        format!("title: '{invisible}'\n  id: A-002"),
        format!("title: bad id\n  id: '{invisible}'"),
        format!("title: bad tag\n  id: A-002\n  tags: ['{invisible}']"),
        format!("title: bad path\n  id: A-002\n  paths: ['{invisible}']"),
        format!("title: bad trace\n  id: A-002\n  traces: ['{invisible}']"),
        format!("title: bad relation\n  id: A-002\n  blocked_by: ['{invisible}']"),
        format!("title: bad owner\n  id: A-002\n  owner: '{invisible}'"),
        format!(
            "title: bad reason\n  id: A-002\n  version_impact: none\n  version_impact_reason: '{invisible}'"
        ),
        format!("title: bad body\n  id: A-002\n  body: '{invisible}'"),
    ] {
        let dir = repo();
        config(dir.path(), "root: aplan\n");
        let input = manifest(
            dir.path(),
            &format!("- title: valid sibling\n  id: A-001\n  effort: 1\n- {entry}\n"),
        );
        let out = common::run(dir.path(), &["add", "--from", &input]);
        assert_eq!(out.code, 2, "{entry:?}: {}", out.text());
        assert!(tasks_in(dir.path()).is_empty(), "{entry:?} leaked files");
    }
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

#[test]
fn a_later_cut_cannot_turn_an_earlier_task_id_into_a_shadowing_name() {
    let dir = repo();
    config(dir.path(), "root: aplan\n");
    let m = manifest(
        dir.path(),
        "- title: Innocent at this point\n  id: s0\n  effort: 1\n\
         - title: The later cut\n  id: M-s0\n  tags: [milestone, s0]\n  blocked_by: [s0]\n",
    );

    let out = common::run(dir.path(), &["add", "--from", &m]);
    assert_eq!(out.code, 2, "{}", out.text());
    assert!(
        out.text().contains("make that label unusable"),
        "{}",
        out.text()
    );
    assert!(
        tasks_in(dir.path()).is_empty(),
        "the complete post-state refusal wrote part of the batch"
    );
}

#[test]
fn a_new_cut_may_use_its_own_id_as_its_label() {
    let dir = repo();
    config(dir.path(), "root: aplan\n");
    let m = manifest(
        dir.path(),
        "- title: Member\n  id: A-001\n  effort: 1\n\
         - title: Self-labelled cut\n  id: M-own\n  tags: [milestone, M-own]\n  blocked_by: [A-001]\n",
    );

    let out = common::run(dir.path(), &["add", "--from", &m]);
    assert_eq!(out.code, 0, "{}", out.text());
    let check = common::run(dir.path(), &["check"]);
    assert_eq!(check.code, 0, "{}", check.text());
    let by_id = common::run(
        dir.path(),
        &["add", "after self label", "--cut", "M-own", "--effort", "1"],
    );
    assert_eq!(by_id.code, 0, "{}", by_id.text());
}

/// A milestone carrying work fields is refused as invalid add input rather
/// than written and left for `check` to reject (`A1-369`).
///
/// Proves the named behaviour `add.refuses_milestone_work_fields` on the batch
/// surface. The `effort: 0` case is the adopter's exact input under
/// `require_effort: true`; the `paths` neighbour is the other field APLAN021
/// makes illegal on a checkpoint. A valid earlier entry proves the refusal is
/// atomic rather than merely skipping the bad file.
#[test]
fn a_manifest_milestone_with_work_fields_refuses_the_whole_batch() {
    for (field, message, policy) in [
        ("effort: 0", "carries `effort_days`", ""),
        (
            "paths: ['src/**']",
            "declares `paths`",
            "advisory: [APLAN021]\n",
        ),
    ] {
        let dir = repo();
        config(
            dir.path(),
            &format!("root: aplan\nrequire_effort: true\n{policy}"),
        );
        let m = manifest(
            dir.path(),
            &format!(
                "- title: Work\n  id: A-001\n  effort: 1\n\
                 - title: A bad cut\n  id: M-bad\n  tags: [milestone]\n  \
                 blocked_by: [A-001]\n  {field}\n"
            ),
        );

        let out = common::run(dir.path(), &["add", "--from", &m, "--no-deny-warnings"]);
        assert_eq!(out.code, 2, "{field} must be refused: {}", out.text());
        assert!(
            out.text().contains("APLAN021") && out.text().contains(message),
            "{}",
            out.text()
        );
        assert!(
            !out.text().contains("A cut is created with its work"),
            "a populated milestone was misdiagnosed as empty: {}",
            out.text()
        );
        assert!(
            tasks_in(dir.path()).is_empty(),
            "the refused batch wrote files: {:?}",
            tasks_in(dir.path())
        );

        let check = common::run(dir.path(), &["check"]);
        assert_eq!(
            check.code,
            0,
            "the refusal must leave the plan clean: {}",
            check.text()
        );
        assert!(check.text().contains("0 warning(s)"), "{}", check.text());

        let clean = manifest(
            dir.path(),
            "- title: Work\n  id: A-001\n  effort: 1\n\
             - title: A clean cut\n  id: M-good\n  tags: [milestone]\n  \
             blocked_by: [A-001]\n",
        );
        let out = common::run(dir.path(), &["add", "--from", &clean]);
        assert_eq!(
            out.code,
            0,
            "a milestone without work fields stays valid: {}",
            out.text()
        );
    }
}

/// The batch surface also distinguishes a bad milestone it creates from an
/// existing cut whose APLAN021 message an improving batch changes (`A1-406`).
/// Proves `add.milestone_work_refusal_is_new_input_only` on `add --from`.
#[test]
fn an_improving_manifest_add_to_an_existing_malformed_milestone_is_not_hard_refused() {
    let dir = repo();
    config(dir.path(), "root: aplan\n");
    let cut_path = dir.path().join("aplan/tasks/M-bad.md");
    std::fs::write(
        &cut_path,
        "---\nid: M-bad\ntitle: Existing bad cut\nphase: 1\nstatus: todo\neffort_days: 1\n\
         tags: [milestone]\n---\n",
    )
    .expect("write malformed milestone");
    let m = manifest(
        dir.path(),
        "- title: First member\n  id: A-001\n  cut: M-bad\n  effort: 1\n\
         - title: Second member\n  id: A-002\n  cut: M-bad\n  effort: 1\n  blocked_by: [A-001]\n",
    );

    let out = common::run(dir.path(), &["add", "--from", &m, "--no-deny-warnings"]);
    assert_eq!(
        out.code,
        0,
        "an improving batch was mistaken for a new bad milestone: {}",
        out.text()
    );
    assert!(
        out.text().contains("APLAN021") && out.text().contains("carries `effort_days`"),
        "the remaining pre-existing warning must still be reported: {}",
        out.text()
    );
    assert!(
        !out.text().contains("has no members"),
        "the fixed clause survived: {}",
        out.text()
    );
    assert_eq!(
        tasks_in(dir.path()),
        ["A-001.md", "A-002.md", "M-bad.md"],
        "the successful batch did not land whole"
    );
    let cut = std::fs::read_to_string(cut_path).expect("read cut");
    assert!(
        cut.contains("A-001") && cut.contains("A-002"),
        "the cut did not gain both members: {cut}"
    );

    let check = common::run(dir.path(), &["check"]);
    assert_eq!(
        check.code,
        0,
        "a warning is lenient by default: {}",
        check.text()
    );
    assert!(
        check.text().contains("1 warning(s)") && !check.text().contains("has no members"),
        "the post-state did not strictly improve the warning: {}",
        check.text()
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

/// Proves `dep.atomic_batch_rewrites`: task creation and reverse edges to
/// existing or same-manifest targets share one guarded publication.
#[test]
fn reverse_blocks_edges_join_creation_transaction() {
    let dir = repo();
    let root = dir.path();
    config(root, "root: aplan\n");
    std::fs::write(
        root.join("aplan/tasks/A-base.md"),
        "---\nid: A-base\ntitle: base\nphase: 1\nstatus: todo\neffort_days: 1\n---\n",
    )
    .expect("base");
    let gate_path = root.join("aplan/tasks/A-gate.md");
    let gate = "---\nid: A-gate\ntitle: gate\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [ 'A-base'] # retained source\n---\n\n## Why\n\ngate\n";
    std::fs::write(&gate_path, gate).expect("gate");
    let m = manifest(
        root,
        "- title: New blocker\n  id: A-new\n  effort: 1\n  blocks: [A-gate, A-later]\n\
         - title: Later target\n  id: A-later\n  effort: 1\n",
    );

    let first = common::run(root, &["add", "--from", &m, "--dry-run"]);
    let second = common::run(root, &["add", "--from", &m, "--dry-run"]);
    assert_eq!(first.code, 0, "{}", first.text());
    assert_eq!(first.stdout, second.stdout, "dry-run output must be stable");
    assert!(
        first.stdout.contains("would write A-new"),
        "{}",
        first.stdout
    );
    assert!(
        first
            .stdout
            .contains("would make `A-gate` come after `A-new`")
            && first
                .stdout
                .contains("would make `A-later` come after `A-new`"),
        "{}",
        first.stdout
    );
    assert_eq!(std::fs::read_to_string(&gate_path).unwrap(), gate);
    assert!(!root.join("aplan/tasks/A-new.md").exists());

    let out = common::run(root, &["add", "--from", &m]);
    assert_eq!(out.code, 0, "{}", out.text());
    let gate_after = std::fs::read_to_string(&gate_path).expect("gate after");
    assert!(
        gate_after.contains("blocked_by: [ 'A-base', 'A-new'] # retained source"),
        "{gate_after}"
    );
    let later = std::fs::read_to_string(root.join("aplan/tasks/A-later.md")).expect("later");
    assert!(later.contains("blocked_by:\n- A-new"), "{later}");
    let check = common::run(root, &["check"]);
    assert_eq!(check.code, 0, "{}", check.text());
}

#[test]
fn reverse_blocks_refusals_and_locked_publication_leak_no_created_task() {
    for (blocks, expected) in [
        ("[NOPE]", "does not have"),
        ("[A-gate, A-gate]", "more than once"),
    ] {
        let dir = repo();
        config(dir.path(), "root: aplan\n");
        std::fs::write(
            dir.path().join("aplan/tasks/A-gate.md"),
            "---\nid: A-gate\ntitle: gate\nphase: 1\nstatus: todo\neffort_days: 1\n---\n",
        )
        .expect("gate");
        let m = manifest(
            dir.path(),
            &format!("- title: blocker\n  id: A-new\n  effort: 1\n  blocks: {blocks}\n"),
        );
        let before = std::fs::read_to_string(dir.path().join("aplan/tasks/A-gate.md")).unwrap();
        let out = common::run(dir.path(), &["add", "--from", &m]);
        assert_eq!(out.code, 2, "{}", out.text());
        assert!(out.text().contains(expected), "{}", out.text());
        assert!(!dir.path().join("aplan/tasks/A-new.md").exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("aplan/tasks/A-gate.md")).unwrap(),
            before
        );
    }

    let dir = repo();
    config(dir.path(), "root: aplan\n");
    let gate_path = dir.path().join("aplan/tasks/A-gate.md");
    let gate = "---\nid: A-gate\ntitle: gate\nphase: 1\nstatus: todo\neffort_days: 1\n---\n";
    std::fs::write(&gate_path, gate).expect("gate");
    let m = manifest(
        dir.path(),
        "- title: blocker\n  id: A-new\n  effort: 1\n  blocks: [A-gate]\n",
    );
    std::fs::write(dir.path().join("aplan/tasks/.A-gate.md.lock"), "").expect("lock");
    let out = common::run(dir.path(), &["add", "--from", &m]);
    assert_eq!(out.code, 2, "{}", out.text());
    assert!(out.text().contains("nothing was written"), "{}", out.text());
    assert!(!dir.path().join("aplan/tasks/A-new.md").exists());
    assert_eq!(std::fs::read_to_string(&gate_path).unwrap(), gate);
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
