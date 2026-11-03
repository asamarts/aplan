//! Reachability: the query this tool owes, over the graph it already holds
//! (`A1-221`).
//!
//! Two questions arrived together -- should a view be scoped by a query, and
//! should this tool have a query engine -- and they have different answers.
//!
//! **What a downstream tool genuinely cannot do is reachability.** Every
//! predicate over a *row* is a `jq` line once `aplan tasks` carries the field:
//! status, owner, tag, cut, effort. What stays out of reach is the transitive
//! shape -- what closing this unblocks, what lies between these two, what no
//! milestone can reach -- because that is a traversal over a graph the consumer
//! would have to rebuild from `blocked_by` first. This tool already holds it,
//! and `ready`, `why` and `critical_path` are each one hard-coded walk of it.
//!
//! **A query language is what this refuses.** Four traversals, each a closure
//! over edges, each with an obvious name. A fifth is the beginning of a grammar,
//! and `docs/adr/0026-reachability-not-a-query-language.md` records why that
//! line is where it is.
//!
//! **Which graph is part of the question.** Reachability over membership edges
//! answers "what is in this cut", which `--cut` already does; over dependency
//! edges it answers what a reader means by "what does this unblock" (`A1-216`).
//! A traversal says which it walked, because the two are different graphs and a
//! reader who is not told will assume the wrong one.

use std::collections::{BTreeSet, VecDeque};

use crate::graph::{EdgeKind, Graph};

/// Which edges a traversal follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edges {
    /// One task must finish before another can start. What a reader means.
    Dependency,
    /// A cut listing its members, which is grouping (ADR-0008).
    Membership,
    /// Both, which is the whole `blocked_by` relation.
    Any,
}

impl Edges {
    fn admits(self, kind: EdgeKind) -> bool {
        match self {
            Edges::Any => true,
            Edges::Dependency => kind == EdgeKind::Dependency,
            Edges::Membership => kind == EdgeKind::Membership,
        }
    }

    /// How a view says which graph it walked.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Edges::Dependency => "dependency edges",
            Edges::Membership => "cut-membership edges",
            Edges::Any => "every `blocked_by` edge",
        }
    }
}

/// One reachability question.
///
/// Four, deliberately. Each came from a real question, each is a closure over
/// edges, and a fifth is where a filter grammar starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// Everything this task waits on, transitively. "What is in my way."
    Upstream(String),
    /// Everything that waits on this task, transitively. "What does closing
    /// this unblock."
    Downstream(String),
    /// Everything on a path from the first to the second. "What lies between."
    Between(String, String),
    /// Everything no milestone can reach: work in the plan and in no cut.
    Unreachable,
}

impl Reach {
    /// The ids this question selects, or `None` when it names a task the plan
    /// does not define.
    ///
    /// The named task is included in its own answer for `upstream` and
    /// `downstream`: a reader asking what `X` waits on is looking at a view
    /// about `X`, and a view that omits its own subject reads as a mistake.
    #[must_use]
    pub fn select<'a>(&self, graph: &Graph<'a>, edges: Edges) -> Option<BTreeSet<&'a str>> {
        match self {
            Reach::Upstream(id) => closure(graph, id, edges, Up),
            Reach::Downstream(id) => closure(graph, id, edges, Down),
            Reach::Between(from, to) => {
                let after = closure(graph, from, edges, Down)?;
                let before = closure(graph, to, edges, Up)?;
                Some(after.intersection(&before).copied().collect())
            }
            // Deliberately not `edges`. "Reachable from no milestone" is a
            // question about *containment*, and containment is the membership
            // relation: over dependency edges a milestone reaches only other
            // milestones, so the answer would be "almost every task", presented
            // as though it meant something. On this plan that was 238 of 248
            // (`A1-221`).
            Reach::Unreachable => Some(unreachable_from_any_milestone(graph)),
        }
    }

    /// The edges this question actually walks, which is not always the edges
    /// the caller asked for.
    ///
    /// `unreachable` is about containment and walks membership whatever it is
    /// given, so a view that reported the caller's choice would say the wrong
    /// thing (`A1-221`).
    #[must_use]
    pub const fn walks(&self, asked: Edges) -> Edges {
        match self {
            Reach::Unreachable => Edges::Membership,
            _ => asked,
        }
    }

    /// The ids this question names, so a caller can refuse an unknown one with
    /// its own wording.
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        match self {
            Reach::Upstream(id) | Reach::Downstream(id) => vec![id.as_str()],
            Reach::Between(a, b) => vec![a.as_str(), b.as_str()],
            Reach::Unreachable => Vec::new(),
        }
    }
}

/// Which way a closure walks.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Up;
#[derive(Clone, Copy, PartialEq, Eq)]
struct Down;

trait Walk {
    fn next<'a>(graph: &Graph<'a>, id: &str, edges: Edges) -> Vec<&'a str>;
}

impl Walk for Up {
    /// What `id` waits on: the tails of the edges pointing at it.
    fn next<'a>(graph: &Graph<'a>, id: &str, edges: Edges) -> Vec<&'a str> {
        let Some(task) = graph.get(id) else {
            return Vec::new();
        };
        graph
            .blockers_of(id)
            .into_iter()
            .filter(|b| {
                graph
                    .get(b)
                    .is_some_and(|tail| edges.admits(crate::edge_kind(tail, task)))
            })
            .collect()
    }
}

impl Walk for Down {
    /// What waits on `id`: the heads of the edges leaving it.
    fn next<'a>(graph: &Graph<'a>, id: &str, edges: Edges) -> Vec<&'a str> {
        let Some(tail) = graph.get(id) else {
            return Vec::new();
        };
        graph
            .dependents_of(id)
            .into_iter()
            .filter(|head| edges.admits(crate::edge_kind(tail, head)))
            // Back to the key the graph owns, so the closure holds one lifetime
            // rather than borrowing from the task. `filter_map` and not an empty
            // sentinel: a dependent the index cannot resolve should disappear
            // from the walk, not become a nameless entry that a later filter
            // happens to drop.
            .filter_map(|head| graph.by_id.get_key_value(head.id.as_str()).map(|(k, _)| *k))
            .collect()
    }
}

/// Breadth-first, iterative: a plan whose tasks are strictly sequenced is a
/// chain, and a recursive closure over one is a stack overflow rather than an
/// answer (the lesson `tree`'s walk already carries).
fn closure<'a, W: Walk>(
    graph: &Graph<'a>,
    from: &str,
    edges: Edges,
    _direction: W,
) -> Option<BTreeSet<&'a str>> {
    let (key, _) = graph.by_id.get_key_value(from)?;
    let mut seen: BTreeSet<&'a str> = BTreeSet::new();
    seen.insert(*key);
    let mut queue: VecDeque<&'a str> = VecDeque::new();
    queue.push_back(*key);

    while let Some(id) = queue.pop_front() {
        for next in W::next(graph, id, edges) {
            if seen.insert(next) {
                queue.push_back(next);
            }
        }
    }
    Some(seen)
}

/// Every task no milestone can reach.
///
/// Work that is in the plan and in no cut, which `APLAN022` reports one task at
/// a time and which nothing could ask for as a set. Milestones themselves are
/// not "unreachable" for being uncontained: a top-level cut is reachable from
/// itself, so the walk seeds from every milestone and asks who is left.
fn unreachable_from_any_milestone<'a>(graph: &Graph<'a>) -> BTreeSet<&'a str> {
    let mut reached: BTreeSet<&'a str> = BTreeSet::new();
    let mut queue: VecDeque<&'a str> = VecDeque::new();
    for (id, task) in &graph.by_id {
        if task.is_milestone() && reached.insert(*id) {
            queue.push_back(*id);
        }
    }
    while let Some(id) = queue.pop_front() {
        for next in Up::next(graph, id, Edges::Membership) {
            if reached.insert(next) {
                queue.push_back(next);
            }
        }
    }
    graph
        .by_id
        .keys()
        .copied()
        .filter(|id| !reached.contains(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{Status, Task};

    fn task(id: &str, blocked_by: &[&str], milestone: bool) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            phase: 0,
            status: Status::Todo,
            owner: None,
            effort_days: Some(1.0),
            blocked_by: blocked_by.iter().map(|s| (*s).to_string()).collect(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            evidence_fingerprints: std::collections::BTreeMap::new(),
            tags: if milestone {
                vec!["milestone".into()]
            } else {
                Vec::new()
            },
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    /// R-1 -> R-2 -> R-3, R-2 -> R-4; cut `M-a` lists R-1 and R-2, cut `M-b`
    /// lists R-3. R-4 and R-9 are in no cut, which is what `unreachable` finds.
    fn plan() -> Vec<Task> {
        vec![
            task("R-1", &[], false),
            task("R-2", &["R-1"], false),
            task("R-3", &["R-2"], false),
            task("R-4", &["R-2"], false),
            task("R-9", &[], false),
            task("M-a", &["R-1", "R-2"], true),
            task("M-b", &["R-3"], true),
        ]
    }

    fn selected(reach: &Reach, edges: Edges) -> Vec<String> {
        let tasks = plan();
        let graph = Graph::new(tasks.iter());
        let mut out: Vec<String> = reach
            .select(&graph, edges)
            .expect("the ids exist")
            .into_iter()
            .map(str::to_string)
            .collect();
        out.sort();
        out
    }

    /// **Which graph, tested where nothing can mask it.** Through `graph` the
    /// view is *also* narrowed to dependency edges (`A1-216`), so a traversal
    /// that ignored `Edges` entirely still produced the right picture and the
    /// end-to-end test passed. Here the traversal is the only thing running.
    #[test]
    fn the_edge_kind_decides_what_a_traversal_reaches() {
        // Over dependencies, R-1 unblocks the chain and reaches no cut.
        assert_eq!(
            selected(&Reach::Downstream("R-1".into()), Edges::Dependency),
            ["R-1", "R-2", "R-3", "R-4"]
        );
        // Over grouping alone, R-1 reaches only the cut that lists it.
        assert_eq!(
            selected(&Reach::Downstream("R-1".into()), Edges::Membership),
            ["M-a", "R-1"]
        );
        // Over everything: the chain, and both cuts it passes through.
        assert_eq!(
            selected(&Reach::Downstream("R-1".into()), Edges::Any),
            ["M-a", "M-b", "R-1", "R-2", "R-3", "R-4"]
        );
    }

    #[test]
    fn upstream_is_transitive_and_includes_its_subject() {
        assert_eq!(
            selected(&Reach::Upstream("R-3".into()), Edges::Dependency),
            ["R-1", "R-2", "R-3"]
        );
    }

    /// The intersection of one closure with the other, which is not the union:
    /// R-4 hangs off R-2 and lies on no path to R-3.
    #[test]
    fn between_is_an_intersection() {
        assert_eq!(
            selected(
                &Reach::Between("R-1".into(), "R-3".into()),
                Edges::Dependency
            ),
            ["R-1", "R-2", "R-3"]
        );
    }

    /// Work in the plan that no cut holds, whatever edges the caller asked for.
    ///
    /// "Reachable from no milestone" is a question about *containment*, and
    /// containment is the membership relation. Asked over dependency edges a
    /// milestone reaches only other milestones, so the answer would be "almost
    /// every task", presented as though it meant something: on this repository's
    /// own plan that was 238 of 248.
    ///
    /// R-4 is the interesting one: it is downstream of R-2, which *is* in a cut,
    /// and it is still in none itself. Being connected to contained work is not
    /// being contained, which is exactly the gap `APLAN022` reports one task at
    /// a time and which nothing could ask for as a set.
    #[test]
    fn unreachable_is_what_no_milestone_holds() {
        // The same answer whatever it is asked over, because the question is
        // not about the caller's choice of edges.
        for asked in [Edges::Membership, Edges::Dependency, Edges::Any] {
            assert_eq!(
                selected(&Reach::Unreachable, asked),
                ["R-4", "R-9"],
                "asked over {asked:?}"
            );
            assert_eq!(
                Reach::Unreachable.walks(asked),
                Edges::Membership,
                "and it says so"
            );
        }
        // A cut is not "unreachable" for being top-level: it reaches itself.
        for cut in ["M-a", "M-b"] {
            assert!(!selected(&Reach::Unreachable, Edges::Membership).contains(&cut.to_string()));
        }
    }

    /// A traversal naming a task the plan does not define answers `None`, so a
    /// caller can refuse it in its own words rather than being handed an empty
    /// set that looks like a real answer.
    #[test]
    fn an_unknown_id_is_not_an_empty_answer() {
        let tasks = plan();
        let graph = Graph::new(tasks.iter());
        assert!(
            Reach::Upstream("NOPE".into())
                .select(&graph, Edges::Any)
                .is_none()
        );
        assert!(
            Reach::Between("R-1".into(), "NOPE".into())
                .select(&graph, Edges::Any)
                .is_none()
        );
        // And `unreachable` names nothing, so it always answers.
        assert!(Reach::Unreachable.select(&graph, Edges::Any).is_some());
    }

    /// A cycle is `APLAN004`, which `check` reports and a traversal does not
    /// refuse: the closure terminates rather than looping.
    #[test]
    fn a_cycle_terminates() {
        let tasks = [task("C-1", &["C-2"], false), task("C-2", &["C-1"], false)];
        let graph = Graph::new(tasks.iter());
        let out = Reach::Upstream("C-1".into())
            .select(&graph, Edges::Any)
            .expect("C-1 exists");
        assert_eq!(out.len(), 2);
    }
}
