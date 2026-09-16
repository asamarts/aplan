//! Reachability is the query this tool owes (`A1-221`, ADR-0026).
//!
//! Every predicate over a *row* is a `jq` line once `aplan tasks` carries the
//! field. What no downstream tool can do is the transitive shape, because that
//! is a traversal over a graph the consumer would have to rebuild from
//! `blocked_by` first. So the tool owns four traversals and refuses a grammar.

use std::path::Path;

mod common;

/// Two cuts, a dependency chain crossing between them, and one task in no cut.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    let task = |id: &str, extra: &str| {
        format!(
            "---\nid: {id}\ntitle: task {id}\nphase: 0\nstatus: todo\nowner: t\n\
             effort_days: 1\n{extra}---\n\nbody\n"
        )
    };
    let files = [
        ("R-1", task("R-1", "")),
        ("R-2", task("R-2", "blocked_by: [R-1]\n")),
        ("R-3", task("R-3", "blocked_by: [R-2]\n")),
        ("R-4", task("R-4", "blocked_by: [R-2]\n")),
        // In the plan and in no cut, which is what `unreachable` finds.
        ("R-9", task("R-9", "")),
        (
            "M-a",
            "---\nid: M-a\ntitle: cut a\nphase: 0\nstatus: todo\ntags: [milestone]\n\
             blocked_by: [R-1, R-2]\n---\n\nbody\n"
                .to_string(),
        ),
        (
            "M-b",
            "---\nid: M-b\ntitle: cut b\nphase: 0\nstatus: todo\ntags: [milestone]\n\
             blocked_by: [R-3, R-4]\n---\n\nbody\n"
                .to_string(),
        ),
    ];
    for (id, body) in files {
        std::fs::write(dir.join(format!("aplan/tasks/{id}.md")), body).expect("task");
    }
}

fn graph(dir: &Path, args: &[&str]) -> std::process::Output {
    let mut all: Vec<&str> = vec!["graph"];
    all.extend(args);
    common::output(dir, &all)
}

/// The ids a scoped view selects.
fn ids(dir: &Path, args: &[&str]) -> std::collections::BTreeSet<String> {
    let mut all: Vec<&str> = args.to_vec();
    all.extend(["--format", "json"]);
    let out = graph(dir, &all);
    assert!(
        out.status.success(),
        "`graph {}` failed: {}",
        all.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    doc["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| n["id"].as_str())
        .map(str::to_string)
        .collect()
}

fn set(items: &[&str]) -> std::collections::BTreeSet<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn upstream_and_downstream_are_transitive() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // R-3 waits on R-2 waits on R-1, and the subject is in its own answer.
    assert_eq!(
        ids(dir.path(), &["--reach", "upstream:R-3"]),
        set(&["R-1", "R-2", "R-3"])
    );
    // R-1 unblocks the whole chain below it, both branches.
    assert_eq!(
        ids(dir.path(), &["--reach", "downstream:R-1"]),
        set(&["R-1", "R-2", "R-3", "R-4"])
    );
}

/// What lies between two tasks: the intersection of one's downstream and the
/// other's upstream, which is not either of them.
#[test]
fn between_is_the_intersection_and_not_a_union() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    assert_eq!(
        ids(dir.path(), &["--reach", "between:R-1..R-3"]),
        set(&["R-1", "R-2", "R-3"]),
        "the path, and nothing off it"
    );
    // R-4 hangs off R-2 and is on no path to R-3, so it is not between them --
    // which a union of the two closures would have included.
    assert!(!ids(dir.path(), &["--reach", "between:R-1..R-3"]).contains("R-4"));
}

/// Work in the plan and in no cut, which `APLAN022` reports one task at a time
/// and nothing could ask for as a set.
#[test]
fn unreachable_finds_work_no_cut_holds() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    assert_eq!(
        ids(dir.path(), &["--reach", "unreachable", "--membership"]),
        set(&["R-9"])
    );
}

/// `unreachable` answers the same thing whatever edges it is asked over, and
/// says which it walked.
///
/// It is a question about *containment*, and containment is the membership
/// relation. Asked over dependency edges a milestone reaches only other
/// milestones, so the answer was "almost every task" -- 238 of this
/// repository's own 248 -- presented as though it meant something.
#[test]
fn unreachable_walks_containment_whatever_it_is_asked_over() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let over_membership = ids(dir.path(), &["--reach", "unreachable", "--membership"]);
    let over_dependency = ids(dir.path(), &["--reach", "unreachable"]);
    assert!(
        over_dependency.is_subset(&over_membership),
        "the answer changed with the edges it was asked over: {over_dependency:?}"
    );
    assert!(
        over_membership.len() < 3,
        "the plan has 7 tasks and this should find the one in no cut, not most of them: \
         {over_membership:?}"
    );

    // And it says which graph it actually walked, not the one it was given.
    let out = graph(dir.path(), &["--reach", "unreachable", "--format", "text"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("cut-membership edges"),
        "it reports the edges it was asked for rather than the ones it walked:\n{text}"
    );
    assert!(
        !text.contains("`unreachable` over dependency edges"),
        "it claims to have walked dependencies:\n{text}"
    );
}

/// The answer agrees with what the plan itself records, on the real plan.
///
/// `cut` is a field `aplan tasks` derives independently of any traversal, so
/// this is the traversal checked against a second opinion rather than against a
/// number somebody typed here.
#[test]
fn unreachable_agrees_with_the_plans_own_cut_field() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the workspace root")
        .to_path_buf();

    let listing = common::output(&root, &["tasks", "--format", "json"]);
    let listing: serde_json::Value =
        serde_json::from_slice(&listing.stdout).expect("tasks --format json");
    let no_cut: std::collections::BTreeSet<String> = listing["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .filter(|t| t["cut"].as_str().is_none_or(str::is_empty))
        .filter_map(|t| t["id"].as_str())
        .map(str::to_string)
        .collect();
    assert!(
        !no_cut.is_empty(),
        "every task is in a cut, so this proves nothing"
    );

    let out = common::output(
        &root,
        &[
            "graph",
            "--reach",
            "unreachable",
            "--membership",
            "--format",
            "json",
        ],
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("graph json");
    let found: std::collections::BTreeSet<String> = doc["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| n["id"].as_str())
        .map(str::to_string)
        .collect();

    assert_eq!(
        found, no_cut,
        "the traversal and the plan's own `cut` field disagree about what is in no cut"
    );
}

/// Which graph a traversal walked is part of its answer.
///
/// ADR-0008 makes `blocked_by` carry grouping and constraint at once, so
/// reachability over one answers a different question from the other. The view
/// says which, in every shape.
#[test]
fn a_traversal_says_which_graph_it_walked() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    for (args, expected) in [
        (vec!["--reach", "downstream:R-1"], "dependency edges"),
        (
            vec!["--reach", "downstream:R-1", "--membership"],
            "every `blocked_by` edge",
        ),
    ] {
        for format in ["mermaid", "dot", "text"] {
            let mut all = args.clone();
            all.extend(["--format", format]);
            let out = graph(dir.path(), &all);
            let text = String::from_utf8_lossy(&out.stdout);
            let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(
                flat.contains(expected),
                "`{format}` does not say it walked {expected}:\n{text}"
            );
        }
    }

    // And the two are genuinely different answers, or saying which would not
    // matter: over grouping, R-1's cut comes into view.
    assert!(!ids(dir.path(), &["--reach", "downstream:R-1"]).contains("M-a"));
    assert!(
        ids(dir.path(), &["--reach", "downstream:R-1", "--membership"]).contains("M-a"),
        "membership reachability should reach the cut that lists R-1"
    );
}

/// A traversal is a selection, so it intersects with the others rather than
/// replacing them: "what this unblocks, in that cut" is one question.
#[test]
fn a_traversal_intersects_with_the_other_selections() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let downstream = ids(dir.path(), &["--reach", "downstream:R-1", "--membership"]);
    let both = ids(
        dir.path(),
        &["--reach", "downstream:R-1", "--cut", "M-b", "--membership"],
    );

    assert!(both.len() < downstream.len(), "the cut did not narrow it");
    assert!(
        both.iter().all(|id| downstream.contains(id)),
        "the intersection is not a subset of the traversal: {both:?}"
    );
    assert!(both.contains("R-3") && both.contains("R-4"), "{both:?}");
    assert!(!both.contains("R-1"), "R-1 is not in that cut: {both:?}");
}

/// The grammar this refuses, refused by name.
#[test]
fn anything_that_is_not_one_of_the_four_is_refused() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    for bad in [
        "status=done",
        "upstream",
        "upstream:",
        "between:R-1",
        "between:..R-3",
        "sideways:R-1",
        "downstream:R-1 and status=todo",
    ] {
        let out = graph(dir.path(), &["--reach", bad]);
        let said = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "`{bad}` was accepted: {said}");
        assert!(
            said.contains("is not one of the four"),
            "`{bad}` was refused without saying what the four are: {said}"
        );
        assert!(
            said.contains("aplan tasks") && said.contains("jq"),
            "`{bad}` was refused without naming what does answer it: {said}"
        );
    }

    // An id the plan does not have is a different mistake and says so.
    let out = graph(dir.path(), &["--reach", "upstream:NOPE"]);
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("no task `NOPE`"), "{said}");
}

/// No *expression* over a task's fields was added, which is the fourth criterion
/// and the whole boundary.
///
/// **This line moved once, deliberately.** It first forbade `--status` and
/// `--owner` on `graph` as well, reading `ADR-0026`'s "no field access" as a rule
/// about flags. It is a rule about the `--reach` *grammar*: the same ADR names
/// `A1-214`'s selector -- a conjunction of equality tests over declared
/// dimensions -- as the thing traversals compose with, and `A1-214` then gave
/// every reporting command those four flags. A fixed set of dimensions can be
/// read off `--help` and cannot grow an operator; an expression naming a field
/// can grow every operator there is. That is the difference the ADR is about, and
/// this checks it rather than the shape of a flag name.
#[test]
fn no_expression_over_a_tasks_fields_was_added() {
    let cli = aplan::cli_command();
    let graph_cmd = cli
        .get_subcommands()
        .find(|c| c.get_name() == "graph")
        .expect("a graph command");
    let flags: Vec<&str> = graph_cmd
        .get_arguments()
        .filter_map(clap::Arg::get_long)
        .collect();

    for expression in ["where", "filter", "query", "expr", "predicate", "select"] {
        assert!(
            !flags.contains(&expression),
            "`graph` gained `--{expression}`, which is where a filter grammar starts: that \
             is `aplan tasks --format json` and `jq` (ADR-0026)"
        );
    }
    assert!(
        flags.contains(&"reach"),
        "the traversal flag is missing: {flags:?}"
    );

    // The declared dimensions are the ones `A1-214` settled, and no more: a
    // fifth arriving here is a dimension somebody added without deciding it is
    // one.
    let dimensions: Vec<&str> = [
        "cut", "tag", "owner", "status", "phase", "effort", "evidence",
    ]
    .into_iter()
    .filter(|d| flags.contains(d))
    .collect();
    assert_eq!(
        dimensions,
        ["cut", "tag", "owner", "status"],
        "the selector's dimensions are not the four `A1-214` decided on"
    );
}
