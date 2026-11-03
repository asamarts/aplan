//! `graph` renders three shapes of one scoped view (`A1-193`).
//!
//! It printed Mermaid and nothing else. Mermaid renders in a README and in a
//! browser, which is why it was chosen, and it is a poor input to anything else:
//! a consumer wanting to lay the graph out, compute over it, or feed Graphviz
//! had to parse a rendering of a DAG the tool already holds.
//!
//! The property that matters is not that three renderers exist -- it is that the
//! *scope* and the *rendering* are independent. `--cut` and `--tag` decide what
//! is in view; the format decides how it is written. Three walks of the plan
//! would eventually disagree about the first, so there is one `GraphView` and
//! three renderings of it, and these tests hold them to that.

use std::collections::BTreeSet;
use std::path::Path;

use crate::common;

/// Two cuts, a dependency inside one, an edge that crosses between them, and a
/// title carrying both characters a quoted label can be broken by.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    let tasks = [
        (
            "G-1",
            concat!(
                "---\nid: G-1\ntitle: |-\n  a title with a \"quote\" and a back\\slash\n  and a second line\n",
                "phase: 0\nstatus: done\nowner: t\neffort_days: 1\n",
                "evidence:\n  - kind: note\n    ref: seeded\n    captured: 2026-01-01\n",
                "---\n\nbody\n",
            ),
        ),
        (
            "G-2",
            "---\nid: G-2\ntitle: waits on G-1\nphase: 0\nstatus: todo\nowner: t\n\
                 effort_days: 1\nblocked_by: [G-1]\ntags: [alpha]\n---\n\nbody\n",
        ),
        (
            "G-3",
            "---\nid: G-3\ntitle: in the other cut, waiting across the boundary\nphase: 0\n\
                 status: todo\nowner: t\neffort_days: 1\nblocked_by: [G-2]\n---\n\nbody\n",
        ),
        (
            "M-a",
            "---\nid: M-a\ntitle: cut a\nphase: 0\nstatus: todo\ntags: [milestone]\n\
                 blocked_by: [G-1, G-2]\n---\n\nbody\n",
        ),
        (
            "M-b",
            "---\nid: M-b\ntitle: cut b\nphase: 0\nstatus: todo\ntags: [milestone]\n\
                 blocked_by: [G-3]\n---\n\nbody\n",
        ),
    ];
    for (id, body) in tasks {
        std::fs::write(dir.join(format!("aplan/tasks/{id}.md")), body).expect("task");
    }
}

/// `aplan graph <args>`, which must succeed.
fn run(dir: &Path, args: &[&str]) -> String {
    let mut all: Vec<&str> = vec!["graph"];
    all.extend(args);
    let out = common::output(dir, &all);
    assert!(
        out.status.success(),
        "`aplan {}` failed: {}",
        all.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn view(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut all: Vec<&str> = args.to_vec();
    all.extend(["--format", "json"]);
    serde_json::from_str(run(dir, &all).trim()).expect("graph --format json is JSON")
}

fn plain(text: &str) -> String {
    anstream::adapter::strip_str(text).to_string()
}

fn painted_text(view: &aplan_output::GraphView) -> String {
    plain(
        &aplan_output::lanes::text_of_presented_view(
            view,
            aplan_output::lanes::Glyphs::ASCII,
            usize::MAX,
            aplan_output::lanes::GraphPresentation::default(),
        )
        .expect("the fixture fits"),
    )
}

/// The CLI text path is a painter over the public semantic scene, byte for byte.
///
/// This crosses the binary boundary to obtain both forms. A unit test that
/// merely calls the painter twice would not catch the command quietly retaining
/// the old direct renderer.
#[test]
fn semantic_scene_and_cli_text_share_one_geometry() {
    let dir = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(dir.path());

    let json = run(dir.path(), &["--membership", "--format", "json"]);
    let graph: aplan_output::GraphView = serde_json::from_str(&json).expect("a graph view");
    let scene = aplan_output::graph_scene::GraphScene::layout(&graph, aplan_output::MAX_WIDTH)
        .expect("the fixture fits");
    let painted = painted_text(&graph);
    assert_eq!(
        painted,
        run(
            dir.path(),
            &["--membership", "--format", "text", "--layout", "lanes"],
        ),
        "the command bypassed the scene or changed the shipped bytes"
    );
    assert_eq!(scene.nodes.len(), graph.nodes.len());
    assert_eq!(scene.edges.len(), graph.edges.len());
    assert!(scene.edges.iter().all(|edge| !edge.cells.is_empty()));
}

/// Proof for `tui.graph_explorer_uses_semantic_scene`: the application model
/// retained by the interactive explorer is byte-for-byte the CLI's JSON view,
/// and its shared scene paints the CLI text for adversarial graph shapes.
#[test]
fn interactive_projection_matches_cli_json_and_text_for_shared_graph_shapes() {
    let dir = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    let tasks = [
        ("A", "root", "", ""),
        ("B", "left", "blocked_by: [A]\n", "tags: [left]\n"),
        ("C", "right", "blocked_by: [A]\n", ""),
        ("D", "merge", "blocked_by: [B, C]\n", ""),
        ("U", "wide \u{754c} disconnected", "", ""),
        ("X", "cycle one", "blocked_by: [Y]\n", ""),
        ("Y", "cycle two", "blocked_by: [X]\n", ""),
    ];
    for (id, title, dependency, tags) in tasks {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: {title}\nphase: 1\nstatus: todo\neffort_days: 1\n{dependency}{tags}---\n"
            ),
        )
        .expect("task");
    }
    let context = aplan_app::PlanContext::new(
        aplan_core::load_plan_identified(&dir.path().join("aplan")),
        aplan_app::ContextInputs {
            repository_root: dir.path().to_path_buf(),
            configuration_file: Some(dir.path().join(".aplan.yml")),
            trust: aplan_app::Trust::Hermetic,
            options: aplan_core::Options::default(),
            verdict: aplan_core::Verdict::default(),
        },
    );
    let workspace = aplan_app::graph_workspace(&context, aplan_app::task_catalogue(&context));

    for (request, args) in [
        (aplan_app::GraphRequest::default(), Vec::<&str>::new()),
        (
            aplan_app::GraphRequest {
                selector: aplan_app::Selector {
                    tags: vec!["left".to_string()],
                    ..aplan_app::Selector::default()
                },
                ..aplan_app::GraphRequest::default()
            },
            vec!["--tag", "left"],
        ),
    ] {
        let interactive = workspace.project(&request);
        let mut json_args = args.clone();
        json_args.extend(["--format", "json"]);
        let cli: aplan_output::GraphView =
            serde_json::from_str(&run(dir.path(), &json_args)).expect("CLI graph JSON");
        assert_eq!(interactive.view, cli, "interactive and CLI scopes diverged");

        let painted = painted_text(&interactive.view);
        let mut text_args = args;
        text_args.extend(["--format", "text", "--layout", "lanes"]);
        assert_eq!(painted, run(dir.path(), &text_args));
    }

    let whole = workspace.project(&aplan_app::GraphRequest::default());
    assert_eq!(
        whole
            .view
            .edges
            .iter()
            .filter(|edge| edge.from == "A")
            .count(),
        2
    );
    assert_eq!(
        whole
            .view
            .edges
            .iter()
            .filter(|edge| edge.to == "D")
            .count(),
        2
    );
    assert!(whole.view.nodes.iter().any(|node| node.id == "U"));
    let scene = aplan_output::graph_scene::GraphScene::layout(&whole.view, 95).expect("scene");
    assert!(scene.edges.iter().any(|edge| edge.omission.is_some()));
    assert!(
        scene
            .nodes
            .iter()
            .any(|node| node.title.contains('\u{754c}'))
    );

    let critical = workspace.project(&aplan_app::GraphRequest {
        scope: aplan_app::GraphScope::CriticalPath,
        ..aplan_app::GraphRequest::default()
    });
    assert_eq!(
        critical.state,
        aplan_app::GraphProjectionState::CyclicCriticalPath
    );
    assert!(critical.view.nodes.is_empty());
}

/// Proof for `graph.text_focus_and_critical_overlay`: presentation flags do
/// not change scope, and every emphasized relationship remains readable in a
/// redirected ASCII stream with no colour.
#[test]
fn cli_text_graph_focus_and_critical_overlay_are_exact_and_pipe_stable() {
    let dir = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(dir.path());

    let output = run(
        dir.path(),
        &[
            "--format",
            "text",
            "--layout",
            "lanes",
            "--focus",
            "G-2",
            "--critical",
        ],
    );
    for expected in [
        "[BC]",
        "[FC>]",
        "[RC>]",
        "G-1  [done]",
        "G-2  [todo]",
        "G-3  [todo]",
        "legend:",
        "`>` an exact critical step enters this node",
        "2/2 exact step(s) visible",
    ] {
        assert!(output.contains(expected), "missing {expected:?}:\n{output}");
    }
    assert!(
        !output.contains('\u{1b}'),
        "a redirected graph leaked ANSI: {output:?}"
    );
    assert!(
        output.contains(['*', 'o']) && !output.contains(['\u{25cf}', '\u{25cb}']),
        "a redirected graph is not deterministic ASCII:\n{output}"
    );
}

/// Proof for `graph.text_focus_and_critical_overlay`: focus never widens a
/// scope or fabricates a drawable dependency relationship.
#[test]
fn graph_focus_refuses_unknown_excluded_and_membership_only_tasks() {
    let dir = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(dir.path());

    for (args, expected) in [
        (
            vec![
                "graph", "--format", "text", "--layout", "lanes", "--focus", "NOPE",
            ],
            "requires an exact task id",
        ),
        (
            vec![
                "graph", "--format", "text", "--layout", "lanes", "--tag", "alpha", "--focus",
                "G-1",
            ],
            "changes emphasis, not scope",
        ),
        (
            vec![
                "graph", "--format", "text", "--layout", "lanes", "--focus", "M-a",
            ],
            "Add `--membership`",
        ),
    ] {
        let output = common::output(dir.path(), &args);
        assert!(!output.status.success(), "unexpected success: {args:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(expected), "missing {expected:?}: {error}");
    }
}

#[test]
fn graph_presentation_flags_are_refused_by_every_non_text_shape() {
    let dir = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(dir.path());

    for format in ["mermaid", "dot", "json"] {
        let output = common::output(
            dir.path(),
            &["graph", "--format", format, "--focus", "G-2", "--critical"],
        );
        assert!(
            !output.status.success(),
            "{format} accepted text decoration"
        );
        assert!(output.stdout.is_empty(), "{format} emitted partial output");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("require `--format text`"), "{error}");
    }
}

#[test]
fn a_cyclic_graph_says_why_the_critical_overlay_is_unavailable() {
    let dir = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(dir.path());
    for (id, blocker) in [("C-1", "C-2"), ("C-2", "C-1")] {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: cycle {id}\nphase: 0\nstatus: todo\neffort_days: 1\n\
                 blocked_by: [{blocker}]\n---\n"
            ),
        )
        .expect("cycle task");
    }

    let output = run(
        dir.path(),
        &["--format", "text", "--layout", "lanes", "--critical"],
    );
    assert!(
        output.contains(
            "critical route: unavailable because the dependency/containment graph is cyclic"
        ),
        "{output}"
    );

    let layered = run(
        dir.path(),
        &[
            "--format",
            "text",
            "--layout",
            "layered",
            "--critical",
            "--width",
            "240",
            "--ascii",
        ],
    );
    assert_eq!(
        layered,
        include_str!("snapshots/layered-cycle-ascii.txt"),
        "the typed feedback/cycle snapshot moved"
    );
}

#[test]
fn graph_help_separates_presentation_from_scope() {
    let dir = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(dir.path());
    let help = common::output(dir.path(), &["graph", "--help"]);
    assert!(help.status.success());
    let text = String::from_utf8_lossy(&help.stdout);
    for expected in [
        "--focus <ID>",
        "Presentation only",
        "--critical",
        "does not change scope",
        "Requires `--format text`",
        "--layout <LAYOUT>",
        "The compatibility row-and-lane document",
        "The shared phase/rank/card scene",
        "--level <LEVEL>",
        "--orientation <ORIENTATION>",
        "--order <ORDER>",
        "--detail <DETAIL>",
        "--width <COLUMNS>",
        "--ascii",
    ] {
        assert!(text.contains(expected), "missing {expected:?}:\n{text}");
    }
}

/// Proof for `graph.layered_text_uses_the_shared_semantic_scene` and
/// `graph.text_defaults_to_layered_with_explicit_lane_compatibility`: omitted
/// layout selects Layered while explicit Lanes retains the old renderer.
#[test]
fn layered_text_is_default_and_lanes_remains_exact_compatibility() {
    let dir = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(dir.path());

    let explicit_compatibility = run(dir.path(), &["--format", "text", "--layout", "lanes"]);
    let graph: aplan_output::GraphView =
        serde_json::from_str(&run(dir.path(), &["--format", "json"])).expect("graph JSON");
    assert_eq!(explicit_compatibility, painted_text(&graph));

    let args = ["--format", "text", "--width", "240", "--ascii"];
    let first = run(dir.path(), &args);
    let explicit_layered = run(
        dir.path(),
        &[
            "--format", "text", "--layout", "layered", "--width", "240", "--ascii",
        ],
    );
    assert_eq!(first, explicit_layered, "the omitted layout is not Layered");
    assert_ne!(
        first, explicit_compatibility,
        "the text default did not move"
    );
    let second = run(dir.path(), &args);
    assert_eq!(first, second, "redirected layered bytes moved between runs");
    assert_eq!(
        first,
        include_str!("snapshots/layered-plan-ascii.txt"),
        "the redirected compatibility snapshot moved"
    );
    assert!(
        !first.lines().any(|line| line.contains("#>")),
        "an automatic navigation anchor leaked as an explicit focus:\n{first}"
    );

    let focused = run(
        dir.path(),
        &[
            "--format", "text", "--layout", "layered", "--focus", "G-1", "--width", "240",
            "--ascii",
        ],
    );
    assert!(
        focused.lines().any(|line| line.contains("#>")),
        "explicit focus was not painted:\n{focused}"
    );

    let reordered = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    std::fs::create_dir_all(reordered.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(reordered.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    let mut sources: Vec<_> = std::fs::read_dir(dir.path().join("aplan/tasks"))
        .expect("task directory")
        .map(|entry| entry.expect("task entry").path())
        .collect();
    sources.sort();
    sources.reverse();
    for source in sources {
        let target = reordered
            .path()
            .join("aplan/tasks")
            .join(source.file_name().expect("task file name"));
        std::fs::write(&target, std::fs::read(&source).expect("task bytes"))
            .expect("reordered task");
    }
    assert_eq!(
        first,
        run(reordered.path(), &args),
        "filesystem creation order changed layered bytes"
    );

    let mut locale_args = vec!["graph"];
    locale_args.extend(args);
    let locale = common::command(dir.path(), &locale_args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .expect("locale run");
    assert!(locale.status.success(), "{locale:?}");
    assert_eq!(
        first.as_bytes(),
        locale.stdout,
        "locale changed layered bytes"
    );
    let cut = run(
        dir.path(),
        &[
            "--format", "text", "--layout", "layered", "--cut", "M-a", "--width", "240", "--ascii",
        ],
    );
    assert!(
        cut.contains("scope boundary dependencies (1): G-2 -> G-3"),
        "{cut}"
    );
    for expected in [
        "M-a",
        "M-b",
        "level: plan",
        "scope: whole plan",
        "5/5 level task(s) visible",
        "2/2 admitted dependency edge(s) represented",
    ] {
        assert!(first.contains(expected), "missing {expected:?}:\n{first}");
    }
    assert!(!first.contains('\u{1b}'), "redirected output carried ANSI");
}

#[test]
/// Proof for `graph.layered_text_uses_the_shared_semantic_scene`: semantic
/// level and layout policies alter the typed scene instead of post-processing
/// painted bytes. The exact top-to-bottom snapshot also proves
/// `graph.layered_phase_labels_are_horizontal_and_non_occluding`.
fn layered_levels_orientations_orders_and_detail_are_real_controls() {
    let dir = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(dir.path());

    let milestone = run(
        dir.path(),
        &[
            "--format",
            "text",
            "--layout",
            "layered",
            "--level",
            "milestone:M-a",
            "--orientation",
            "top-to-bottom",
            "--order",
            "status-readiness",
            "--detail",
            "compact",
            "--width",
            "400",
            "--ascii",
        ],
    );
    for expected in [
        "G-1",
        "G-2",
        "level: milestone:M-a",
        "direction: top-to-bottom",
        "order: status/readiness peers",
        "detail: compact",
    ] {
        assert!(
            milestone.contains(expected),
            "missing {expected:?}:\n{milestone}"
        );
    }
    assert!(
        !milestone.contains("a title with"),
        "compact leaked a title"
    );
    assert_eq!(
        milestone,
        include_str!("snapshots/layered-milestone-top-to-bottom-ascii.txt"),
        "the level/orientation/detail snapshot moved"
    );

    for order in [
        "readability",
        "plan-priority",
        "id",
        "title",
        "status-readiness",
        "criticality",
    ] {
        let output = run(
            dir.path(),
            &[
                "--format", "text", "--layout", "layered", "--level", "phase:0", "--order", order,
                "--width", "400", "--ascii",
            ],
        );
        assert!(output.contains("level: phase:0"), "{order}:\n{output}");
    }

    let task = run(
        dir.path(),
        &[
            "--format",
            "text",
            "--layout",
            "layered",
            "--level",
            "task:G-2",
            "--orientation",
            "left-to-right",
            "--width",
            "400",
            "--ascii",
        ],
    );
    assert!(task.contains("level: task:G-2"), "{task}");
    assert!(task.contains("direction: left-to-right"), "{task}");
}

#[test]
/// Proof for `graph.layered_text_uses_the_shared_semantic_scene` and
/// `graph.text_defaults_to_layered_with_explicit_lane_compatibility`:
/// text-only controls are rejected before plan loading and pipes cannot
/// provide an accidental width policy.
fn layered_controls_refuse_wrong_formats_and_missing_redirected_width_before_loading() {
    let dir = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(dir.path());

    for args in [
        vec!["graph", "--format", "mermaid", "--layout", "layered"],
        vec!["graph", "--format", "json", "--width", "120"],
        vec![
            "graph",
            "--format",
            "text",
            "--layout",
            "lanes",
            "--orientation",
            "auto",
        ],
    ] {
        let output = common::output(dir.path(), &args);
        assert!(!output.status.success(), "accepted {args:?}");
        assert!(output.stdout.is_empty(), "partial output for {args:?}");
    }

    for args in [
        vec!["graph", "--format", "text"],
        vec!["graph", "--format", "text", "--layout", "layered"],
    ] {
        let output = common::output(dir.path(), &args);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("redirected layered text requires `--width"),
            "{error}"
        );
    }

    for (extra, expected) in [
        (
            vec!["--tag", "alpha", "--focus", "G-1"],
            "outside the active graph scope",
        ),
        (
            vec!["--level", "milestone:M-b", "--focus", "G-1"],
            "not represented at the requested layered level",
        ),
    ] {
        let mut args = vec![
            "graph", "--format", "text", "--layout", "layered", "--width", "240",
        ];
        args.extend(extra);
        let output = common::output(dir.path(), &args);
        assert!(!output.status.success(), "accepted {args:?}");
        assert!(output.stdout.is_empty(), "partial output for {args:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(expected), "missing {expected:?}: {error}");
    }

    let output = common::output(
        dir.path(),
        &[
            "--root",
            "missing-plan",
            "graph",
            "--format",
            "text",
            "--layout",
            "layered",
        ],
    );
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("redirected layered text requires `--width")
            && !error.contains("not a directory"),
        "argument validation opened the plan first: {error}"
    );
}

#[test]
/// Proof for `graph.layered_text_uses_the_shared_semantic_scene` and
/// `graph.auto_text_orientation_prefers_a_scene_which_fits`: a static scene is
/// complete or refused, never clipped, and Auto reports the smaller complete
/// candidate when neither orientation fits.
fn layered_width_refusal_is_exact_and_emits_no_clipped_picture() {
    let dir = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(dir.path());
    let output = common::output(
        dir.path(),
        &[
            "graph", "--format", "text", "--layout", "layered", "--width", "20", "--ascii",
        ],
    );
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "a clipped graph escaped: {output:?}"
    );
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("needs") && error.contains("`--width 20`"),
        "{error}"
    );
    assert!(error.contains("--orientation top-to-bottom"), "{error}");

    let required = |error: &[u8]| {
        let error = String::from_utf8_lossy(error);
        error
            .split("needs ")
            .nth(1)
            .and_then(|tail| tail.split(" columns").next())
            .and_then(|columns| columns.parse::<usize>().ok())
            .unwrap_or_else(|| panic!("no required width in {error}"))
    };
    let mut forced_required = Vec::new();
    for orientation in ["left-to-right", "top-to-bottom"] {
        let forced = common::output(
            dir.path(),
            &[
                "graph",
                "--format",
                "text",
                "--layout",
                "layered",
                "--orientation",
                orientation,
                "--width",
                "20",
                "--ascii",
            ],
        );
        assert!(!forced.status.success(), "{orientation} unexpectedly fit");
        assert!(forced.stdout.is_empty(), "{orientation} emitted a clip");
        forced_required.push(required(&forced.stderr));
    }
    assert_eq!(
        required(&output.stderr),
        *forced_required.iter().min().expect("two forced candidates"),
        "Auto did not report its smallest usable width"
    );
}

#[test]
fn layered_high_fan_and_crossing_shape_accounts_every_node_and_edge() {
    let dir = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    for (id, blocked_by) in [
        ("A", ""),
        ("B", "blocked_by: [A]\n"),
        ("C", "blocked_by: [A]\n"),
        ("D", "blocked_by: [A]\n"),
        ("E", "blocked_by: [B, C, D]\n"),
        ("F", "blocked_by: [A, C]\n"),
    ] {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: node {id}\nphase: 0\nstatus: todo\neffort_days: 1\n{blocked_by}---\n"
            ),
        )
        .expect("task");
    }
    let args = [
        "--format",
        "text",
        "--layout",
        "layered",
        "--orientation",
        "left-to-right",
        "--detail",
        "compact",
        "--width",
        "400",
        "--ascii",
    ];
    let first = run(dir.path(), &args);
    assert_eq!(first, run(dir.path(), &args));
    for id in ["A", "B", "C", "D", "E", "F"] {
        assert!(
            first
                .lines()
                .any(|line| line.split(['|', '!']).any(|cell| cell.trim() == id)),
            "{id}:\n{first}"
        );
    }
    assert!(first.contains("6/6 level task(s) visible"), "{first}");
    assert!(
        first.contains("8/8 admitted dependency edge(s) represented"),
        "{first}"
    );
}

#[test]
fn layered_scope_level_and_numeric_refusals_are_specific_and_emit_nothing() {
    let dir = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(dir.path());

    for reach in ["upstream:G-3", "downstream:G-1", "between:G-1..G-3"] {
        let output = run(
            dir.path(),
            &[
                "--format", "text", "--layout", "layered", "--reach", reach, "--width", "400",
                "--ascii",
            ],
        );
        assert!(output.contains(&format!("reach `{reach}`")), "{output}");
    }

    let critical = run(
        dir.path(),
        &[
            "--format",
            "text",
            "--layout",
            "layered",
            "--focus",
            "G-2",
            "--critical",
            "--order",
            "criticality",
            "--width",
            "400",
            "--ascii",
        ],
    );
    assert!(critical.contains("critical route:"), "{critical}");
    assert!(critical.contains("visible cards marked `C`"), "{critical}");
    assert!(critical.contains("#>C"), "{critical}");

    for (extra, expected) in [
        (
            vec!["--width", "19"],
            "must be between 20 and 65535 terminal columns",
        ),
        (
            vec!["--width", "65536"],
            "must be between 20 and 65535 terminal columns",
        ),
        (
            vec!["--width", "240", "--level", "milestone:G-1"],
            "names an ordinary task",
        ),
        (
            vec!["--width", "240", "--level", "task:NOPE"],
            "no task `NOPE`",
        ),
        (
            vec!["--width", "240", "--focus", "NOPE"],
            "requires an exact task id",
        ),
        (
            vec!["--width", "240", "--tag", "does-not-exist"],
            "layered graph level is empty",
        ),
    ] {
        let mut args = vec!["graph", "--format", "text", "--layout", "layered"];
        args.extend(extra);
        let output = common::output(dir.path(), &args);
        assert!(!output.status.success(), "accepted {args:?}");
        assert!(output.stdout.is_empty(), "partial graph for {args:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(expected), "missing {expected:?}: {error}");
    }
}

#[test]
/// Proof for `graph.aggregate_levels_are_bounded_and_continuable`: an
/// oversized static plan level emits one accounted aggregate window instead
/// of reaching the cell-scene limit with the complete source.
fn layered_static_surface_explains_phase_boundaries_windows_and_layout_limits() {
    let phase = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(phase.path());
    std::fs::write(
        phase.path().join("aplan/tasks/G-4.md"),
        "---\nid: G-4\ntitle: later phase\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [G-3]\n---\n",
    )
    .expect("cross-phase task");
    let phase_output = run(
        phase.path(),
        &[
            "--format", "text", "--layout", "layered", "--level", "phase:0", "--width", "400",
            "--ascii",
        ],
    );
    assert!(
        phase_output.contains("phase boundary dependencies"),
        "{phase_output}"
    );

    let large = large_chain_fixture();

    let aggregate = run(
        large.path(),
        &[
            "--format",
            "text",
            "--layout",
            "layered",
            "--orientation",
            "top-to-bottom",
            "--detail",
            "compact",
            "--width",
            "80",
            "--ascii",
        ],
    );
    assert!(aggregate.contains("level: plan"), "{aggregate}");
    assert!(aggregate.contains("window continuations"), "{aggregate}");
    assert!(aggregate.contains("window omissions"), "{aggregate}");
    assert!(
        aggregate.contains("10/70 level task(s) visible"),
        "{aggregate}"
    );

    let window = run(
        large.path(),
        &[
            "--format",
            "text",
            "--layout",
            "layered",
            "--level",
            "task:T-35",
            "--orientation",
            "top-to-bottom",
            "--detail",
            "compact",
            "--width",
            "65535",
            "--ascii",
        ],
    );
    assert!(window.contains("level: task:T-35"), "{window}");
    assert!(window.contains("window continuation"), "{window}");
    assert!(window.contains("window omission"), "{window}");
}

fn large_chain_fixture() -> tempfile::TempDir {
    let large = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    std::fs::create_dir_all(large.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(large.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    for index in 0usize..70 {
        let id = format!("T-{index:02}");
        let blocked_by = index
            .checked_sub(1)
            .map_or_else(String::new, |prior| format!("blocked_by: [T-{prior:02}]\n"));
        std::fs::write(
            large.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: chain {index}\nphase: 0\nstatus: todo\neffort_days: 1\n{blocked_by}---\n"
            ),
        )
        .expect("large task");
    }
    large
}

#[test]
/// Proof for `graph.auto_text_orientation_prefers_a_scene_which_fits`: Auto
/// keeps both complete candidates until the static width is known, while an
/// explicitly forced direction remains forced.
fn layered_auto_orientation_chooses_a_complete_scene_which_fits() {
    let large = large_chain_fixture();
    for width in ["80", "120"] {
        let automatic = run(
            large.path(),
            &[
                "--format",
                "text",
                "--layout",
                "layered",
                "--orientation",
                "auto",
                "--detail",
                "compact",
                "--width",
                width,
                "--ascii",
            ],
        );
        assert!(automatic.contains("level: plan"), "{automatic}");
        assert!(automatic.contains("window continuations"), "{automatic}");
        assert!(automatic.contains("window omissions"), "{automatic}");
        assert!(
            automatic.contains("10/70 level task(s) visible"),
            "{automatic}"
        );
    }

    let forced = common::output(
        large.path(),
        &[
            "graph",
            "--format",
            "text",
            "--layout",
            "layered",
            "--orientation",
            "left-to-right",
            "--detail",
            "compact",
            "--width",
            "80",
            "--ascii",
        ],
    );
    assert!(
        !forced.status.success(),
        "forced direction unexpectedly fit"
    );
    assert!(forced.stdout.is_empty(), "forced direction emitted a clip");
    assert!(
        String::from_utf8_lossy(&forced.stderr).contains("needs 145 columns"),
        "{}",
        String::from_utf8_lossy(&forced.stderr)
    );
}

fn ids(doc: &serde_json::Value) -> BTreeSet<String> {
    doc["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .map(|n| n["id"].as_str().expect("an id").to_string())
        .collect()
}

/// The ids a Mermaid diagram declares as nodes.
fn mermaid_nodes(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter(|l| l.starts_with("    ") && !l.contains("-->"))
        .filter_map(|l| l.trim().split(['[', '(']).next())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .collect()
}

/// The ids a text drawing labels.
fn text_nodes(text: &str) -> BTreeSet<String> {
    // Only the drawing. A blank line separates it from the notes beneath, which
    // are prose: filtering rows by "carries a node glyph" matched the `o` in
    // "outside" and took `edge` for a task id. Structure, not glyphs.
    let drawing = text.split("\n\n").next().unwrap_or_default();
    drawing
        .lines()
        // A merge row is gutter only, and a folded title continues on a row
        // with no glyph; a task row is the one carrying a node.
        .filter(|l| l.contains(['*', 'o', '\u{25cf}', '\u{25cb}']))
        .filter_map(|l| {
            l.split_whitespace()
                .find(|w| w.starts_with(|c: char| c.is_alphanumeric()) && w.len() > 1)
        })
        .map(str::to_string)
        .collect()
}

/// The ids a DOT digraph declares as nodes.
fn dot_nodes(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter(|l| l.trim_start().starts_with('"') && l.contains("[label="))
        .filter_map(|l| l.trim().split('"').nth(1))
        .map(str::to_string)
        .collect()
}

/// The scope is one decision and the rendering is another.
#[test]
fn every_shape_sees_the_same_scope() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // The last one is the intersection: `--cut` and `--tag` together are one
    // question, not two, and a shape that applied only the first would still
    // agree with a json that applied only the first.
    for scope in [
        vec![],
        vec!["--cut", "M-a"],
        vec!["--tag", "alpha"],
        vec!["--cut", "M-a", "--tag", "alpha"],
    ] {
        let json = view(dir.path(), &scope);
        let expected = ids(&json);
        assert!(
            !expected.is_empty(),
            "the scope {scope:?} matched nothing, so this compares three empty sets"
        );

        let mut mermaid_args = scope.clone();
        mermaid_args.extend(["--format", "mermaid"]);
        let mut dot_args = scope.clone();
        dot_args.extend(["--format", "dot"]);

        assert_eq!(
            mermaid_nodes(&run(dir.path(), &mermaid_args)),
            expected,
            "mermaid and json disagree about what {scope:?} puts in view"
        );
        assert_eq!(
            dot_nodes(&run(dir.path(), &dot_args)),
            expected,
            "dot and json disagree about what {scope:?} puts in view"
        );

        let mut text_args = scope.clone();
        text_args.extend(["--format", "text", "--layout", "lanes"]);
        assert_eq!(
            text_nodes(&run(dir.path(), &text_args)),
            expected,
            "the text drawing and json disagree about what {scope:?} puts in view"
        );
    }
}

/// `--cut` really narrows, or the test above compares three identical wholes.
#[test]
fn a_cut_narrows_every_shape() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let whole = ids(&view(dir.path(), &[]));
    let scoped = ids(&view(dir.path(), &["--cut", "M-a"]));
    assert!(
        scoped.len() < whole.len(),
        "`--cut` did not narrow anything: {scoped:?} vs {whole:?}"
    );
    assert!(
        scoped.contains("G-1") && !scoped.contains("G-3"),
        "{scoped:?}"
    );
}

/// An edge with one end outside the scope is reported, not dropped: a view that
/// loses them quietly answers a question the reader did not ask (`A1-126`).
#[test]
fn an_edge_leaving_the_scope_is_counted_in_every_shape() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let json = view(dir.path(), &["--cut", "M-a"]);
    let hidden = json["hidden_edges"].as_u64().expect("a hidden count");
    assert!(
        hidden > 0,
        "the fixture has no edge crossing the cut: {json}"
    );

    for (format, marker) in [("mermaid", "%%"), ("dot", "//")] {
        let text = run(dir.path(), &["--cut", "M-a", "--format", format]);
        assert!(
            text.lines()
                .any(|l| l.trim_start().starts_with(marker) && l.contains("outside this view")),
            "`{format}` does not say that {hidden} edge(s) leave the view:\n{text}"
        );
    }
}

/// Structural, because Graphviz is not installed here and is not in the CI
/// image: a test that shelled out to `dot` would skip in exactly the place it
/// needs to run.
#[test]
fn the_dot_is_well_formed() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let text = run(dir.path(), &["--format", "dot"]);

    assert!(text.starts_with("digraph plan {"), "{text}");
    assert!(text.trim_end().ends_with('}'), "{text}");
    assert_eq!(
        text.matches('{').count(),
        text.matches('}').count(),
        "unbalanced braces:\n{text}"
    );

    let declared = dot_nodes(&text);
    let mut edges = 0_usize;
    for line in text.lines().filter(|l| l.contains(" -> ")) {
        let quoted: Vec<&str> = line.split('"').collect();
        // `    "from" -> "to";` splits to ["    ", from, " -> ", to, ";"]
        assert_eq!(quoted.len(), 5, "an edge is not two quoted ids: {line}");
        for end in [quoted[1], quoted[3]] {
            assert!(
                declared.contains(end),
                "edge names `{end}`, which no node declares:\n{text}"
            );
        }
        edges += 1;
    }
    assert!(
        edges > 0,
        "no edges, so the edge check proves nothing:\n{text}"
    );

    // Every quoted string closes. An odd count on a line means a title's own
    // quote escaped the label, which is the defect `escape` exists for on the
    // Mermaid side.
    for line in text.lines().filter(|l| l.contains('"')) {
        let unescaped = line
            .char_indices()
            .filter(|(i, c)| *c == '"' && (*i == 0 || line.as_bytes()[i - 1] != b'\\'))
            .count();
        assert_eq!(unescaped % 2, 0, "an unbalanced quote: {line}");
    }
}

/// A title carrying a quote and a newline. YAML permits both, and either one
/// unescaped ends a label early.
#[test]
fn a_hostile_title_survives_every_shape() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let json = view(dir.path(), &[]);
    let title = json["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["id"] == "G-1")
        .expect("G-1")["title"]
        .as_str()
        .expect("a title");
    assert!(
        title.contains('"') && title.contains('\n'),
        "the fixture's hostile title lost its hostility: {title:?}"
    );

    let mermaid = run(dir.path(), &["--format", "mermaid"]);
    let node = mermaid
        .lines()
        .find(|l| l.contains("G-1["))
        .or_else(|| mermaid.lines().find(|l| l.contains("G-1(")))
        .unwrap_or_else(|| panic!("no node for G-1:\n{mermaid}"));
    assert!(
        !node.contains(r#"a title with a ""#),
        "the quote was not escaped: {node}"
    );

    let dot = run(dir.path(), &["--format", "dot"]);
    let line = dot
        .lines()
        .find(|l| l.contains("\"G-1\" [label="))
        .unwrap_or_else(|| panic!("no node for G-1:\n{dot}"));
    assert!(
        line.contains(r#"\""#),
        "the quote was not escaped for DOT: {line}"
    );
    assert!(!line.contains('\n'), "a newline survived into a DOT label");
}

/// Proof for `graph.attended_default_is_layered_and_redirected_default_is_mermaid`:
/// redirected bare output retains the Mermaid contract scripts already use.
#[test]
fn redirected_bare_graph_remains_mermaid() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    assert_eq!(
        run(dir.path(), &[]),
        run(dir.path(), &["--format", "mermaid"]),
        "`graph` with no flag is no longer `--format mermaid`"
    );
}

/// Proof for `graph.attended_default_is_layered_and_redirected_default_is_mermaid`:
/// a person at a terminal reaches the spatial graph without knowing its format
/// spelling, while explicit export intent still wins in that same terminal.
#[test]
fn attended_bare_graph_is_layered_while_explicit_mermaid_stays_mermaid() {
    let dir = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(dir.path());

    let Some(attended) = common::on_a_terminal(dir.path(), 240, &["graph", "--ascii"]) else {
        return;
    };
    assert!(
        attended.contains("level: plan;") && attended.contains("accounting:"),
        "attended bare graph did not enter the layered renderer:\n{attended}"
    );
    assert!(!attended.contains("graph TD"), "attended graph was Mermaid");

    let Some(export) = common::on_a_terminal(dir.path(), 240, &["graph", "--format", "mermaid"])
    else {
        return;
    };
    assert!(
        export.contains("graph TD"),
        "explicit Mermaid was ignored:\n{export}"
    );
    assert!(
        !export.contains("level: plan;"),
        "Mermaid leaked layered notes"
    );
}

/// The formats `graph` refuses are refused by name, through the same table
/// every other command uses (`A1-184`).
#[test]
fn graph_refuses_the_formats_that_do_not_render_a_graph() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    for bad in ["human", "agent", "github", "sarif", "markdown"] {
        let out = common::output(dir.path(), &["graph", "--format", bad]);
        let said = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "`{bad}` was accepted: {said}");
        assert!(
            said.contains("renders a dependency graph") && said.contains(bad),
            "the refusal does not say what `graph` takes: {said}"
        );
    }
}

/// A backslash is the other character DOT gives meaning to inside a quoted
/// string, and a title may carry one.
#[test]
fn a_backslash_in_a_title_is_escaped_for_dot() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let title = view(dir.path(), &[])["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["id"] == "G-1")
        .expect("G-1")["title"]
        .as_str()
        .expect("a title")
        .to_string();
    assert!(
        title.contains('\\'),
        "the fixture's title carries no backslash: {title:?}"
    );

    let line = run(dir.path(), &["--format", "dot"])
        .lines()
        .find(|l| l.contains("\"G-1\" [label="))
        .expect("a node for G-1")
        .to_string();
    assert!(
        line.contains("back\\\\slash"),
        "the backslash was not doubled, so DOT reads it as an escape: {line}"
    );
}

/// A filter that matches nothing is refused, and refused the same way whichever
/// shape was asked for.
///
/// The refusal exists because an empty *diagram* renders as a blank rectangle
/// and reads as "this plan has nothing in it" (`A1-126`). An empty JSON document
/// would not be ambiguous, so the uniform answer is a decision rather than an
/// oversight: a filter matching nothing is a mistake in the question, and
/// handing a script an empty list lets it proceed as though the plan were empty.
#[test]
fn a_filter_matching_nothing_is_refused_in_every_shape() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    for format in ["mermaid", "dot", "json"] {
        let out = common::output(dir.path(), &["graph", "--tag", "nope", "--format", format]);
        let said = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`{format}` answered instead: {said}"
        );
        assert!(
            said.contains("no task is in view"),
            "`{format}` does not say why: {said}"
        );
        assert!(
            out.stdout.is_empty(),
            "`{format}` printed a document beside the refusal"
        );
    }
}

/// `aplan facts` says which shape redirected/programmatic `graph` emits with no flag.
///
/// The question was trivial while every command defaulted to `human`. This is
/// the first command that does not, and a consumer omitting `--format` cannot
/// otherwise tell what it will get (`A1-193`).
#[test]
fn facts_says_which_shape_graph_defaults_to() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let out = common::output(dir.path(), &["facts"]);
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("facts is JSON");

    let mut checked = 0_usize;
    for entry in doc["commands"].as_array().expect("commands") {
        let name = entry["name"].as_str().expect("a name");
        let formats = entry["formats"].as_array().expect("a formats list");
        if formats.is_empty() {
            assert!(
                entry["default_format"].is_null(),
                "`{name}` takes no format but names a default"
            );
            continue;
        }
        checked += 1;
        let default = entry["default_format"]
            .as_str()
            .unwrap_or_else(|| panic!("`{name}` takes formats but names no default: {entry}"));
        assert!(
            formats.iter().any(|f| f == default),
            "`{name}` defaults to `{default}`, which is not among what it takes: {entry}"
        );
    }
    assert!(checked > 10, "only {checked} commands were checked");

    let graph = doc["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|c| c["name"] == "graph")
        .expect("a graph command");
    assert_eq!(
        graph["default_format"], "mermaid",
        "graph no longer advertises Mermaid as its redirected default: {graph}"
    );
}

/// The drawing a person reads at a terminal (`A1-219`).
#[test]
fn the_text_drawing_shows_what_a_task_waits_on() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // G-3 waits on G-2, which waits on G-1: a chain, drawn in one column.
    let text = run(dir.path(), &["--format", "text", "--layout", "lanes"]);
    for id in ["G-1", "G-2", "G-3"] {
        assert!(text.contains(id), "{id} is not drawn:\n{text}");
    }
    assert!(
        text.lines().count() >= 5,
        "one row per task, and a row per merge:\n{text}"
    );
    // Piped, so the fallback table is what a CI log would capture.
    assert!(
        !text.contains(['\u{25cf}', '\u{25cb}']),
        "a pipe should get the ASCII table:\n{text}"
    );

    // A done task is filled and an unfinished one is hollow: the distinction
    // Mermaid rounds a box for and DOT draws an ellipse for.
    // The *gutter* of a task's row: everything left of its id. Checking the
    // whole line kept matching the `o` in a title -- "quote" here -- which is
    // the third time prose has been read as a glyph. The gutter is where the
    // glyphs are, so that is what to look at.
    let gutter = |id: &str| {
        let row = text
            .lines()
            .find(|l| l.contains(&format!("{id}  ")))
            .unwrap_or_else(|| panic!("no row for {id}:\n{text}"));
        row.split(id).next().unwrap_or_default().to_string()
    };
    assert!(
        gutter("G-1").contains('*') && !gutter("G-1").contains('o'),
        "G-1 is done and is not drawn filled: {:?}",
        gutter("G-1")
    );
    assert!(
        gutter("G-2").contains('o') && !gutter("G-2").contains('*'),
        "G-2 is not done and is not drawn hollow: {:?}",
        gutter("G-2")
    );
}

/// A scoped drawing says what it could not draw, as the other two shapes do.
///
/// It said nothing at all: `--cut M-a` hides an edge to `G-3` and the drawing
/// dropped it in silence, which is the one thing `A1-126` settled that every
/// shape has to honour.
#[test]
fn a_scoped_drawing_says_what_it_left_out() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let hidden = view(dir.path(), &["--cut", "M-a"])["hidden_edges"]
        .as_u64()
        .expect("a hidden count");
    assert!(
        hidden > 0,
        "the fixture hides no edge, so this proves nothing"
    );

    let text = run(
        dir.path(),
        &["--cut", "M-a", "--format", "text", "--layout", "lanes"],
    );
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("outside this view are not drawn")
            || flat.contains("outside this view is not drawn"),
        "the drawing does not say that {hidden} edge(s) left the view:\n{text}"
    );
}

/// A graph too wide to draw says so and says what narrows it, rather than
/// truncating -- which is the failure this repository refuses everywhere else.
#[test]
fn a_graph_too_wide_to_draw_says_what_to_narrow_by() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");

    // Sixty roots feeding one task: sixty edges in flight at once, which no
    // terminal has the columns for.
    let mut blockers = Vec::new();
    for i in 0..60 {
        let id = format!("W-{i:02}");
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: root {i}\nphase: 0\nstatus: todo\nowner: t\n\
                     effort_days: 1\n---\n\nbody\n"
            ),
        )
        .expect("task");
        blockers.push(id);
    }
    std::fs::write(
        dir.path().join("aplan/tasks/W-sink.md"),
        format!(
            "---\nid: W-sink\ntitle: waits on everything\nphase: 0\nstatus: todo\nowner: t\n\
             effort_days: 1\nblocked_by: [{}]\n---\n\nbody\n",
            blockers.join(", ")
        ),
    )
    .expect("sink");

    let out = common::output(
        dir.path(),
        &["graph", "--format", "text", "--layout", "lanes"],
    );
    let said = String::from_utf8_lossy(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(2),
        "it drew instead of refusing: {said}"
    );
    assert!(
        out.stdout.is_empty(),
        "it printed a truncated drawing beside the refusal"
    );
    assert!(
        said.contains("60 edges wide"),
        "it does not say how wide: {said}"
    );
    for narrow in ["--cut", "--tag", "mermaid", "dot"] {
        assert!(
            said.contains(narrow),
            "it does not offer `{narrow}`: {said}"
        );
    }

    // And the shapes with no width to fit still answer.
    for format in ["mermaid", "dot", "json"] {
        let ok = common::output(dir.path(), &["graph", "--format", format]);
        assert!(
            ok.status.success(),
            "`{format}` was refused for a width it does not have"
        );
    }
}

/// The two counts a narrowed, scoped view reports are disjoint.
///
/// `hidden_edges` is an edge with one end outside the scope; `grouping.edges` is
/// a membership edge inside it. An edge counted in both would tell a reader that
/// more was left out than there is, and the two are computed in different steps,
/// so nothing but a test says they do not overlap.
#[test]
fn an_edge_is_not_counted_as_both_hidden_and_grouping() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let scoped = view(dir.path(), &["--cut", "M-a"]);
    let whole = view(dir.path(), &[]);

    let hidden = scoped["hidden_edges"].as_u64().expect("hidden");
    let grouping = scoped["grouping"]["edges"].as_u64().expect("grouping");
    assert!(
        hidden > 0 && grouping > 0,
        "one of the two is zero: {scoped}"
    );

    // Every edge of the plan is in exactly one of: drawn, hidden by the scope,
    // or left out as grouping. `--membership --cut` counts what the scope
    // itself keeps, so the arithmetic has somewhere to land.
    let in_scope = view(dir.path(), &["--cut", "M-a", "--membership"]);
    let drawn_with_grouping = in_scope["edges"].as_array().expect("edges").len() as u64;
    let drawn = scoped["edges"].as_array().expect("edges").len() as u64;
    assert_eq!(
        drawn + grouping,
        drawn_with_grouping,
        "an edge is counted as grouping and drawn, or as neither: {scoped}"
    );
    assert_eq!(
        in_scope["hidden_edges"].as_u64(),
        Some(hidden),
        "narrowing changed how many edges the *scope* hides, which it must not"
    );
    assert!(
        whole["edges"].as_array().expect("edges").len() as u64 >= drawn,
        "the scoped view draws more than the whole plan has"
    );
}
