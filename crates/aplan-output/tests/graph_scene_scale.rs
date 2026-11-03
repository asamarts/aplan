//! Reproducible scale baseline for the semantic graph scene (`A1-455`).
//!
//! This is ignored in the ordinary suite because elapsed time is evidence for a
//! later budget, not a reliable verdict on a shared runner. The assertions keep
//! the measured work honest and catch only a catastrophic regression.

use std::hint::black_box;
use std::time::{Duration, Instant};

use aplan_core::{EdgeKind, Status};
use aplan_output::graph_scene::{GraphScene, ScenePoint, SceneRect};
use aplan_output::{GraphEdge, GraphNode, GraphView, Grouping};

const NODES: usize = 10_000;
const LANES: usize = 8;

fn large_view() -> GraphView {
    let nodes = (0..NODES)
        .map(|index| GraphNode {
            id: format!("T-{index:05}"),
            title: format!("representative task {index}"),
            status: Status::Todo,
            done: false,
        })
        .collect();
    let edges = (LANES..NODES)
        .map(|index| GraphEdge {
            from: format!("T-{:05}", index - LANES),
            to: format!("T-{index:05}"),
            kind: EdgeKind::Dependency,
        })
        .collect();
    GraphView {
        nodes,
        edges,
        hidden_edges: 0,
        grouping: Grouping::default(),
        scope: Vec::new(),
    }
}

fn fan_in_view() -> GraphView {
    const GROUPS: usize = 1_111;
    let mut nodes = Vec::with_capacity(GROUPS * (LANES + 1));
    let mut edges = Vec::with_capacity(GROUPS * LANES * 2);
    for group in 0..GROUPS {
        let sink = format!("F-{group:04}-S");
        for lane in 0..LANES {
            let root = format!("F-{group:04}-R-{lane}");
            nodes.push(GraphNode {
                id: root.clone(),
                title: format!("fan-in root {group}/{lane}"),
                status: Status::Todo,
                done: false,
            });
            if group > 0 {
                edges.push(GraphEdge {
                    from: format!("F-{:04}-S", group - 1),
                    to: root.clone(),
                    kind: EdgeKind::Dependency,
                });
            }
            edges.push(GraphEdge {
                from: root,
                to: sink.clone(),
                kind: EdgeKind::Dependency,
            });
        }
        nodes.push(GraphNode {
            id: sink,
            title: format!("fan-in sink {group}"),
            status: Status::Todo,
            done: false,
        });
    }
    GraphView {
        nodes,
        edges,
        hidden_edges: 0,
        grouping: Grouping::default(),
        scope: Vec::new(),
    }
}

fn crossing_view() -> GraphView {
    const GROUPS: usize = 2_000;
    let mut nodes = Vec::with_capacity(GROUPS * 5);
    let mut edges = Vec::with_capacity(GROUPS * 7);
    for group in 0..GROUPS {
        for name in ['A', 'B', 'C', 'D', 'E'] {
            nodes.push(GraphNode {
                id: format!("X-{group:04}-{name}"),
                title: format!("crossing task {group}/{name}"),
                status: Status::Todo,
                done: false,
            });
        }
        if group > 0 {
            for name in ['A', 'B', 'C'] {
                edges.push(GraphEdge {
                    from: format!("X-{:04}-E", group - 1),
                    to: format!("X-{group:04}-{name}"),
                    kind: EdgeKind::Dependency,
                });
            }
        }
        for (from, to) in [('A', 'D'), ('C', 'D'), ('B', 'E'), ('D', 'E')] {
            edges.push(GraphEdge {
                from: format!("X-{group:04}-{from}"),
                to: format!("X-{group:04}-{to}"),
                kind: EdgeKind::Dependency,
            });
        }
    }
    GraphView {
        nodes,
        edges,
        hidden_edges: 0,
        grouping: Grouping::default(),
        scope: Vec::new(),
    }
}

fn measure(label: &str, view: &GraphView) {
    let node_count = view.nodes.len();
    let edge_count = view.edges.len();
    let started = Instant::now();
    let scene = GraphScene::layout(view, 95).expect("representative lanes fit");
    let construction = started.elapsed();
    assert_eq!(scene.nodes.len(), node_count);
    assert_eq!(scene.edges.len(), edge_count);
    assert!(scene.issues.is_empty());
    assert!(scene.edges.iter().all(|edge| !edge.cells.is_empty()));

    let viewport = SceneRect {
        row: scene.bounds.rows / 2,
        column: 0,
        height: 40,
        width: 40,
    };
    let started = Instant::now();
    let mut visible = 0usize;
    for _ in 0..1_000 {
        visible += black_box(scene.connector_cells_in(viewport).count());
    }
    let viewport_time = started.elapsed();
    assert!(visible > 40_000, "the viewport measured no connector work");

    let started = Instant::now();
    let mut hits = 0usize;
    for node in &scene.nodes {
        hits += black_box(scene.hit_test(ScenePoint {
            row: node.marker.row,
            column: node.marker.column,
        }))
        .nodes
        .len();
    }
    let hit_time = started.elapsed();
    assert_eq!(hits, node_count);
    assert!(
        construction < Duration::from_secs(30)
            && viewport_time < Duration::from_secs(10)
            && hit_time < Duration::from_secs(10),
        "catastrophic {label} scene regression: construction {construction:?}, 1,000 viewports \
         {viewport_time:?}, {node_count} hits {hit_time:?}"
    );
    eprintln!(
        "graph scene {label} baseline: {node_count} nodes, {edge_count} edges, {}x{} cells; \
         construction {construction:?}; 1,000 40x40 viewports {viewport_time:?}; {node_count} \
         hits {hit_time:?}",
        scene.bounds.columns, scene.bounds.rows,
    );
}

#[test]
#[ignore = "records the A1-455 construction, viewport and hit-test baseline"]
fn large_scene_construction_viewport_and_hit_testing_baseline() {
    measure("sparse", &large_view());
    measure("fan-in", &fan_in_view());
    measure("crossing-heavy", &crossing_view());
}
