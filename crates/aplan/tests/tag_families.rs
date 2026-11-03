//! `APLAN029` reaches a verdict from a real `.aplan.yml` (`A1-085`).
//!
//! The library tests build the family map in Rust, which skips the half an
//! adopter writes and the half that produced the finding: the fourth pilot's
//! mistake was in the YAML, not in the plan. It declared `epic: [a, b]` with
//! tasks tagged `epic:a`, where the documented shape is the whole tag, and every
//! layer accepted it (pre-public adoption evidence, item 9).

use crate::common;
use common::aplan;

/// One task carrying `tag`, under the given `.aplan.yml`.
fn repo(config: &str, tag: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan dir");
    std::fs::write(root.join(".aplan.yml"), config).expect("config");
    std::fs::write(
        root.join("aplan/tasks/R-1.md"),
        format!(
            "---\nid: R-1\ntitle: A task carrying one tag in this fixture\nphase: 1\n\
             status: todo\nowner: kaminsod\neffort_days: 1\ntags:\n- {tag}\n---\n\n\
             ## Why\n\nSomething.\n"
        ),
    )
    .expect("task");
    dir
}

/// The pilot's own configuration, end to end.
#[test]
fn the_pilots_mis_declared_family_is_reported_through_the_binary() {
    let dir = repo("tag_families:\n  epic: [a, b]\n", "epic:a");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    // A warning, so the build still passes: a vocabulary declared before its
    // first use is as likely as a mis-declaration.
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("no task carries the `epic` family"), "{out}");
    assert!(out.contains("`a`, `b`"), "{out}");
}

/// The documented shape stays silent, which is what makes the test above a
/// measurement of the mistake rather than of the rule being on.
#[test]
fn the_documented_shape_is_silent() {
    let dir = repo("tag_families:\n  epic: [epic-a, epic-b]\n", "epic-a");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("APLAN029"), "{out}");
    assert!(out.contains("0 warning(s)"), "{out}");
}

/// A repository that declares no vocabulary is untouched, or the rule would be a
/// migration for everyone who has never opened `.aplan.yml`.
#[test]
fn a_repository_declaring_no_family_is_untouched() {
    let dir = repo("require_effort: true\n", "anything");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("APLAN029"), "{out}");
}

/// The finding is about a declaration, so it points at the file the declaration
/// is in. Building the family map in Rust cannot prove this: the path comes from
/// wherever the CLI found `.aplan.yml`, and nothing had driven that.
#[test]
fn the_finding_points_at_the_config_file_the_cli_found() {
    let dir = repo("tag_families:\n  epic: [a, b]\n  kind: [x, y]\n", "epic:a");
    let (out, _) = aplan(dir.path(), &["check", "--root", "aplan"]);
    let arrows: Vec<&str> = out
        .lines()
        .filter(|l| l.trim_start().starts_with("--> "))
        .collect();
    assert_eq!(arrows.len(), 2, "one location per finding:\n{out}");
    for line in arrows {
        assert!(line.ends_with(".aplan.yml"), "{line}");
    }
    // Both families, and the second is the one that would otherwise be a bare
    // sentence: `check` prints the fix once per rule.
    assert!(out.contains("`epic` family"), "{out}");
    assert!(out.contains("`kind` family"), "{out}");
}

/// And the annotation lands on `.aplan.yml`, so a reviewer meets it on the file
/// they would edit rather than on the workflow run.
#[test]
fn the_annotation_lands_on_the_config_file() {
    let dir = repo("tag_families:\n  epic: [a, b]\n", "epic:a");
    let (out, _) = aplan(
        dir.path(),
        &["check", "--root", "aplan", "--format", "github"],
    );
    let line = out
        .lines()
        .find(|l| l.contains("APLAN029"))
        .expect("an annotation");
    assert!(line.starts_with("::warning file="), "{line}");
    assert!(line.contains(".aplan.yml,title=APLAN029::"), "{line}");
}

/// One mistake, one finding: a tag in a family's shape counts as using it, even
/// though `APLAN026` is about to reject that tag. Otherwise a single typo is
/// reported twice, once as an unknown member and once as an unused vocabulary.
#[test]
fn a_typo_in_a_tag_is_not_also_reported_as_an_unused_family() {
    let dir = repo("tag_families:\n  epic: [epic-a, epic-b]\n", "epic-x");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("APLAN026"),
        "the typo must still be caught:\n{out}"
    );
    assert!(!out.contains("APLAN029"), "reported twice:\n{out}");
}

// ---------------------------------------------------------------------------
// `APLAN030`, the cardinality half (`A1-133`). Declaring what a vocabulary
// contains said nothing about how many of it a task carries, so two priorities
// and no priority were both clean.
// ---------------------------------------------------------------------------

/// The long form is opt-in, and the list shorthand keeps meaning what it meant.
#[test]
fn the_list_shorthand_declares_no_cardinality() {
    let dir = repo("tag_families:\n  epic: [epic-a, epic-b]\n", "epic-a");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("APLAN030"), "{out}");
}

/// The vocabulary as the fifth pilot needs it, through a real `.aplan.yml`.
///
/// Two tasks, because one is not enough to test this: a plan where nothing
/// carries the family at all is `APLAN029`'s finding and this rule stays out of
/// it, so a single untagged task would prove the requirement fires for the wrong
/// reason.
fn priority_repo(second_tag: &str) -> tempfile::TempDir {
    let dir = repo(
        "tag_families:\n  priority:\n    members: [priority-p0, priority-p1]\n\
         \x20   require: exactly-one\n    status: live\n",
        "priority-p0",
    );
    std::fs::write(
        dir.path().join("aplan/tasks/R-2.md"),
        format!(
            "---\nid: R-2\ntitle: The second task in this fixture\nphase: 1\n\
             status: todo\nowner: kaminsod\neffort_days: 1\ntags:\n- {second_tag}\n---\n\n\
             ## Why\n\nSomething.\n"
        ),
    )
    .expect("second task");
    dir
}

#[test]
fn exactly_one_is_enforced_on_the_statuses_it_names() {
    // None, on a family the plan does carry elsewhere.
    let dir = priority_repo("unrelated");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("`priority` is declared exactly-one on live tasks and this task carries none"),
        "{out}"
    );
    // One.
    let dir = priority_repo("priority-p1");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("APLAN030"), "{out}");
}

/// A requirement over a vocabulary no tag matches is unbindable, not unmet.
/// `exactly-one` would fail every task in the plan and `at-most-one` would pass
/// every one, and both would be confident verdicts about a declaration that was
/// never bound. `APLAN029` is that finding, and it is one finding rather than one
/// per task plus one, which is the reason it is this rule's dependency.
#[test]
fn a_requirement_over_an_unbound_vocabulary_defers_to_the_rule_that_owns_it() {
    let dir = repo(
        "tag_families:\n  priority:\n    members: [a, b]\n    require: exactly-one\n",
        "unrelated",
    );
    for extra in ["R-2", "R-3"] {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{extra}.md")),
            format!(
                "---\nid: {extra}\ntitle: Another task carrying no priority at all\nphase: 1\n\
                 status: todo\nowner: kaminsod\neffort_days: 1\n---\n\n## Why\n\nSomething.\n"
            ),
        )
        .expect("task");
    }
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    // The declaration is the finding, so the build still passes: `APLAN029` is a
    // warning and `APLAN030` said nothing.
    assert_eq!(code, 0, "{out}");
    assert_eq!(out.matches("APLAN029").count(), 1, "{out}");
    assert!(
        !out.contains("APLAN030"),
        "one mistake, three tasks:\n{out}"
    );
}

/// An unknown value is refused where it is written, naming the key and what it
/// would have accepted. The untagged form of this enum reported "data did not
/// match any variant" for every mistake in the block, which named neither.
#[test]
fn an_unsupported_predicate_is_rejected_with_what_it_expected() {
    for (bad, expects) in [
        ("require: sometimes", "`optional`, `at-most-one`"),
        ("status: alive", "`any`, `live`"),
        ("milestones: maybe", "`include`, `exclude`"),
    ] {
        let dir = repo(
            &format!("tag_families:\n  priority:\n    members: [priority-p0]\n    {bad}\n"),
            "priority-p0",
        );
        let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
        assert_eq!(code, 2, "`{bad}` was accepted:\n{out}");
        assert!(out.contains("unknown variant"), "{out}");
        assert!(out.contains(expects), "{out}");
    }
    // And a misspelled key names the keys the block takes.
    let dir = repo(
        "tag_families:\n  priority:\n    members: [priority-p0]\n    reqiure: exactly-one\n",
        "priority-p0",
    );
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("unknown field `reqiure`"), "{out}");
    assert!(
        out.contains("`members`, `require`, `status`, `milestones`"),
        "{out}"
    );
}

/// `add` is the one write path that knows the tags, the status it writes, and
/// whether this is a milestone, so it refuses before writing rather than leaving
/// the author a file `check` rejects.
/// Proves the named behaviour `mutation.refused_when_check_would_reject` (`A1-211`).
#[test]
fn add_refuses_before_writing_what_check_would_reject_after() {
    let dir = repo(
        "tag_families:\n  priority:\n    members: [priority-p0, priority-p1]\n\
         \x20   require: exactly-one\n    status: live\n",
        "priority-p0",
    );
    let before = std::fs::read_dir(dir.path().join("aplan/tasks"))
        .expect("tasks")
        .count();

    let (out, code) = aplan(
        dir.path(),
        &["add", "A task with no priority", "--root", "aplan"],
    );
    assert_ne!(code, 0, "{out}");
    // The refusal comes from the shared post-state check now, so it names the
    // rule and carries its remedy, rather than being a message this command
    // wrote about one rule it happened to know (`A1-113`).
    assert!(out.contains("APLAN030 R-2:"), "{out}");
    assert!(out.contains("is declared exactly-one"), "{out}");
    assert!(
        out.contains("fix: Carry exactly one of `tag_families.priority`"),
        "{out}"
    );

    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "A task with two priorities",
            "--root",
            "aplan",
            "--tag",
            "priority-p0",
            "--tag",
            "priority-p1",
        ],
    );
    assert_ne!(code, 0, "{out}");

    // Nothing was written by either refusal.
    assert_eq!(
        std::fs::read_dir(dir.path().join("aplan/tasks"))
            .expect("tasks")
            .count(),
        before,
        "a refused `add` left a file behind"
    );

    // And the satisfying form goes through.
    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "A task with exactly one priority",
            "--root",
            "aplan",
            "--tag",
            "priority-p0",
        ],
    );
    assert_eq!(code, 0, "{out}");
}
