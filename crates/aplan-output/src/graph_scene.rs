//! Semantic terminal geometry for a dependency graph (`A1-455`).
//!
//! [`GraphView`] owns which nodes, edges and omissions mean something. This
//! module assigns those facts deterministic lane geometry once, before either a
//! stream painter or an interactive viewport sees them. Coordinates are
//! terminal cells, never byte or scalar offsets, and every painted connector
//! carries the semantic identities that pass through it.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use aplan_core::{EdgeKind, Status};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr;

use crate::{GraphEdge, GraphNode, GraphNoteKind, GraphView};

const ROOM_FOR_A_LABEL: usize = 12;

/// A location in the scene's terminal-cell coordinate system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScenePoint {
    pub row: usize,
    pub column: usize,
}

/// A half-open rectangle in terminal cells.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneRect {
    pub row: usize,
    pub column: usize,
    pub height: usize,
    pub width: usize,
}

impl SceneRect {
    /// Whether this rectangle contains `point`.
    #[must_use]
    pub fn contains(self, point: ScenePoint) -> bool {
        point.row >= self.row
            && point.row < self.row.saturating_add(self.height)
            && point.column >= self.column
            && point.column < self.column.saturating_add(self.width)
    }
}

/// The total rows and columns occupied by a scene.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneBounds {
    pub rows: usize,
    pub columns: usize,
}

/// A horizontal hit region on one scene row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneSpan {
    pub row: usize,
    pub column: usize,
    pub width: usize,
}

impl SceneSpan {
    fn contains(self, point: ScenePoint) -> bool {
        self.row == point.row
            && point.column >= self.column
            && point.column < self.column.saturating_add(self.width)
    }
}

/// Stable identity for one edge, including repeated equal endpoints.
///
/// `ordinal` is assigned after sorting by endpoints and kind, so shuffling a
/// `GraphView` cannot move it. Equal duplicate edges are indistinguishable as
/// facts; their deterministic ordinals make the multiplicity addressable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SceneEdgeId {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub ordinal: usize,
}

/// Why an admitted edge has no connector cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneEdgeOmission {
    /// At least one endpoint is absent from the admitted node set.
    MissingEndpoint,
    /// The edge points to an earlier row in a cyclic invalid graph.
    CycleBackEdge,
}

/// One edge and the connector cells that represent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneEdge {
    pub id: SceneEdgeId,
    pub cells: Vec<ConnectorCellId>,
    pub omission: Option<SceneEdgeOmission>,
}

/// Stable identity for a connector cell. Its coordinate is its identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnectorCellId(pub ScenePoint);

/// Glyph-independent topology at one connector cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorKind {
    Node { done: bool },
    Vertical,
    Horizontal,
    Tee,
    Corner,
    Join,
    Cross,
}

/// One semantic cell in the lane drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorCell {
    pub id: ConnectorCellId,
    pub kind: ConnectorKind,
    /// The node drawn at this cell, when it is a node marker.
    pub node: Option<String>,
    /// Every edge whose route owns or joins this cell.
    pub edges: Vec<SceneEdgeId>,
}

/// The semantic owner of a row's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneTextOwner {
    Node(String),
    Note(GraphNoteKind),
}

/// One row, with glyph-free text and semantic connector cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneRow {
    pub index: usize,
    /// Spaces occupy the connector gutter. A painter replaces only those cells.
    pub plain_text: String,
    pub gutter_columns: usize,
    /// Connector-only rows omit unused trailing gutter cells in stream output.
    pub trim_gutter: bool,
    pub text_owner: Option<SceneTextOwner>,
    pub text_span: Option<SceneSpan>,
    pub connectors: Vec<ConnectorCell>,
}

/// One placed graph node and all of its text hit regions.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneNode {
    /// Raw semantic identity, never rewritten for display.
    pub id: String,
    pub title: String,
    pub status: Status,
    pub done: bool,
    /// Safe one-line rendering of `id`.
    pub display_id: String,
    pub marker: ScenePoint,
    pub bounds: SceneRect,
    pub text_spans: Vec<SceneSpan>,
}

/// One typed note placed below the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneNote {
    pub kind: GraphNoteKind,
    /// The semantic prose supplied by `GraphView`.
    pub text: String,
    /// The safe text handed to the terminal painter.
    pub display_text: String,
    pub bounds: SceneRect,
    pub text_spans: Vec<SceneSpan>,
}

/// Semantic identities under one terminal cell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SceneHit {
    pub nodes: Vec<String>,
    pub edges: Vec<SceneEdgeId>,
    pub notes: Vec<GraphNoteKind>,
}

/// A malformed input fact the scene could preserve only by canonicalizing it.
///
/// [`GraphView::of`](crate::GraphView::of) never produces these conditions,
/// but `GraphView` is a public data format and can be deserialized or built by
/// another consumer. Reporting the repair here prevents a renderer from
/// quietly presenting incomplete data as a complete graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneIssue {
    /// Several node records used one semantic identity. The lexicographically
    /// first record is drawn because edges cannot distinguish the others.
    DuplicateNodeId { id: String, occurrences: usize },
}

/// A deterministic lane layout independent of any terminal backend or glyph set.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphScene {
    pub bounds: SceneBounds,
    pub nodes: Vec<SceneNode>,
    pub edges: Vec<SceneEdge>,
    pub notes: Vec<SceneNote>,
    /// Invalid source facts that were canonicalized to produce this scene.
    pub issues: Vec<SceneIssue>,
    rows: Vec<SceneRow>,
}

impl GraphScene {
    /// The peak number of connector lanes required by this semantic view.
    ///
    /// This runs the same canonical ordering and cycle omission pass as
    /// [`Self::layout`], but does not expand labels or notes. Application
    /// projections can therefore enforce a lane budget without guessing from
    /// edge count or parsing a rendered scene.
    #[must_use]
    pub fn required_lanes(view: &GraphView) -> usize {
        let (_, _, raw, _) = canonical_geometry(view);
        raw.iter()
            .map(|row| row.gutter_columns)
            .max()
            .unwrap_or(0)
            .div_ceil(2)
    }

    /// Lay out a graph for a terminal-cell width.
    ///
    /// # Errors
    ///
    /// Returns [`crate::lanes::TooWide`] when the connector gutter leaves no
    /// useful label room, preserving the CLI's existing refusal boundary.
    pub fn layout(view: &GraphView, width: usize) -> Result<Self, crate::lanes::TooWide> {
        let width = width.min(crate::MAX_WIDTH);
        let (nodes, edges, raw, issues) = canonical_geometry(view);
        let gutter = raw.iter().map(|row| row.gutter_columns).max().unwrap_or(0);
        if gutter + ROOM_FOR_A_LABEL > width {
            return Err(crate::lanes::TooWide {
                lanes: gutter.div_ceil(2),
                needed: gutter,
                width,
            });
        }

        Ok(expand(view, width, gutter, &nodes, edges, &raw, issues))
    }

    /// Every row in document order.
    #[must_use]
    pub fn rows(&self) -> &[SceneRow] {
        &self.rows
    }

    /// The rows intersecting a half-open row range, clamped to the scene.
    #[must_use]
    pub fn rows_in(&self, rows: Range<usize>) -> &[SceneRow] {
        let start = rows.start.min(self.rows.len());
        let end = rows.end.max(start).min(self.rows.len());
        &self.rows[start..end]
    }

    /// Connector cells intersecting a viewport, without visiting off-screen rows.
    pub fn connector_cells_in(&self, viewport: SceneRect) -> impl Iterator<Item = &ConnectorCell> {
        let row_end = viewport.row.saturating_add(viewport.height);
        self.rows_in(viewport.row..row_end)
            .iter()
            .flat_map(|row| row.connectors.iter())
            .filter(move |cell| viewport.contains(cell.id.0))
    }

    /// Semantic node, edge and note identities under one terminal cell.
    #[must_use]
    pub fn hit_test(&self, point: ScenePoint) -> SceneHit {
        let mut hit = SceneHit::default();
        let Some(row) = self.rows.get(point.row) else {
            return hit;
        };
        if row.text_span.is_some_and(|span| span.contains(point)) {
            match &row.text_owner {
                Some(SceneTextOwner::Node(id)) => hit.nodes.push(id.clone()),
                Some(SceneTextOwner::Note(kind)) => hit.notes.push(*kind),
                None => {}
            }
        }
        if let Some(cell) = row.connectors.iter().find(|cell| cell.id.0 == point) {
            if let Some(node) = &cell.node {
                hit.nodes.push(node.clone());
            }
            hit.edges.extend(cell.edges.iter().cloned());
        }
        hit.nodes.sort();
        hit.nodes.dedup();
        hit.edges.sort();
        hit.edges.dedup();
        hit.notes.sort();
        hit.notes.dedup();
        hit
    }
}

fn canonical_geometry(
    view: &GraphView,
) -> (
    Vec<&GraphNode>,
    Vec<SceneEdge>,
    Vec<RawRow>,
    Vec<SceneIssue>,
) {
    let (nodes, node_index, issues) = sorted_nodes(view);
    let mut edges = sorted_edges(view, &node_index);
    let candidates: Vec<usize> = edges
        .iter()
        .enumerate()
        .filter_map(|(index, edge)| edge.omission.is_none().then_some(index))
        .collect();
    let order = topological(&nodes, &edges, &candidates);
    let positions: BTreeMap<&str, usize> = order
        .iter()
        .enumerate()
        .map(|(position, id)| (id.as_str(), position))
        .collect();
    let mut drawable = Vec::new();
    for index in candidates {
        let edge = &mut edges[index];
        let from = positions[edge.id.from.as_str()];
        let to = positions[edge.id.to.as_str()];
        if from >= to {
            edge.omission = Some(SceneEdgeOmission::CycleBackEdge);
        } else {
            drawable.push(index);
        }
    }
    let raw = lay_out(&nodes, &node_index, &edges, &drawable, &order);
    (nodes, edges, raw, issues)
}

#[derive(Debug)]
struct RawCell {
    kind: ConnectorKind,
    node: Option<usize>,
    edges: BTreeSet<usize>,
}

#[derive(Debug)]
struct RawRow {
    gutter_columns: usize,
    cells: BTreeMap<usize, RawCell>,
    node: Option<usize>,
}

fn sorted_nodes(view: &GraphView) -> (Vec<&GraphNode>, BTreeMap<&str, usize>, Vec<SceneIssue>) {
    let mut nodes: Vec<&GraphNode> = view.nodes.iter().collect();
    nodes.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.done.cmp(&right.done))
            .then_with(|| left.status.as_str().cmp(right.status.as_str()))
    });
    let mut issues = Vec::new();
    for duplicate in nodes.chunk_by(|left, right| left.id == right.id) {
        if duplicate.len() > 1 {
            issues.push(SceneIssue::DuplicateNodeId {
                id: duplicate[0].id.clone(),
                occurrences: duplicate.len(),
            });
        }
    }
    nodes.dedup_by(|left, right| left.id == right.id);
    let index = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect();
    (nodes, index, issues)
}

fn sorted_edges(view: &GraphView, nodes: &BTreeMap<&str, usize>) -> Vec<SceneEdge> {
    let mut source: Vec<&GraphEdge> = view.edges.iter().collect();
    source.sort_by(|left, right| {
        (&left.from, &left.to, left.kind).cmp(&(&right.from, &right.to, right.kind))
    });
    let mut previous: Option<(&str, &str, EdgeKind)> = None;
    let mut ordinal = 0usize;
    source
        .into_iter()
        .map(|edge| {
            let key = (edge.from.as_str(), edge.to.as_str(), edge.kind);
            if previous == Some(key) {
                ordinal += 1;
            } else {
                previous = Some(key);
                ordinal = 0;
            }
            SceneEdge {
                id: SceneEdgeId {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                    kind: edge.kind,
                    ordinal,
                },
                cells: Vec::new(),
                omission: (!nodes.contains_key(edge.from.as_str())
                    || !nodes.contains_key(edge.to.as_str()))
                .then_some(SceneEdgeOmission::MissingEndpoint),
            }
        })
        .collect()
}

fn topological(nodes: &[&GraphNode], edges: &[SceneEdge], candidates: &[usize]) -> Vec<String> {
    let ids: BTreeSet<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
    let mut successors: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut indegree: BTreeMap<&str, usize> = ids.iter().map(|id| (*id, 0)).collect();
    for index in candidates {
        let edge = &edges[*index].id;
        if successors
            .entry(edge.from.as_str())
            .or_default()
            .insert(edge.to.as_str())
        {
            *indegree
                .get_mut(edge.to.as_str())
                .expect("a known endpoint") += 1;
        }
    }
    let mut ready: BTreeSet<&str> = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect();
    let mut order = Vec::with_capacity(ids.len());
    while let Some(next) = ready.pop_first() {
        order.push(next.to_string());
        for successor in successors.get(next).into_iter().flatten() {
            let degree = indegree.get_mut(*successor).expect("a known successor");
            *degree -= 1;
            if *degree == 0 {
                ready.insert(successor);
            }
        }
    }
    let placed: BTreeSet<String> = order.iter().cloned().collect();
    order.extend(
        ids.into_iter()
            .filter(|id| !placed.contains(*id))
            .map(str::to_string),
    );
    order
}

fn lay_out(
    nodes: &[&GraphNode],
    node_index: &BTreeMap<&str, usize>,
    edges: &[SceneEdge],
    drawable: &[usize],
    order: &[String],
) -> Vec<RawRow> {
    let mut successors: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for edge in drawable {
        successors
            .entry(edges[*edge].id.from.as_str())
            .or_default()
            .push(*edge);
    }
    let mut lanes: Vec<Option<usize>> = Vec::new();
    let mut rows = Vec::new();
    for id in order {
        let hits: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter_map(|(lane, edge)| {
                edge.and_then(|edge| (edges[edge].id.to == *id).then_some(lane))
            })
            .collect();
        let lane = hits
            .first()
            .copied()
            .unwrap_or_else(|| free_lane(&mut lanes));
        if hits.len() > 1 {
            rows.push(merge_row(&lanes, lane, &hits));
        }
        let incoming: Vec<usize> = hits.iter().filter_map(|lane| lanes[*lane]).collect();
        for hit in &hits {
            lanes[*hit] = None;
        }
        let outgoing = successors.get(id.as_str()).cloned().unwrap_or_default();
        let node = node_index[id.as_str()];
        rows.push(node_row(
            &lanes,
            lane,
            node,
            nodes[node].done,
            incoming.iter().chain(&outgoing).copied(),
        ));
        for (offset, edge) in outgoing.into_iter().enumerate() {
            let destination = if offset == 0 {
                lane
            } else {
                free_lane(&mut lanes)
            };
            lanes[destination] = Some(edge);
        }
        trim_lanes(&mut lanes);
    }
    rows
}

fn free_lane(lanes: &mut Vec<Option<usize>>) -> usize {
    lanes.iter().position(Option::is_none).unwrap_or_else(|| {
        lanes.push(None);
        lanes.len() - 1
    })
}

fn trim_lanes(lanes: &mut Vec<Option<usize>>) {
    while lanes.last().is_some_and(Option::is_none) {
        lanes.pop();
    }
}

fn node_row(
    lanes: &[Option<usize>],
    lane: usize,
    node: usize,
    done: bool,
    node_edges: impl Iterator<Item = usize>,
) -> RawRow {
    let node_edges: BTreeSet<usize> = node_edges.collect();
    let mut cells = BTreeMap::new();
    for (index, edge) in lanes.iter().enumerate() {
        if index == lane {
            cells.insert(
                index * 2,
                RawCell {
                    kind: ConnectorKind::Node { done },
                    node: Some(node),
                    edges: node_edges.clone(),
                },
            );
        } else if let Some(edge) = edge {
            cells.insert(
                index * 2,
                RawCell {
                    kind: ConnectorKind::Vertical,
                    node: None,
                    edges: BTreeSet::from([*edge]),
                },
            );
        }
    }
    RawRow {
        gutter_columns: lanes.len() * 2,
        cells,
        node: Some(node),
    }
}

fn merge_row(lanes: &[Option<usize>], lane: usize, hits: &[usize]) -> RawRow {
    let last = hits.last().copied().unwrap_or(lane);
    let joining: Vec<(usize, usize)> = hits
        .iter()
        .filter_map(|hit| lanes[*hit].map(|edge| (*hit, edge)))
        .collect();
    let mut cells = BTreeMap::new();
    for (index, active) in lanes.iter().enumerate() {
        let (kind, horizontal) = if index == lane {
            (Some(ConnectorKind::Tee), true)
        } else if hits.contains(&index) {
            (
                Some(if index < last {
                    ConnectorKind::Join
                } else {
                    ConnectorKind::Corner
                }),
                index < last,
            )
        } else if index > lane && index < last {
            (
                Some(if active.is_some() {
                    ConnectorKind::Cross
                } else {
                    ConnectorKind::Horizontal
                }),
                true,
            )
        } else if active.is_some() {
            (Some(ConnectorKind::Vertical), false)
        } else {
            (None, false)
        };
        if let Some(kind) = kind {
            let mut owners: BTreeSet<usize> = if index == lane || hits.contains(&index) {
                joining
                    .iter()
                    .filter_map(|(hit, edge)| (*hit >= index).then_some(*edge))
                    .collect()
            } else if index > lane && index < last {
                joining
                    .iter()
                    .filter_map(|(hit, edge)| (*hit > index).then_some(*edge))
                    .collect()
            } else {
                BTreeSet::new()
            };
            if let Some(active) = active {
                owners.insert(*active);
            }
            cells.insert(
                index * 2,
                RawCell {
                    kind,
                    node: None,
                    edges: owners,
                },
            );
        }
        if horizontal {
            let owners = joining
                .iter()
                .filter_map(|(hit, edge)| (*hit > index).then_some(*edge))
                .collect();
            cells.insert(
                index * 2 + 1,
                RawCell {
                    kind: ConnectorKind::Horizontal,
                    node: None,
                    edges: owners,
                },
            );
        }
    }
    RawRow {
        gutter_columns: lanes.len() * 2,
        cells,
        node: None,
    }
}

fn expand(
    view: &GraphView,
    width: usize,
    gutter: usize,
    nodes: &[&GraphNode],
    mut edges: Vec<SceneEdge>,
    raw: &[RawRow],
    issues: Vec<SceneIssue>,
) -> GraphScene {
    let displays = display_labels(nodes);
    let id_width = displays
        .iter()
        .map(|(id, _)| display_columns(id))
        .max()
        .unwrap_or(0);
    let mut scene_nodes = placed_nodes(nodes, &displays);
    let edge_ids: Vec<SceneEdgeId> = edges.iter().map(|edge| edge.id.clone()).collect();
    let mut rows = Vec::new();
    {
        let mut expander = RowExpander {
            width,
            gutter,
            id_width,
            displays: &displays,
            nodes: &mut scene_nodes,
            edge_ids: &edge_ids,
            edges: &mut edges,
            rows: &mut rows,
        };
        for raw_row in raw {
            expander.push(raw_row);
        }
    }
    let annotations = append_notes(view, width, &mut rows);
    for node in &mut scene_nodes {
        let mut spans = node.text_spans.clone();
        spans.push(SceneSpan {
            row: node.marker.row,
            column: node.marker.column,
            width: 1,
        });
        node.bounds = spans_bounds(&spans, node.marker.row);
    }
    let columns = rows
        .iter()
        .map(|row| {
            display_columns(&row.plain_text).max(
                row.connectors
                    .last()
                    .map_or(0, |cell| cell.id.0.column.saturating_add(1)),
            )
        })
        .max()
        .unwrap_or(0);
    GraphScene {
        bounds: SceneBounds {
            rows: rows.len(),
            columns,
        },
        nodes: scene_nodes,
        edges,
        notes: annotations,
        issues,
        rows,
    }
}

fn display_labels(nodes: &[&GraphNode]) -> Vec<(String, String)> {
    nodes
        .iter()
        .map(|node| (visible_id(&node.id), visible_one_line(&node.title)))
        .collect()
}

fn placed_nodes(nodes: &[&GraphNode], displays: &[(String, String)]) -> Vec<SceneNode> {
    nodes
        .iter()
        .zip(displays)
        .map(|(node, (display_id, _))| SceneNode {
            id: node.id.clone(),
            title: node.title.clone(),
            status: node.status,
            done: node.done,
            display_id: display_id.clone(),
            marker: ScenePoint { row: 0, column: 0 },
            bounds: SceneRect::default(),
            text_spans: Vec::new(),
        })
        .collect()
}

struct RowExpander<'a> {
    width: usize,
    gutter: usize,
    id_width: usize,
    displays: &'a [(String, String)],
    nodes: &'a mut [SceneNode],
    edge_ids: &'a [SceneEdgeId],
    edges: &'a mut [SceneEdge],
    rows: &'a mut Vec<SceneRow>,
}

impl RowExpander<'_> {
    fn push(&mut self, raw: &RawRow) {
        let first_row = self.rows.len();
        let connectors = connector_cells(raw, first_row, self.nodes, self.edge_ids, self.edges);
        let Some(node_index) = raw.node else {
            self.rows.push(SceneRow {
                index: first_row,
                plain_text: " ".repeat(raw.gutter_columns),
                gutter_columns: raw.gutter_columns,
                trim_gutter: true,
                text_owner: None,
                text_span: None,
                connectors,
            });
            return;
        };
        let (display_id, title) = &self.displays[node_index];
        let padding = self.id_width.saturating_sub(display_columns(display_id));
        let prefix = format!(
            "{}{display_id}{}  ",
            " ".repeat(self.gutter),
            " ".repeat(padding)
        );
        let title = title.clone();
        self.push_node_lines(node_index, &prefix, &title, connectors);
        let marker_column = raw
            .cells
            .iter()
            .find_map(|(column, cell)| (cell.node == Some(node_index)).then_some(*column))
            .expect("a node row has its marker");
        self.nodes[node_index].marker = ScenePoint {
            row: first_row,
            column: marker_column,
        };
    }

    fn push_node_lines(
        &mut self,
        node_index: usize,
        prefix: &str,
        title: &str,
        connectors: Vec<ConnectorCell>,
    ) {
        let title_column = display_columns(prefix);
        let node_id = self.nodes[node_index].id.clone();
        let mut first_connectors = Some(connectors);
        for (offset, line) in fold_after_lines(prefix, title, self.width)
            .into_iter()
            .enumerate()
        {
            let row = self.rows.len();
            let column = if offset == 0 {
                self.gutter
            } else {
                title_column
            };
            let span = SceneSpan {
                row,
                column,
                width: display_columns(&line).saturating_sub(column),
            };
            if span.width > 0 {
                self.nodes[node_index].text_spans.push(span);
            }
            self.rows.push(SceneRow {
                index: row,
                plain_text: line,
                gutter_columns: if offset == 0 { self.gutter } else { 0 },
                trim_gutter: false,
                text_owner: Some(SceneTextOwner::Node(node_id.clone())),
                text_span: (span.width > 0).then_some(span),
                connectors: if offset == 0 {
                    first_connectors.take().expect("the first line owns them")
                } else {
                    Vec::new()
                },
            });
        }
    }
}

fn append_notes(view: &GraphView, width: usize, rows: &mut Vec<SceneRow>) -> Vec<SceneNote> {
    let mut scene_notes = Vec::new();
    for note in view.notes() {
        push_plain_row(rows, String::new(), None, None);
        let display_text = visible_one_line(&note.text);
        let start = rows.len();
        let mut spans = Vec::new();
        for line in fold_lines(&display_text, width) {
            let row = rows.len();
            let span = SceneSpan {
                row,
                column: 0,
                width: display_columns(&line),
            };
            if span.width > 0 {
                spans.push(span);
            }
            push_plain_row(
                rows,
                line,
                Some(SceneTextOwner::Note(note.kind)),
                (span.width > 0).then_some(span),
            );
        }
        scene_notes.push(SceneNote {
            kind: note.kind,
            text: note.text,
            display_text,
            bounds: spans_bounds(&spans, start),
            text_spans: spans,
        });
    }
    scene_notes
}

fn connector_cells(
    raw: &RawRow,
    row: usize,
    nodes: &[SceneNode],
    edge_ids: &[SceneEdgeId],
    edges: &mut [SceneEdge],
) -> Vec<ConnectorCell> {
    raw.cells
        .iter()
        .map(|(column, cell)| {
            let id = ConnectorCellId(ScenePoint {
                row,
                column: *column,
            });
            let owners: Vec<SceneEdgeId> = cell
                .edges
                .iter()
                .map(|edge| {
                    edges[*edge].cells.push(id);
                    edge_ids[*edge].clone()
                })
                .collect();
            ConnectorCell {
                id,
                kind: cell.kind,
                node: cell.node.map(|node| nodes[node].id.clone()),
                edges: owners,
            }
        })
        .collect()
}

fn push_plain_row(
    rows: &mut Vec<SceneRow>,
    plain_text: String,
    owner: Option<SceneTextOwner>,
    span: Option<SceneSpan>,
) {
    rows.push(SceneRow {
        index: rows.len(),
        plain_text,
        gutter_columns: 0,
        trim_gutter: false,
        text_owner: owner,
        text_span: span,
        connectors: Vec::new(),
    });
}

fn spans_bounds(spans: &[SceneSpan], fallback_row: usize) -> SceneRect {
    let Some(first) = spans.first() else {
        return SceneRect {
            row: fallback_row,
            ..SceneRect::default()
        };
    };
    let top = spans.iter().map(|span| span.row).min().unwrap_or(first.row);
    let bottom = spans
        .iter()
        .map(|span| span.row.saturating_add(1))
        .max()
        .unwrap_or(top);
    let left = spans
        .iter()
        .map(|span| span.column)
        .min()
        .unwrap_or(first.column);
    let right = spans
        .iter()
        .map(|span| span.column.saturating_add(span.width))
        .max()
        .unwrap_or(left);
    SceneRect {
        row: top,
        column: left,
        height: bottom.saturating_sub(top),
        width: right.saturating_sub(left),
    }
}

fn visible_one_line(text: &str) -> String {
    crate::sanitize::visible(text)
        .chars()
        .map(|character| {
            if character.is_whitespace() && character != ' ' {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn visible_id(text: &str) -> String {
    let one_line = visible_one_line(text);
    if one_line.is_empty() || one_line.chars().all(|character| character == ' ') {
        return crate::sanitize::MARKER.to_string();
    }
    one_line
        .graphemes(true)
        .map(|grapheme| {
            if UnicodeWidthStr::width(grapheme) == 0 {
                crate::sanitize::MARKER.to_string()
            } else {
                grapheme.to_string()
            }
        })
        .collect()
}

fn fold_after_lines(prefix: &str, trailing: &str, width: usize) -> Vec<String> {
    let prefix_width = display_columns(prefix);
    let room = width.saturating_sub(prefix_width);
    if room == 0 || display_columns(trailing) <= room {
        return vec![format!("{prefix}{trailing}")];
    }
    fold_words(trailing, room)
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                format!("{prefix}{line}")
            } else {
                format!("{}{line}", " ".repeat(prefix_width))
            }
        })
        .collect()
}

fn fold_lines(text: &str, width: usize) -> Vec<String> {
    if display_columns(text) <= width {
        vec![text.to_string()]
    } else {
        fold_words(text, width)
    }
}

fn fold_words(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut column = 0usize;
    for word in text.split_whitespace() {
        let word_width = display_columns(word);
        let current = lines.last_mut().expect("one output line");
        if current.is_empty() {
            current.push_str(word);
            column = word_width;
        } else if column.saturating_add(1).saturating_add(word_width) > width {
            lines.push(word.to_string());
            column = word_width;
        } else {
            current.push(' ');
            current.push_str(word);
            column += 1 + word_width;
        }
    }
    lines
}

/// Terminal columns occupied by safe graph text, measured by grapheme cluster.
#[must_use]
pub fn display_columns(text: &str) -> usize {
    text.graphemes(true).map(UnicodeWidthStr::width).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GraphEdge, GraphNode, Grouping};

    fn node(id: &str) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            title: format!("task {id}"),
            status: Status::Todo,
            done: false,
        }
    }

    fn edge(from: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            kind: EdgeKind::Dependency,
        }
    }

    fn view(ids: &[&str], pairs: &[(&str, &str)]) -> GraphView {
        GraphView {
            nodes: ids.iter().map(|id| node(id)).collect(),
            edges: pairs.iter().map(|(from, to)| edge(from, to)).collect(),
            hidden_edges: 0,
            grouping: Grouping::default(),
            scope: Vec::new(),
        }
    }

    fn geometry(scene: &GraphScene) -> (SceneBounds, Vec<ScenePoint>, Vec<SceneEdge>) {
        (
            scene.bounds,
            scene.nodes.iter().map(|node| node.marker).collect(),
            scene.edges.clone(),
        )
    }

    fn assert_scene_invariants(scene: &GraphScene) {
        let edge_by_id: BTreeMap<&SceneEdgeId, &SceneEdge> =
            scene.edges.iter().map(|edge| (&edge.id, edge)).collect();
        assert_eq!(
            edge_by_id.len(),
            scene.edges.len(),
            "duplicate edge identity"
        );

        let mut cell_ids = BTreeSet::new();
        for (row_index, row) in scene.rows().iter().enumerate() {
            assert_eq!(
                row.index, row_index,
                "row index disagrees with its position"
            );
            assert!(display_columns(&row.plain_text) <= scene.bounds.columns);
            if let Some(span) = row.text_span {
                assert_eq!(span.row, row_index);
                assert!(span.column.saturating_add(span.width) <= scene.bounds.columns);
                assert!(row.text_owner.is_some(), "text span has no semantic owner");
            }
            for cell in &row.connectors {
                assert!(cell_ids.insert(cell.id), "duplicate connector id: {cell:?}");
                assert_eq!(cell.id.0.row, row_index);
                assert!(cell.id.0.column < scene.bounds.columns);
                let mut owners = BTreeSet::new();
                for edge_id in &cell.edges {
                    assert!(owners.insert(edge_id), "duplicate cell owner: {cell:?}");
                    let edge = edge_by_id
                        .get(edge_id)
                        .unwrap_or_else(|| panic!("unknown edge owner: {edge_id:?}"));
                    assert_eq!(
                        edge.cells.iter().filter(|id| **id == cell.id).count(),
                        1,
                        "edge-to-cell ownership is not reciprocal: {edge:?} / {cell:?}"
                    );
                }
                let hit = scene.hit_test(cell.id.0);
                assert_eq!(hit.edges, cell.edges, "hit testing changed cell ownership");
                assert_eq!(hit.nodes, cell.node.iter().cloned().collect::<Vec<_>>());
            }
        }
        assert_eq!(scene.bounds.rows, scene.rows().len());

        for edge in &scene.edges {
            let unique: BTreeSet<ConnectorCellId> = edge.cells.iter().copied().collect();
            assert_eq!(
                unique.len(),
                edge.cells.len(),
                "edge repeats a cell: {edge:?}"
            );
            assert_eq!(edge.omission.is_some(), edge.cells.is_empty(), "{edge:?}");
            for cell_id in &edge.cells {
                let cell = scene.rows()[cell_id.0.row]
                    .connectors
                    .iter()
                    .find(|cell| cell.id == *cell_id)
                    .expect("edge names an existing cell");
                assert!(cell.edges.contains(&edge.id));
            }
        }

        let node_ids: BTreeSet<&str> = scene.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(
            node_ids.len(),
            scene.nodes.len(),
            "duplicate placed node id"
        );
        for node in &scene.nodes {
            assert!(
                node.bounds.contains(node.marker),
                "marker outside node bounds"
            );
            assert!(scene.hit_test(node.marker).nodes.contains(&node.id));
            for span in &node.text_spans {
                assert!(span.width > 0);
                let first = ScenePoint {
                    row: span.row,
                    column: span.column,
                };
                let last = ScenePoint {
                    row: span.row,
                    column: span.column.saturating_add(span.width - 1),
                };
                assert!(node.bounds.contains(first) && node.bounds.contains(last));
                assert!(scene.hit_test(first).nodes.contains(&node.id));
                assert!(scene.hit_test(last).nodes.contains(&node.id));
            }
        }

        for note in &scene.notes {
            for span in &note.text_spans {
                let first = ScenePoint {
                    row: span.row,
                    column: span.column,
                };
                let last = ScenePoint {
                    row: span.row,
                    column: span.column.saturating_add(span.width - 1),
                };
                assert!(note.bounds.contains(first) && note.bounds.contains(last));
                assert!(scene.hit_test(first).notes.contains(&note.kind));
                assert!(scene.hit_test(last).notes.contains(&note.kind));
            }
        }
    }

    #[test]
    fn scene_carries_every_admitted_identity_through_adversarial_shapes() {
        let ids = ["A", "B", "C", "D", "E", "F", "G", "M", "X", "Y"];
        let mut graph = view(
            &ids,
            &[
                ("A", "C"),
                ("A", "C"),
                ("B", "C"),
                ("C", "D"),
                ("C", "E"),
                ("B", "F"),
                ("D", "F"),
                ("X", "Y"),
                ("Y", "X"),
                ("G", "G"),
                ("missing", "A"),
            ],
        );
        graph.edges.push(GraphEdge {
            from: "G".into(),
            to: "M".into(),
            kind: EdgeKind::Membership,
        });
        let scene = GraphScene::layout(&graph, 95).expect("the graph fits");

        assert_scene_invariants(&scene);

        assert_eq!(scene.nodes.len(), ids.len(), "a disconnected node vanished");
        assert_eq!(scene.edges.len(), graph.edges.len());
        assert!(
            scene
                .edges
                .iter()
                .all(|edge| !edge.cells.is_empty() || edge.omission.is_some()),
            "an edge is neither represented nor reported: {:?}",
            scene.edges
        );
        assert_eq!(
            scene
                .edges
                .iter()
                .filter(|edge| edge.id.from == "A" && edge.id.to == "C")
                .map(|edge| edge.id.ordinal)
                .collect::<Vec<_>>(),
            vec![0, 1],
            "equal endpoints lost their stable multiplicity"
        );
        assert!(
            scene
                .edges
                .iter()
                .any(|edge| { edge.id.kind == EdgeKind::Membership && edge.omission.is_none() })
        );
        assert!(scene.edges.iter().any(|edge| {
            edge.id.from == "missing" && edge.omission == Some(SceneEdgeOmission::MissingEndpoint)
        }));
        assert_eq!(
            scene
                .edges
                .iter()
                .filter(|edge| edge.omission == Some(SceneEdgeOmission::CycleBackEdge))
                .count(),
            2,
            "one direction of the cycle and the self-edge should be explicitly omitted"
        );

        for node in &scene.nodes {
            let hit = scene.hit_test(node.marker);
            assert!(hit.nodes.contains(&node.id), "no hit for {}", node.id);
        }
        for edge in scene.edges.iter().filter(|edge| edge.omission.is_none()) {
            let hit = scene.hit_test(edge.cells[0].0);
            assert!(hit.edges.contains(&edge.id), "no hit for {:?}", edge.id);
        }
    }

    #[test]
    fn placement_is_stable_under_input_order_and_status_only_changes() {
        let original = view(
            &["A", "B", "C", "D", "E"],
            &[("A", "C"), ("B", "C"), ("C", "D"), ("C", "E")],
        );
        let expected = GraphScene::layout(&original, 60).expect("fits");
        let mut shuffled = original.clone();
        shuffled.nodes.reverse();
        shuffled.edges.rotate_left(2);
        let actual = GraphScene::layout(&shuffled, 60).expect("fits");
        assert_eq!(geometry(&expected), geometry(&actual));

        shuffled.nodes[0].status = Status::Done;
        shuffled.nodes[0].done = true;
        let changed = GraphScene::layout(&shuffled, 60).expect("fits");
        assert_eq!(
            expected
                .nodes
                .iter()
                .map(|node| node.marker)
                .collect::<Vec<_>>(),
            changed
                .nodes
                .iter()
                .map(|node| node.marker)
                .collect::<Vec<_>>()
        );
        assert_eq!(expected.edges, changed.edges);
    }

    #[test]
    fn viewport_iteration_and_hit_testing_share_scene_coordinates() {
        let graph = view(&["A", "B", "C", "D"], &[("A", "C"), ("B", "C"), ("C", "D")]);
        let scene = GraphScene::layout(&graph, 60).expect("fits");
        let viewport = SceneRect {
            row: 1,
            column: 0,
            height: 2,
            width: 4,
        };
        assert_eq!(scene.rows_in(1..3).len(), 2);
        let cells: Vec<&ConnectorCell> = scene.connector_cells_in(viewport).collect();
        assert!(!cells.is_empty(), "the viewport culled everything");
        assert!(cells.iter().all(|cell| viewport.contains(cell.id.0)));
        for cell in cells {
            let hit = scene.hit_test(cell.id.0);
            assert!(
                !hit.nodes.is_empty() || !hit.edges.is_empty(),
                "a visible semantic cell has no hit identity: {cell:?}"
            );
        }
        assert_eq!(
            scene.hit_test(ScenePoint {
                row: scene.bounds.rows,
                column: 0,
            }),
            SceneHit::default()
        );

        for (start, end) in [(usize::MAX, 0), (3, 1)] {
            assert!(scene.rows_in(start..end).is_empty());
        }
        for empty in [
            SceneRect {
                row: 0,
                column: 0,
                height: 0,
                width: usize::MAX,
            },
            SceneRect {
                row: 0,
                column: 0,
                height: usize::MAX,
                width: 0,
            },
        ] {
            assert_eq!(scene.connector_cells_in(empty).count(), 0);
        }
        let saturated = SceneRect {
            row: 0,
            column: 0,
            height: usize::MAX,
            width: usize::MAX,
        };
        assert_eq!(
            scene.connector_cells_in(saturated).count(),
            scene
                .rows()
                .iter()
                .map(|row| row.connectors.len())
                .sum::<usize>()
        );
    }

    #[test]
    fn crossing_cells_name_only_the_edges_that_use_them() {
        let graph = view(
            &["A", "B", "C", "D", "E"],
            &[("A", "D"), ("B", "E"), ("C", "D")],
        );
        let scene = GraphScene::layout(&graph, 60).expect("fits");
        let crossing = scene
            .rows()
            .iter()
            .flat_map(|row| &row.connectors)
            .find(|cell| cell.kind == ConnectorKind::Cross)
            .expect("the unrelated lane crosses the merge");
        let identities: BTreeSet<(&str, &str)> = crossing
            .edges
            .iter()
            .map(|edge| (edge.from.as_str(), edge.to.as_str()))
            .collect();
        assert_eq!(identities, BTreeSet::from([("B", "E"), ("C", "D")]));
        assert!(
            !identities.contains(&("A", "D")),
            "the edge that ended left of the crossing was over-reported"
        );
    }

    /// A merge begins at the leftmost lane arriving at its destination, not at
    /// column zero. The lane renderer used to extend the join across unrelated
    /// lanes on its left, so both the glyph and its hit identities asserted
    /// edges that never traversed those cells.
    ///
    /// This proves `graph.merge_starts_at_destination_lane`.
    #[test]
    fn merge_routes_do_not_extend_left_of_their_destination_lane() {
        let graph = view(
            &["A", "B", "C", "D", "E", "Y", "Z"],
            &[("A", "Z"), ("B", "D"), ("C", "Y"), ("E", "D")],
        );
        let scene = GraphScene::layout(&graph, 60).expect("fits");
        let destination = scene
            .nodes
            .iter()
            .find(|node| node.id == "D")
            .expect("the destination");
        assert!(
            destination.marker.column > 0,
            "the merge must not start at zero"
        );
        let merge = &scene.rows()[destination.marker.row - 1];

        for cell in merge
            .connectors
            .iter()
            .filter(|cell| cell.id.0.column < destination.marker.column)
        {
            assert_eq!(cell.kind, ConnectorKind::Vertical, "{cell:?}");
            assert_eq!(
                cell.edges
                    .iter()
                    .map(|edge| (edge.from.as_str(), edge.to.as_str()))
                    .collect::<Vec<_>>(),
                vec![("A", "Z")],
                "the D merge claimed a cell left of its destination lane"
            );
        }
    }

    #[test]
    fn grapheme_width_empty_titles_and_hostile_text_are_cell_safe() {
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}";
        assert_eq!(display_columns("e\u{301}"), 1, "a combining grapheme");
        assert_eq!(display_columns("\u{754c}"), 2, "a wide grapheme");
        assert_eq!(display_columns(family), 2, "a joined emoji grapheme");

        let mut graph = view(&["A\u{754c}", "e\u{301}", family, "unsafe"], &[]);
        graph.nodes[0].title.clear();
        graph.nodes[1].title = "same title".into();
        graph.nodes[2].title = "same title".into();
        graph.nodes[3].title = "bad \u{1b}[2J and \u{202e} tail".into();
        let scene = GraphScene::layout(&graph, 60).expect("fits");
        let painted = crate::lanes::text_of_scene(&scene, crate::lanes::Glyphs::UNICODE);
        assert!(!painted.contains('\u{1b}'), "an escape reached the painter");
        assert!(
            !painted.contains('\u{202e}'),
            "bidi control reached the painter"
        );
        assert!(painted.contains(family), "a grapheme was split or dropped");

        let title_columns: Vec<usize> = scene
            .rows()
            .iter()
            .filter(|row| row.plain_text.contains("same title"))
            .map(|row| {
                let byte = row.plain_text.find("same title").expect("the title");
                display_columns(&row.plain_text[..byte])
            })
            .collect();
        assert_eq!(title_columns.len(), 2);
        assert_eq!(title_columns[0], title_columns[1]);
        assert_eq!(
            scene.bounds.columns,
            scene
                .rows()
                .iter()
                .map(|row| display_columns(&row.plain_text))
                .max()
                .expect("rows")
        );
        assert_scene_invariants(&scene);
    }

    #[test]
    fn invisible_and_empty_public_ids_keep_visible_raw_identity_targets() {
        let graph = view(&["", "   ", "\u{200b}", "\u{200d}"], &[]);
        let scene = GraphScene::layout(&graph, 60).expect("fits");
        assert_eq!(scene.nodes.len(), graph.nodes.len());
        for node in &scene.nodes {
            assert!(display_columns(&node.display_id) > 0, "{node:?}");
            assert!(scene.hit_test(node.marker).nodes.contains(&node.id));
        }
        assert_scene_invariants(&scene);
    }

    #[test]
    fn every_five_node_dag_preserves_scene_ownership_invariants() {
        let ids = ["A", "B", "C", "D", "E"];
        let possible: Vec<(&str, &str)> = ids
            .iter()
            .enumerate()
            .flat_map(|(from, id)| ids.iter().skip(from + 1).map(move |to| (*id, *to)))
            .collect();
        for mask in 0usize..(1usize << possible.len()) {
            let pairs: Vec<(&str, &str)> = possible
                .iter()
                .enumerate()
                .filter_map(|(bit, pair)| ((mask & (1 << bit)) != 0).then_some(*pair))
                .collect();
            let scene = GraphScene::layout(&view(&ids, &pairs), 60).expect("five nodes fit");
            assert_scene_invariants(&scene);
            let unicode = crate::lanes::text_of_scene(&scene, crate::lanes::Glyphs::UNICODE);
            let ascii = crate::lanes::text_of_scene(&scene, crate::lanes::Glyphs::ASCII);
            assert_eq!(unicode.lines().count(), scene.bounds.rows);
            assert_eq!(ascii.lines().count(), scene.bounds.rows);
        }
    }

    #[test]
    fn required_lanes_uses_the_exact_canonical_layout_and_ignores_source_order() {
        let mut graph = view(
            &["A", "B", "C", "D", "E"],
            &[("A", "D"), ("B", "D"), ("C", "D"), ("D", "E")],
        );
        let expected = GraphScene::required_lanes(&graph);
        assert!(expected > 1, "the fixture must exercise concurrent lanes");
        let too_narrow = expected.saturating_mul(2) + ROOM_FOR_A_LABEL - 1;
        assert_eq!(
            GraphScene::layout(&graph, too_narrow)
                .expect_err("one fewer column must refuse")
                .lanes,
            expected
        );
        GraphScene::layout(&graph, expected * 2 + ROOM_FOR_A_LABEL)
            .expect("the reported lane budget must fit");

        graph.nodes.reverse();
        graph.edges.reverse();
        assert_eq!(GraphScene::required_lanes(&graph), expected);
    }

    #[test]
    fn malformed_public_views_report_canonicalized_duplicate_nodes() {
        let mut graph = view(&["same", "target"], &[("same", "target")]);
        let mut duplicate = node("same");
        duplicate.title = "a lexicographically first duplicate".into();
        graph.nodes.push(duplicate);

        let scene = GraphScene::layout(&graph, 60).expect("the canonical graph fits");
        assert_eq!(
            scene.issues,
            vec![SceneIssue::DuplicateNodeId {
                id: "same".into(),
                occurrences: 2,
            }],
            "an invalid public GraphView was silently made to look complete"
        );
        assert_eq!(
            scene.nodes.iter().filter(|node| node.id == "same").count(),
            1,
            "one semantic identity must not produce ambiguous hit targets"
        );
        assert_eq!(
            scene
                .nodes
                .iter()
                .find(|node| node.id == "same")
                .expect("canonical node")
                .title,
            "a lexicographically first duplicate"
        );
        assert_scene_invariants(&scene);

        let mut status_tie = view(&["same"], &[]);
        let mut other = node("same");
        other.status = Status::Blocked;
        status_tie.nodes.push(other);
        let forward = GraphScene::layout(&status_tie, 60).expect("fits");
        status_tie.nodes.reverse();
        let reversed = GraphScene::layout(&status_tie, 60).expect("fits");
        assert_eq!(
            forward, reversed,
            "canonicalization depended on duplicate input order"
        );
    }

    #[test]
    fn typed_omission_notes_survive_empty_and_scoped_views() {
        let empty = GraphView {
            nodes: Vec::new(),
            edges: Vec::new(),
            hidden_edges: 2,
            grouping: Grouping { edges: 3, tasks: 1 },
            scope: vec!["tag urgent".into()],
        };
        let scene = GraphScene::layout(&empty, 48).expect("an empty scene fits");
        assert_eq!(scene.nodes.len(), 0);
        assert_eq!(
            scene.notes.iter().map(|note| note.kind).collect::<Vec<_>>(),
            vec![
                GraphNoteKind::Scope,
                GraphNoteKind::NoEdges,
                GraphNoteKind::Grouping,
                GraphNoteKind::HiddenEdges,
            ]
        );
        for note in &scene.notes {
            assert!(note.bounds.height > 0, "note has no geometry: {note:?}");
            let hit = scene.hit_test(ScenePoint {
                row: note.bounds.row,
                column: note.bounds.column,
            });
            assert!(hit.notes.contains(&note.kind));
        }
        let text = crate::lanes::text_of_scene(&scene, crate::lanes::Glyphs::ASCII);
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains("0 tasks in view and no edges between them"));
        assert!(flat.contains("outside this view are not drawn"));
    }
}
