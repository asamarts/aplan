//! Grouping is not constraint, and this plan is mostly grouping (`A1-216`).
//!
//! ADR-0008 models a cut as a meta-task that is `blocked_by` its members. That
//! is a good decision for the model and it makes one field carry two different
//! facts, and `graph` drew them as the same thing: three quarters of what it
//! drew was grouping, which `roadmap` already prints as a heading.
//!
//! The consequence was arithmetic rather than taste. Layered by longest path the
//! whole plan was nine layers with a widest layer of 177, and no layout fits
//! that on a screen. The reasoning behind the task rests on that ratio, so the
//! ratio is a test: **the numbers move as the plan grows, and the shape of the
//! claim must not**.

use std::path::{Path, PathBuf};

mod common;

/// The repository's own plan, which is the plan the task measured.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the workspace root")
        .to_path_buf()
}

fn graph_json(args: &[&str]) -> serde_json::Value {
    let root = workspace_root();
    let mut all: Vec<&str> = vec!["graph", "--format", "json"];
    all.extend(args);
    let out = common::output(&root, &all);
    assert!(
        out.status.success(),
        "`aplan {}` failed: {}",
        all.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("graph --format json is JSON")
}

/// `aplan tasks --format json`, which carries the `milestone` tag as a field.
fn tasks_json() -> serde_json::Value {
    let out = common::output(&workspace_root(), &["tasks", "--format", "json"]);
    serde_json::from_slice(&out.stdout).expect("tasks --format json is JSON")
}

fn count(doc: &serde_json::Value, field: &str) -> usize {
    doc[field].as_array().expect("an array").len()
}

/// Most of what `blocked_by` records in this plan is grouping.
///
/// The task's own measurement, taken again on whatever the plan is now. If this
/// fails the plan has changed shape, and the reasoning that put grouping behind
/// a flag deserves re-reading rather than a new number here.
#[test]
fn most_of_this_plans_edges_are_grouping_rather_than_constraint() {
    let whole = graph_json(&["--membership"]);
    let deps = graph_json(&[]);

    let all_edges = count(&whole, "edges");
    let dependency_edges = count(&deps, "edges");
    let membership_edges = all_edges - dependency_edges;

    assert!(
        membership_edges > dependency_edges,
        "grouping is no longer the majority of this plan's edges: {membership_edges} \
         membership against {dependency_edges} dependency. The task behind `--membership` \
         reasoned from that ratio; re-read it rather than adjusting this number."
    );
    assert_eq!(
        membership_edges, deps["grouping"]["edges"],
        "the count the view reports disagrees with the difference between the two views"
    );
}

/// Narrowing to dependencies makes the graph a different, smaller object -- the
/// claim that made this task the precondition for every visual improvement.
#[test]
fn narrowing_to_dependencies_shrinks_the_graph_by_more_than_half() {
    let whole = graph_json(&["--membership"]);
    let deps = graph_json(&[]);

    let (all_nodes, kept_nodes) = (count(&whole, "nodes"), count(&deps, "nodes"));
    assert!(
        kept_nodes * 2 < all_nodes,
        "the dependency graph is no longer much smaller than the whole: {kept_nodes} of \
         {all_nodes} nodes"
    );
    assert_eq!(
        all_nodes - kept_nodes,
        deps["grouping"]["tasks"],
        "the tasks left out are not the tasks reported as left out"
    );
}

/// The narrowed view says what it left out, in every shape.
///
/// A default that silently omitted three quarters of the plan's edges would be
/// the same false completeness this repository refuses elsewhere.
#[test]
fn every_shape_says_what_narrowing_left_out() {
    let deps = graph_json(&[]);
    let left_out = deps["grouping"]["edges"].as_u64().expect("a count");
    assert!(left_out > 0, "nothing was left out, so this proves nothing");

    let root = workspace_root();
    for (format, marker) in [("mermaid", "%%"), ("dot", "//"), ("text", "")] {
        let out = common::output(&root, &["graph", "--format", format]);
        let text = String::from_utf8_lossy(&out.stdout);
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains(&format!("{left_out} cut-membership edge(s)")),
            "`{format}` does not say how many grouping edges it left out"
        );
        assert!(
            flat.contains("`--membership` draws them."),
            "`{format}` does not say how to see them"
        );
        if !marker.is_empty() {
            assert!(
                text.lines().any(|l| l.trim_start().starts_with(marker)),
                "`{format}` does not say it in its own comment syntax"
            );
        }
    }
}

/// A cut lists its members; everything else is a constraint a reader wants
/// drawn, including one cut sequenced after another (ADR-0013).
#[test]
fn cut_sequencing_is_a_dependency_not_a_membership() {
    let whole = graph_json(&["--membership"]);
    // From the `milestone_task` field rather than from the id: an id prefix is
    // a convention this plan happens to follow, and the classification under
    // test reads the `milestone` *tag*. A test that guessed differently from
    // the code would agree with it only by luck.
    let listing = tasks_json();
    let milestones: Vec<&str> = listing["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .filter(|t| t["milestone_task"] == serde_json::json!(true))
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert!(milestones.len() > 1, "the plan has no cuts to sequence");

    let mut sequencing = 0_usize;
    for edge in whole["edges"].as_array().expect("edges") {
        let (from, to) = (
            edge["from"].as_str().expect("from"),
            edge["to"].as_str().expect("to"),
        );
        if milestones.contains(&from) && milestones.contains(&to) {
            sequencing += 1;
            assert_eq!(
                edge["kind"], "dependency",
                "a cut sequenced after another is a constraint, not grouping: {from} -> {to}"
            );
        }
        if !milestones.contains(&from) && milestones.contains(&to) {
            assert_eq!(
                edge["kind"], "membership",
                "a cut listing a member is grouping: {from} -> {to}"
            );
        }
    }
    assert!(
        sequencing > 0,
        "no cut is sequenced after another, so that arm is untested"
    );
}

/// The distinction lives in one place, and stays there.
///
/// `A1-216`'s first criterion is that a membership edge is distinguishable from
/// a dependency edge *in one place*. It was not: the predicate
/// `to.is_milestone() && !from.is_milestone()` was written out by hand in six
/// more, including two rules in `validate`, the cut-sequencing derivation in
/// `graph`, the roadmap's member rollup, and the note `tree` prints beside a
/// cut. Each agreed with the others by coincidence, and the task's own Why says
/// the derivation "lives in one place if this task puts it there, and in every
/// consumer if it does not".
///
/// A source scan, because there is no type that can enforce this: the shape it
/// looks for is the predicate itself, spelled any of the ways it was spelled.
#[test]
fn nothing_re_derives_what_edge_kind_answers() {
    let root = workspace_root();
    let mut offenders = Vec::new();
    let mut scanned = 0_usize;

    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "target") {
                    stack.push(path);
                }
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            scanned += 1;
            // The one place allowed to say it is the place that answers it --
            // and this file, which has to name the shapes to look for them.
            if path.ends_with("graph.rs") || path.ends_with("edge_kinds.rs") {
                continue;
            }
            // Collapsed to one line, so a predicate broken across lines by
            // `rustfmt` is still recognised.
            let flat = source.split_whitespace().collect::<Vec<_>>().join(" ");
            for shape in [
                "is_milestone() && !dep.is_milestone()",
                "is_milestone() && !from.is_milestone()",
                "is_milestone() && !task.is_milestone()",
                "is_milestone() && !graph.get(parent).is_some_and(Task::is_milestone)",
            ] {
                if flat.contains(shape) {
                    offenders.push(format!("{}: {shape}", path.display()));
                }
            }
        }
    }

    assert!(scanned > 20, "only {scanned} files were scanned");
    assert!(
        offenders.is_empty(),
        "the membership predicate is re-derived outside `aplan_core::edge_kind`, which is the \
         drift `A1-216` removed. Ask `edge_kind` instead:\n  {}",
        offenders.join("\n  ")
    );
}
