//! Ratatui painting for the bounded terminal-cell graph scene.
//!
//! The application layer owns graph meaning and scene geometry. This module
//! only translates validated cells into a clipped viewport. Painting and
//! pointer hit testing share [`GraphTransform`], so neither reconstructs an
//! identity from a border, label or edge glyph.

use std::collections::{BTreeMap, BTreeSet};

use aplan_app::{
    CellCardSize, CellEdgeIdentity, CellFeedbackKind, CellPoint, CellRect, CellRouteKind,
    CellScene, CellSceneBand, CellSceneDensity, CellSceneEdge, GraphExplorerAnchorReason,
    GraphExplorerIdentity, GraphExplorerNode, GraphExplorerProjection, Status,
};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget};

use crate::text;

/// User-selectable card density for one terminal session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum GraphDensity {
    #[default]
    Auto,
    Compact,
    Comfortable,
}

impl GraphDensity {
    pub(crate) const fn next(self) -> Self {
        match self {
            Self::Auto => Self::Compact,
            Self::Compact => Self::Comfortable,
            Self::Comfortable => Self::Auto,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Compact => "compact",
            Self::Comfortable => "comfortable",
        }
    }
}

/// Which exact subset of an unchanged scene is painted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum GraphEdgePresentation {
    #[default]
    All,
    RelationsOnly,
}

impl GraphEdgePresentation {
    pub(crate) const fn is_relations_only(self) -> bool {
        matches!(self, Self::RelationsOnly)
    }

    pub(crate) const fn toggled(self) -> Self {
        match self {
            Self::All => Self::RelationsOnly,
            Self::RelationsOnly => Self::All,
        }
    }
}

/// Density actually painted after deterministic Auto hysteresis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ResolvedGraphDensity {
    Compact,
    #[default]
    Comfortable,
}

impl ResolvedGraphDensity {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Comfortable => "comfortable",
        }
    }
}

/// Resolve Auto without flapping in the 88-104 column transition band.
pub(crate) const fn resolve_density(
    requested: GraphDensity,
    previous: ResolvedGraphDensity,
    columns: u16,
    rows: u16,
) -> ResolvedGraphDensity {
    match requested {
        GraphDensity::Compact => ResolvedGraphDensity::Compact,
        GraphDensity::Comfortable => ResolvedGraphDensity::Comfortable,
        GraphDensity::Auto if columns <= 88 || rows <= 16 => ResolvedGraphDensity::Compact,
        GraphDensity::Auto if columns >= 104 && rows >= 20 => ResolvedGraphDensity::Comfortable,
        GraphDensity::Auto => previous,
    }
}

/// Measure exactly the semantic rows painted at one density.
pub(crate) fn card_size(node: &GraphExplorerNode, density: ResolvedGraphDensity) -> CellCardSize {
    CellCardSize::for_density(
        node,
        match density {
            ResolvedGraphDensity::Compact => CellSceneDensity::Compact,
            ResolvedGraphDensity::Comfortable => CellSceneDensity::Comfortable,
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GraphStyles {
    pub(crate) normal: Style,
    pub(crate) muted: Style,
    pub(crate) accent: Style,
    pub(crate) selected: Style,
    pub(crate) warning: Style,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GraphHit {
    Node(GraphExplorerIdentity),
    Edge(CellEdgeIdentity),
    Phase(u32),
    VerticalScrollbar,
    HorizontalScrollbar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphHitRegion {
    pub(crate) area: Rect,
    pub(crate) target: GraphHit,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LayeredGraphState {
    pub(crate) hits: Vec<GraphHitRegion>,
    pub(crate) painted_nodes: usize,
    pub(crate) painted_edges: usize,
    pub(crate) visited_cells: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GraphViewport {
    pub(crate) row: usize,
    pub(crate) column: usize,
}

#[derive(Debug, Clone, Copy)]
struct GraphTransform {
    area: Rect,
    scene_x: i32,
    scene_y: i32,
}

#[derive(Debug, Clone, Copy)]
struct GraphStroke {
    style: Style,
    ascii: bool,
}

impl GraphTransform {
    fn new(area: Rect, scene: &CellScene, viewport: GraphViewport) -> Self {
        Self {
            area,
            scene_x: scene
                .bounds
                .x
                .saturating_add(i32::try_from(viewport.column).unwrap_or(i32::MAX)),
            scene_y: scene
                .bounds
                .y
                .saturating_add(i32::try_from(viewport.row).unwrap_or(i32::MAX)),
        }
    }

    fn point(self, point: CellPoint) -> Option<Position> {
        let x = point.x.checked_sub(self.scene_x)?;
        let y = point.y.checked_sub(self.scene_y)?;
        let x = u16::try_from(x).ok()?;
        let y = u16::try_from(y).ok()?;
        let position = Position::new(self.area.x.saturating_add(x), self.area.y.saturating_add(y));
        self.area.contains(position).then_some(position)
    }

    fn scene_point(self, position: Position) -> Option<CellPoint> {
        self.area.contains(position).then(|| CellPoint {
            x: self
                .scene_x
                .saturating_add(i32::from(position.x.saturating_sub(self.area.x))),
            y: self
                .scene_y
                .saturating_add(i32::from(position.y.saturating_sub(self.area.y))),
        })
    }

    fn rect(self, rect: CellRect) -> Option<Rect> {
        let left = rect.x.max(self.scene_x);
        let top = rect.y.max(self.scene_y);
        let right = rect
            .x
            .saturating_add(i32::from(rect.width))
            .min(self.scene_x.saturating_add(i32::from(self.area.width)));
        let bottom = rect
            .y
            .saturating_add(i32::from(rect.height))
            .min(self.scene_y.saturating_add(i32::from(self.area.height)));
        if left >= right || top >= bottom {
            return None;
        }
        Some(Rect::new(
            self.area
                .x
                .saturating_add(u16::try_from(left - self.scene_x).ok()?),
            self.area
                .y
                .saturating_add(u16::try_from(top - self.scene_y).ok()?),
            u16::try_from(right - left).ok()?,
            u16::try_from(bottom - top).ok()?,
        ))
    }
}

pub(crate) struct LayeredGraph<'a> {
    pub(crate) scene: &'a CellScene,
    pub(crate) projection: &'a GraphExplorerProjection,
    pub(crate) viewport: GraphViewport,
    pub(crate) density: ResolvedGraphDensity,
    pub(crate) ascii: bool,
    pub(crate) edge_presentation: GraphEdgePresentation,
    pub(crate) styles: GraphStyles,
}

pub(crate) const FALLBACK_MESSAGE: &str =
    "Spatial layout is unavailable; showing the complete admitted fallback list.";

pub(crate) fn fallback_message(fallback: &aplan_app::CellSceneFallback) -> String {
    let mut reasons = fallback
        .reasons
        .iter()
        .map(fallback_reason)
        .collect::<Vec<_>>();
    reasons.dedup();
    if reasons.is_empty() {
        reasons.push("the layout engine returned no reason".to_string());
    }
    format!(
        "Spatial layout is unavailable: {}; showing the complete admitted fallback list.",
        reasons.join("; ")
    )
}

fn fallback_reason(reason: &aplan_app::CellSceneDegradation) -> String {
    use aplan_app::CellSceneDegradation;
    match reason {
        CellSceneDegradation::SourceState { state } => {
            format!("the semantic graph is {state:?}")
        }
        CellSceneDegradation::EmptyProjection => "the semantic graph is empty".to_string(),
        CellSceneDegradation::LimitExceeded {
            resource,
            got,
            limit,
        } => format!("{resource} need {got}, above the layout limit {limit}"),
        CellSceneDegradation::MissingCardSize => "a node has no measured card".to_string(),
        CellSceneDegradation::InvalidCardSize { columns, rows } => {
            format!("a card has invalid size {columns} by {rows}")
        }
        CellSceneDegradation::InvalidProjection { issue } => issue.clone(),
        CellSceneDegradation::CellExtentExceeded { axis, cells } => {
            format!("the {axis} extent needs {cells} cells")
        }
        CellSceneDegradation::KernelRejected => {
            "the coordinate engine rejected the bounded graph".to_string()
        }
        CellSceneDegradation::MissingNodePlacement => {
            "a semantic node has no coordinate placement".to_string()
        }
        CellSceneDegradation::MissingRoute => "a dependency has no complete route".to_string(),
        CellSceneDegradation::CollapsedRoute => {
            "a dependency route collapsed during cell conversion".to_string()
        }
        CellSceneDegradation::NonOrthogonalSegment => {
            "a dependency route is not orthogonal".to_string()
        }
        CellSceneDegradation::NonMonotoneForwardRoute => {
            "a forward dependency route reverses direction".to_string()
        }
        CellSceneDegradation::InvalidSourcePort => {
            "a dependency has an invalid source port".to_string()
        }
        CellSceneDegradation::InvalidTargetPort => {
            "a dependency has an invalid target port".to_string()
        }
        CellSceneDegradation::OutOfBounds => {
            "layout geometry leaves the declared scene".to_string()
        }
        CellSceneDegradation::RouteThroughNode { node } => {
            format!("a dependency route crosses node `{}`", node.id())
        }
        CellSceneDegradation::NodeOverlap { other } => {
            format!("node `{}` overlaps another card", other.id())
        }
        CellSceneDegradation::EmptyPhaseBand => "a filtered phase has no visible node".to_string(),
        CellSceneDegradation::InvalidPhaseBand => {
            "a phase band does not contain its nodes".to_string()
        }
    }
}

pub(crate) fn fallback_node_text(node: &GraphExplorerNode, selected: bool) -> String {
    let marker = if selected { ">" } else { " " };
    let facts = if node.facts.tasks == 0 {
        String::new()
    } else {
        format!(
            " | {}/{} done, {} ready",
            node.facts.done, node.facts.tasks, node.facts.ready
        )
    };
    format!(
        "{marker} {} [{}] {}{facts}",
        node.identity.id(),
        status_label(node.status),
        node.title
    )
}

pub(crate) fn fallback_node_width(node: &GraphExplorerNode) -> usize {
    text::cells(&text::safe(&fallback_node_text(node, false)))
}

impl StatefulWidget for LayeredGraph<'_> {
    type State = LayeredGraphState;

    fn render(self, area: Rect, buffer: &mut Buffer, state: &mut Self::State) {
        state.hits.clear();
        state.painted_nodes = 0;
        state.painted_edges = 0;
        state.visited_cells = 0;
        if area.is_empty() {
            return;
        }

        let transform = GraphTransform::new(area, self.scene, self.viewport);
        let selected = self.scene.focus.as_ref().map(|focus| &focus.identity);
        let related = related_identities(self.scene, selected);
        self.paint_bands(buffer, state, transform);
        self.paint_edges(buffer, state, transform, selected);
        self.paint_nodes(buffer, state, transform, selected, &related);
        self.paint_ports(buffer, state, transform, selected);
        self.paint_narrow_phase_markers(buffer, state, area);
        self.paint_scrollbars(buffer, state, area);
    }
}

impl LayeredGraph<'_> {
    fn paint_bands(
        &self,
        buffer: &mut Buffer,
        state: &mut LayeredGraphState,
        transform: GraphTransform,
    ) {
        for band in &self.scene.bands {
            let Some(scene_bounds) = band.bounds else {
                continue;
            };
            if transform.rect(scene_bounds).is_none() {
                continue;
            }
            paint_scene_outline(
                buffer,
                scene_bounds,
                transform,
                self.styles.muted,
                self.ascii,
                false,
            );
            if let Some(anchor) = band.label_anchor {
                let label = format!(" phase {} ", band.phase);
                let hit = paint_scene_text(
                    buffer,
                    anchor,
                    &label,
                    text::cells(&label),
                    transform,
                    self.styles.accent,
                    self.ascii,
                );
                if let Some(area) = hit {
                    state.hits.push(GraphHitRegion {
                        area,
                        target: GraphHit::Phase(band.phase),
                    });
                }
            }
        }
    }

    fn paint_edges(
        &self,
        buffer: &mut Buffer,
        state: &mut LayeredGraphState,
        transform: GraphTransform,
        selected: Option<&GraphExplorerIdentity>,
    ) {
        for edge in &self.scene.edges {
            let related = edge_is_related(edge, selected);
            if !edge_is_visible(edge, selected, self.edge_presentation) {
                continue;
            }
            let style = if related {
                self.styles.accent
            } else {
                self.styles.muted
            };
            let before = state.hits.len();
            for pair in edge.points.windows(2) {
                paint_segment(
                    buffer,
                    state,
                    transform,
                    pair[0],
                    pair[1],
                    edge,
                    GraphStroke {
                        style,
                        ascii: self.ascii,
                    },
                );
            }
            if let Some(arrow) = edge.arrowhead.and_then(|point| transform.point(point)) {
                let glyph = arrow_glyph(edge, self.ascii);
                put(buffer, arrow, glyph, style.add_modifier(Modifier::BOLD));
                state.visited_cells = state.visited_cells.saturating_add(1);
                state.hits.push(GraphHitRegion {
                    area: Rect::new(arrow.x, arrow.y, 1, 1),
                    target: GraphHit::Edge(edge.identity.clone()),
                });
            }
            if state.hits.len() > before {
                state.painted_edges = state.painted_edges.saturating_add(1);
            }
        }
        for crossing in &self.scene.crossings {
            let visible = self
                .scene
                .edges
                .iter()
                .filter(|edge| edge.identity == crossing.first || edge.identity == crossing.second)
                .all(|edge| edge_is_visible(edge, selected, self.edge_presentation));
            if visible && let Some(at) = transform.point(crossing.at) {
                put(
                    buffer,
                    at,
                    if self.ascii { "+" } else { "\u{253c}" },
                    self.styles.warning,
                );
            }
        }
        for junction in &self.scene.junctions {
            let visible_edges = self
                .scene
                .edges
                .iter()
                .filter(|edge| junction.edges.contains(&edge.identity))
                .filter(|edge| edge_is_visible(edge, selected, self.edge_presentation))
                .count();
            if visible_edges > 1
                && let Some(at) = transform.point(junction.at)
            {
                put(
                    buffer,
                    at,
                    if self.ascii { "*" } else { "\u{25c6}" },
                    self.styles.accent,
                );
            }
        }
    }

    fn paint_ports(
        &self,
        buffer: &mut Buffer,
        state: &mut LayeredGraphState,
        transform: GraphTransform,
        selected: Option<&GraphExplorerIdentity>,
    ) {
        for edge in &self.scene.edges {
            if !edge_is_visible(edge, selected, self.edge_presentation) {
                continue;
            }
            let style = if edge_is_related(edge, selected) {
                self.styles.accent
            } else {
                self.styles.muted
            }
            .add_modifier(Modifier::BOLD);
            if let Some(port) = edge.source_port
                && let Some(at) = transform.point(port.point)
            {
                put(buffer, at, if self.ascii { "o" } else { "\u{25cb}" }, style);
                state.hits.push(GraphHitRegion {
                    area: Rect::new(at.x, at.y, 1, 1),
                    target: GraphHit::Edge(edge.identity.clone()),
                });
                state.visited_cells = state.visited_cells.saturating_add(1);
            }
            if let Some(port) = edge.target_port
                && let Some(at) = transform.point(port.point)
            {
                put(buffer, at, arrow_glyph(edge, self.ascii), style);
                state.hits.push(GraphHitRegion {
                    area: Rect::new(at.x, at.y, 1, 1),
                    target: GraphHit::Edge(edge.identity.clone()),
                });
                state.visited_cells = state.visited_cells.saturating_add(1);
            }
        }
    }

    fn paint_nodes(
        &self,
        buffer: &mut Buffer,
        state: &mut LayeredGraphState,
        transform: GraphTransform,
        selected: Option<&GraphExplorerIdentity>,
        related: &BTreeSet<GraphExplorerIdentity>,
    ) {
        let semantic: BTreeMap<_, _> = self
            .projection
            .graph
            .nodes
            .iter()
            .map(|node| (&node.identity, node))
            .collect();
        for node in &self.scene.nodes {
            let Some(visible) = transform.rect(node.bounds) else {
                continue;
            };
            let selected_node = selected == Some(&node.identity);
            let related_node = related.contains(&node.identity);
            let style = if selected_node {
                self.styles.selected.add_modifier(Modifier::BOLD)
            } else if related_node {
                self.styles.accent
            } else if self.edge_presentation.is_relations_only() {
                self.styles.muted.add_modifier(Modifier::DIM)
            } else {
                self.styles.normal
            };
            if selected_node {
                paint_scene_interior(buffer, node.bounds, transform, style);
            }
            paint_scene_outline(
                buffer,
                node.bounds,
                transform,
                style,
                self.ascii,
                selected_node,
            );
            if node.bounds.width >= 3 && node.bounds.height >= 2 {
                let marker = if selected_node {
                    if self.ascii { ">" } else { "\u{25b6}" }
                } else if related_node {
                    if self.ascii { "+" } else { "\u{2022}" }
                } else {
                    " "
                };
                if let Some(at) = transform.point(CellPoint {
                    x: node.bounds.x.saturating_add(1),
                    y: node.bounds.y,
                }) {
                    put(buffer, at, marker, style);
                }
                if let Some(source) = semantic.get(&node.identity) {
                    paint_card(
                        buffer,
                        node.bounds,
                        transform,
                        source,
                        self.density,
                        self.ascii,
                        style,
                    );
                }
            }
            state.hits.push(GraphHitRegion {
                area: visible,
                target: GraphHit::Node(node.identity.clone()),
            });
            state.painted_nodes = state.painted_nodes.saturating_add(1);
            state.visited_cells = state
                .visited_cells
                .saturating_add(usize::from(visible.width) * usize::from(visible.height));
        }
    }

    fn paint_scrollbars(&self, buffer: &mut Buffer, state: &mut LayeredGraphState, area: Rect) {
        let scene_rows = usize::from(self.scene.bounds.height);
        let scene_columns = usize::from(self.scene.bounds.width);
        if scene_rows > usize::from(area.height) {
            let mut scroll_state = ScrollbarState::new(scene_rows)
                .position(self.viewport.row)
                .viewport_content_length(usize::from(area.height));
            let mut scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(self.styles.accent)
                .track_style(self.styles.muted);
            if self.ascii {
                scrollbar = scrollbar
                    .thumb_symbol("#")
                    .track_symbol(Some("|"))
                    .begin_symbol(Some("^"))
                    .end_symbol(Some("v"));
            }
            scrollbar.render(area, buffer, &mut scroll_state);
            state.hits.push(GraphHitRegion {
                area: Rect::new(area.right().saturating_sub(1), area.y, 1, area.height),
                target: GraphHit::VerticalScrollbar,
            });
        }
        if scene_columns > usize::from(area.width) {
            let mut scroll_state = ScrollbarState::new(scene_columns)
                .position(self.viewport.column)
                .viewport_content_length(usize::from(area.width));
            let mut scrollbar = Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
                .thumb_style(self.styles.accent)
                .track_style(self.styles.muted);
            if self.ascii {
                scrollbar = scrollbar
                    .thumb_symbol("#")
                    .track_symbol(Some("-"))
                    .begin_symbol(Some("<"))
                    .end_symbol(Some(">"));
            }
            scrollbar.render(area, buffer, &mut scroll_state);
            state.hits.push(GraphHitRegion {
                area: Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
                target: GraphHit::HorizontalScrollbar,
            });
        }
    }

    fn paint_narrow_phase_markers(
        &self,
        buffer: &mut Buffer,
        state: &mut LayeredGraphState,
        area: Rect,
    ) {
        if area.width >= 72 || area.height == 0 {
            return;
        }
        let selected = self.scene.focus.as_ref().map(|focus| &focus.identity);
        let Some(phase) = selected.and_then(|identity| {
            self.projection
                .graph
                .nodes
                .iter()
                .find(|node| node.identity == *identity)
                .map(|node| node.phase)
        }) else {
            return;
        };
        let phases = self
            .projection
            .graph
            .phase_bands
            .iter()
            .map(|band| band.phase)
            .collect::<Vec<_>>();
        let (previous, next) = if let Some(context) = &self.projection.phase_context {
            (context.previous, context.next)
        } else {
            let Some(index) = phases.iter().position(|candidate| *candidate == phase) else {
                return;
            };
            (
                index
                    .checked_sub(1)
                    .and_then(|index| phases.get(index).copied()),
                phases.get(index.saturating_add(1)).copied(),
            )
        };
        if let Some(previous) = previous {
            let label = format!("< P{previous}");
            buffer.set_stringn(
                area.x,
                area.y,
                &label,
                usize::from(area.width),
                self.styles.accent.add_modifier(Modifier::BOLD),
            );
            state.hits.push(GraphHitRegion {
                area: Rect::new(
                    area.x,
                    area.y,
                    u16::try_from(text::cells(&label)).unwrap_or(u16::MAX),
                    1,
                ),
                target: GraphHit::Phase(previous),
            });
        }
        if let Some(next) = next {
            let label = format!("P{next} >");
            let width = u16::try_from(text::cells(&label)).unwrap_or(u16::MAX);
            if width <= area.width {
                let x = area.right().saturating_sub(width);
                buffer.set_stringn(
                    x,
                    area.y,
                    &label,
                    usize::from(width),
                    self.styles.accent.add_modifier(Modifier::BOLD),
                );
                state.hits.push(GraphHitRegion {
                    area: Rect::new(x, area.y, width, 1),
                    target: GraphHit::Phase(next),
                });
            }
        }
    }
}

fn paint_segment(
    buffer: &mut Buffer,
    state: &mut LayeredGraphState,
    transform: GraphTransform,
    from: CellPoint,
    to: CellPoint,
    edge: &CellSceneEdge,
    stroke: GraphStroke,
) {
    let Some((visible_from, visible_to)) = clip_segment(from, to, transform) else {
        paint_segment_continuation(buffer, transform, from, to, stroke.style, stroke.ascii);
        return;
    };
    let dx = (visible_to.x - visible_from.x).signum();
    let dy = (visible_to.y - visible_from.y).signum();
    let length = visible_to
        .x
        .abs_diff(visible_from.x)
        .saturating_add(visible_to.y.abs_diff(visible_from.y));
    let glyph = edge_glyph(edge, dx, dy, stroke.ascii);
    let mut point = visible_from;
    for _ in 0..=length {
        if let Some(screen) = transform.point(point) {
            merge_edge_cell(buffer, screen, glyph, stroke.style, stroke.ascii);
            state.hits.push(GraphHitRegion {
                area: Rect::new(screen.x, screen.y, 1, 1),
                target: GraphHit::Edge(edge.identity.clone()),
            });
            state.visited_cells = state.visited_cells.saturating_add(1);
        }
        point.x = point.x.saturating_add(dx);
        point.y = point.y.saturating_add(dy);
    }
    paint_segment_continuation(buffer, transform, from, to, stroke.style, stroke.ascii);
}

fn clip_segment(
    from: CellPoint,
    to: CellPoint,
    transform: GraphTransform,
) -> Option<(CellPoint, CellPoint)> {
    let min_x = transform.scene_x;
    let min_y = transform.scene_y;
    let max_x = min_x
        .saturating_add(i32::from(transform.area.width))
        .saturating_sub(1);
    let max_y = min_y
        .saturating_add(i32::from(transform.area.height))
        .saturating_sub(1);
    if from.y == to.y {
        if from.y < min_y || from.y > max_y {
            return None;
        }
        let low = from.x.min(to.x).max(min_x);
        let high = from.x.max(to.x).min(max_x);
        if low > high {
            return None;
        }
        return if from.x <= to.x {
            Some((
                CellPoint { x: low, y: from.y },
                CellPoint { x: high, y: from.y },
            ))
        } else {
            Some((
                CellPoint { x: high, y: from.y },
                CellPoint { x: low, y: from.y },
            ))
        };
    }
    if from.x != to.x || from.x < min_x || from.x > max_x {
        return None;
    }
    let low = from.y.min(to.y).max(min_y);
    let high = from.y.max(to.y).min(max_y);
    if low > high {
        return None;
    }
    if from.y <= to.y {
        Some((
            CellPoint { x: from.x, y: low },
            CellPoint { x: from.x, y: high },
        ))
    } else {
        Some((
            CellPoint { x: from.x, y: high },
            CellPoint { x: from.x, y: low },
        ))
    }
}

fn paint_segment_continuation(
    buffer: &mut Buffer,
    transform: GraphTransform,
    from: CellPoint,
    to: CellPoint,
    style: Style,
    ascii: bool,
) {
    let min_x = transform.scene_x;
    let min_y = transform.scene_y;
    let max_x = min_x
        .saturating_add(i32::from(transform.area.width))
        .saturating_sub(1);
    let max_y = min_y
        .saturating_add(i32::from(transform.area.height))
        .saturating_sub(1);
    let candidates = [
        (
            CellPoint {
                x: min_x,
                y: from.y,
            },
            from.x < min_x || to.x < min_x,
            "<",
            "\u{25c0}",
        ),
        (
            CellPoint {
                x: max_x,
                y: from.y,
            },
            from.x > max_x || to.x > max_x,
            ">",
            "\u{25b6}",
        ),
        (
            CellPoint {
                x: from.x,
                y: min_y,
            },
            from.y < min_y || to.y < min_y,
            "^",
            "\u{25b2}",
        ),
        (
            CellPoint {
                x: from.x,
                y: max_y,
            },
            from.y > max_y || to.y > max_y,
            "v",
            "\u{25bc}",
        ),
    ];
    for (point, continues, ascii_glyph, unicode_glyph) in candidates {
        if continues
            && point_on_segment(point, from, to)
            && let Some(at) = transform.point(point)
        {
            put(
                buffer,
                at,
                if ascii { ascii_glyph } else { unicode_glyph },
                style.add_modifier(Modifier::BOLD),
            );
        }
    }
}

fn point_on_segment(point: CellPoint, from: CellPoint, to: CellPoint) -> bool {
    if from.x == to.x {
        point.x == from.x && point.y >= from.y.min(to.y) && point.y <= from.y.max(to.y)
    } else {
        point.y == from.y && point.x >= from.x.min(to.x) && point.x <= from.x.max(to.x)
    }
}

fn edge_glyph(edge: &CellSceneEdge, dx: i32, _dy: i32, ascii: bool) -> &'static str {
    if edge.feedback != CellFeedbackKind::None {
        return if ascii { ":" } else { "\u{2504}" };
    }
    if matches!(edge.kind, Some(CellRouteKind::Bundled { .. })) {
        return if ascii { "=" } else { "\u{2550}" };
    }
    if dx == 0 {
        if ascii { "|" } else { "\u{2502}" }
    } else if ascii {
        "-"
    } else {
        "\u{2500}"
    }
}

fn arrow_glyph(edge: &CellSceneEdge, ascii: bool) -> &'static str {
    let Some(arrow) = edge.arrowhead else {
        return if ascii { ">" } else { "\u{25b6}" };
    };
    let Some(previous) = edge.points.iter().rev().find(|point| **point != arrow) else {
        return if ascii { ">" } else { "\u{25b6}" };
    };
    if arrow.x > previous.x {
        if ascii { ">" } else { "\u{25b6}" }
    } else if arrow.x < previous.x {
        if ascii { "<" } else { "\u{25c0}" }
    } else if arrow.y > previous.y {
        if ascii { "v" } else { "\u{25bc}" }
    } else if ascii {
        "^"
    } else {
        "\u{25b2}"
    }
}

fn merge_edge_cell(buffer: &mut Buffer, at: Position, glyph: &str, style: Style, ascii: bool) {
    let occupied = buffer.cell(at).is_some_and(|cell| cell.symbol() != " ");
    put(
        buffer,
        at,
        if occupied {
            if ascii { "+" } else { "\u{253c}" }
        } else {
            glyph
        },
        style,
    );
}

fn put(buffer: &mut Buffer, at: Position, glyph: &str, style: Style) {
    if let Some(cell) = buffer.cell_mut(at) {
        cell.set_symbol(glyph).set_style(style);
    }
}

fn paint_scene_outline(
    buffer: &mut Buffer,
    scene: CellRect,
    transform: GraphTransform,
    style: Style,
    ascii: bool,
    selected: bool,
) {
    if scene.width == 0 || scene.height == 0 {
        return;
    }
    let (horizontal, vertical, top_left, top_right, bottom_left, bottom_right) =
        if ascii && selected {
            ("=", "!", "#", "#", "#", "#")
        } else if ascii {
            ("-", "|", "+", "+", "+", "+")
        } else if selected {
            (
                "\u{2550}", "\u{2551}", "\u{2554}", "\u{2557}", "\u{255a}", "\u{255d}",
            )
        } else {
            (
                "\u{2500}", "\u{2502}", "\u{250c}", "\u{2510}", "\u{2514}", "\u{2518}",
            )
        };
    let right = scene
        .x
        .saturating_add(i32::from(scene.width))
        .saturating_sub(1);
    let bottom = scene
        .y
        .saturating_add(i32::from(scene.height))
        .saturating_sub(1);
    let visible_left = scene.x.max(transform.scene_x);
    let visible_right = right.min(
        transform
            .scene_x
            .saturating_add(i32::from(transform.area.width))
            .saturating_sub(1),
    );
    let visible_top = scene.y.max(transform.scene_y);
    let visible_bottom = bottom.min(
        transform
            .scene_y
            .saturating_add(i32::from(transform.area.height))
            .saturating_sub(1),
    );
    if visible_left > visible_right || visible_top > visible_bottom {
        return;
    }
    if scene.y >= visible_top && scene.y <= visible_bottom {
        for x in visible_left..=visible_right {
            if let Some(at) = transform.point(CellPoint { x, y: scene.y }) {
                put(buffer, at, horizontal, style);
            }
        }
    }
    if scene.height > 1 && bottom >= visible_top && bottom <= visible_bottom {
        for x in visible_left..=visible_right {
            if let Some(at) = transform.point(CellPoint { x, y: bottom }) {
                put(buffer, at, horizontal, style);
            }
        }
    }
    if scene.x >= visible_left && scene.x <= visible_right {
        for y in visible_top..=visible_bottom {
            if let Some(at) = transform.point(CellPoint { x: scene.x, y }) {
                put(buffer, at, vertical, style);
            }
        }
    }
    if scene.width > 1 && right >= visible_left && right <= visible_right {
        for y in visible_top..=visible_bottom {
            if let Some(at) = transform.point(CellPoint { x: right, y }) {
                put(buffer, at, vertical, style);
            }
        }
    }
    if let Some(at) = transform.point(CellPoint {
        x: scene.x,
        y: scene.y,
    }) {
        put(buffer, at, top_left, style);
    }
    if scene.width > 1
        && let Some(at) = transform.point(CellPoint {
            x: right,
            y: scene.y,
        })
    {
        put(buffer, at, top_right, style);
    }
    if scene.height > 1 {
        if let Some(at) = transform.point(CellPoint {
            x: scene.x,
            y: bottom,
        }) {
            put(buffer, at, bottom_left, style);
        }
        if scene.width > 1
            && let Some(at) = transform.point(CellPoint {
                x: right,
                y: bottom,
            })
        {
            put(buffer, at, bottom_right, style);
        }
    }
}

fn paint_scene_interior(
    buffer: &mut Buffer,
    scene: CellRect,
    transform: GraphTransform,
    style: Style,
) {
    if scene.width <= 2 || scene.height <= 2 {
        return;
    }
    let interior = CellRect {
        x: scene.x.saturating_add(1),
        y: scene.y.saturating_add(1),
        width: scene.width.saturating_sub(2),
        height: scene.height.saturating_sub(2),
    };
    let Some(visible) = transform.rect(interior) else {
        return;
    };
    for y in visible.y..visible.bottom() {
        for x in visible.x..visible.right() {
            put(buffer, Position { x, y }, " ", style);
        }
    }
}

fn paint_card(
    buffer: &mut Buffer,
    area: CellRect,
    transform: GraphTransform,
    node: &GraphExplorerNode,
    density: ResolvedGraphDensity,
    ascii: bool,
    style: Style,
) {
    let width = usize::from(area.width.saturating_sub(2));
    if width == 0 || area.height < 2 {
        return;
    }
    let id = text::fit(node.identity.id(), width, ascii);
    paint_scene_text(
        buffer,
        CellPoint {
            x: area.x.saturating_add(1),
            y: area.y.saturating_add(1),
        },
        &id,
        width,
        transform,
        style.add_modifier(Modifier::BOLD),
        ascii,
    );
    let status = status_label(node.status);
    if area.height >= 3 {
        let title_width = width.saturating_sub(status.len().saturating_add(1));
        let title = if density == ResolvedGraphDensity::Comfortable {
            text::fit(&node.title, title_width, ascii)
        } else {
            String::new()
        };
        let line = if title.is_empty() {
            status.to_string()
        } else {
            format!("{status} {title}")
        };
        paint_scene_text(
            buffer,
            CellPoint {
                x: area.x.saturating_add(1),
                y: area.y.saturating_add(2),
            },
            &line,
            width,
            transform,
            style,
            ascii,
        );
    }
    if density == ResolvedGraphDensity::Comfortable && area.height >= 5 && node.facts.tasks > 0 {
        let facts = format!(
            "{}/{} done | {} ready",
            node.facts.done, node.facts.tasks, node.facts.ready
        );
        paint_scene_text(
            buffer,
            CellPoint {
                x: area.x.saturating_add(1),
                y: area.y.saturating_add(3),
            },
            &text::fit(&facts, width, ascii),
            width,
            transform,
            style,
            ascii,
        );
    }
}

fn paint_scene_text(
    buffer: &mut Buffer,
    start: CellPoint,
    value: &str,
    max_width: usize,
    transform: GraphTransform,
    style: Style,
    ascii: bool,
) -> Option<Rect> {
    let screen_y = u16::try_from(start.y.checked_sub(transform.scene_y)?).ok()?;
    if screen_y >= transform.area.height {
        return None;
    }
    let clipped_left = usize::try_from(transform.scene_x.saturating_sub(start.x).max(0)).ok()?;
    let screen_x = u16::try_from(start.x.saturating_sub(transform.scene_x).max(0)).ok()?;
    if screen_x >= transform.area.width || clipped_left >= max_width {
        return None;
    }
    let available = max_width
        .saturating_sub(clipped_left)
        .min(usize::from(transform.area.width.saturating_sub(screen_x)));
    let fitted = text::fit(value, max_width, ascii);
    let visible = text::slice_cells(&fitted, clipped_left, available);
    let width = text::cells(&visible).min(available);
    if width == 0 {
        return None;
    }
    let x = transform.area.x.saturating_add(screen_x);
    let y = transform.area.y.saturating_add(screen_y);
    buffer.set_stringn(x, y, visible, available, style);
    Some(Rect::new(x, y, u16::try_from(width).unwrap_or(u16::MAX), 1))
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

fn related_identities(
    scene: &CellScene,
    selected: Option<&GraphExplorerIdentity>,
) -> BTreeSet<GraphExplorerIdentity> {
    let mut answer = BTreeSet::new();
    let Some(selected) = selected else {
        return answer;
    };
    answer.insert(selected.clone());
    for edge in &scene.edges {
        if edge.identity.from == *selected {
            answer.insert(edge.identity.to.clone());
        }
        if edge.identity.to == *selected {
            answer.insert(edge.identity.from.clone());
        }
    }
    answer
}

fn edge_is_related(edge: &CellSceneEdge, selected: Option<&GraphExplorerIdentity>) -> bool {
    selected.is_none_or(|identity| edge.identity.from == *identity || edge.identity.to == *identity)
}

fn edge_is_visible(
    edge: &CellSceneEdge,
    selected: Option<&GraphExplorerIdentity>,
    presentation: GraphEdgePresentation,
) -> bool {
    !presentation.is_relations_only() || edge_is_related(edge, selected)
}

pub(crate) fn hit_test(
    scene: &CellScene,
    area: Rect,
    viewport: GraphViewport,
    position: Position,
    edge_presentation: GraphEdgePresentation,
) -> Option<GraphHit> {
    if !area.contains(position) {
        return None;
    }
    let transform = GraphTransform::new(area, scene, viewport);
    if usize::from(scene.bounds.width) > usize::from(area.width)
        && position.y == area.bottom().saturating_sub(1)
    {
        return Some(GraphHit::HorizontalScrollbar);
    }
    if usize::from(scene.bounds.height) > usize::from(area.height)
        && position.x == area.right().saturating_sub(1)
    {
        return Some(GraphHit::VerticalScrollbar);
    }
    let scene_point = transform.scene_point(position)?;
    let selected = scene.focus.as_ref().map(|focus| &focus.identity);
    for edge in scene.edges.iter().rev() {
        if !edge_is_visible(edge, selected, edge_presentation) {
            continue;
        }
        if edge
            .source_port
            .is_some_and(|port| port.point == scene_point)
            || edge
                .target_port
                .is_some_and(|port| port.point == scene_point)
        {
            return Some(GraphHit::Edge(edge.identity.clone()));
        }
    }
    for node in scene.nodes.iter().rev() {
        if transform
            .rect(node.bounds)
            .is_some_and(|rect| rect.contains(position))
        {
            return Some(GraphHit::Node(node.identity.clone()));
        }
    }
    for edge in scene.edges.iter().rev() {
        if !edge_is_visible(edge, selected, edge_presentation) {
            continue;
        }
        if edge
            .points
            .windows(2)
            .any(|pair| point_on_segment(scene_point, pair[0], pair[1]))
        {
            return Some(GraphHit::Edge(edge.identity.clone()));
        }
    }
    for band in scene.bands.iter().rev() {
        if phase_label_contains(band, transform, position) {
            return Some(GraphHit::Phase(band.phase));
        }
    }
    None
}

fn phase_label_contains(
    band: &CellSceneBand,
    transform: GraphTransform,
    position: Position,
) -> bool {
    let Some(anchor) = band.label_anchor.and_then(|anchor| transform.point(anchor)) else {
        return false;
    };
    Rect::new(
        anchor.x,
        anchor.y,
        u16::try_from(format!(" phase {} ", band.phase).len()).unwrap_or(u16::MAX),
        1,
    )
    .contains(position)
}

pub(crate) fn anchor_reason(reason: &GraphExplorerAnchorReason) -> &'static str {
    match reason {
        GraphExplorerAnchorReason::Contextual => "context",
        GraphExplorerAnchorReason::Remembered => "remembered",
        GraphExplorerAnchorReason::InProgress => "in-progress work",
        GraphExplorerAnchorReason::Ready => "ready work",
        GraphExplorerAnchorReason::FirstUnfinishedMilestone => "first unfinished milestone",
        GraphExplorerAnchorReason::FirstMilestone => "first milestone",
        GraphExplorerAnchorReason::FirstTask => "first task",
        GraphExplorerAnchorReason::DetailAnchor => "detail context",
        GraphExplorerAnchorReason::NoMilestone => "ungrouped work",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_interior_fill_is_clipped_and_cannot_escape_its_card() {
        use ratatui::style::Color;

        let area = Rect::new(5, 7, 6, 5);
        let prior = Style::new().fg(Color::Yellow).bg(Color::Blue);
        let selected = Style::new()
            .fg(Color::Black)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD);
        let mut buffer = Buffer::empty(area);
        buffer.set_style(area, prior);

        // The card interior is x=11..14 and y=21..23 in scene cells. The
        // viewport starts at x=12/y=22, so only x=12..14 and y=22..23 may be
        // touched at buffer x=5..7 and y=7..8. Non-zero buffer coordinates
        // make an accidental scene/screen-coordinate mix-up observable.
        paint_scene_interior(
            &mut buffer,
            CellRect {
                x: 10,
                y: 20,
                width: 6,
                height: 5,
            },
            GraphTransform {
                area,
                scene_x: 12,
                scene_y: 22,
            },
            selected,
        );

        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let cell = &buffer[(x, y)];
                let expected = if x < 8 && y < 9 { selected } else { prior };
                assert_eq!(
                    (cell.fg, cell.bg, cell.modifier),
                    (
                        expected.fg.unwrap_or_default(),
                        expected.bg.unwrap_or_default(),
                        expected.add_modifier,
                    ),
                    "unexpected style at buffer cell {x},{y}",
                );
            }
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn every_fallback_reason_has_specific_user_facing_copy() {
        use aplan_app::CellSceneDegradation;

        let task = GraphExplorerIdentity::Task("A1-682".to_string());
        let cases = vec![
            (
                CellSceneDegradation::SourceState {
                    state: aplan_app::GraphExplorerState::InvalidTaskCycle,
                },
                "the semantic graph is InvalidTaskCycle",
            ),
            (
                CellSceneDegradation::EmptyProjection,
                "the semantic graph is empty",
            ),
            (
                CellSceneDegradation::LimitExceeded {
                    resource: "edges".to_string(),
                    got: 97,
                    limit: 96,
                },
                "edges need 97, above the layout limit 96",
            ),
            (
                CellSceneDegradation::MissingCardSize,
                "a node has no measured card",
            ),
            (
                CellSceneDegradation::InvalidCardSize {
                    columns: 0,
                    rows: 4,
                },
                "a card has invalid size 0 by 4",
            ),
            (
                CellSceneDegradation::InvalidProjection {
                    issue: "duplicate identity".to_string(),
                },
                "duplicate identity",
            ),
            (
                CellSceneDegradation::CellExtentExceeded {
                    axis: "x".to_string(),
                    cells: 70_000,
                },
                "the x extent needs 70000 cells",
            ),
            (
                CellSceneDegradation::KernelRejected,
                "the coordinate engine rejected the bounded graph",
            ),
            (
                CellSceneDegradation::MissingNodePlacement,
                "a semantic node has no coordinate placement",
            ),
            (
                CellSceneDegradation::MissingRoute,
                "a dependency has no complete route",
            ),
            (
                CellSceneDegradation::CollapsedRoute,
                "a dependency route collapsed during cell conversion",
            ),
            (
                CellSceneDegradation::NonOrthogonalSegment,
                "a dependency route is not orthogonal",
            ),
            (
                CellSceneDegradation::NonMonotoneForwardRoute,
                "a forward dependency route reverses direction",
            ),
            (
                CellSceneDegradation::InvalidSourcePort,
                "a dependency has an invalid source port",
            ),
            (
                CellSceneDegradation::InvalidTargetPort,
                "a dependency has an invalid target port",
            ),
            (
                CellSceneDegradation::OutOfBounds,
                "layout geometry leaves the declared scene",
            ),
            (
                CellSceneDegradation::RouteThroughNode { node: task.clone() },
                "a dependency route crosses node `A1-682`",
            ),
            (
                CellSceneDegradation::NodeOverlap { other: task },
                "node `A1-682` overlaps another card",
            ),
            (
                CellSceneDegradation::EmptyPhaseBand,
                "a filtered phase has no visible node",
            ),
            (
                CellSceneDegradation::InvalidPhaseBand,
                "a phase band does not contain its nodes",
            ),
        ];

        for (reason, expected) in cases {
            assert_eq!(fallback_reason(&reason), expected);
        }
    }

    #[allow(clippy::too_many_lines)]
    fn interactive_scene() -> CellScene {
        let a = GraphExplorerIdentity::Task("A".to_string());
        let b = GraphExplorerIdentity::Task("B".to_string());
        let edge = CellEdgeIdentity {
            from: a.clone(),
            to: b.clone(),
            ordinal: 0,
            provenance: vec![aplan_app::GraphExplorerDependency {
                from: "A".to_string(),
                to: "B".to_string(),
            }],
        };
        CellScene {
            generation: 1,
            input: aplan_app::InputIdentity {
                source: "test",
                plan_root: "/repo/aplan".into(),
                plan_sha256: "plan".to_string(),
                repository_root: "/repo".into(),
                repository: aplan_app::RepositoryIdentity::NotObserved {
                    reason: "test".to_string(),
                },
                configuration: aplan_app::ConfigurationIdentity {
                    path: None,
                    sha256: "configuration".to_string(),
                    content: aplan_app::ConfigurationContentIdentity::Absent,
                },
                trust: aplan_app::Trust::Hermetic,
                sha256: "input".to_string(),
            },
            location: aplan_app::GraphExplorerLocation::Phase(1),
            geometry_key: "geometry".to_string(),
            direction: aplan_app::CellSceneDirection::LeftToRight,
            ordering_policy: aplan_app::CellPeerOrder::Readability,
            bounds: CellRect {
                x: 0,
                y: 0,
                width: 30,
                height: 8,
            },
            nodes: vec![
                aplan_app::CellSceneNode {
                    identity: a.clone(),
                    bounds: CellRect {
                        x: 1,
                        y: 2,
                        width: 8,
                        height: 4,
                    },
                    rank: Some(0),
                    order: 0,
                },
                aplan_app::CellSceneNode {
                    identity: b.clone(),
                    bounds: CellRect {
                        x: 20,
                        y: 2,
                        width: 8,
                        height: 4,
                    },
                    rank: Some(1),
                    order: 0,
                },
            ],
            edges: vec![CellSceneEdge {
                identity: edge,
                disposition: aplan_app::GraphExplorerEdgeDisposition::Forward,
                feedback: CellFeedbackKind::None,
                kind: Some(CellRouteKind::Direct),
                points: vec![CellPoint { x: 9, y: 4 }, CellPoint { x: 19, y: 4 }],
                source_port: Some(aplan_app::CellPort {
                    point: CellPoint { x: 9, y: 4 },
                    side: aplan_app::CellPortSide::Right,
                }),
                target_port: Some(aplan_app::CellPort {
                    point: CellPoint { x: 19, y: 4 },
                    side: aplan_app::CellPortSide::Left,
                }),
                arrowhead: Some(CellPoint { x: 19, y: 4 }),
                degradations: Vec::new(),
            }],
            bands: vec![CellSceneBand {
                phase: 1,
                label: "Phase 1".to_string(),
                label_anchor: Some(CellPoint { x: 0, y: 0 }),
                nodes: vec![a.clone(), b],
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
            focus: Some(aplan_app::CellSceneFocus {
                identity: a,
                bounds: CellRect {
                    x: 1,
                    y: 2,
                    width: 8,
                    height: 4,
                },
                recenter: false,
            }),
            movement: aplan_app::CellSceneMovement::Fresh,
            metrics: aplan_app::CellSceneMetrics::default(),
        }
    }

    #[test]
    fn static_and_interactive_consumers_share_exact_card_measurement() {
        let node = GraphExplorerNode {
            identity: GraphExplorerIdentity::Milestone("M-wide".to_string()),
            title: "wide \u{754c} e\u{301} title".to_string(),
            phase: 2,
            milestone: None,
            status: Status::InProgress,
            checkpoint_in_scope: true,
            represented_ids: vec!["M-wide".to_string(), "T-1".to_string()],
            member_ids: vec!["T-1".to_string()],
            member_phases: vec![2],
            facts: aplan_app::GraphExplorerFacts {
                tasks: 2,
                done: 1,
                ready: 1,
                ..aplan_app::GraphExplorerFacts::default()
            },
            internal_dependencies: Vec::new(),
        };
        assert_eq!(
            card_size(&node, ResolvedGraphDensity::Compact),
            CellCardSize::for_density(&node, CellSceneDensity::Compact)
        );
        assert_eq!(
            card_size(&node, ResolvedGraphDensity::Comfortable),
            CellCardSize::for_density(&node, CellSceneDensity::Comfortable)
        );
    }

    #[test]
    fn auto_density_has_a_stable_transition_band() {
        assert_eq!(
            resolve_density(
                GraphDensity::Auto,
                ResolvedGraphDensity::Comfortable,
                88,
                24,
            ),
            ResolvedGraphDensity::Compact
        );
        assert_eq!(
            resolve_density(GraphDensity::Auto, ResolvedGraphDensity::Compact, 96, 24,),
            ResolvedGraphDensity::Compact
        );
        assert_eq!(
            resolve_density(
                GraphDensity::Auto,
                ResolvedGraphDensity::Comfortable,
                96,
                24,
            ),
            ResolvedGraphDensity::Comfortable
        );
        assert_eq!(
            resolve_density(GraphDensity::Auto, ResolvedGraphDensity::Compact, 104, 20,),
            ResolvedGraphDensity::Comfortable
        );
    }

    #[test]
    fn hit_testing_uses_typed_scene_identity_at_every_visual_layer() {
        let scene = interactive_scene();
        let full = Rect::new(0, 0, 30, 8);
        let viewport = GraphViewport { row: 0, column: 0 };

        assert_eq!(
            hit_test(
                &scene,
                full,
                viewport,
                Position::new(9, 4),
                GraphEdgePresentation::All,
            ),
            Some(GraphHit::Edge(scene.edges[0].identity.clone()))
        );
        assert_eq!(
            hit_test(
                &scene,
                full,
                viewport,
                Position::new(2, 3),
                GraphEdgePresentation::All,
            ),
            Some(GraphHit::Node(scene.nodes[0].identity.clone()))
        );
        assert_eq!(
            hit_test(
                &scene,
                full,
                viewport,
                Position::new(15, 4),
                GraphEdgePresentation::All,
            ),
            Some(GraphHit::Edge(scene.edges[0].identity.clone()))
        );
        assert_eq!(
            hit_test(
                &scene,
                full,
                viewport,
                Position::new(1, 0),
                GraphEdgePresentation::All,
            ),
            Some(GraphHit::Phase(1))
        );
        assert_eq!(
            hit_test(
                &scene,
                full,
                viewport,
                Position::new(29, 7),
                GraphEdgePresentation::All,
            ),
            None
        );
        assert_eq!(
            hit_test(
                &scene,
                full,
                viewport,
                Position::new(31, 7),
                GraphEdgePresentation::All,
            ),
            None
        );

        let narrow = Rect::new(0, 0, 10, 5);
        assert_eq!(
            hit_test(
                &scene,
                narrow,
                viewport,
                Position::new(3, 4),
                GraphEdgePresentation::All,
            ),
            Some(GraphHit::HorizontalScrollbar)
        );
        assert_eq!(
            hit_test(
                &scene,
                narrow,
                viewport,
                Position::new(9, 2),
                GraphEdgePresentation::All,
            ),
            Some(GraphHit::VerticalScrollbar)
        );

        let related = related_identities(&scene, scene.focus.as_ref().map(|focus| &focus.identity));
        assert_eq!(related.len(), 2);
        assert!(related.contains(&scene.nodes[0].identity));
        assert!(related.contains(&scene.nodes[1].identity));
        assert!(related_identities(&scene, None).is_empty());
        assert!(edge_is_visible(
            &scene.edges[0],
            Some(&scene.nodes[0].identity),
            GraphEdgePresentation::RelationsOnly,
        ));
        let unrelated = GraphExplorerIdentity::Task("elsewhere".to_string());
        assert!(!edge_is_visible(
            &scene.edges[0],
            Some(&unrelated),
            GraphEdgePresentation::RelationsOnly,
        ));
    }

    #[test]
    fn every_anchor_reason_has_concise_user_facing_copy() {
        let reasons = [
            GraphExplorerAnchorReason::Contextual,
            GraphExplorerAnchorReason::Remembered,
            GraphExplorerAnchorReason::InProgress,
            GraphExplorerAnchorReason::Ready,
            GraphExplorerAnchorReason::FirstUnfinishedMilestone,
            GraphExplorerAnchorReason::FirstMilestone,
            GraphExplorerAnchorReason::FirstTask,
            GraphExplorerAnchorReason::DetailAnchor,
            GraphExplorerAnchorReason::NoMilestone,
        ];
        let labels: BTreeSet<_> = reasons.iter().map(anchor_reason).collect();
        assert_eq!(labels.len(), reasons.len());
        assert!(labels.iter().all(|label| !label.is_empty()));
    }
}
