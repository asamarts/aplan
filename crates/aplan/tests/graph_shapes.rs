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

mod common;

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
        text_args.extend(["--format", "text"]);
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

/// Mermaid is still what `graph` prints with no flag, so the shape a reader
/// already depends on did not move under them.
#[test]
fn mermaid_is_still_the_default() {
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

/// `aplan facts` says which shape `graph` emits with no flag.
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
        "graph no longer advertises mermaid as its default: {graph}"
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
    let text = run(dir.path(), &["--format", "text"]);
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

    let text = run(dir.path(), &["--cut", "M-a", "--format", "text"]);
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

    let out = common::output(dir.path(), &["graph", "--format", "text"]);
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
