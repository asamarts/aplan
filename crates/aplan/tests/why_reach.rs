//! `why` answers the question a person asked (`A1-263`).
//!
//! `p0000014` sequences eight cuts the way ADR-0013 prescribes and asked why one
//! launch task was blocked. The answer was 86 lines: two work blockers, the
//! unfinished cut in front of it, and then every live member of all three
//! earlier cuts. Graph-theoretically true, and the three reasons a person could
//! act on were buried in eighty-three they could not. Through MCP the same
//! answer spends agent context on a roadmap the agent can query separately.
//!
//! So the default stops at a cut and reports it as itself, `--all` gives the
//! closure, and **every answer names the traversal that produced it** -- a fold
//! that did not say it was one would be a truncation reading as the whole truth,
//! which is worse than the eighty-three lines.

use std::fmt::Write as _;
use std::path::Path;

use crate::common;
use common::aplan;

/// Four cuts, sequenced, fifteen tasks each, and one last-cut task that also has
/// two direct blockers of its own. `p0000014`'s shape, smaller.
fn sequenced_plan() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\nrequire_cut_membership: true\n",
    )
    .expect("config");

    let mut entries = String::new();
    for c in 0..4 {
        let _ = write!(
            entries,
            "- title: Cut {c}\n  id: M-s{c}\n  tags: [milestone]\n"
        );
        if c > 0 {
            let _ = writeln!(entries, "  blocked_by: [M-s{}]", c - 1);
        }
    }
    for c in 0..4 {
        for i in 1..=15 {
            let _ = write!(
                entries,
                "- title: Task {c}.{i} in the plan\n  id: FW-{c}{i:02}\n  cut: M-s{c}\n  effort: 1\n"
            );
            if c == 3 && i == 15 {
                entries.push_str("  blocked_by: [FW-301, FW-302]\n");
            }
        }
    }
    std::fs::write(dir.path().join("plan.yml"), entries).expect("manifest");
    let (out, code) = aplan(dir.path(), &["add", "--from", "plan.yml"]);
    assert_eq!(code, 0, "{out}");
    dir
}

fn why(dir: &Path, args: &[&str]) -> String {
    let mut invocation = vec!["why"];
    invocation.extend_from_slice(args);
    let (out, code) = aplan(dir, &invocation);
    assert_eq!(code, 0, "{out}");
    out
}

/// The measured case. Three reasons a person can act on, and the size of what
/// they are not being shown.
///
/// Proves the named behaviour `why.cut_bounded_by_default` (`A1-263`).
#[test]
fn a_late_task_names_its_own_blockers_and_the_cut_in_front_of_it() {
    let dir = sequenced_plan();
    let bounded = why(dir.path(), &["FW-315"]);

    for id in ["FW-301", "FW-302", "M-s2"] {
        assert!(bounded.contains(id), "{id} missing from:\n{bounded}");
    }
    // Not one member of the folded cuts.
    assert!(
        !bounded.contains("FW-101") && !bounded.contains("FW-201"),
        "walked through a cut into its membership:\n{bounded}"
    );
    assert!(
        bounded.contains("unmet behind it, `--all` lists them"),
        "folded without saying how much, or how to see it:\n{bounded}"
    );
    assert!(bounded.contains("traversal: cut-bounded"), "{bounded}");

    // The closure is still there, and is the thing worth compacting: an order of
    // magnitude, on a plan a quarter of the pilot's size.
    let complete = why(dir.path(), &["FW-315", "--all"]);
    assert!(complete.contains("FW-101"), "{complete}");
    // Nothing was folded, so nothing is labelled: the word belongs on a partial
    // answer, and a complete one carries no caveat.
    assert!(!complete.contains("traversal:"), "{complete}");
    assert!(
        complete.lines().count() > bounded.lines().count() * 5,
        "bounded {} lines, complete {} -- if these are close the fold is not earning its keep",
        bounded.lines().count(),
        complete.lines().count()
    );
}

/// `behind` counts the cut's own remaining work, and is **not** what `--all`
/// adds (`A1-263`, audit).
///
/// A task can be a direct blocker of the subject *and* sit behind the folded
/// cut, in which case it is already named and the complete answer grows by
/// less. `p0000014`'s own case is that shape -- its two work blockers are in
/// cuts behind the one it folded -- so a consumer adding `behind` to the length
/// of `blocked_by` to predict the complete size would be wrong on the very
/// example this task came from.
#[test]
fn a_blocker_that_is_also_behind_the_cut_is_counted_once_by_each() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("plan.yml"),
        "- title: Cut zero\n  id: M-s0\n  tags: [milestone]\n\
         - title: A member\n  id: T-1\n  cut: M-s0\n  effort: 1\n\
         - title: Another member\n  id: T-2\n  cut: M-s0\n  effort: 1\n\
         - title: Blocked by the cut and by one of its members\n  id: T-9\n  effort: 1\n\
         \x20 blocked_by: [T-1, M-s0]\n",
    )
    .expect("manifest");
    let (out, code) = aplan(dir.path(), &["add", "--from", "plan.yml"]);
    assert_eq!(code, 0, "{out}");

    let bounded: serde_json::Value =
        serde_json::from_str(&aplan(dir.path(), &["why", "T-9", "--format", "json"]).0)
            .expect("json");
    let complete: serde_json::Value =
        serde_json::from_str(&aplan(dir.path(), &["why", "T-9", "--format", "json", "--all"]).0)
            .expect("json");

    let cut = bounded["blocked_by"]
        .as_array()
        .expect("blockers")
        .iter()
        .find(|b| b["id"] == "M-s0")
        .expect("the cut");
    // Both members are behind the cut, and one of them is also a direct blocker.
    assert_eq!(cut["behind"], 2, "{bounded}");

    let bounded_n = bounded["blocked_by"].as_array().expect("a").len();
    let complete_n = complete["blocked_by"].as_array().expect("a").len();
    assert_eq!((bounded_n, complete_n), (2, 3), "{bounded}\n{complete}");
    assert_ne!(
        bounded_n + 2,
        complete_n,
        "`behind` has become a delta; the doc comments and the message say it is not"
    );
}

/// An ordinary chain of work tasks is *not* folded: only a cut is, because only
/// a cut has a name that is a better answer than its members.
///
/// Proves the named behaviour `why.cut_bounded_by_default` (`A1-263`).
#[test]
fn a_chain_of_ordinary_tasks_is_walked_whole() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    let mut entries = String::new();
    for i in 1..=5 {
        let _ = write!(entries, "- title: Link {i}\n  id: T-{i}\n  effort: 1\n");
        if i > 1 {
            let _ = writeln!(entries, "  blocked_by: [T-{}]", i - 1);
        }
    }
    std::fs::write(dir.path().join("plan.yml"), entries).expect("manifest");
    let (out, code) = aplan(dir.path(), &["add", "--from", "plan.yml"]);
    assert_eq!(code, 0, "{out}");

    let bounded = why(dir.path(), &["T-5"]);
    for id in ["T-4", "T-3", "T-2", "T-1"] {
        assert!(bounded.contains(id), "{id} missing, five deep:\n{bounded}");
    }
    // And nothing was folded, so the answer is not labelled partial.
    assert!(!bounded.contains("traversal:"), "{bounded}");
}

/// A **release** folds like any other cut, because a release is a milestone
/// first (ADR-0022) and this rule keys on that.
///
/// Untested until its audit: `aplan`'s own plan carries no release-tagged task,
/// so the whole path ran on nothing. A release is the coarser boundary of the
/// two, and reporting it as itself is more valuable rather than less -- it
/// stands for every cut it closes.
#[test]
fn a_release_is_a_boundary_too() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("plan.yml"),
        "- title: Cut zero\n  id: M-s0\n  tags: [milestone]\n\
         - title: Work in it\n  id: T-1\n  cut: M-s0\n  effort: 1\n\
         - title: The first release\n  id: R-1\n  tags: [milestone, release]\n\
         \x20 blocked_by: [M-s0]\n\
         - title: Waits on the release\n  id: T-9\n  effort: 1\n  blocked_by: [R-1]\n",
    )
    .expect("manifest");
    let (out, code) = aplan(dir.path(), &["add", "--from", "plan.yml"]);
    assert_eq!(code, 0, "{out}");

    let bounded = why(dir.path(), &["T-9"]);
    assert!(bounded.contains("R-1"), "{bounded}");
    assert!(
        !bounded.contains("T-1") && !bounded.contains("M-s0"),
        "walked through a release into the cuts it closes:\n{bounded}"
    );
    assert!(bounded.contains("unmet behind it"), "{bounded}");

    let complete = why(dir.path(), &["T-9", "--all"]);
    assert!(
        complete.contains("M-s0") && complete.contains("T-1"),
        "{complete}"
    );
}

/// A finished cut is not a blocker at all, so nothing is folded behind it and
/// the note does not appear for it.
#[test]
fn a_cut_that_is_done_is_not_reported_and_folds_nothing() {
    let dir = sequenced_plan();
    // Close cut 0 whole, then the cut itself.
    for i in 1..=15 {
        let id = format!("FW-0{i:02}");
        let (out, _) = aplan(
            dir.path(),
            &["close", &id, "--evidence", "note:done", "--no-resolve"],
        );
        assert!(out.contains("captured") || out.contains("closed"), "{out}");
    }
    let bounded = why(dir.path(), &["FW-315"]);
    assert!(
        !bounded.contains("M-s0"),
        "a cut whose work is done is still reported as a reason:\n{bounded}"
    );
}

/// A task nothing blocks says so, and says nothing about a traversal: there was
/// no walk to describe.
#[test]
fn an_unblocked_task_names_no_traversal() {
    let dir = sequenced_plan();
    let out = why(dir.path(), &["FW-001"]);
    assert!(out.contains("is ready"), "{out}");
    assert!(
        !out.contains("traversal:"),
        "described a walk that did not happen:\n{out}"
    );
}

/// An id the plan does not define is refused, unchanged by any of this.
#[test]
fn an_unknown_id_is_still_refused() {
    let dir = sequenced_plan();
    let (out, code) = aplan(dir.path(), &["why", "NOPE"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("no task `NOPE`"), "{out}");
}

/// A blocker the plan references and never defines is reported under both
/// traversals: it is not behind a cut, and folding cannot reach it.
#[test]
fn an_undefined_blocker_survives_both_traversals() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: Blocked by a ghost\nphase: 1\nstatus: todo\n\
         effort_days: 1\nblocked_by: [GHOST]\n---\n\n## Why\n\nSomething.\n",
    )
    .expect("task");

    for args in [vec!["T-1"], vec!["T-1", "--all"]] {
        let out = why(dir.path(), &args);
        assert!(out.contains("GHOST"), "{args:?}:\n{out}");
        assert!(
            out.contains("referenced but not defined"),
            "{args:?}:\n{out}"
        );
    }
}

/// Proof for `why.complete_retains_nested_undefined_blockers`: the human and
/// JSON CLI adapters consume the same graph answer. A bounded walk hides the
/// ghost with the cut; `--all` must not call itself complete while dropping the
/// final unresolved edge.
#[test]
fn a_nested_undefined_blocker_respects_the_walk_boundary_on_every_why_surface() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    for (name, body) in [
        (
            "T-1.md",
            "---\nid: T-1\ntitle: Leaf with unknown blocker\nphase: 1\nstatus: todo\n\
             effort_days: 1\nblocked_by: [GHOST]\n---\n\n## Why\n\nSomething.\n",
        ),
        (
            "M-s1.md",
            "---\nid: M-s1\ntitle: A cut boundary\nphase: 1\nstatus: todo\n\
             blocked_by: [T-1]\ntags: [milestone]\n---\n\n## Why\n\nA cut.\n",
        ),
        (
            "T-9.md",
            "---\nid: T-9\ntitle: Waits on the cut\nphase: 1\nstatus: todo\n\
             effort_days: 1\nblocked_by: [M-s1]\n---\n\n## Why\n\nSomething.\n",
        ),
    ] {
        std::fs::write(dir.path().join("aplan/tasks").join(name), body).expect("task");
    }

    let bounded = why(dir.path(), &["T-9"]);
    assert!(bounded.contains("M-s1"), "{bounded}");
    assert!(!bounded.contains("GHOST"), "{bounded}");

    let complete = why(dir.path(), &["T-9", "--all"]);
    assert!(complete.contains("T-1"), "{complete}");
    assert!(complete.contains("GHOST"), "{complete}");
    assert!(
        complete.contains("referenced but not defined"),
        "{complete}"
    );

    let json: serde_json::Value =
        serde_json::from_str(&aplan(dir.path(), &["why", "T-9", "--all", "--format", "json"]).0)
            .expect("json");
    assert_eq!(json["traversal"], "complete");
    assert_eq!(json["unknown"], serde_json::json!(["GHOST"]));
}

/// The JSON says the same thing, in the fields a consumer reads.
///
/// Proves the named behaviour `why.cut_bounded_by_default` (`A1-263`).
#[test]
fn the_json_names_the_traversal_and_what_was_folded() {
    let dir = sequenced_plan();
    let (out, _) = aplan(dir.path(), &["why", "FW-315", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&out).expect("json");
    assert_eq!(v["traversal"], "cut-bounded");

    let blockers = v["blocked_by"].as_array().expect("blockers");
    let cut = blockers
        .iter()
        .find(|b| b["id"] == "M-s2")
        .expect("the cut is reported");
    assert!(
        cut["behind"].as_u64().is_some_and(|n| n > 0),
        "the cut carries no count of what was folded: {cut}"
    );
    // An ordinary blocker carries no count, so the field's presence is the
    // signal rather than a number that means nothing.
    let task = blockers
        .iter()
        .find(|b| b["id"] == "FW-301")
        .expect("the direct blocker is reported");
    assert!(task.get("behind").is_none(), "{task}");

    let (out, _) = aplan(dir.path(), &["why", "FW-315", "--format", "json", "--all"]);
    let v: serde_json::Value = serde_json::from_str(&out).expect("json");
    assert_eq!(v["traversal"], "complete");
    assert!(
        v["blocked_by"]
            .as_array()
            .expect("blockers")
            .iter()
            .all(|b| b.get("behind").is_none()),
        "a complete walk folded nothing and must claim nothing: {v}"
    );
}
