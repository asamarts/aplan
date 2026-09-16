//! The dependency graph as text a person can read at a terminal (`A1-219`).
//!
//! `graph` emitted Mermaid, which renders in a README and in a browser and
//! nowhere a person is standing when they ask the question, and `tree` is one
//! task's ancestry rather than a picture of the graph.
//!
//! **Lanes, not layers.** A layered picture is bounded by its widest layer, and
//! this plan's is 177 nodes wide; a lane rendering is bounded by how many edges
//! are in flight at once, which is 21 once grouping stops being drawn as
//! constraint. One row per task also means height grows with the plan and a
//! terminal already knows how to scroll. It is the same idiom `git log --graph`
//! uses, for the same reason.
//!
//! **Hand-rolled, on the measurement rather than on preference.** The subject is
//! 68 nodes, 65 edges and 21 peak lanes: small enough that the layout is not the
//! hard part, which is what a dependency would have been bought for. The one
//! crate close to fitting is a single maintainer's, and this is a core output
//! path whose bytes are pinned by the corpus -- pinning them to a dependency's
//! version is the caution `HLD` section 11 already raised.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::{GraphEdge, GraphNode, GraphView};

/// The glyphs a rendering is drawn with.
///
/// A table rather than a second renderer: box drawing is what makes this
/// readable and is not guaranteed in every terminal or CI log, and the two
/// differ only in which characters they draw, never in what they draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyphs {
    /// A task whose status satisfies a dependency: filled, because it is done.
    done: char,
    /// A task still to do: hollow. Mermaid rounds the box for this and DOT
    /// draws an ellipse, and a drawing that made no distinction would be the
    /// one shape that lost what the other two carry.
    node: char,
    vertical: char,
    horizontal: char,
    /// The lane the merge lands in, which continues below.
    tee: char,
    /// The rightmost lane of a merge, which ends here and turns left.
    corner: char,
    /// A lane between the merge's ends: it joins the run, which continues past
    /// it. Drawing a corner here would claim the run stopped.
    join: char,
    /// A lane passing through a horizontal run.
    cross: char,
}

impl Glyphs {
    /// Box drawing, which is what makes this readable.
    pub const UNICODE: Glyphs = Glyphs {
        done: '●',
        node: '○',
        vertical: '│',
        horizontal: '─',
        tee: '├',
        corner: '┘',
        join: '┴',
        cross: '┼',
    };

    /// For a terminal or a CI log without box drawing.
    pub const ASCII: Glyphs = Glyphs {
        done: '*',
        node: 'o',
        vertical: '|',
        horizontal: '-',
        tee: '+',
        corner: '\'',
        join: '+',
        cross: '+',
    };
}

/// Why a graph could not be drawn in the space available.
///
/// A renderer that silently truncated would be the failure this repository
/// refuses everywhere else, so this says how wide the graph is, how wide the
/// space is, and what narrows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TooWide {
    /// Edges in flight at the widest point: what the gutter has to hold.
    pub lanes: usize,
    /// Columns the gutter would need.
    pub needed: usize,
    /// Columns available.
    pub width: usize,
}

impl std::fmt::Display for TooWide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the graph is {} edges wide at its widest point, which needs {} columns of gutter \
             and leaves nothing for a title in {}. Narrow it with `--cut <id>` or `--tag <tag>`, \
             or ask for `--format mermaid` or `--format dot`, which have no width to fit",
            self.lanes, self.needed, self.width
        )
    }
}

/// A row of the drawing: the gutter, and what it is about.
struct Row {
    gutter: String,
    /// `None` for a connector row, which is gutter only.
    label: Option<(String, String)>,
}

/// The view as text, drawn with `glyphs` into `width` columns.
///
/// # Errors
///
/// [`TooWide`] when the gutter alone would fill the width, which is a graph to
/// narrow rather than a rendering to truncate.
pub fn text_of_view(view: &GraphView, glyphs: Glyphs, width: usize) -> Result<String, TooWide> {
    // Capped even when the caller has no width to give. Folding follows the
    // terminal and a pipe has no screen edge (`A1-206`), but this is a *picture*
    // drawn for a person, and a graph too wide to read is too wide whether or
    // not anyone is watching -- redirecting it does not make 218 lanes legible.
    // A title needs somewhere to start. The id is the shortest useful label, so
    // the gutter has to leave room for one and a space.
    const ROOM_FOR_A_LABEL: usize = 12;

    let width = width.min(crate::MAX_WIDTH);
    let order = topological(view);
    let rows = lay_out(view, &order, glyphs);

    let gutter = rows
        .iter()
        .map(|r| r.gutter.chars().count())
        .max()
        .unwrap_or(0);
    if gutter + ROOM_FOR_A_LABEL > width {
        return Err(TooWide {
            lanes: gutter.div_ceil(2),
            needed: gutter,
            width,
        });
    }

    // One id column, so the titles line up. Without it a four-character id
    // pushes its own title a column right of its neighbours', and a drawing
    // whose columns do not line up is harder to read than one with no columns.
    let id_width = rows
        .iter()
        .filter_map(|r| r.label.as_ref())
        .map(|(id, _)| crate::fold::display_width(id))
        .max()
        .unwrap_or(0);

    let mut out = String::new();
    for row in &rows {
        match &row.label {
            None => {
                let _ = writeln!(out, "{}", row.gutter.trim_end());
            }
            Some((id, title)) => {
                let prefix = format!("{:<gutter$}{id:<id_width$}  ", row.gutter);
                let _ = writeln!(out, "{}", crate::fold::fold_after(&prefix, title, width));
            }
        }
    }

    // What the drawing could not show. Mermaid and DOT say this in their own
    // comment syntax; this is read by a person, so it is a sentence. A scoped
    // view that silently drops the edges leaving it answers a question the
    // reader did not ask (`A1-126`), and this was the one shape that did.
    for note in view.notes() {
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", crate::fold::fold(&note, width, 0));
    }
    Ok(out)
}

/// Kahn's algorithm, ties broken by id.
///
/// Deterministic because the corpus pins these bytes: two runs over one plan
/// must draw the same picture, whatever order the filesystem walk produced.
fn topological(view: &GraphView) -> Vec<String> {
    let ids: BTreeSet<&str> = view.nodes.iter().map(|n| n.id.as_str()).collect();
    let mut successors: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut indegree: BTreeMap<&str, usize> = ids.iter().map(|id| (*id, 0)).collect();
    for GraphEdge { from, to, .. } in &view.edges {
        if successors
            .entry(from.as_str())
            .or_default()
            .insert(to.as_str())
        {
            *indegree.entry(to.as_str()).or_default() += 1;
        }
    }

    let mut ready: BTreeSet<&str> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut order = Vec::with_capacity(ids.len());
    while let Some(next) = ready.iter().next().copied() {
        ready.remove(next);
        order.push(next.to_string());
        for s in successors.get(next).into_iter().flatten() {
            let d = indegree.get_mut(*s).expect("a known successor");
            *d -= 1;
            if *d == 0 {
                ready.insert(s);
            }
        }
    }

    // A cycle leaves tasks unplaced. `check` reports one as `APLAN004` and this
    // command does not refuse it, so the rest are drawn after the acyclic part
    // rather than silently dropped.
    let placed: BTreeSet<&str> = order.iter().map(String::as_str).collect();
    let unplaced: Vec<String> = ids
        .iter()
        .filter(|id| !placed.contains(*id))
        .map(|id| (*id).to_string())
        .collect();
    order.extend(unplaced);
    order
}

/// Assign lanes and build the rows.
///
/// **A lane carries an edge, not a destination.** Collapsing every edge into one
/// task down to a single lane draws a much narrower picture and loses the fact
/// that matters most in a dependency graph: how many things a task is waiting
/// on. Fan-in is the question a reader brings to this drawing, so each edge gets
/// its own lane and they merge where they arrive.
fn lay_out(view: &GraphView, order: &[String], glyphs: Glyphs) -> Vec<Row> {
    let by_id: BTreeMap<&str, &GraphNode> = view.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut successors: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for GraphEdge { from, to, .. } in &view.edges {
        successors
            .entry(from.as_str())
            .or_default()
            .insert(to.as_str());
    }

    // Each lane is one edge in flight, named by the task it is heading to.
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::new();

    for id in order {
        let hits: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| l.as_deref() == Some(id.as_str()))
            .map(|(i, _)| i)
            .collect();
        let lane = match hits.first() {
            Some(first) => *first,
            None => free_lane(&mut lanes),
        };

        // Everything arriving from further right bends into this lane, on a row
        // of its own: a merge drawn on the node's own row would put two things
        // in one column.
        if hits.len() > 1 {
            rows.push(Row {
                gutter: merge_gutter(&lanes, lane, &hits, glyphs),
                label: None,
            });
        }
        for h in &hits {
            lanes[*h] = None;
        }

        let node = by_id.get(id.as_str());
        let mark = if node.is_some_and(|n| n.done) {
            glyphs.done
        } else {
            glyphs.node
        };
        rows.push(Row {
            gutter: node_gutter(&lanes, lane, mark, glyphs),
            label: node.map(|n| (n.id.clone(), n.title.clone())),
        });

        // One lane per task waiting on this one. The first reuses the lane this
        // task was drawn in, so a chain stays in one column.
        for (n, s) in successors
            .get(id.as_str())
            .into_iter()
            .flatten()
            .enumerate()
        {
            if n == 0 {
                lanes[lane] = Some((*s).to_string());
            } else {
                let free = free_lane(&mut lanes);
                lanes[free] = Some((*s).to_string());
            }
        }
        trim(&mut lanes);
    }
    rows
}

/// The leftmost lane carrying nothing, or a new one on the right.
fn free_lane(lanes: &mut Vec<Option<String>>) -> usize {
    if let Some(i) = lanes.iter().position(Option::is_none) {
        i
    } else {
        lanes.push(None);
        lanes.len() - 1
    }
}

/// Drop empty lanes off the right, so the gutter is as narrow as the graph.
fn trim(lanes: &mut Vec<Option<String>>) {
    while lanes.last().is_some_and(Option::is_none) {
        lanes.pop();
    }
}

/// `● │ │` -- the node in its lane, drawn with `mark`, and every other lane
/// passing through.
fn node_gutter(lanes: &[Option<String>], lane: usize, mark: char, glyphs: Glyphs) -> String {
    let mut s = String::new();
    for (i, l) in lanes.iter().enumerate() {
        s.push(if i == lane {
            mark
        } else if l.is_some() {
            glyphs.vertical
        } else {
            ' '
        });
        s.push(' ');
    }
    s
}

/// `├─┴─┘` -- lanes to the right bending into `lane`, which continues below.
fn merge_gutter(lanes: &[Option<String>], lane: usize, hits: &[usize], glyphs: Glyphs) -> String {
    let last = hits.last().copied().unwrap_or(lane);
    let mut s = String::new();
    for (i, l) in lanes.iter().enumerate() {
        let (here, after) = if i == lane {
            (glyphs.tee, glyphs.horizontal)
        } else if hits.contains(&i) {
            if i < last {
                (glyphs.join, glyphs.horizontal)
            } else {
                (glyphs.corner, ' ')
            }
        } else if i < last {
            // A lane the run passes over, still open below.
            (
                if l.is_some() {
                    glyphs.cross
                } else {
                    glyphs.horizontal
                },
                glyphs.horizontal,
            )
        } else if l.is_some() {
            (glyphs.vertical, ' ')
        } else {
            (' ', ' ')
        };
        s.push(here);
        s.push(after);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use aplan_core::Status;

    /// A view from `(id, blocked_by)` pairs.
    fn view(spec: &[(&str, &[&str])]) -> GraphView {
        GraphView {
            nodes: spec
                .iter()
                .map(|(id, _)| GraphNode {
                    id: (*id).to_string(),
                    title: format!("task {id}"),
                    status: Status::Todo,
                    done: false,
                })
                .collect(),
            edges: spec
                .iter()
                .flat_map(|(id, deps)| {
                    deps.iter().map(move |d| GraphEdge {
                        from: (*d).to_string(),
                        to: (*id).to_string(),
                        kind: aplan_core::EdgeKind::Dependency,
                    })
                })
                .collect(),
            hidden_edges: 0,
            grouping: crate::Grouping::default(),
            scope: Vec::new(),
        }
    }

    fn drawn(spec: &[(&str, &[&str])]) -> String {
        text_of_view(&view(spec), Glyphs::UNICODE, 60).expect("it fits")
    }

    /// A chain is one column: the lane a task was drawn in carries the first
    /// thing waiting on it, so nothing wanders sideways for no reason.
    #[test]
    fn a_chain_stays_in_one_column() {
        let out = drawn(&[("A", &[]), ("B", &["A"]), ("C", &["B"])]);
        assert_eq!(
            out,
            "○ A  task A\n\
             ○ B  task B\n\
             ○ C  task C\n"
        );
    }

    /// Fan-in is the question a reader brings to a dependency graph, so every
    /// edge has its own lane and they merge where they arrive.
    #[test]
    fn a_fan_in_merges_where_it_arrives() {
        let out = drawn(&[("A", &[]), ("B", &[]), ("C", &["A", "B"])]);
        assert_eq!(
            out,
            "○   A  task A\n\
             │ ○ B  task B\n\
             ├─┘\n\
             ○   C  task C\n"
        );
    }

    /// Fan-out opens a lane per dependent, and a lane closes as soon as what it
    /// carried has been drawn -- so the column B occupied is blank under it
    /// rather than a bar leading nowhere.
    #[test]
    fn a_fan_out_opens_a_lane_for_each() {
        let out = drawn(&[("A", &[]), ("B", &["A"]), ("C", &["A"])]);
        assert_eq!(
            out,
            "○   A  task A\n\
             ○ │ B  task B\n  \
             ○ C  task C\n"
        );
    }

    /// The two drawings differ in which characters they use and in nothing
    /// else, which is what makes the fallback a table rather than a second
    /// renderer.
    #[test]
    fn the_ascii_drawing_differs_only_in_its_glyphs() {
        let v = view(&[
            ("A", &[]),
            ("B", &[]),
            ("C", &["A", "B"]),
            ("D", &["C"]),
            ("E", &["C"]),
        ]);
        let unicode = text_of_view(&v, Glyphs::UNICODE, 60).expect("it fits");
        let ascii = text_of_view(&v, Glyphs::ASCII, 60).expect("it fits");
        assert_ne!(unicode, ascii, "the two tables produced the same bytes");

        let translated: String = unicode
            .chars()
            .map(|c| match c {
                '●' => '*',
                '○' => 'o',
                '│' => '|',
                '─' => '-',
                '├' | '┼' | '┴' => '+',
                '┘' => '\'',
                c => c,
            })
            .collect();
        assert_eq!(
            translated, ascii,
            "the fallback draws something different, not the same thing differently"
        );
    }

    /// `done` is the distinction Mermaid rounds a box for and DOT draws an
    /// ellipse for, and the drawing a person reads must not be the one shape
    /// that loses it.
    #[test]
    fn a_done_task_is_filled_and_an_unfinished_one_is_hollow() {
        let mut v = view(&[("A", &[]), ("B", &["A"])]);
        v.nodes[0].done = true;
        v.nodes[0].status = Status::Done;
        let out = text_of_view(&v, Glyphs::UNICODE, 60).expect("fits");
        assert_eq!(out, "● A  task A\n○ B  task B\n");

        let ascii = text_of_view(&v, Glyphs::ASCII, 60).expect("fits");
        assert_eq!(ascii, "* A  task A\no B  task B\n");
    }

    /// A scoped drawing says what it could not show, as the other two shapes do
    /// in their comment syntax. This one said nothing at all.
    #[test]
    fn a_scoped_drawing_says_what_it_could_not_draw() {
        let mut v = view(&[("A", &[]), ("B", &["A"])]);
        v.hidden_edges = 3;
        let out = text_of_view(&v, Glyphs::UNICODE, 60).expect("fits");
        // Compared against the note as one line: it is prose and folds like
        // prose, so a phrase may be split across two rows.
        let flat = out.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains("3 edges to tasks outside this view are not drawn"),
            "the drawing dropped them silently:\n{out}"
        );
        assert!(
            flat.contains("`aplan tree <id>` follows them"),
            "and does not say how to follow them:\n{out}"
        );
        for line in out.lines() {
            assert!(crate::fold::display_width(line) <= 60, "{line}");
        }
    }

    /// One id column, so the titles line up under each other.
    #[test]
    fn the_titles_line_up_whatever_the_ids_are() {
        let out = drawn(&[("A", &[]), ("LONGER-ID", &[])]);
        let columns: Vec<usize> = out.lines().filter_map(|l| l.find("task ")).collect();
        assert_eq!(columns.len(), 2, "{out}");
        assert_eq!(
            columns[0], columns[1],
            "the titles start in different columns:\n{out}"
        );
    }

    /// A graph too wide to draw says how wide it is and what narrows it, rather
    /// than truncating.
    #[test]
    fn a_graph_too_wide_says_what_to_narrow_by() {
        // Twenty roots feeding one task: twenty edges in flight at once.
        let roots: Vec<String> = (0..20).map(|i| format!("R{i:02}")).collect();
        let refs: Vec<&str> = roots.iter().map(String::as_str).collect();
        let mut spec: Vec<(&str, &[&str])> = refs.iter().map(|r| (*r, &[][..])).collect();
        spec.push(("SINK", &refs));

        let err = text_of_view(&view(&spec), Glyphs::UNICODE, 40).expect_err("it cannot fit");
        assert_eq!(err.lanes, 20);
        assert_eq!(err.needed, 40, "two columns of gutter per lane");
        assert_eq!(err.width, 40);

        // Wide enough for the gutter *and* a label is where it starts drawing:
        // the refusal is about leaving room for a title, not about the gutter
        // alone fitting.
        // 40 columns of gutter plus the twelve a label needs.
        assert!(text_of_view(&view(&spec), Glyphs::UNICODE, 52).is_ok());
        assert!(text_of_view(&view(&spec), Glyphs::UNICODE, 51).is_err());

        let said = err.to_string();
        for expected in ["20 edges wide", "--cut", "--tag", "mermaid", "dot"] {
            assert!(
                said.contains(expected),
                "the refusal omits {expected}: {said}"
            );
        }
    }

    /// The same plan draws the same picture, whatever order the walk produced.
    #[test]
    fn the_drawing_does_not_depend_on_input_order() {
        let forward = view(&[("A", &[]), ("B", &[]), ("C", &["A", "B"])]);
        let mut reversed = view(&[("C", &["A", "B"]), ("B", &[]), ("A", &[])]);
        reversed.nodes.reverse();
        reversed.edges.reverse();
        assert_eq!(
            text_of_view(&forward, Glyphs::UNICODE, 60).expect("fits"),
            text_of_view(&reversed, Glyphs::UNICODE, 60).expect("fits"),
        );
    }

    /// A cyclic plan is `APLAN004`, which `check` reports and this command does
    /// not refuse, so every task is still drawn rather than silently dropped.
    #[test]
    fn a_cycle_still_draws_every_task() {
        let out = text_of_view(
            &view(&[("A", &["B"]), ("B", &["A"]), ("C", &[])]),
            Glyphs::UNICODE,
            60,
        )
        .expect("fits");
        for id in ["A", "B", "C"] {
            assert!(
                out.contains(&format!(" {id}  task {id}")),
                "{id} is missing:\n{out}"
            );
        }
    }

    /// A long title folds under itself rather than under the gutter (`A1-206`).
    #[test]
    fn a_long_title_folds_under_the_title() {
        let mut v = view(&[("A", &[]), ("B", &[]), ("C", &["A", "B"])]);
        v.nodes[2].title = "a title long enough that it has to fold somewhere sensible".into();
        let out = text_of_view(&v, Glyphs::UNICODE, 40).expect("fits");
        let continuation = out
            .lines()
            .find(|l| l.contains("sensible"))
            .expect("a folded line");
        assert!(
            continuation.starts_with("       "),
            "the continuation did not line up under the title: {continuation:?}"
        );
        for line in out.lines() {
            assert!(crate::fold::display_width(line) <= 40, "{line}");
        }
    }
}
