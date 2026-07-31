//! The dependency graph.
//!
//! Tasks form a DAG under `blocked_by`. Two questions matter operationally:
//! what can I work on now (`ready`), and why can't I work on this (`why`).
//! Both are cheap; the expensive-to-get-wrong one is proving acyclicity.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::task::{Status, Task};

/// The critical path: the longest chain of dependent tasks weighted by
/// `effort_days`, the plan's spine. Advisory, never a gate (a plan is not wrong
/// for being long). Owned so it renders without borrowing the graph.
#[derive(Debug, Clone, PartialEq)]
pub struct CriticalPath {
    /// Total effort along the path, in days.
    pub total_effort_days: f64,
    /// The tasks on the path, in execution order (blockers first).
    pub tasks: Vec<CriticalTask>,
}

/// One task on the [`CriticalPath`].
#[derive(Debug, Clone, PartialEq)]
pub struct CriticalTask {
    pub id: String,
    pub title: String,
    /// The task's own `effort_days` (0 when unset), its contribution to the total.
    pub effort_days: f64,
    pub status: Status,
}

/// An index over tasks by id. `BTreeMap` so iteration order is deterministic,
/// diagnostics must not reorder between runs.
#[derive(Debug, Default)]
pub struct Graph<'a> {
    pub by_id: BTreeMap<&'a str, &'a Task>,
}

impl<'a> Graph<'a> {
    pub fn new(tasks: impl IntoIterator<Item = &'a Task>) -> Self {
        let mut by_id = BTreeMap::new();
        for t in tasks {
            // First writer wins; duplicate ids are reported separately by the
            // validator, and clobbering here would hide the second one.
            by_id.entry(t.id.as_str()).or_insert(t);
        }
        Self { by_id }
    }

    pub fn get(&self, id: &str) -> Option<&'a Task> {
        self.by_id.get(id).copied()
    }

    pub fn is_done(&self, id: &str) -> bool {
        self.get(id)
            .is_some_and(|t| t.status.satisfies_dependency())
    }

    /// The direct `blocked_by` entries of `id` that name no known task.
    ///
    /// [`why_blocked`](Self::why_blocked) walks only real tasks, so without
    /// this a task blocked solely by an unknown id would look "ready", the
    /// opposite of what `check` reports, which flags the same edge as
    /// `APLAN003`.
    pub fn direct_unknown_blockers(&self, id: &str) -> Vec<&'a str> {
        let Some(t) = self.get(id) else {
            return Vec::new();
        };
        t.blocked_by
            .iter()
            .map(String::as_str)
            .filter(|b| !self.by_id.contains_key(*b))
            .collect()
    }

    /// Tasks that are not terminal, not milestones, and whose blockers are all
    /// `done`. Milestones are checkpoints, never workable, so advertising one as
    /// ready would offer a checkpoint as if it were work.
    pub fn ready(&self) -> Vec<&'a Task> {
        self.by_id
            .values()
            .copied()
            .filter(|t| !t.is_milestone())
            .filter(|t| t.is_ready(|b| self.is_done(b)))
            .collect()
    }

    /// The unmet blockers of `id`, transitively, nearest first.
    ///
    /// Returns an empty vec when the task is ready (or unknown).
    pub fn why_blocked(&self, id: &str) -> Vec<&'a Task> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        let mut queue = VecDeque::new();

        if let Some(t) = self.get(id) {
            for b in &t.blocked_by {
                queue.push_back(b.as_str());
            }
        }

        while let Some(cur) = queue.pop_front() {
            if !seen.insert(cur) {
                continue;
            }
            let Some(task) = self.get(cur) else { continue };
            if task.status.satisfies_dependency() {
                continue;
            }
            out.push(task);
            for b in &task.blocked_by {
                queue.push_back(b.as_str());
            }
        }
        out
    }

    /// Find one dependency cycle, if any, as the sequence of ids forming it.
    ///
    /// Returns the first cycle found in deterministic (sorted) id order. One is
    /// enough: a cycle is a hard error and the author must break it before the
    /// rest of the graph means anything.
    pub fn find_cycle(&self) -> Option<Vec<String>> {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            White,
            Grey,
            Black,
        }

        let mut mark: BTreeMap<&str, Mark> = self.by_id.keys().map(|k| (*k, Mark::White)).collect();
        let mut stack: Vec<&str> = Vec::new();

        // Iterative DFS so a pathological plan cannot blow the Rust stack.
        for start in self.by_id.keys().copied() {
            if mark[start] != Mark::White {
                continue;
            }
            let mut work: Vec<(&str, usize)> = vec![(start, 0)];
            while let Some((node, idx)) = work.pop() {
                if idx == 0 {
                    if mark[node] == Mark::Grey {
                        // Already on the current path: cycle closes here.
                        let pos = stack.iter().position(|n| *n == node).unwrap_or(0);
                        let mut cyc: Vec<String> =
                            stack[pos..].iter().map(|s| (*s).to_string()).collect();
                        cyc.push(node.to_string());
                        return Some(cyc);
                    }
                    if mark[node] == Mark::Black {
                        continue;
                    }
                    mark.insert(node, Mark::Grey);
                    stack.push(node);
                }

                let deps: &[String] = self.get(node).map_or(&[], |t| t.blocked_by.as_slice());
                if idx < deps.len() {
                    work.push((node, idx + 1));
                    let next = deps[idx].as_str();
                    // Unknown deps are reported by the validator, not here; a
                    // self-edge is a self-dependency (`SELF_DEP`), also reported
                    // separately, surface neither as a graph cycle.
                    if next != node && self.by_id.contains_key(next) {
                        work.push((next, 0));
                    }
                } else {
                    mark.insert(node, Mark::Black);
                    stack.pop();
                }
            }
        }
        None
    }

    /// A topological order of every known task (blockers before dependents), or
    /// `None` if the graph is cyclic. Deterministic: ties break by id, so the
    /// same plan always yields the same order across runs and filesystems.
    pub fn toposort(&self) -> Option<Vec<&'a str>> {
        let nodes: BTreeSet<&str> = self.by_id.keys().copied().collect();
        self.toposort_nodes(&nodes)
    }

    /// Kahn's algorithm over `nodes` only (edges to tasks outside the set, or
    /// unknown ids, are ignored, so a subset can be sorted on its own). Processes
    /// in-degree-0 nodes in id order, so the result is deterministic; returns
    /// `None` when a cycle leaves some node unprocessed.
    fn toposort_nodes(&self, nodes: &BTreeSet<&'a str>) -> Option<Vec<&'a str>> {
        let mut indegree: BTreeMap<&str, usize> = nodes.iter().map(|n| (*n, 0)).collect();
        let mut dependents: BTreeMap<&str, Vec<&'a str>> =
            nodes.iter().map(|n| (*n, Vec::new())).collect();
        for &v in nodes {
            for b in self.blockers_within(v, nodes) {
                *indegree.get_mut(v).unwrap() += 1;
                dependents.get_mut(b).unwrap().push(v);
            }
        }

        // A `BTreeSet` frontier makes the pop order deterministic (smallest id).
        let mut ready: BTreeSet<&str> = indegree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(&n, _)| n)
            .collect();
        let mut order = Vec::with_capacity(nodes.len());
        while let Some(&n) = ready.iter().next() {
            ready.remove(n);
            order.push(n);
            for &d in &dependents[n] {
                let e = indegree.get_mut(d).unwrap();
                *e -= 1;
                if *e == 0 {
                    ready.insert(d);
                }
            }
        }
        (order.len() == nodes.len()).then_some(order)
    }

    /// The unique known blockers of `v` that lie within `nodes` (self-edges and
    /// unknown/outside ids dropped). Sorted, so downstream tie-breaks are stable.
    fn blockers_within(&self, v: &str, nodes: &BTreeSet<&'a str>) -> BTreeSet<&'a str> {
        self.get(v)
            .into_iter()
            .flat_map(|t| t.blocked_by.iter())
            .filter_map(|b| nodes.get(b.as_str()).copied())
            .filter(|&b| b != v)
            .collect()
    }

    /// The critical path: the longest `effort_days`-weighted chain of dependent
    /// tasks. `None` if the graph is cyclic (a longest path is then undefined;
    /// `check` reports the cycle as `APLAN004`). Dropped tasks are excluded, they
    /// are not going to happen. Milestones stay in the graph as zero-weight
    /// passthrough nodes, so a task that depends on a whole cut still inherits the
    /// cut's work, but a milestone is never the endpoint and is filtered out of
    /// the reported spine: a checkpoint is not itself work.
    pub fn critical_path(&self) -> Option<CriticalPath> {
        let nodes: BTreeSet<&str> = self
            .by_id
            .iter()
            .filter(|(_, t)| t.status != Status::Dropped)
            .map(|(id, _)| *id)
            .collect();
        self.critical_path_within(&nodes)
    }

    /// The critical path over an explicit node set: used by [`critical_path`] for
    /// the whole plan, and by the roadmap for a single cut's members. Edges to
    /// tasks outside `nodes` are ignored (so a cut's path counts only its own
    /// work). Milestones in `nodes` are zero-weight passthroughs, never the
    /// endpoint, and filtered from the reported spine. `None` if the induced
    /// subgraph is cyclic.
    pub(crate) fn critical_path_within(&self, nodes: &BTreeSet<&'a str>) -> Option<CriticalPath> {
        let order = self.toposort_nodes(nodes)?;

        // Longest-path DP in topological order: each task's finish is its own
        // effort plus the greatest finish among its blockers.
        let mut finish: BTreeMap<&str, f64> = BTreeMap::new();
        let mut pred: BTreeMap<&str, Option<&str>> = BTreeMap::new();
        for &v in &order {
            let effort = self.get(v).and_then(|t| t.effort_days).unwrap_or(0.0);
            let mut best: Option<&str> = None;
            let mut best_finish = 0.0_f64;
            for b in self.blockers_within(v, nodes) {
                let f = finish[b];
                if best.is_none() || f > best_finish {
                    best = Some(b);
                    best_finish = f;
                }
            }
            finish.insert(v, effort + best_finish);
            pred.insert(v, best);
        }

        // The endpoint is the non-milestone task with the greatest finish
        // (smallest id on a tie, since `nodes` iterates in id order). A milestone
        // is a checkpoint, never the spine's endpoint; its finish never exceeds
        // its heaviest member's, so skipping it drops no chain.
        let mut end: Option<&str> = None;
        let mut end_finish = 0.0_f64;
        for &v in nodes {
            if self.get(v).is_some_and(Task::is_milestone) {
                continue;
            }
            let f = finish[v];
            if end.is_none() || f > end_finish {
                end = Some(v);
                end_finish = f;
            }
        }

        // Walk predecessors back to the chain's start, then reverse into
        // execution order.
        let mut ids = Vec::new();
        let mut cur = end;
        while let Some(v) = cur {
            ids.push(v);
            cur = pred[v];
        }
        ids.reverse();

        let tasks: Vec<CriticalTask> = ids
            .iter()
            .filter_map(|id| self.get(id))
            .filter(|t| !t.is_milestone())
            .map(|t| CriticalTask {
                id: t.id.clone(),
                title: t.title.clone(),
                effort_days: t.effort_days.unwrap_or(0.0),
                status: t.status,
            })
            .collect();
        let total_effort_days = tasks.iter().map(|t| t.effort_days).sum();
        Some(CriticalPath {
            total_effort_days,
            tasks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{Status, Task};

    fn task(id: &str, status: Status, blocked_by: &[&str]) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            phase: 0,
            status,
            owner: None,
            effort_days: None,
            blocked_by: blocked_by.iter().map(|s| (*s).to_string()).collect(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn detects_direct_cycle() {
        let a = task("A", Status::Todo, &["B"]);
        let b = task("B", Status::Todo, &["A"]);
        let g = Graph::new([&a, &b]);
        let cycle = g.find_cycle().expect("cycle exists");
        assert!(cycle.len() >= 2, "cycle: {cycle:?}");
    }

    #[test]
    fn self_loop_is_not_reported_as_a_cycle() {
        // A self-dependency is `SELF_DEP` at the validator, not a graph cycle;
        // `find_cycle` ignores self-edges so the two aren't double-reported.
        let a = task("A", Status::Todo, &["A"]);
        let g = Graph::new([&a]);
        assert_eq!(g.find_cycle(), None);
    }

    #[test]
    fn direct_unknown_blockers_finds_ghosts() {
        let a = task("A", Status::Todo, &["GHOST", "B"]);
        let b = task("B", Status::Todo, &[]);
        let g = Graph::new([&a, &b]);
        assert_eq!(g.direct_unknown_blockers("A"), vec!["GHOST"]);
        assert!(g.direct_unknown_blockers("B").is_empty());
    }

    #[test]
    fn acyclic_chain_has_no_cycle() {
        let a = task("A", Status::Done, &[]);
        let b = task("B", Status::Todo, &["A"]);
        let c = task("C", Status::Todo, &["B"]);
        let g = Graph::new([&a, &b, &c]);
        assert_eq!(g.find_cycle(), None);
    }

    #[test]
    fn ready_excludes_blocked_and_terminal() {
        let a = task("A", Status::Done, &[]);
        let b = task("B", Status::Todo, &["A"]); // ready: blocker done
        let c = task("C", Status::Todo, &["B"]); // not ready: B not done
        let g = Graph::new([&a, &b, &c]);
        let ready: Vec<_> = g.ready().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ready, vec!["B"]);
    }

    #[test]
    fn ready_excludes_milestones() {
        // A milestone whose members are all done would pass `is_ready` (workable,
        // blockers done), but it is a checkpoint, not work, so it is never
        // offered as ready. A normal task in the same state still is.
        let a = task("A", Status::Done, &[]);
        let work = task("W", Status::Todo, &["A"]);
        let mut milestone = task("M", Status::Todo, &["A"]);
        milestone.tags = vec!["milestone".into()];
        let g = Graph::new([&a, &work, &milestone]);
        let ready: Vec<_> = g.ready().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ready, vec!["W"]);
    }

    #[test]
    fn dropped_blocker_does_not_satisfy() {
        let a = task("A", Status::Dropped, &[]);
        let b = task("B", Status::Todo, &["A"]);
        let g = Graph::new([&a, &b]);
        assert!(g.ready().is_empty(), "dropped blocker must not unblock");
    }

    #[test]
    fn why_blocked_is_transitive() {
        let a = task("A", Status::Todo, &[]);
        let b = task("B", Status::Todo, &["A"]);
        let c = task("C", Status::Todo, &["B"]);
        let g = Graph::new([&a, &b, &c]);
        let why: Vec<_> = g.why_blocked("C").iter().map(|t| t.id.as_str()).collect();
        assert_eq!(why, vec!["B", "A"]);
    }

    fn task_e(id: &str, blocked_by: &[&str], effort: f64) -> Task {
        let mut t = task(id, Status::Todo, blocked_by);
        t.effort_days = Some(effort);
        t
    }

    #[test]
    fn toposort_orders_blockers_before_dependents_deterministically() {
        let a = task("A", Status::Todo, &[]);
        let b = task("B", Status::Todo, &["A"]);
        let c = task("C", Status::Todo, &["B"]);
        // Insertion order shuffled; the sort is by dependency, tie-broken by id.
        assert_eq!(
            Graph::new([&c, &a, &b]).toposort(),
            Some(vec!["A", "B", "C"])
        );
    }

    #[test]
    fn toposort_is_none_on_a_cycle() {
        let a = task("A", Status::Todo, &["B"]);
        let b = task("B", Status::Todo, &["A"]);
        assert_eq!(Graph::new([&a, &b]).toposort(), None);
    }

    #[test]
    fn critical_path_is_the_longest_weighted_chain() {
        // Two routes to D: A->B->D = 1+1+1 = 3, and A->C->D = 1+5+1 = 7. The
        // critical path is the heavier one, in execution order.
        let a = task_e("A", &[], 1.0);
        let b = task_e("B", &["A"], 1.0);
        let c = task_e("C", &["A"], 5.0);
        let d = task_e("D", &["B", "C"], 1.0);
        let cp = Graph::new([&a, &b, &c, &d])
            .critical_path()
            .expect("acyclic");
        let ids: Vec<&str> = cp.tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["A", "C", "D"]);
        assert!((cp.total_effort_days - 7.0).abs() < 1e-9);
    }

    #[test]
    fn critical_path_excludes_dropped_tasks() {
        // A dropped task with huge effort is not going to happen, so it is not on
        // the plan's spine.
        let a = task_e("A", &[], 2.0);
        let mut dropped = task_e("Z", &[], 100.0);
        dropped.status = Status::Dropped;
        let cp = Graph::new([&a, &dropped]).critical_path().unwrap();
        assert_eq!(
            cp.tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["A"]
        );
        assert!((cp.total_effort_days - 2.0).abs() < 1e-9);
    }

    #[test]
    fn critical_path_excludes_milestones() {
        // A milestone is `blocked_by` its members, so it sits at the end of their
        // chain, but it is a zero-effort checkpoint and must not be reported as
        // the spine's endpoint.
        let a = task_e("A", &[], 2.0);
        let b = task_e("B", &["A"], 3.0);
        let mut milestone = task("M", Status::Todo, &["A", "B"]);
        milestone.tags = vec!["milestone".into()];
        let cp = Graph::new([&a, &b, &milestone]).critical_path().unwrap();
        assert_eq!(
            cp.tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["A", "B"]
        );
        assert!((cp.total_effort_days - 5.0).abs() < 1e-9);
    }

    #[test]
    fn critical_path_treats_a_milestone_as_a_zero_weight_passthrough() {
        // Regression: a task can depend on a whole cut (`blocked_by` a milestone).
        // The milestone weighs nothing but must not sever the chain, X's
        // dependency on the cut's work still counts. Dropping the milestone node
        // entirely (an earlier bug) reported just A (3.0d) and lost that A gates X.
        let a = task_e("A", &[], 3.0);
        let mut milestone = task("M", Status::Todo, &["A"]);
        milestone.tags = vec!["milestone".into()];
        let x = task_e("X", &["M"], 1.0);
        let cp = Graph::new([&a, &milestone, &x]).critical_path().unwrap();
        assert_eq!(
            cp.tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["A", "X"],
            "the milestone is hidden but its member A still gates X"
        );
        assert!((cp.total_effort_days - 4.0).abs() < 1e-9);
    }

    #[test]
    fn critical_path_is_none_on_a_cycle() {
        let a = task("A", Status::Todo, &["B"]);
        let b = task("B", Status::Todo, &["A"]);
        assert!(Graph::new([&a, &b]).critical_path().is_none());
    }

    #[test]
    fn critical_path_of_an_empty_plan_is_empty() {
        let cp = Graph::new([]).critical_path().unwrap();
        assert!(cp.tasks.is_empty());
        assert!((cp.total_effort_days - 0.0).abs() < 1e-9);
    }
}
