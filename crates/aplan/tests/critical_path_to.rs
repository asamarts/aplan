//! `aplan critical-path <ID>`: the longest chain that ends at one checkpoint
//! (`A1-345`).
//!
//! Driven through the built binary rather than through `Graph::critical_path_to`,
//! for `A1-107`'s reason: the unit tests beside the DP prove the arithmetic, and
//! what is under test here is the seam -- the argument reaching the graph, the
//! target reaching the renderer, and the two questions staying distinguishable
//! in what a reader and a consumer actually receive.

use std::path::Path;

use crate::common;

/// A plan where the whole-plan chain and the chain to `M-a` are different
/// numbers, so a command answering the wrong one is caught rather than agreeing
/// by coincidence.
///
/// ```text
///   OUT (5) --> A (1) --> M-a        M-a: cut a, holds A
///   HEAVY (40) ----------> M-b       M-b: cut b, holds HEAVY
///                          R         release, waits on M-a only
/// ```
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    let task = |id: &str, effort: f64, blocked: &[&str]| {
        let deps = if blocked.is_empty() {
            String::new()
        } else {
            let rows: Vec<String> = blocked.iter().map(|b| format!("  - {b}\n")).collect();
            format!("blocked_by:\n{}", rows.concat())
        };
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: task {id}\nphase: 1\nstatus: todo\n\
                 effort_days: {effort}\n{deps}---\n\nbody\n"
            ),
        )
        .expect("task");
    };
    let milestone = |id: &str, tags: &str, blocked: &[&str]| {
        let rows: Vec<String> = blocked.iter().map(|b| format!("  - {b}\n")).collect();
        let deps = if blocked.is_empty() {
            String::new()
        } else {
            format!("blocked_by:\n{}", rows.concat())
        };
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: milestone {id}\nphase: 1\nstatus: todo\n\
                 tags:\n{tags}{deps}---\n\nbody\n"
            ),
        )
        .expect("milestone");
    };
    task("OUT", 5.0, &[]);
    task("A", 1.0, &["OUT"]);
    task("HEAVY", 40.0, &[]);
    milestone("M-a", "  - milestone\n  - a\n", &["A"]);
    milestone("M-b", "  - milestone\n  - b\n", &["HEAVY"]);
    milestone("R", "  - milestone\n  - release\n", &["M-a"]);
}

fn tmp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir")
}

fn json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut all: Vec<&str> = vec!["critical-path"];
    all.extend(args);
    all.extend(["--format", "json"]);
    let out = common::run(dir, &all);
    assert_eq!(out.code, 0, "{}", out.text());
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", out.stdout))
}

fn ids(doc: &serde_json::Value) -> Vec<String> {
    doc["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .filter_map(|t| t["id"].as_str())
        .map(str::to_string)
        .collect()
}

/// Everything below rests on this fixture, so it has to be a plan `check`
/// accepts. A test whose fixture is refused proves something about a shape
/// nobody can author (`A1-345`, audit).
#[test]
fn the_fixture_is_a_plan_check_accepts() {
    let dir = tmp();
    fixture(dir.path());
    let out = common::run(dir.path(), &["check"]);
    assert_eq!(out.code, 0, "{}", out.text());
}

/// **One derivation, two surfaces.** Every row whose chain is bounded by its own
/// checkpoint must report the number `critical-path <that id>` reports. Two code
/// paths answering one question is the defect this repository keeps finding, and
/// this is the gate against introducing it here.
#[test]
fn every_borrowing_row_agrees_with_the_command() {
    let dir = tmp();
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/C-top.md"),
        "---\nid: C-top\ntitle: everything\nphase: 1\nstatus: todo\n\
         tags:\n  - milestone\n  - top\ncontains:\n  - M-a\n  - M-b\n\
         blocked_by:\n  - M-a\n  - M-b\n---\n\nbody\n",
    )
    .expect("container");
    let roadmap: serde_json::Value =
        serde_json::from_str(&common::run(dir.path(), &["roadmap", "--format", "json"]).stdout)
            .expect("json");
    let mut checked = 0;
    for cut in roadmap["cuts"].as_array().expect("cuts") {
        let id = cut["milestone"].as_str().expect("milestone");
        let bounded = cut["critical_path_to"].as_str();
        if bounded.is_none() {
            // A plain cut keeps the member-induced number, which is a different
            // question and must *not* be expected to match.
            continue;
        }
        assert_eq!(
            bounded,
            Some(id),
            "a row is bounded by something else: {cut}"
        );
        let doc = json(dir.path(), &[id]);
        assert_eq!(
            doc["total_effort_days"], cut["critical_path_days"],
            "`{id}`: the row and the command disagree\nrow {cut}\ncommand {doc}"
        );
        assert_eq!(
            doc["tasks"]
                .as_array()
                .expect("tasks")
                .iter()
                .filter_map(|t| t["id"].as_str())
                .collect::<Vec<_>>(),
            cut["critical_path"]
                .as_array()
                .expect("critical_path")
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>(),
            "`{id}`: same number, different spine"
        );
        checked += 1;
    }
    assert!(
        checked >= 2,
        "only {checked} bounded row(s): the gate is vacuous"
    );
}

/// The acceptance criterion: a caller names a checkpoint and receives the chain
/// terminating there, not the plan's longest chain.
#[test]
fn a_named_target_gets_the_chain_that_ends_at_it() {
    let dir = tmp();
    fixture(dir.path());
    let doc = json(dir.path(), &["M-a"]);
    assert_eq!(ids(&doc), ["OUT", "A"], "{doc}");
    assert_eq!(doc["total_effort_days"], serde_json::json!(6.0), "{doc}");
}

/// Naming a checkpoint is not selecting a subset (`A1-214`): `OUT` lives outside
/// cut `a`, and a filter-first answer would report 1.0d rather than 6.0d.
#[test]
fn a_prerequisite_outside_the_target_group_still_counts() {
    let dir = tmp();
    fixture(dir.path());
    let doc = json(dir.path(), &["M-a"]);
    assert!(
        ids(&doc).contains(&"OUT".to_string()),
        "a blocker that crosses the cut boundary was dropped: {doc}"
    );
}

/// The release row's finding, at the command: what a release waits on is not the
/// longest chain in the plan, and both numbers stay reachable.
#[test]
fn the_release_chain_and_the_whole_plan_chain_are_different_answers() {
    let dir = tmp();
    fixture(dir.path());
    let release = json(dir.path(), &["R"]);
    let whole = json(dir.path(), &[]);
    assert_eq!(ids(&release), ["OUT", "A"], "{release}");
    assert_eq!(release["total_effort_days"], serde_json::json!(6.0));
    assert_eq!(ids(&whole), ["HEAVY"], "{whole}");
    assert_eq!(whole["total_effort_days"], serde_json::json!(40.0));
}

/// A consumer must be able to tell the two apart from the document alone: the
/// numbers do not say which question was asked.
#[test]
fn the_json_names_the_target_and_says_null_for_the_whole_plan() {
    let dir = tmp();
    fixture(dir.path());
    assert_eq!(json(dir.path(), &["M-a"])["to"], serde_json::json!("M-a"));
    assert_eq!(json(dir.path(), &[])["to"], serde_json::Value::Null);
}

/// The milestone is a checkpoint, not work: it is never a row and never adds
/// effort to the total of the chain that ends at it.
#[test]
fn the_target_milestone_is_not_itself_a_row() {
    let dir = tmp();
    fixture(dir.path());
    let doc = json(dir.path(), &["M-a"]);
    assert!(!ids(&doc).contains(&"M-a".to_string()), "{doc}");
}

/// Naming a task is the same question one rung down, and the task itself is the
/// chain's last row.
#[test]
fn a_task_target_ends_the_chain_with_that_task() {
    let dir = tmp();
    fixture(dir.path());
    let doc = json(dir.path(), &["A"]);
    assert_eq!(ids(&doc), ["OUT", "A"], "{doc}");
    assert_eq!(doc["total_effort_days"], serde_json::json!(6.0));
}

/// A checkpoint nothing leads to is an answer, not a failure, and the human form
/// says which.
#[test]
fn a_target_with_no_ancestry_says_so_rather_than_printing_nothing() {
    let dir = tmp();
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/M-c.md"),
        "---\nid: M-c\ntitle: an empty cut\nphase: 1\nstatus: todo\n\
         tags:\n  - milestone\n  - c\n---\n\nbody\n",
    )
    .expect("milestone");
    let out = common::run(dir.path(), &["critical-path", "M-c"]);
    assert_eq!(out.code, 0, "{}", out.text());
    assert!(
        out.stdout.contains("M-c") && out.stdout.contains("none"),
        "an empty chain printed nothing a reader can act on: {}",
        out.stdout
    );
    let doc = json(dir.path(), &["M-c"]);
    assert_eq!(doc["to"], serde_json::json!("M-c"));
    assert_eq!(doc["total_effort_days"], serde_json::json!(0.0), "{doc}");
    assert!(ids(&doc).is_empty(), "{doc}");
}

/// The human form names the target too, or a reader with two terminals open
/// cannot tell which answer is on screen.
#[test]
fn the_human_form_names_the_target() {
    let dir = tmp();
    fixture(dir.path());
    let named = common::run(dir.path(), &["critical-path", "M-a"]);
    assert_eq!(named.code, 0, "{}", named.text());
    assert!(named.stdout.contains("M-a"), "{}", named.stdout);
    let whole = common::run(dir.path(), &["critical-path"]);
    assert_eq!(whole.code, 0, "{}", whole.text());
    assert!(
        whole.stdout.starts_with("critical path:"),
        "the whole-plan answer claimed a target: {}",
        whole.stdout
    );
}

/// An id that is not in the plan is a refusal that names it, not an empty chain
/// that reads as "nothing blocks this".
#[test]
fn an_unknown_target_is_refused_and_named() {
    let dir = tmp();
    fixture(dir.path());
    let out = common::run(dir.path(), &["critical-path", "NOPE"]);
    assert_ne!(out.code, 0, "{}", out.text());
    assert!(out.stderr.contains("NOPE"), "{}", out.stderr);
}

/// Work that is not going to happen has no chain leading to it, and saying
/// "0 days" about a dropped checkpoint would read as "ready to go".
#[test]
fn a_dropped_target_is_refused_with_its_reason() {
    let dir = tmp();
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/A.md"),
        "---\nid: A\ntitle: task A\nphase: 1\nstatus: dropped\neffort_days: 1\n\
         blocked_by:\n  - OUT\n---\n\nbody\n",
    )
    .expect("task");
    let out = common::run(dir.path(), &["critical-path", "A"]);
    assert_ne!(out.code, 0, "{}", out.text());
    assert!(
        out.stderr.contains("dropped"),
        "the refusal did not say why: {}",
        out.stderr
    );
}

/// A container holds cuts through `contains` and waits on them, which is the
/// only shape `APLAN047` accepts. The chain to it is the chain through the work
/// beneath it, and the roadmap row must agree with the command.
#[test]
fn a_container_reaches_the_work_it_holds() {
    let dir = tmp();
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/C-top.md"),
        "---\nid: C-top\ntitle: everything\nphase: 1\nstatus: todo\n\
         tags:\n  - milestone\n  - top\ncontains:\n  - M-a\n  - M-b\n\
         blocked_by:\n  - M-a\n  - M-b\n---\n\nbody\n",
    )
    .expect("container");
    // The fixture has to be one `check` accepts, or the test proves nothing
    // about a plan anybody can have (`A1-345`, audit).
    let checked = common::run(dir.path(), &["check"]);
    assert_eq!(
        checked.code,
        0,
        "the fixture is not a valid plan: {}",
        checked.text()
    );
    let doc = json(dir.path(), &["C-top"]);
    assert_eq!(doc["to"], serde_json::json!("C-top"));
    assert_eq!(
        ids(&doc),
        ["HEAVY"],
        "the container took the heavier of the two branches it holds: {doc}"
    );
    assert_eq!(doc["total_effort_days"], serde_json::json!(40.0), "{doc}");

    // And the roadmap row agrees, because both ask the same question of the
    // same walk: a number on a page and a chain a reader can print must not be
    // two derivations.
    let roadmap: serde_json::Value =
        serde_json::from_str(&common::run(dir.path(), &["roadmap", "--format", "json"]).stdout)
            .expect("json");
    let top = roadmap["cuts"]
        .as_array()
        .expect("cuts")
        .iter()
        .find(|c| c["milestone"] == "C-top")
        .expect("the container row");
    assert_eq!(top["critical_path_days"], serde_json::json!(40.0), "{top}");
    assert_eq!(top["critical_path_to"], serde_json::json!("C-top"), "{top}");
    assert_eq!(top["release"], serde_json::json!(false), "{top}");
}

/// A chain ending at a late checkpoint runs through work that is already
/// finished, and a total with no status beside it reads as work remaining.
#[test]
fn the_rows_say_which_of_them_are_already_done() {
    let dir = tmp();
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/OUT.md"),
        "---\nid: OUT\ntitle: task OUT\nphase: 1\nstatus: done\neffort_days: 5\n\
         evidence:\n  - kind: note\n    ref: shipped\n---\n\nbody\n",
    )
    .expect("task");
    let out = common::run(dir.path(), &["critical-path", "M-a"]);
    assert_eq!(out.code, 0, "{}", out.text());
    let done = out
        .stdout
        .lines()
        .find(|l| l.contains("OUT"))
        .unwrap_or_else(|| panic!("no OUT row: {}", out.stdout));
    assert!(done.contains("done"), "{done}");
    let todo = out
        .stdout
        .lines()
        .find(|l| l.trim_start().starts_with('A'))
        .unwrap_or_else(|| panic!("no A row: {}", out.stdout));
    assert!(todo.contains("todo"), "{todo}");
}

/// **What is left is a second number, and it is re-walked** (`A1-352`).
///
/// The fixture's whole-plan chain is `OUT -> A` at 6.0d with nothing done, so
/// making `OUT` done leaves 1.0d on that chain -- while `HEAVY` at 40.0d is
/// untouched and is the real bound on what remains. Subtracting would answer
/// 1.0d; the walk answers 40.0d.
#[test]
fn what_is_left_is_measured_rather_than_subtracted() {
    let dir = tmp();
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/OUT.md"),
        "---\nid: OUT\ntitle: task OUT\nphase: 1\nstatus: done\neffort_days: 5\n\
         evidence:\n  - kind: note\n    ref: shipped\n---\n\nbody\n",
    )
    .expect("task");
    let doc = json(dir.path(), &["M-a"]);
    assert_eq!(doc["total_effort_days"], serde_json::json!(6.0), "{doc}");
    assert_eq!(
        doc["remaining_effort_days"],
        serde_json::json!(1.0),
        "{doc}"
    );
    assert_eq!(
        doc["remaining"],
        serde_json::json!(["A"]),
        "a finished row is not work left: {doc}"
    );

    // The whole-plan form: the remaining chain ends somewhere else entirely.
    let whole = json(dir.path(), &[]);
    assert_eq!(whole["total_effort_days"], serde_json::json!(40.0));
    assert_eq!(whole["remaining_effort_days"], serde_json::json!(40.0));
}

/// A reader sees both numbers without asking for the second one.
#[test]
fn the_human_header_says_how_much_is_left() {
    let dir = tmp();
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/OUT.md"),
        "---\nid: OUT\ntitle: task OUT\nphase: 1\nstatus: done\neffort_days: 5\n\
         evidence:\n  - kind: note\n    ref: shipped\n---\n\nbody\n",
    )
    .expect("task");
    let out = common::run(dir.path(), &["critical-path", "M-a"]);
    assert_eq!(out.code, 0, "{}", out.text());
    let head = out.stdout.lines().next().unwrap_or_default();
    assert!(
        head.contains("6.0 days") && head.contains("1.0d of it not yet done"),
        "both quantities belong on the header: {head}"
    );
}

/// A chain with nothing left says so, rather than printing a number a reader
/// has to compare against the first one to understand.
#[test]
fn a_finished_chain_says_none_of_it_is_left() {
    let dir = tmp();
    fixture(dir.path());
    for (id, effort) in [("OUT", 5), ("A", 1)] {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: task {id}\nphase: 1\nstatus: done\n\
                 effort_days: {effort}\n{}evidence:\n  - kind: note\n    ref: shipped\n\
                 ---\n\nbody\n",
                if id == "A" {
                    "blocked_by:\n  - OUT\n"
                } else {
                    ""
                }
            ),
        )
        .expect("task");
    }
    let out = common::run(dir.path(), &["critical-path", "M-a"]);
    assert_eq!(out.code, 0, "{}", out.text());
    let head = out.stdout.lines().next().unwrap_or_default();
    assert!(head.contains("none of it is left"), "{head}");
    let doc = json(dir.path(), &["M-a"]);
    assert_eq!(
        doc["remaining_effort_days"],
        serde_json::json!(0.0),
        "{doc}"
    );
    assert_eq!(doc["remaining"], serde_json::json!([]), "{doc}");
}

/// A chain with nothing finished says one number, not the same number twice.
#[test]
fn a_chain_with_nothing_done_does_not_repeat_its_own_total() {
    let dir = tmp();
    fixture(dir.path());
    let out = common::run(dir.path(), &["critical-path", "M-a"]);
    assert_eq!(out.code, 0, "{}", out.text());
    let head = out.stdout.lines().next().unwrap_or_default();
    assert!(
        !head.contains("not yet done") && !head.contains("none of it is left"),
        "nothing is done, so the second number would say the first again: {head}"
    );
}

/// An unestimated live task is work left, and says so (`A1-352`, audit).
///
/// The row filter asked whether a task *weighs* anything, so a `todo` task with
/// no `effort_days` was read as finished and dropped -- and the endpoint search
/// asked the same question, so on a plan whose live work carried no estimates it
/// ended the chain at a finished task and reported nothing left at all.
/// `APLAN009` reports a missing estimate as a warning precisely because plans
/// carry them.
#[test]
fn unestimated_live_work_is_reported_as_left() {
    let dir = tmp();
    fixture(dir.path());
    // OUT is finished; A is live and carries no estimate.
    std::fs::write(
        dir.path().join("aplan/tasks/OUT.md"),
        "---\nid: OUT\ntitle: task OUT\nphase: 1\nstatus: done\neffort_days: 5\n\
         evidence:\n  - kind: note\n    ref: shipped\n---\n\nbody\n",
    )
    .expect("task");
    std::fs::write(
        dir.path().join("aplan/tasks/A.md"),
        "---\nid: A\ntitle: task A\nphase: 1\nstatus: todo\nblocked_by:\n  - OUT\n---\n\nbody\n",
    )
    .expect("task");
    let doc = json(dir.path(), &["M-a"]);
    assert_eq!(
        doc["remaining"],
        serde_json::json!(["A"]),
        "an unestimated live task vanished from what is left: {doc}"
    );
    assert_eq!(
        doc["remaining_effort_days"],
        serde_json::json!(0.0),
        "{doc}"
    );
    let out = common::run(dir.path(), &["critical-path", "M-a"]);
    let head = out.stdout.lines().next().unwrap_or_default();
    assert!(
        head.contains("0.0d of it not yet done across 1 task(s)"),
        "the row is counted even though it weighs nothing: {head}"
    );
}

/// The unqualified command is unchanged, which is what makes this additive.
#[test]
fn the_unqualified_command_still_answers_about_the_whole_plan() {
    let dir = tmp();
    fixture(dir.path());
    let doc = json(dir.path(), &[]);
    assert_eq!(ids(&doc), ["HEAVY"], "{doc}");
    assert_eq!(doc["schema"], serde_json::json!("aplan/critical-path/v1"));
}

/// Selecting a subset is still refused, and the help says why the endpoint form
/// is a different thing rather than leaving a reader to guess (`A1-214`).
#[test]
fn a_subset_is_still_refused_and_the_help_explains_the_difference() {
    let dir = tmp();
    fixture(dir.path());
    let out = common::run(dir.path(), &["critical-path", "--cut", "a"]);
    assert_ne!(out.code, 0, "a subset was accepted: {}", out.text());
    let help = common::run(dir.path(), &["critical-path", "--help"]);
    assert_eq!(help.code, 0, "{}", help.text());
    let text = help.stdout.to_lowercase();
    for word in ["filter", "end"] {
        assert!(
            text.contains(word),
            "the help does not explain `{word}`: {}",
            help.stdout
        );
    }
}

/// A cut's `crit` in `roadmap` is a third number: the longest chain among that
/// cut's own members. It is unlabelled, being the column's original meaning, so
/// a reader who checks one against the other finds two numbers for one id -- and
/// the help is where they would look.
#[test]
fn the_help_says_why_a_cut_row_reports_a_different_number() {
    let dir = tmp();
    fixture(dir.path());
    let help = common::run(dir.path(), &["critical-path", "--help"]);
    assert_eq!(help.code, 0, "{}", help.text());
    let text = help.stdout.to_lowercase();
    for phrase in ["roadmap", "to here", "members"] {
        assert!(
            text.contains(phrase),
            "the help does not connect the two: `{phrase}` missing from {}",
            help.stdout
        );
    }

    // And the numbers really are different, or the sentence is describing
    // nothing: cut `a` holds A alone, while the chain to it runs through OUT.
    let doc = json(dir.path(), &["M-a"]);
    let roadmap: serde_json::Value =
        serde_json::from_str(&common::run(dir.path(), &["roadmap", "--format", "json"]).stdout)
            .expect("json");
    let cut = roadmap["cuts"]
        .as_array()
        .expect("cuts")
        .iter()
        .find(|c| c["milestone"] == "M-a")
        .expect("the cut row");
    assert_eq!(cut["critical_path_days"], serde_json::json!(1.0), "{cut}");
    assert!(
        cut["critical_path_to"].is_null(),
        "a cut is not bounded: {cut}"
    );
    assert_eq!(doc["total_effort_days"], serde_json::json!(6.0), "{doc}");
}
