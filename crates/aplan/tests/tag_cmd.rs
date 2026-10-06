//! `aplan tag`: the vocabulary a plan maintains (`A1-213`).
//!
//! Driven through the built binary rather than through `set_tags`, for
//! `A1-107`'s reason: the unit tests beside the setter prove the round-trip, and
//! what is under test here is the seam -- the mutation actually judged before it
//! writes, and the rename actually one change over many files.

use std::path::Path;

mod common;

/// Two tasks carrying a required family member and a word to migrate.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(
        dir.join(".aplan.yml"),
        "root: aplan\ntag_families:\n  priority:\n    members: [priority-p0, priority-p1]\n\
         \x20   require: exactly-one\n",
    )
    .expect("config");
    for i in 1..=2 {
        std::fs::write(
            dir.join(format!("aplan/tasks/W-{i}.md")),
            format!(
                "---\nid: W-{i}\ntitle: work {i}\nphase: 0\nstatus: todo\neffort_days: 1\n\
                 tags:\n  - priority-p0\n  - legacy\n---\n\nbody\n"
            ),
        )
        .expect("task");
    }
}

fn tmp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir")
}

fn tags_of(dir: &Path, id: &str) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(
        &common::run(dir, &["show", id, "--format", "json", "--no-resolve"]).stdout,
    )
    .expect("json");
    doc["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .filter_map(|t| t.as_str())
        .map(str::to_string)
        .collect()
}

/// The criterion: a tag is added and removed without opening the file.
#[test]
fn a_tag_can_be_added_and_removed() {
    let dir = tmp();
    fixture(dir.path());
    assert_eq!(
        common::run(dir.path(), &["tag", "add", "W-1", "cli"]).code,
        0
    );
    assert!(tags_of(dir.path(), "W-1").contains(&"cli".to_string()));
    assert_eq!(
        common::run(dir.path(), &["tag", "rm", "W-1", "cli"]).code,
        0
    );
    assert!(!tags_of(dir.path(), "W-1").contains(&"cli".to_string()));
}

/// **The whole value over hand editing**: judged before it writes, so an edit
/// that would break a declared cardinality refuses rather than writing and then
/// reporting (`A1-113`).
#[test]
fn a_tag_edit_that_would_break_a_cardinality_refuses() {
    let dir = tmp();
    fixture(dir.path());
    let before = tags_of(dir.path(), "W-1");
    let out = common::run(dir.path(), &["tag", "rm", "W-1", "priority-p0"]);
    assert_ne!(out.code, 0, "{}", out.text());
    assert!(out.text().contains("APLAN030"), "{}", out.text());
    assert_eq!(
        tags_of(dir.path(), "W-1"),
        before,
        "it reported and wrote anyway"
    );
}

/// Renaming across the plan is one command, which is the case hand editing is
/// worst at and the reason `rename` exists beside `add` and `rm`.
#[test]
fn a_rename_touches_every_task_that_carries_it() {
    let dir = tmp();
    fixture(dir.path());
    let out = common::run(dir.path(), &["tag", "rename", "legacy", "archived"]);
    assert_eq!(out.code, 0, "{}", out.text());
    assert!(out.stdout.contains("2 task(s)"), "{}", out.stdout);
    for id in ["W-1", "W-2"] {
        let tags = tags_of(dir.path(), id);
        assert!(tags.contains(&"archived".to_string()), "{id}: {tags:?}");
        assert!(!tags.contains(&"legacy".to_string()), "{id}: {tags:?}");
    }
}

/// A rename onto a word a task already carries leaves one of it, not two.
#[test]
fn a_rename_onto_an_existing_tag_does_not_duplicate_it() {
    let dir = tmp();
    fixture(dir.path());
    assert_eq!(
        common::run(dir.path(), &["tag", "add", "W-1", "keep"]).code,
        0
    );
    assert_eq!(
        common::run(dir.path(), &["tag", "rename", "legacy", "keep"]).code,
        0
    );
    let tags = tags_of(dir.path(), "W-1");
    assert_eq!(
        tags.iter().filter(|t| *t == "keep").count(),
        1,
        "a document its own loader reads as one tag and a reviewer as a mistake: {tags:?}"
    );
}

/// A rename is judged as a whole before any file is written, so one task that
/// cannot take it stops all of them.
#[test]
fn a_rename_that_would_break_a_cardinality_writes_nothing() {
    let dir = tmp();
    fixture(dir.path());
    // W-2 keeps a member of the family, so the family stays *used* and the
    // finding is `APLAN030` about W-1 rather than `APLAN029` about the family:
    // a family nothing carries at all is one warning, not an error per task.
    std::fs::write(
        dir.path().join("aplan/tasks/W-2.md"),
        "---\nid: W-2\ntitle: work 2\nphase: 0\nstatus: todo\neffort_days: 1\n\
         tags:\n  - priority-p1\n  - legacy\n---\n\nbody\n",
    )
    .expect("task");
    let before = (tags_of(dir.path(), "W-1"), tags_of(dir.path(), "W-2"));
    let out = common::run(dir.path(), &["tag", "rename", "priority-p0", "urgent"]);
    assert_ne!(out.code, 0, "{}", out.text());
    assert!(out.text().contains("APLAN030"), "{}", out.text());
    assert_eq!(
        (tags_of(dir.path(), "W-1"), tags_of(dir.path(), "W-2")),
        before,
        "a refused rename left a file edited"
    );
}

/// **A reserved word is not a label, so `rename` refuses it** (`A1-213`, audit).
///
/// `tag rename milestone epic` dissolves every cut in the plan, and the only
/// thing `check` then reports is that the former cuts carry no estimate --
/// `APLAN009`, which reads as advice about an estimate rather than as "your
/// plan has no cuts any more". That is the hazard `A1-212` documented as a
/// *typo*, reachable deliberately and across every task at once.
#[test]
fn rename_refuses_the_words_that_decide_what_a_plan_is() {
    let dir = tmp();
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/M-a.md"),
        "---\nid: M-a\ntitle: a cut\nphase: 0\nstatus: todo\n\
         tags:\n  - milestone\n  - s0\nblocked_by:\n  - W-1\n---\n\nbody\n",
    )
    .expect("cut");

    for args in [
        ["tag", "rename", "milestone", "epic"],
        ["tag", "rename", "s0", "release"],
        ["tag", "rename", "cross-cutting", "spans"],
        ["tag", "rename", "legacy", "deprecation"],
        ["tag", "rename", "legacy", "removal"],
    ] {
        let out = common::run(dir.path(), &args);
        assert_ne!(out.code, 0, "{args:?} was allowed: {}", out.text());
        assert!(
            out.text().contains("decide what a plan is"),
            "{args:?}: {}",
            out.text()
        );
    }

    // The cut is still a cut, and an ordinary word still renames.
    let tags = tags_of(dir.path(), "M-a");
    assert!(tags.contains(&"milestone".to_string()), "{tags:?}");
    assert_eq!(
        common::run(dir.path(), &["tag", "rename", "s0", "stage-0"]).code,
        0
    );
}

/// **Taking `milestone` off a cut that holds work is refused** (`A1-213`, audit).
///
/// The rename guard covers the word reaching every task at once; this is the
/// same loss one task at a time, and it is quieter. Membership *is* a
/// non-milestone sitting in a milestone's `blocked_by` (ADR-0013), so the
/// members do not become orphans that something can name -- nothing records
/// that they were ever members. Where `require_cut_membership` is off, which is
/// the default, the only thing `check` then reports is `APLAN009`: that the
/// former cut carries no estimate.
#[test]
fn taking_milestone_off_a_cut_that_holds_work_is_refused() {
    let dir = tmp();
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/M-a.md"),
        "---\nid: M-a\ntitle: a cut\nphase: 0\nstatus: todo\n\
         tags:\n  - milestone\nblocked_by:\n  - W-1\n  - W-2\n---\n\nbody\n",
    )
    .expect("cut");

    let out = common::run(dir.path(), &["tag", "rm", "M-a", "milestone"]);
    assert_ne!(out.code, 0, "the cut was dissolved: {}", out.text());
    assert!(
        out.text().contains("holding 2 task(s)"),
        "the refusal did not say what would be lost: {}",
        out.text()
    );
    assert!(
        tags_of(dir.path(), "M-a").contains(&"milestone".to_string()),
        "the tag was taken off anyway"
    );

    // An empty cut orphans nothing, so it is still allowed: `APLAN021` already
    // describes a milestone with no members, and refusing here would stop a
    // mistagged task being untagged.
    std::fs::write(
        dir.path().join("aplan/tasks/M-b.md"),
        "---\nid: M-b\ntitle: an empty cut\nphase: 0\nstatus: todo\n\
         tags:\n  - milestone\n---\n\nbody\n",
    )
    .expect("empty cut");
    assert_eq!(
        common::run(dir.path(), &["tag", "rm", "M-b", "milestone"]).code,
        0,
        "an empty cut orphans nothing"
    );

    // And a cut whose `blocked_by` names only other cuts holds no work either.
    assert_eq!(
        common::run(dir.path(), &["tag", "add", "M-b", "milestone"]).code,
        0
    );
    let out = common::run(dir.path(), &["tag", "rm", "M-a", "milestone"]);
    assert_ne!(out.code, 0, "{}", out.text());
}

/// **Every command that takes a tag refuses a blank one, in the same words**
/// (`A1-251`, audit).
///
/// `tag add` has said "a tag is a word" since this command existed. `add --tag
/// ""` and `supersede --tag ""` wrote `tags: ['']` instead: a tag no rule
/// reports, nobody meant, and the loader reads as an empty string. One argument
/// reached two answers depending on which command took it, which is the defect
/// this increment kept finding in other shapes.
#[test]
fn a_blank_tag_is_refused_by_every_command_that_takes_one() {
    let dir = tmp();
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/W-3.md"),
        "---\nid: W-3\ntitle: closed\nphase: 0\nstatus: done\neffort_days: 1\n\
         tags:\n  - priority-p0\nevidence:\n  - kind: note\n    ref: reviewed\n\
         \x20   captured: 2026-08-14\n---\n\nbody\n",
    )
    .expect("closed task");

    for (what, args) in [
        ("tag add", vec!["tag", "add", "W-1", ""]),
        ("tag rm", vec!["tag", "rm", "W-1", ""]),
        ("tag rename", vec!["tag", "rename", "legacy", ""]),
        (
            "add",
            vec!["add", "A new task", "--tag", "", "--effort", "1"],
        ),
        (
            "supersede",
            vec!["supersede", "W-3", "A successor", "--tag", ""],
        ),
        // Whitespace is not a word either, and `trim` is what decides it.
        (
            "add, spaces",
            vec!["add", "A task", "--tag", "   ", "--effort", "1"],
        ),
    ] {
        let out = common::run(dir.path(), &args);
        assert_ne!(out.code, 0, "{what} accepted a blank tag: {}", out.text());
        assert!(
            out.text().contains("a tag is a word"),
            "{what} refused a blank tag in different words: {}",
            out.text()
        );
    }
}

/// The no-op and nonsense cases say what happened rather than writing.
#[test]
fn the_cases_that_change_nothing_say_so() {
    let dir = tmp();
    fixture(dir.path());
    let none = common::run(dir.path(), &["tag", "rename", "nobody-has-this", "x"]);
    assert_eq!(none.code, 0, "{}", none.text());
    assert!(none.stdout.contains("no task carries"), "{}", none.stdout);

    let same = common::run(dir.path(), &["tag", "rename", "legacy", "legacy"]);
    assert_ne!(same.code, 0, "{}", same.text());

    let already = common::run(dir.path(), &["tag", "add", "W-1", "legacy"]);
    assert_eq!(already.code, 0, "{}", already.text());
    assert!(
        already.stdout.contains("already carries"),
        "{}",
        already.stdout
    );

    let absent = common::run(dir.path(), &["tag", "rm", "W-1", "never-had-it"]);
    assert_eq!(absent.code, 0, "{}", absent.text());
    assert!(
        absent.stdout.contains("does not carry"),
        "{}",
        absent.stdout
    );

    let blank = common::run(dir.path(), &["tag", "add", "W-1", "  "]);
    assert_ne!(blank.code, 0, "{}", blank.text());
}

/// **Whether a warning blocks is the repository's call, on every form of the
/// command** (`A1-112`).
///
/// Emptying a declared family is `APLAN029`, a warning: nothing carries it any
/// more, which may be a migration in progress or may be the mistake. So the same
/// edit is written or refused depending on what the repository says, and both
/// flags override it in both directions -- for `rm` and for `rename` alike,
/// because a policy that held for one form of a command and not another would be
/// the kind of surprise a verdict flag exists to remove.
#[test]
fn the_verdict_policy_applies_to_every_form_of_the_command() {
    // One task, so removing the member leaves the family carried by nobody:
    // `APLAN029` about the family rather than `APLAN030` about the task.
    let only = |dir: &Path| {
        std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
        std::fs::write(
            dir.join(".aplan.yml"),
            "root: aplan
tag_families:
  priority:
    members: [priority-p0, priority-p1]
",
        )
        .expect("config");
        std::fs::write(
            dir.join("aplan/tasks/W-1.md"),
            "---\nid: W-1\ntitle: work\nphase: 0\nstatus: todo\neffort_days: 1\n\
             tags:\n  - priority-p0\n---\n\nbody\n",
        )
        .expect("task");
    };

    for (verb, args) in [
        ("rm", vec!["tag", "rm", "W-1", "priority-p0"]),
        ("rename", vec!["tag", "rename", "priority-p0", "urgent"]),
    ] {
        // Denied: the warning blocks, and nothing is written.
        let dir = tmp();
        only(dir.path());
        let mut denied = args.clone();
        denied.push("--deny-warnings");
        let out = common::run(dir.path(), &denied);
        assert_ne!(out.code, 0, "{verb} --deny-warnings: {}", out.text());
        assert!(
            tags_of(dir.path(), "W-1").contains(&"priority-p0".to_string()),
            "{verb} --deny-warnings refused and wrote anyway"
        );

        // Allowed: the same edit, with the warning heard and not obeyed.
        let dir = tmp();
        only(dir.path());
        let mut allowed = args.clone();
        allowed.push("--no-deny-warnings");
        let out = common::run(dir.path(), &allowed);
        assert_eq!(out.code, 0, "{verb} --no-deny-warnings: {}", out.text());
        assert!(
            !tags_of(dir.path(), "W-1").contains(&"priority-p0".to_string()),
            "{verb} --no-deny-warnings blocked anyway"
        );
        assert!(out.text().contains("APLAN029"), "{verb}: {}", out.text());
    }
}

/// `--no-deny-warnings` overrides a repository that says warnings block.
#[test]
fn no_deny_warnings_overrides_the_repositorys_own_policy() {
    let dir = tmp();
    fixture(dir.path());
    let cfg = dir.path().join(".aplan.yml");
    let with_deny = std::fs::read_to_string(&cfg).expect("config") + "deny_warnings: true\n";
    std::fs::write(&cfg, with_deny).expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/M-a.md"),
        "---\nid: M-a\ntitle: a cut\nphase: 0\nstatus: todo\n\
         tags:\n  - milestone\nblocked_by:\n  - W-1\n---\n\nbody\n",
    )
    .expect("cut");

    let blocked = common::run(dir.path(), &["tag", "add", "W-1", "cross-cutting"]);
    assert_ne!(
        blocked.code,
        0,
        "the repository said warnings block: {}",
        blocked.text()
    );

    let anyway = common::run(
        dir.path(),
        &["tag", "add", "W-1", "cross-cutting", "--no-deny-warnings"],
    );
    assert_eq!(anyway.code, 0, "{}", anyway.text());
    assert!(tags_of(dir.path(), "W-1").contains(&"cross-cutting".to_string()));
}

/// Putting `cross-cutting` on a task in a cut is *noted*, and blocks only where
/// the repository says a warning blocks (`A1-212`, `A1-112`).
///
/// The post-state check is the same one `check` runs, so the severity is the
/// same too: `APLAN052` is a warning, and a mutation gate that refused warnings
/// on its own would be stricter than the gate it stands in for.
#[test]
fn a_reserved_tag_that_would_contradict_the_plan_is_noted_and_can_block() {
    let dir = tmp();
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/M-a.md"),
        "---\nid: M-a\ntitle: a cut\nphase: 0\nstatus: todo\n\
         tags:\n  - milestone\nblocked_by:\n  - W-1\n---\n\nbody\n",
    )
    .expect("cut");

    let noted = common::run(dir.path(), &["tag", "add", "W-1", "cross-cutting"]);
    assert_eq!(noted.code, 0, "{}", noted.text());
    assert!(noted.text().contains("APLAN052"), "{}", noted.text());
    assert!(
        tags_of(dir.path(), "W-1").contains(&"cross-cutting".to_string()),
        "a warning is reported, not refused"
    );

    assert_eq!(
        common::run(dir.path(), &["tag", "rm", "W-1", "cross-cutting"]).code,
        0
    );
    let refused = common::run(
        dir.path(),
        &["tag", "add", "W-1", "cross-cutting", "--deny-warnings"],
    );
    assert_ne!(refused.code, 0, "{}", refused.text());
    assert!(
        !tags_of(dir.path(), "W-1").contains(&"cross-cutting".to_string()),
        "refused and wrote anyway"
    );
}
