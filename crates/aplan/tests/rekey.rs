//! Stable-id migration is one guarded change, and prose stays a question.

use crate::common;

use std::path::Path;

const STALE_FILENAME_LINK: &str = "\u{03bb} See [the old task](../../aplan/tasks/T-old.md).";

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("directories");
    std::fs::write(path, text).expect("write fixture");
}

fn task(id: &str, extra: &str, body: &str) -> String {
    format!(
        "---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: todo\neffort_days: 1\n{extra}---\n\n{body}\n"
    )
}

fn collision_fixture(dir: &Path) {
    write(
        dir,
        ".aplan.yml",
        "root: aplan\ndocs:\n  membership:\n    - doc: ROADMAP.md\n      declared_by: line\n      marker: \"Tasks:\"\n",
    );
    write(
        dir,
        "aplan/tasks/T-old.md",
        &task("T-old", "", "This task used to be cited as T-old."),
    );
    write(
        dir,
        "aplan/tasks/T-blocked.md",
        &task(
            "T-blocked",
            "blocked_by : # producer\n  - 'T-old' # collision owner\n  - T-preferred # unchanged edge\n",
            "No prose reference here.",
        ),
    );
    write(
        dir,
        "aplan/tasks/T-preferred.md",
        &task(
            "T-preferred",
            "prefer_after: [T-old] # preference\n",
            "Preference.",
        ),
    );
    write(
        dir,
        "aplan/tasks/T-ratified.md",
        &task(
            "T-ratified",
            "ratified_by: [T-old] # approval\n",
            "Ratification.",
        ),
    );
    write(
        dir,
        "aplan/tasks/M-cut.md",
        "---\nid: M-cut\ntitle: cut\nphase: 1\nstatus: todo\ntags: [milestone, cut]\nblocked_by: [T-old, T-blocked, T-preferred, T-ratified] # membership\n---\n\nA cut.\n",
    );
    write(
        dir,
        "docs/pilots/collision.md",
        &format!(
            "# Collision\n\nThe other branch's T-old is deliberately not this task.\nT-oldish and T-old.part are different ids and must not be reported.\n{STALE_FILENAME_LINK}\n"
        ),
    );
    write(
        dir,
        "ROADMAP.md",
        "# Membership\n\nTasks: T-old, T-oldish\n",
    );
}

/// The collision repair itself: graph facts move, an equally-spelled sentence
/// does not, and the machine report makes the unresolved occurrence visible.
/// Its commented block list proves behaviour `rekey.surgical_yaml_scalars`.
/// Its stale Markdown link proves behaviour `rekey.reports_moved_filename_links`.
#[test]
fn the_collision_fixture_repairs_every_graph_reference_and_reports_prose() {
    let dir = tempfile::Builder::new()
        .prefix("rekey-collision-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    collision_fixture(dir.path());

    let run = common::run(dir.path(), &["rekey", "T-old", "T-new", "--format", "json"]);
    assert!(run.ok(), "{run:?}");
    let report: serde_json::Value = serde_json::from_str(&run.stdout).expect("JSON report");
    assert_eq!(report["schema"], "aplan/rekey/v1");
    assert_eq!(report["old"], "T-old");
    assert_eq!(report["new"], "T-new");
    assert_eq!(report["renamed_file"], true);
    let occurrences = report["possible_prose_references"]
        .as_array()
        .expect("occurrences");
    assert!(
        occurrences
            .iter()
            .any(|row| { row["path"] == "docs/pilots/collision.md" && row["line"] == 3 }),
        "the other branch's prose was hidden: {report}"
    );
    assert!(
        occurrences
            .iter()
            .any(|row| { row["path"] == "ROADMAP.md" && row["line"] == 3 }),
        "the configured membership document was hidden: {report}"
    );
    assert!(
        !occurrences
            .iter()
            .any(|row| row["path"] == "docs/pilots/collision.md" && row["line"] == 4),
        "a longer id was reported as the old id: {report}"
    );
    assert!(
        occurrences.iter().any(|row| {
            row["path"] == "docs/pilots/collision.md"
                && row["line"] == 5
                && row["column"]
                    == STALE_FILENAME_LINK
                        .find("T-old")
                        .map(|at| STALE_FILENAME_LINK[..at].chars().count() + 1)
                        .expect("old id in fixture")
        }),
        "the moved conventional filename was hidden: {report}"
    );
    assert!(
        std::fs::read_to_string(dir.path().join("docs/pilots/collision.md"))
            .expect("pilot")
            .contains("T-old.md"),
        "the possible filename reference was rewritten"
    );

    assert!(!dir.path().join("aplan/tasks/T-old.md").exists());
    let moved = std::fs::read_to_string(dir.path().join("aplan/tasks/T-new.md")).expect("moved");
    assert!(moved.contains("id: T-new"), "{moved}");
    assert!(
        moved.contains("cited as T-old"),
        "the body was rewritten: {moved}"
    );
    let dependent =
        std::fs::read_to_string(dir.path().join("aplan/tasks/T-blocked.md")).expect("dependent");
    assert!(
        dependent.contains(
            "blocked_by : # producer\n  - 'T-new' # collision owner\n  - T-preferred # unchanged edge"
        ),
        "block-list formatting or comments moved: {dependent}"
    );
    let preferred =
        std::fs::read_to_string(dir.path().join("aplan/tasks/T-preferred.md")).expect("preferred");
    assert!(preferred.contains("prefer_after: [T-new]"), "{preferred}");
    let ratified =
        std::fs::read_to_string(dir.path().join("aplan/tasks/T-ratified.md")).expect("ratified");
    assert!(ratified.contains("ratified_by: [T-new]"), "{ratified}");
    let cut = std::fs::read_to_string(dir.path().join("aplan/tasks/M-cut.md")).expect("cut");
    assert!(
        cut.contains("blocked_by: [T-new, T-blocked, T-preferred, T-ratified]"),
        "{cut}"
    );
    assert!(cut.contains("# membership"), "{cut}");

    let check = common::run(dir.path(), &["check"]);
    assert!(check.ok(), "the repaired graph is not clean: {check:?}");
}

/// A milestone id is a policy key as well as a graph key. Its label is not.
/// The commented block proves behaviour `rekey.policy_cut_block_comments`.
#[test]
fn a_milestone_rekeys_contains_and_policy_but_not_its_unchanged_label() {
    let dir = tempfile::Builder::new()
        .prefix("rekey-milestone-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    write(
        dir.path(),
        ".aplan.yml",
        "root: aplan\npolicy:\n  - when:\n      cut: # both ways to name it\n        - M-old # id\n        - release-label # unchanged label\n    evidence:\n      require_min_strength: [test]\n",
    );
    write(
        dir.path(),
        "aplan/tasks/T-work.md",
        &task("T-work", "", "Work."),
    );
    write(
        dir.path(),
        "aplan/tasks/M-old.md",
        "---\nid: M-old\ntitle: old cut\nphase: 1\nstatus: todo\ntags: [milestone, release-label]\nblocked_by: [T-work]\n---\n\nCut.\n",
    );
    write(
        dir.path(),
        "aplan/tasks/M-parent.md",
        "---\nid: M-parent\ntitle: parent\nphase: 1\nstatus: todo\ntags: [milestone, parent]\nblocked_by: [M-old] # waits for child\ncontains: [M-old] # child\n---\n\nParent.\n",
    );

    let run = common::run(dir.path(), &["rekey", "M-old", "M-new"]);
    assert!(run.ok(), "{run:?}");
    let config = std::fs::read_to_string(dir.path().join(".aplan.yml")).expect("config");
    assert!(
        config.contains(
            "cut: # both ways to name it\n        - M-new # id\n        - release-label # unchanged label"
        ),
        "id and label were not distinguished: {config}"
    );
    let parent =
        std::fs::read_to_string(dir.path().join("aplan/tasks/M-parent.md")).expect("parent");
    assert!(parent.contains("contains: [M-new] # child"), "{parent}");
}

/// Identity lives in frontmatter. A custom filename and the document's bytes
/// around the scalar survive, while terminal history needs an explicit choice.
#[test]
fn a_custom_filename_stays_and_a_terminal_task_needs_the_flag() {
    let dir = tempfile::Builder::new()
        .prefix("rekey-terminal-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    write(dir.path(), ".aplan.yml", "root: aplan\n");
    let original = "---\r\nid : T-old # identity\r\ntitle: terminal\r\nphase: 1\r\nstatus: done\r\neffort_days: 1\r\nevidence:\r\n  - kind: note\r\n    ref: reviewed\r\n    captured: 2026-10-07\r\n---\r\n\r\nBody names T-old and stays.\r\n";
    write(dir.path(), "aplan/tasks/custom-name.md", original);

    let refused = common::run(dir.path(), &["rekey", "T-old", "T-new"]);
    assert_eq!(refused.code, 2, "{refused:?}");
    assert!(refused.text().contains("--allow-terminal"), "{refused:?}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("aplan/tasks/custom-name.md")).expect("unchanged"),
        original
    );

    let allowed = common::run(dir.path(), &["rekey", "T-old", "T-new", "--allow-terminal"]);
    assert!(allowed.ok(), "{allowed:?}");
    assert!(allowed.stdout.contains("Git history"), "{allowed:?}");
    assert!(allowed.stdout.contains("other repositories"), "{allowed:?}");
    assert!(!dir.path().join("aplan/tasks/T-new.md").exists());
    let updated =
        std::fs::read_to_string(dir.path().join("aplan/tasks/custom-name.md")).expect("updated");
    assert!(updated.contains("id : T-new # identity\r\n"), "{updated:?}");
    assert!(updated.ends_with("Body names T-old and stays.\r\n"));
}

/// Proof for behaviour `rekey.surgical_yaml_scalars`: the command accepts the
/// same quoted identity the loader does, and its edit stops at the scalar boundary.
#[test]
fn a_quoted_source_id_with_whitespace_rekeys_surgically() {
    let dir = tempfile::Builder::new()
        .prefix("rekey-quoted-id-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    write(dir.path(), ".aplan.yml", "root: aplan\n");
    let original = "\u{feff}---\r\nid : \"T old\"  # identity\r\ntitle: quoted identity\r\nphase: 1\r\nstatus: todo\r\neffort_days: 1\r\n---\r\n\r\nBody stays byte-for-byte.\r\n";
    write(dir.path(), "aplan/tasks/custom-name.md", original);

    let before = common::run(dir.path(), &["check"]);
    assert!(before.ok(), "the source plan must be valid: {before:?}");
    let run = common::run(dir.path(), &["rekey", "T old", "T new"]);
    assert!(run.ok(), "{run:?}");

    let updated =
        std::fs::read_to_string(dir.path().join("aplan/tasks/custom-name.md")).expect("updated");
    assert!(
        updated.starts_with("\u{feff}---\r\nid : T new  # identity\r\n"),
        "{updated:?}"
    );
    assert!(
        updated.ends_with("---\r\n\r\nBody stays byte-for-byte.\r\n"),
        "{updated:?}"
    );
    let after = common::run(dir.path(), &["show", "T new", "--format", "json"]);
    assert!(after.ok(), "the new identity did not load: {after:?}");
    let old = common::run(dir.path(), &["show", "T old", "--format", "json"]);
    assert_eq!(old.code, 2, "the old identity still resolves: {old:?}");
}

/// Proves `mutation.refuses_multiline_plain_scalar_rewrites` for the stable-id
/// migration and its source-file relocation boundary.
#[test]
fn a_multiline_plain_source_id_refuses_without_writing() {
    let dir = tempfile::Builder::new()
        .prefix("rekey-multiline-id-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    write(dir.path(), ".aplan.yml", "root: aplan\n");
    let original = "---\nid: T-\n  old\ntitle: multiline identity\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nBody stays exact.\n";
    let path = dir.path().join("aplan/tasks/custom-name.md");
    write(dir.path(), "aplan/tasks/custom-name.md", original);

    let before = common::run(dir.path(), &["check"]);
    assert!(before.ok(), "the source plan must be valid: {before:?}");
    let run = common::run(dir.path(), &["rekey", "T- old", "T-new"]);
    assert_eq!(
        run.code, 2,
        "the source defect was classified as internal: {run:?}"
    );
    assert!(run.text().contains("no top-level `id:` line"), "{run:?}");
    assert!(run.text().contains("change it by hand"), "{run:?}");
    assert!(!run.text().contains("bug in aplan"), "{run:?}");
    assert_eq!(
        std::fs::read_to_string(&path).expect("source after refusal"),
        original,
        "rekey changed the multiline source before refusing"
    );
    assert!(!dir.path().join("aplan/tasks/T-new.md").exists());
}

#[test]
fn human_output_cannot_be_steered_by_a_replacement_id() {
    let dir = tempfile::Builder::new()
        .prefix("rekey-visible-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    write(dir.path(), ".aplan.yml", "root: aplan\n");
    write(
        dir.path(),
        "aplan/tasks/custom.md",
        &task("T-old", "", "Work."),
    );

    let hostile = "T-\u{1b}[2J";
    let run = common::run(dir.path(), &["rekey", "T-old", hostile]);
    assert!(run.ok(), "{run:?}");
    assert!(
        !run.stdout.contains('\u{1b}'),
        "the replacement id reached the terminal: {run:?}"
    );
    let updated =
        std::fs::read_to_string(dir.path().join("aplan/tasks/custom.md")).expect("updated");
    assert_eq!(
        yaml_compat::from_str::<aplan_core::Task>(
            updated
                .strip_prefix("---\n")
                .and_then(|text| text.split_once("---\n").map(|(frontmatter, _)| frontmatter))
                .expect("frontmatter")
        )
        .expect("task")
        .id,
        hostile
    );
}

/// A selected policy outside the plan's repository is evidence to stop, not
/// authority to edit another tree.
#[test]
fn an_external_selected_config_refuses_before_any_file_changes() {
    let outer = tempfile::Builder::new()
        .prefix("rekey-external-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let repo = outer.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git marker");
    write(&repo, "aplan/tasks/T-work.md", &task("T-work", "", "Work."));
    let old = "---\nid: M-old\ntitle: cut\nphase: 1\nstatus: todo\ntags: [milestone, cut]\nblocked_by: [T-work]\n---\n";
    write(&repo, "aplan/tasks/M-old.md", old);
    let policy = outer.path().join("trusted.yml");
    write(
        outer.path(),
        "trusted.yml",
        "policy:\n  - when:\n      cut: [M-old]\n    evidence:\n      require_min_strength: [test]\n",
    );

    let run = common::run(
        &repo,
        &[
            "--config",
            policy.to_str().expect("path"),
            "--root",
            "aplan",
            "rekey",
            "M-old",
            "M-new",
        ],
    );
    assert_eq!(run.code, 2, "{run:?}");
    assert!(
        run.text().contains("outside the plan repository"),
        "{run:?}"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("aplan/tasks/M-old.md")).expect("old"),
        old
    );
    assert!(!repo.join("aplan/tasks/M-new.md").exists());
    assert!(
        std::fs::read_to_string(policy)
            .expect("policy")
            .contains("cut: [M-old]")
    );
}

#[test]
fn invalid_and_occupied_destinations_refuse_without_writing() {
    for destination in ["", "../escape", "T-taken", "T-file", "cut-label"] {
        let dir = tempfile::Builder::new()
            .prefix("rekey-refusal-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        write(dir.path(), ".aplan.yml", "root: aplan\n");
        let original = task("T-old", "", "Old.");
        write(dir.path(), "aplan/tasks/T-old.md", &original);
        if destination == "T-taken" {
            write(
                dir.path(),
                "aplan/tasks/T-taken.md",
                &task("T-taken", "", "Taken."),
            );
        } else if destination == "T-file" {
            write(
                dir.path(),
                "aplan/tasks/T-file.md",
                &task("T-other", "", "The filename is occupied."),
            );
        } else if destination == "cut-label" {
            write(
                dir.path(),
                "aplan/tasks/M-cut.md",
                "---\nid: M-cut\ntitle: Cut\nphase: 1\nstatus: todo\ntags: [milestone, cut-label]\nblocked_by: [T-old]\n---\n",
            );
        }

        let run = common::run(dir.path(), &["rekey", "T-old", destination]);
        assert_eq!(run.code, 2, "{destination:?}: {run:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("aplan/tasks/T-old.md")).expect("old"),
            original,
            "{destination:?} wrote before refusing"
        );
    }

    let dir = tempfile::Builder::new()
        .prefix("rekey-missing-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    write(dir.path(), ".aplan.yml", "root: aplan\n");
    write(
        dir.path(),
        "aplan/tasks/T-one.md",
        &task("T-one", "", "One."),
    );
    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/T-one.md")).expect("before");
    let run = common::run(dir.path(), &["rekey", "T-missing", "T-new"]);
    assert_eq!(run.code, 2, "{run:?}");
    assert!(run.text().contains("no task `T-missing`"), "{run:?}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("aplan/tasks/T-one.md")).expect("after"),
        before
    );
}
