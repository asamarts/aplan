//! Semantic navigation and peer ordering for the layered graph explorer.
//!
//! The painter owns no navigation order. This module consumes typed scene
//! geometry and semantic edges, so arrow movement cannot regress to source,
//! id or formatted-row order.

use std::collections::{BTreeMap, BTreeSet};

use aplan_app::{
    CellPeerOrder, CellPoint, CellScene, CellSceneDirection, GraphExplorerIdentity,
    GraphExplorerProjection, TaskBrowserRow, cell_scene_ordering_hints,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum GraphOrdering {
    #[default]
    Readability,
    PlanPriority,
    Identity,
    Title,
    StatusReadiness,
    Criticality,
}

impl GraphOrdering {
    pub(crate) const fn next(self) -> Self {
        match self {
            Self::Readability => Self::PlanPriority,
            Self::PlanPriority => Self::Identity,
            Self::Identity => Self::Title,
            Self::Title => Self::StatusReadiness,
            Self::StatusReadiness => Self::Criticality,
            Self::Criticality => Self::Readability,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Readability => "readability",
            Self::PlanPriority => "plan priority",
            Self::Identity => "id",
            Self::Title => "title",
            Self::StatusReadiness => "status/readiness",
            Self::Criticality => "criticality",
        }
    }

    pub(crate) const fn layout_cost(self) -> &'static str {
        match self {
            Self::Readability => "crossing-minimized",
            _ => "peer sort may add crossings",
        }
    }

    pub(crate) const fn cell_order(self) -> CellPeerOrder {
        match self {
            Self::Readability => CellPeerOrder::Readability,
            Self::PlanPriority => CellPeerOrder::PlanPriority,
            Self::Identity => CellPeerOrder::Identity,
            Self::Title => CellPeerOrder::Title,
            Self::StatusReadiness => CellPeerOrder::Status,
            Self::Criticality => CellPeerOrder::Criticality,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpatialDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Total peer keys. The layout kernel applies them only inside one phase/rank
/// slot, so ordering never invents dependency rank or phase causality.
pub(crate) fn ordering_hints(
    ordering: GraphOrdering,
    projection: &GraphExplorerProjection,
    catalogue: &[TaskBrowserRow],
) -> BTreeMap<GraphExplorerIdentity, i64> {
    cell_scene_ordering_hints(ordering.cell_order(), projection, catalogue)
}

/// Select a node from the actual scene. Along-flow movement follows a visible
/// edge; cross-flow movement chooses a sibling in the same rank before the
/// nearest other rank.
pub(crate) fn spatial_target(
    scene: &CellScene,
    selected: &GraphExplorerIdentity,
    direction: SpatialDirection,
) -> Option<GraphExplorerIdentity> {
    let current = scene.nodes.iter().find(|node| &node.identity == selected)?;
    let along = match scene.direction {
        CellSceneDirection::LeftToRight => match direction {
            SpatialDirection::Left => Some(false),
            SpatialDirection::Right => Some(true),
            SpatialDirection::Up | SpatialDirection::Down => None,
        },
        CellSceneDirection::TopToBottom => match direction {
            SpatialDirection::Up => Some(false),
            SpatialDirection::Down => Some(true),
            SpatialDirection::Left | SpatialDirection::Right => None,
        },
    };
    if let Some(successor) = along {
        let related: BTreeSet<&GraphExplorerIdentity> = scene
            .edges
            .iter()
            .filter_map(|edge| {
                if successor && edge.identity.from == *selected {
                    Some(&edge.identity.to)
                } else if !successor && edge.identity.to == *selected {
                    Some(&edge.identity.from)
                } else {
                    None
                }
            })
            .collect();
        return nearest(scene, current, direction, |candidate| {
            related.contains(&candidate.identity)
        });
    }
    let same_rank_exists = scene
        .nodes
        .iter()
        .any(|node| node.identity != *selected && node.rank == current.rank);
    nearest(scene, current, direction, |candidate| {
        !same_rank_exists || candidate.rank == current.rank
    })
}

fn nearest(
    scene: &CellScene,
    current: &aplan_app::CellSceneNode,
    direction: SpatialDirection,
    include: impl Fn(&aplan_app::CellSceneNode) -> bool,
) -> Option<GraphExplorerIdentity> {
    let origin = current.bounds.center();
    scene
        .nodes
        .iter()
        .filter(|candidate| candidate.identity != current.identity && include(candidate))
        .filter_map(|candidate| {
            let center = candidate.bounds.center();
            let (primary, cross) = directional_distance(origin, center, direction)?;
            Some((primary, cross, candidate.identity.clone()))
        })
        .min()
        .map(|(_, _, identity)| identity)
}

fn directional_distance(
    origin: CellPoint,
    target: CellPoint,
    direction: SpatialDirection,
) -> Option<(u32, u32)> {
    let (primary, cross) = match direction {
        SpatialDirection::Left if target.x < origin.x => {
            (origin.x - target.x, (origin.y - target.y).unsigned_abs())
        }
        SpatialDirection::Right if target.x > origin.x => {
            (target.x - origin.x, (origin.y - target.y).unsigned_abs())
        }
        SpatialDirection::Up if target.y < origin.y => {
            (origin.y - target.y, (origin.x - target.x).unsigned_abs())
        }
        SpatialDirection::Down if target.y > origin.y => {
            (target.y - origin.y, (origin.x - target.x).unsigned_abs())
        }
        _ => return None,
    };
    Some((u32::try_from(primary).unwrap_or(u32::MAX), cross))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aplan_app::{CellRect, CellSceneNode};

    fn node(id: &str, x: i32, y: i32, rank: i32) -> CellSceneNode {
        CellSceneNode {
            identity: GraphExplorerIdentity::Task(id.to_string()),
            bounds: CellRect {
                x,
                y,
                width: 6,
                height: 3,
            },
            rank: Some(rank),
            order: 0,
        }
    }

    #[test]
    fn directional_distance_refuses_the_wrong_half_plane() {
        let origin = CellPoint { x: 5, y: 5 };
        assert!(
            directional_distance(origin, CellPoint { x: 4, y: 9 }, SpatialDirection::Left)
                .is_some()
        );
        assert!(
            directional_distance(origin, CellPoint { x: 6, y: 5 }, SpatialDirection::Left)
                .is_none()
        );
    }

    #[test]
    fn nearest_cross_flow_prefers_the_same_rank() {
        let current = node("A", 10, 10, 1);
        let scene_nodes = [
            current.clone(),
            node("near-other-rank", 10, 5, 0),
            node("same-rank", 30, 0, 1),
        ];
        let same_rank_exists = scene_nodes.iter().any(|candidate| {
            candidate.identity != current.identity && candidate.rank == current.rank
        });
        let target = scene_nodes
            .iter()
            .filter(|candidate| {
                candidate.identity != current.identity
                    && (!same_rank_exists || candidate.rank == current.rank)
            })
            .filter_map(|candidate| {
                let (primary, cross) = directional_distance(
                    current.bounds.center(),
                    candidate.bounds.center(),
                    SpatialDirection::Up,
                )?;
                Some((primary, cross, candidate.identity.clone()))
            })
            .min()
            .map(|(_, _, identity)| identity);
        assert_eq!(
            target,
            Some(GraphExplorerIdentity::Task("same-rank".to_string()))
        );
    }
}
