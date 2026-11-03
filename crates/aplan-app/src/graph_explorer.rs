//! Truthful multiscale projections over one immutable task graph.
//!
//! The explorer does not create a second planning model. It groups the task
//! identities already admitted by [`crate::GraphWorkspace`], carries every
//! dependency edge as exact provenance, and delegates bounded task detail to
//! [`crate::GraphWindowProjection`]. Coordinates and painting belong to later
//! layers.

use std::collections::{BTreeMap, BTreeSet};

use aplan_core::EdgeKind;
use aplan_core::task::MILESTONE_TAG;
use aplan_output::{GraphEdge, GraphNode, GraphView, Grouping};
use serde::Serialize;

use crate::{
    GraphContinuation, GraphProjection, GraphProjectionState, GraphRequest, GraphWindowBudget,
    GraphWindowProjection, GraphWindowRequest, GraphWorkspace, InputIdentity, Status,
    TaskBrowserRow,
};

pub(crate) const AGGREGATE_WINDOW_NODES: usize = 10;
pub(crate) const AGGREGATE_WINDOW_EDGES: usize = 20;

/// The semantic level currently presented by the graph explorer.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "level", content = "id", rename_all = "snake_case")]
pub enum GraphExplorerLocation {
    #[default]
    Plan,
    Phase(u32),
    Milestone(String),
    Task(String),
}

/// A selectable semantic subject. Phases are grouping bands, never nodes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum GraphExplorerIdentity {
    Milestone(String),
    Task(String),
}

impl GraphExplorerIdentity {
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Milestone(id) | Self::Task(id) => id,
        }
    }
}

/// Why a retained selection is allowed to bypass automatic entry policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphExplorerSelectionOrigin {
    #[default]
    Automatic,
    Contextual,
    Remembered,
}

/// One renderer-independent explorer request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphExplorerRequest {
    pub graph: GraphRequest,
    pub location: GraphExplorerLocation,
    /// The raw semantic subject survives collapse even when an aggregate card
    /// is the node that currently represents it.
    pub selected: Option<GraphExplorerIdentity>,
    pub selection_origin: GraphExplorerSelectionOrigin,
    /// Search locates admitted task identities. It does not narrow the graph.
    pub search: String,
    pub budget: GraphWindowBudget,
    /// Exact continuation for any bounded semantic level. It remains
    /// source-bound by `GraphWindowProjection`.
    pub continuation: Option<crate::GraphContinuationCursor>,
}

/// Persisted status and derived work facts for one represented set.
///
/// `done` counts only the persisted `done` status. `ready` is the same derived
/// answer as `aplan ready`; neither field is an aggregate status or a health
/// verdict.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GraphExplorerFacts {
    pub tasks: usize,
    pub todo: usize,
    pub in_progress: usize,
    pub blocked: usize,
    pub done: usize,
    pub dropped: usize,
    pub ready: usize,
}

impl GraphExplorerFacts {
    fn from_rows<'a>(rows: impl IntoIterator<Item = &'a TaskBrowserRow>) -> Self {
        let mut facts = Self::default();
        for row in rows {
            if is_milestone(row) {
                continue;
            }
            facts.tasks += 1;
            match row.status {
                Status::Todo => facts.todo += 1,
                Status::InProgress => facts.in_progress += 1,
                Status::Blocked => facts.blocked += 1,
                Status::Done => facts.done += 1,
                Status::Dropped => facts.dropped += 1,
            }
            facts.ready += usize::from(row.ready_rank.is_some());
        }
        facts
    }

    fn add(&mut self, other: &Self) {
        self.tasks += other.tasks;
        self.todo += other.todo;
        self.in_progress += other.in_progress;
        self.blocked += other.blocked;
        self.done += other.done;
        self.dropped += other.dropped;
        self.ready += other.ready;
    }
}

/// One exact task dependency carried through aggregation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct GraphExplorerDependency {
    pub from: String,
    pub to: String,
}

/// Why a projected relationship may route against the dominant flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphExplorerEdgeDisposition {
    Forward,
    /// Aggregation created a cyclic quotient while the underlying task graph
    /// remained acyclic. This is a layout fact, not a plan finding.
    AggregateFeedback,
}

/// One semantic node before any layout is attempted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphExplorerNode {
    pub identity: GraphExplorerIdentity,
    pub title: String,
    pub phase: u32,
    /// Grouping metadata for a task-level node. This never becomes a causal
    /// edge; explicit membership inspection reads this field.
    pub milestone: Option<String>,
    /// Status of this identity's persisted task. For a milestone card this is
    /// the checkpoint's status, never a roll-up of member statuses.
    pub status: Status,
    /// A grouping container can remain visible because admitted members map to
    /// it even when the active selector excluded the checkpoint task itself.
    pub checkpoint_in_scope: bool,
    /// Admitted canonical identities represented by this node. These vectors
    /// partition the admitted identities at aggregate levels.
    pub represented_ids: Vec<String>,
    /// Admitted non-milestone work grouped into this node.
    pub member_ids: Vec<String>,
    pub member_phases: Vec<u32>,
    pub facts: GraphExplorerFacts,
    /// Dependencies whose two ends collapse into this one node.
    pub internal_dependencies: Vec<GraphExplorerDependency>,
}

/// One visible aggregate or task relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphExplorerEdge {
    pub from: GraphExplorerIdentity,
    pub to: GraphExplorerIdentity,
    pub disposition: GraphExplorerEdgeDisposition,
    pub dependency_count: usize,
    /// Sorted and complete underlying task dependencies. Duplicate source
    /// entries remain duplicate entries so invalid input is never normalized
    /// into a cleaner-looking graph.
    pub provenance: Vec<GraphExplorerDependency>,
}

/// Phase is an ordered grouping coordinate, not a causal graph node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphExplorerPhaseBand {
    pub phase: u32,
    pub nodes: Vec<GraphExplorerIdentity>,
    pub facts: GraphExplorerFacts,
}

/// A dependency crossing the selected phase boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphExplorerBoundaryEdge {
    pub visible: GraphExplorerIdentity,
    pub remote: GraphExplorerIdentity,
    pub direction: GraphExplorerBoundaryDirection,
    pub disposition: GraphExplorerEdgeDisposition,
    pub dependency_count: usize,
    pub provenance: Vec<GraphExplorerDependency>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphExplorerBoundaryDirection {
    Incoming,
    Outgoing,
}

/// Exact context returned by a phase-focused narrow projection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GraphExplorerPhaseContext {
    pub previous: Option<u32>,
    pub next: Option<u32>,
    pub omitted: Vec<GraphExplorerIdentity>,
    pub boundary_edges: Vec<GraphExplorerBoundaryEdge>,
}

/// The coordinate-free graph supplied to layout and non-visual consumers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GraphExplorerGraph {
    pub nodes: Vec<GraphExplorerNode>,
    pub edges: Vec<GraphExplorerEdge>,
    pub phase_bands: Vec<GraphExplorerPhaseBand>,
}

/// A typed exceptional or informative condition. Conditions compose: a view
/// can be disconnected, cross-phase and contain ungrouped tasks at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "condition", rename_all = "snake_case")]
pub enum GraphExplorerCondition {
    DisconnectedComponents {
        components: usize,
    },
    TasksOutsideMilestones {
        ids: Vec<String>,
    },
    CrossPhaseDependencies {
        edges: Vec<GraphExplorerDependency>,
    },
    CrossCutDependencies {
        edges: Vec<GraphExplorerDependency>,
    },
    EmptyPhases {
        phases: Vec<u32>,
    },
    MilestoneOnly,
    NoMilestones,
    AggregateFeedback {
        components: Vec<Vec<GraphExplorerIdentity>>,
    },
    InvalidTaskCycle {
        components: Vec<Vec<String>>,
    },
}

/// Whether the requested level can be used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum GraphExplorerState {
    Complete,
    Empty,
    InvalidTaskCycle,
    SourceUnavailable(GraphProjectionState),
    UnknownLocation(GraphExplorerLocation),
}

/// Why the explorer selected its initial subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphExplorerAnchorReason {
    Contextual,
    Remembered,
    InProgress,
    Ready,
    FirstUnfinishedMilestone,
    FirstMilestone,
    FirstTask,
    DetailAnchor,
    NoMilestone,
}

/// Raw selection and the node which represents it at the current level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphExplorerSelection {
    pub subject: GraphExplorerIdentity,
    pub represented_by: GraphExplorerIdentity,
    pub reason: GraphExplorerAnchorReason,
}

/// Exact accounting across selection, semantic level and bounded windowing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GraphExplorerAccounting {
    pub admitted_task_ids: Vec<String>,
    pub level_task_ids: Vec<String>,
    pub visible_task_ids: Vec<String>,
    pub level_omitted_task_ids: Vec<String>,
    pub window_omitted_task_ids: Vec<String>,
    pub admitted_dependency_edges: usize,
    pub represented_dependency_edges: usize,
    pub boundary_dependency_edges: usize,
    /// Exact canonical dependencies with one admitted and one excluded end.
    pub boundary_dependencies: Vec<GraphExplorerDependency>,
    /// Cut-membership relationships excluded from this dependency projection.
    pub grouping: aplan_output::Grouping,
    /// Exact selector and structural-scope explanation from the source graph.
    pub source_scope: Vec<String>,
}

/// One admitted task found by graph search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphExplorerSearchMatch {
    pub task_id: String,
    pub title: String,
    pub represented_by: Option<GraphExplorerIdentity>,
    pub open: GraphExplorerLocation,
}

/// A source-bound reversible semantic transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphExplorerTransition {
    pub input_sha256: String,
    pub from: GraphExplorerLocation,
    pub to: GraphExplorerLocation,
    pub from_selection: Option<GraphExplorerIdentity>,
    pub to_selection: Option<GraphExplorerIdentity>,
}

impl GraphExplorerTransition {
    #[must_use]
    pub fn inverse(&self) -> Self {
        Self {
            input_sha256: self.input_sha256.clone(),
            from: self.to.clone(),
            to: self.from.clone(),
            from_selection: self.to_selection.clone(),
            to_selection: self.from_selection.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GraphExplorerTransitionFailure {
    #[error("graph transition belongs to input {expected}, not current input {actual}")]
    StaleInput { expected: String, actual: String },
    #[error("graph transition starts at {expected:?}, not current location {actual:?}")]
    WrongSource {
        expected: GraphExplorerLocation,
        actual: GraphExplorerLocation,
    },
}

/// Complete semantic answer for one explorer level.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphExplorerProjection {
    pub input: InputIdentity,
    pub location: GraphExplorerLocation,
    pub state: GraphExplorerState,
    pub graph: GraphExplorerGraph,
    pub conditions: Vec<GraphExplorerCondition>,
    pub selection: Option<GraphExplorerSelection>,
    pub no_selection_reason: Option<GraphExplorerAnchorReason>,
    pub phase_context: Option<GraphExplorerPhaseContext>,
    pub semantic_window: Option<GraphWindowProjection>,
    pub boundaries: Vec<GraphContinuation>,
    pub accounting: GraphExplorerAccounting,
    /// Canonical critical-route task ids which survived the active selector
    /// and structural scope. Aggregate painters map these raw ids through node
    /// provenance instead of inventing an aggregate critical path.
    pub critical_path: Vec<String>,
    pub search_matches: Vec<GraphExplorerSearchMatch>,
    pub transitions: Vec<GraphExplorerTransition>,
}

impl GraphExplorerProjection {
    /// Retarget selection inside this already projected semantic level.
    ///
    /// The graph, accounting, search and omissions are selection-independent.
    /// Updating the represented identity and transition subjects here avoids
    /// repeating complete-source projection for pointer and keyboard focus.
    /// A subject omitted from this bounded level is refused so the caller can
    /// open an appropriate detail location instead.
    pub fn select_contextual(&mut self, subject: &GraphExplorerIdentity) -> bool {
        let Some(represented_by) = self.graph.nodes.iter().find_map(|node| {
            (&node.identity == subject
                || node
                    .represented_ids
                    .iter()
                    .any(|represented| represented == subject.id()))
            .then(|| node.identity.clone())
        }) else {
            return false;
        };
        self.selection = Some(GraphExplorerSelection {
            subject: subject.clone(),
            represented_by,
            reason: GraphExplorerAnchorReason::Contextual,
        });
        self.no_selection_reason = None;
        self.transitions = transitions_for(&self.input, &self.location, Some(subject), &self.graph);
        true
    }
}

#[derive(Debug, Clone)]
struct Overview {
    graph: GraphExplorerGraph,
    dependencies: Vec<GraphExplorerDependency>,
    conditions: Vec<GraphExplorerCondition>,
    task_to_node: BTreeMap<String, GraphExplorerIdentity>,
    phases: Vec<u32>,
}

type RowIndex<'a> = BTreeMap<&'a str, &'a TaskBrowserRow>;

struct OverviewMapping {
    task_to_node: BTreeMap<String, GraphExplorerIdentity>,
    represented_by_node: BTreeMap<GraphExplorerIdentity, Vec<String>>,
    node_ids: BTreeSet<GraphExplorerIdentity>,
}

fn overview_mapping(rows: &RowIndex<'_>, admitted: &BTreeSet<String>) -> OverviewMapping {
    let mut task_to_node = BTreeMap::new();
    let mut represented_by_node: BTreeMap<GraphExplorerIdentity, Vec<String>> = BTreeMap::new();
    let mut node_ids = BTreeSet::new();
    for id in admitted {
        let Some(row) = rows.get(id.as_str()).copied() else {
            continue;
        };
        let identity = if is_milestone(row) {
            GraphExplorerIdentity::Milestone(row.id.clone())
        } else if let Some(milestone) = &row.milestone {
            GraphExplorerIdentity::Milestone(milestone.clone())
        } else {
            GraphExplorerIdentity::Task(row.id.clone())
        };
        task_to_node.insert(id.clone(), identity.clone());
        represented_by_node
            .entry(identity.clone())
            .or_default()
            .push(id.clone());
        node_ids.insert(identity);
    }
    OverviewMapping {
        task_to_node,
        represented_by_node,
        node_ids,
    }
}

struct OverviewEdges {
    edges: Vec<GraphExplorerEdge>,
    internal: BTreeMap<GraphExplorerIdentity, Vec<GraphExplorerDependency>>,
    quotient_cycles: Vec<Vec<GraphExplorerIdentity>>,
}

fn overview_edges(
    dependencies: &[GraphExplorerDependency],
    mapping: &OverviewMapping,
) -> OverviewEdges {
    let mut grouped: BTreeMap<
        (GraphExplorerIdentity, GraphExplorerIdentity),
        Vec<GraphExplorerDependency>,
    > = BTreeMap::new();
    let mut exact_task_edges = Vec::new();
    let mut internal: BTreeMap<GraphExplorerIdentity, Vec<GraphExplorerDependency>> =
        BTreeMap::new();
    for dependency in dependencies {
        let Some(from) = mapping.task_to_node.get(&dependency.from).cloned() else {
            continue;
        };
        let Some(to) = mapping.task_to_node.get(&dependency.to).cloned() else {
            continue;
        };
        if matches!(from, GraphExplorerIdentity::Task(_))
            && matches!(to, GraphExplorerIdentity::Task(_))
        {
            exact_task_edges.push(GraphExplorerEdge {
                from,
                to,
                disposition: GraphExplorerEdgeDisposition::Forward,
                dependency_count: 1,
                provenance: vec![dependency.clone()],
            });
        } else if from == to {
            internal.entry(from).or_default().push(dependency.clone());
        } else {
            grouped
                .entry((from, to))
                .or_default()
                .push(dependency.clone());
        }
    }
    let quotient_pairs: Vec<_> = grouped.keys().cloned().collect();
    let quotient_cycles = cyclic_components(&mapping.node_ids, &quotient_pairs);
    let cycle_sets: Vec<BTreeSet<GraphExplorerIdentity>> = quotient_cycles
        .iter()
        .cloned()
        .map(|component| component.into_iter().collect())
        .collect();
    let mut edges: Vec<GraphExplorerEdge> = grouped
        .into_iter()
        .map(|((from, to), mut provenance)| {
            provenance.sort();
            let feedback = cycle_sets
                .iter()
                .any(|component| component.contains(&from) && component.contains(&to));
            GraphExplorerEdge {
                from,
                to,
                disposition: if feedback {
                    GraphExplorerEdgeDisposition::AggregateFeedback
                } else {
                    GraphExplorerEdgeDisposition::Forward
                },
                dependency_count: provenance.len(),
                provenance,
            }
        })
        .collect();
    edges.append(&mut exact_task_edges);
    edges.sort_by(|left, right| {
        left.from
            .cmp(&right.from)
            .then(left.to.cmp(&right.to))
            .then(left.provenance.cmp(&right.provenance))
    });
    OverviewEdges {
        edges,
        internal,
        quotient_cycles,
    }
}

fn overview_nodes(
    rows: &RowIndex<'_>,
    admitted: &BTreeSet<String>,
    mapping: &mut OverviewMapping,
    internal: &mut BTreeMap<GraphExplorerIdentity, Vec<GraphExplorerDependency>>,
) -> Vec<GraphExplorerNode> {
    let mut nodes = Vec::new();
    for identity in &mapping.node_ids {
        let Some(row) = rows.get(identity.id()).copied() else {
            continue;
        };
        let mut represented_ids = mapping
            .represented_by_node
            .remove(identity)
            .unwrap_or_default();
        represented_ids.sort();
        let member_ids: Vec<String> = represented_ids
            .iter()
            .filter(|member| {
                rows.get(member.as_str())
                    .is_some_and(|member_row| !is_milestone(member_row))
            })
            .cloned()
            .collect();
        let member_rows: Vec<&TaskBrowserRow> = member_ids
            .iter()
            .filter_map(|member| rows.get(member.as_str()).copied())
            .collect();
        let mut member_phases: Vec<u32> = member_rows.iter().map(|member| member.phase).collect();
        member_phases.sort_unstable();
        member_phases.dedup();
        let mut internal_dependencies = internal.remove(identity).unwrap_or_default();
        internal_dependencies.sort();
        let milestone = matches!(identity, GraphExplorerIdentity::Milestone(_));
        nodes.push(GraphExplorerNode {
            identity: identity.clone(),
            title: row.title.clone(),
            phase: row.phase,
            milestone: if milestone {
                None
            } else {
                row.milestone.clone()
            },
            status: row.status,
            checkpoint_in_scope: !milestone || admitted.contains(identity.id()),
            represented_ids,
            member_ids,
            member_phases,
            facts: GraphExplorerFacts::from_rows(member_rows),
            internal_dependencies,
        });
    }
    nodes.sort_by(|left, right| left.identity.cmp(&right.identity));
    nodes
}

fn overview_phase_bands(
    catalogue: &[TaskBrowserRow],
    rows: &RowIndex<'_>,
    admitted: &BTreeSet<String>,
    nodes: &[GraphExplorerNode],
) -> Vec<GraphExplorerPhaseBand> {
    let phases: BTreeSet<u32> = catalogue.iter().map(|row| row.phase).collect();
    let mut band_nodes: BTreeMap<u32, Vec<GraphExplorerIdentity>> = BTreeMap::new();
    for node in nodes {
        band_nodes
            .entry(node.phase)
            .or_default()
            .push(node.identity.clone());
    }
    let mut band_facts: BTreeMap<u32, GraphExplorerFacts> = BTreeMap::new();
    for id in admitted {
        if let Some(row) = rows.get(id.as_str()).copied() {
            band_facts
                .entry(row.phase)
                .or_default()
                .add(&GraphExplorerFacts::from_rows([row]));
        }
    }
    phases
        .into_iter()
        .map(|phase| GraphExplorerPhaseBand {
            phase,
            nodes: band_nodes.remove(&phase).unwrap_or_default(),
            facts: band_facts.remove(&phase).unwrap_or_default(),
        })
        .collect()
}

fn dependency_conditions(
    rows: &RowIndex<'_>,
    dependencies: &[GraphExplorerDependency],
) -> Vec<GraphExplorerCondition> {
    let cross_phase: Vec<GraphExplorerDependency> = dependencies
        .iter()
        .filter(|edge| {
            rows.get(edge.from.as_str())
                .zip(rows.get(edge.to.as_str()))
                .is_some_and(|(from, to)| from.phase != to.phase)
        })
        .cloned()
        .collect();
    let cross_cut: Vec<GraphExplorerDependency> = dependencies
        .iter()
        .filter(|edge| {
            rows.get(edge.from.as_str())
                .zip(rows.get(edge.to.as_str()))
                .is_some_and(|(from, to)| from.milestone != to.milestone)
        })
        .cloned()
        .collect();
    let mut conditions = Vec::new();
    if !cross_phase.is_empty() {
        conditions.push(GraphExplorerCondition::CrossPhaseDependencies { edges: cross_phase });
    }
    if !cross_cut.is_empty() {
        conditions.push(GraphExplorerCondition::CrossCutDependencies { edges: cross_cut });
    }
    conditions
}

fn overview_conditions(
    rows: &RowIndex<'_>,
    admitted: &BTreeSet<String>,
    dependencies: &[GraphExplorerDependency],
    graph: &GraphExplorerGraph,
    quotient_cycles: Vec<Vec<GraphExplorerIdentity>>,
) -> Vec<GraphExplorerCondition> {
    let mut conditions = Vec::new();
    let component_count = connected_component_count(&graph.nodes, &graph.edges);
    if component_count > 1 {
        conditions.push(GraphExplorerCondition::DisconnectedComponents {
            components: component_count,
        });
    }
    let outside: Vec<String> = admitted
        .iter()
        .filter(|id| {
            rows.get(id.as_str())
                .is_some_and(|row| !is_milestone(row) && row.milestone.is_none())
        })
        .cloned()
        .collect();
    if !outside.is_empty() {
        conditions.push(GraphExplorerCondition::TasksOutsideMilestones { ids: outside });
    }
    conditions.extend(dependency_conditions(rows, dependencies));
    let empty_phases: Vec<u32> = graph
        .phase_bands
        .iter()
        .filter(|band| band.nodes.is_empty())
        .map(|band| band.phase)
        .collect();
    if !empty_phases.is_empty() {
        conditions.push(GraphExplorerCondition::EmptyPhases {
            phases: empty_phases,
        });
    }
    if !admitted.is_empty()
        && admitted
            .iter()
            .all(|id| rows.get(id.as_str()).is_some_and(|row| is_milestone(row)))
    {
        conditions.push(GraphExplorerCondition::MilestoneOnly);
    }
    if !admitted.is_empty()
        && !graph
            .nodes
            .iter()
            .any(|node| matches!(node.identity, GraphExplorerIdentity::Milestone(_)))
    {
        conditions.push(GraphExplorerCondition::NoMilestones);
    }
    let task_pairs: Vec<(String, String)> = dependencies
        .iter()
        .map(|edge| (edge.from.clone(), edge.to.clone()))
        .collect();
    let task_cycles = cyclic_components(admitted, &task_pairs);
    if !quotient_cycles.is_empty() && task_cycles.is_empty() {
        conditions.push(GraphExplorerCondition::AggregateFeedback {
            components: quotient_cycles,
        });
    }
    if !task_cycles.is_empty() {
        conditions.push(GraphExplorerCondition::InvalidTaskCycle {
            components: task_cycles,
        });
    }
    conditions
}

fn focused_phase(
    overview: &Overview,
    phase: u32,
) -> (GraphExplorerGraph, GraphExplorerPhaseContext) {
    let visible: BTreeSet<GraphExplorerIdentity> = overview
        .graph
        .nodes
        .iter()
        .filter(|node| node.phase == phase)
        .map(|node| node.identity.clone())
        .collect();
    let omitted = overview
        .graph
        .nodes
        .iter()
        .filter(|node| !visible.contains(&node.identity))
        .map(|node| node.identity.clone())
        .collect();
    let mut boundary_edges = Vec::new();
    let mut internal_edges = Vec::new();
    for edge in &overview.graph.edges {
        let boundary = match (visible.contains(&edge.from), visible.contains(&edge.to)) {
            (true, true) => {
                internal_edges.push(edge.clone());
                None
            }
            (true, false) => Some((
                edge.from.clone(),
                edge.to.clone(),
                GraphExplorerBoundaryDirection::Outgoing,
            )),
            (false, true) => Some((
                edge.to.clone(),
                edge.from.clone(),
                GraphExplorerBoundaryDirection::Incoming,
            )),
            (false, false) => None,
        };
        if let Some((visible, remote, direction)) = boundary {
            boundary_edges.push(GraphExplorerBoundaryEdge {
                visible,
                remote,
                direction,
                disposition: edge.disposition,
                dependency_count: edge.dependency_count,
                provenance: edge.provenance.clone(),
            });
        }
    }
    boundary_edges.sort_by(|left, right| {
        left.visible
            .cmp(&right.visible)
            .then(left.remote.cmp(&right.remote))
            .then(left.direction.cmp(&right.direction))
    });
    let graph = GraphExplorerGraph {
        nodes: overview
            .graph
            .nodes
            .iter()
            .filter(|node| visible.contains(&node.identity))
            .cloned()
            .collect(),
        edges: internal_edges,
        phase_bands: overview
            .graph
            .phase_bands
            .iter()
            .filter(|band| band.phase == phase)
            .cloned()
            .collect(),
    };
    let index = overview
        .phases
        .iter()
        .position(|candidate| *candidate == phase)
        .expect("known phase has an index");
    let context = GraphExplorerPhaseContext {
        previous: index
            .checked_sub(1)
            .and_then(|previous| overview.phases.get(previous).copied()),
        next: overview.phases.get(index + 1).copied(),
        omitted,
        boundary_edges,
    };
    (graph, context)
}

fn detail_level_ids(
    base: &GraphProjection,
    overview: &Overview,
    rows: &RowIndex<'_>,
    id: &str,
    task_focus: bool,
) -> BTreeSet<String> {
    if task_focus {
        return base.admitted.clone();
    }
    let mut level_ids: BTreeSet<String> = base
        .admitted
        .iter()
        .filter(|task| {
            rows.get(task.as_str())
                .is_some_and(|row| !is_milestone(row) && row.milestone.as_deref() == Some(id))
        })
        .cloned()
        .collect();
    let members = level_ids.clone();
    for dependency in &overview.dependencies {
        if members.contains(&dependency.from) {
            level_ids.insert(dependency.to.clone());
        }
        if members.contains(&dependency.to) {
            level_ids.insert(dependency.from.clone());
        }
    }
    level_ids
}

fn preferred_detail_anchor(
    request: &GraphExplorerRequest,
    level_ids: &BTreeSet<String>,
    id: &str,
    task_focus: bool,
) -> Option<String> {
    request
        .selected
        .as_ref()
        .and_then(|identity| match identity {
            GraphExplorerIdentity::Task(task) if level_ids.contains(task) => Some(task.clone()),
            GraphExplorerIdentity::Milestone(_) | GraphExplorerIdentity::Task(_) => None,
        })
        .or_else(|| task_focus.then(|| id.to_string()))
        .or_else(|| level_ids.iter().next().cloned())
}

fn requested_selection(
    request: &GraphExplorerRequest,
    task_to_node: &BTreeMap<String, GraphExplorerIdentity>,
    visible: &BTreeSet<GraphExplorerIdentity>,
) -> Option<GraphExplorerSelection> {
    let subject = request.selected.as_ref()?;
    let represented_by = (match subject {
        GraphExplorerIdentity::Task(id) => task_to_node.get(id).cloned(),
        GraphExplorerIdentity::Milestone(id) => Some(GraphExplorerIdentity::Milestone(id.clone())),
    })
    .filter(|node| visible.contains(node))?;
    let reason = match request.selection_origin {
        GraphExplorerSelectionOrigin::Remembered => GraphExplorerAnchorReason::Remembered,
        GraphExplorerSelectionOrigin::Contextual | GraphExplorerSelectionOrigin::Automatic => {
            GraphExplorerAnchorReason::Contextual
        }
    };
    Some(GraphExplorerSelection {
        subject: subject.clone(),
        represented_by,
        reason,
    })
}

impl GraphWorkspace {
    /// Project one multiscale semantic graph without performing layout.
    #[must_use]
    pub fn project_explorer(&self, request: &GraphExplorerRequest) -> GraphExplorerProjection {
        let base = self.project(&request.graph);
        let overview = self.explorer_overview(&base);
        let source_unavailable = match &base.state {
            GraphProjectionState::UnknownTask(_) | GraphProjectionState::CyclicCriticalPath => {
                Some(base.state.clone())
            }
            GraphProjectionState::Complete
            | GraphProjectionState::EmptyScope
            | GraphProjectionState::NoEdges => None,
        };

        let mut answer = match &request.location {
            GraphExplorerLocation::Plan => self.plan_explorer(request, &base, overview),
            GraphExplorerLocation::Phase(phase) => {
                self.phase_explorer(request, &base, overview, *phase)
            }
            GraphExplorerLocation::Milestone(id) => {
                self.detail_explorer(request, &base, overview, id, false)
            }
            GraphExplorerLocation::Task(id) => {
                self.detail_explorer(request, &base, overview, id, true)
            }
        };
        if let Some(state) = source_unavailable {
            answer.state = GraphExplorerState::SourceUnavailable(state);
        }
        answer
    }

    /// Apply a transition only to the snapshot and location which emitted it.
    pub fn follow_explorer_transition(
        &self,
        current: &GraphExplorerRequest,
        transition: &GraphExplorerTransition,
    ) -> Result<GraphExplorerProjection, GraphExplorerTransitionFailure> {
        if transition.input_sha256 != self.input.sha256 {
            return Err(GraphExplorerTransitionFailure::StaleInput {
                expected: transition.input_sha256.clone(),
                actual: self.input.sha256.clone(),
            });
        }
        if transition.from != current.location {
            return Err(GraphExplorerTransitionFailure::WrongSource {
                expected: transition.from.clone(),
                actual: current.location.clone(),
            });
        }
        let mut next = current.clone();
        next.location = transition.to.clone();
        next.selected.clone_from(&transition.to_selection);
        next.selection_origin = GraphExplorerSelectionOrigin::Contextual;
        Ok(self.project_explorer(&next))
    }

    fn explorer_overview(&self, base: &GraphProjection) -> Overview {
        let rows: RowIndex<'_> = self
            .catalogue
            .iter()
            .map(|row| (row.id.as_str(), row))
            .collect();
        let admitted = &base.admitted;
        let mut mapping = overview_mapping(&rows, admitted);
        let dependencies = self.admitted_dependencies(admitted);
        let mut aggregated = overview_edges(&dependencies, &mapping);
        let nodes = overview_nodes(&rows, admitted, &mut mapping, &mut aggregated.internal);
        let phase_bands = overview_phase_bands(&self.catalogue, &rows, admitted, &nodes);
        let graph = GraphExplorerGraph {
            nodes,
            edges: aggregated.edges,
            phase_bands,
        };
        let conditions = overview_conditions(
            &rows,
            admitted,
            &dependencies,
            &graph,
            aggregated.quotient_cycles,
        );
        let phases = graph.phase_bands.iter().map(|band| band.phase).collect();
        Overview {
            graph,
            dependencies,
            conditions,
            task_to_node: mapping.task_to_node,
            phases,
        }
    }

    fn plan_explorer(
        &self,
        request: &GraphExplorerRequest,
        base: &GraphProjection,
        overview: Overview,
    ) -> GraphExplorerProjection {
        let selection = self.choose_selection(request, base, &overview, &overview.graph);
        let (graph, window) = self.aggregate_window(
            request,
            base,
            &overview,
            &overview.graph,
            selection.as_ref(),
        );
        let selection = aggregate_window_selection(selection, &graph, &window);
        let no_selection_reason = no_selection_reason(&overview.conditions, selection.as_ref());
        let state = state_for(&overview.conditions, graph.nodes.is_empty());
        let visible_ids = represented_task_ids(&graph);
        let accounting = self.accounting(
            base,
            &overview.dependencies,
            &base.admitted,
            &visible_ids,
            graph_dependency_count(&graph),
        );
        let search_matches = self.search_matches(request, base, &overview, &graph);
        let transitions = transitions_for(
            &self.input,
            &request.location,
            selection.as_ref().map(|selected| &selected.subject),
            &graph,
        );
        let boundaries = window.continuations.clone();
        GraphExplorerProjection {
            input: self.input.clone(),
            location: request.location.clone(),
            state,
            graph,
            conditions: overview.conditions,
            selection,
            no_selection_reason,
            phase_context: None,
            semantic_window: Some(window),
            boundaries,
            accounting,
            critical_path: admitted_critical_path(base),
            search_matches,
            transitions,
        }
    }

    fn phase_explorer(
        &self,
        request: &GraphExplorerRequest,
        base: &GraphProjection,
        overview: Overview,
        phase: u32,
    ) -> GraphExplorerProjection {
        if !overview.phases.contains(&phase) {
            return self.unknown_location(request, base, overview, request.location.clone());
        }
        let (complete_graph, mut phase_context) = focused_phase(&overview, phase);
        let selection = self.choose_selection(request, base, &overview, &complete_graph);
        let level_ids = represented_task_ids(&complete_graph);
        let (graph, window) = self.aggregate_window(
            request,
            base,
            &overview,
            &complete_graph,
            selection.as_ref(),
        );
        let selection = aggregate_window_selection(selection, &graph, &window);
        let visible_nodes: BTreeSet<_> = graph
            .nodes
            .iter()
            .map(|node| node.identity.clone())
            .collect();
        phase_context
            .boundary_edges
            .retain(|edge| visible_nodes.contains(&edge.visible));
        let no_selection_reason = no_selection_reason(&overview.conditions, selection.as_ref());
        let visible_ids = represented_task_ids(&graph);
        let search_matches = self.search_matches(request, base, &overview, &graph);
        let transitions = transitions_for(
            &self.input,
            &request.location,
            selection.as_ref().map(|selected| &selected.subject),
            &graph,
        );
        let accounting = self.accounting(
            base,
            &overview.dependencies,
            &level_ids,
            &visible_ids,
            graph_dependency_count(&graph) + phase_context_dependency_count(&phase_context),
        );
        let boundaries = window.continuations.clone();
        GraphExplorerProjection {
            input: self.input.clone(),
            location: request.location.clone(),
            state: state_for(&overview.conditions, graph.nodes.is_empty()),
            graph,
            conditions: overview.conditions,
            selection,
            no_selection_reason,
            phase_context: Some(phase_context),
            semantic_window: Some(window),
            boundaries,
            accounting,
            critical_path: admitted_critical_path(base),
            search_matches,
            transitions,
        }
    }

    fn detail_explorer(
        &self,
        request: &GraphExplorerRequest,
        base: &GraphProjection,
        overview: Overview,
        id: &str,
        task_focus: bool,
    ) -> GraphExplorerProjection {
        let row_by_id: BTreeMap<&str, &TaskBrowserRow> = self
            .catalogue
            .iter()
            .map(|row| (row.id.as_str(), row))
            .collect();
        let valid = if task_focus {
            base.admitted.contains(id)
        } else {
            overview
                .graph
                .nodes
                .iter()
                .any(|node| node.identity == GraphExplorerIdentity::Milestone(id.to_string()))
        };
        if !valid {
            return self.unknown_location(request, base, overview, request.location.clone());
        }

        let level_ids = detail_level_ids(base, &overview, &row_by_id, id, task_focus);
        let source = self.dependency_projection(base, &level_ids);
        let window = source.window(&GraphWindowRequest {
            preferred_anchor: preferred_detail_anchor(request, &level_ids, id, task_focus),
            budget: request.budget,
            continuation: request.continuation.clone(),
        });
        let visible_ids: BTreeSet<String> = window
            .view
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect();
        let graph = Self::task_graph(&window.view, &row_by_id);
        let represented_by: BTreeMap<String, GraphExplorerIdentity> = graph
            .nodes
            .iter()
            .map(|node| (node.identity.id().to_string(), node.identity.clone()))
            .collect();
        let mut detail_overview = overview.clone();
        detail_overview.task_to_node = represented_by;
        let selection = detail_selection(request, &visible_ids, &window);
        let accounting = self.accounting(
            base,
            &overview.dependencies,
            &level_ids,
            &visible_ids,
            graph_dependency_count(&graph),
        );
        let search_matches = self.search_matches(request, base, &detail_overview, &graph);
        let transitions = transitions_for(
            &self.input,
            &request.location,
            selection.as_ref().map(|selected| &selected.subject),
            &graph,
        );
        let state = state_for(&overview.conditions, graph.nodes.is_empty());
        GraphExplorerProjection {
            input: self.input.clone(),
            location: request.location.clone(),
            state,
            graph,
            conditions: overview.conditions,
            selection,
            no_selection_reason: None,
            phase_context: None,
            boundaries: window.continuations.clone(),
            semantic_window: Some(window),
            accounting,
            critical_path: admitted_critical_path(base),
            search_matches,
            transitions,
        }
    }

    fn unknown_location(
        &self,
        request: &GraphExplorerRequest,
        base: &GraphProjection,
        overview: Overview,
        location: GraphExplorerLocation,
    ) -> GraphExplorerProjection {
        let accounting = self.accounting(
            base,
            &overview.dependencies,
            &BTreeSet::new(),
            &BTreeSet::new(),
            0,
        );
        let search_matches =
            self.search_matches(request, base, &overview, &GraphExplorerGraph::default());
        GraphExplorerProjection {
            input: self.input.clone(),
            location: request.location.clone(),
            state: GraphExplorerState::UnknownLocation(location),
            graph: GraphExplorerGraph::default(),
            conditions: overview.conditions,
            selection: None,
            no_selection_reason: None,
            phase_context: None,
            semantic_window: None,
            boundaries: Vec::new(),
            accounting,
            critical_path: admitted_critical_path(base),
            search_matches,
            transitions: Vec::new(),
        }
    }

    fn dependency_projection(
        &self,
        base: &GraphProjection,
        include: &BTreeSet<String>,
    ) -> GraphProjection {
        let rows: BTreeMap<&str, &TaskBrowserRow> = self
            .catalogue
            .iter()
            .map(|row| (row.id.as_str(), row))
            .collect();
        let nodes = include
            .iter()
            .filter_map(|id| rows.get(id.as_str()).copied())
            .map(|row| GraphNode {
                id: row.id.clone(),
                title: row.title.clone(),
                status: row.status,
                done: row.status.satisfies_dependency(),
            })
            .collect();
        let mut hidden_edges = 0;
        let edges = self
            .whole
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Dependency)
            .filter_map(
                |edge| match (include.contains(&edge.from), include.contains(&edge.to)) {
                    (true, true) => Some(edge.clone()),
                    (true, false) | (false, true) => {
                        hidden_edges += 1;
                        None
                    }
                    (false, false) => None,
                },
            )
            .collect();
        let view = GraphView {
            nodes,
            edges,
            hidden_edges,
            grouping: Grouping::default(),
            scope: base.view.scope.clone(),
        };
        let state = if view.nodes.is_empty() {
            GraphProjectionState::EmptyScope
        } else if view.edges.is_empty() {
            GraphProjectionState::NoEdges
        } else {
            GraphProjectionState::Complete
        };
        GraphProjection {
            input: self.input.clone(),
            view,
            state,
            admitted: include.clone(),
            critical_path: base
                .critical_path
                .iter()
                .filter(|id| include.contains(*id))
                .cloned()
                .collect(),
            exclusions: base.exclusions.clone(),
        }
    }

    fn aggregate_window(
        &self,
        request: &GraphExplorerRequest,
        base: &GraphProjection,
        overview: &Overview,
        graph: &GraphExplorerGraph,
        selection: Option<&GraphExplorerSelection>,
    ) -> (GraphExplorerGraph, GraphWindowProjection) {
        let source = self.aggregate_window_source(base, overview, graph);
        let budget = GraphWindowBudget {
            nodes: request.budget.nodes.min(AGGREGATE_WINDOW_NODES),
            edges: request.budget.edges.min(AGGREGATE_WINDOW_EDGES),
            lanes: usize::MAX,
        };
        let window = source.window(&GraphWindowRequest {
            preferred_anchor: selection.map(|selected| selected.represented_by.id().to_string()),
            budget,
            continuation: request.continuation.clone(),
        });
        let visible: BTreeSet<&str> = window
            .view
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect();
        let visible_edges: BTreeSet<(&str, &str)> = window
            .view
            .edges
            .iter()
            .map(|edge| (edge.from.as_str(), edge.to.as_str()))
            .collect();
        let graph = GraphExplorerGraph {
            nodes: graph
                .nodes
                .iter()
                .filter(|node| visible.contains(node.identity.id()))
                .cloned()
                .collect(),
            edges: graph
                .edges
                .iter()
                .filter(|edge| visible_edges.contains(&(edge.from.id(), edge.to.id())))
                .cloned()
                .collect(),
            phase_bands: graph
                .phase_bands
                .iter()
                .map(|band| GraphExplorerPhaseBand {
                    phase: band.phase,
                    nodes: band
                        .nodes
                        .iter()
                        .filter(|identity| visible.contains(identity.id()))
                        .cloned()
                        .collect(),
                    facts: band.facts.clone(),
                })
                .collect(),
        };
        (graph, window)
    }

    fn aggregate_window_source(
        &self,
        base: &GraphProjection,
        overview: &Overview,
        graph: &GraphExplorerGraph,
    ) -> GraphProjection {
        let nodes = graph
            .nodes
            .iter()
            .map(|node| GraphNode {
                id: node.identity.id().to_string(),
                title: node.title.clone(),
                status: node.status,
                done: node.status.satisfies_dependency(),
            })
            .collect::<Vec<_>>();
        let edges = graph
            .edges
            .iter()
            .map(|edge| GraphEdge {
                from: edge.from.id().to_string(),
                to: edge.to.id().to_string(),
                kind: EdgeKind::Dependency,
            })
            .collect::<Vec<_>>();
        let admitted = nodes.iter().map(|node| node.id.clone()).collect();
        let mut critical_path = Vec::new();
        for task in &base.critical_path {
            let Some(identity) = overview.task_to_node.get(task) else {
                continue;
            };
            if critical_path
                .last()
                .is_none_or(|last| last != identity.id())
            {
                critical_path.push(identity.id().to_string());
            }
        }
        let state = if nodes.is_empty() {
            GraphProjectionState::EmptyScope
        } else if edges.is_empty() {
            GraphProjectionState::NoEdges
        } else {
            GraphProjectionState::Complete
        };
        GraphProjection {
            input: self.input.clone(),
            view: GraphView {
                nodes,
                edges,
                hidden_edges: 0,
                grouping: Grouping::default(),
                scope: base.view.scope.clone(),
            },
            state,
            admitted,
            critical_path,
            exclusions: BTreeMap::new(),
        }
    }

    fn task_graph(view: &GraphView, rows: &BTreeMap<&str, &TaskBrowserRow>) -> GraphExplorerGraph {
        let mut nodes: Vec<GraphExplorerNode> = view
            .nodes
            .iter()
            .filter_map(|node| rows.get(node.id.as_str()).copied())
            .map(|row| GraphExplorerNode {
                identity: GraphExplorerIdentity::Task(row.id.clone()),
                title: row.title.clone(),
                phase: row.phase,
                milestone: row.milestone.clone(),
                status: row.status,
                checkpoint_in_scope: true,
                represented_ids: vec![row.id.clone()],
                member_ids: (!is_milestone(row))
                    .then(|| row.id.clone())
                    .into_iter()
                    .collect(),
                member_phases: vec![row.phase],
                facts: GraphExplorerFacts::from_rows([row]),
                internal_dependencies: Vec::new(),
            })
            .collect();
        nodes.sort_by(|left, right| left.identity.cmp(&right.identity));
        let edges = view
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Dependency)
            .map(|edge| GraphExplorerEdge {
                from: GraphExplorerIdentity::Task(edge.from.clone()),
                to: GraphExplorerIdentity::Task(edge.to.clone()),
                disposition: GraphExplorerEdgeDisposition::Forward,
                dependency_count: 1,
                provenance: vec![GraphExplorerDependency {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                }],
            })
            .collect();
        let mut bands: BTreeMap<u32, GraphExplorerPhaseBand> = BTreeMap::new();
        for node in &nodes {
            let band = bands
                .entry(node.phase)
                .or_insert_with(|| GraphExplorerPhaseBand {
                    phase: node.phase,
                    nodes: Vec::new(),
                    facts: GraphExplorerFacts::default(),
                });
            band.nodes.push(node.identity.clone());
            band.facts.add(&node.facts);
        }
        GraphExplorerGraph {
            nodes,
            edges,
            phase_bands: bands.into_values().collect(),
        }
    }

    fn admitted_dependencies(&self, admitted: &BTreeSet<String>) -> Vec<GraphExplorerDependency> {
        let mut dependencies: Vec<GraphExplorerDependency> = self
            .whole
            .edges
            .iter()
            .filter(|edge| {
                edge.kind == EdgeKind::Dependency
                    && admitted.contains(&edge.from)
                    && admitted.contains(&edge.to)
            })
            .map(|edge| GraphExplorerDependency {
                from: edge.from.clone(),
                to: edge.to.clone(),
            })
            .collect();
        dependencies.sort();
        dependencies
    }

    fn ungrouped_selection(
        base: &GraphProjection,
        rows: &BTreeMap<&str, &TaskBrowserRow>,
        visible: &BTreeSet<GraphExplorerIdentity>,
    ) -> Option<GraphExplorerSelection> {
        let represented = |row: &TaskBrowserRow| {
            let identity = GraphExplorerIdentity::Task(row.id.clone());
            visible.contains(&identity).then_some(identity)
        };
        let mut candidates: Vec<&TaskBrowserRow> = base
            .admitted
            .iter()
            .filter_map(|id| rows.get(id.as_str()).copied())
            .filter(|row| represented(row).is_some())
            .collect();
        candidates.sort_by_key(|row| {
            (
                match row.status {
                    Status::InProgress => 0,
                    _ if row.ready_rank.is_some() => 1,
                    _ => 2,
                },
                row.ready_rank.unwrap_or(usize::MAX),
                row.phase,
                row.id.as_str(),
            )
        });
        candidates.first().map(|row| {
            let identity = represented(row).expect("filtered above");
            GraphExplorerSelection {
                subject: identity.clone(),
                represented_by: identity,
                reason: if row.status == Status::InProgress {
                    GraphExplorerAnchorReason::InProgress
                } else if row.ready_rank.is_some() {
                    GraphExplorerAnchorReason::Ready
                } else {
                    GraphExplorerAnchorReason::FirstTask
                },
            }
        })
    }

    fn choose_selection(
        &self,
        request: &GraphExplorerRequest,
        base: &GraphProjection,
        overview: &Overview,
        graph: &GraphExplorerGraph,
    ) -> Option<GraphExplorerSelection> {
        let visible: BTreeSet<GraphExplorerIdentity> = graph
            .nodes
            .iter()
            .map(|node| node.identity.clone())
            .collect();
        if let Some(selection) = requested_selection(request, &overview.task_to_node, &visible) {
            return Some(selection);
        }

        let rows: BTreeMap<&str, &TaskBrowserRow> = self
            .catalogue
            .iter()
            .map(|row| (row.id.as_str(), row))
            .collect();
        let milestones: Vec<&GraphExplorerNode> = graph
            .nodes
            .iter()
            .filter(|node| matches!(node.identity, GraphExplorerIdentity::Milestone(_)))
            .collect();
        if milestones.is_empty() {
            return Self::ungrouped_selection(base, &rows, &visible);
        }
        let member_milestone = |row: &TaskBrowserRow| {
            row.milestone
                .as_ref()
                .map(|id| GraphExplorerIdentity::Milestone(id.clone()))
                .filter(|identity| visible.contains(identity))
        };
        let mut in_progress: Vec<&TaskBrowserRow> = base
            .admitted
            .iter()
            .filter_map(|id| rows.get(id.as_str()).copied())
            .filter(|row| row.status == Status::InProgress)
            .filter(|row| member_milestone(row).is_some())
            .collect();
        in_progress.sort_by_key(|row| (row.ready_rank.unwrap_or(usize::MAX), row.id.as_str()));
        if let Some(row) = in_progress.first() {
            let represented_by = member_milestone(row).expect("filtered above");
            return Some(GraphExplorerSelection {
                subject: GraphExplorerIdentity::Task(row.id.clone()),
                represented_by,
                reason: GraphExplorerAnchorReason::InProgress,
            });
        }
        let mut ready: Vec<&TaskBrowserRow> = base
            .admitted
            .iter()
            .filter_map(|id| rows.get(id.as_str()).copied())
            .filter(|row| row.ready_rank.is_some())
            .filter(|row| member_milestone(row).is_some())
            .collect();
        ready.sort_by_key(|row| (row.ready_rank.unwrap_or(usize::MAX), row.id.as_str()));
        if let Some(row) = ready.first() {
            let represented_by = member_milestone(row).expect("filtered above");
            return Some(GraphExplorerSelection {
                subject: GraphExplorerIdentity::Task(row.id.clone()),
                represented_by,
                reason: GraphExplorerAnchorReason::Ready,
            });
        }
        let mut unfinished: Vec<&GraphExplorerNode> = milestones
            .iter()
            .copied()
            .filter(|node| !node.status.is_terminal())
            .collect();
        unfinished.sort_by_key(|node| (node.phase, node.identity.clone()));
        let (node, reason) = unfinished.first().copied().map_or_else(
            || {
                let mut all = milestones;
                all.sort_by_key(|node| (node.phase, node.identity.clone()));
                (all[0], GraphExplorerAnchorReason::FirstMilestone)
            },
            |node| (node, GraphExplorerAnchorReason::FirstUnfinishedMilestone),
        );
        Some(GraphExplorerSelection {
            subject: node.identity.clone(),
            represented_by: node.identity.clone(),
            reason,
        })
    }

    fn accounting(
        &self,
        base: &GraphProjection,
        dependencies: &[GraphExplorerDependency],
        level_ids: &BTreeSet<String>,
        visible_ids: &BTreeSet<String>,
        represented_dependency_edges: usize,
    ) -> GraphExplorerAccounting {
        let level_omitted_task_ids = base.admitted.difference(level_ids).cloned().collect();
        let window_omitted_task_ids = level_ids.difference(visible_ids).cloned().collect();
        let mut boundary_dependencies: Vec<GraphExplorerDependency> = self
            .whole
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Dependency)
            .filter(|edge| base.admitted.contains(&edge.from) ^ base.admitted.contains(&edge.to))
            .map(|edge| GraphExplorerDependency {
                from: edge.from.clone(),
                to: edge.to.clone(),
            })
            .collect();
        boundary_dependencies.sort();
        let boundary_dependency_edges = boundary_dependencies.len();
        GraphExplorerAccounting {
            admitted_task_ids: base.admitted.iter().cloned().collect(),
            level_task_ids: level_ids.iter().cloned().collect(),
            visible_task_ids: visible_ids.iter().cloned().collect(),
            level_omitted_task_ids,
            window_omitted_task_ids,
            admitted_dependency_edges: dependencies.len(),
            represented_dependency_edges,
            boundary_dependency_edges,
            boundary_dependencies,
            grouping: base.view.grouping.clone(),
            source_scope: base.view.scope.clone(),
        }
    }

    fn search_matches(
        &self,
        request: &GraphExplorerRequest,
        base: &GraphProjection,
        overview: &Overview,
        graph: &GraphExplorerGraph,
    ) -> Vec<GraphExplorerSearchMatch> {
        let query = request.search.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let visible: BTreeSet<GraphExplorerIdentity> = graph
            .nodes
            .iter()
            .map(|node| node.identity.clone())
            .collect();
        let mut matches: Vec<GraphExplorerSearchMatch> = self
            .catalogue
            .iter()
            .filter(|row| base.admitted.contains(&row.id))
            .filter(|row| {
                row.id.to_lowercase().contains(&query) || row.title.to_lowercase().contains(&query)
            })
            .map(|row| {
                let represented_by = overview
                    .task_to_node
                    .get(&row.id)
                    .cloned()
                    .filter(|identity| visible.contains(identity));
                GraphExplorerSearchMatch {
                    task_id: row.id.clone(),
                    title: row.title.clone(),
                    represented_by,
                    open: GraphExplorerLocation::Task(row.id.clone()),
                }
            })
            .collect();
        matches.sort_by(|left, right| left.task_id.cmp(&right.task_id));
        matches
    }
}

fn graph_dependency_count(graph: &GraphExplorerGraph) -> usize {
    graph
        .edges
        .iter()
        .map(|edge| edge.dependency_count)
        .sum::<usize>()
        + graph
            .nodes
            .iter()
            .map(|node| node.internal_dependencies.len())
            .sum::<usize>()
}

fn represented_task_ids(graph: &GraphExplorerGraph) -> BTreeSet<String> {
    graph
        .nodes
        .iter()
        .flat_map(|node| node.represented_ids.iter().cloned())
        .collect()
}

fn aggregate_window_selection(
    selection: Option<GraphExplorerSelection>,
    graph: &GraphExplorerGraph,
    window: &GraphWindowProjection,
) -> Option<GraphExplorerSelection> {
    if selection.as_ref().is_some_and(|selection| {
        graph
            .nodes
            .iter()
            .any(|node| node.identity == selection.represented_by)
    }) {
        return selection;
    }
    let anchor = window.anchor.as_ref()?;
    let node = graph
        .nodes
        .iter()
        .find(|node| node.identity.id() == anchor.selected)?;
    Some(GraphExplorerSelection {
        subject: node.identity.clone(),
        represented_by: node.identity.clone(),
        reason: GraphExplorerAnchorReason::Contextual,
    })
}

fn admitted_critical_path(base: &GraphProjection) -> Vec<String> {
    base.critical_path
        .iter()
        .filter(|id| base.admitted.contains(*id))
        .cloned()
        .collect()
}

fn phase_context_dependency_count(context: &GraphExplorerPhaseContext) -> usize {
    context
        .boundary_edges
        .iter()
        .map(|edge| edge.dependency_count)
        .sum()
}

fn transitions_for(
    input: &InputIdentity,
    from: &GraphExplorerLocation,
    selected: Option<&GraphExplorerIdentity>,
    graph: &GraphExplorerGraph,
) -> Vec<GraphExplorerTransition> {
    let mut transitions = Vec::new();
    if *from != GraphExplorerLocation::Plan {
        transitions.push(GraphExplorerTransition {
            input_sha256: input.sha256.clone(),
            from: from.clone(),
            to: GraphExplorerLocation::Plan,
            from_selection: selected.cloned(),
            to_selection: selected.cloned(),
        });
    }
    for band in &graph.phase_bands {
        let to = GraphExplorerLocation::Phase(band.phase);
        if &to != from {
            let to_selection = selected
                .filter(|subject| {
                    graph.nodes.iter().any(|node| {
                        band.nodes.contains(&node.identity)
                            && (&node.identity == *subject
                                || node.represented_ids.iter().any(|id| id == subject.id()))
                    })
                })
                .cloned()
                .or_else(|| band.nodes.first().cloned());
            transitions.push(GraphExplorerTransition {
                input_sha256: input.sha256.clone(),
                from: from.clone(),
                to,
                from_selection: selected.cloned(),
                to_selection,
            });
        }
    }
    for node in &graph.nodes {
        let to = match &node.identity {
            GraphExplorerIdentity::Milestone(id) => GraphExplorerLocation::Milestone(id.clone()),
            GraphExplorerIdentity::Task(id) => GraphExplorerLocation::Task(id.clone()),
        };
        if &to == from {
            continue;
        }
        let to_selection = match &node.identity {
            GraphExplorerIdentity::Milestone(_) => selected
                .filter(|subject| node.represented_ids.iter().any(|id| id == subject.id()))
                .cloned()
                .or_else(|| {
                    node.member_ids
                        .first()
                        .cloned()
                        .map(GraphExplorerIdentity::Task)
                }),
            GraphExplorerIdentity::Task(id) => Some(GraphExplorerIdentity::Task(id.clone())),
        };
        transitions.push(GraphExplorerTransition {
            input_sha256: input.sha256.clone(),
            from: from.clone(),
            to,
            from_selection: selected.cloned(),
            to_selection,
        });
    }
    transitions.sort_by(|left, right| left.to.cmp(&right.to));
    transitions.dedup_by(|left, right| left.to == right.to);
    transitions
}

fn detail_selection(
    request: &GraphExplorerRequest,
    visible_ids: &BTreeSet<String>,
    window: &GraphWindowProjection,
) -> Option<GraphExplorerSelection> {
    if let Some(GraphExplorerIdentity::Task(id)) = request.selected.as_ref()
        && visible_ids.contains(id)
    {
        let reason = match request.selection_origin {
            GraphExplorerSelectionOrigin::Remembered => GraphExplorerAnchorReason::Remembered,
            GraphExplorerSelectionOrigin::Automatic | GraphExplorerSelectionOrigin::Contextual => {
                GraphExplorerAnchorReason::Contextual
            }
        };
        return Some(GraphExplorerSelection {
            subject: GraphExplorerIdentity::Task(id.clone()),
            represented_by: GraphExplorerIdentity::Task(id.clone()),
            reason,
        });
    }
    window.anchor.as_ref().map(|anchor| {
        let identity = GraphExplorerIdentity::Task(anchor.selected.clone());
        GraphExplorerSelection {
            subject: identity.clone(),
            represented_by: identity,
            reason: GraphExplorerAnchorReason::DetailAnchor,
        }
    })
}

fn no_selection_reason(
    conditions: &[GraphExplorerCondition],
    selection: Option<&GraphExplorerSelection>,
) -> Option<GraphExplorerAnchorReason> {
    if selection.is_none()
        && conditions
            .iter()
            .any(|condition| matches!(condition, GraphExplorerCondition::NoMilestones))
    {
        Some(GraphExplorerAnchorReason::NoMilestone)
    } else {
        None
    }
}

fn state_for(conditions: &[GraphExplorerCondition], empty: bool) -> GraphExplorerState {
    if conditions
        .iter()
        .any(|condition| matches!(condition, GraphExplorerCondition::InvalidTaskCycle { .. }))
    {
        GraphExplorerState::InvalidTaskCycle
    } else if empty {
        GraphExplorerState::Empty
    } else {
        GraphExplorerState::Complete
    }
}

fn is_milestone(row: &TaskBrowserRow) -> bool {
    row.tags.iter().any(|tag| tag == MILESTONE_TAG)
}

fn connected_component_count(nodes: &[GraphExplorerNode], edges: &[GraphExplorerEdge]) -> usize {
    let mut adjacency: BTreeMap<GraphExplorerIdentity, BTreeSet<GraphExplorerIdentity>> = nodes
        .iter()
        .map(|node| (node.identity.clone(), BTreeSet::new()))
        .collect();
    for edge in edges {
        adjacency
            .entry(edge.from.clone())
            .or_default()
            .insert(edge.to.clone());
        adjacency
            .entry(edge.to.clone())
            .or_default()
            .insert(edge.from.clone());
    }
    let mut unseen: BTreeSet<GraphExplorerIdentity> = adjacency.keys().cloned().collect();
    let mut components = 0;
    while let Some(start) = unseen.iter().next().cloned() {
        components += 1;
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            if !unseen.remove(&node) {
                continue;
            }
            stack.extend(adjacency.get(&node).into_iter().flatten().cloned());
        }
    }
    components
}

/// Deterministic strongly connected components which actually contain a cycle.
fn cyclic_components<T>(nodes: &BTreeSet<T>, edges: &[(T, T)]) -> Vec<Vec<T>>
where
    T: Clone + Ord,
{
    let mut adjacency: BTreeMap<T, Vec<T>> = nodes
        .iter()
        .cloned()
        .map(|node| (node, Vec::new()))
        .collect();
    let mut reverse = adjacency.clone();
    for (from, to) in edges {
        adjacency.entry(from.clone()).or_default().push(to.clone());
        adjacency.entry(to.clone()).or_default();
        reverse.entry(to.clone()).or_default().push(from.clone());
        reverse.entry(from.clone()).or_default();
    }
    for neighbours in adjacency.values_mut().chain(reverse.values_mut()) {
        neighbours.sort();
        neighbours.dedup();
    }

    // Iterative Kosaraju avoids making source graph depth equal call-stack
    // depth. A 10,000-node chain is a normal search source, not a stack-overflow
    // fixture.
    let mut visited = BTreeSet::new();
    let mut finish_order = Vec::new();
    for start in adjacency.keys() {
        if visited.contains(start) {
            continue;
        }
        let mut stack = vec![(start.clone(), false)];
        while let Some((node, expanded)) = stack.pop() {
            if expanded {
                finish_order.push(node);
                continue;
            }
            if !visited.insert(node.clone()) {
                continue;
            }
            stack.push((node.clone(), true));
            for next in adjacency.get(&node).into_iter().flatten().rev() {
                if !visited.contains(next) {
                    stack.push((next.clone(), false));
                }
            }
        }
    }

    let mut assigned = BTreeSet::new();
    let mut components = Vec::new();
    for start in finish_order.into_iter().rev() {
        if !assigned.insert(start.clone()) {
            continue;
        }
        let mut component = Vec::new();
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            component.push(node.clone());
            for next in reverse.get(&node).into_iter().flatten().rev() {
                if assigned.insert(next.clone()) {
                    stack.push(next.clone());
                }
            }
        }
        component.sort();
        components.push(component);
    }
    components.retain(|component| {
        component.len() > 1
            || component
                .first()
                .is_some_and(|node| edges.iter().any(|(from, to)| from == node && to == node))
    });
    components.sort();
    components
}
