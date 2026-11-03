//! Deterministic terminal-cell geometry for the semantic graph explorer.
//!
//! The public model in this module is deliberately independent of the
//! coordinate kernel. Delivery adapters can retain, navigate, test and paint a
//! [`CellScene`] without importing graphlay or depending on its output model.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

use crate::{
    GraphExplorerCondition, GraphExplorerDependency, GraphExplorerEdgeDisposition,
    GraphExplorerIdentity, GraphExplorerLocation, GraphExplorerNode, GraphExplorerProjection,
    GraphExplorerState, InputIdentity, Status, TaskBrowserRow,
};

mod graphlay_adapter;

use graphlay_adapter::{
    AdapterEdge, AdapterFailure, AdapterInput, AdapterNode, AdapterOrientation, AdapterOutput,
    AdapterRouteKind, AdapterSide,
};

const DEFAULT_CARD_COLUMNS: u16 = 26;
const CELL_PADDING: i32 = 1;
const AUTO_HYSTERESIS_PERCENT: u64 = 12;

/// Requested direction for the layered terminal scene.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellSceneOrientation {
    #[default]
    Auto,
    LeftToRight,
    TopToBottom,
}

/// Concrete direction chosen for one scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellSceneDirection {
    LeftToRight,
    TopToBottom,
}

/// A sort applied only to peers in the same phase lane and dependency rank.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellPeerOrder {
    #[default]
    Readability,
    Identity,
    Title,
    Status,
    PlanPriority,
    Criticality,
}

/// Card detail shared by interactive and static cell-scene consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellSceneDensity {
    Compact,
    Comfortable,
}

/// Hard work bounds for an interactive layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CellSceneBudget {
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_routing_waypoints: usize,
}

impl Default for CellSceneBudget {
    fn default() -> Self {
        Self {
            max_nodes: 64,
            max_edges: 96,
            max_routing_waypoints: 1_536,
        }
    }
}

/// Size of one semantic card in terminal display cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct CellCardSize {
    pub columns: u16,
    pub rows: u16,
}

impl CellCardSize {
    /// Fit the node identity and title without splitting a grapheme cluster.
    #[must_use]
    pub fn fit(node: &GraphExplorerNode, max_content_columns: u16) -> Self {
        let content_columns = max_content_columns.max(4);
        let id_columns = node.identity.id().width();
        let title_rows = grapheme_wrapped_rows(&node.title, usize::from(content_columns));
        let widest = id_columns
            .max(widest_grapheme_line(
                &node.title,
                usize::from(content_columns),
            ))
            .min(usize::from(content_columns));
        Self {
            columns: u16::try_from(widest.saturating_add(2))
                .unwrap_or(u16::MAX)
                .max(6),
            rows: title_rows.saturating_add(3).max(4),
        }
    }

    /// Measure exactly the card rows shared by the TUI and static CLI painter.
    #[must_use]
    pub fn for_density(node: &GraphExplorerNode, density: CellSceneDensity) -> Self {
        let maximum = match density {
            CellSceneDensity::Compact => 14,
            CellSceneDensity::Comfortable => 26,
        };
        let base = Self::fit(node, maximum);
        let mut widest =
            usize::from(base.columns.saturating_sub(2)).max(status_label(node.status).width());
        let mut rows = 4;
        if density == CellSceneDensity::Comfortable {
            widest = widest.max(
                status_label(node.status)
                    .width()
                    .saturating_add(1)
                    .saturating_add(node.title.width()),
            );
            if node.facts.tasks > 0 {
                widest = widest.max(
                    format!(
                        "{}/{} done | {} ready",
                        node.facts.done, node.facts.tasks, node.facts.ready
                    )
                    .width(),
                );
                rows = 5;
            }
        }
        Self {
            columns: u16::try_from(widest.min(usize::from(maximum)).saturating_add(2))
                .unwrap_or(u16::MAX)
                .max(6),
            rows,
        }
    }
}

const fn status_label(status: Status) -> &'static str {
    match status {
        Status::Todo => "TODO",
        Status::InProgress => "DOING",
        Status::Blocked => "BLOCKED",
        Status::Done => "DONE",
        Status::Dropped => "DROPPED",
    }
}

/// Total peer keys shared by every `CellScene` client.
///
/// The coordinate kernel applies these keys only inside one phase/rank slot,
/// so a presentation order can never invent dependency or phase causality.
#[must_use]
pub fn cell_scene_ordering_hints(
    order: CellPeerOrder,
    projection: &GraphExplorerProjection,
    catalogue: &[TaskBrowserRow],
) -> BTreeMap<GraphExplorerIdentity, i64> {
    if order == CellPeerOrder::Readability {
        return BTreeMap::new();
    }
    let rows: BTreeMap<&str, (&TaskBrowserRow, usize)> = catalogue
        .iter()
        .enumerate()
        .map(|(index, row)| (row.id.as_str(), (row, index)))
        .collect();
    let critical: BTreeSet<&str> = projection
        .critical_path
        .iter()
        .map(String::as_str)
        .collect();
    let mut nodes: Vec<&GraphExplorerNode> = projection.graph.nodes.iter().collect();
    nodes.sort_by(|left, right| {
        cell_node_order(order, left, right, &rows, &critical)
            .then_with(|| left.identity.cmp(&right.identity))
    });
    nodes
        .into_iter()
        .enumerate()
        .map(|(index, node)| {
            (
                node.identity.clone(),
                i64::try_from(index).unwrap_or(i64::MAX),
            )
        })
        .collect()
}

fn cell_node_order(
    order: CellPeerOrder,
    left: &GraphExplorerNode,
    right: &GraphExplorerNode,
    rows: &BTreeMap<&str, (&TaskBrowserRow, usize)>,
    critical: &BTreeSet<&str>,
) -> Ordering {
    match order {
        CellPeerOrder::Readability | CellPeerOrder::Identity => left.identity.cmp(&right.identity),
        CellPeerOrder::Title => left
            .title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.title.cmp(&right.title)),
        CellPeerOrder::Status => {
            cell_node_status_key(left, rows).cmp(&cell_node_status_key(right, rows))
        }
        CellPeerOrder::PlanPriority => {
            cell_node_priority_key(left, rows).cmp(&cell_node_priority_key(right, rows))
        }
        CellPeerOrder::Criticality => cell_node_is_critical(left, critical)
            .cmp(&cell_node_is_critical(right, critical))
            .reverse(),
    }
}

fn represented_rows<'a>(
    node: &'a GraphExplorerNode,
    rows: &'a BTreeMap<&str, (&'a TaskBrowserRow, usize)>,
) -> impl Iterator<Item = (&'a TaskBrowserRow, usize)> + 'a {
    node.represented_ids
        .iter()
        .filter_map(|id| rows.get(id.as_str()).copied())
}

fn cell_node_priority_key(
    node: &GraphExplorerNode,
    rows: &BTreeMap<&str, (&TaskBrowserRow, usize)>,
) -> (usize, usize) {
    represented_rows(node, rows)
        .map(|(row, index)| (row.ready_rank.unwrap_or(usize::MAX), index))
        .min()
        .unwrap_or((usize::MAX, usize::MAX))
}

const fn status_rank(status: Status) -> u8 {
    match status {
        Status::InProgress => 0,
        Status::Todo => 1,
        Status::Blocked => 2,
        Status::Done => 3,
        Status::Dropped => 4,
    }
}

fn cell_node_status_key(
    node: &GraphExplorerNode,
    rows: &BTreeMap<&str, (&TaskBrowserRow, usize)>,
) -> (u8, usize, usize) {
    let (ready, source) = represented_rows(node, rows)
        .map(|(row, index)| (row.ready_rank.unwrap_or(usize::MAX), index))
        .min()
        .unwrap_or((usize::MAX, usize::MAX));
    (status_rank(node.status), ready, source)
}

fn cell_node_is_critical(node: &GraphExplorerNode, critical: &BTreeSet<&str>) -> bool {
    node.represented_ids
        .iter()
        .any(|id| critical.contains(id.as_str()))
}

/// One immutable layout request. It owns its inputs so it can be moved to a
/// worker without borrowing the paint path.
#[derive(Debug, Clone, PartialEq)]
pub struct CellSceneRequest {
    pub projection: GraphExplorerProjection,
    pub card_sizes: BTreeMap<GraphExplorerIdentity, CellCardSize>,
    pub ordering_hints: BTreeMap<GraphExplorerIdentity, i64>,
    pub orientation: CellSceneOrientation,
    pub peer_order: CellPeerOrder,
    pub selected: Option<GraphExplorerIdentity>,
    pub prior: Option<Box<CellScene>>,
    pub budget: CellSceneBudget,
}

impl CellSceneRequest {
    /// Construct a bounded request with grapheme-safe default card sizes.
    #[must_use]
    pub fn new(projection: GraphExplorerProjection) -> Self {
        let card_sizes = projection
            .graph
            .nodes
            .iter()
            .map(|node| {
                (
                    node.identity.clone(),
                    CellCardSize::fit(node, DEFAULT_CARD_COLUMNS),
                )
            })
            .collect();
        let selected = projection
            .selection
            .as_ref()
            .map(|selection| selection.represented_by.clone());
        Self {
            projection,
            card_sizes,
            ordering_hints: BTreeMap::new(),
            orientation: CellSceneOrientation::Auto,
            peer_order: CellPeerOrder::Readability,
            selected,
            prior: None,
            budget: CellSceneBudget::default(),
        }
    }
}

/// Work item intentionally run by an application worker, never by a painter.
#[derive(Debug, Clone, PartialEq)]
pub struct CellSceneJob {
    generation: u64,
    request: CellSceneRequest,
}

impl CellSceneJob {
    #[must_use]
    pub const fn new(generation: u64, request: CellSceneRequest) -> Self {
        Self {
            generation,
            request,
        }
    }

    /// Run the bounded coordinate kernel and terminal-cell validator.
    #[must_use]
    pub fn run(self) -> CellSceneCompletion {
        CellSceneCompletion {
            generation: self.generation,
            result: build_scene(self.generation, &self.request),
        }
    }
}

/// Generation-tagged worker completion.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellSceneCompletion {
    pub generation: u64,
    pub result: CellSceneResult,
}

impl CellSceneCompletion {
    /// Accept this completion only for the generation which requested it.
    pub fn into_current(self, expected: u64) -> Result<CellSceneResult, StaleCellSceneCompletion> {
        if self.generation == expected {
            Ok(self.result)
        } else {
            Err(StaleCellSceneCompletion {
                expected,
                completed: self.generation,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cell-scene generation {completed} is stale; current generation is {expected}")]
pub struct StaleCellSceneCompletion {
    pub expected: u64,
    pub completed: u64,
}

/// Either validated cell geometry or an honest fallback retaining the last
/// valid scene when one was supplied.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CellSceneResult {
    Ready(Box<CellScene>),
    Fallback(Box<CellSceneFallback>),
}

/// A terminal-cell point.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct CellPoint {
    pub x: i32,
    pub y: i32,
}

/// A non-empty terminal-cell rectangle, with an exclusive right/bottom edge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct CellRect {
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
}

impl CellRect {
    #[must_use]
    pub fn center(self) -> CellPoint {
        CellPoint {
            x: self.x + i32::from(self.width) / 2,
            y: self.y + i32::from(self.height) / 2,
        }
    }

    fn right(self) -> i32 {
        self.x + i32::from(self.width)
    }

    fn bottom(self) -> i32 {
        self.y + i32::from(self.height)
    }

    fn overlaps(self, other: Self) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellPortSide {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CellPort {
    pub point: CellPoint,
    pub side: CellPortSide,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct CellEdgeIdentity {
    pub from: GraphExplorerIdentity,
    pub to: GraphExplorerIdentity,
    /// Stable ordinal among edges with the same semantic endpoints.
    pub ordinal: u32,
    pub provenance: Vec<GraphExplorerDependency>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellRouteKind {
    Direct,
    Chained { spans: u32 },
    SelfLoop,
    Bundled { lane: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellFeedbackKind {
    None,
    AggregateQuotient,
    CanonicalTaskCycle,
    KernelCycleReversal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellSceneNode {
    pub identity: GraphExplorerIdentity,
    pub bounds: CellRect,
    pub rank: Option<i32>,
    pub order: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellSceneEdge {
    pub identity: CellEdgeIdentity,
    pub disposition: GraphExplorerEdgeDisposition,
    pub feedback: CellFeedbackKind,
    pub kind: Option<CellRouteKind>,
    pub points: Vec<CellPoint>,
    pub source_port: Option<CellPort>,
    pub target_port: Option<CellPort>,
    pub arrowhead: Option<CellPoint>,
    pub degradations: Vec<CellSceneDegradation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellSceneBand {
    pub phase: u32,
    pub label: String,
    pub label_anchor: Option<CellPoint>,
    pub nodes: Vec<GraphExplorerIdentity>,
    pub bounds: Option<CellRect>,
    pub degradation: Option<CellSceneDegradation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellCrossing {
    pub at: CellPoint,
    pub first: CellEdgeIdentity,
    pub second: CellEdgeIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellSharedSegment {
    pub from: CellPoint,
    pub to: CellPoint,
    pub edges: Vec<CellEdgeIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellJunction {
    pub at: CellPoint,
    pub edges: Vec<CellEdgeIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellParallelEdges {
    pub from: GraphExplorerIdentity,
    pub to: GraphExplorerIdentity,
    pub edges: Vec<CellEdgeIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellSceneFocus {
    pub identity: GraphExplorerIdentity,
    pub bounds: CellRect,
    /// A consumer should recenter this semantic subject after an orientation
    /// change instead of preserving an unrelated absolute scroll offset.
    pub recenter: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellRelayoutReason {
    StructuralGraphChange,
    CardSizeChange,
    OrientationChange,
    OrderingPolicyChange,
    LayoutInputChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CellSceneMovement {
    Fresh,
    ReusedStatusOnly,
    Relayout {
        moved_nodes: usize,
        total_distance: u64,
        max_distance: u32,
        orientation_changed: bool,
        reasons: Vec<CellRelayoutReason>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CellSceneMetrics {
    pub crossings: usize,
    pub bends: usize,
    pub shared_segments: usize,
    pub junctions: usize,
    pub parallel_groups: usize,
    pub routing_waypoints: u32,
    pub node_edge_hits: usize,
    pub node_overlaps: usize,
    pub endpoint_errors: usize,
    pub degraded_edges: usize,
    pub area_cells: u64,
    pub quality_score: u64,
}

/// Fully validated scene in integer display cells.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellScene {
    pub generation: u64,
    pub input: InputIdentity,
    pub location: GraphExplorerLocation,
    pub geometry_key: String,
    pub direction: CellSceneDirection,
    pub ordering_policy: CellPeerOrder,
    pub bounds: CellRect,
    pub nodes: Vec<CellSceneNode>,
    pub edges: Vec<CellSceneEdge>,
    pub bands: Vec<CellSceneBand>,
    pub crossings: Vec<CellCrossing>,
    pub shared_segments: Vec<CellSharedSegment>,
    pub junctions: Vec<CellJunction>,
    pub parallel_edges: Vec<CellParallelEdges>,
    pub focus: Option<CellSceneFocus>,
    pub movement: CellSceneMovement,
    pub metrics: CellSceneMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum CellSceneDegradation {
    SourceState {
        state: GraphExplorerState,
    },
    EmptyProjection,
    LimitExceeded {
        resource: String,
        got: usize,
        limit: usize,
    },
    MissingCardSize,
    InvalidCardSize {
        columns: u16,
        rows: u16,
    },
    InvalidProjection {
        issue: String,
    },
    CellExtentExceeded {
        axis: String,
        cells: i32,
    },
    KernelRejected,
    MissingNodePlacement,
    MissingRoute,
    CollapsedRoute,
    NonOrthogonalSegment,
    NonMonotoneForwardRoute,
    InvalidSourcePort,
    InvalidTargetPort,
    OutOfBounds,
    RouteThroughNode {
        node: GraphExplorerIdentity,
    },
    NodeOverlap {
        other: GraphExplorerIdentity,
    },
    EmptyPhaseBand,
    InvalidPhaseBand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellNodeOmission {
    pub identity: GraphExplorerIdentity,
    pub reason: CellSceneDegradation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellEdgeOmission {
    pub identity: CellEdgeIdentity,
    pub reason: CellSceneDegradation,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellSceneFallback {
    pub generation: u64,
    pub input: InputIdentity,
    pub location: GraphExplorerLocation,
    pub reasons: Vec<CellSceneDegradation>,
    pub nodes: Vec<CellNodeOmission>,
    pub edges: Vec<CellEdgeOmission>,
    pub retained: Option<Box<CellScene>>,
}

/// An application/layout invariant failed while adapting a cell scene for the
/// dependency-free static output painter.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayeredTextSceneError {
    #[error("cell scene node `{identity}` has no matching semantic graph node")]
    MissingSemanticNode { identity: String },
}

/// Adapt one validated cell scene without re-deriving graph meaning.
///
/// This is the only application-to-static-output boundary. It deliberately
/// retains identities, ranks, peer order, group members, edge provenance and
/// omission accounting even where the painter needs only a subset. That makes
/// parity testable as typed data and turns model drift into an error instead of
/// a panic or a plausible-looking partial graph.
#[allow(clippy::too_many_lines)]
pub fn layered_text_scene(
    projection: &GraphExplorerProjection,
    scene: &CellScene,
    show_selection: bool,
    show_critical: bool,
) -> Result<aplan_output::layered::LayeredTextScene, LayeredTextSceneError> {
    use aplan_output::layered::{
        LayeredBand, LayeredDirection, LayeredEdge, LayeredFacts, LayeredNode, LayeredOmissions,
        LayeredTextScene,
    };

    // Every interactive scene has an anchor so keyboard navigation has a
    // stable starting point. Static output must not turn that implementation
    // detail into a user-visible `--focus` claim: only an explicit static
    // selection receives the selected/related treatment.
    let selected = show_selection
        .then(|| scene.focus.as_ref().map(|focus| &focus.identity))
        .flatten();
    let related: BTreeSet<_> = scene
        .edges
        .iter()
        .filter_map(|edge| {
            if selected == Some(&edge.identity.from) {
                Some(edge.identity.to.clone())
            } else if selected == Some(&edge.identity.to) {
                Some(edge.identity.from.clone())
            } else {
                None
            }
        })
        .collect();
    let semantic: BTreeMap<_, _> = projection
        .graph
        .nodes
        .iter()
        .map(|node| (&node.identity, node))
        .collect();
    let critical: BTreeSet<_> = projection
        .critical_path
        .iter()
        .map(String::as_str)
        .collect();

    let nodes = scene
        .nodes
        .iter()
        .map(|node| {
            let source = semantic.get(&node.identity).ok_or_else(|| {
                LayeredTextSceneError::MissingSemanticNode {
                    identity: node.identity.id().to_string(),
                }
            })?;
            Ok(LayeredNode {
                id: node.identity.id().to_string(),
                represented_ids: source.represented_ids.clone(),
                rank: node.rank,
                order: node.order,
                title: source.title.clone(),
                status: source.status,
                bounds: output_rect(node.bounds),
                facts: LayeredFacts {
                    tasks: source.facts.tasks,
                    done: source.facts.done,
                    ready: source.facts.ready,
                },
                selected: selected == Some(&node.identity),
                related: related.contains(&node.identity),
                critical: show_critical
                    && source
                        .represented_ids
                        .iter()
                        .any(|id| critical.contains(id.as_str())),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let edges = scene
        .edges
        .iter()
        .map(|edge| LayeredEdge {
            from: edge.identity.from.id().to_string(),
            to: edge.identity.to.id().to_string(),
            ordinal: edge.identity.ordinal,
            provenance: edge
                .identity
                .provenance
                .iter()
                .map(|dependency| (dependency.from.clone(), dependency.to.clone()))
                .collect(),
            points: edge.points.iter().copied().map(output_point).collect(),
            source_port: edge.source_port.map(|port| output_point(port.point)),
            target_port: edge.target_port.map(|port| output_point(port.point)),
            arrowhead: edge.arrowhead.map(output_point),
            feedback: edge.feedback != CellFeedbackKind::None,
            bundled: matches!(edge.kind, Some(CellRouteKind::Bundled { .. })),
            related: selected.is_some_and(|selected| {
                selected == &edge.identity.from || selected == &edge.identity.to
            }),
            degradations: edge
                .degradations
                .iter()
                .map(serialized_degradation)
                .collect(),
        })
        .collect();
    let bands = scene
        .bands
        .iter()
        .map(|band| LayeredBand {
            phase: band.phase,
            nodes: band
                .nodes
                .iter()
                .map(|identity| identity.id().to_string())
                .collect(),
            bounds: band.bounds.map(output_rect),
            label_anchor: band.label_anchor.map(output_point),
        })
        .collect();
    let accounting = &projection.accounting;
    Ok(LayeredTextScene {
        direction: match scene.direction {
            CellSceneDirection::LeftToRight => LayeredDirection::LeftToRight,
            CellSceneDirection::TopToBottom => LayeredDirection::TopToBottom,
        },
        bounds: output_rect(scene.bounds),
        nodes,
        edges,
        bands,
        crossings: scene
            .crossings
            .iter()
            .map(|crossing| output_point(crossing.at))
            .collect(),
        junctions: scene
            .junctions
            .iter()
            .map(|junction| output_point(junction.at))
            .collect(),
        omissions: LayeredOmissions {
            admitted_task_ids: accounting.admitted_task_ids.clone(),
            level_task_ids: accounting.level_task_ids.clone(),
            visible_task_ids: accounting.visible_task_ids.clone(),
            level_omitted_task_ids: accounting.level_omitted_task_ids.clone(),
            window_omitted_task_ids: accounting.window_omitted_task_ids.clone(),
            admitted_dependency_edges: accounting.admitted_dependency_edges,
            represented_dependency_edges: accounting.represented_dependency_edges,
            boundary_dependency_edges: accounting.boundary_dependency_edges,
            boundary_dependencies: accounting
                .boundary_dependencies
                .iter()
                .map(|edge| (edge.from.clone(), edge.to.clone()))
                .collect(),
        },
        notes: Vec::new(),
    })
}

fn output_point(point: CellPoint) -> aplan_output::layered::LayeredPoint {
    aplan_output::layered::LayeredPoint {
        x: point.x,
        y: point.y,
    }
}

fn output_rect(rect: CellRect) -> aplan_output::layered::LayeredRect {
    aplan_output::layered::LayeredRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

fn serialized_degradation(reason: &CellSceneDegradation) -> String {
    serde_json::to_string(reason).unwrap_or_else(|_| format!("{reason:?}"))
}

#[derive(Debug)]
struct SceneInput {
    geometry_key: String,
    edge_identities: Vec<CellEdgeIdentity>,
    adapter: AdapterInput,
}

fn build_scene(generation: u64, request: &CellSceneRequest) -> CellSceneResult {
    let fallback_reason = source_failure(&request.projection)
        .or_else(|| budget_failure(request))
        .or_else(|| semantic_failure(request));
    if let Some(reason) = fallback_reason {
        return fallback(generation, request, vec![reason]);
    }
    let prepared = match prepare_input(request) {
        Ok(prepared) => prepared,
        Err(reason) => return fallback(generation, request, vec![reason]),
    };
    if let Some(prior) = request.prior.as_deref() {
        let direction_allowed = request.orientation == CellSceneOrientation::Auto
            || direction_for(request.orientation) == Some(prior.direction);
        if direction_allowed && prior.geometry_key == prepared.geometry_key {
            let mut scene = prior.clone();
            scene.generation = generation;
            scene.input = request.projection.input.clone();
            scene.location = request.projection.location.clone();
            scene.focus = focus_for(&scene.nodes, request.selected.as_ref(), false);
            scene.movement = CellSceneMovement::ReusedStatusOnly;
            return CellSceneResult::Ready(Box::new(scene));
        }
    }

    let directions: &[CellSceneDirection] = match request.orientation {
        CellSceneOrientation::Auto => &[
            CellSceneDirection::LeftToRight,
            CellSceneDirection::TopToBottom,
        ],
        CellSceneOrientation::LeftToRight => &[CellSceneDirection::LeftToRight],
        CellSceneOrientation::TopToBottom => &[CellSceneDirection::TopToBottom],
    };
    let mut candidates = Vec::new();
    let mut failures = Vec::new();
    for direction in directions {
        match layout_candidate(generation, request, &prepared, *direction) {
            Ok(scene) => candidates.push(scene),
            Err(reason) => failures.push(reason),
        }
    }
    if candidates.is_empty() {
        return fallback(generation, request, failures);
    }
    candidates.sort_by(|left, right| {
        left.metrics
            .quality_score
            .cmp(&right.metrics.quality_score)
            .then(left.direction.cmp(&right.direction))
    });
    let mut selected = candidates.remove(0);
    if request.orientation == CellSceneOrientation::Auto
        && let Some(prior_direction) = request.prior.as_ref().map(|prior| prior.direction)
        && let Some(index) = candidates
            .iter()
            .position(|candidate| candidate.direction == prior_direction)
    {
        let incumbent = &candidates[index];
        if within_auto_hysteresis(
            selected.metrics.quality_score,
            incumbent.metrics.quality_score,
        ) {
            selected = candidates.remove(index);
        }
    }
    CellSceneResult::Ready(Box::new(selected))
}

fn semantic_failure(request: &CellSceneRequest) -> Option<CellSceneDegradation> {
    let mut node_phases = BTreeMap::new();
    for node in &request.projection.graph.nodes {
        if node_phases
            .insert(node.identity.clone(), node.phase)
            .is_some()
        {
            return Some(invalid_projection("duplicate node identity"));
        }
    }
    let node_ids: BTreeSet<_> = node_phases.keys().cloned().collect();
    let sized_ids: BTreeSet<_> = request.card_sizes.keys().cloned().collect();
    if sized_ids != node_ids {
        return Some(invalid_projection("card sizes do not match admitted nodes"));
    }
    if request
        .ordering_hints
        .keys()
        .any(|identity| !node_ids.contains(identity))
    {
        return Some(invalid_projection("ordering hint names an unadmitted node"));
    }

    let mut phases = BTreeSet::new();
    let mut members = BTreeSet::new();
    for band in &request.projection.graph.phase_bands {
        if !phases.insert(band.phase) {
            return Some(invalid_projection("duplicate phase band"));
        }
        for identity in &band.nodes {
            if node_phases.get(identity) != Some(&band.phase) {
                return Some(invalid_projection(
                    "phase membership disagrees with its node",
                ));
            }
            if !members.insert(identity.clone()) {
                return Some(invalid_projection(
                    "node belongs to more than one phase band",
                ));
            }
        }
    }
    if members != node_ids {
        return Some(invalid_projection("an admitted node has no phase band"));
    }
    None
}

fn invalid_projection(issue: &str) -> CellSceneDegradation {
    CellSceneDegradation::InvalidProjection {
        issue: issue.to_string(),
    }
}

fn within_auto_hysteresis(best: u64, incumbent: u64) -> bool {
    let threshold = best.saturating_mul(100 + AUTO_HYSTERESIS_PERCENT) / 100;
    incumbent <= threshold
}

fn source_failure(projection: &GraphExplorerProjection) -> Option<CellSceneDegradation> {
    match &projection.state {
        GraphExplorerState::Complete | GraphExplorerState::InvalidTaskCycle => None,
        GraphExplorerState::Empty => Some(CellSceneDegradation::EmptyProjection),
        state => Some(CellSceneDegradation::SourceState {
            state: state.clone(),
        }),
    }
}

fn budget_failure(request: &CellSceneRequest) -> Option<CellSceneDegradation> {
    let counts = [
        (
            "nodes",
            request.projection.graph.nodes.len(),
            request.budget.max_nodes,
        ),
        (
            "edges",
            request.projection.graph.edges.len(),
            request.budget.max_edges,
        ),
    ];
    counts
        .into_iter()
        .find(|(_, got, limit)| got > limit)
        .map(
            |(resource, got, limit)| CellSceneDegradation::LimitExceeded {
                resource: resource.to_string(),
                got,
                limit,
            },
        )
}

fn prepare_input(request: &CellSceneRequest) -> Result<SceneInput, CellSceneDegradation> {
    let peer_keys = peer_keys(request);
    let phase_groups: BTreeMap<_, _> = request
        .projection
        .graph
        .phase_bands
        .iter()
        .enumerate()
        .map(|(index, band)| (band.phase, u32::try_from(index).unwrap_or(u32::MAX)))
        .collect();
    let prior_centers: BTreeMap<_, _> = request
        .prior
        .as_deref()
        .map(|prior| {
            prior
                .nodes
                .iter()
                .map(|node| (node.identity.clone(), node.bounds.center()))
                .collect()
        })
        .unwrap_or_default();
    let nodes = adapter_nodes(request, &peer_keys, &phase_groups, &prior_centers)?;
    let (edges, edge_identities) = adapter_edges(request)?;
    Ok(SceneInput {
        geometry_key: geometry_key(request),
        edge_identities,
        adapter: AdapterInput {
            nodes,
            edges,
            max_nodes: request.budget.max_nodes,
            max_edges: request.budget.max_edges,
            max_routing_waypoints: request.budget.max_routing_waypoints,
        },
    })
}

fn peer_keys(request: &CellSceneRequest) -> BTreeMap<GraphExplorerIdentity, i64> {
    if request.peer_order == CellPeerOrder::Readability {
        BTreeMap::new()
    } else {
        let mut peer_order: Vec<&GraphExplorerNode> =
            request.projection.graph.nodes.iter().collect();
        peer_order.sort_by(|left, right| peer_cmp(request.peer_order, left, right));
        peer_order
            .into_iter()
            .enumerate()
            .map(|(index, node)| {
                (
                    node.identity.clone(),
                    i64::try_from(index).unwrap_or(i64::MAX),
                )
            })
            .collect()
    }
}

fn adapter_nodes(
    request: &CellSceneRequest,
    peer_keys: &BTreeMap<GraphExplorerIdentity, i64>,
    phase_groups: &BTreeMap<u32, u32>,
    prior_centers: &BTreeMap<GraphExplorerIdentity, CellPoint>,
) -> Result<Vec<AdapterNode>, CellSceneDegradation> {
    let mut nodes = Vec::with_capacity(request.projection.graph.nodes.len());
    for node in &request.projection.graph.nodes {
        let Some(size) = request.card_sizes.get(&node.identity).copied() else {
            return Err(CellSceneDegradation::MissingCardSize);
        };
        if size.columns == 0 || size.rows == 0 {
            return Err(CellSceneDegradation::InvalidCardSize {
                columns: size.columns,
                rows: size.rows,
            });
        }
        nodes.push(AdapterNode {
            key: node.identity.id().to_string(),
            size,
            group: phase_groups.get(&node.phase).copied(),
            lane_key: request
                .ordering_hints
                .get(&node.identity)
                .copied()
                .or_else(|| peer_keys.get(&node.identity).copied()),
            seed: prior_centers.get(&node.identity).copied(),
        });
    }
    Ok(nodes)
}

fn adapter_edges(
    request: &CellSceneRequest,
) -> Result<(Vec<AdapterEdge>, Vec<CellEdgeIdentity>), CellSceneDegradation> {
    let node_indices: BTreeMap<_, _> = request
        .projection
        .graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.identity.clone(), index))
        .collect();
    let mut ordinals = BTreeMap::<(GraphExplorerIdentity, GraphExplorerIdentity), u32>::new();
    let mut edge_identities = Vec::with_capacity(request.projection.graph.edges.len());
    let mut edges = Vec::with_capacity(request.projection.graph.edges.len());
    for edge in &request.projection.graph.edges {
        let Some(&source) = node_indices.get(&edge.from) else {
            return Err(CellSceneDegradation::KernelRejected);
        };
        let Some(&target) = node_indices.get(&edge.to) else {
            return Err(CellSceneDegradation::KernelRejected);
        };
        let ordinal = ordinals
            .entry((edge.from.clone(), edge.to.clone()))
            .or_default();
        edge_identities.push(CellEdgeIdentity {
            from: edge.from.clone(),
            to: edge.to.clone(),
            ordinal: *ordinal,
            provenance: edge.provenance.clone(),
        });
        *ordinal = ordinal.saturating_add(1);
        edges.push(AdapterEdge {
            source,
            target,
            reverse_for_ranking: edge.disposition
                == GraphExplorerEdgeDisposition::AggregateFeedback,
        });
    }
    Ok((edges, edge_identities))
}

fn peer_cmp(
    order: CellPeerOrder,
    left: &GraphExplorerNode,
    right: &GraphExplorerNode,
) -> std::cmp::Ordering {
    let order = match order {
        CellPeerOrder::Readability | CellPeerOrder::Identity => left.identity.cmp(&right.identity),
        CellPeerOrder::Title => left.title.cmp(&right.title),
        CellPeerOrder::Status => left.status.as_str().cmp(right.status.as_str()),
        // These policies carry client-computed total keys in `ordering_hints`.
        // Identity remains the deterministic fail-closed order if a caller
        // deliberately omits a hint.
        CellPeerOrder::PlanPriority | CellPeerOrder::Criticality => {
            left.identity.cmp(&right.identity)
        }
    };
    order.then(left.identity.cmp(&right.identity))
}

fn layout_candidate(
    generation: u64,
    request: &CellSceneRequest,
    prepared: &SceneInput,
    direction: CellSceneDirection,
) -> Result<CellScene, CellSceneDegradation> {
    let orientation = match direction {
        CellSceneDirection::LeftToRight => AdapterOrientation::LeftToRight,
        CellSceneDirection::TopToBottom => AdapterOrientation::TopToBottom,
    };
    let raw =
        graphlay_adapter::layout(&prepared.adapter, orientation).map_err(
            |failure| match failure {
                AdapterFailure::TooLarge {
                    resource,
                    got,
                    limit,
                } => CellSceneDegradation::LimitExceeded {
                    resource,
                    got,
                    limit,
                },
                AdapterFailure::Rejected => CellSceneDegradation::KernelRejected,
            },
        )?;
    quantize_scene(generation, request, prepared, direction, &raw)
}

fn quantize_scene(
    generation: u64,
    request: &CellSceneRequest,
    prepared: &SceneInput,
    direction: CellSceneDirection,
    raw: &AdapterOutput,
) -> Result<CellScene, CellSceneDegradation> {
    if raw.nodes.len() != request.projection.graph.nodes.len() {
        return Err(CellSceneDegradation::MissingNodePlacement);
    }
    if raw.edges.len() != prepared.edge_identities.len()
        || raw
            .reversed
            .iter()
            .any(|index| *index >= prepared.edge_identities.len())
    {
        return Err(CellSceneDegradation::KernelRejected);
    }
    let nodes: Vec<_> = raw
        .nodes
        .iter()
        .zip(&request.projection.graph.nodes)
        .map(|(placed, semantic)| CellSceneNode {
            identity: semantic.identity.clone(),
            bounds: placed.bounds,
            rank: placed.rank,
            order: placed.order,
        })
        .collect();
    validate_node_separation(&nodes)?;
    let bounds = scene_bounds(&nodes, raw)?;
    validate_node_bounds(&nodes, bounds)?;
    let cycle_components = invalid_cycle_components(&request.projection.conditions);
    let reversed: BTreeSet<usize> = raw.reversed.iter().copied().collect();
    let mut edges = Vec::with_capacity(prepared.edge_identities.len());
    for (index, identity) in prepared.edge_identities.iter().enumerate() {
        let semantic = &request.projection.graph.edges[index];
        let feedback = feedback_kind(semantic, &cycle_components, reversed.contains(&index));
        let route = raw.edges.get(index);
        edges.push(quantize_edge(
            identity.clone(),
            semantic.disposition,
            feedback,
            route,
            &nodes,
            bounds,
            direction,
        ));
    }
    // A route whose geometry failed validation is not a drawable edge. A
    // scene containing one would still carry the right edge count while
    // painting no connector, which is a more dangerous answer than an
    // explicit fallback because it looks complete. Let Auto try its other
    // orientation; if no candidate is complete, retain the prior scene only
    // as typed recovery data and present the current projection as a list.
    let bands = quantize_bands(request, &nodes, raw, bounds);
    ensure_complete_geometry(&edges, &bands)?;
    let shared_segments = shared_segments(&edges);
    let crossings = crossings(&edges);
    let junctions = junctions(raw, &edges);
    let parallel_edges = parallel_edges(&edges);
    let movement = movement(
        request.prior.as_deref(),
        &nodes,
        &prepared.edge_identities,
        direction,
        request.peer_order,
    );
    let orientation_changed = matches!(
        movement,
        CellSceneMovement::Relayout {
            orientation_changed: true,
            ..
        }
    );
    let focus = focus_for(&nodes, request.selected.as_ref(), orientation_changed);
    let metrics = scene_metrics(
        raw,
        bounds,
        SceneMetricsParts {
            edges: &edges,
            crossings: &crossings,
            shared_segments: &shared_segments,
            junctions: &junctions,
            parallel_edges: &parallel_edges,
            movement: &movement,
        },
    );
    Ok(CellScene {
        generation,
        input: request.projection.input.clone(),
        location: request.projection.location.clone(),
        geometry_key: prepared.geometry_key.clone(),
        direction,
        ordering_policy: request.peer_order,
        bounds,
        nodes,
        edges,
        bands,
        crossings,
        shared_segments,
        junctions,
        parallel_edges,
        focus,
        movement,
        metrics,
    })
}

fn ensure_complete_geometry(
    edges: &[CellSceneEdge],
    bands: &[CellSceneBand],
) -> Result<(), CellSceneDegradation> {
    if let Some(reason) = edges
        .iter()
        .flat_map(|edge| edge.degradations.iter())
        .next()
        .cloned()
    {
        return Err(reason);
    }
    if let Some(reason) = bands
        .iter()
        .filter_map(|band| band.degradation.as_ref())
        .find(|reason| **reason != CellSceneDegradation::EmptyPhaseBand)
        .cloned()
    {
        return Err(reason);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct SceneMetricsParts<'a> {
    edges: &'a [CellSceneEdge],
    crossings: &'a [CellCrossing],
    shared_segments: &'a [CellSharedSegment],
    junctions: &'a [CellJunction],
    parallel_edges: &'a [CellParallelEdges],
    movement: &'a CellSceneMovement,
}

fn scene_metrics(
    raw: &AdapterOutput,
    bounds: CellRect,
    parts: SceneMetricsParts<'_>,
) -> CellSceneMetrics {
    let degraded_edges = parts
        .edges
        .iter()
        .filter(|edge| !edge.degradations.is_empty())
        .count();
    let bends = parts
        .edges
        .iter()
        .map(|edge| route_bends(&edge.points))
        .sum();
    let node_edge_hits = parts
        .edges
        .iter()
        .flat_map(|edge| &edge.degradations)
        .filter(|reason| matches!(reason, CellSceneDegradation::RouteThroughNode { .. }))
        .count();
    let area_cells = u64::from(bounds.width) * u64::from(bounds.height);
    CellSceneMetrics {
        crossings: parts.crossings.len(),
        bends,
        shared_segments: parts.shared_segments.len(),
        junctions: parts.junctions.len(),
        parallel_groups: parts.parallel_edges.len(),
        routing_waypoints: raw.routing_waypoints,
        node_edge_hits,
        node_overlaps: raw.metrics.node_overlaps,
        endpoint_errors: raw.metrics.endpoint_errors,
        degraded_edges,
        area_cells,
        quality_score: quality_score(
            parts.crossings.len(),
            node_edge_hits,
            bends,
            area_cells,
            degraded_edges,
            movement_distance(parts.movement),
        ),
    }
}

fn route_bends(points: &[CellPoint]) -> usize {
    points
        .windows(3)
        .filter(|window| {
            let first_vertical = window[0].x == window[1].x;
            let second_vertical = window[1].x == window[2].x;
            first_vertical != second_vertical
        })
        .count()
}

fn validate_node_separation(nodes: &[CellSceneNode]) -> Result<(), CellSceneDegradation> {
    for (index, left) in nodes.iter().enumerate() {
        if let Some(right) = nodes[index + 1..]
            .iter()
            .find(|right| left.bounds.overlaps(right.bounds))
        {
            return Err(CellSceneDegradation::NodeOverlap {
                other: right.identity.clone(),
            });
        }
    }
    Ok(())
}

fn validate_node_bounds(
    nodes: &[CellSceneNode],
    bounds: CellRect,
) -> Result<(), CellSceneDegradation> {
    if nodes.iter().any(|node| {
        node.bounds.x < bounds.x
            || node.bounds.y < bounds.y
            || node.bounds.right() > bounds.right()
            || node.bounds.bottom() > bounds.bottom()
    }) {
        Err(CellSceneDegradation::OutOfBounds)
    } else {
        Ok(())
    }
}

fn quantize_edge(
    identity: CellEdgeIdentity,
    disposition: GraphExplorerEdgeDisposition,
    feedback: CellFeedbackKind,
    route: Option<&graphlay_adapter::AdapterRoute>,
    nodes: &[CellSceneNode],
    bounds: CellRect,
    direction: CellSceneDirection,
) -> CellSceneEdge {
    let mut degradations = Vec::new();
    let Some(route) = route else {
        degradations.push(CellSceneDegradation::MissingRoute);
        return CellSceneEdge {
            identity,
            disposition,
            feedback,
            kind: None,
            points: Vec::new(),
            source_port: None,
            target_port: None,
            arrowhead: None,
            degradations,
        };
    };
    let mut points = route.points.clone();
    points.dedup();
    repair_route_around_nodes(&mut points, &identity, nodes, bounds, direction, feedback);
    points.dedup();
    if points.len() < 2 {
        degradations.push(CellSceneDegradation::CollapsedRoute);
    }
    if points
        .windows(2)
        .any(|segment| segment[0].x != segment[1].x && segment[0].y != segment[1].y)
    {
        degradations.push(CellSceneDegradation::NonOrthogonalSegment);
    }
    if feedback == CellFeedbackKind::None && !monotone(&points, direction) {
        degradations.push(CellSceneDegradation::NonMonotoneForwardRoute);
    }
    let source_node = nodes.iter().find(|node| node.identity == identity.from);
    let target_node = nodes.iter().find(|node| node.identity == identity.to);
    let source_port = route.source_port.map(port_from_adapter);
    let target_port = route.target_port.map(port_from_adapter);
    if !source_node.zip(source_port).is_some_and(|(node, port)| {
        port_on_bounds(port, node.bounds) && points.first().copied() == Some(port.point)
    }) {
        degradations.push(CellSceneDegradation::InvalidSourcePort);
    }
    if !target_node.zip(target_port).is_some_and(|(node, port)| {
        port_on_bounds(port, node.bounds) && points.last().copied() == Some(port.point)
    }) {
        degradations.push(CellSceneDegradation::InvalidTargetPort);
    }
    if points.iter().any(|point| !point_in_bounds(*point, bounds)) {
        degradations.push(CellSceneDegradation::OutOfBounds);
    }
    for node in nodes
        .iter()
        .filter(|node| node.identity != identity.from && node.identity != identity.to)
    {
        if route_hits_rect(&points, node.bounds) {
            degradations.push(CellSceneDegradation::RouteThroughNode {
                node: node.identity.clone(),
            });
        }
    }
    let kind = match route.kind {
        AdapterRouteKind::Direct => Some(CellRouteKind::Direct),
        AdapterRouteKind::Chained { spans } => Some(CellRouteKind::Chained { spans }),
        AdapterRouteKind::SelfLoop => Some(CellRouteKind::SelfLoop),
        AdapterRouteKind::Bundled { lane } => Some(CellRouteKind::Bundled { lane }),
        AdapterRouteKind::Unrouted => {
            degradations.push(CellSceneDegradation::MissingRoute);
            None
        }
    };
    if !degradations.is_empty() {
        return CellSceneEdge {
            identity,
            disposition,
            feedback,
            kind: None,
            points: Vec::new(),
            source_port: None,
            target_port: None,
            arrowhead: None,
            degradations,
        };
    }
    CellSceneEdge {
        identity,
        disposition,
        feedback,
        kind,
        arrowhead: points.last().copied(),
        points,
        source_port,
        target_port,
        degradations,
    }
}

fn quantize_bands(
    request: &CellSceneRequest,
    nodes: &[CellSceneNode],
    raw: &AdapterOutput,
    scene: CellRect,
) -> Vec<CellSceneBand> {
    request
        .projection
        .graph
        .phase_bands
        .iter()
        .enumerate()
        .map(|(index, band)| {
            if band.nodes.is_empty() {
                return CellSceneBand {
                    phase: band.phase,
                    label: format!("Phase {}", band.phase),
                    label_anchor: None,
                    nodes: Vec::new(),
                    bounds: None,
                    degradation: Some(CellSceneDegradation::EmptyPhaseBand),
                };
            }
            let group = u32::try_from(index).unwrap_or(u32::MAX);
            let kernel_lane = raw
                .lanes
                .iter()
                .find(|lane| lane.group == Some(group))
                .map(|lane| lane.bounds);
            // The semantic node boxes are authoritative after cell
            // quantization. A floating kernel lane can round inward at either
            // edge, so union it with the exact member extent before using it
            // as a containment boundary. Falling back to member extent also
            // keeps grouping truthful if a kernel omits an optional lane.
            let members = bounds_of_identities(nodes, &band.nodes);
            let lane = match (kernel_lane, members) {
                (Some(kernel), Some(members)) => Some(CellRect {
                    x: kernel.x.min(members.x),
                    y: kernel.y.min(members.y),
                    width: u16::try_from(
                        kernel.right().max(members.right()) - kernel.x.min(members.x),
                    )
                    .unwrap_or(u16::MAX),
                    height: u16::try_from(
                        kernel.bottom().max(members.bottom()) - kernel.y.min(members.y),
                    )
                    .unwrap_or(u16::MAX),
                }),
                (Some(kernel), None) => Some(kernel),
                (None, members) => members,
            };
            let bounds = lane.and_then(|lane| match raw.orientation {
                AdapterOrientation::LeftToRight => Some(CellRect {
                    x: scene.x,
                    y: lane.y.max(scene.y),
                    width: scene.width,
                    height: u16::try_from(lane.bottom().min(scene.bottom()) - lane.y.max(scene.y))
                        .ok()?
                        .max(1),
                }),
                AdapterOrientation::TopToBottom => Some(CellRect {
                    x: lane.x.max(scene.x),
                    y: scene.y,
                    width: u16::try_from(lane.right().min(scene.right()) - lane.x.max(scene.x))
                        .ok()?
                        .max(1),
                    height: scene.height,
                }),
            });
            let bounds = bounds.filter(|bounds| {
                band.nodes.iter().all(|identity| {
                    nodes
                        .iter()
                        .find(|node| &node.identity == identity)
                        .is_some_and(|node| rect_contains(*bounds, node.bounds))
                })
            });
            // Both terminal painters write labels horizontally. Keeping the
            // anchor on the top border prevents a top-to-bottom band's label
            // from overwriting its left border and first card.
            let label_anchor = bounds.map(|bounds| CellPoint {
                x: bounds.x.saturating_add(1),
                y: bounds.y,
            });
            CellSceneBand {
                phase: band.phase,
                label: format!("Phase {}", band.phase),
                label_anchor,
                nodes: band.nodes.clone(),
                bounds,
                degradation: bounds
                    .is_none()
                    .then_some(CellSceneDegradation::InvalidPhaseBand),
            }
        })
        .collect()
}

fn scene_bounds(
    nodes: &[CellSceneNode],
    raw: &AdapterOutput,
) -> Result<CellRect, CellSceneDegradation> {
    let mut max_x = 1;
    let mut max_y = 1;
    for node in nodes {
        max_x = max_x.max(node.bounds.right());
        max_y = max_y.max(node.bounds.bottom());
    }
    for point in raw.edges.iter().flat_map(|edge| &edge.points) {
        max_x = max_x.max(point.x.saturating_add(CELL_PADDING));
        max_y = max_y.max(point.y.saturating_add(CELL_PADDING));
    }
    let width =
        u16::try_from(max_x.max(1)).map_err(|_| CellSceneDegradation::CellExtentExceeded {
            axis: "width".to_string(),
            cells: max_x,
        })?;
    let height =
        u16::try_from(max_y.max(1)).map_err(|_| CellSceneDegradation::CellExtentExceeded {
            axis: "height".to_string(),
            cells: max_y,
        })?;
    Ok(CellRect {
        x: 0,
        y: 0,
        width,
        height,
    })
}

fn feedback_kind(
    edge: &crate::GraphExplorerEdge,
    cycle_components: &[BTreeSet<String>],
    reversed: bool,
) -> CellFeedbackKind {
    if edge.disposition == GraphExplorerEdgeDisposition::AggregateFeedback {
        return CellFeedbackKind::AggregateQuotient;
    }
    if edge.provenance.iter().any(|dependency| {
        cycle_components.iter().any(|component| {
            component.contains(&dependency.from) && component.contains(&dependency.to)
        })
    }) {
        CellFeedbackKind::CanonicalTaskCycle
    } else if reversed {
        CellFeedbackKind::KernelCycleReversal
    } else {
        CellFeedbackKind::None
    }
}

fn invalid_cycle_components(conditions: &[GraphExplorerCondition]) -> Vec<BTreeSet<String>> {
    conditions
        .iter()
        .filter_map(|condition| match condition {
            GraphExplorerCondition::InvalidTaskCycle { components } => Some(components),
            _ => None,
        })
        .flatten()
        .map(|component| component.iter().cloned().collect())
        .collect()
}

fn movement(
    prior: Option<&CellScene>,
    nodes: &[CellSceneNode],
    edges: &[CellEdgeIdentity],
    direction: CellSceneDirection,
    ordering_policy: CellPeerOrder,
) -> CellSceneMovement {
    let Some(prior) = prior else {
        return CellSceneMovement::Fresh;
    };
    let before: BTreeMap<_, _> = prior
        .nodes
        .iter()
        .map(|node| (node.identity.clone(), node.bounds.center()))
        .collect();
    let mut moved_nodes = 0usize;
    let mut total_distance = 0u64;
    let mut max_distance = 0u32;
    for node in nodes {
        let Some(old) = before.get(&node.identity) else {
            continue;
        };
        let new = node.bounds.center();
        let distance = old.x.abs_diff(new.x).saturating_add(old.y.abs_diff(new.y));
        if distance != 0 {
            moved_nodes += 1;
            total_distance = total_distance.saturating_add(u64::from(distance));
            max_distance = max_distance.max(distance);
        }
    }
    let prior_node_ids: BTreeSet<_> = prior
        .nodes
        .iter()
        .map(|node| node.identity.clone())
        .collect();
    let node_ids: BTreeSet<_> = nodes.iter().map(|node| node.identity.clone()).collect();
    let prior_edges: BTreeSet<_> = prior
        .edges
        .iter()
        .map(|edge| edge.identity.clone())
        .collect();
    let edges: BTreeSet<_> = edges.iter().cloned().collect();
    let mut reasons = BTreeSet::new();
    if prior_node_ids != node_ids || prior_edges != edges {
        reasons.insert(CellRelayoutReason::StructuralGraphChange);
    }
    if nodes.iter().any(|node| {
        prior.nodes.iter().any(|before| {
            before.identity == node.identity
                && (before.bounds.width != node.bounds.width
                    || before.bounds.height != node.bounds.height)
        })
    }) {
        reasons.insert(CellRelayoutReason::CardSizeChange);
    }
    if prior.direction != direction {
        reasons.insert(CellRelayoutReason::OrientationChange);
    }
    if prior.ordering_policy != ordering_policy {
        reasons.insert(CellRelayoutReason::OrderingPolicyChange);
    }
    if reasons.is_empty() {
        reasons.insert(CellRelayoutReason::LayoutInputChange);
    }
    CellSceneMovement::Relayout {
        moved_nodes,
        total_distance,
        max_distance,
        orientation_changed: prior.direction != direction,
        reasons: reasons.into_iter().collect(),
    }
}

fn movement_distance(movement: &CellSceneMovement) -> u64 {
    match movement {
        CellSceneMovement::Relayout { total_distance, .. } => *total_distance,
        CellSceneMovement::Fresh | CellSceneMovement::ReusedStatusOnly => 0,
    }
}

fn focus_for(
    nodes: &[CellSceneNode],
    selected: Option<&GraphExplorerIdentity>,
    recenter: bool,
) -> Option<CellSceneFocus> {
    selected.and_then(|identity| {
        nodes
            .iter()
            .find(|node| &node.identity == identity)
            .map(|node| CellSceneFocus {
                identity: identity.clone(),
                bounds: node.bounds,
                recenter,
            })
    })
}

fn quality_score(
    crossings: usize,
    node_edge_hits: usize,
    bends: usize,
    area: u64,
    degraded_edges: usize,
    displacement: u64,
) -> u64 {
    u64::try_from(crossings)
        .unwrap_or(u64::MAX)
        .saturating_mul(1_000_000_000)
        .saturating_add(
            u64::try_from(node_edge_hits)
                .unwrap_or(u64::MAX)
                .saturating_mul(1_000_000_000),
        )
        .saturating_add(
            u64::try_from(degraded_edges)
                .unwrap_or(u64::MAX)
                .saturating_mul(100_000_000),
        )
        .saturating_add(
            u64::try_from(bends)
                .unwrap_or(u64::MAX)
                .saturating_mul(100_000),
        )
        .saturating_add(displacement.saturating_mul(1_000))
        .saturating_add(area)
}

fn geometry_key(request: &CellSceneRequest) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(format!("{:?}", request.projection.location).as_bytes());
    hasher.update(format!("{:?}", request.orientation).as_bytes());
    hasher.update(format!("{:?}", request.peer_order).as_bytes());
    hasher.update(
        format!(
            "{}:{}:{}",
            request.budget.max_nodes,
            request.budget.max_edges,
            request.budget.max_routing_waypoints
        )
        .as_bytes(),
    );
    for node in &request.projection.graph.nodes {
        hasher.update(node.identity.id().as_bytes());
        hasher.update(&node.phase.to_le_bytes());
        match request.peer_order {
            CellPeerOrder::Title => hasher.update(node.title.as_bytes()),
            CellPeerOrder::Status => hasher.update(node.status.as_str().as_bytes()),
            CellPeerOrder::Readability
            | CellPeerOrder::Identity
            | CellPeerOrder::PlanPriority
            | CellPeerOrder::Criticality => &mut hasher,
        };
        if let Some(size) = request.card_sizes.get(&node.identity) {
            hasher.update(&size.columns.to_le_bytes());
            hasher.update(&size.rows.to_le_bytes());
        }
        if let Some(hint) = request.ordering_hints.get(&node.identity) {
            hasher.update(&hint.to_le_bytes());
        }
    }
    for band in &request.projection.graph.phase_bands {
        hasher.update(&band.phase.to_le_bytes());
        for identity in &band.nodes {
            hasher.update(identity.id().as_bytes());
        }
    }
    for edge in &request.projection.graph.edges {
        hasher.update(edge.from.id().as_bytes());
        hasher.update(edge.to.id().as_bytes());
        hasher.update(&[match edge.disposition {
            GraphExplorerEdgeDisposition::Forward => 0,
            GraphExplorerEdgeDisposition::AggregateFeedback => 1,
        }]);
        for provenance in &edge.provenance {
            hasher.update(provenance.from.as_bytes());
            hasher.update(provenance.to.as_bytes());
        }
    }
    hasher.finalize().to_hex().to_string()
}

fn fallback(
    generation: u64,
    request: &CellSceneRequest,
    mut reasons: Vec<CellSceneDegradation>,
) -> CellSceneResult {
    if reasons.is_empty() {
        reasons.push(CellSceneDegradation::KernelRejected);
    }
    let reason = reasons[0].clone();
    let edge_identities = edge_identities(&request.projection);
    CellSceneResult::Fallback(Box::new(CellSceneFallback {
        generation,
        input: request.projection.input.clone(),
        location: request.projection.location.clone(),
        reasons,
        nodes: request
            .projection
            .graph
            .nodes
            .iter()
            .map(|node| CellNodeOmission {
                identity: node.identity.clone(),
                reason: reason.clone(),
            })
            .collect(),
        edges: edge_identities
            .into_iter()
            .map(|identity| CellEdgeOmission {
                identity,
                reason: reason.clone(),
            })
            .collect(),
        retained: request.prior.clone(),
    }))
}

fn edge_identities(projection: &GraphExplorerProjection) -> Vec<CellEdgeIdentity> {
    let mut ordinals = BTreeMap::<(GraphExplorerIdentity, GraphExplorerIdentity), u32>::new();
    projection
        .graph
        .edges
        .iter()
        .map(|edge| {
            let ordinal = ordinals
                .entry((edge.from.clone(), edge.to.clone()))
                .or_default();
            let identity = CellEdgeIdentity {
                from: edge.from.clone(),
                to: edge.to.clone(),
                ordinal: *ordinal,
                provenance: edge.provenance.clone(),
            };
            *ordinal = ordinal.saturating_add(1);
            identity
        })
        .collect()
}

fn direction_for(orientation: CellSceneOrientation) -> Option<CellSceneDirection> {
    match orientation {
        CellSceneOrientation::Auto => None,
        CellSceneOrientation::LeftToRight => Some(CellSceneDirection::LeftToRight),
        CellSceneOrientation::TopToBottom => Some(CellSceneDirection::TopToBottom),
    }
}

fn grapheme_wrapped_rows(text: &str, max_columns: usize) -> u16 {
    if text.is_empty() {
        return 1;
    }
    let mut rows = 1u16;
    let mut used = 0usize;
    for grapheme in text.graphemes(true) {
        if is_line_break(grapheme) {
            rows = rows.saturating_add(1);
            used = 0;
            continue;
        }
        let width = grapheme.width();
        if used != 0 && used.saturating_add(width) > max_columns {
            rows = rows.saturating_add(1);
            used = 0;
        }
        used = used.saturating_add(width);
    }
    rows
}

fn widest_grapheme_line(text: &str, max_columns: usize) -> usize {
    let mut widest = 0usize;
    let mut used = 0usize;
    for grapheme in text.graphemes(true) {
        if is_line_break(grapheme) {
            widest = widest.max(used);
            used = 0;
            continue;
        }
        let width = grapheme.width();
        if used != 0 && used.saturating_add(width) > max_columns {
            widest = widest.max(used);
            used = 0;
        }
        used = used.saturating_add(width);
    }
    widest.max(used)
}

fn is_line_break(grapheme: &str) -> bool {
    matches!(grapheme, "\n" | "\r" | "\r\n")
}

fn port_from_adapter(port: graphlay_adapter::AdapterPort) -> CellPort {
    CellPort {
        point: port.point,
        side: match port.side {
            AdapterSide::Left => CellPortSide::Left,
            AdapterSide::Right => CellPortSide::Right,
            AdapterSide::Top => CellPortSide::Top,
            AdapterSide::Bottom => CellPortSide::Bottom,
        },
    }
}

fn port_on_bounds(port: CellPort, bounds: CellRect) -> bool {
    match port.side {
        CellPortSide::Left => {
            port.point.x == bounds.x && (bounds.y..=bounds.bottom()).contains(&port.point.y)
        }
        CellPortSide::Right => {
            port.point.x == bounds.right() && (bounds.y..=bounds.bottom()).contains(&port.point.y)
        }
        CellPortSide::Top => {
            port.point.y == bounds.y && (bounds.x..=bounds.right()).contains(&port.point.x)
        }
        CellPortSide::Bottom => {
            port.point.y == bounds.bottom() && (bounds.x..=bounds.right()).contains(&port.point.x)
        }
    }
}

fn point_in_bounds(point: CellPoint, bounds: CellRect) -> bool {
    (bounds.x..=bounds.right()).contains(&point.x)
        && (bounds.y..=bounds.bottom()).contains(&point.y)
}

fn monotone(points: &[CellPoint], direction: CellSceneDirection) -> bool {
    points.windows(2).all(|segment| match direction {
        CellSceneDirection::LeftToRight => segment[0].x <= segment[1].x,
        CellSceneDirection::TopToBottom => segment[0].y <= segment[1].y,
    })
}

fn route_hits_rect(points: &[CellPoint], rect: CellRect) -> bool {
    points.windows(2).any(|segment| {
        let [a, b] = [segment[0], segment[1]];
        if a.x == b.x {
            let (start, end) = ordered(a.y, b.y);
            a.x >= rect.x && a.x <= rect.right() && start <= rect.bottom() && end >= rect.y
        } else if a.y == b.y {
            let (start, end) = ordered(a.x, b.x);
            a.y >= rect.y && a.y <= rect.bottom() && start <= rect.right() && end >= rect.x
        } else {
            false
        }
    })
}

fn repair_route_around_nodes(
    points: &mut Vec<CellPoint>,
    identity: &CellEdgeIdentity,
    nodes: &[CellSceneNode],
    bounds: CellRect,
    direction: CellSceneDirection,
    feedback: CellFeedbackKind,
) {
    let obstacles: Vec<_> = nodes
        .iter()
        .filter(|node| node.identity != identity.from && node.identity != identity.to)
        .map(|node| node.bounds)
        .collect();
    let mut collisions = route_collision_count(points, &obstacles);
    let limit = points.len().saturating_mul(obstacles.len().max(1));
    for _ in 0..limit {
        if collisions == 0 || points.len() < 2 {
            break;
        }
        let best = best_route_repair(points, &obstacles, bounds, direction, feedback, collisions);
        let Some((remaining, _, _, _, repaired)) = best else {
            break;
        };
        *points = repaired;
        collisions = remaining;
    }
}

type RouteRepair = (usize, usize, u32, i32, Vec<CellPoint>);

fn best_route_repair(
    points: &[CellPoint],
    obstacles: &[CellRect],
    bounds: CellRect,
    direction: CellSceneDirection,
    feedback: CellFeedbackKind,
    collisions: usize,
) -> Option<RouteRepair> {
    let mut best = None;
    for index in 0..points.len().saturating_sub(1) {
        for coordinate in detour_coordinates(points[index], points[index + 1], obstacles) {
            let displacement = if points[index].x == points[index + 1].x {
                points[index].x.abs_diff(coordinate)
            } else {
                points[index].y.abs_diff(coordinate)
            };
            for mut candidate in route_repair_variants(points, index, coordinate) {
                candidate.dedup();
                if !valid_route_repair(&candidate, bounds, direction, feedback) {
                    continue;
                }
                let remaining = route_collision_count(&candidate, obstacles);
                if remaining >= collisions {
                    continue;
                }
                let score = (
                    remaining,
                    candidate.len(),
                    displacement,
                    coordinate,
                    candidate,
                );
                if best.as_ref().is_none_or(|current| score < *current) {
                    best = Some(score);
                }
            }
        }
    }
    best
}

fn detour_coordinates(a: CellPoint, b: CellPoint, obstacles: &[CellRect]) -> BTreeSet<i32> {
    if a.x == b.x {
        obstacles
            .iter()
            .filter(|rect| route_hits_rect(&[a, b], **rect))
            .flat_map(|rect| [rect.x.saturating_sub(1), rect.right().saturating_add(1)])
            .collect()
    } else if a.y == b.y {
        obstacles
            .iter()
            .filter(|rect| route_hits_rect(&[a, b], **rect))
            .flat_map(|rect| [rect.y.saturating_sub(1), rect.bottom().saturating_add(1)])
            .collect()
    } else {
        BTreeSet::new()
    }
}

fn route_repair_variants(
    points: &[CellPoint],
    index: usize,
    coordinate: i32,
) -> Vec<Vec<CellPoint>> {
    let (a, b) = (points[index], points[index + 1]);
    let vertical = a.x == b.x;
    let detour = if vertical {
        [
            CellPoint {
                x: coordinate,
                y: a.y,
            },
            CellPoint {
                x: coordinate,
                y: b.y,
            },
        ]
    } else {
        [
            CellPoint {
                x: a.x,
                y: coordinate,
            },
            CellPoint {
                x: b.x,
                y: coordinate,
            },
        ]
    };
    let mut variants = Vec::with_capacity(2);
    if index > 0 && index + 1 < points.len() - 1 {
        let mut shifted = points.to_vec();
        if vertical {
            shifted[index].x = coordinate;
            shifted[index + 1].x = coordinate;
        } else {
            shifted[index].y = coordinate;
            shifted[index + 1].y = coordinate;
        }
        variants.push(shifted);
    }
    let mut diverted = Vec::with_capacity(points.len().saturating_add(2));
    diverted.extend_from_slice(&points[..=index]);
    diverted.extend(detour);
    diverted.extend_from_slice(&points[index + 1..]);
    variants.push(diverted);
    variants
}

fn valid_route_repair(
    points: &[CellPoint],
    bounds: CellRect,
    direction: CellSceneDirection,
    feedback: CellFeedbackKind,
) -> bool {
    points.len() >= 2
        && points.iter().all(|point| point_in_bounds(*point, bounds))
        && points
            .windows(2)
            .all(|segment| segment[0].x == segment[1].x || segment[0].y == segment[1].y)
        && (feedback != CellFeedbackKind::None || monotone(points, direction))
}

fn route_collision_count(points: &[CellPoint], obstacles: &[CellRect]) -> usize {
    obstacles
        .iter()
        .filter(|rect| route_hits_rect(points, **rect))
        .count()
}

fn ordered(left: i32, right: i32) -> (i32, i32) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn bounds_of_identities(
    nodes: &[CellSceneNode],
    identities: &[GraphExplorerIdentity],
) -> Option<CellRect> {
    let selected: Vec<_> = nodes
        .iter()
        .filter(|node| identities.contains(&node.identity))
        .map(|node| node.bounds)
        .collect();
    let first = *selected.first()?;
    let min_x = selected.iter().map(|rect| rect.x).min().unwrap_or(first.x);
    let min_y = selected.iter().map(|rect| rect.y).min().unwrap_or(first.y);
    let max_x = selected
        .iter()
        .map(|rect| rect.right())
        .max()
        .unwrap_or(first.right());
    let max_y = selected
        .iter()
        .map(|rect| rect.bottom())
        .max()
        .unwrap_or(first.bottom());
    Some(CellRect {
        x: min_x,
        y: min_y,
        width: u16::try_from(max_x - min_x).unwrap_or(u16::MAX),
        height: u16::try_from(max_y - min_y).unwrap_or(u16::MAX),
    })
}

fn rect_contains(outer: CellRect, inner: CellRect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

fn shared_segments(edges: &[CellSceneEdge]) -> Vec<CellSharedSegment> {
    let mut owners = BTreeMap::<(CellPoint, CellPoint), BTreeSet<CellEdgeIdentity>>::new();
    for (left_index, left) in edges.iter().enumerate() {
        for right in &edges[left_index + 1..] {
            for left_segment in left.points.windows(2) {
                for right_segment in right.points.windows(2) {
                    if let Some(key) = collinear_overlap(left_segment, right_segment) {
                        let identities = owners.entry(key).or_default();
                        identities.insert(left.identity.clone());
                        identities.insert(right.identity.clone());
                    }
                }
            }
        }
    }
    owners
        .into_iter()
        .map(|((from, to), edges)| CellSharedSegment {
            from,
            to,
            edges: edges.into_iter().collect(),
        })
        .collect()
}

fn collinear_overlap(left: &[CellPoint], right: &[CellPoint]) -> Option<(CellPoint, CellPoint)> {
    if left[0].x == left[1].x && right[0].x == right[1].x && left[0].x == right[0].x {
        let (left_start, left_end) = ordered(left[0].y, left[1].y);
        let (right_start, right_end) = ordered(right[0].y, right[1].y);
        let start = left_start.max(right_start);
        let end = left_end.min(right_end);
        (start < end).then_some((
            CellPoint {
                x: left[0].x,
                y: start,
            },
            CellPoint {
                x: left[0].x,
                y: end,
            },
        ))
    } else if left[0].y == left[1].y && right[0].y == right[1].y && left[0].y == right[0].y {
        let (left_start, left_end) = ordered(left[0].x, left[1].x);
        let (right_start, right_end) = ordered(right[0].x, right[1].x);
        let start = left_start.max(right_start);
        let end = left_end.min(right_end);
        (start < end).then_some((
            CellPoint {
                x: start,
                y: left[0].y,
            },
            CellPoint {
                x: end,
                y: left[0].y,
            },
        ))
    } else {
        None
    }
}

fn crossings(edges: &[CellSceneEdge]) -> Vec<CellCrossing> {
    let mut found = Vec::new();
    for (left_index, left) in edges.iter().enumerate() {
        for right in &edges[left_index + 1..] {
            for left_segment in left.points.windows(2) {
                for right_segment in right.points.windows(2) {
                    if let Some(at) = orthogonal_crossing(left_segment, right_segment) {
                        found.push(CellCrossing {
                            at,
                            first: left.identity.clone(),
                            second: right.identity.clone(),
                        });
                    }
                }
            }
        }
    }
    found.sort_by(|left, right| {
        left.at
            .cmp(&right.at)
            .then(left.first.cmp(&right.first))
            .then(left.second.cmp(&right.second))
    });
    found.dedup();
    found
}

fn orthogonal_crossing(left: &[CellPoint], right: &[CellPoint]) -> Option<CellPoint> {
    let (horizontal, vertical) = if left[0].y == left[1].y && right[0].x == right[1].x {
        (left, right)
    } else if right[0].y == right[1].y && left[0].x == left[1].x {
        (right, left)
    } else {
        return None;
    };
    let (x0, x1) = ordered(horizontal[0].x, horizontal[1].x);
    let (y0, y1) = ordered(vertical[0].y, vertical[1].y);
    let point = CellPoint {
        x: vertical[0].x,
        y: horizontal[0].y,
    };
    (point.x > x0 && point.x < x1 && point.y > y0 && point.y < y1).then_some(point)
}

fn junctions(raw: &AdapterOutput, edges: &[CellSceneEdge]) -> Vec<CellJunction> {
    raw.junctions
        .iter()
        .copied()
        .filter_map(|at| {
            let edges: Vec<_> = edges
                .iter()
                .filter(|edge| {
                    edge.points.contains(&at)
                        || edge
                            .points
                            .windows(2)
                            .any(|segment| point_on_segment(at, segment[0], segment[1]))
                })
                .map(|edge| edge.identity.clone())
                .collect();
            (edges.len() > 1).then_some(CellJunction { at, edges })
        })
        .collect()
}

fn point_on_segment(point: CellPoint, from: CellPoint, to: CellPoint) -> bool {
    if from.x == to.x && point.x == from.x {
        let (start, end) = ordered(from.y, to.y);
        (start..=end).contains(&point.y)
    } else if from.y == to.y && point.y == from.y {
        let (start, end) = ordered(from.x, to.x);
        (start..=end).contains(&point.x)
    } else {
        false
    }
}

fn parallel_edges(edges: &[CellSceneEdge]) -> Vec<CellParallelEdges> {
    let mut groups =
        BTreeMap::<(GraphExplorerIdentity, GraphExplorerIdentity), Vec<CellEdgeIdentity>>::new();
    for edge in edges {
        groups
            .entry((edge.identity.from.clone(), edge.identity.to.clone()))
            .or_default()
            .push(edge.identity.clone());
    }
    groups
        .into_iter()
        .filter(|(_, edges)| edges.len() > 1)
        .map(|((from, to), edges)| CellParallelEdges { from, to, edges })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_card_measurement_names_every_status_at_both_densities() {
        for status in [
            Status::Todo,
            Status::InProgress,
            Status::Blocked,
            Status::Done,
            Status::Dropped,
        ] {
            let node = GraphExplorerNode {
                identity: GraphExplorerIdentity::Task(status.as_str().to_string()),
                title: "status card".to_string(),
                phase: 0,
                milestone: None,
                status,
                checkpoint_in_scope: false,
                represented_ids: vec![status.as_str().to_string()],
                member_ids: Vec::new(),
                member_phases: vec![0],
                facts: crate::GraphExplorerFacts::default(),
                internal_dependencies: Vec::new(),
            };
            assert!(CellCardSize::for_density(&node, CellSceneDensity::Compact).columns >= 6);
            assert!(
                usize::from(
                    CellCardSize::for_density(&node, CellSceneDensity::Comfortable).columns
                ) >= status.as_str().len()
            );
        }
        assert_eq!(
            [
                status_rank(Status::InProgress),
                status_rank(Status::Todo),
                status_rank(Status::Blocked),
                status_rank(Status::Done),
                status_rank(Status::Dropped),
            ],
            [0, 1, 2, 3, 4]
        );
    }

    #[test]
    fn grapheme_measurement_keeps_combining_and_wide_clusters_whole() {
        assert_eq!(grapheme_wrapped_rows("e\u{301}e\u{301}", 1), 2);
        assert_eq!(grapheme_wrapped_rows("\u{1f642}\u{1f642}", 2), 2);
        assert_eq!(widest_grapheme_line("\u{1f642}a", 2), 2);
        assert_eq!(grapheme_wrapped_rows("one\r\ntwo\n", 20), 3);
        assert_eq!(widest_grapheme_line("one\r\nlonger", 20), 6);
    }

    #[test]
    fn auto_hysteresis_has_an_exact_deterministic_boundary() {
        assert!(within_auto_hysteresis(100, 112));
        assert!(!within_auto_hysteresis(100, 113));
        assert!(within_auto_hysteresis(0, 0));
        assert!(!within_auto_hysteresis(0, 1));
    }

    #[test]
    fn stale_completion_is_refused_by_generation() {
        let completion = CellSceneCompletion {
            generation: 3,
            result: CellSceneResult::Fallback(Box::new(CellSceneFallback {
                generation: 3,
                input: test_identity(),
                location: GraphExplorerLocation::Plan,
                reasons: vec![CellSceneDegradation::EmptyProjection],
                nodes: Vec::new(),
                edges: Vec::new(),
                retained: None,
            })),
        };
        assert_eq!(
            completion.into_current(4),
            Err(StaleCellSceneCompletion {
                expected: 4,
                completed: 3,
            })
        );
    }

    fn unsafe_route_fixture() -> (
        CellEdgeIdentity,
        Vec<CellSceneNode>,
        graphlay_adapter::AdapterRoute,
        GraphExplorerIdentity,
    ) {
        let source = GraphExplorerIdentity::Task("A".to_string());
        let target = GraphExplorerIdentity::Task("B".to_string());
        let obstruction = GraphExplorerIdentity::Task("X".to_string());
        let nodes = vec![
            CellSceneNode {
                identity: source.clone(),
                bounds: CellRect {
                    x: 1,
                    y: 1,
                    width: 4,
                    height: 3,
                },
                rank: Some(0),
                order: 0,
            },
            CellSceneNode {
                identity: target.clone(),
                bounds: CellRect {
                    x: 10,
                    y: 1,
                    width: 4,
                    height: 3,
                },
                rank: Some(1),
                order: 0,
            },
            CellSceneNode {
                identity: obstruction.clone(),
                bounds: CellRect {
                    x: 7,
                    y: 0,
                    width: 2,
                    height: 4,
                },
                rank: Some(0),
                order: 1,
            },
        ];
        let identity = CellEdgeIdentity {
            from: source,
            to: target,
            ordinal: 0,
            provenance: vec![GraphExplorerDependency {
                from: "A".to_string(),
                to: "B".to_string(),
            }],
        };
        let route = graphlay_adapter::AdapterRoute {
            points: vec![
                CellPoint { x: 5, y: 2 },
                CellPoint { x: 4, y: 2 },
                CellPoint { x: 8, y: 3 },
                CellPoint { x: 15, y: 3 },
            ],
            source_port: Some(graphlay_adapter::AdapterPort {
                point: CellPoint { x: 4, y: 2 },
                side: AdapterSide::Left,
            }),
            target_port: Some(graphlay_adapter::AdapterPort {
                point: CellPoint { x: 15, y: 3 },
                side: AdapterSide::Right,
            }),
            kind: AdapterRouteKind::Direct,
        };
        (identity, nodes, route, obstruction)
    }

    #[test]
    fn quantization_names_every_unsafe_route_condition() {
        let (identity, nodes, route, obstruction) = unsafe_route_fixture();
        let edge = quantize_edge(
            identity,
            GraphExplorerEdgeDisposition::Forward,
            CellFeedbackKind::None,
            Some(&route),
            &nodes,
            CellRect {
                x: 0,
                y: 0,
                width: 14,
                height: 6,
            },
            CellSceneDirection::LeftToRight,
        );
        assert!(
            edge.degradations
                .contains(&CellSceneDegradation::NonOrthogonalSegment)
        );
        assert!(
            edge.degradations
                .contains(&CellSceneDegradation::NonMonotoneForwardRoute)
        );
        assert!(
            edge.degradations
                .contains(&CellSceneDegradation::InvalidSourcePort)
        );
        assert!(
            edge.degradations
                .contains(&CellSceneDegradation::InvalidTargetPort)
        );
        assert!(
            edge.degradations
                .contains(&CellSceneDegradation::OutOfBounds)
        );
        assert!(
            edge.degradations
                .contains(&CellSceneDegradation::RouteThroughNode { node: obstruction })
        );
        assert!(edge.points.is_empty());
        assert!(edge.source_port.is_none());
        assert!(edge.target_port.is_none());
        assert!(edge.arrowhead.is_none());

        let collapsed = graphlay_adapter::AdapterRoute {
            points: vec![CellPoint { x: 5, y: 2 }, CellPoint { x: 5, y: 2 }],
            source_port: None,
            target_port: None,
            kind: AdapterRouteKind::Unrouted,
        };
        let collapsed = quantize_edge(
            edge.identity,
            GraphExplorerEdgeDisposition::Forward,
            CellFeedbackKind::None,
            Some(&collapsed),
            &nodes,
            CellRect {
                x: 0,
                y: 0,
                width: 14,
                height: 6,
            },
            CellSceneDirection::LeftToRight,
        );
        assert!(
            collapsed
                .degradations
                .contains(&CellSceneDegradation::CollapsedRoute)
        );
        assert!(
            collapsed
                .degradations
                .contains(&CellSceneDegradation::MissingRoute)
        );
        assert!(collapsed.points.is_empty());
    }

    /// Proof for `graph.layered_scene_is_complete_or_fallback`: a route which
    /// cannot be drawn never survives inside a scene that callers may paint as
    /// complete, while an intentionally empty phase remains valid context.
    #[test]
    fn complete_scene_contract_rejects_degraded_geometry() {
        let (identity, nodes, route, _) = unsafe_route_fixture();
        let degraded = quantize_edge(
            identity,
            GraphExplorerEdgeDisposition::Forward,
            CellFeedbackKind::None,
            Some(&route),
            &nodes,
            CellRect {
                x: 0,
                y: 0,
                width: 14,
                height: 6,
            },
            CellSceneDirection::LeftToRight,
        );
        assert_eq!(
            ensure_complete_geometry(&[degraded], &[]),
            Err(CellSceneDegradation::NonOrthogonalSegment)
        );
        assert_eq!(
            ensure_complete_geometry(
                &[],
                &[CellSceneBand {
                    phase: 7,
                    label: "Phase 7".to_string(),
                    label_anchor: None,
                    nodes: Vec::new(),
                    bounds: None,
                    degradation: Some(CellSceneDegradation::EmptyPhaseBand),
                }]
            ),
            Ok(())
        );
        assert_eq!(
            ensure_complete_geometry(
                &[],
                &[CellSceneBand {
                    phase: 7,
                    label: "Phase 7".to_string(),
                    label_anchor: None,
                    nodes: Vec::new(),
                    bounds: None,
                    degradation: Some(CellSceneDegradation::InvalidPhaseBand),
                }]
            ),
            Err(CellSceneDegradation::InvalidPhaseBand)
        );
    }

    #[test]
    fn oversized_cell_extent_is_refused_instead_of_saturating() {
        let nodes = vec![CellSceneNode {
            identity: GraphExplorerIdentity::Task("wide".to_string()),
            bounds: CellRect {
                x: 65_530,
                y: 1,
                width: 10,
                height: 4,
            },
            rank: Some(0),
            order: 0,
        }];
        let raw = AdapterOutput {
            orientation: AdapterOrientation::LeftToRight,
            nodes: Vec::new(),
            edges: Vec::new(),
            reversed: Vec::new(),
            routing_waypoints: 0,
            junctions: Vec::new(),
            lanes: Vec::new(),
            metrics: graphlay_adapter::AdapterMetrics::default(),
        };
        assert_eq!(
            scene_bounds(&nodes, &raw),
            Err(CellSceneDegradation::CellExtentExceeded {
                axis: "width".to_string(),
                cells: 65_540,
            })
        );
    }

    #[test]
    fn partially_overlapping_route_trunks_are_reported_with_exact_edges() {
        let identity = |to: &str| CellEdgeIdentity {
            from: GraphExplorerIdentity::Task("source".to_string()),
            to: GraphExplorerIdentity::Task(to.to_string()),
            ordinal: 0,
            provenance: vec![GraphExplorerDependency {
                from: "source".to_string(),
                to: to.to_string(),
            }],
        };
        let first = identity("first");
        let second = identity("second");
        let edge = |identity, points| CellSceneEdge {
            identity,
            disposition: GraphExplorerEdgeDisposition::Forward,
            feedback: CellFeedbackKind::None,
            kind: Some(CellRouteKind::Direct),
            points,
            source_port: None,
            target_port: None,
            arrowhead: None,
            degradations: Vec::new(),
        };
        assert_eq!(
            shared_segments(&[
                edge(
                    first.clone(),
                    vec![CellPoint { x: 2, y: 5 }, CellPoint { x: 12, y: 5 }],
                ),
                edge(
                    second.clone(),
                    vec![CellPoint { x: 7, y: 5 }, CellPoint { x: 15, y: 5 }],
                ),
            ]),
            vec![CellSharedSegment {
                from: CellPoint { x: 7, y: 5 },
                to: CellPoint { x: 12, y: 5 },
                edges: vec![first, second],
            }]
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn static_adapter_preserves_typed_nodes_edges_ranks_groups_and_omissions() {
        let a = GraphExplorerIdentity::Task("A".to_string());
        let b = GraphExplorerIdentity::Task("B".to_string());
        let dependency = GraphExplorerDependency {
            from: "A".to_string(),
            to: "B".to_string(),
        };
        let semantic_node = |identity: GraphExplorerIdentity, phase, status| GraphExplorerNode {
            title: format!("{} title", identity.id()),
            represented_ids: vec![identity.id().to_string()],
            identity,
            phase,
            milestone: None,
            status,
            checkpoint_in_scope: true,
            member_ids: Vec::new(),
            member_phases: vec![phase],
            facts: crate::GraphExplorerFacts {
                tasks: 1,
                ready: 1,
                ..crate::GraphExplorerFacts::default()
            },
            internal_dependencies: Vec::new(),
        };
        let projection = GraphExplorerProjection {
            input: test_identity(),
            location: GraphExplorerLocation::Phase(1),
            state: GraphExplorerState::Complete,
            graph: crate::GraphExplorerGraph {
                nodes: vec![
                    semantic_node(a.clone(), 1, Status::Todo),
                    semantic_node(b.clone(), 1, Status::Done),
                ],
                edges: vec![crate::GraphExplorerEdge {
                    from: a.clone(),
                    to: b.clone(),
                    disposition: GraphExplorerEdgeDisposition::Forward,
                    dependency_count: 1,
                    provenance: vec![dependency.clone()],
                }],
                phase_bands: vec![crate::GraphExplorerPhaseBand {
                    phase: 1,
                    nodes: vec![a.clone(), b.clone()],
                    facts: crate::GraphExplorerFacts::default(),
                }],
            },
            conditions: Vec::new(),
            selection: None,
            no_selection_reason: None,
            phase_context: None,
            semantic_window: None,
            boundaries: Vec::new(),
            accounting: crate::GraphExplorerAccounting {
                admitted_task_ids: vec!["A".to_string(), "B".to_string(), "C".to_string()],
                level_task_ids: vec!["A".to_string(), "B".to_string(), "C".to_string()],
                visible_task_ids: vec!["A".to_string(), "B".to_string()],
                level_omitted_task_ids: Vec::new(),
                window_omitted_task_ids: vec!["C".to_string()],
                admitted_dependency_edges: 2,
                represented_dependency_edges: 1,
                boundary_dependency_edges: 1,
                boundary_dependencies: vec![GraphExplorerDependency {
                    from: "B".to_string(),
                    to: "C".to_string(),
                }],
                grouping: aplan_output::Grouping::default(),
                source_scope: Vec::new(),
            },
            critical_path: vec!["A".to_string(), "B".to_string()],
            search_matches: Vec::new(),
            transitions: Vec::new(),
        };
        let rect_a = CellRect {
            x: 1,
            y: 2,
            width: 8,
            height: 4,
        };
        let rect_b = CellRect {
            x: 20,
            y: 2,
            width: 8,
            height: 4,
        };
        let edge_identity = CellEdgeIdentity {
            from: a.clone(),
            to: b.clone(),
            ordinal: 3,
            provenance: vec![dependency],
        };
        let scene = CellScene {
            generation: 7,
            input: test_identity(),
            location: GraphExplorerLocation::Phase(1),
            geometry_key: "geometry".to_string(),
            direction: CellSceneDirection::LeftToRight,
            ordering_policy: CellPeerOrder::Title,
            bounds: CellRect {
                x: 0,
                y: 0,
                width: 30,
                height: 8,
            },
            nodes: vec![
                CellSceneNode {
                    identity: a.clone(),
                    bounds: rect_a,
                    rank: Some(0),
                    order: 1,
                },
                CellSceneNode {
                    identity: b.clone(),
                    bounds: rect_b,
                    rank: Some(1),
                    order: 0,
                },
            ],
            edges: vec![CellSceneEdge {
                identity: edge_identity,
                disposition: GraphExplorerEdgeDisposition::Forward,
                feedback: CellFeedbackKind::None,
                kind: Some(CellRouteKind::Direct),
                points: vec![CellPoint { x: 9, y: 4 }, CellPoint { x: 19, y: 4 }],
                source_port: Some(CellPort {
                    point: CellPoint { x: 9, y: 4 },
                    side: CellPortSide::Right,
                }),
                target_port: Some(CellPort {
                    point: CellPoint { x: 19, y: 4 },
                    side: CellPortSide::Left,
                }),
                arrowhead: Some(CellPoint { x: 19, y: 4 }),
                degradations: Vec::new(),
            }],
            bands: vec![CellSceneBand {
                phase: 1,
                label: "Phase 1".to_string(),
                label_anchor: Some(CellPoint { x: 0, y: 0 }),
                nodes: vec![a.clone(), b.clone()],
                bounds: Some(CellRect {
                    x: 0,
                    y: 0,
                    width: 30,
                    height: 8,
                }),
                degradation: None,
            }],
            crossings: Vec::new(),
            shared_segments: Vec::new(),
            junctions: Vec::new(),
            parallel_edges: Vec::new(),
            focus: Some(CellSceneFocus {
                identity: a,
                bounds: rect_a,
                recenter: false,
            }),
            movement: CellSceneMovement::Fresh,
            metrics: CellSceneMetrics::default(),
        };

        let adapted = layered_text_scene(&projection, &scene, true, true).expect("typed adapter");
        assert_eq!(adapted.nodes[0].represented_ids, ["A"]);
        assert_eq!(
            (adapted.nodes[0].rank, adapted.nodes[0].order),
            (Some(0), 1)
        );
        assert!(adapted.nodes[0].selected && adapted.nodes[0].critical);
        assert!(adapted.nodes[1].related && adapted.nodes[1].critical);
        assert_eq!(adapted.edges[0].ordinal, 3);
        assert_eq!(
            adapted.edges[0].provenance,
            [("A".to_string(), "B".to_string())]
        );
        assert_eq!(adapted.bands[0].nodes, ["A", "B"]);
        assert_eq!(adapted.omissions.window_omitted_task_ids, ["C"]);
        assert_eq!(adapted.omissions.admitted_dependency_edges, 2);
        assert_eq!(adapted.omissions.represented_dependency_edges, 1);
        assert_eq!(adapted.omissions.boundary_dependency_edges, 1);
        assert_eq!(
            adapted.omissions.boundary_dependencies,
            [("B".to_string(), "C".to_string())]
        );

        let unselected = layered_text_scene(&projection, &scene, false, true)
            .expect("automatic scene anchor is valid");
        assert!(unselected.nodes.iter().all(|node| !node.selected));
        assert!(unselected.nodes.iter().all(|node| !node.related));

        let mut stale = scene;
        stale.nodes.push(CellSceneNode {
            identity: GraphExplorerIdentity::Task("missing".to_string()),
            bounds: rect_b,
            rank: Some(2),
            order: 0,
        });
        assert_eq!(
            layered_text_scene(&projection, &stale, false, false),
            Err(LayeredTextSceneError::MissingSemanticNode {
                identity: "missing".to_string(),
            })
        );
    }

    fn test_identity() -> InputIdentity {
        InputIdentity {
            source: "test",
            plan_root: std::path::PathBuf::from("/repo/aplan"),
            plan_sha256: "plan".to_string(),
            repository_root: std::path::PathBuf::from("/repo"),
            repository: crate::RepositoryIdentity::NotObserved {
                reason: "test".to_string(),
            },
            configuration: crate::ConfigurationIdentity {
                path: None,
                sha256: "configuration".to_string(),
                content: crate::ConfigurationContentIdentity::Absent,
            },
            trust: crate::Trust::Hermetic,
            sha256: "test".to_string(),
        }
    }
}
