use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::process::Command;
use std::time::Instant;

use aplan_core::{EdgeKind, Status};
use aplan_output::graph_scene::{ConnectorKind, GraphScene};
use aplan_output::{GraphEdge, GraphNode, GraphView, Grouping};
use graphlay::{
    layout, metrics, Direction, Edge, Effort, EffortBudget, Graph, GraphBuilder, GroupId,
    GroupLanes, LayoutOptions, Node, Point, SeedMode, Size, Spacing,
};
use serde::Deserialize;
use unicode_width::UnicodeWidthStr;

const SIZES: [(usize, usize); 4] = [(48, 14), (80, 24), (120, 36), (160, 50)];
const SCALE: f64 = 10.0;

#[derive(Clone)]
struct Spec {
    name: String,
    view: GraphView,
    groups: BTreeMap<String, u32>,
    anchor: Option<String>,
}

#[derive(Deserialize)]
struct TaskEnvelope {
    tasks: Vec<TaskRow>,
}

#[derive(Deserialize)]
struct TaskRow {
    id: String,
    title: String,
    phase: u32,
    status: Status,
    milestone: Option<String>,
    milestone_task: bool,
}

#[derive(Clone, Copy)]
struct BoxAt {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Default)]
struct SimpleResult {
    boxes: BTreeMap<String, BoxAt>,
    width: f64,
    height: f64,
    crossings: usize,
    path_risk_edges: usize,
    feedback: usize,
}

fn command_json(binary: &str, repo: &str, args: &[&str]) -> serde_json::Value {
    let output = Command::new(binary)
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run aplan");
    assert!(
        output.status.success(),
        "aplan failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("aplan JSON")
}

fn real_specs(binary: &str, repo: &str) -> Vec<Spec> {
    let whole: GraphView =
        serde_json::from_value(command_json(binary, repo, &["graph", "--format", "json"]))
            .expect("whole graph");
    let cut: GraphView = serde_json::from_value(command_json(
        binary,
        repo,
        &["graph", "--format", "json", "--cut", "v0.13"],
    ))
    .expect("cut graph");
    let tasks: TaskEnvelope =
        serde_json::from_value(command_json(binary, repo, &["tasks", "--format", "json"]))
            .expect("tasks");
    let overview = milestone_overview(&whole, &tasks.tasks);
    vec![
        Spec {
            name: "real-whole-tasks".into(),
            view: whole,
            groups: BTreeMap::new(),
            anchor: Some("A1-676".into()),
        },
        Spec {
            name: "real-v0.13-tasks".into(),
            view: cut,
            groups: BTreeMap::new(),
            anchor: Some("A1-676".into()),
        },
        overview,
    ]
}

fn milestone_overview(whole: &GraphView, tasks: &[TaskRow]) -> Spec {
    let rows: BTreeMap<&str, &TaskRow> = tasks.iter().map(|row| (row.id.as_str(), row)).collect();
    let milestone_of = |id: &str| -> Option<String> {
        rows.get(id).and_then(|row| {
            if row.milestone_task {
                Some(row.id.clone())
            } else {
                row.milestone.clone()
            }
        })
    };
    let mut ids: BTreeSet<String> = tasks
        .iter()
        .filter(|row| row.milestone_task)
        .map(|row| row.id.clone())
        .collect();
    let mut edges = BTreeSet::new();
    for edge in whole
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Dependency)
    {
        let (Some(from), Some(to)) = (milestone_of(&edge.from), milestone_of(&edge.to)) else {
            continue;
        };
        ids.insert(from.clone());
        ids.insert(to.clone());
        if from != to {
            edges.insert((from, to));
        }
    }
    let nodes = ids
        .iter()
        .filter_map(|id| rows.get(id.as_str()))
        .map(|row| GraphNode {
            id: row.id.clone(),
            title: row.title.clone(),
            status: row.status,
            done: row.status.satisfies_dependency(),
        })
        .collect();
    let graph_edges = edges
        .into_iter()
        .map(|(from, to)| GraphEdge {
            from,
            to,
            kind: EdgeKind::Dependency,
        })
        .collect();
    let groups = ids
        .iter()
        .filter_map(|id| rows.get(id.as_str()).map(|row| (id.clone(), row.phase)))
        .collect();
    Spec {
        name: "real-plan-milestones".into(),
        view: GraphView {
            nodes,
            edges: graph_edges,
            hidden_edges: 0,
            grouping: Grouping::default(),
            scope: vec!["milestone aggregate".into()],
        },
        groups,
        anchor: Some("M-v0.13".into()),
    }
}

fn synthetic_specs() -> Vec<Spec> {
    let mut specs = Vec::new();
    specs.push(synthetic(
        "chain",
        12,
        (0..11).map(|i| (i, i + 1)).collect(),
        false,
    ));
    specs.push(synthetic(
        "diamond",
        8,
        vec![
            (0, 1),
            (0, 2),
            (1, 3),
            (2, 3),
            (3, 4),
            (3, 5),
            (4, 6),
            (5, 6),
            (6, 7),
        ],
        false,
    ));
    let mut fan = Vec::new();
    for i in 1..=12 {
        fan.push((0, i));
        fan.push((i, 13));
    }
    specs.push(synthetic("high-fan", 14, fan, false));
    let mut fan_window = Vec::new();
    for i in 1..=48 {
        fan_window.push((0, i));
        fan_window.push((i, 49));
    }
    specs.push(synthetic("high-fan-window", 50, fan_window, false));
    let mut crossing = Vec::new();
    for rank in 0..4 {
        for i in 0..8 {
            crossing.push((rank * 8 + i, (rank + 1) * 8 + (7 - i)));
        }
        for i in 0..7 {
            crossing.push((rank * 8 + i, (rank + 1) * 8 + i + 1));
        }
    }
    specs.push(synthetic("crossing-heavy", 40, crossing, false));
    let mut crossing_window = Vec::new();
    for rank in 0..6 {
        for i in 0..8 {
            crossing_window.push((rank * 8 + i, (rank + 1) * 8 + (7 - i)));
        }
        for i in 0..7 {
            crossing_window.push((rank * 8 + i, (rank + 1) * 8 + i + 1));
        }
    }
    specs.push(synthetic("crossing-window", 56, crossing_window, false));
    let mut cross_cut = synthetic(
        "cross-cut",
        12,
        vec![
            (0, 6),
            (6, 8),
            (1, 7),
            (7, 9),
            (10, 2),
            (2, 4),
            (11, 3),
            (3, 5),
        ],
        false,
    );
    cross_cut.groups = (0..12)
        .map(|i| (format!("N-{i:02}"), if i < 6 { 1 } else { 2 }))
        .collect();
    cross_cut.anchor = Some("N-06".into());
    specs.push(cross_cut);
    specs.push(synthetic(
        "disconnected",
        18,
        vec![(0, 1), (1, 2), (3, 4), (3, 5), (6, 7), (8, 9), (9, 10)],
        false,
    ));
    specs.push(synthetic(
        "cycle",
        6,
        vec![(0, 1), (1, 2), (2, 0), (2, 3), (3, 4), (4, 5)],
        false,
    ));
    specs.push(synthetic(
        "unicode",
        8,
        vec![
            (0, 1),
            (0, 2),
            (1, 3),
            (2, 3),
            (3, 4),
            (3, 5),
            (4, 6),
            (5, 7),
        ],
        true,
    ));
    specs
}

fn synthetic(name: &str, count: usize, pairs: Vec<(usize, usize)>, unicode: bool) -> Spec {
    let labels = [
        "alpha",
        "\u{754c}",
        "e\u{301}",
        "\u{1f642}",
        "\u{1f469}\u{200d}\u{1f4bb}",
        "\u{645}\u{631}\u{62d}\u{628}\u{627}",
        "a-very-long-title",
        "omega",
    ];
    let nodes = (0..count)
        .map(|i| GraphNode {
            id: format!("N-{i:02}"),
            title: if unicode {
                labels[i % labels.len()].to_string()
            } else {
                format!("fixture task {i}")
            },
            status: Status::Todo,
            done: false,
        })
        .collect();
    let edges = pairs
        .into_iter()
        .map(|(from, to)| GraphEdge {
            from: format!("N-{from:02}"),
            to: format!("N-{to:02}"),
            kind: EdgeKind::Dependency,
        })
        .collect();
    Spec {
        name: name.into(),
        view: GraphView {
            nodes,
            edges,
            hidden_edges: 0,
            grouping: Grouping::default(),
            scope: vec![],
        },
        groups: BTreeMap::new(),
        anchor: Some("N-00".into()),
    }
}

fn node_size(node: &GraphNode) -> (f64, f64) {
    let label = format!("{} {}", node.id, node.title);
    (
        (UnicodeWidthStr::width(label.as_str()) + 2).clamp(10, 26) as f64,
        3.0,
    )
}

fn simple_layout(spec: &Spec, direction: Direction) -> SimpleResult {
    let ids: Vec<String> = spec.view.nodes.iter().map(|n| n.id.clone()).collect();
    let id_set: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    let mut incoming: BTreeMap<String, Vec<String>> =
        ids.iter().map(|id| (id.clone(), vec![])).collect();
    let mut outgoing = incoming.clone();
    for edge in spec.view.edges.iter().filter(|e| {
        e.kind == EdgeKind::Dependency
            && id_set.contains(e.from.as_str())
            && id_set.contains(e.to.as_str())
    }) {
        incoming.get_mut(&edge.to).unwrap().push(edge.from.clone());
        outgoing.get_mut(&edge.from).unwrap().push(edge.to.clone());
    }
    let mut indegree: BTreeMap<String, usize> = incoming
        .iter()
        .map(|(id, edges)| (id.clone(), edges.len()))
        .collect();
    let mut queue: VecDeque<String> = indegree
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut rank: BTreeMap<String, usize> = ids.iter().map(|id| (id.clone(), 0)).collect();
    let mut visited = BTreeSet::new();
    while let Some(id) = queue.pop_front() {
        visited.insert(id.clone());
        let next_rank = rank[&id] + 1;
        for next in &outgoing[&id] {
            rank.entry(next.clone())
                .and_modify(|r| *r = (*r).max(next_rank));
            let degree = indegree.get_mut(next).unwrap();
            *degree -= 1;
            if *degree == 0 {
                queue.push_back(next.clone());
            }
        }
    }
    let feedback = ids.len() - visited.len();
    for id in ids.iter().filter(|id| !visited.contains(*id)) {
        rank.insert(id.clone(), 0);
    }
    let max_rank = rank.values().copied().max().unwrap_or(0);
    let mut layers = vec![Vec::new(); max_rank + 1];
    for id in &ids {
        layers[rank[id]].push(id.clone());
    }
    for layer in &mut layers {
        layer.sort();
    }
    for _ in 0..4 {
        for r in 1..layers.len() {
            reorder(&mut layers, r, &incoming);
        }
        for r in (0..layers.len().saturating_sub(1)).rev() {
            reorder(&mut layers, r, &outgoing);
        }
    }
    let node_map: BTreeMap<&str, &GraphNode> =
        spec.view.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut boxes = BTreeMap::new();
    let mut along = 0.0_f64;
    for layer in &layers {
        let mut cross = 0.0_f64;
        let along_extent = layer
            .iter()
            .map(|id| {
                let (w, h) = node_size(node_map[id.as_str()]);
                if direction == Direction::LeftRight {
                    w
                } else {
                    h
                }
            })
            .fold(0.0, f64::max);
        for id in layer {
            let (w, h) = node_size(node_map[id.as_str()]);
            let b = if direction == Direction::LeftRight {
                BoxAt {
                    x: along,
                    y: cross,
                    w,
                    h,
                }
            } else {
                BoxAt {
                    x: cross,
                    y: along,
                    w,
                    h,
                }
            };
            cross += if direction == Direction::LeftRight {
                h + 2.0
            } else {
                w + 2.0
            };
            boxes.insert(id.clone(), b);
        }
        along += along_extent + 4.0;
    }
    let width = boxes.values().map(|b| b.x + b.w).fold(0.0, f64::max);
    let height = boxes.values().map(|b| b.y + b.h).fold(0.0, f64::max);
    let segments: Vec<_> = spec
        .view
        .edges
        .iter()
        .enumerate()
        .filter_map(|(edge_index, e)| {
            let (a, b) = (boxes.get(&e.from)?, boxes.get(&e.to)?);
            Some((
                (a.x + a.w / 2.0, a.y + a.h / 2.0),
                (b.x + b.w / 2.0, b.y + b.h / 2.0),
                &e.from,
                &e.to,
                edge_index,
            ))
        })
        .collect();
    let mut crossings = 0;
    let mut path_risk_edges = BTreeSet::new();
    for i in 0..segments.len() {
        for j in i + 1..segments.len() {
            if segments[i].2 == segments[j].2
                || segments[i].2 == segments[j].3
                || segments[i].3 == segments[j].2
                || segments[i].3 == segments[j].3
            {
                continue;
            }
            let crosses = proper_cross(segments[i].0, segments[i].1, segments[j].0, segments[j].1);
            crossings += usize::from(crosses);
            if crosses {
                path_risk_edges.insert(segments[i].4);
                path_risk_edges.insert(segments[j].4);
            }
        }
    }
    SimpleResult {
        boxes,
        width,
        height,
        crossings,
        path_risk_edges: path_risk_edges.len(),
        feedback,
    }
}

fn reorder(layers: &mut [Vec<String>], r: usize, neighbours: &BTreeMap<String, Vec<String>>) {
    let positions: BTreeMap<String, f64> = layers
        .iter()
        .flat_map(|layer| {
            layer
                .iter()
                .enumerate()
                .map(|(i, id)| (id.clone(), i as f64))
        })
        .collect();
    layers[r].sort_by(|a, b| {
        let score = |id: &str| {
            let mut xs: Vec<f64> = neighbours[id]
                .iter()
                .filter_map(|n| positions.get(n).copied())
                .collect();
            xs.sort_by(f64::total_cmp);
            if xs.is_empty() {
                f64::INFINITY
            } else {
                xs[xs.len() / 2]
            }
        };
        score(a).total_cmp(&score(b)).then(a.cmp(b))
    });
}

fn proper_cross(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    fn orient(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    }
    let (x, y, z, w) = (
        orient(a, b, c),
        orient(a, b, d),
        orient(c, d, a),
        orient(c, d, b),
    );
    x * y < 0.0 && z * w < 0.0
}

fn graphlay_graph(
    spec: &Spec,
    reverse: bool,
    seeds: Option<&BTreeMap<String, Point>>,
) -> (Graph, Vec<String>) {
    let mut nodes = spec.view.nodes.clone();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    if reverse {
        nodes.reverse();
    }
    let mut builder = GraphBuilder::new();
    let mut map = BTreeMap::new();
    let mut ordered = Vec::new();
    for node in nodes {
        let (w, h) = node_size(&node);
        let id = builder.add_node(Node {
            size: Size::new(w * SCALE, h * SCALE),
            key: Some(node.id.clone()),
            group: spec.groups.get(&node.id).copied().map(GroupId),
            position: seeds.and_then(|s| s.get(&node.id).copied()),
            ..Node::default()
        });
        map.insert(node.id.clone(), id);
        ordered.push(node.id);
    }
    let mut edges = spec.view.edges.clone();
    edges.sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
    for edge in edges {
        if let (Some(&a), Some(&b)) = (map.get(&edge.from), map.get(&edge.to)) {
            builder.add_edge(Edge {
                thickness: 1.0,
                ..Edge::new(a, b)
            });
        }
    }
    (builder.build(), ordered)
}

fn options(direction: Direction, groups: bool) -> LayoutOptions {
    let mut o = LayoutOptions::layered_default();
    o.direction = direction;
    o.effort = Effort::Fast;
    o.spacing = Spacing {
        layer_gap: 4.0 * SCALE,
        node_gap: 2.0 * SCALE,
        dummy_gap: 1.0 * SCALE,
        port_gap: 1.0 * SCALE,
        port_stroke_clear: 0.2 * SCALE,
        stub: 1.0 * SCALE,
        bundle_lane_gap: 1.0 * SCALE,
        self_loop_gap: 2.0 * SCALE,
        edge_label_margin: 1.0 * SCALE,
        edge_label_stack_gap: 1.0 * SCALE,
        reroute_node_gap: 2.0 * SCALE,
        isolated_gap: 2.0 * SCALE,
        isolated_max_columns: 8,
        compound_padding: 1.0 * SCALE,
        compound_header: 1.0 * SCALE,
    };
    o.routing.lane_gap = 1.0 * SCALE;
    if groups {
        o.order.group_lanes = GroupLanes::Strict;
    }
    o.limits.max_nodes = Some(1_000);
    o.limits.max_edges = Some(2_000);
    o.limits.max_dummies = Some(10_000);
    o
}

fn visible(
    boxes: &BTreeMap<String, BoxAt>,
    anchor: Option<&str>,
    width: usize,
    height: usize,
) -> usize {
    let centre = anchor
        .and_then(|id| boxes.get(id))
        .map(|b| (b.x + b.w / 2.0, b.y + b.h / 2.0))
        .unwrap_or((width as f64 / 2.0, height as f64 / 2.0));
    let left = centre.0 - width as f64 / 2.0;
    let top = centre.1 - height as f64 / 2.0;
    boxes
        .values()
        .filter(|b| {
            b.x + b.w > left
                && b.x < left + width as f64
                && b.y + b.h > top
                && b.y < top + height as f64
        })
        .count()
}

fn graphlay_boxes(ordered: &[String], layout: &graphlay::Layout) -> BTreeMap<String, BoxAt> {
    ordered
        .iter()
        .zip(&layout.nodes)
        .map(|(id, p)| {
            (
                id.clone(),
                BoxAt {
                    x: (p.center.x - p.size.width / 2.0) / SCALE,
                    y: (p.center.y - p.size.height / 2.0) / SCALE,
                    w: p.size.width / SCALE,
                    h: p.size.height / SCALE,
                },
            )
        })
        .collect()
}

fn quantized_node_overlaps(boxes: &BTreeMap<String, BoxAt>) -> usize {
    let values: Vec<_> = boxes
        .values()
        .map(|b| {
            (
                b.x.round(),
                b.y.round(),
                (b.x + b.w).round(),
                (b.y + b.h).round(),
            )
        })
        .collect();
    let mut total = 0;
    for i in 0..values.len() {
        for j in i + 1..values.len() {
            let a = values[i];
            let b = values[j];
            total += usize::from(a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3);
        }
    }
    total
}

fn lane_path_risk_edges(scene: &GraphScene) -> usize {
    scene
        .rows()
        .iter()
        .flat_map(|row| row.connectors.iter())
        .filter(|cell| cell.kind == ConnectorKind::Cross)
        .flat_map(|cell| cell.edges.iter().cloned())
        .collect::<BTreeSet<_>>()
        .len()
}

fn graphlay_path_risk_edges(graph: &Graph, laid: &graphlay::Layout) -> usize {
    let mut risk = BTreeSet::new();
    for (edge_index, route) in laid.edges.iter().enumerate() {
        if route.points.windows(2).any(|segment| {
            (segment[0].x / SCALE).round() == (segment[1].x / SCALE).round()
                && (segment[0].y / SCALE).round() == (segment[1].y / SCALE).round()
        }) {
            risk.insert(edge_index);
        }
    }
    for i in 0..laid.edges.len() {
        for j in i + 1..laid.edges.len() {
            let first = &graph.edges[i];
            let second = &graph.edges[j];
            if first.source == second.source
                || first.source == second.target
                || first.target == second.source
                || first.target == second.target
            {
                continue;
            }
            let intersects = laid.edges[i].points.windows(2).any(|a| {
                laid.edges[j]
                    .points
                    .windows(2)
                    .any(|b| segments_intersect(a[0], a[1], b[0], b[1]))
            });
            if intersects {
                risk.insert(i);
                risk.insert(j);
            }
        }
    }
    risk.len()
}

fn segments_intersect(a: Point, b: Point, c: Point, d: Point) -> bool {
    const EPSILON: f64 = 1e-6;
    let orient =
        |p: Point, q: Point, r: Point| (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
    let (ab_c, ab_d, cd_a, cd_b) = (
        orient(a, b, c),
        orient(a, b, d),
        orient(c, d, a),
        orient(c, d, b),
    );
    if ab_c * ab_d < -EPSILON && cd_a * cd_b < -EPSILON {
        return true;
    }
    let within = |value: f64, left: f64, right: f64| {
        value >= left.min(right) - EPSILON && value <= left.max(right) + EPSILON
    };
    (ab_c.abs() <= EPSILON && within(c.x, a.x, b.x) && within(c.y, a.y, b.y))
        || (ab_d.abs() <= EPSILON && within(d.x, a.x, b.x) && within(d.y, a.y, b.y))
        || (cd_a.abs() <= EPSILON && within(a.x, c.x, d.x) && within(a.y, c.y, d.y))
        || (cd_b.abs() <= EPSILON && within(b.x, c.x, d.x) && within(b.y, c.y, d.y))
}

fn run_spec(spec: &Spec) {
    println!(
        "VIEW {} nodes={} edges={}",
        spec.name,
        spec.view.nodes.len(),
        spec.view.edges.len()
    );
    for (w, h) in SIZES {
        match GraphScene::layout(&spec.view, w) {
            Ok(scene) => {
                let boxes: BTreeMap<String, BoxAt> = scene
                    .nodes
                    .iter()
                    .map(|n| {
                        (
                            n.id.clone(),
                            BoxAt {
                                x: n.marker.column as f64,
                                y: n.marker.row as f64,
                                w: 1.0,
                                h: 1.0,
                            },
                        )
                    })
                    .collect();
                println!(
                    " lane {w}x{h} scene={}x{} visible={} path_risk_edges={}",
                    scene.bounds.columns,
                    scene.bounds.rows,
                    visible(&boxes, spec.anchor.as_deref(), w, h),
                    lane_path_risk_edges(&scene)
                );
            }
            Err(e) => println!(
                " lane {w}x{h} REFUSED lanes={} needed={} effective_width={}",
                e.lanes, e.needed, e.width
            ),
        }
    }
    for direction in [Direction::LeftRight, Direction::TopBottom] {
        let simple = simple_layout(spec, direction);
        print!(
            " simple {:?} bounds={:.0}x{:.0} crossings={} path_risk_edges={} feedback={}",
            direction,
            simple.width,
            simple.height,
            simple.crossings,
            simple.path_risk_edges,
            simple.feedback
        );
        for (w, h) in SIZES {
            print!(
                " {w}x{h}:{}",
                visible(&simple.boxes, spec.anchor.as_deref(), w, h)
            );
        }
        println!();

        let (graph, ordered) = graphlay_graph(spec, false, None);
        let opts = options(direction, !spec.groups.is_empty());
        let mut times = Vec::new();
        let mut chosen = None;
        for _ in 0..5 {
            let start = Instant::now();
            let l = layout(&graph, &opts).expect("graphlay layout");
            times.push(start.elapsed());
            chosen = Some(l);
        }
        times.sort();
        let laid = chosen.unwrap();
        let again = layout(&graph, &opts).expect("repeat");
        assert_eq!(laid, again, "graphlay nondeterministic");
        let m = metrics::measure(&graph, &opts, &laid);
        let boxes = graphlay_boxes(&ordered, &laid);
        let collapsed = laid
            .edges
            .iter()
            .flat_map(|e| e.points.windows(2))
            .filter(|p| {
                (p[0].x / SCALE).round() == (p[1].x / SCALE).round()
                    && (p[0].y / SCALE).round() == (p[1].y / SCALE).round()
            })
            .count();
        let (reverse_graph, reverse_order) = graphlay_graph(spec, true, None);
        let reverse_laid = layout(&reverse_graph, &opts).expect("reverse order");
        let reverse_boxes = graphlay_boxes(&reverse_order, &reverse_laid);
        let shuffle_delta = boxes
            .iter()
            .filter_map(|(id, a)| {
                reverse_boxes
                    .get(id)
                    .map(|b| (a.x - b.x).abs().max((a.y - b.y).abs()))
            })
            .fold(0.0_f64, f64::max);
        let quant_overlaps = quantized_node_overlaps(&boxes);
        let path_risk_edges = graphlay_path_risk_edges(&graph, &laid);
        print!(" graphlay-fast {:?} bounds={:.0}x{:.0} crossings={} path_risk_edges={} bends={} overlaps={} node_hits={} collapsed={} quant_node_overlap={} shuffle_delta={:.2} median_us={}",direction,m.width/SCALE,m.height/SCALE,m.crossings,path_risk_edges,m.bends,m.overlaps,m.node_edge_hits,collapsed,quant_overlaps,shuffle_delta,times[2].as_micros());
        for (w, h) in SIZES {
            print!(" {w}x{h}:{}", visible(&boxes, spec.anchor.as_deref(), w, h));
        }
        println!();
        println!(
            " minimap-capacity {:?} wide=160x50:{} reserve-26x12:{}",
            direction,
            visible(&boxes, spec.anchor.as_deref(), 160, 50),
            visible(&boxes, spec.anchor.as_deref(), 134, 50)
        );

        if spec.view.nodes.len() <= 64 {
            let mut balanced_opts = opts;
            balanced_opts.effort = Effort::Balanced;
            let mut balanced_times = Vec::new();
            let mut balanced = None;
            for _ in 0..3 {
                let start = Instant::now();
                let l = layout(&graph, &balanced_opts).expect("balanced");
                balanced_times.push(start.elapsed());
                balanced = Some(l);
            }
            balanced_times.sort();
            let balanced = balanced.unwrap();
            let bm = metrics::measure(&graph, &balanced_opts, &balanced);
            println!(" graphlay-balanced {:?} bounds={:.0}x{:.0} crossings={} bends={} overlaps={} node_hits={} median_us={}",direction,bm.width/SCALE,bm.height/SCALE,bm.crossings,bm.bends,bm.overlaps,bm.node_edge_hits,balanced_times[1].as_micros());

            let mut interactive_opts = opts;
            interactive_opts.effort = Effort::Custom(EffortBudget {
                order_seeds: 1,
                order_sweeps: 2,
                exhaustive_layer_max: 4,
                sifting_rounds: 0,
                pullin_rounds: 0,
                kick_patience: 1,
            });
            interactive_opts.tuning.order.kick_rounds = 0;
            interactive_opts.tuning.order.chain_lane_rounds = 0;
            interactive_opts.tuning.order.balance_rounds = 0;
            let mut interactive_times = Vec::new();
            let mut interactive = None;
            for _ in 0..3 {
                let start = Instant::now();
                let l = layout(&graph, &interactive_opts).expect("interactive");
                interactive_times.push(start.elapsed());
                interactive = Some(l);
            }
            interactive_times.sort();
            let interactive = interactive.unwrap();
            let im = metrics::measure(&graph, &interactive_opts, &interactive);
            println!(" graphlay-interactive {:?} bounds={:.0}x{:.0} crossings={} bends={} overlaps={} node_hits={} median_us={}",direction,im.width/SCALE,im.height/SCALE,im.crossings,im.bends,im.overlaps,im.node_edge_hits,interactive_times[1].as_micros());
        }

        let seeds: BTreeMap<String, Point> = ordered
            .iter()
            .zip(&laid.nodes)
            .map(|(id, p)| (id.clone(), p.center))
            .collect();
        let (seeded_graph, seed_order) = graphlay_graph(spec, false, Some(&seeds));
        let mut seed_opts = opts;
        seed_opts.seed_mode = SeedMode::Structure;
        let seeded = layout(&seeded_graph, &seed_opts).expect("seeded");
        let pairs: Vec<_> = (0..seed_order.len())
            .map(|i| (graphlay::NodeId(i as u32), graphlay::NodeId(i as u32)))
            .collect();
        let stable = metrics::compare(&laid, &seeded, &pairs);
        println!(
            " stability {:?} common={} hausdorff_cells={:.3} ordering_flip={:.4}",
            direction,
            stable.common_nodes,
            stable.hausdorff / SCALE,
            stable.orthogonal_ordering
        );
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let binary = args.next().expect("aplan binary");
    let repo = args.next().expect("repo");
    let mut specs = real_specs(&binary, &repo);
    specs.extend(synthetic_specs());
    let started = Instant::now();
    for spec in &specs {
        run_spec(spec);
    }
    eprintln!("total {:?}", started.elapsed());
}
