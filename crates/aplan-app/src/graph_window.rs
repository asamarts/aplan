//! Deterministic, fully accounted windows over a semantic graph projection.
//!
//! A window is not a clipped [`aplan_output::GraphView`]. It names its anchor,
//! budgets, source size, exact omissions and typed ways to reach structure
//! outside the current view. Selector, scope and membership policy have already
//! been applied by [`crate::GraphWorkspace`] before this module sees the graph.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use aplan_core::EdgeKind;
use aplan_output::graph_scene::GraphScene;
use aplan_output::{GraphEdge, GraphNode, GraphView};

use crate::{GraphProjection, GraphProjectionState, InputIdentity};

/// Independent limits on one interactive graph window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphWindowBudget {
    pub nodes: usize,
    pub edges: usize,
    pub lanes: usize,
}

impl Default for GraphWindowBudget {
    fn default() -> Self {
        Self {
            nodes: 64,
            edges: 96,
            lanes: 20,
        }
    }
}

/// Which side of a visible boundary contains omitted graph structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GraphContinuationDirection {
    Incoming,
    Outgoing,
    /// A disconnected component has no visible boundary edge.
    Component,
}

/// Stable identity of one continuation family.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GraphContinuationKey {
    /// Visible boundary node, or the deterministic root of a disconnected
    /// component.
    pub boundary: String,
    pub direction: GraphContinuationDirection,
    /// Relationship meaning for incoming/outgoing continuations. Components
    /// have no relationship kind.
    pub kind: Option<EdgeKind>,
}

/// A deterministic page request for one continuation family.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GraphContinuationCursor {
    /// Identity of the immutable application snapshot which emitted this
    /// cursor. A cursor must never be reinterpreted against refreshed input.
    pub input_sha256: String,
    pub key: GraphContinuationKey,
    /// Half-open range in the family's canonical remote-node order. Separate
    /// ranges prevent a continuation from redisplaying siblings already in
    /// the window merely because omitted siblings occur on both sides.
    pub start: usize,
    pub offset: usize,
    pub end: usize,
}

/// Why a continuation is offered from the current window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GraphContinuationTravel {
    Enter,
    Earlier,
    Later,
}

/// Stable identity and exact local accounting for one available traversal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GraphContinuation {
    pub cursor: GraphContinuationCursor,
    pub travel: GraphContinuationTravel,
    /// Distinct omitted remote nodes reached by this traversal.
    pub omitted_nodes: usize,
    /// Omitted boundary edges reached by this traversal. This is not intended
    /// to partition `omitted_edges`: edges wholly beyond the boundary are
    /// accounted globally and become boundary edges in a later window.
    pub omitted_edges: usize,
}

/// One bounded projection request. A cursor is emitted by an earlier answer;
/// without one the workspace chooses a useful deterministic neighbourhood.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphWindowRequest {
    pub preferred_anchor: Option<String>,
    pub budget: GraphWindowBudget,
    pub continuation: Option<GraphContinuationCursor>,
}

/// How the requested anchor resolved against this immutable projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphWindowAnchor {
    pub selected: String,
    pub requested: Option<String>,
    /// Present only when a requested identity disappeared or was excluded and
    /// the engine selected a deterministic replacement.
    pub replacement_reason: Option<String>,
}

/// Navigation metadata for a high-degree continuation page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphWindowPage {
    pub current: GraphContinuationCursor,
    pub previous: Option<GraphContinuationCursor>,
    pub next: Option<GraphContinuationCursor>,
    pub shown_nodes: usize,
    pub total_nodes: usize,
}

/// A bounded graph is a first-class semantic state, not a renderer error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphWindowState {
    Complete,
    Windowed,
    Empty,
    /// The selected semantic projection is cyclic. `windowed` still states
    /// whether all source facts fit this answer.
    Cyclic {
        windowed: bool,
    },
    Failed(GraphWindowFailure),
}

/// A request failure which cannot truthfully be represented as an empty graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphWindowFailure {
    NoNodeCapacity,
    SourceUnavailable(GraphProjectionState),
    InvalidContinuation(GraphContinuationCursor),
    RelationshipCannotFit(GraphContinuationKey),
}

/// One deterministic bounded graph answer.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphWindowProjection {
    pub input: InputIdentity,
    pub state: GraphWindowState,
    pub source_state: GraphProjectionState,
    pub anchor: Option<GraphWindowAnchor>,
    pub view: GraphView,
    pub budget: GraphWindowBudget,
    pub required_lanes: usize,
    pub source_nodes: usize,
    pub source_edges: usize,
    pub omitted_nodes: usize,
    pub omitted_edges: usize,
    pub continuations: Vec<GraphContinuation>,
    pub page: Option<GraphWindowPage>,
    pub critical_path: Vec<String>,
}

impl GraphWindowProjection {
    #[must_use]
    pub fn is_windowed(&self) -> bool {
        matches!(
            self.state,
            GraphWindowState::Windowed | GraphWindowState::Cyclic { windowed: true }
        )
    }
}

pub(crate) fn project_window(
    source: &GraphProjection,
    request: &GraphWindowRequest,
) -> GraphWindowProjection {
    if let Some(terminal) = terminal_source_answer(source, request.budget) {
        return terminal;
    }

    let index = GraphIndex::new(&source.view);
    let selection = if let Some(cursor) = &request.continuation {
        match select_continuation_page(&index, cursor, request.budget, &source.input.sha256) {
            Ok(page) => page,
            Err(failure) => {
                return answer(
                    source,
                    request.budget,
                    GraphWindowState::Failed(failure),
                    None,
                    empty_view(source),
                    Vec::new(),
                    None,
                );
            }
        }
    } else {
        initial_selection(source, &index, request)
    };

    let view = index.view(&selection.selection, true);
    let omitted_nodes = source.view.nodes.len().saturating_sub(view.nodes.len());
    let omitted_edges = source.view.edges.len().saturating_sub(view.edges.len());
    let cyclic = is_cyclic(&source.view);
    let windowed = omitted_nodes > 0 || omitted_edges > 0;
    let state = if cyclic {
        GraphWindowState::Cyclic { windowed }
    } else if windowed {
        GraphWindowState::Windowed
    } else {
        GraphWindowState::Complete
    };
    let continuations = continuations(
        &index,
        &selection.selection,
        selection.active_group.as_ref(),
        &source.input.sha256,
    );
    answer(
        source,
        request.budget,
        state,
        Some(selection.anchor),
        view,
        continuations,
        selection.page,
    )
}

fn terminal_source_answer(
    source: &GraphProjection,
    budget: GraphWindowBudget,
) -> Option<GraphWindowProjection> {
    let state = if budget.nodes == 0 {
        Some(GraphWindowState::Failed(GraphWindowFailure::NoNodeCapacity))
    } else {
        match &source.state {
            GraphProjectionState::UnknownTask(_) => Some(GraphWindowState::Failed(
                GraphWindowFailure::SourceUnavailable(source.state.clone()),
            )),
            GraphProjectionState::CyclicCriticalPath => {
                Some(GraphWindowState::Cyclic { windowed: false })
            }
            GraphProjectionState::EmptyScope if source.view.nodes.is_empty() => {
                Some(GraphWindowState::Empty)
            }
            GraphProjectionState::Complete
            | GraphProjectionState::EmptyScope
            | GraphProjectionState::NoEdges => None,
        }
    }?;
    Some(answer(
        source,
        budget,
        state,
        None,
        empty_view(source),
        Vec::new(),
        None,
    ))
}

fn empty_view(source: &GraphProjection) -> GraphView {
    GraphView {
        nodes: Vec::new(),
        edges: Vec::new(),
        hidden_edges: source.view.hidden_edges,
        grouping: source.view.grouping.clone(),
        scope: source.view.scope.clone(),
    }
}

fn initial_selection(
    source: &GraphProjection,
    index: &GraphIndex<'_>,
    request: &GraphWindowRequest,
) -> PageSelection {
    let anchor = choose_anchor(source, request.preferred_anchor.as_deref())
        .expect("a non-terminal source has a visible node");
    let order = index.walk_from(&anchor.selected);
    let mut selected = Selection::default();
    for candidate in order {
        let next = index.with_candidate(&selected, &candidate);
        if !fits(index, &next, request.budget) {
            break;
        }
        selected = next;
    }
    PageSelection {
        selection: selected,
        anchor,
        page: None,
        active_group: None,
    }
}

fn answer(
    source: &GraphProjection,
    budget: GraphWindowBudget,
    state: GraphWindowState,
    anchor: Option<GraphWindowAnchor>,
    view: GraphView,
    continuations: Vec<GraphContinuation>,
    page: Option<GraphWindowPage>,
) -> GraphWindowProjection {
    let source_nodes = source.view.nodes.len();
    let source_edges = source.view.edges.len();
    GraphWindowProjection {
        input: source.input.clone(),
        state,
        source_state: source.state.clone(),
        anchor,
        required_lanes: GraphScene::required_lanes(&view),
        omitted_nodes: source_nodes.saturating_sub(view.nodes.len()),
        omitted_edges: source_edges.saturating_sub(view.edges.len()),
        source_nodes,
        source_edges,
        view,
        budget,
        continuations,
        page,
        critical_path: source.critical_path.clone(),
    }
}

fn choose_anchor(source: &GraphProjection, requested: Option<&str>) -> Option<GraphWindowAnchor> {
    if let Some(requested) = requested
        && source.contains(requested)
    {
        return Some(GraphWindowAnchor {
            selected: requested.to_string(),
            requested: Some(requested.to_string()),
            replacement_reason: None,
        });
    }
    let visible: BTreeSet<&str> = source
        .view
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect();
    let selected = source
        .critical_path
        .iter()
        .find(|id| {
            visible.contains(id.as_str())
                && source
                    .view
                    .nodes
                    .iter()
                    .find(|node| node.id == **id)
                    .is_some_and(|node| !node.done)
        })
        .cloned()
        .or_else(|| {
            source
                .view
                .nodes
                .iter()
                .filter(|node| !node.done)
                .map(|node| &node.id)
                .min()
                .cloned()
        })
        .or_else(|| source.view.nodes.iter().map(|node| &node.id).min().cloned())?;
    let replacement_reason = requested.map(|requested| {
        source.exclusion(requested).map_or_else(
            || format!("`{requested}` is no longer present in this graph projection"),
            str::to_string,
        )
    });
    Some(GraphWindowAnchor {
        selected,
        requested: requested.map(str::to_string),
        replacement_reason,
    })
}

#[derive(Debug)]
struct GraphIndex<'a> {
    base: &'a GraphView,
    nodes: BTreeMap<&'a str, &'a GraphNode>,
    edges: Vec<&'a GraphEdge>,
    adjacency: BTreeMap<&'a str, Vec<usize>>,
}

impl<'a> GraphIndex<'a> {
    fn new(base: &'a GraphView) -> Self {
        let nodes: BTreeMap<&str, &GraphNode> = base
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect();
        let mut edges: Vec<&GraphEdge> = base.edges.iter().collect();
        edges.sort_by(|left, right| {
            (&left.from, &left.to, left.kind).cmp(&(&right.from, &right.to, right.kind))
        });
        let mut adjacency: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (edge_index, edge) in edges.iter().enumerate() {
            if nodes.contains_key(edge.from.as_str()) && nodes.contains_key(edge.to.as_str()) {
                adjacency
                    .entry(edge.from.as_str())
                    .or_default()
                    .push(edge_index);
                if edge.from != edge.to {
                    adjacency
                        .entry(edge.to.as_str())
                        .or_default()
                        .push(edge_index);
                }
            }
        }
        Self {
            base,
            nodes,
            edges,
            adjacency,
        }
    }

    fn with_candidate(&self, selected: &Selection, id: &str) -> Selection {
        let mut next = selected.clone();
        if !self.nodes.contains_key(id) || !next.nodes.insert(id.to_string()) {
            return next;
        }
        for edge_index in self.adjacency.get(id).into_iter().flatten() {
            let edge = self.edges[*edge_index];
            if next.nodes.contains(&edge.from) && next.nodes.contains(&edge.to) {
                next.edges.insert(*edge_index);
            }
        }
        next
    }

    fn view(&self, selected: &Selection, account_crossings: bool) -> GraphView {
        let crossing = if account_crossings {
            self.edges
                .iter()
                .filter(|edge| {
                    selected.nodes.contains(&edge.from) ^ selected.nodes.contains(&edge.to)
                })
                .count()
        } else {
            0
        };
        GraphView {
            nodes: selected
                .nodes
                .iter()
                .filter_map(|id| self.nodes.get(id.as_str()).map(|node| (*node).clone()))
                .collect(),
            edges: selected
                .edges
                .iter()
                .map(|edge| self.edges[*edge].clone())
                .collect(),
            hidden_edges: self.base.hidden_edges.saturating_add(crossing),
            grouping: self.base.grouping.clone(),
            scope: self.base.scope.clone(),
        }
    }

    fn neighbours(&self, id: &str) -> Vec<String> {
        let mut neighbours = BTreeSet::new();
        for edge_index in self.adjacency.get(id).into_iter().flatten() {
            let edge = self.edges[*edge_index];
            let (priority, remote) = if edge.to == id {
                (
                    if edge.kind == EdgeKind::Dependency {
                        0
                    } else {
                        2
                    },
                    edge.from.as_str(),
                )
            } else {
                (
                    if edge.kind == EdgeKind::Dependency {
                        1
                    } else {
                        3
                    },
                    edge.to.as_str(),
                )
            };
            if remote != id {
                neighbours.insert((priority, remote));
            }
        }
        neighbours
            .into_iter()
            .map(|(_, id)| id.to_string())
            .collect()
    }

    fn walk_from(&self, anchor: &str) -> Vec<String> {
        let mut roots = Vec::with_capacity(self.nodes.len());
        roots.push(anchor.to_string());
        roots.extend(
            self.nodes
                .keys()
                .filter(|id| **id != anchor)
                .map(|id| (*id).to_string()),
        );
        let mut seen = BTreeSet::new();
        let mut order = Vec::with_capacity(self.nodes.len());
        for root in roots {
            if !seen.insert(root.clone()) {
                continue;
            }
            let mut queue = VecDeque::from([root]);
            while let Some(id) = queue.pop_front() {
                order.push(id.clone());
                for neighbour in self.neighbours(&id) {
                    if seen.insert(neighbour.clone()) {
                        queue.push_back(neighbour);
                    }
                }
            }
        }
        order
    }

    fn component_from(&self, anchor: &str) -> Vec<String> {
        if !self.nodes.contains_key(anchor) {
            return Vec::new();
        }
        let mut seen = BTreeSet::from([anchor.to_string()]);
        let mut queue = VecDeque::from([anchor.to_string()]);
        let mut order = Vec::new();
        while let Some(id) = queue.pop_front() {
            order.push(id.clone());
            for neighbour in self.neighbours(&id) {
                if seen.insert(neighbour.clone()) {
                    queue.push_back(neighbour);
                }
            }
        }
        order
    }

    fn group_nodes(&self, key: &GraphContinuationKey) -> Option<Vec<String>> {
        if key.direction == GraphContinuationDirection::Component {
            return (key.kind.is_none() && self.nodes.contains_key(key.boundary.as_str()))
                .then(|| self.component_from(&key.boundary));
        }
        let kind = key.kind?;
        let mut remote = BTreeSet::new();
        for edge_index in self.adjacency.get(key.boundary.as_str())? {
            let edge = self.edges[*edge_index];
            if edge.kind != kind {
                continue;
            }
            match key.direction {
                GraphContinuationDirection::Incoming if edge.to == key.boundary => {
                    remote.insert(edge.from.clone());
                }
                GraphContinuationDirection::Outgoing if edge.from == key.boundary => {
                    remote.insert(edge.to.clone());
                }
                GraphContinuationDirection::Incoming
                | GraphContinuationDirection::Outgoing
                | GraphContinuationDirection::Component => {}
            }
        }
        (!remote.is_empty()).then(|| remote.into_iter().collect())
    }
}

#[derive(Debug, Clone, Default)]
struct Selection {
    nodes: BTreeSet<String>,
    edges: BTreeSet<usize>,
}

fn fits(index: &GraphIndex<'_>, selection: &Selection, budget: GraphWindowBudget) -> bool {
    if selection.nodes.len() > budget.nodes || selection.edges.len() > budget.edges {
        return false;
    }
    GraphScene::required_lanes(&index.view(selection, false)) <= budget.lanes
}

#[derive(Debug)]
struct PageSelection {
    selection: Selection,
    anchor: GraphWindowAnchor,
    page: Option<GraphWindowPage>,
    active_group: Option<ActiveGroup>,
}

#[derive(Debug)]
struct ActiveGroup {
    key: GraphContinuationKey,
    range_start: usize,
    range_end: usize,
    offset: usize,
    end: usize,
    nodes: Vec<String>,
}

fn select_continuation_page(
    index: &GraphIndex<'_>,
    cursor: &GraphContinuationCursor,
    budget: GraphWindowBudget,
    input_sha256: &str,
) -> Result<PageSelection, GraphWindowFailure> {
    if cursor.input_sha256 != input_sha256 {
        return Err(GraphWindowFailure::InvalidContinuation(cursor.clone()));
    }
    let Some(nodes) = index.group_nodes(&cursor.key) else {
        return Err(GraphWindowFailure::InvalidContinuation(cursor.clone()));
    };
    if cursor.start > cursor.offset || cursor.offset >= cursor.end || cursor.end > nodes.len() {
        return Err(GraphWindowFailure::InvalidContinuation(cursor.clone()));
    }
    if cursor.key.direction == GraphContinuationDirection::Component {
        if cursor.start != 0 || cursor.offset != 0 || cursor.end != nodes.len() {
            return Err(GraphWindowFailure::InvalidContinuation(cursor.clone()));
        }
        let mut selection = Selection::default();
        for candidate in &nodes {
            let next = index.with_candidate(&selection, candidate);
            if !fits(index, &next, budget) {
                break;
            }
            selection = next;
        }
        return Ok(PageSelection {
            selection,
            anchor: GraphWindowAnchor {
                selected: cursor.key.boundary.clone(),
                requested: Some(cursor.key.boundary.clone()),
                replacement_reason: None,
            },
            page: None,
            active_group: None,
        });
    }

    let mut selection = index.with_candidate(&Selection::default(), &cursor.key.boundary);
    if selection.nodes.is_empty() {
        return Err(GraphWindowFailure::InvalidContinuation(cursor.clone()));
    }
    let mut end = cursor.offset;
    for candidate in &nodes[cursor.offset..cursor.end] {
        let next = index.with_candidate(&selection, candidate);
        if !fits(index, &next, budget) {
            break;
        }
        selection = next;
        end += 1;
    }
    if end == cursor.offset {
        return Err(GraphWindowFailure::RelationshipCannotFit(
            cursor.key.clone(),
        ));
    }
    let previous_offset = previous_page_offset(
        index,
        &cursor.key,
        &nodes,
        cursor.start,
        cursor.offset,
        budget,
    )?;
    let previous = previous_offset.map(|offset| GraphContinuationCursor {
        input_sha256: cursor.input_sha256.clone(),
        key: cursor.key.clone(),
        start: cursor.start,
        offset,
        end: cursor.end,
    });
    let next = (end < cursor.end).then(|| GraphContinuationCursor {
        input_sha256: cursor.input_sha256.clone(),
        key: cursor.key.clone(),
        start: cursor.start,
        offset: end,
        end: cursor.end,
    });
    Ok(PageSelection {
        selection,
        anchor: GraphWindowAnchor {
            selected: cursor.key.boundary.clone(),
            requested: Some(cursor.key.boundary.clone()),
            replacement_reason: None,
        },
        page: Some(GraphWindowPage {
            current: cursor.clone(),
            previous,
            next,
            shown_nodes: end - cursor.offset,
            total_nodes: cursor.end - cursor.start,
        }),
        active_group: Some(ActiveGroup {
            key: cursor.key.clone(),
            range_start: cursor.start,
            range_end: cursor.end,
            offset: cursor.offset,
            end,
            nodes,
        }),
    })
}

fn previous_page_offset(
    index: &GraphIndex<'_>,
    key: &GraphContinuationKey,
    nodes: &[String],
    range_start: usize,
    target: usize,
    budget: GraphWindowBudget,
) -> Result<Option<usize>, GraphWindowFailure> {
    if target == range_start {
        return Ok(None);
    }
    let mut start = range_start;
    let mut previous = None;
    while start < target {
        let mut selection = index.with_candidate(&Selection::default(), &key.boundary);
        let page_start = start;
        for candidate in &nodes[start..target] {
            let next = index.with_candidate(&selection, candidate);
            if !fits(index, &next, budget) {
                break;
            }
            selection = next;
            start += 1;
        }
        if start == page_start {
            return Err(GraphWindowFailure::RelationshipCannotFit(key.clone()));
        }
        if start >= target {
            return Ok(Some(page_start));
        }
        previous = Some(page_start);
    }
    Ok(previous)
}

fn continuations(
    index: &GraphIndex<'_>,
    selection: &Selection,
    active: Option<&ActiveGroup>,
    input_sha256: &str,
) -> Vec<GraphContinuation> {
    let mut out = boundary_continuations(index, selection, active, input_sha256);
    if let Some(active) = active {
        push_active_page_continuations(&mut out, index, active, input_sha256);
    }
    push_component_continuations(&mut out, index, selection, input_sha256);
    out.sort();
    out
}

fn boundary_continuations(
    index: &GraphIndex<'_>,
    selection: &Selection,
    active: Option<&ActiveGroup>,
    input_sha256: &str,
) -> Vec<GraphContinuation> {
    let mut groups = BTreeSet::new();
    for edge in &index.edges {
        let from = selection.nodes.contains(&edge.from);
        let to = selection.nodes.contains(&edge.to);
        let (boundary, direction) = match (from, to) {
            (false, true) => (edge.to.clone(), GraphContinuationDirection::Incoming),
            (true, false) => (edge.from.clone(), GraphContinuationDirection::Outgoing),
            (true, true) | (false, false) => continue,
        };
        let key = GraphContinuationKey {
            boundary,
            direction,
            kind: Some(edge.kind),
        };
        groups.insert(key);
    }

    let mut out = Vec::new();
    for key in groups {
        if active.is_some_and(|active| active.key == key) {
            continue;
        }
        let nodes = index
            .group_nodes(&key)
            .expect("a crossing group has remote nodes");
        let mut start = None;
        for (offset, node) in nodes.iter().enumerate() {
            let omitted = !selection.nodes.contains(node);
            match (start, omitted) {
                (None, true) => start = Some(offset),
                (Some(run_start), false) => {
                    push_group_continuation(
                        &mut out,
                        index,
                        &key,
                        &nodes,
                        run_start,
                        offset,
                        input_sha256,
                    );
                    start = None;
                }
                (None, false) | (Some(_), true) => {}
            }
        }
        if let Some(run_start) = start {
            push_group_continuation(
                &mut out,
                index,
                &key,
                &nodes,
                run_start,
                nodes.len(),
                input_sha256,
            );
        }
    }
    out
}

fn push_active_page_continuations(
    out: &mut Vec<GraphContinuation>,
    index: &GraphIndex<'_>,
    active: &ActiveGroup,
    input_sha256: &str,
) {
    if active.offset > active.range_start {
        let earlier: BTreeSet<&str> = active.nodes[active.range_start..active.offset]
            .iter()
            .map(String::as_str)
            .collect();
        out.push(GraphContinuation {
            cursor: GraphContinuationCursor {
                input_sha256: input_sha256.to_string(),
                key: active.key.clone(),
                start: active.range_start,
                offset: active.range_start,
                end: active.range_end,
            },
            travel: GraphContinuationTravel::Earlier,
            omitted_nodes: earlier.len(),
            omitted_edges: group_edge_count(index, &active.key, &earlier),
        });
    }
    if active.end < active.range_end {
        let later: BTreeSet<&str> = active.nodes[active.end..active.range_end]
            .iter()
            .map(String::as_str)
            .collect();
        out.push(GraphContinuation {
            cursor: GraphContinuationCursor {
                input_sha256: input_sha256.to_string(),
                key: active.key.clone(),
                start: active.range_start,
                offset: active.end,
                end: active.range_end,
            },
            travel: GraphContinuationTravel::Later,
            omitted_nodes: later.len(),
            omitted_edges: group_edge_count(index, &active.key, &later),
        });
    }
}

fn push_component_continuations(
    out: &mut Vec<GraphContinuation>,
    index: &GraphIndex<'_>,
    selection: &Selection,
    input_sha256: &str,
) {
    for component in disconnected_components(index, &selection.nodes) {
        let root = component.iter().next().expect("a component is non-empty");
        let edges = index
            .edges
            .iter()
            .filter(|edge| component.contains(&edge.from) && component.contains(&edge.to))
            .count();
        out.push(GraphContinuation {
            cursor: GraphContinuationCursor {
                input_sha256: input_sha256.to_string(),
                key: GraphContinuationKey {
                    boundary: root.clone(),
                    direction: GraphContinuationDirection::Component,
                    kind: None,
                },
                start: 0,
                offset: 0,
                end: component.len(),
            },
            travel: GraphContinuationTravel::Enter,
            omitted_nodes: component.len(),
            omitted_edges: edges,
        });
    }
}

fn push_group_continuation(
    out: &mut Vec<GraphContinuation>,
    index: &GraphIndex<'_>,
    key: &GraphContinuationKey,
    nodes: &[String],
    start: usize,
    end: usize,
    input_sha256: &str,
) {
    let remote: BTreeSet<&str> = nodes[start..end].iter().map(String::as_str).collect();
    out.push(GraphContinuation {
        cursor: GraphContinuationCursor {
            input_sha256: input_sha256.to_string(),
            key: key.clone(),
            start,
            offset: start,
            end,
        },
        travel: GraphContinuationTravel::Enter,
        omitted_nodes: remote.len(),
        omitted_edges: group_edge_count(index, key, &remote),
    });
}

fn group_edge_count(
    index: &GraphIndex<'_>,
    key: &GraphContinuationKey,
    remote: &BTreeSet<&str>,
) -> usize {
    index
        .edges
        .iter()
        .filter(|edge| {
            edge.kind == key.kind.expect("an edge group has a kind")
                && match key.direction {
                    GraphContinuationDirection::Incoming => {
                        edge.to == key.boundary && remote.contains(edge.from.as_str())
                    }
                    GraphContinuationDirection::Outgoing => {
                        edge.from == key.boundary && remote.contains(edge.to.as_str())
                    }
                    GraphContinuationDirection::Component => false,
                }
        })
        .count()
}

fn disconnected_components(
    index: &GraphIndex<'_>,
    visible: &BTreeSet<String>,
) -> Vec<BTreeSet<String>> {
    let mut remaining: BTreeSet<String> = index
        .nodes
        .keys()
        .filter(|id| !visible.contains(**id))
        .map(|id| (*id).to_string())
        .collect();
    let mut components = Vec::new();
    while let Some(root) = remaining.pop_first() {
        let mut component = BTreeSet::from([root.clone()]);
        let mut queue = VecDeque::from([root]);
        let mut touches_visible = false;
        while let Some(id) = queue.pop_front() {
            for neighbour in index.neighbours(&id) {
                if visible.contains(&neighbour) {
                    touches_visible = true;
                } else if remaining.remove(&neighbour) {
                    component.insert(neighbour.clone());
                    queue.push_back(neighbour);
                }
            }
        }
        if !touches_visible {
            components.push(component);
        }
    }
    components
}

fn is_cyclic(view: &GraphView) -> bool {
    let nodes: BTreeSet<&str> = view.nodes.iter().map(|node| node.id.as_str()).collect();
    let mut successors: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut indegree: BTreeMap<&str, usize> = nodes.iter().map(|id| (*id, 0)).collect();
    for edge in &view.edges {
        if !nodes.contains(edge.from.as_str()) || !nodes.contains(edge.to.as_str()) {
            continue;
        }
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
    let mut visited = 0usize;
    while let Some(id) = ready.pop_first() {
        visited += 1;
        for successor in successors.get(id).into_iter().flatten() {
            let degree = indegree.get_mut(*successor).expect("a known successor");
            *degree -= 1;
            if *degree == 0 {
                ready.insert(successor);
            }
        }
    }
    visited != nodes.len()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use aplan_core::Status;
    use aplan_output::Grouping;

    use super::*;
    use crate::{ConfigurationContentIdentity, ConfigurationIdentity, RepositoryIdentity, Trust};

    fn input() -> InputIdentity {
        InputIdentity {
            source: "graph-window-test",
            plan_root: PathBuf::from("/plan"),
            plan_sha256: "plan".to_string(),
            repository_root: PathBuf::from("/repo"),
            repository: RepositoryIdentity::NotObserved {
                reason: "hermetic test".to_string(),
            },
            configuration: ConfigurationIdentity {
                path: None,
                sha256: "config".to_string(),
                content: ConfigurationContentIdentity::Absent,
            },
            trust: Trust::Hermetic,
            sha256: "input".to_string(),
        }
    }

    fn projection(
        ids: impl IntoIterator<Item = String>,
        edges: impl IntoIterator<Item = (String, String, EdgeKind)>,
        state: GraphProjectionState,
    ) -> GraphProjection {
        let nodes: Vec<GraphNode> = ids
            .into_iter()
            .map(|id| GraphNode {
                title: format!("task {id}"),
                id,
                status: Status::Todo,
                done: false,
            })
            .collect();
        let admitted = nodes.iter().map(|node| node.id.clone()).collect();
        let critical_path = nodes.iter().map(|node| node.id.clone()).collect();
        GraphProjection {
            input: input(),
            view: GraphView {
                nodes,
                edges: edges
                    .into_iter()
                    .map(|(from, to, kind)| GraphEdge { from, to, kind })
                    .collect(),
                hidden_edges: 0,
                grouping: Grouping::default(),
                scope: vec!["test scope".to_string()],
            },
            state,
            admitted,
            critical_path,
            exclusions: BTreeMap::new(),
        }
    }

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn dependency(from: &str, to: &str) -> (String, String, EdgeKind) {
        (from.to_string(), to.to_string(), EdgeKind::Dependency)
    }

    fn budget(nodes: usize, edges: usize, lanes: usize) -> GraphWindowBudget {
        GraphWindowBudget {
            nodes,
            edges,
            lanes,
        }
    }

    #[test]
    fn complete_window_keeps_the_preferred_anchor_and_every_source_fact() {
        let source = projection(
            ids(&["A", "B", "C"]),
            [dependency("A", "B"), dependency("B", "C")],
            GraphProjectionState::Complete,
        );
        let answer = project_window(
            &source,
            &GraphWindowRequest {
                preferred_anchor: Some("B".to_string()),
                budget: budget(3, 2, 1),
                continuation: None,
            },
        );
        assert_eq!(answer.state, GraphWindowState::Complete);
        assert_eq!(
            answer
                .anchor
                .as_ref()
                .map(|anchor| anchor.selected.as_str()),
            Some("B")
        );
        assert_eq!(answer.source_nodes, 3);
        assert_eq!(answer.source_edges, 2);
        assert_eq!(answer.omitted_nodes, 0);
        assert_eq!(answer.omitted_edges, 0);
        assert_eq!(answer.required_lanes, 1);
        assert!(answer.continuations.is_empty());
    }

    /// Proof for `tui.graph_windows_are_bounded_and_accounted`.
    #[test]
    fn bounded_graph_windows_account_for_every_fact_and_preserve_anchor_identity() {
        let source = projection(
            ids(&["A", "B", "C", "D", "E"]),
            [
                dependency("A", "B"),
                dependency("B", "C"),
                dependency("C", "D"),
                dependency("D", "E"),
            ],
            GraphProjectionState::Complete,
        );
        let answer = project_window(
            &source,
            &GraphWindowRequest {
                preferred_anchor: Some("B".to_string()),
                budget: budget(3, 2, 1),
                continuation: None,
            },
        );
        assert_eq!(answer.state, GraphWindowState::Windowed);
        assert_eq!(
            answer
                .anchor
                .as_ref()
                .map(|anchor| anchor.selected.as_str()),
            Some("B")
        );
        assert_eq!(
            answer.view.nodes.len() + answer.omitted_nodes,
            source.view.nodes.len()
        );
        assert_eq!(
            answer.view.edges.len() + answer.omitted_edges,
            source.view.edges.len()
        );
        assert!(answer.view.nodes.len() <= answer.budget.nodes);
        assert!(answer.view.edges.len() <= answer.budget.edges);
        assert!(answer.required_lanes <= answer.budget.lanes);
        assert!(!answer.continuations.is_empty());

        let replaced = project_window(
            &source,
            &GraphWindowRequest {
                preferred_anchor: Some("removed".to_string()),
                budget: budget(2, 1, 1),
                continuation: None,
            },
        );
        let anchor = replaced.anchor.expect("fallback anchor");
        assert_eq!(anchor.requested.as_deref(), Some("removed"));
        assert!(anchor.replacement_reason.is_some());
        assert_eq!(anchor.selected, "A");
    }

    /// Proof for `tui.graph_windows_are_bounded_and_accounted`.
    #[test]
    fn high_degree_continuations_page_without_repeating_or_losing_a_relationship() {
        let leaves: Vec<String> = (0..10).map(|index| format!("N{index:02}")).collect();
        let mut node_ids = vec!["H".to_string()];
        node_ids.extend(leaves.clone());
        let edges = leaves
            .iter()
            .map(|leaf| dependency("H", leaf))
            .collect::<Vec<_>>();
        let source = projection(node_ids, edges, GraphProjectionState::Complete);
        let request = GraphWindowRequest {
            preferred_anchor: Some("H".to_string()),
            budget: budget(4, 3, 3),
            continuation: None,
        };
        let first = project_window(&source, &request);
        let mut reached: BTreeSet<String> = first
            .view
            .nodes
            .iter()
            .filter(|node| node.id != "H")
            .map(|node| node.id.clone())
            .collect();
        let continuation = first
            .continuations
            .iter()
            .find(|continuation| {
                continuation.cursor.key.boundary == "H"
                    && continuation.cursor.key.direction == GraphContinuationDirection::Outgoing
            })
            .expect("remaining fan-out")
            .clone();
        assert_eq!(continuation.cursor.start, 3);
        assert_eq!(continuation.cursor.end, 10);
        assert_eq!(continuation.omitted_nodes, 7);
        assert_eq!(continuation.omitted_edges, 7);

        let mut cursor = Some(continuation.cursor);
        let mut prior = None;
        while let Some(current) = cursor {
            let page = project_window(
                &source,
                &GraphWindowRequest {
                    preferred_anchor: None,
                    budget: request.budget,
                    continuation: Some(current.clone()),
                },
            );
            for node in &page.view.nodes {
                if node.id != "H" {
                    assert!(reached.insert(node.id.clone()), "{} repeated", node.id);
                }
            }
            let metadata = page.page.expect("edge continuation page");
            assert_eq!(metadata.previous, prior);
            prior = Some(metadata.current);
            cursor = metadata.next;
        }
        assert_eq!(reached, leaves.into_iter().collect());
    }

    /// Proof for `tui.graph_windows_are_bounded_and_accounted`.
    #[test]
    fn every_five_node_dag_is_fully_reachable_through_bounded_continuations() {
        let node_ids = ids(&["A", "B", "C", "D", "E"]);
        let possible = [
            ("A", "B"),
            ("A", "C"),
            ("A", "D"),
            ("A", "E"),
            ("B", "C"),
            ("B", "D"),
            ("B", "E"),
            ("C", "D"),
            ("C", "E"),
            ("D", "E"),
        ];
        for mask in 0usize..(1usize << possible.len()) {
            let edges = possible
                .iter()
                .enumerate()
                .filter(|(bit, _)| (mask & (1 << bit)) != 0)
                .map(|(_, (from, to))| dependency(from, to))
                .collect::<Vec<_>>();
            let source = projection(
                node_ids.clone(),
                edges.clone(),
                GraphProjectionState::Complete,
            );
            let (reached_nodes, reached_edges) = traverse_all(&source, budget(2, 1, 1));
            assert_eq!(
                reached_nodes,
                node_ids.iter().cloned().collect(),
                "node loss for edge mask {mask:#x}"
            );
            assert_eq!(
                reached_edges,
                edges.into_iter().collect(),
                "edge loss for edge mask {mask:#x}"
            );
        }
    }

    fn traverse_all(
        source: &GraphProjection,
        window_budget: GraphWindowBudget,
    ) -> (BTreeSet<String>, BTreeSet<(String, String, EdgeKind)>) {
        let mut requests: VecDeque<Option<GraphContinuationCursor>> = VecDeque::from([None]);
        let mut seen = BTreeSet::new();
        let mut nodes = BTreeSet::new();
        let mut edges = BTreeSet::new();
        while let Some(cursor) = requests.pop_front() {
            if let Some(cursor) = &cursor
                && !seen.insert(cursor.clone())
            {
                continue;
            }
            let answer = project_window(
                source,
                &GraphWindowRequest {
                    preferred_anchor: Some("A".to_string()),
                    budget: window_budget,
                    continuation: cursor,
                },
            );
            assert!(
                !matches!(answer.state, GraphWindowState::Failed(_)),
                "an emitted continuation failed: {answer:?}"
            );
            nodes.extend(answer.view.nodes.into_iter().map(|node| node.id));
            edges.extend(
                answer
                    .view
                    .edges
                    .into_iter()
                    .map(|edge| (edge.from, edge.to, edge.kind)),
            );
            requests.extend(
                answer
                    .continuations
                    .into_iter()
                    .map(|continuation| Some(continuation.cursor)),
            );
        }
        (nodes, edges)
    }

    /// Proof for `tui.graph_windows_are_bounded_and_accounted`.
    #[test]
    fn cycles_membership_and_disconnected_components_remain_typed_and_reachable() {
        let source = projection(
            ids(&["A", "B", "C", "M", "X"]),
            [
                dependency("A", "B"),
                dependency("B", "A"),
                ("C".to_string(), "M".to_string(), EdgeKind::Membership),
            ],
            GraphProjectionState::Complete,
        );
        let answer = project_window(
            &source,
            &GraphWindowRequest {
                preferred_anchor: Some("A".to_string()),
                budget: budget(3, 2, 2),
                continuation: None,
            },
        );
        assert_eq!(answer.state, GraphWindowState::Cyclic { windowed: true });
        assert_eq!(answer.omitted_nodes, 2);
        assert_eq!(answer.omitted_edges, 1);
        assert!(
            answer
                .continuations
                .iter()
                .any(|continuation| { continuation.cursor.key.kind == Some(EdgeKind::Membership) })
        );
        let component = answer
            .continuations
            .iter()
            .find(|continuation| {
                continuation.cursor.key.direction == GraphContinuationDirection::Component
                    && continuation.cursor.key.boundary == "X"
            })
            .expect("isolated component");
        let reached = project_window(
            &source,
            &GraphWindowRequest {
                preferred_anchor: None,
                budget: budget(3, 2, 2),
                continuation: Some(component.cursor.clone()),
            },
        );
        assert!(reached.view.nodes.iter().any(|node| node.id == "X"));
    }

    /// Proof for `tui.graph_window_cursors_are_source_bound`.
    #[test]
    fn continuation_cursors_refuse_a_different_input_identity() {
        let source = projection(
            ids(&["A", "B", "C"]),
            [dependency("A", "B"), dependency("A", "C")],
            GraphProjectionState::Complete,
        );
        let first = project_window(
            &source,
            &GraphWindowRequest {
                preferred_anchor: Some("A".to_string()),
                budget: budget(2, 1, 1),
                continuation: None,
            },
        );
        let cursor = first
            .continuations
            .first()
            .expect("one omitted relationship")
            .cursor
            .clone();

        let mut refreshed = source;
        refreshed.input.sha256 = "different-input".to_string();
        let answer = project_window(
            &refreshed,
            &GraphWindowRequest {
                preferred_anchor: None,
                budget: budget(2, 1, 1),
                continuation: Some(cursor.clone()),
            },
        );

        assert_eq!(
            answer.state,
            GraphWindowState::Failed(GraphWindowFailure::InvalidContinuation(cursor))
        );
    }

    #[test]
    fn empty_cyclic_source_and_invalid_requests_do_not_impersonate_windows() {
        let empty_source = projection(Vec::new(), Vec::new(), GraphProjectionState::EmptyScope);
        assert_eq!(
            project_window(&empty_source, &GraphWindowRequest::default()).state,
            GraphWindowState::Empty
        );

        let cyclic_source = projection(
            Vec::new(),
            Vec::new(),
            GraphProjectionState::CyclicCriticalPath,
        );
        assert_eq!(
            project_window(&cyclic_source, &GraphWindowRequest::default()).state,
            GraphWindowState::Cyclic { windowed: false }
        );

        let unknown_source = projection(
            Vec::new(),
            Vec::new(),
            GraphProjectionState::UnknownTask("missing".to_string()),
        );
        assert_eq!(
            project_window(&unknown_source, &GraphWindowRequest::default()).state,
            GraphWindowState::Failed(GraphWindowFailure::SourceUnavailable(
                GraphProjectionState::UnknownTask("missing".to_string())
            ))
        );

        let source = projection(
            ids(&["A", "B"]),
            [dependency("A", "B")],
            GraphProjectionState::Complete,
        );
        assert_eq!(
            project_window(
                &source,
                &GraphWindowRequest {
                    preferred_anchor: None,
                    budget: budget(0, 0, 0),
                    continuation: None,
                }
            )
            .state,
            GraphWindowState::Failed(GraphWindowFailure::NoNodeCapacity)
        );
        let key = GraphContinuationKey {
            boundary: "A".to_string(),
            direction: GraphContinuationDirection::Outgoing,
            kind: Some(EdgeKind::Dependency),
        };
        let cursor = GraphContinuationCursor {
            input_sha256: source.input.sha256.clone(),
            key: key.clone(),
            start: 0,
            offset: 0,
            end: 1,
        };
        let invalid = GraphContinuationCursor {
            end: 2,
            ..cursor.clone()
        };
        assert_eq!(
            project_window(
                &source,
                &GraphWindowRequest {
                    preferred_anchor: None,
                    budget: budget(2, 1, 1),
                    continuation: Some(invalid.clone()),
                }
            )
            .state,
            GraphWindowState::Failed(GraphWindowFailure::InvalidContinuation(invalid))
        );
        assert_eq!(
            project_window(
                &source,
                &GraphWindowRequest {
                    preferred_anchor: None,
                    budget: budget(2, 0, 0),
                    continuation: Some(cursor),
                }
            )
            .state,
            GraphWindowState::Failed(GraphWindowFailure::RelationshipCannotFit(key))
        );
    }

    /// Proof for `tui.graph_windows_are_bounded_and_accounted`.
    #[test]
    fn ten_thousand_node_projection_has_bounded_output_and_deterministic_work() {
        let node_ids: Vec<String> = (0..10_000).map(|index| format!("N{index:05}")).collect();
        let edges = node_ids
            .windows(2)
            .map(|pair| dependency(&pair[0], &pair[1]))
            .collect::<Vec<_>>();
        let source = projection(node_ids, edges, GraphProjectionState::Complete);
        let request = GraphWindowRequest {
            preferred_anchor: Some("N05000".to_string()),
            budget: budget(32, 31, 2),
            continuation: None,
        };
        let first = project_window(&source, &request);
        let second = project_window(&source, &request);
        assert_eq!(first, second);
        assert_eq!(first.state, GraphWindowState::Windowed);
        assert!(first.view.nodes.len() <= 32);
        assert!(first.view.edges.len() <= 31);
        assert!(first.required_lanes <= 2);
        assert_eq!(first.view.nodes.len() + first.omitted_nodes, 10_000);
        assert_eq!(first.view.edges.len() + first.omitted_edges, 9_999);
        assert!(!first.continuations.is_empty());
    }

    #[test]
    fn source_order_cannot_change_window_or_continuation_identity() {
        let original_ids = ids(&["A", "B", "C", "D", "E", "F"]);
        let original_edges = vec![
            dependency("A", "C"),
            dependency("B", "C"),
            dependency("C", "D"),
            dependency("C", "E"),
            dependency("E", "F"),
        ];
        let mut reversed_ids = original_ids.clone();
        reversed_ids.reverse();
        let mut reversed_edges = original_edges.clone();
        reversed_edges.reverse();
        let request = GraphWindowRequest {
            preferred_anchor: Some("C".to_string()),
            budget: budget(4, 3, 3),
            continuation: None,
        };
        let original_source =
            projection(original_ids, original_edges, GraphProjectionState::Complete);
        let reversed_source =
            projection(reversed_ids, reversed_edges, GraphProjectionState::Complete);
        let original = project_window(&original_source, &request);
        let reversed = project_window(&reversed_source, &request);
        assert_eq!(original.view, reversed.view);
        assert_eq!(original.continuations, reversed.continuations);
        assert_eq!(original.state, reversed.state);
    }
}
