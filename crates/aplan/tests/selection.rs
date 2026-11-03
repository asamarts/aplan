//! One selector, on every command where a subset is meaningful (`A1-214`).
//!
//! `--tag` existed on exactly one command, and nine that report answered about
//! the whole plan or about one task with nothing in between. Adding them a flag
//! at a time is forty-five flags, each with its own parsing and its own way of
//! being slightly different.
//!
//! The property that matters most is not that the flags exist. It is that
//! **selection narrows what is shown and never what is computed**: a task whose
//! blocker is outside the subset is still blocked, and a filter applied before
//! the derivation would report it ready. That is a false answer rather than a
//! narrow one, and it is the shape of mistake this whole repository is about.

use std::path::Path;

use crate::common;

/// A blocker outside the subset, a task inside it that the blocker holds, and a
/// task inside it that is genuinely ready.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    let files = [
        (
            "S-1",
            "---\nid: S-1\ntitle: the blocker, outside the subset\nphase: 0\nstatus: todo\n\
             owner: outsider\neffort_days: 1\ntags: [other]\n---\n\nbody\n",
        ),
        (
            "S-2",
            "---\nid: S-2\ntitle: inside the subset, blocked from outside\nphase: 1\n\
             status: todo\nowner: kim\neffort_days: 1\nblocked_by: [S-1]\ntags: [cli]\n\
             ---\n\nbody\n",
        ),
        (
            "S-3",
            "---\nid: S-3\ntitle: inside and genuinely ready\nphase: 1\nstatus: todo\n\
             owner: kim\neffort_days: 1\ntags: [cli]\n---\n\nbody\n",
        ),
    ];
    for (id, body) in files {
        std::fs::write(dir.join(format!("aplan/tasks/{id}.md")), body).expect("task");
    }
}

fn stdout(dir: &Path, args: &[&str]) -> String {
    let out = common::output(dir, args);
    assert!(
        out.status.success(),
        "`aplan {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The invariant. Every other test here is downstream of it.
#[test]
fn selection_narrows_what_is_shown_never_what_is_computed() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // S-2 is in the subset and is blocked by S-1, which is not. It is still
    // blocked: a filter applied before the derivation would offer it.
    let ready = stdout(dir.path(), &["ready", "--owner", "kim"]);
    assert!(
        ready.contains("S-3"),
        "the genuinely ready task is missing:\n{ready}"
    );
    assert!(
        !ready.contains("S-2"),
        "a task blocked from outside the subset was offered as ready:\n{ready}"
    );

    // The same invariant holds for the phase coordinate specifically. Phase is
    // a reporting selector, never an instruction to discard the phase-0 blocker
    // before readiness is derived.
    let ready_phase = stdout(dir.path(), &["ready", "--phase", "1"]);
    assert!(ready_phase.contains("S-3"), "{ready_phase}");
    assert!(
        !ready_phase.contains("S-2"),
        "phase selection erased an external blocker:\n{ready_phase}"
    );

    // The row says the same thing.
    let doc: serde_json::Value = serde_json::from_str(&stdout(
        dir.path(),
        &["tasks", "--owner", "kim", "--format", "json"],
    ))
    .expect("json");
    let ready_of = |id: &str| -> bool {
        doc["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .find(|t| t["id"] == id)
            .unwrap_or_else(|| panic!("no row for {id}"))["ready"]
            == serde_json::json!(true)
    };
    assert!(!ready_of("S-2"), "the row claims a blocked task is ready");
    assert!(ready_of("S-3"));

    // And so does the rollup, which counts readiness over the whole plan and
    // rolls up only the subset.
    let summary: serde_json::Value = serde_json::from_str(&stdout(
        dir.path(),
        &["summary", "--owner", "kim", "--format", "json"],
    ))
    .expect("json");
    assert_eq!(summary["ready"], serde_json::json!(1), "{summary}");
    let tasks: u64 = summary["phases"]
        .as_array()
        .expect("phases")
        .iter()
        .map(|p| p["tasks"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(tasks, 2, "the rollup counted outside the subset: {summary}");
}

/// One definition, so every command that takes a subset takes the same five.
#[test]
fn every_selecting_command_takes_the_same_flags() {
    const SELECTS: &[&str] = &[
        "ready", "tasks", "summary", "due", "stale", "vacuity", "paths", "graph",
    ];
    /// Commands that report but take no subset, and why.
    const WITHOUT: &[(&str, &str)] = &[
        (
            "critical-path",
            "one derived chain; a subset of a path is not a path, and the longest \
             chain through part of a plan is a different question from the one \
             asked. It takes a positional endpoint instead (`A1-345`), which \
             removes nothing: every edge is kept and only the chain's end is \
             chosen",
        ),
        (
            "roadmap",
            "the document *is* the cut structure, so selecting a cut renders a \
             document about one section rather than a narrowed plan",
        ),
        (
            "why",
            "answers about one task, which is already the narrowest subset there is",
        ),
        (
            "tree",
            "answers about one task; `--depth` and `--live` narrow it (`A1-217`)",
        ),
    ];

    let cli = aplan::cli_command();
    for name in SELECTS {
        let sub = cli
            .get_subcommands()
            .find(|c| c.get_name() == *name)
            .unwrap_or_else(|| panic!("no `{name}` command"));
        let flags: Vec<&str> = sub
            .get_arguments()
            .filter_map(clap::Arg::get_long)
            .collect();
        for dimension in ["cut", "phase", "tag", "owner", "status"] {
            assert!(
                flags.contains(&dimension),
                "`{name}` does not take `--{dimension}`, so selection is not one thing"
            );
        }
    }
    for (name, reason) in WITHOUT {
        assert!(reason.len() > 30, "`{name}` is held with no real reason");
        let sub = cli
            .get_subcommands()
            .find(|c| c.get_name() == *name)
            .unwrap_or_else(|| panic!("no `{name}` command"));
        let flags: Vec<&str> = sub
            .get_arguments()
            .filter_map(clap::Arg::get_long)
            .collect();
        assert!(
            !flags.contains(&"owner"),
            "`{name}` is held out of selection but takes `--owner`"
        );
    }
}

/// Within a dimension, any; across dimensions, all.
#[test]
fn one_dimension_widens_and_two_narrow() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let ids = |args: &[&str]| -> Vec<String> {
        let mut all: Vec<&str> = vec!["tasks", "--format", "json"];
        all.extend(args);
        let doc: serde_json::Value = serde_json::from_str(&stdout(dir.path(), &all)).expect("json");
        doc["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .filter_map(|t| t["id"].as_str())
            .map(str::to_string)
            .collect()
    };

    assert_eq!(ids(&["--tag", "cli"]), ["S-2", "S-3"]);
    assert_eq!(
        ids(&["--tag", "cli", "--tag", "other"]),
        ["S-1", "S-2", "S-3"]
    );
    assert_eq!(ids(&["--tag", "cli", "--owner", "kim"]), ["S-2", "S-3"]);
    assert_eq!(
        ids(&["--tag", "other", "--owner", "kim"]),
        Vec::<String>::new()
    );
    assert_eq!(ids(&["--status", "todo"]).len(), 3);
    assert_eq!(ids(&["--phase", "1"]), ["S-2", "S-3"]);
    assert_eq!(
        ids(&["--phase", "0", "--phase", "1"]),
        ["S-1", "S-2", "S-3"]
    );
}

/// A gate over a subset would report a clean plan while ignoring the rest.
///
/// The one decision in this task that can produce a false green: a qualified
/// verdict is still green to an exit code, so the selection is refused instead.
#[test]
fn a_gate_refuses_a_selection_rather_than_qualifying_it() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // Every dimension, not one of them. A refusal wired to `--owner` alone would
    // leave the other four qualifying the gate silently, which is the answer
    // this refusal exists to prevent -- and the suite exercised only `--owner`
    // until `A1-208` counted (`A1-208`).
    let mut checked = 0_usize;
    for gate in ["check", "verify"] {
        for (flag, value) in [
            ("--owner", "kim"),
            ("--cut", "a"),
            ("--phase", "1"),
            ("--tag", "cli"),
            ("--status", "todo"),
        ] {
            checked += 1;
            let out = common::output(dir.path(), &[gate, flag, value]);
            let said = String::from_utf8_lossy(&out.stderr);
            assert_eq!(
                out.status.code(),
                Some(2),
                "`{gate} {flag}` accepted a subset: {said}"
            );
            assert!(
                said.contains("is a gate"),
                "`{gate} {flag}` refused without saying why: {said}"
            );
            assert!(
                said.contains("aplan tasks"),
                "`{gate} {flag}` refused without naming what does inspect a subset: {said}"
            );
        }
        // And without a selection it does not refuse: whatever else it says,
        // it is not this. (`verify` wants a git repository, which this fixture
        // is not, so the code alone cannot carry the assertion.)
        let whole = common::output(dir.path(), &[gate]);
        let whole_said = String::from_utf8_lossy(&whole.stderr);
        assert!(
            !whole_said.contains("is a gate"),
            "`{gate}` refused a selection nobody made: {whole_said}"
        );
    }
    assert_eq!(checked, 10, "a (gate, dimension) pair was skipped");
}

/// A status the model does not have is refused with the ones it does.
#[test]
fn an_unknown_status_lists_the_ones_that_exist() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let out = common::output(dir.path(), &["tasks", "--status", "nope"]);
    let said = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "`nope` was accepted");
    for status in ["todo", "in_progress", "blocked", "done", "dropped"] {
        assert!(
            said.contains(status),
            "the refusal omits `{status}`: {said}"
        );
    }
}

/// `--cut` keeps working where it already existed, which is what the fourth
/// criterion asks: no runway is needed because nothing was removed.
#[test]
fn the_cut_flag_that_existed_keeps_working() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/C-1.md"),
        "---\nid: C-1\ntitle: in the cut\nphase: 0\nstatus: todo\nowner: t\neffort_days: 1\n\
         ---\n\nbody\n",
    )
    .expect("task");
    std::fs::write(
        dir.path().join("aplan/tasks/M-s0.md"),
        "---\nid: M-s0\ntitle: the cut\nphase: 0\nstatus: todo\ntags: [milestone]\n\
         blocked_by: [C-1]\n---\n\nbody\n",
    )
    .expect("milestone");

    // By milestone id, on the command that had the flag first. The label form
    // resolves through the shared resolver every writing command uses, which
    // `mutate.rs` already holds to accepting both.
    let cut = "M-s0";
    let out = stdout(dir.path(), &["graph", "--cut", cut, "--membership"]);
    assert!(out.contains("C-1"), "`--cut {cut}` lost the member:\n{out}");
}

/// The same selection means the same thing on every command that takes it.
///
/// `graph` applied each `--cut` with its own intersection, so `--cut a --cut b`
/// matched nothing there while `aplan tasks` with the same flags returned both:
/// two commands, one documented rule, opposite answers, from the task that
/// exists to remove exactly that. Within a dimension, any.
#[test]
fn two_commands_given_one_selection_select_the_same_tasks() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");

    let write = |id: &str, body: String| {
        std::fs::write(dir.path().join(format!("aplan/tasks/{id}.md")), body).expect("task");
    };
    for (id, cut) in [("T-1", "M-a"), ("T-2", "M-a"), ("T-3", "M-b")] {
        let _ = cut;
        write(
            id,
            format!(
                "---\nid: {id}\ntitle: task {id}\nphase: 0\nstatus: todo\nowner: t\n\
                 effort_days: 1\n---\n\nbody\n"
            ),
        );
    }
    write(
        "M-a",
        "---\nid: M-a\ntitle: cut a\nphase: 0\nstatus: todo\ntags: [milestone]\n\
         blocked_by: [T-1, T-2]\n---\n\nbody\n"
            .to_string(),
    );
    write(
        "M-b",
        "---\nid: M-b\ntitle: cut b\nphase: 0\nstatus: todo\ntags: [milestone]\n\
         blocked_by: [T-3]\n---\n\nbody\n"
            .to_string(),
    );

    let from_tasks = |args: &[&str]| -> std::collections::BTreeSet<String> {
        let mut all: Vec<&str> = vec!["tasks", "--format", "json"];
        all.extend(args);
        let doc: serde_json::Value = serde_json::from_str(&stdout(dir.path(), &all)).expect("json");
        doc["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .filter_map(|t| t["id"].as_str())
            .map(str::to_string)
            .collect()
    };
    let from_graph = |args: &[&str]| -> std::collections::BTreeSet<String> {
        let mut all: Vec<&str> = vec!["graph", "--membership", "--format", "json"];
        all.extend(args);
        let doc: serde_json::Value = serde_json::from_str(&stdout(dir.path(), &all)).expect("json");
        doc["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .filter_map(|n| n["id"].as_str())
            .map(str::to_string)
            .collect()
    };

    // Two cuts widen, in both commands, to the same set.
    let both = from_tasks(&["--cut", "M-a", "--cut", "M-b"]);
    assert_eq!(
        both.len(),
        5,
        "the union should be every task and both cuts: {both:?}"
    );
    assert_eq!(
        from_graph(&["--cut", "M-a", "--cut", "M-b"]),
        both,
        "`graph` and `tasks` disagree about what two cuts select"
    );

    // And one cut narrows, in both, to the same set.
    let one = from_tasks(&["--cut", "M-a"]);
    assert!(one.len() < both.len(), "one cut did not narrow: {one:?}");
    assert_eq!(
        from_graph(&["--cut", "M-a"]),
        one,
        "`graph` and `tasks` disagree about what one cut selects"
    );
}

/// A reason may name a task outside the subset, and must.
///
/// `ready` ranks over the whole plan, so a task inside the subset can prefer to
/// follow one outside it. That reference looks wrong -- a row pointing at
/// something not in the list -- and filtering it would be the tempting fix. It
/// would also be false: the task really is ranked below that one, and hiding the
/// reason leaves an order that looks arbitrary. The rule is the same rule:
/// selection narrows what is shown, never what is computed.
#[test]
fn a_reason_may_name_a_task_the_selection_excluded() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/P-1.md"),
        "---\nid: P-1\ntitle: outside the subset\nphase: 0\nstatus: todo\nowner: other\n\
         effort_days: 3\n---\n\nbody\n",
    )
    .expect("task");
    std::fs::write(
        dir.path().join("aplan/tasks/P-2.md"),
        "---\nid: P-2\ntitle: inside, prefers to follow P-1\nphase: 0\nstatus: todo\n\
         owner: kim\neffort_days: 1\nprefer_after: [P-1]\n---\n\nbody\n",
    )
    .expect("task");

    let doc: serde_json::Value = serde_json::from_str(&stdout(
        dir.path(),
        &["ready", "--owner", "kim", "--format", "json"],
    ))
    .expect("json");
    let rows = doc["ready"].as_array().expect("ready");
    assert_eq!(rows.len(), 1, "the subset is one task: {doc}");
    assert_eq!(
        rows[0]["prefers_to_follow"],
        serde_json::json!(["P-1"]),
        "the reason was filtered to the subset, which leaves an order nobody can check"
    );

    let explained = stdout(dir.path(), &["ready", "--explain", "--owner", "kim"]);
    assert!(
        explained.contains("prefers to follow P-1"),
        "the sentence dropped a reason the field carries:\n{explained}"
    );
}

// --- every dimension, on every command, through the binary (`A1-208`) --------

/// A plan with two of everything: two cuts, phases, tags, owners and statuses.
///
/// [`every_selecting_command_takes_the_same_flags`] asks clap whether the flags
/// exist, which is a declaration checked against a declaration. This is the
/// fixture for asking the binary whether they *do* anything, and it needs a
/// second value in every dimension or "narrowed" and "whole" are the same set.
fn two_of_everything(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    let files = [
        (
            "M-a",
            "---\nid: M-a\ntitle: cut a\nphase: 0\nstatus: todo\ntags: [milestone]\n\
             blocked_by: [D-1, D-2, D-5]\n---\n\nbody\n",
        ),
        (
            "M-b",
            "---\nid: M-b\ntitle: cut b\nphase: 0\nstatus: todo\ntags: [milestone]\n\
             blocked_by: [D-3, D-4, D-6]\n---\n\nbody\n",
        ),
        (
            "D-1",
            "---\nid: D-1\ntitle: in cut a, kim, cli, todo\nphase: 1\nstatus: todo\n\
             owner: kim\neffort_days: 1\npaths:\n- src/one/**\ntags: [cli]\n---\n\nbody\n",
        ),
        (
            "D-2",
            "---\nid: D-2\ntitle: in cut a, ada, docs, in_progress\nphase: 1\n\
             status: in_progress\nowner: ada\neffort_days: 2\nblocked_by: [D-1]\n\
             paths:\n- src/two/**\ntags: [docs]\n---\n\nbody\n",
        ),
        (
            "D-3",
            "---\nid: D-3\ntitle: in cut b, ada, cli, todo\nphase: 2\nstatus: todo\n\
             owner: ada\neffort_days: 3\npaths:\n- src/three/**\ntags: [cli]\n---\n\nbody\n",
        ),
        (
            "D-4",
            "---\nid: D-4\ntitle: in cut b, ada, cli, blocked\nphase: 2\nstatus: blocked\n\
             owner: ada\neffort_days: 4\nblocked_by: [D-3]\npaths:\n- src/four/**\n\
             tags: [cli]\nrevisit_at: 2020-01-01T00:00:00Z\n---\n\nbody\n",
        ),
        // Two closed tasks citing a commit and declaring no `paths`, which is
        // what gives `vacuity` a subject to be blind about: with no evidence
        // anywhere it reports zero for every subset, and a selection that
        // narrows nothing to nothing looks like a flag that does nothing.
        (
            "D-5",
            "---\nid: D-5\ntitle: closed in cut a, kim, cli\nphase: 1\nstatus: done\n\
             owner: kim\neffort_days: 1\ntags: [cli]\n\
             evidence:\n  - kind: commit\n    ref: 0000000\n---\n\nbody\n",
        ),
        (
            "D-6",
            "---\nid: D-6\ntitle: closed in cut b, ada, docs\nphase: 2\nstatus: done\n\
             owner: ada\neffort_days: 1\ntags: [docs]\n\
             evidence:\n  - kind: commit\n    ref: 0000000\n---\n\nbody\n",
        ),
    ];
    for (id, body) in files {
        std::fs::write(dir.join(format!("aplan/tasks/{id}.md")), body).expect("task");
    }

    // A real repository: `stale` reads history to date a task, and refuses
    // rather than guessing without one.
    let repo = git2::Repository::init(dir).expect("git init");
    let mut index = repo.index().expect("index");
    index
        .add_all(["."].iter(), git2::IndexAddOption::DEFAULT, None)
        .expect("add");
    index.write().expect("write index");
    let tree = repo
        .find_tree(index.write_tree().expect("tree"))
        .expect("tree");
    let sig = git2::Signature::now("t", "t@e.com").expect("sig");
    repo.commit(Some("HEAD"), &sig, &sig, "in", &tree, &[])
        .expect("commit");
}

/// **Every selecting command, every dimension, run.**
///
/// The parity test above proves the five flags are declared on all eight
/// commands. That is a table checked against a table: it would pass just as
/// happily if `--status` were parsed and thrown away, which is the defect
/// `A1-209` is about and the one `A1-208` found twenty-eight instances of --
/// eight commands times five dimensions, of which the suite invoked a handful.
///
/// The property asked is the weakest honest one, and it is enough: a selection
/// matching a strict subset must not render what the whole plan renders. A flag
/// that changes nothing fails it.
/// This is the executable proof for `selection.phase_coordinate`: a phase is a
/// real selector dimension on every command, rather than a parsed no-op.
#[test]
fn every_dimension_narrows_on_every_selecting_command() {
    const SELECTS: &[&str] = &[
        "ready", "tasks", "summary", "due", "stale", "vacuity", "paths", "graph",
    ];
    // A value each dimension holds for some tasks and not others. `stale`
    // reports on age rather than on the subset, so it is given a window that
    // makes every task stale and the selection is what differs.
    const DIMENSIONS: &[(&str, &str)] = &[
        ("--cut", "a"),
        ("--phase", "1"),
        ("--tag", "docs"),
        ("--owner", "kim"),
        ("--status", "in_progress"),
    ];

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    two_of_everything(dir.path());

    // The *answer*, not the exit status. A selection that leaves `graph` with no
    // dependency edge is refused, and correctly: "there is no graph here" is a
    // different answer from a graph, which is exactly what is being asked.
    let answer = |args: &[&str]| {
        let out = common::output(dir.path(), args);
        format!(
            "{}|{}|{}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };

    let mut unchanged = Vec::new();
    let mut checked = 0_usize;
    for command in SELECTS {
        let extra: &[&str] = if *command == "stale" {
            &["--older-than", "0"]
        } else {
            &[]
        };
        let mut whole: Vec<&str> = vec![command, "--format", "json"];
        whole.extend_from_slice(extra);
        // The baseline has to be a real answer, or "it changed" is met by two
        // different failures.
        stdout(dir.path(), &whole);
        let all = answer(&whole);
        for (flag, value) in DIMENSIONS {
            let mut args = whole.clone();
            args.extend_from_slice(&[flag, value]);
            checked += 1;
            if answer(&args) == all {
                unchanged.push(format!("`aplan {command} {flag} {value}`"));
            }
        }
    }

    assert_eq!(
        checked,
        SELECTS.len() * DIMENSIONS.len(),
        "a (command, dimension) pair was skipped rather than run"
    );
    assert!(
        unchanged.is_empty(),
        "{} selection(s) that render exactly what the whole plan renders, so the flag \
         is parsed and thrown away:\n  {}",
        unchanged.len(),
        unchanged.join("\n  ")
    );
}

// --- a release is not a cut, on every command that selects (`A1-240`) --------

/// Two cuts under one release, which is ADR-0022's shape.
fn two_cuts_and_a_release(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    let files = [
        (
            "R-1",
            "---\nid: R-1\ntitle: work in s0\nphase: 0\nstatus: todo\nowner: kim\n\
             effort_days: 1\n---\n\nbody\n",
        ),
        (
            "R-2",
            "---\nid: R-2\ntitle: work in s1\nphase: 0\nstatus: todo\nowner: ada\n\
             effort_days: 1\n---\n\nbody\n",
        ),
        (
            "M-s0",
            "---\nid: M-s0\ntitle: first cut\nphase: 0\nstatus: todo\n\
             tags: [milestone, s0]\nblocked_by: [R-1]\n---\n\nbody\n",
        ),
        (
            "M-s1",
            "---\nid: M-s1\ntitle: second cut\nphase: 0\nstatus: todo\n\
             tags: [milestone, s1]\nblocked_by: [R-2]\n---\n\nbody\n",
        ),
        (
            "M-v01",
            "---\nid: M-v01\ntitle: the release\nphase: 0\nstatus: todo\n\
             tags: [milestone, release, v0.1]\nblocked_by: [M-s0, M-s1]\n---\n\nbody\n",
        ),
    ];
    for (id, body) in files {
        std::fs::write(dir.join(format!("aplan/tasks/{id}.md")), body).expect("task");
    }
}

/// **`--cut` naming a release is refused, and says what it closes.**
///
/// A release claims no task (ADR-0022), so after `A1-240` no row reports one as
/// its cut -- and a selection by its label therefore matched nothing at all,
/// silently, on every command but `graph`. `graph` resolves `--cut` through the
/// milestone's members instead and went on returning its cuts' work, so the two
/// disagreed: one flag, two commands, opposite answers, which is the defect
/// `A1-214` exists to remove.
///
/// Refused rather than expanded to the cuts it closes, because the rows would
/// then report a different cut than the one asked for -- and the message is one
/// keystroke from the answer.
/// Proves the named behaviour `rollup.cut_excludes_releases` (`A1-211`).
#[test]
fn a_release_is_refused_as_a_cut_on_every_command_that_selects() {
    const SELECTS: &[&str] = &[
        "ready", "tasks", "summary", "due", "stale", "vacuity", "paths", "graph",
    ];

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    two_cuts_and_a_release(dir.path());

    let mut wrong = Vec::new();
    let mut checked = 0_usize;
    // By label and by milestone id, because `--cut` takes either (`A1-139`).
    for named in ["v0.1", "M-v01"] {
        for command in SELECTS {
            checked += 1;
            let out = common::run(dir.path(), &[command, "--cut", named]);
            let said = out.text();
            if out.code != 2 {
                wrong.push(format!(
                    "`aplan {command} --cut {named}` exited {}: {said}",
                    out.code
                ));
                continue;
            }
            let names_both = said.contains("`s0`") && said.contains("`s1`");
            if !said.contains("names a release") || !names_both {
                wrong.push(format!(
                    "`aplan {command} --cut {named}` refused without naming what it closes: {said}"
                ));
            }
        }
    }
    assert_eq!(checked, SELECTS.len() * 2, "a command was skipped");
    assert!(
        wrong.is_empty(),
        "{} command(s) that do not refuse a release as a cut:\n  {}",
        wrong.len(),
        wrong.join("\n  ")
    );

    // And a real cut still selects, or the refusal above is met by refusing
    // everything.
    let ok = stdout(dir.path(), &["tasks", "--format", "json", "--cut", "s0"]);
    assert!(ok.contains("R-1"), "`--cut s0` returned nothing: {ok}");
    assert!(!ok.contains("R-2"), "`--cut s0` did not narrow: {ok}");
}
