//! `aplan deprecations`, the runway a plan declares (`A1-021`).
//!
//! "Announce the removal in one release, do it in a later one" is the cleanest
//! checkable planning promise there is (FIELD-RESEARCH section 6 P2), and it
//! needs no schema: two ordinary tasks and the `blocked_by` edge between them.
//!
//! What these pin is that the report reads what is written down rather than
//! guessing -- the cut each end lands in, the ways the pair breaks, and
//! the shapes that are *not* a runway (a milestone, a cancelled announcement)
//! -- and that finding a fault never fails a build.

use std::path::Path;

mod common;
use common::aplan;

#[test]
fn a_runway_reports_where_it_was_announced_and_where_it_lands() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    runway(dir.path(), "v0.3", "v0.5");

    let (out, code) = aplan(dir.path(), &["deprecations"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("announced in v0.3"), "{out}");
    assert!(out.contains("removed in v0.5 by T-REM"), "{out}");
    assert!(
        !out.contains(" ! "),
        "a well-formed runway has no fault: {out}"
    );

    let (json, _) = aplan(dir.path(), &["deprecations", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&json).expect("json");
    let runway = &doc["runways"][0];
    assert_eq!(runway["announced"]["cut"], "v0.3", "{json}");
    assert_eq!(runway["removals"][0]["cut"], "v0.5", "{json}");
    assert_eq!(
        runway["faults"].as_array().expect("faults").len(),
        0,
        "{json}"
    );
}

/// The most common real-world deprecation failure: the notice ships and the
/// removal never gets planned.
#[test]
fn an_announcement_with_no_removal_scheduled_is_named() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    task(
        dir.path(),
        "T-ANN",
        "Announce X",
        "todo",
        &["deprecation"],
        &[],
    );

    let (out, code) = aplan(dir.path(), &["deprecations"]);
    assert_eq!(code, 0, "a report is not a verdict: {out}");
    assert!(out.contains("no task is scheduled to remove it"), "{out}");
}

/// A dropped removal is not a scheduled one, and the difference is the whole
/// promise: the plan announced a removal and then cancelled it.
#[test]
fn a_cancelled_removal_is_not_a_scheduled_one() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    task(
        dir.path(),
        "T-ANN",
        "Announce X",
        "todo",
        &["deprecation"],
        &[],
    );
    task(
        dir.path(),
        "T-REM",
        "Remove X",
        "dropped",
        &["removal"],
        &["T-ANN"],
    );

    let (out, _) = aplan(dir.path(), &["deprecations"]);
    assert!(out.contains("no task is scheduled to remove it"), "{out}");
    // And the cancelled task is still shown, so the reader sees it was called
    // off rather than never written.
    assert!(out.contains("T-REM (dropped)"), "{out}");
}

/// The runway *is* the gap. Announced and removed in one cut gives a user no
/// release to migrate in, which is the promise the artifact exists to make.
#[test]
fn a_runway_of_zero_length_is_named_however_it_is_written() {
    let same_cut = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    runway(same_cut.path(), "v0.3", "v0.3");
    let (out, _) = aplan(same_cut.path(), &["deprecations"]);
    assert!(out.contains("no release in between"), "{out}");

    // The same mistake written as one task carrying both tags. Two independent
    // scans would call this an announcement with no removal *and* a removal
    // nobody announced: two findings for one honest mistake.
    let one_task = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(one_task.path());
    task(
        one_task.path(),
        "T-BOTH",
        "Deprecate and remove X at once",
        "todo",
        &["deprecation", "removal"],
        &[],
    );
    let (out, _) = aplan(one_task.path(), &["deprecations"]);
    assert!(out.contains("no release in between"), "{out}");
    assert_eq!(out.matches("T-BOTH").count(), 1, "reported once: {out}");
    assert!(
        !out.contains("nobody was told about"),
        "not also an unannounced removal: {out}"
    );
}

#[test]
fn a_removal_landing_before_its_announcement_is_named() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    runway(dir.path(), "v0.5", "v0.3");
    let (out, _) = aplan(dir.path(), &["deprecations"]);
    assert!(out.contains("before the one announcing it"), "{out}");
}

#[test]
fn a_removal_nothing_announced_is_named_from_the_other_end() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    task(dir.path(), "T-REM", "Remove X", "todo", &["removal"], &[]);

    let (out, code) = aplan(dir.path(), &["deprecations"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("nobody was told about"), "{out}");

    let (json, _) = aplan(dir.path(), &["deprecations", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(doc["unannounced_removals"][0]["id"], "T-REM", "{json}");
}

/// A cut whose label is not a version is a vocabulary, not a missing version,
/// and ordering one against another would invent a finding.
#[test]
fn cuts_this_tool_cannot_order_produce_no_ordering_finding() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    runway(dir.path(), "s3", "s1");
    let (out, _) = aplan(dir.path(), &["deprecations"]);
    assert!(out.contains("announced in s3"), "{out}");
    assert!(out.contains("removed in s1"), "{out}");
    assert!(
        !out.contains("before the one announcing it"),
        "`s1` and `s3` are labels this tool does not order: {out}"
    );
}

/// A plan with no runway is not a plan with a problem. Six real repositories
/// have been measured for this and none declares one.
#[test]
fn a_plan_without_a_runway_says_so_and_explains_the_convention() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    task(dir.path(), "T-1", "Ordinary work", "todo", &[], &[]);

    let (out, code) = aplan(dir.path(), &["deprecations"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("declares no deprecation runway"), "{out}");
    assert!(out.contains("`blocked_by`"), "and how to write one: {out}");
}

/// A cut tagged `deprecation` is not an announcement: a milestone is not work,
/// exactly as every other rollup reads it.
#[test]
fn a_milestone_is_not_an_announcement() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/M-x.md"),
        "---\nid: M-x\ntitle: A cut about deprecations\nphase: 1\nstatus: todo\n\
         tags: [milestone, v0.3, deprecation]\nblocked_by: [T-1]\n---\n\nbody\n",
    )
    .expect("milestone");
    task(dir.path(), "T-1", "Ordinary work", "todo", &[], &[]);

    let (out, _) = aplan(dir.path(), &["deprecations"]);
    assert!(out.contains("declares no deprecation runway"), "{out}");
}

/// A deprecation somebody called off is not a runway with a missing half.
#[test]
fn a_cancelled_announcement_is_not_a_runway() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    task(
        dir.path(),
        "T-ANN",
        "Announce X",
        "dropped",
        &["deprecation"],
        &[],
    );
    let (out, _) = aplan(dir.path(), &["deprecations"]);
    assert!(out.contains("declares no deprecation runway"), "{out}");
}

/// The table a project publishes, which is the artifact the research points at.
#[test]
fn the_markdown_table_is_the_shape_a_project_publishes() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    runway(dir.path(), "v0.3", "v0.5");
    let (out, code) = aplan(dir.path(), &["deprecations", "--format", "markdown"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("| Announcement | Announced in | Removed in |"),
        "{out}"
    );
    assert!(out.contains("| v0.3 | v0.5 | planned |"), "{out}");
    assert!(
        out.contains("do not edit"),
        "a generated file says so: {out}"
    );
}

/// It reads the plan and never git, so it belongs beside `roadmap` rather than
/// in the `verify` family: a directory that is no repository at all still gets
/// an answer.
#[test]
fn it_is_hermetic() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    runway(dir.path(), "v0.3", "v0.5");
    assert!(
        !dir.path().join(".git").exists(),
        "the fixture is deliberately not a repository"
    );
    let (out, code) = aplan(dir.path(), &["deprecations"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("announced in v0.3"), "{out}");
}

/// A removal whose announcement was cancelled was *not* a removal nobody was
/// told about: the announcement is sitting right there in the plan, dropped, and
/// the first version sent the reader looking for one that already existed. It is
/// the mirror of the dropped-removal case, which this had handled and its
/// opposite had not.
#[test]
fn a_removal_outliving_a_cancelled_announcement_is_not_an_unannounced_one() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    task(
        dir.path(),
        "T-ANN",
        "Announce X",
        "dropped",
        &["deprecation"],
        &[],
    );
    task(
        dir.path(),
        "T-REM",
        "Remove X",
        "todo",
        &["removal"],
        &["T-ANN"],
    );

    let (out, code) = aplan(dir.path(), &["deprecations"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !out.contains("blocked by no `deprecation` task"),
        "it was announced, and then the announcement was cancelled: {out}"
    );
    assert!(out.contains("the announcement was cancelled"), "{out}");
    assert!(out.contains("T-ANN"), "and the runway names it: {out}");

    let (json, _) = aplan(dir.path(), &["deprecations", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(
        doc["runways"][0]["faults"][0]["kind"], "announcement_cancelled",
        "{json}"
    );
    assert_eq!(
        doc["unannounced_removals"].as_array().expect("array").len(),
        0,
        "{json}"
    );
}

/// A runway called off at both ends is a decision recorded, not a shape broken.
/// Faulting it would say the announcement was cancelled and the removal was not,
/// beside a second line saying nothing is scheduled to remove it: two complaints
/// about one clean cancellation.
#[test]
fn a_runway_cancelled_at_both_ends_is_not_a_fault() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    task(
        dir.path(),
        "T-ANN",
        "Announce X",
        "dropped",
        &["deprecation"],
        &[],
    );
    task(
        dir.path(),
        "T-REM",
        "Remove X",
        "dropped",
        &["removal"],
        &["T-ANN"],
    );

    let (out, _) = aplan(dir.path(), &["deprecations"]);
    assert!(!out.contains(" ! "), "{out}");
    // Both statuses are on the page, which is how the reader sees it was called
    // off rather than never written.
    assert!(out.contains("(dropped)"), "{out}");
    assert!(out.contains("T-REM (dropped)"), "{out}");
}

/// An announcement nothing points at stays out of the report when it is
/// cancelled: that half is a decision already taken, and reporting it as a
/// runway missing its removal is a finding about the decision.
#[test]
fn a_cancelled_announcement_nothing_points_at_stays_out() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    task(
        dir.path(),
        "T-ANN",
        "Announce X",
        "dropped",
        &["deprecation"],
        &[],
    );
    let (out, _) = aplan(dir.path(), &["deprecations"]);
    assert!(out.contains("declares no deprecation runway"), "{out}");
}

/// A format this command does not take names the ones it does. `Format`'s own
/// error lists every rendering the *tool* has, which for this command is three
/// it refuses and not the one the reader was reaching for.
#[test]
fn an_unaccepted_format_names_the_ones_this_command_takes() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    runway(dir.path(), "v0.3", "v0.5");

    for bad in ["sarif", "markdwn"] {
        let (out, code) = aplan(dir.path(), &["deprecations", "--format", bad]);
        assert_ne!(code, 0, "{out}");
        assert!(
            out.contains("`human`, `json` or `markdown`"),
            "{bad}: {out}"
        );
        assert!(out.contains(bad), "and what was given: {out}");
        assert!(!out.contains("sarif, "), "not the tool's whole list: {out}");
    }
}

/// Cut versions order numerically, not as text. A string compare calls `v2.0`
/// later than `v10.0`, which is the ordering bug every version-sorting tool has
/// had once, and it would report a correct runway as running backwards.
#[test]
fn cut_versions_order_by_number_not_by_text() {
    let backwards = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    runway(backwards.path(), "v10.0", "v9.9");
    let (out, _) = aplan(backwards.path(), &["deprecations"]);
    assert!(out.contains("before the one announcing it"), "{out}");

    let forwards = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    runway(forwards.path(), "v2.0", "v10.0");
    let (out, _) = aplan(forwards.path(), &["deprecations"]);
    assert!(
        !out.contains(" ! "),
        "`v10.0` ships after `v2.0`, whatever a string compare says: {out}"
    );
}

/// One removal retiring two announcements appears under both, because both are
/// promises somebody was given.
#[test]
fn one_removal_may_close_more_than_one_announcement() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    task(
        dir.path(),
        "T-A1",
        "Announce one",
        "todo",
        &["deprecation"],
        &[],
    );
    task(
        dir.path(),
        "T-A2",
        "Announce two",
        "todo",
        &["deprecation"],
        &[],
    );
    task(
        dir.path(),
        "T-REM",
        "Remove both",
        "todo",
        &["removal"],
        &["T-A1", "T-A2"],
    );

    let (out, _) = aplan(dir.path(), &["deprecations"]);
    assert_eq!(out.matches("by T-REM").count(), 2, "{out}");
    assert!(out.contains("2 runway(s)"), "{out}");
    assert!(
        !out.contains("blocked by no `deprecation` task"),
        "it announces both: {out}"
    );
}

/// The published table is markdown, and a title carrying a `|` must not split
/// the row it is in.
#[test]
fn a_pipe_in_a_title_does_not_break_the_published_table() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    task(
        dir.path(),
        "T-ANN",
        "Announce `a|b` is going",
        "todo",
        &["deprecation"],
        &[],
    );
    task(
        dir.path(),
        "T-REM",
        "Remove it",
        "todo",
        &["removal"],
        &["T-ANN"],
    );

    let (out, _) = aplan(dir.path(), &["deprecations", "--format", "markdown"]);
    // Not `contains("Announce")`: the header cell is `Announcement`, and
    // matching it made this test read the wrong line.
    let row = out
        .lines()
        .find(|l| l.contains("is going"))
        .unwrap_or_else(|| panic!("no row in {out}"));
    assert!(row.contains(r"a\|b"), "{row}");
    assert_eq!(
        row.matches('|').count() - row.matches(r"\|").count(),
        5,
        "four columns, five delimiters: {row}"
    );
}

fn plan(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
}

fn task(dir: &Path, id: &str, title: &str, status: &str, tags: &[&str], blocked_by: &[&str]) {
    let tags = if tags.is_empty() {
        String::new()
    } else {
        format!("tags: [{}]\n", tags.join(", "))
    };
    let blocked = if blocked_by.is_empty() {
        String::new()
    } else {
        format!("blocked_by: [{}]\n", blocked_by.join(", "))
    };
    std::fs::write(
        dir.join(format!("aplan/tasks/{id}.md")),
        format!(
            "---\nid: {id}\ntitle: {title}\nphase: 1\nstatus: {status}\neffort_days: 1\n\
             {tags}{blocked}---\n\nbody\n"
        ),
    )
    .expect("task");
}

/// An announcement in `from` and a removal in `to`, each a member of its cut.
fn runway(dir: &Path, from: &str, to: &str) {
    plan(dir);
    task(
        dir,
        "T-ANN",
        "Announce X is going away",
        "todo",
        &["deprecation"],
        &[],
    );
    task(dir, "T-REM", "Remove X", "todo", &["removal"], &["T-ANN"]);
    if from == to {
        cut(dir, from, &["T-ANN", "T-REM"]);
    } else {
        cut(dir, from, &["T-ANN"]);
        cut(dir, to, &["T-REM"]);
    }
}

fn cut(dir: &Path, label: &str, members: &[&str]) {
    std::fs::write(
        dir.join(format!("aplan/tasks/M-{label}.md")),
        format!(
            "---\nid: M-{label}\ntitle: The {label} cut\nphase: 1\nstatus: todo\n\
             tags: [milestone, {label}]\nblocked_by: [{}]\n---\n\nbody\n",
            members.join(", ")
        ),
    )
    .expect("milestone");
}

/// **This repository's own announced deprecation says so where a reader meets
/// it** (`A1-235`).
///
/// A runway is a promise, and `aplan deprecations` reporting one is the *plan's*
/// record of it. That is not where a user finds out: they run the command. The
/// failure this guards is the announcement being quietly dropped between the cut
/// that announced it and the cut that removes the behaviour -- at which point the
/// removal arrives unannounced no matter what the plan says.
///
/// It names the flag that will *restore* today's view, because an announcement
/// that only says a default is changing leaves a scripted caller with nothing to
/// do about it.
#[test]
fn the_announced_tree_default_is_in_the_command_that_will_change() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let help = common::run(dir.path(), &["tree", "--help"]);
    // Only the description, not the whole page. `--live` is listed under
    // `Options:` whatever the description says, so asserting against the full
    // text passed with the announcement's mention of it deleted -- measured, by
    // deleting it.
    let text = help
        .stdout
        .split("Usage:")
        .next()
        .expect("a description before the usage line")
        .to_string();
    // Deliberately not the task id: help is read beside the consumer's own
    // `docs/adr/` and this project's numbers resolve to nothing there (`A1-044`,
    // pinned by `no_help_text_cites_this_projects_adrs`, which refused the first
    // version of this announcement). What a reader needs is the guarantee and
    // the flag that restores what they have.
    for wanted in ["--live", "--all"] {
        assert!(
            text.contains(wanted),
            "`tree --help` does not mention {wanted}, so the announced default change is \
             invisible to the person it will surprise:\n{text}"
        );
    }
    // And it says the change has not happened, so a reader of today's binary is
    // not told to expect an output they will not get.
    assert!(
        text.contains("Announced") && text.contains("later cut"),
        "the help does not say the change is still ahead:\n{text}"
    );
}
