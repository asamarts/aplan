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

use crate::common;

/// The repository's own plan, which is the plan the task measured.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the workspace root")
        .to_path_buf()
}

fn historical_edge_measurement_exists() -> bool {
    workspace_root().join("aplan/tasks/A1-216.md").is_file()
}

fn assert_fresh_public_epoch() {
    let tasks = std::fs::read_dir(workspace_root().join("aplan/tasks"))
        .expect("public plan task directory")
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|value| value == "md"))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        !tasks.is_empty() && tasks.iter().all(|name| name.starts_with("A2-")),
        "the A1 edge measurement may be absent only in the explicit A2 public-plan epoch"
    );
}

fn graph_json(fixture: &common::WorkspacePlanFixture, args: &[&str]) -> serde_json::Value {
    let mut all: Vec<&str> = vec![
        "--no-config",
        "--root",
        "aplan",
        "graph",
        "--format",
        "json",
    ];
    all.extend(args);
    let out = common::output(fixture.directory(), &all);
    assert!(
        out.status.success(),
        "`aplan {}` failed: {}",
        all.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("graph --format json is JSON")
}

/// `aplan tasks --format json`, which carries the `milestone` tag as a field.
fn tasks_json(fixture: &common::WorkspacePlanFixture) -> serde_json::Value {
    let out = common::output(
        fixture.directory(),
        &[
            "--no-config",
            "--root",
            "aplan",
            "tasks",
            "--format",
            "json",
        ],
    );
    assert!(
        out.status.success(),
        "`aplan tasks --format json` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("tasks --format json is JSON")
}

/// `A1-637`: resolving the source and copying its files are one snapshot even
/// when another writer moves `HEAD` between those two operations.
#[test]
fn repository_self_fixture_reads_every_file_from_one_resolved_commit() {
    use std::io::Write as _;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    let source = tempfile::Builder::new()
        .prefix("committed-plan-source-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create committed plan source");
    std::fs::create_dir(source.path().join("aplan")).expect("create source plan");
    std::fs::write(source.path().join("aplan/snapshot.md"), "first\n")
        .expect("write first committed plan byte");
    let executable = source.path().join("aplan/executable.md");
    std::fs::write(&executable, "executable bytes\n").expect("write executable plan byte");
    #[cfg(unix)]
    {
        let mut permissions = std::fs::metadata(&executable)
            .expect("read executable mode")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).expect("set executable mode");
    }
    common::git(source.path(), &["init", "-q"]);
    common::git(
        source.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    common::git(source.path(), &["config", "user.name", "fixture"]);
    common::git(source.path(), &["add", "aplan"]);
    common::git(source.path(), &["commit", "-q", "-m", "first"]);

    let first = common::git(source.path(), &["rev-parse", "HEAD"]);
    let first = String::from_utf8(first.stdout)
        .expect("commit id is UTF-8")
        .trim()
        .parse::<git2::Oid>()
        .expect("commit id");
    let fixture = common::committed_plan_fixture_from(source.path(), |resolved| {
        assert_eq!(resolved, first);
        std::fs::write(source.path().join("aplan/snapshot.md"), "second\n")
            .expect("replace source after fixture resolution");
        let mut added = std::fs::File::create(source.path().join("aplan/added.md"))
            .expect("add source file after fixture resolution");
        writeln!(added, "second commit only").expect("write added source file");
        common::git(source.path(), &["add", "aplan"]);
        common::git(source.path(), &["commit", "-q", "-m", "second"]);
    });

    assert_eq!(fixture.source_commit(), first);
    assert_eq!(
        std::fs::read_to_string(fixture.plan_root().join("snapshot.md"))
            .expect("read captured source byte"),
        "first\n"
    );
    assert!(
        !fixture.plan_root().join("added.md").exists(),
        "a file from the later tree leaked into the resolved fixture"
    );
    assert_eq!(
        std::fs::read(fixture.plan_root().join("executable.md"))
            .expect("read captured executable blob"),
        b"executable bytes\n"
    );
    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(fixture.plan_root().join("executable.md"))
            .expect("read captured executable mode")
            .permissions()
            .mode()
            & 0o111,
        0o111
    );
}

fn fixture_refusal(source: &Path) -> String {
    let result = std::panic::catch_unwind(|| {
        let _fixture = common::committed_plan_fixture_from(source, |_| {});
    });
    let payload = match result {
        Ok(()) => panic!("the unsupported committed plan was copied"),
        Err(payload) => payload,
    };
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else {
        panic!("fixture refusal had no textual diagnostic")
    }
}

fn init_fixture_repository(source: &Path) {
    common::git(source, &["init", "-q"]);
    common::git(source, &["config", "user.email", "fixture@example.invalid"]);
    common::git(source, &["config", "user.name", "fixture"]);
}

fn commit_fixture_repository(source: &Path, message: &str) {
    common::git(source, &["add", "aplan"]);
    common::git(source, &["commit", "-q", "-m", message]);
}

/// `A1-639`: unsupported or unsafe Git tree entries are refused with the
/// context `A1-637` claims, rather than copied or lossily renamed.
#[test]
fn committed_plan_fixture_refuses_unsupported_and_unsafe_entries() {
    use std::os::unix::ffi::OsStringExt as _;
    use std::os::unix::fs::symlink;

    let symlink_source = tempfile::Builder::new()
        .prefix("committed-plan-symlink-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create symlink source");
    std::fs::create_dir(symlink_source.path().join("aplan")).expect("create symlink plan");
    std::fs::write(symlink_source.path().join("target.md"), "outside plan\n")
        .expect("write symlink target");
    symlink("../target.md", symlink_source.path().join("aplan/link.md"))
        .expect("create committed symlink");
    init_fixture_repository(symlink_source.path());
    commit_fixture_repository(symlink_source.path(), "symlink");
    let refusal = fixture_refusal(symlink_source.path());
    assert!(
        refusal.contains("aplan/link.md")
            && refusal.contains("unsupported kind Some(Blob)")
            && refusal.contains("mode 120000"),
        "symlink refusal lost its path, kind or mode: {refusal}"
    );

    let non_utf8_source = tempfile::Builder::new()
        .prefix("committed-plan-non-utf8-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create non-UTF-8 source");
    std::fs::create_dir(non_utf8_source.path().join("aplan")).expect("create non-UTF-8 plan");
    let name = std::ffi::OsString::from_vec(b"task-\xff.md".to_vec());
    std::fs::write(non_utf8_source.path().join("aplan").join(name), "task\n")
        .expect("write non-UTF-8 plan entry");
    init_fixture_repository(non_utf8_source.path());
    commit_fixture_repository(non_utf8_source.path(), "non utf8");
    let refusal = fixture_refusal(non_utf8_source.path());
    assert!(
        refusal.contains("non-UTF-8 name"),
        "non-UTF-8 refusal was not actionable: {refusal}"
    );

    let unsafe_source = tempfile::Builder::new()
        .prefix("committed-plan-unsafe-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create unsafe-name source");
    std::fs::create_dir(unsafe_source.path().join("aplan")).expect("create unsafe-name plan");
    std::fs::write(unsafe_source.path().join("aplan/task\\name.md"), "task\n")
        .expect("write unsafe-name plan entry");
    init_fixture_repository(unsafe_source.path());
    commit_fixture_repository(unsafe_source.path(), "unsafe name");
    let refusal = fixture_refusal(unsafe_source.path());
    assert!(
        refusal.contains("unsafe name") && refusal.contains(r"task\\name.md"),
        "unsafe-name refusal lost its offending name: {refusal}"
    );
}

/// `A1-639`: the immutable helper refuses an unborn repository rather than
/// falling back to its mutable `aplan/` working tree.
#[test]
fn committed_plan_fixture_requires_an_immutable_head() {
    let source = tempfile::Builder::new()
        .prefix("committed-plan-unborn-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create unborn source");
    std::fs::create_dir(source.path().join("aplan")).expect("create unborn plan");
    std::fs::write(source.path().join("aplan/live.md"), "mutable\n")
        .expect("write mutable working-tree plan");
    init_fixture_repository(source.path());

    let refusal = fixture_refusal(source.path());
    assert!(
        refusal.contains("resolve one committed HEAD for plan fixture"),
        "unborn repository refusal did not name the immutable boundary: {refusal}"
    );
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
    if !historical_edge_measurement_exists() {
        assert_fresh_public_epoch();
        return;
    }
    let fixture = common::workspace_plan_fixture();
    let whole = graph_json(&fixture, &["--membership"]);
    let deps = graph_json(&fixture, &[]);

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
    if !historical_edge_measurement_exists() {
        assert_fresh_public_epoch();
        return;
    }
    let fixture = common::workspace_plan_fixture();
    let whole = graph_json(&fixture, &["--membership"]);
    let deps = graph_json(&fixture, &[]);

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
    const CUT: &str = "v0.4";
    if !historical_edge_measurement_exists() {
        assert_fresh_public_epoch();
        return;
    }
    // This is a rendering contract, not another measurement of the growing
    // whole plan. v0.4 is closed and small enough to render at the supported
    // text cap while still omitting real membership edges. Later plan growth
    // must not turn this assertion into a width-refusal test by accident.
    let fixture = common::workspace_plan_fixture();
    let listing = tasks_json(&fixture);
    let cut = listing["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|task| task["id"] == "M-v0.4")
        .expect("the stable cut exists");
    assert_eq!(cut["status"], "done", "the fixture cut remains closed");

    let deps = graph_json(&fixture, &["--cut", CUT]);
    let left_out = deps["grouping"]["edges"].as_u64().expect("a count");
    assert!(left_out > 0, "nothing was left out, so this proves nothing");

    for (format, marker) in [("mermaid", "%%"), ("dot", "//"), ("text", "")] {
        let mut args = vec![
            "--no-config",
            "--root",
            "aplan",
            "graph",
            "--cut",
            CUT,
            "--format",
            format,
        ];
        if format == "text" {
            args.extend(["--width", "240"]);
        }
        let out = common::output(fixture.directory(), &args);
        assert!(
            out.status.success(),
            "`aplan graph --cut {CUT} --format {format}` failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
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
    if !historical_edge_measurement_exists() {
        assert_fresh_public_epoch();
        return;
    }
    let fixture = common::workspace_plan_fixture();
    let whole = graph_json(&fixture, &["--membership"]);
    // From the `milestone_task` field rather than from the id: an id prefix is
    // a convention this plan happens to follow, and the classification under
    // test reads the `milestone` *tag*. A test that guessed differently from
    // the code would agree with it only by luck.
    let listing = tasks_json(&fixture);
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
