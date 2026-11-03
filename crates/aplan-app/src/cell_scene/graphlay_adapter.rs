//! Private adapter around the replaceable floating-point coordinate kernel.
//!
//! Nothing in this module is re-exported. In particular, graphlay identities,
//! geometry, errors and route enums cannot cross the application boundary.

use graphlay::{
    Direction, Edge, Effort, EffortBudget, GraphBuilder, GroupId, GroupLanes, Layout, LayoutError,
    LayoutOptions, MergeEdges, Node, Point, PortSides, RouteKind, SeedMode, Side, Size, Spacing,
    layout as graphlay_layout, metrics, reroute as graphlay_reroute,
};

use super::{CellCardSize, CellPoint, CellRect};

const SCALE: f64 = 8.0;
const PADDING: f64 = SCALE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdapterOrientation {
    LeftToRight,
    TopToBottom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdapterNode {
    pub key: String,
    pub size: CellCardSize,
    pub group: Option<u32>,
    pub lane_key: Option<i64>,
    pub seed: Option<CellPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AdapterEdge {
    pub source: usize,
    pub target: usize,
    pub reverse_for_ranking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdapterInput {
    pub nodes: Vec<AdapterNode>,
    pub edges: Vec<AdapterEdge>,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_routing_waypoints: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdapterSide {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AdapterPort {
    pub point: CellPoint,
    pub side: AdapterSide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdapterRouteKind {
    Direct,
    Chained { spans: u32 },
    SelfLoop,
    Bundled { lane: u32 },
    Unrouted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdapterPlacement {
    pub bounds: CellRect,
    pub rank: Option<i32>,
    pub order: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdapterRoute {
    pub points: Vec<CellPoint>,
    pub source_port: Option<AdapterPort>,
    pub target_port: Option<AdapterPort>,
    pub kind: AdapterRouteKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct AdapterMetrics {
    pub crossings: usize,
    pub bends: usize,
    pub node_edge_hits: usize,
    pub node_overlaps: usize,
    pub endpoint_errors: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AdapterLane {
    pub group: Option<u32>,
    pub bounds: CellRect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdapterOutput {
    pub orientation: AdapterOrientation,
    pub nodes: Vec<AdapterPlacement>,
    pub edges: Vec<AdapterRoute>,
    pub reversed: Vec<usize>,
    pub routing_waypoints: u32,
    pub junctions: Vec<CellPoint>,
    pub lanes: Vec<AdapterLane>,
    pub metrics: AdapterMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AdapterFailure {
    TooLarge {
        resource: String,
        got: usize,
        limit: usize,
    },
    Rejected,
}

pub(super) fn layout(
    input: &AdapterInput,
    orientation: AdapterOrientation,
) -> Result<AdapterOutput, AdapterFailure> {
    let all_seeded = input.nodes.iter().all(|node| node.seed.is_some());
    let mut graph = build_graph(input)?;
    let options = options(input, orientation, all_seeded);
    let output = graphlay_layout(&graph, &options).map_err(map_error)?;
    let measured = metrics::measure(&graph, &options, &output);
    let converted = convert_output(
        input,
        orientation,
        &output,
        AdapterMetrics {
            crossings: measured.crossings,
            bends: measured.bends,
            node_edge_hits: measured.node_edge_hits,
            node_overlaps: measured.node_overlaps,
            endpoint_errors: measured.endpoint_off_outline,
        },
    )?;
    if !adapter_routes_hit_nodes(input, &converted) {
        return Ok(converted);
    }
    for (node, placed) in graph.nodes.iter_mut().zip(&output.nodes) {
        node.position = Some(placed.center);
    }
    let repaired = graphlay_reroute(&graph, &options).map_err(map_error)?;
    let measured = metrics::measure(&graph, &options, &repaired);
    convert_output(
        input,
        orientation,
        &repaired,
        AdapterMetrics {
            crossings: measured.crossings,
            bends: measured.bends,
            node_edge_hits: measured.node_edge_hits,
            node_overlaps: measured.node_overlaps,
            endpoint_errors: measured.endpoint_off_outline,
        },
    )
}

fn adapter_routes_hit_nodes(input: &AdapterInput, output: &AdapterOutput) -> bool {
    output.edges.iter().enumerate().any(|(edge_index, route)| {
        let Some(edge) = input.edges.get(edge_index) else {
            return true;
        };
        output.nodes.iter().enumerate().any(|(node_index, node)| {
            node_index != edge.source
                && node_index != edge.target
                && adapter_route_hits_rect(&route.points, node.bounds)
        })
    })
}

fn adapter_route_hits_rect(points: &[CellPoint], rect: CellRect) -> bool {
    points.windows(2).any(|segment| {
        let [a, b] = [segment[0], segment[1]];
        if a.x == b.x {
            let (start, end) = if a.y <= b.y { (a.y, b.y) } else { (b.y, a.y) };
            a.x >= rect.x && a.x <= rect.right() && start <= rect.bottom() && end >= rect.y
        } else if a.y == b.y {
            let (start, end) = if a.x <= b.x { (a.x, b.x) } else { (b.x, a.x) };
            a.y >= rect.y && a.y <= rect.bottom() && start <= rect.right() && end >= rect.x
        } else {
            false
        }
    })
}

fn build_graph(input: &AdapterInput) -> Result<graphlay::Graph, AdapterFailure> {
    let mut builder = GraphBuilder::new();
    let node_ids: Vec<_> = input
        .nodes
        .iter()
        .map(|node| {
            builder.add_node(Node {
                size: Size::new(
                    f64::from(node.size.columns) * SCALE,
                    f64::from(node.size.rows) * SCALE,
                ),
                key: Some(node.key.clone()),
                group: node.group.map(GroupId),
                lane_key: node.lane_key,
                position: node.seed.map(|point| {
                    Point::new(f64::from(point.x) * SCALE, f64::from(point.y) * SCALE)
                }),
                ..Node::default()
            })
        })
        .collect();
    for edge in &input.edges {
        let Some(&source) = node_ids.get(edge.source) else {
            return Err(AdapterFailure::Rejected);
        };
        let Some(&target) = node_ids.get(edge.target) else {
            return Err(AdapterFailure::Rejected);
        };
        builder.add_edge(Edge {
            thickness: 1.0,
            reverse_for_ranking: edge.reverse_for_ranking,
            ..Edge::new(source, target)
        });
    }
    Ok(builder.build())
}

fn options(
    input: &AdapterInput,
    orientation: AdapterOrientation,
    all_seeded: bool,
) -> LayoutOptions {
    let mut options = LayoutOptions::layered_default();
    options.direction = match orientation {
        AdapterOrientation::LeftToRight => Direction::LeftRight,
        AdapterOrientation::TopToBottom => Direction::TopBottom,
    };
    options.spacing = Spacing {
        layer_gap: 6.0 * SCALE,
        node_gap: 3.0 * SCALE,
        dummy_gap: 2.0 * SCALE,
        port_gap: SCALE,
        port_stroke_clear: 0.5 * SCALE,
        stub: SCALE,
        bundle_lane_gap: 0.75 * SCALE,
        self_loop_gap: 1.5 * SCALE,
        edge_label_margin: 2.0 * SCALE,
        edge_label_stack_gap: SCALE,
        reroute_node_gap: 3.0 * SCALE,
        isolated_gap: 3.0 * SCALE,
        isolated_max_columns: 8,
        compound_padding: 2.0 * SCALE,
        compound_header: 0.0,
    };
    options.routing.lane_gap = SCALE;
    options.routing.merge_edges = MergeEdges::Both;
    options.routing.port_sides = PortSides::Any;
    options.order.group_lanes = GroupLanes::Strict;
    options.effort = Effort::Custom(EffortBudget {
        order_seeds: 3,
        order_sweeps: 4,
        exhaustive_layer_max: 4,
        sifting_rounds: 1,
        pullin_rounds: 1,
        kick_patience: 8,
    });
    options.seed_mode = if all_seeded {
        SeedMode::Structure
    } else {
        SeedMode::Placement
    };
    options.limits.max_nodes = Some(input.max_nodes);
    options.limits.max_edges = Some(input.max_edges);
    options.limits.max_dummies = Some(input.max_routing_waypoints);
    options
}

fn convert_output(
    input: &AdapterInput,
    orientation: AdapterOrientation,
    output: &Layout,
    metrics: AdapterMetrics,
) -> Result<AdapterOutput, AdapterFailure> {
    let origin_x = output.bounds.min.x - PADDING;
    let origin_y = output.bounds.min.y - PADDING;
    let mut nodes = Vec::with_capacity(output.nodes.len());
    for (placement, input) in output.nodes.iter().zip(&input.nodes) {
        let min = cell_point(
            Point::new(
                placement.center.x - placement.size.width / 2.0,
                placement.center.y - placement.size.height / 2.0,
            ),
            origin_x,
            origin_y,
        )?;
        nodes.push(AdapterPlacement {
            bounds: CellRect {
                x: min.x,
                y: min.y,
                width: input.size.columns,
                height: input.size.rows,
            },
            rank: placement.rank,
            order: placement.order,
        });
    }
    let mut edges = Vec::with_capacity(output.edges.len());
    for route in &output.edges {
        edges.push(AdapterRoute {
            points: route
                .points
                .iter()
                .copied()
                .map(|point| cell_point(point, origin_x, origin_y))
                .collect::<Result<_, _>>()?,
            source_port: route
                .source_port
                .map(|port| adapter_port(port, origin_x, origin_y))
                .transpose()?,
            target_port: route
                .target_port
                .map(|port| adapter_port(port, origin_x, origin_y))
                .transpose()?,
            kind: match route.kind {
                RouteKind::Direct => AdapterRouteKind::Direct,
                RouteKind::Chained { spans } => AdapterRouteKind::Chained { spans },
                RouteKind::SelfLoop => AdapterRouteKind::SelfLoop,
                RouteKind::Bundled { lane } => AdapterRouteKind::Bundled { lane },
                RouteKind::Unrouted => AdapterRouteKind::Unrouted,
            },
        });
    }
    let mut lanes = Vec::with_capacity(output.lanes.len());
    for lane in &output.lanes {
        let min = cell_point(lane.rect.min, origin_x, origin_y)?;
        let max = cell_point(lane.rect.max, origin_x, origin_y)?;
        let width = max
            .x
            .checked_sub(min.x)
            .and_then(|cells| u16::try_from(cells.max(1)).ok())
            .ok_or(AdapterFailure::Rejected)?;
        let height = max
            .y
            .checked_sub(min.y)
            .and_then(|cells| u16::try_from(cells.max(1)).ok())
            .ok_or(AdapterFailure::Rejected)?;
        lanes.push(AdapterLane {
            group: lane.group.map(|group| group.0),
            bounds: CellRect {
                x: min.x,
                y: min.y,
                width,
                height,
            },
        });
    }
    Ok(AdapterOutput {
        orientation,
        nodes,
        edges,
        reversed: output.reversed.iter().map(|edge| edge.index()).collect(),
        routing_waypoints: output.stats.dummy_count,
        junctions: output
            .junctions
            .iter()
            .copied()
            .map(|point| cell_point(point, origin_x, origin_y))
            .collect::<Result<_, _>>()?,
        lanes,
        metrics,
    })
}

fn adapter_port(
    port: graphlay::PortInfo,
    origin_x: f64,
    origin_y: f64,
) -> Result<AdapterPort, AdapterFailure> {
    Ok(AdapterPort {
        point: cell_point(port.point, origin_x, origin_y)?,
        side: match port.side {
            Side::Left => AdapterSide::Left,
            Side::Right => AdapterSide::Right,
            Side::Top => AdapterSide::Top,
            Side::Bottom => AdapterSide::Bottom,
        },
    })
}

fn cell_point(point: Point, origin_x: f64, origin_y: f64) -> Result<CellPoint, AdapterFailure> {
    Ok(CellPoint {
        x: cell_coordinate((point.x - origin_x) / SCALE)?,
        y: cell_coordinate((point.y - origin_y) / SCALE)?,
    })
}

fn cell_coordinate(value: f64) -> Result<i32, AdapterFailure> {
    let rounded = value.round();
    if !rounded.is_finite() || rounded < f64::from(i32::MIN) || rounded > f64::from(i32::MAX) {
        return Err(AdapterFailure::Rejected);
    }
    #[allow(clippy::cast_possible_truncation)]
    let coordinate = rounded as i32;
    Ok(coordinate)
}

fn map_error(error: LayoutError) -> AdapterFailure {
    match error {
        LayoutError::TooLarge { what, got, limit } => AdapterFailure::TooLarge {
            resource: what.to_string(),
            got,
            limit,
        },
        _ => AdapterFailure::Rejected,
    }
}
