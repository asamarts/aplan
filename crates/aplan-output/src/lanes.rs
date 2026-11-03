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

use crate::GraphView;
use crate::graph_scene::{ConnectorKind, GraphScene, SceneEdgeId, SceneTextOwner, display_columns};
use aplan_core::graph::CriticalRoute;

const ROLE_COLUMNS: usize = 6;

/// Whether the terminal graph projects the whole-plan critical route.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CriticalOverlay<'a> {
    /// Do not compute or paint the route.
    Off,
    /// The route is undefined because the ordering graph is cyclic.
    Unavailable,
    /// The exact route chosen by core critical-path analysis.
    Route(&'a CriticalRoute),
}

/// Human-only emphasis applied over a semantic graph scene.
///
/// Neither field changes membership, ordering or machine data. `focus` marks
/// one already-visible node and its visible direct dependency neighbours;
/// `critical` projects the whole-plan route over whichever portion is in view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphPresentation<'a> {
    pub focus: Option<&'a str>,
    pub critical: CriticalOverlay<'a>,
}

impl Default for GraphPresentation<'_> {
    fn default() -> Self {
        Self {
            focus: None,
            critical: CriticalOverlay::Off,
        }
    }
}

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

    const fn connector(self, kind: ConnectorKind) -> char {
        match kind {
            ConnectorKind::Node { done: true } => self.done,
            ConnectorKind::Node { done: false } => self.node,
            ConnectorKind::Vertical => self.vertical,
            ConnectorKind::Horizontal => self.horizontal,
            ConnectorKind::Tee => self.tee,
            ConnectorKind::Corner => self.corner,
            ConnectorKind::Join => self.join,
            ConnectorKind::Cross => self.cross,
        }
    }

    const fn critical_connector(self, kind: ConnectorKind) -> char {
        match kind {
            // Status remains the node marker's meaning. The adjacent role
            // badge carries critical-node identity without stealing it.
            ConnectorKind::Node { .. } => self.connector(kind),
            ConnectorKind::Vertical => {
                if self.vertical == '│' {
                    '┃'
                } else {
                    '!'
                }
            }
            ConnectorKind::Horizontal => {
                if self.horizontal == '─' {
                    '━'
                } else {
                    '='
                }
            }
            ConnectorKind::Tee => {
                if self.tee == '├' {
                    '┣'
                } else {
                    '#'
                }
            }
            ConnectorKind::Corner => {
                if self.corner == '┘' {
                    '┛'
                } else {
                    '#'
                }
            }
            ConnectorKind::Join => {
                if self.join == '┴' {
                    '┻'
                } else {
                    '#'
                }
            }
            ConnectorKind::Cross => {
                if self.cross == '┼' {
                    '╋'
                } else {
                    '#'
                }
            }
        }
    }
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
             and leaves nothing for a title in {}. Narrow it with `--cut <id>`, `--tag <tag>` or \
             `--reach upstream:<id>`/`downstream:<id>`; then use `--focus <id>` to anchor the \
             subject. Or ask for `--format mermaid` or `--format dot`, which have no width to fit",
            self.lanes, self.needed, self.width
        )
    }
}

/// The view as text, drawn with `glyphs` into `width` columns.
///
/// # Errors
///
/// [`TooWide`] when the gutter alone would fill the width, which is a graph to
/// narrow rather than a rendering to truncate.
pub fn text_of_view(view: &GraphView, glyphs: Glyphs, width: usize) -> Result<String, TooWide> {
    GraphScene::layout(view, width).map(|scene| text_of_scene(&scene, glyphs))
}

/// Paint a focused, status-bearing terminal graph over the canonical scene.
///
/// This is deliberately separate from [`text_of_scene`]. Machine renderers and
/// the TUI keep consuming the undecorated semantic geometry; the CLI's human
/// text adapter adds role badges, exact status words, heavy critical edges and
/// a legend without mutating the [`GraphView`]. Stripping ANSI leaves the same
/// non-colour cues.
pub fn text_of_presented_view(
    view: &GraphView,
    glyphs: Glyphs,
    width: usize,
    presentation: GraphPresentation<'_>,
) -> Result<String, TooWide> {
    // Exact statuses belong in the human label, but not in GraphView's machine
    // title field or in the TUI's shared scene. Prefixing the title here lets
    // the canonical wrapper account for their width and keeps long titles from
    // being clipped by a post-layout suffix.
    let mut presented = view.clone();
    for node in &mut presented.nodes {
        node.title = format!("[{}] {}", node.status.as_str(), node.title);
    }
    let scene_width = width.saturating_sub(ROLE_COLUMNS);
    let scene = GraphScene::layout(&presented, scene_width).map_err(|mut too_wide| {
        too_wide.needed = too_wide.needed.saturating_add(ROLE_COLUMNS);
        too_wide.width = width;
        too_wide
    })?;
    Ok(text_of_presented_scene(&scene, glyphs, width, presentation))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct NodeRole {
    focus: FocusRole,
    critical: CriticalRole,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum FocusRole {
    #[default]
    None,
    Focus,
    Blocker,
    Dependent,
    BlockerAndDependent,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum CriticalRole {
    #[default]
    None,
    Node,
    Incoming,
    NodeAndIncoming,
}

impl NodeRole {
    fn badge(self) -> String {
        let mut letters = String::new();
        match self.focus {
            FocusRole::None => {}
            FocusRole::Focus => letters.push('F'),
            FocusRole::Blocker => letters.push('B'),
            FocusRole::Dependent => letters.push('R'),
            FocusRole::BlockerAndDependent => letters.push_str("BR"),
        }
        match self.critical {
            CriticalRole::None => {}
            CriticalRole::Node => letters.push('C'),
            CriticalRole::Incoming => letters.push('>'),
            CriticalRole::NodeAndIncoming => letters.push_str("C>"),
        }
        if letters.is_empty() {
            return " ".repeat(ROLE_COLUMNS);
        }
        let mut badge = format!("[{letters}]");
        badge.push_str(&" ".repeat(ROLE_COLUMNS.saturating_sub(badge.len())));
        badge
    }

    fn style(self) -> anstyle::Style {
        match (self.focus, self.critical) {
            (FocusRole::Focus, _) => crate::style::HEADING,
            (FocusRole::Blocker | FocusRole::BlockerAndDependent, _) => crate::style::WARNING,
            (FocusRole::Dependent, _) => crate::style::OK,
            (FocusRole::None, CriticalRole::Node | CriticalRole::NodeAndIncoming) => {
                crate::style::ID
            }
            (FocusRole::None, CriticalRole::None | CriticalRole::Incoming) => {
                crate::style::status("todo")
            }
        }
    }

    const fn is_critical(self) -> bool {
        matches!(
            self.critical,
            CriticalRole::Node | CriticalRole::NodeAndIncoming
        )
    }
}

struct PresentationFacts<'a> {
    roles: BTreeMap<&'a str, NodeRole>,
    critical_steps: BTreeSet<(&'a str, &'a str)>,
    total_critical_nodes: usize,
    total_critical_steps: usize,
}

fn presentation_facts<'a>(
    scene: &'a GraphScene,
    presentation: GraphPresentation<'a>,
) -> PresentationFacts<'a> {
    let focus = presentation.focus;
    let mut blockers = BTreeSet::new();
    let mut dependents = BTreeSet::new();
    if let Some(focus) = focus {
        for edge in &scene.edges {
            if edge.id.kind != aplan_core::EdgeKind::Dependency {
                continue;
            }
            if edge.id.to == focus {
                blockers.insert(edge.id.from.as_str());
            }
            if edge.id.from == focus {
                dependents.insert(edge.id.to.as_str());
            }
        }
    }
    let (critical_nodes, critical_steps, total_critical_nodes, total_critical_steps) =
        match presentation.critical {
            CriticalOverlay::Route(route) => (
                route.nodes.iter().map(String::as_str).collect(),
                route
                    .steps
                    .iter()
                    .map(|step| (step.from.as_str(), step.to.as_str()))
                    .collect(),
                route.nodes.len(),
                route.steps.len(),
            ),
            CriticalOverlay::Off | CriticalOverlay::Unavailable => {
                (BTreeSet::new(), BTreeSet::new(), 0, 0)
            }
        };
    let roles = scene
        .nodes
        .iter()
        .map(|node| {
            let is_focus = focus == Some(node.id.as_str());
            let critical_incoming = scene.edges.iter().any(|edge| {
                edge.omission.is_none()
                    && edge.id.to == node.id
                    && critical_edge(&edge.id, &critical_steps)
            });
            let focus = if is_focus {
                FocusRole::Focus
            } else {
                match (
                    blockers.contains(node.id.as_str()),
                    dependents.contains(node.id.as_str()),
                ) {
                    (false, false) => FocusRole::None,
                    (true, false) => FocusRole::Blocker,
                    (false, true) => FocusRole::Dependent,
                    (true, true) => FocusRole::BlockerAndDependent,
                }
            };
            let critical = match (critical_nodes.contains(node.id.as_str()), critical_incoming) {
                (false, false) => CriticalRole::None,
                (true, false) => CriticalRole::Node,
                (false, true) => CriticalRole::Incoming,
                (true, true) => CriticalRole::NodeAndIncoming,
            };
            (node.id.as_str(), NodeRole { focus, critical })
        })
        .collect();
    PresentationFacts {
        roles,
        critical_steps,
        total_critical_nodes,
        total_critical_steps,
    }
}

fn text_of_presented_scene(
    scene: &GraphScene,
    glyphs: Glyphs,
    width: usize,
    presentation: GraphPresentation<'_>,
) -> String {
    let facts = presentation_facts(scene, presentation);
    let nodes: BTreeMap<&str, _> = scene
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    let id_width = scene
        .nodes
        .iter()
        .map(|node| display_columns(&node.display_id))
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for row in scene.rows() {
        let role = match &row.text_owner {
            Some(SceneTextOwner::Node(id))
                if nodes
                    .get(id.as_str())
                    .is_some_and(|node| node.marker.row == row.index) =>
            {
                facts.roles.get(id.as_str()).copied()
            }
            _ => None,
        };
        let body = presented_row(
            row,
            scene,
            &nodes,
            id_width,
            glyphs,
            &facts.critical_steps,
            role.unwrap_or_default(),
        );
        let graph_row = matches!(row.text_owner, Some(SceneTextOwner::Node(_)))
            || (!body.is_empty() && row.text_owner.is_none());
        if graph_row {
            let badge = role.unwrap_or_default().badge();
            let badge_style = role.unwrap_or_default().style();
            if role.is_some_and(|role| role != NodeRole::default()) {
                let _ = write!(out, "{badge_style}{badge}{badge_style:#}");
            } else {
                out.push_str(&badge);
            }
        }
        out.push_str(&body);
        out.push('\n');
    }
    out.push_str(&presentation_notes(
        scene,
        glyphs,
        width,
        presentation,
        &facts,
    ));
    out
}

fn presentation_notes(
    scene: &GraphScene,
    glyphs: Glyphs,
    width: usize,
    presentation: GraphPresentation<'_>,
    facts: &PresentationFacts<'_>,
) -> String {
    let mut out = String::new();
    out.push('\n');
    out.push_str(&legend(glyphs, width));
    if let Some(focus) = presentation.focus {
        out.push_str(&crate::fold::fold(
            &format!(
                "focus: `{focus}`; B and R mark direct dependency edges visible in this scope."
            ),
            width,
            0,
        ));
        out.push('\n');
    }
    match presentation.critical {
        CriticalOverlay::Off => {}
        CriticalOverlay::Unavailable => {
            out.push_str(&crate::fold::fold(
                "critical route: unavailable because the dependency/containment graph is cyclic.",
                width,
                0,
            ));
            out.push('\n');
        }
        CriticalOverlay::Route(_) if facts.total_critical_nodes == 0 => {
            out.push_str(&crate::fold::fold(
                "critical route: none; this plan has no live work task to project.",
                width,
                0,
            ));
            out.push('\n');
        }
        CriticalOverlay::Route(_) => {
            let visible_nodes = facts
                .roles
                .values()
                .filter(|role| role.is_critical())
                .count();
            // A route step is an identity, not an edge record. `GraphScene`
            // deliberately preserves repeated and differently typed edges with
            // equal endpoints, and counting those records could claim that two
            // of one exact steps were visible. Ask once per route step whether
            // any drawable scene edge represents it instead.
            let visible_steps = facts
                .critical_steps
                .iter()
                .filter(|(from, to)| {
                    scene.edges.iter().any(|edge| {
                        edge.omission.is_none() && edge.id.from == *from && edge.id.to == *to
                    })
                })
                .count();
            out.push_str(&crate::fold::fold(
                &format!(
                    "critical route: {visible_nodes}/{} node(s) and {visible_steps}/{} exact \
                     step(s) visible; the deterministic whole-plan route includes checkpoint \
                     passthroughs, and its work-only tasks match `critical-path`.",
                    facts.total_critical_nodes, facts.total_critical_steps,
                ),
                width,
                0,
            ));
            out.push('\n');
        }
    }
    out
}

fn critical_edge(edge: &SceneEdgeId, critical: &BTreeSet<(&str, &str)>) -> bool {
    critical.contains(&(edge.from.as_str(), edge.to.as_str()))
}

fn presented_row(
    row: &crate::graph_scene::SceneRow,
    scene: &GraphScene,
    nodes: &BTreeMap<&str, &crate::graph_scene::SceneNode>,
    id_width: usize,
    glyphs: Glyphs,
    critical_steps: &BTreeSet<(&str, &str)>,
    role: NodeRole,
) -> String {
    let mut gutter = vec![None; row.gutter_columns];
    for cell in &row.connectors {
        if let Some(slot) = gutter.get_mut(cell.id.0.column) {
            let critical = cell
                .edges
                .iter()
                .any(|edge| critical_edge(edge, critical_steps));
            *slot = Some((
                if critical {
                    glyphs.critical_connector(cell.kind)
                } else {
                    glyphs.connector(cell.kind)
                },
                critical,
            ));
        }
    }
    let mut line = String::new();
    for cell in gutter {
        match cell {
            Some((glyph, true)) => {
                let style = crate::style::ID;
                let _ = write!(line, "{style}{glyph}{style:#}");
            }
            Some((glyph, false)) => line.push(glyph),
            None => line.push(' '),
        }
    }
    let tail = &row.plain_text[row.gutter_columns..];
    let Some(SceneTextOwner::Node(id)) = &row.text_owner else {
        line.push_str(tail);
        if row.trim_gutter {
            line.truncate(line.trim_end().len());
        }
        return line;
    };
    let node = nodes[id.as_str()];
    if row.index != node.marker.row {
        line.push_str(tail);
        return line;
    }

    let id_column = node
        .text_spans
        .first()
        .map_or(0, |span| span.column)
        .saturating_sub(row.gutter_columns);
    let (before_id, after_before) = tail.split_at(id_column.min(tail.len()));
    let (display_id, after_id) = after_before.split_at(node.display_id.len());
    line.push_str(before_id);
    let id_style = if role == NodeRole::default() {
        crate::style::ID
    } else {
        role.style()
    };
    let _ = write!(line, "{id_style}{display_id}{id_style:#}");
    let gap = id_width
        .saturating_sub(display_columns(&node.display_id))
        .saturating_add(2);
    let (spacing, title) = after_id.split_at(gap.min(after_id.len()));
    line.push_str(spacing);
    let status = format!("[{}]", node.status.as_str());
    if let Some(rest) = title.strip_prefix(&status) {
        let status_style = crate::style::status(node.status.as_str());
        let _ = write!(line, "{status_style}{status}{status_style:#}");
        line.push_str(rest);
    } else {
        // A malformed public scene is still displayed rather than panicking.
        line.push_str(title);
    }
    // Keep this parameter semantically tied to the scene used to derive
    // `node`; accepting it and never reading it would let a caller pair facts
    // from two scenes without any review signal.
    debug_assert!(scene.nodes.iter().any(|candidate| candidate.id == node.id));
    line
}

fn legend(glyphs: Glyphs, width: usize) -> String {
    let f = crate::style::HEADING;
    let b = crate::style::WARNING;
    let r = crate::style::OK;
    let c = crate::style::ID;
    let roles = format!(
        "legend: {f}[F]{f:#} focus; {b}[B]{b:#} direct blocker; {r}[R]{r:#} direct dependent; \
         {c}[C]{c:#} critical route; `>` an exact critical step enters this node; letters combine."
    );
    let markers = format!(
        "status: `[status]` is exact; `{}` dependency satisfied; `{}` dependency unsatisfied; \
         `{}`/`{}` critical edge.",
        glyphs.done,
        glyphs.node,
        glyphs.critical_connector(ConnectorKind::Horizontal),
        glyphs.critical_connector(ConnectorKind::Vertical),
    );
    format!(
        "{}\n{}\n",
        crate::fold::fold(&roles, width, 0),
        crate::fold::fold(&markers, width, 0)
    )
}

/// Paint a semantic scene with either the Unicode or ASCII glyph table.
///
/// The scene owns all geometry and identity. This function is deliberately a
/// byte sink: changing glyphs cannot change placement, hit targets or meaning.
#[must_use]
pub fn text_of_scene(scene: &GraphScene, glyphs: Glyphs) -> String {
    let mut out = String::new();
    for row in scene.rows() {
        let line = text_of_row(row, glyphs);
        let _ = writeln!(out, "{line}");
    }
    out
}

/// Paint one semantic scene row with the same glyph and trimming policy as
/// [`text_of_scene`].
///
/// A viewport can slice this result without copying the connector painter or
/// allowing interactive and CLI graph topology to drift.
#[must_use]
pub fn text_of_row(row: &crate::graph_scene::SceneRow, glyphs: Glyphs) -> String {
    let mut gutter = vec![' '; row.gutter_columns];
    for cell in &row.connectors {
        if let Some(slot) = gutter.get_mut(cell.id.0.column) {
            *slot = glyphs.connector(cell.kind);
        }
    }
    let mut line: String = gutter.into_iter().collect();
    line.push_str(&row.plain_text[row.gutter_columns..]);
    if row.trim_gutter {
        line.truncate(line.trim_end().len());
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GraphEdge, GraphNode};
    use aplan_core::Status;
    use aplan_core::graph::{CriticalPath, CriticalStep, CriticalTask};

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

    fn plain(text: &str) -> String {
        anstream::adapter::strip_str(text).to_string()
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

    /// A merge in a nonzero lane leaves an unrelated route to its left alone.
    /// The semantic regression is proved at the scene boundary; this exact
    /// rendering holds the shipped Unicode bytes to the same fact.
    #[test]
    fn a_merge_does_not_extend_left_of_its_destination_lane() {
        let out = drawn(&[
            ("A", &[]),
            ("B", &[]),
            ("C", &[]),
            ("D", &["B", "E"]),
            ("E", &[]),
            ("Y", &["C"]),
            ("Z", &["A"]),
        ]);
        assert_eq!(
            out,
            "○       A  task A\n\
             │ ○     B  task B\n\
             │ │ ○   C  task C\n\
             │ │ │ ○ E  task E\n\
             │ ├─┼─┘\n\
             │ ○ │   D  task D\n\
             │   ○   Y  task Y\n\
             ○       Z  task Z\n"
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

    #[test]
    fn the_presented_drawing_does_not_depend_on_input_order() {
        let forward = view(&[("A", &[]), ("B", &[]), ("C", &["A", "B"])]);
        let mut reversed = view(&[("C", &["A", "B"]), ("B", &[]), ("A", &[])]);
        reversed.nodes.reverse();
        reversed.edges.reverse();
        let route = CriticalRoute {
            path: CriticalPath {
                total_effort_days: 2.0,
                tasks: Vec::new(),
            },
            nodes: ["A", "C"].into_iter().map(str::to_string).collect(),
            steps: vec![CriticalStep {
                from: "A".to_string(),
                to: "C".to_string(),
            }],
        };
        let presentation = GraphPresentation {
            focus: Some("C"),
            critical: CriticalOverlay::Route(&route),
        };

        assert_eq!(
            text_of_presented_view(&forward, Glyphs::ASCII, 60, presentation).expect("fits"),
            text_of_presented_view(&reversed, Glyphs::ASCII, 60, presentation).expect("fits"),
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

    /// Proof for `graph.text_focus_and_critical_overlay`: role badges, exact
    /// statuses and heavy critical connectors all survive without colour,
    /// while colour adds no words or geometry.
    #[test]
    fn focused_graph_roles_statuses_and_critical_edges_are_non_colour_semantics() {
        let mut graph = view(&[
            ("A", &[]),
            ("B", &["A"]),
            ("C", &["A"]),
            ("D", &["B", "C"]),
            ("E", &[]),
        ]);
        for (id, status) in [
            ("A", Status::Done),
            ("B", Status::Blocked),
            ("C", Status::InProgress),
            ("D", Status::Todo),
            ("E", Status::Dropped),
        ] {
            let node = graph.nodes.iter_mut().find(|node| node.id == id).unwrap();
            node.status = status;
            node.done = status.satisfies_dependency();
        }
        let route = CriticalRoute {
            path: CriticalPath {
                total_effort_days: 3.0,
                tasks: ["A", "C", "D"]
                    .into_iter()
                    .map(|id| CriticalTask {
                        id: id.to_string(),
                        title: id.to_string(),
                        effort_days: 1.0,
                        status: graph
                            .nodes
                            .iter()
                            .find(|node| node.id == id)
                            .unwrap()
                            .status,
                    })
                    .collect(),
            },
            nodes: ["A", "C", "D"].into_iter().map(str::to_string).collect(),
            steps: [("A", "C"), ("C", "D")]
                .into_iter()
                .map(|(from, to)| CriticalStep {
                    from: from.to_string(),
                    to: to.to_string(),
                })
                .collect(),
        };
        let styled = text_of_presented_view(
            &graph,
            Glyphs::ASCII,
            72,
            GraphPresentation {
                focus: Some("D"),
                critical: CriticalOverlay::Route(&route),
            },
        )
        .expect("fits");
        let text = plain(&styled);
        let prose = text.split_whitespace().collect::<Vec<_>>().join(" ");

        for expected in [
            "[C]",
            "[B]",
            "[BC>]",
            "[FC>]",
            "[done]",
            "[blocked]",
            "[in_progress]",
            "[todo]",
            "[dropped]",
            "2/2 exact step(s) visible",
        ] {
            assert!(text.contains(expected), "missing {expected:?}:\n{text}");
        }
        assert!(prose.contains("letters combine"), "{text}");
        assert!(
            text.contains(['=', '!', '#']),
            "no heavy ASCII edge:\n{text}"
        );
        assert!(
            styled.contains('\u{1b}'),
            "the role hierarchy has no styling"
        );
        assert_eq!(
            plain(&styled),
            text,
            "stripping colour changed graph information"
        );
        for line in text.lines() {
            assert!(crate::fold::display_width(line) <= 72, "{line}");
        }
    }

    /// Proof for `graph.critical_visibility_counts_step_identities`: a route
    /// has one exact step even when a public scene preserves several edge
    /// records with those endpoints, and an omitted edge is not visible.
    #[test]
    fn critical_route_visibility_counts_step_identities_not_scene_records() {
        let route = CriticalRoute {
            path: CriticalPath {
                total_effort_days: 2.0,
                tasks: Vec::new(),
            },
            nodes: ["A", "B"].into_iter().map(str::to_string).collect(),
            steps: vec![CriticalStep {
                from: "A".to_string(),
                to: "B".to_string(),
            }],
        };
        let presentation = GraphPresentation {
            focus: None,
            critical: CriticalOverlay::Route(&route),
        };

        let mut repeated = view(&[("A", &[]), ("B", &["A"])]);
        repeated.edges.push(repeated.edges[0].clone());
        repeated.edges.push(crate::GraphEdge {
            from: "A".to_string(),
            to: "B".to_string(),
            kind: aplan_core::EdgeKind::Membership,
        });
        let rendered = plain(
            &text_of_presented_view(&repeated, Glyphs::ASCII, 72, presentation)
                .expect("repeated edges fit"),
        );
        assert!(rendered.contains("1/1 exact step(s) visible"), "{rendered}");
        assert!(!rendered.contains("2/1"), "{rendered}");
        assert!(!rendered.contains("3/1"), "{rendered}");

        let omitted = GraphView {
            nodes: repeated
                .nodes
                .into_iter()
                .filter(|node| node.id == "B")
                .collect(),
            edges: vec![crate::GraphEdge {
                from: "A".to_string(),
                to: "B".to_string(),
                kind: aplan_core::EdgeKind::Dependency,
            }],
            hidden_edges: 0,
            grouping: crate::Grouping::default(),
            scope: Vec::new(),
        };
        let rendered = plain(
            &text_of_presented_view(&omitted, Glyphs::ASCII, 72, presentation)
                .expect("omitted edge view fits"),
        );
        assert!(rendered.contains("0/1 exact step(s) visible"), "{rendered}");
        assert!(rendered.contains("[C]"), "{rendered}");
        assert!(!rendered.contains("[C>]"), "{rendered}");
    }

    #[test]
    fn presented_graph_wraps_wide_unicode_and_reports_an_unavailable_route() {
        let mut graph = view(&[("A", &[]), ("B", &["A"])]);
        graph.nodes[1].title = format!(
            "{} long title whose final words must remain visible",
            "\u{754c}".repeat(8)
        );
        let rendered = text_of_presented_view(
            &graph,
            Glyphs::UNICODE,
            42,
            GraphPresentation {
                focus: Some("A"),
                critical: CriticalOverlay::Unavailable,
            },
        )
        .expect("fits");
        let text = plain(&rendered);
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains("final words must remain visible"), "{text}");
        assert!(flat.contains("unavailable because"), "{text}");
        assert!(flat.contains("`━`/`┃` critical edge"), "{text}");
        for line in text.lines() {
            assert!(crate::fold::display_width(line) <= 42, "{line}");
        }
    }

    #[test]
    fn an_empty_critical_route_is_disclosed_instead_of_claiming_a_zero_step_route() {
        let graph = view(&[("A", &[]), ("B", &["A"])]);
        let route = CriticalRoute {
            path: CriticalPath {
                total_effort_days: 0.0,
                tasks: Vec::new(),
            },
            nodes: Vec::new(),
            steps: Vec::new(),
        };
        let rendered = plain(
            &text_of_presented_view(
                &graph,
                Glyphs::ASCII,
                72,
                GraphPresentation {
                    focus: None,
                    critical: CriticalOverlay::Route(&route),
                },
            )
            .expect("fits"),
        );

        assert!(rendered.contains("critical route: none"), "{rendered}");
        assert!(!rendered.contains("0/0"), "{rendered}");
    }

    #[test]
    fn presented_graph_width_accounts_for_role_badges() {
        let roots: Vec<String> = (0..20).map(|i| format!("R{i:02}")).collect();
        let refs: Vec<&str> = roots.iter().map(String::as_str).collect();
        let mut spec: Vec<(&str, &[&str])> = refs.iter().map(|root| (*root, &[][..])).collect();
        spec.push(("SINK", &refs));

        let error = text_of_presented_view(
            &view(&spec),
            Glyphs::ASCII,
            46,
            GraphPresentation::default(),
        )
        .expect_err("the role column plus gutter cannot fit");
        assert_eq!(error.needed, 46);
        assert_eq!(error.width, 46);
        let message = error.to_string();
        for action in ["--reach upstream:<id>", "--focus <id>", "--format mermaid"] {
            assert!(message.contains(action), "missing {action}: {message}");
        }
    }
}
