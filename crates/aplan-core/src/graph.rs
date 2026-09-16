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

/// How far [`Graph::why_blocked`] walks.
///
/// The complete closure answers a different question from the one a person
/// asking *why is this blocked* has. Both are true; only one is actionable.
/// `p0000014` asked about one task and got 86 lines, of which three could be
/// acted on (`A1-263`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Walk {
    /// Report a cut and stop there, rather than walking into its membership.
    CutBounded,
    /// Every unmet blocker, transitively. What an audit wants.
    Complete,
}

/// What a `blocked_by` edge *means* (`A1-216`).
///
/// ADR-0008 models a cut as a meta-task that is `blocked_by` its members, which
/// is a good decision for the model and makes one field carry two different
/// facts. Measured over this project's own plan, three quarters of its edges
/// were grouping rather than constraint, and drawing them as if they were the
/// same thing produced a graph nine layers deep and 177 nodes wide.
///
/// **Derived here, once.** The distinction is not in the model and must not
/// become one -- ADR-0008 is not in question. It is a property of the pair, so
/// it lives in one place and every consumer asks rather than re-deriving it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// One task must finish before another can start. What a reader means by a
    /// dependency graph.
    Dependency,
    /// A cut lists one of its members. The same fact `roadmap` prints as a
    /// heading, and the reason `graph` was unreadable.
    Membership,
}

/// Which of the two an edge is.
///
/// A cut listing a member is membership. Everything else is a dependency,
/// including a cut sequenced after another cut (ADR-0013, both ends
/// milestones) and a task waiting on a whole cut to close -- both of which are
/// constraints a reader wants drawn.
#[must_use]
pub fn edge_kind(from: &Task, to: &Task) -> EdgeKind {
    if to.is_milestone() && !from.is_milestone() {
        EdgeKind::Membership
    } else {
        EdgeKind::Dependency
    }
}

/// Why a ready task is where it is in the answer (`A1-125`, ADR-0017).
///
/// Every field is *derived* from what the plan already records, so the order
/// carries no claim the tool could not check. A `priority:` field would be an
/// opinion, and this project's thesis is that an unverifiable claim in a plan is
/// the thing it exists to distrust.
///
/// **Two of the four inputs ADR-0017 listed are not here, and both were measured
/// before they were dropped** (`A1-125`):
///
/// - **Cut sequence** cannot discriminate between two *ready* tasks. Where cuts
///   are sequenced (ADR-0013), every member of a later cut inherits the earlier
///   milestone as a blocker, so it is not ready at all until that cut closes; and
///   when a cut *is* closed, none of its members are workable. Two ready tasks can
///   therefore only be in cuts that are unsequenced with respect to each other,
///   where the plan states no order. On this repository's plan the input is empty
///   twice over: no cut is sequenced, and the only pair that would have
///   discriminated (`M-v0.8` after `M-v0.7`) is *refused*, correctly, with a cycle
///   and sixty `APLAN007`s, because v0.8 holds sixty finished tasks.
/// - **Effort as a tiebreak between equals** stays, but only *after* the two
///   inputs that are about the plan rather than the task. See [`Rank::key`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rank {
    /// Unfinished tasks that become ready when this one is done.
    pub releases: usize,
    /// The longest chain of live work waiting behind this one, over declared
    /// task-to-task edges (`A1-298`).
    pub downstream_depth: usize,
    /// Whether it lies on the critical path, the plan's spine.
    pub critical: bool,
    /// Its own `effort_days`, `0.0` when unestimated.
    pub effort_days: f64,
}

impl Rank {
    /// The precedence, recorded here because it is the decision rather than an
    /// implementation detail.
    ///
    /// 1. **What it releases**, descending. The only input that changes what the
    ///    *plan* can do next rather than describing this task alone, and the only
    ///    one derived from mandatory logic rather than from a heuristic.
    /// 2. **On the critical path**, before off it. The spine is where slack is
    ///    zero by construction, so a day lost here is a day lost on the plan.
    /// 3. **The longest chain waiting behind it**, descending (`A1-298`). It
    ///    exists because the two terms above are silent on almost everything:
    ///    measured across twelve plans, 88% of 213 ready tasks released nothing
    ///    and sat off the spine, and counting the chain behind them takes that
    ///    to 67%.
    ///
    ///    **Placed after the spine on purpose**, which was decided by asking what
    ///    it would override. Ahead of it, any task with one thing behind it
    ///    outranks a critical-path task with none, and the sentence above about
    ///    slack stops being true. Behind it, this only ever breaks a tie that
    ///    previously fell through to effort -- so it changes no ordering ADR-0017
    ///    argued for, and the discrimination it buys is identical either way,
    ///    since the bucket a task lands in does not depend on the order the terms
    ///    are compared in.
    /// 4. **Effort, descending.** A cut cannot finish sooner than its largest
    ///    remaining task, so the biggest piece is the one most likely to become
    ///    what everything waits for. The opposite rule (quick wins first) is
    ///    coherent and was rejected: it optimises the count of finished tasks,
    ///    which is not what "what should I do next" asks. An unestimated task
    ///    counts as `0.0` and sorts last among its peers rather than being given
    ///    the benefit of the doubt; `APLAN009` is the rule that asks for the
    ///    estimate.
    /// 5. **Id**, ascending, applied by the caller. Not a ranking, just something
    ///    stable, so equal work does not reorder between runs.
    #[must_use]
    pub fn key(
        &self,
    ) -> (
        std::cmp::Reverse<usize>,
        std::cmp::Reverse<bool>,
        std::cmp::Reverse<usize>,
        OrderedEffort,
    ) {
        (
            std::cmp::Reverse(self.releases),
            std::cmp::Reverse(self.critical),
            std::cmp::Reverse(self.downstream_depth),
            OrderedEffort(self.effort_days),
        )
    }

    /// The rank in a clause, for a reader who wants to know why this is first.
    #[must_use]
    pub fn because(&self) -> String {
        let mut parts = Vec::new();
        if self.releases > 0 {
            parts.push(format!("releases {}", self.releases));
        }
        if self.critical {
            parts.push("on the critical path".to_string());
        }
        if self.downstream_depth > 0 {
            parts.push(format!("a chain of {} behind it", self.downstream_depth));
        }
        if self.effort_days > 0.0 {
            parts.push(format!("{}d", self.effort_days));
        }
        if parts.is_empty() {
            // Said rather than left blank: a task that releases nothing, sits off
            // the spine and carries no estimate is ordered by its id, and the
            // reader should know that is all the plan supports.
            return "nothing to rank it by; ordered by id".to_string();
        }
        parts.join(", ")
    }
}

/// `effort_days` as a descending sort key, since `f64` is not `Ord`.
///
/// `total_cmp` rather than a partial comparison, so this is a total order for
/// every `f64` and the sort cannot be inconsistent. The loader already refuses
/// `NaN` and a negative estimate as `APLAN001`, so neither reaches here; this is
/// what makes the `Ord` impl honest rather than a case it has to handle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrderedEffort(pub f64);

impl Eq for OrderedEffort {}

impl Ord for OrderedEffort {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.0.total_cmp(&self.0)
    }
}

impl PartialOrd for OrderedEffort {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// An index over tasks by id. `BTreeMap` so iteration order is deterministic,
/// diagnostics must not reorder between runs.
#[derive(Debug, Default)]
pub struct Graph<'a> {
    pub by_id: BTreeMap<&'a str, &'a Task>,
    /// Blockers a task inherits from the cut it belongs to (`A1-073`).
    sequenced: BTreeMap<&'a str, Vec<&'a str>>,
}

impl<'a> Graph<'a> {
    pub fn new(tasks: impl IntoIterator<Item = &'a Task>) -> Self {
        let mut by_id = BTreeMap::new();
        for t in tasks {
            // First writer wins; duplicate ids are reported separately by the
            // validator, and clobbering here would hide the second one.
            by_id.entry(t.id.as_str()).or_insert(t);
        }
        let sequenced = Self::sequenced_edges(&by_id);
        Self { by_id, sequenced }
    }

    /// Cut sequencing, derived rather than hand-maintained (`A1-073`).
    ///
    /// A milestone listing another milestone in `blocked_by` says its cut
    /// follows that one. That edge is already legal and already excluded from
    /// cut membership, so it carries no other meaning today; what it did not do
    /// was reach the *members*, and members are what `ready` offers and what
    /// `critical_path` measures. So a plan whose cuts are strictly sequential had
    /// to spell the relationship out on every task: the fourth pilot needed 81
    /// such edges, every one derivable from the cut the task was already in, and
    /// without them `ready` offered work from three cuts ahead while
    /// `critical_path` reported a 105-day plan as eleven days.
    ///
    /// Opt-in by construction: a plan whose milestones do not block one another,
    /// including this repository's own, is unchanged.
    fn sequenced_edges(by_id: &BTreeMap<&'a str, &'a Task>) -> BTreeMap<&'a str, Vec<&'a str>> {
        // The two halves of a cut's `blocked_by` are the two edge kinds, asked
        // for by name rather than re-derived: what precedes the cut is a
        // constraint, what it lists is grouping (`A1-216`).
        let kind = |b: &str, m: &'a Task| {
            by_id.get(b).map_or(crate::EdgeKind::Dependency, |tail| {
                crate::edge_kind(tail, m)
            })
        };
        let mut out: BTreeMap<&'a str, Vec<&'a str>> = BTreeMap::new();

        for m in by_id.values().filter(|m| m.is_milestone()) {
            let predecessors: Vec<&'a str> = m
                .blocked_by
                .iter()
                .map(String::as_str)
                .filter(|b| by_id.contains_key(b) && kind(b, m) == crate::EdgeKind::Dependency)
                .filter_map(|b| by_id.get_key_value(b).map(|(k, _)| *k))
                .collect();
            if predecessors.is_empty() {
                continue;
            }
            for member in m
                .blocked_by
                .iter()
                .map(String::as_str)
                .filter(|b| by_id.contains_key(b) && kind(b, m) == crate::EdgeKind::Membership)
                .filter_map(|b| by_id.get_key_value(b).map(|(k, _)| *k))
            {
                out.entry(member).or_default().extend(&predecessors);
            }
        }
        for v in out.values_mut() {
            v.sort_unstable();
            v.dedup();
        }
        out
    }

    /// Every blocker of `id`: those it declares, plus those its cut implies.
    ///
    /// Sorted and deduplicated, so a task that both declares and inherits an
    /// edge is blocked by it once and the order never depends on how a plan
    /// happened to be written.
    pub fn blockers_of(&self, id: &str) -> Vec<&'a str> {
        let mut all: BTreeSet<&'a str> = self
            .get(id)
            .into_iter()
            .flat_map(|t| t.blocked_by.iter().map(String::as_str))
            .filter_map(|b| self.by_id.get_key_value(b).map(|(k, _)| *k))
            .collect();
        // A declared edge to an id that does not exist is `APLAN003`'s finding,
        // and `direct_unknown_blockers` reports it; it must still count as
        // unmet, or a task blocked solely by a ghost would look ready.
        if let Some(t) = self.get(id) {
            for b in &t.blocked_by {
                if !self.by_id.contains_key(b.as_str()) {
                    all.insert(b.as_str());
                }
            }
        }
        if let Some(extra) = self.sequenced.get(id) {
            all.extend(extra.iter().copied());
        }
        all.into_iter().collect()
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
    /// The blockers of `id` that are not `done`.
    ///
    /// Shared rather than reimplemented per surface: `close` must refuse a task
    /// whose blockers are unfinished, and that refusal lives outside
    /// [`crate::close()`] because it needs the whole graph, not one file. A second
    /// copy in another surface is a copy that can be weaker than this one, which
    /// is how an agent ends up able to close through MCP what the CLI refuses.
    pub fn unmet_blockers(&self, id: &str) -> Vec<&'a str> {
        let Some(dependent) = self.get(id) else {
            return Vec::new();
        };
        self.blockers_of(id)
            .into_iter()
            .filter(|b| self.blocks(dependent, b))
            .collect()
    }

    /// Whether `blocker` stops `dependent` from proceeding.
    ///
    /// The one definition. It lived in several places, and when `A1-081` relaxed
    /// it for a cut's cancelled member only the validator was changed: `check`
    /// then called a plan clean while `why` still named the cancelled member as a
    /// blocker and `close` refused to close the cut at all, which is the very
    /// situation that fix existed for.
    ///
    /// An unknown id blocks: it is `APLAN003`'s finding, and a task waiting on a
    /// typo is not ready.
    #[must_use]
    pub fn blocks(&self, dependent: &Task, blocker: &str) -> bool {
        let Some(dep) = self.get(blocker) else {
            return true;
        };
        if dep.status.satisfies_dependency() {
            return false;
        }
        // A cut is a checkpoint over its members, and a member that will never be
        // done must not hold the checkpoint open. A cancelled *predecessor cut*
        // is a different fact and still blocks: a milestone in another
        // milestone's `blocked_by` is a sequencing edge (ADR-0013), and a cut
        // whose stated prerequisite was abandoned is a plan its author must
        // resolve.
        let cancelled_member = crate::edge_kind(dep, dependent) == crate::EdgeKind::Membership
            && dep.status == Status::Dropped;
        !cancelled_member
    }

    /// Milestones whose members are all done: the cuts that can be closed.
    ///
    /// [`Self::ready`] excludes milestones, because a checkpoint is not work you
    /// can pick up (ADR-0008), and that is right. What it produced was a cliff: at
    /// the moment a cut completes, `ready` prints nothing at all, which is
    /// indistinguishable from a finished plan or a broken config, while every task
    /// in the next cut waits on a milestone somebody could close. The fourth pilot
    /// met it on its first cut boundary and had no surface that said so
    /// (`A1-105`, pre-public adoption evidence item 21).
    ///
    /// Kept out of `ready` and offered beside it, so the command's contract still
    /// answers "work you can pick up" and the reader is still told what to do.
    #[must_use]
    pub fn closeable_milestones(&self) -> Vec<&'a Task> {
        self.by_id
            .values()
            .copied()
            .filter(|t| t.is_milestone())
            // A cut with no members is not "all done", it is mis-shaped, which is
            // `APLAN021`'s finding. `all()` over nothing is true, so without this
            // an empty milestone was offered as closeable and the reader was told
            // to record the completion of a cut that scoped no work.
            .filter(|t| !t.blocked_by.is_empty())
            .filter(|t| {
                t.status.is_workable() && self.blockers_of(&t.id).iter().all(|b| !self.blocks(t, b))
            })
            .collect()
    }

    /// Work that can be picked up now, best first (`A1-125`, ADR-0017).
    ///
    /// The order is [`Rank`]'s, and it is derived rather than declared: a plan
    /// cannot claim a priority this tool could not check.
    pub fn ready(&self) -> Vec<&'a Task> {
        self.ready_ranked().into_iter().map(|(t, _)| t).collect()
    }

    /// The same answer with the reason each task is where it is.
    ///
    /// Ordered by [`Rank::key`]: what a task releases, then whether it is on the
    /// spine, then its effort, then its id. The explanation travels with the
    /// order because an order nobody can interrogate is one people learn to
    /// distrust, which is the standard `why` already sets for blocking.
    #[must_use]
    pub fn ready_ranked(&self) -> Vec<(&'a Task, Rank)> {
        let workable: Vec<&'a Task> = self
            .by_id
            .values()
            .copied()
            .filter(|t| !t.is_milestone())
            .filter(|t| {
                t.status.is_workable() && self.blockers_of(&t.id).iter().all(|b| !self.blocks(t, b))
            })
            .collect();

        let spine: BTreeSet<String> = self
            .critical_path()
            // A spine of no days is not a spine. With no estimates in the plan
            // every chain weighs zero, the endpoint is whichever id sorts first,
            // and calling that task "critical" would be precisely the arbitrary
            // answer dressed as a real one that this rank exists to replace
            // (`A1-125`, found while auditing it).
            .filter(|p| p.total_effort_days > 0.0)
            .map(|p| p.tasks.into_iter().map(|t| t.id).collect())
            .unwrap_or_default();

        // Both indexes are built once for the whole answer, not per task: asking
        // either question per ready task is what made `ready` quadratic once
        // already (`A1-125`).
        let releases = self.releases_all();
        let downstream = self.downstream_depth_all();
        let mut out: Vec<(&'a Task, Rank)> = workable
            .into_iter()
            .map(|t| {
                let rank = Rank {
                    releases: releases.get(t.id.as_str()).copied().unwrap_or(0),
                    downstream_depth: downstream.get(t.id.as_str()).copied().unwrap_or(0),
                    critical: spine.contains(&t.id),
                    effort_days: t.effort_days.unwrap_or(0.0),
                };
                (t, rank)
            })
            .collect();
        out.sort_by(|(a, ra), (b, rb)| ra.key().cmp(&rb.key()).then_with(|| a.id.cmp(&b.id)));
        Self::honour_preferences(&mut out);
        out
    }

    /// Reorder an already-ranked list so a task follows the offered tasks it
    /// prefers to follow (`A1-150`, ADR-0025).
    ///
    /// A **stable topological pass over the derived order**, not a second
    /// ranking: the input order is the answer the plan derives, and a preference
    /// only moves a task later, never earlier than something it does not name.
    /// Two tasks with no preference between them keep the order rank gave them,
    /// which is what makes this deterministic and reviewable.
    ///
    /// Preferences naming a task that is *not offered* -- done, dropped, blocked,
    /// or not in this list -- are inert. The field says "offer me after that
    /// one", and a task nobody is being offered cannot be followed.
    ///
    /// A cycle cannot be ordered. `APLAN033` reports it, and the tasks in it keep
    /// their derived order here rather than being dropped from the answer: a
    /// defect in the plan must not make work disappear from "what next".
    fn honour_preferences(ranked: &mut Vec<(&Task, Rank)>) {
        let offered: BTreeSet<&str> = ranked.iter().map(|(t, _)| t.id.as_str()).collect();
        let mut waiting: Vec<(&Task, Rank)> = std::mem::take(ranked);
        let mut placed: BTreeSet<String> = BTreeSet::new();
        while !waiting.is_empty() {
            // The first task in derived order whose preferences are all placed.
            let next = waiting.iter().position(|(t, _)| {
                t.prefer_after
                    .iter()
                    .all(|p| !offered.contains(p.as_str()) || placed.contains(p))
            });
            // None means every remaining task waits on another remaining one: a
            // cycle. Take the first in derived order and carry on, so the answer
            // stays total.
            let index = next.unwrap_or(0);
            let (task, rank) = waiting.remove(index);
            placed.insert(task.id.clone());
            ranked.push((task, rank));
        }
    }

    /// How many unfinished tasks would become ready if `id` were done.
    ///
    /// One lookup into [`Self::releases_all`], which is where the definition
    /// lives, so a single query and the ranking cannot answer differently.
    #[must_use]
    pub fn releases(&self, id: &str) -> usize {
        self.releases_all().get(id).copied().unwrap_or(0)
    }

    /// For every task, how many unfinished tasks it *alone* is holding up.
    ///
    /// A dependent counts only when this is the one blocker still blocking it,
    /// because a task waiting behind two unfinished ones is released by neither
    /// alone.
    ///
    /// **A milestone is never counted as released *work*, and that is the whole
    /// reason it is filtered here.** ADR-0008 makes a cut a checkpoint rather
    /// than work, and `APLAN021` keeps that honest by refusing one that carries
    /// effort or paths, so "unfinished tasks that become ready" must not include
    /// a cut becoming closeable.
    ///
    /// The filter is **asymmetric on purpose**, and the earlier wording here
    /// ("excluded on both sides") described something this code does not do. A
    /// milestone may still be credited as a *blocker*: a task waiting behind a
    /// cut really is released when that cut closes, and `closeable_cuts` reports
    /// exactly that number. What a milestone may not be is the thing released.
    ///
    /// **The reason is definitional, not statistical**, which is worth saying
    /// because the first version of this comment gave the statistical one. It
    /// said counting membership "scores nearly every task in a plan
    /// identically", and that was true of the implementation this replaced,
    /// which credited *every* dependent rather than only the releasing one
    /// (`A1-125`: 27 of 30 ready tasks scored 1). Once a dependent counts only
    /// for its sole blocker, a cut with several unfinished members is nobody's
    /// release, and this filter stops being what prevents it. Measured across ten
    /// plans and 2,326 tasks on 2026-09-09: removing it changes exactly **one**
    /// score (`A1-233`).
    ///
    /// **Built in one pass over the plan on purpose.** Asking this question per
    /// ready task, each answer scanning every task and rebuilding its blocker
    /// set, made `ready` quadratic: on a 2000-task plan it went from 46ms to
    /// 765ms, a regression an adopter with a large plan would have met before
    /// anyone here did (`A1-125`, measured while auditing it).
    #[must_use]
    pub fn releases_all(&self) -> BTreeMap<&'a str, usize> {
        let mut out: BTreeMap<&'a str, usize> = BTreeMap::new();
        for d in self
            .by_id
            .values()
            .copied()
            .filter(|d| !d.is_milestone() && d.status.is_workable())
        {
            let mut blocking = self
                .blockers_of(&d.id)
                .into_iter()
                .filter(|b| self.blocks(d, b));
            // Exactly one, and not the task itself: a task that blocks itself is
            // `APLAN006`'s finding, and it releases nothing by being finished.
            if let (Some(only), None) = (blocking.next(), blocking.next()) {
                if only != d.id {
                    *out.entry(only).or_default() += 1;
                }
            }
        }
        out
    }

    /// For every task, the longest chain of live work waiting behind it
    /// (`A1-298`).
    ///
    /// `releases_all` answers one step and only where this task is a dependent's
    /// *sole* remaining blocker, which is silent on almost every plan: measured
    /// across twelve plans on 2026-09-09, 88% of 213 ready tasks released nothing
    /// and sat off the spine, so the order fell through to effort and then id.
    /// Counting the chain behind them takes that to 67%.
    ///
    /// **Declared task-to-task edges only.** An edge counts when neither end is a
    /// milestone, which excludes both directions of ADR-0008's grouping in one
    /// rule: a task listing its cut is membership, and a cut listing a task is
    /// the same edge read the other way. It also excludes the constraint a member
    /// *inherits* from the cut its own cut follows (ADR-0013) -- that edge is a
    /// milestone's, and counting it would give a predecessor cut the whole of
    /// every later cut, a number that says nothing about any task in it.
    ///
    /// **Depth rather than a count of descendants, and that was measured rather
    /// than assumed.** Counting distinct descendants exactly needs a set per
    /// node, which is `O(V^2)` memory on the chain-shaped plans `A1-229`'s gate
    /// generates. The two were compared on every plan available first: inserted
    /// into the precedence, exact counting and this depth put *identically* many
    /// ready tasks in the modal bucket, plan for plan, 143 of 213 either way. The
    /// cheaper one was not a compromise.
    ///
    /// One pass with memoisation, and iterative for the reason `find_cycle` is:
    /// a plan whose tasks form a long chain would otherwise recurse as deep as it
    /// is long. A cycle is `APLAN004`'s finding; here it simply stops, so this
    /// answers on a plan `check` would refuse.
    #[must_use]
    pub fn downstream_depth_all(&self) -> BTreeMap<&'a str, usize> {
        // Reverse *dependency* edges: blocker -> the live work waiting on it.
        let mut waiting: BTreeMap<&'a str, Vec<&'a str>> = BTreeMap::new();
        for d in self
            .by_id
            .values()
            .copied()
            .filter(|d| !d.is_milestone() && d.status.is_workable())
        {
            for b in &d.blocked_by {
                if let Some((k, tail)) = self.by_id.get_key_value(b.as_str())
                    && !tail.is_milestone()
                    && tail.status.is_workable()
                {
                    waiting.entry(*k).or_default().push(
                        self.by_id
                            .get_key_value(d.id.as_str())
                            .map_or(d.id.as_str(), |(k, _)| *k),
                    );
                }
            }
        }

        let mut depth: BTreeMap<&'a str, usize> = BTreeMap::new();
        let mut open: BTreeSet<&str> = BTreeSet::new();
        for start in self.by_id.keys().copied() {
            if depth.contains_key(start) {
                continue;
            }
            let mut work: Vec<(&'a str, usize)> = vec![(start, 0)];
            while let Some((node, idx)) = work.pop() {
                let kids: &[&'a str] = waiting.get(node).map_or(&[], Vec::as_slice);
                if idx == 0 {
                    open.insert(node);
                }
                if idx < kids.len() {
                    work.push((node, idx + 1));
                    let next = kids[idx];
                    // `open` is the cycle guard: a plan `check` refuses can still
                    // be asked this question, and it must answer rather than spin.
                    if !depth.contains_key(next) && !open.contains(next) {
                        work.push((next, 0));
                    }
                } else {
                    let d = kids
                        .iter()
                        .map(|k| depth.get(k).map_or(0, |v| v + 1))
                        .max()
                        .unwrap_or(0);
                    depth.insert(node, d);
                    open.remove(node);
                }
            }
        }
        depth
    }

    /// The unmet blockers of `id`, nearest first, as far as `walk` goes.
    ///
    /// Returns an empty vec when the task is ready (or unknown).
    pub fn why_blocked(&self, id: &str, walk: Walk) -> Vec<&'a Task> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        let mut queue = VecDeque::new();

        if let Some(t) = self.get(id) {
            for b in self.blockers_of(id) {
                if self.blocks(t, b) {
                    queue.push_back(b);
                }
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
            // A cut is reported and not walked through, under `CutBounded`. The
            // members behind it are true and are not the answer: `p0000014`
            // asked why one task was blocked and got 86 lines, of which the
            // three that could be acted on were two work blockers and the
            // unfinished cut, and the other 83 were that cut's membership
            // (`A1-263`). A milestone is the one node whose *name* is a better
            // answer than its members, because closing it is a real step and
            // the reader can ask for the list.
            if walk == Walk::CutBounded && task.is_milestone() {
                continue;
            }
            for b in self.blockers_of(&task.id) {
                if self.blocks(task, b) {
                    queue.push_back(b);
                }
            }
        }
        out
    }

    /// How many unmet tasks lie behind `id`, transitively.
    ///
    /// The size of the cut's own remaining work, so a reader is told what they
    /// are not being shown rather than left to wonder whether the answer was
    /// truncated.
    ///
    /// **Not a delta, and not additive with the blockers beside it.** A task can
    /// be a direct blocker of the subject *and* sit behind the folded cut, in
    /// which case it is already named and the complete answer grows by less than
    /// this. `p0000014`'s own case is that shape: its two work blockers are in
    /// cuts behind the one it folded. Counted per blocker for the same reason --
    /// two cuts can share a member, and a single total would be true of neither
    /// (`A1-263`, audit).
    #[must_use]
    pub fn folded_behind(&self, id: &str) -> usize {
        self.why_blocked(id, Walk::Complete).len()
    }

    /// The tasks that wait on `id`: [`blockers_of`](Self::blockers_of) inverted.
    ///
    /// One lookup into [`Self::dependents_index`], which is where the definition
    /// lives, so a single query and a whole walk cannot answer differently.
    #[must_use]
    pub fn dependents_of(&self, id: &str) -> Vec<&'a Task> {
        self.dependents_index().remove(id).unwrap_or_default()
    }

    /// Every task's dependents, inverted in one pass.
    ///
    /// Includes the edges a cut implies (ADR-0013) rather than only the declared
    /// ones, so the answer matches what `ready` acts on. Sorted by id, because
    /// every view here is deterministic. Milestones are kept: a cut waiting on
    /// its member is a real edge, and hiding it from a *view* would make the
    /// picture disagree with the graph it is a picture of (`A1-126`).
    ///
    /// **One pass, for the reason `releases_all` is one pass.** Asking per node
    /// while walking a tree, with each answer scanning every task, made `tree`
    /// quadratic: a 4000-task chain took 7.7s. The same defect twice in two
    /// commits is what makes it worth a shared index rather than a note
    /// (`A1-126`, measured while auditing it).
    #[must_use]
    pub fn dependents_index(&self) -> BTreeMap<&'a str, Vec<&'a Task>> {
        let mut out: BTreeMap<&'a str, Vec<&'a Task>> = BTreeMap::new();
        for d in self.by_id.values().copied() {
            for b in self.blockers_of(&d.id) {
                out.entry(b).or_default().push(d);
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

        // **Once per node, not once per edge** (`A1-227`). The walk below pushes
        // `(node, idx + 1)` and pops it, so it re-enters a node once for each of
        // its blockers -- and asking `blockers_of` there rebuilt that node's
        // whole blocker set every time. A cut's `blocked_by` *is* its
        // membership, so a milestone of `M` members was M+1 rebuilds of an
        // M-element `BTreeSet`: `O(M^2)` per cut, and the reason `check` was
        // quadratic in a cut's *width* rather than in the plan's size. Measured
        // at 10,000 tasks in ten cuts, timing this call: 2,234.8ms of a 2,347ms
        // `validate` before, and 13.4ms after.
        //
        // The set is what `blockers_of` returns, in the order it returns it, so
        // the cycle a plan reports does not move.
        let adjacency: BTreeMap<&str, Vec<&'a str>> = self
            .by_id
            .keys()
            .map(|id| (*id, self.blockers_of(id)))
            .collect();

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

                // The inherited cut edges count here too. Without them `check`
                // reported "DAG acyclic" on a plan `critical-path` refused as
                // cyclic, and told the reader to run `check` (`A1-073`).
                let deps = &adjacency[node];
                if idx < deps.len() {
                    work.push((node, idx + 1));
                    let next = deps[idx];
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
        self.blockers_of(v)
            .into_iter()
            .filter_map(|b| nodes.get(b).copied())
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
        // `+ 0.0` normalises negative zero. `Sum` for floats folds from `-0.0`,
        // the identity that preserves signed-zero semantics, so an empty path
        // sums to `-0.0` and a cut with no live work rendered `crit -0.0d`
        // (`A1-074`). Adding `0.0` maps `-0.0` to `0.0` and leaves every other
        // value untouched.
        let total_effort_days = tasks.iter().map(|t| t.effort_days).sum::<f64>() + 0.0;
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
            prefer_after: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
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

    /// Two sequential cuts: `M-s1` follows `M-s0`, declared once on the
    /// milestone rather than on every member (`A1-073`).
    fn sequential_cuts() -> Vec<Task> {
        let mut m0 = task("M-s0", Status::Todo, &["A1", "A2"]);
        m0.tags = vec!["milestone".into(), "s0".into()];
        let mut m1 = task("M-s1", Status::Todo, &["B1", "M-s0"]);
        m1.tags = vec!["milestone".into(), "s1".into()];
        vec![
            task_e("A1", &[], 2.0),
            task_e("A2", &[], 1.0),
            task_e("B1", &[], 3.0),
            m0,
            m1,
        ]
    }

    #[test]
    fn a_cut_that_follows_another_blocks_its_members() {
        // The fourth pilot offered work from three cuts ahead on day one,
        // because a milestone is `blocked_by` its members and nothing said one
        // cut follows another. Declaring it on the milestone now reaches the
        // members, which is what `ready` offers.
        let tasks = sequential_cuts();
        let g = Graph::new(tasks.iter());
        let ready: Vec<&str> = g.ready().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ready, vec!["A1", "A2"], "cut 1 is not ready yet");
        assert!(
            g.unmet_blockers("B1").contains(&"M-s0"),
            "the member inherits its cut's predecessor"
        );
    }

    #[test]
    fn a_sequenced_cut_puts_its_members_on_the_critical_path() {
        // The other half: without the inherited edge the longest chain the tool
        // can see is a fragment, and a 105-day plan reported as eleven.
        let tasks = sequential_cuts();
        let g = Graph::new(tasks.iter());
        let path = g.critical_path().expect("a path");
        // A1 (2d) -> M-s0 (0d, passthrough) -> B1 (3d).
        assert!(
            (path.total_effort_days - 5.0).abs() < f64::EPSILON,
            "{:?}",
            path.tasks
        );
    }

    #[test]
    fn every_surface_agrees_about_a_cut_with_a_cancelled_member() {
        // `A1-081` relaxed the rule in the validator only, so `check` called the
        // plan clean while `why` named the cancelled member as a blocker and
        // `close` refused to close the cut, which is the situation that fix
        // existed for. One definition now, and this asserts they share it.
        let mut m = task("M-s0", Status::Todo, &["A-01", "A-02"]);
        m.tags = vec!["milestone".into(), "s0".into()];
        let tasks = [
            m,
            task("A-01", Status::Dropped, &[]),
            task("A-02", Status::Done, &[]),
        ];
        let g = Graph::new(tasks.iter());
        let milestone = g.get("M-s0").expect("the cut");

        assert!(
            !g.blocks(milestone, "A-01"),
            "a cancelled member does not hold the checkpoint open"
        );
        assert!(
            g.unmet_blockers("M-s0").is_empty(),
            "so `close` must not refuse: {:?}",
            g.unmet_blockers("M-s0")
        );
        assert!(
            g.why_blocked("M-s0", Walk::Complete).is_empty(),
            "and `why` must not name it: {:?}",
            g.why_blocked("M-s0", Walk::Complete)
                .iter()
                .map(|t| &t.id)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_cancelled_predecessor_cut_still_blocks_every_surface() {
        // The other half of the same rule: a milestone in another milestone's
        // `blocked_by` is a sequencing edge, not a member.
        let mut dropped_cut = task("M-s0", Status::Dropped, &[]);
        dropped_cut.tags = vec!["milestone".into(), "s0".into()];
        let mut live_cut = task("M-s1", Status::Todo, &["M-s0"]);
        live_cut.tags = vec!["milestone".into(), "s1".into()];
        let tasks = [dropped_cut, live_cut];
        let g = Graph::new(tasks.iter());
        let live = g.get("M-s1").expect("the live cut");

        assert!(g.blocks(live, "M-s0"), "an abandoned prerequisite blocks");
        assert_eq!(g.unmet_blockers("M-s1"), vec!["M-s0"]);
    }

    #[test]
    fn an_unknown_blocker_blocks() {
        // `APLAN003` reports the typo; a task waiting on one is still not ready,
        // and `blocks` must say so or the graph would disagree with the gate.
        let tasks = [task("A", Status::Todo, &["GHOST"])];
        let g = Graph::new(tasks.iter());
        let a = g.get("A").expect("A");
        assert!(g.blocks(a, "GHOST"));
    }

    #[test]
    fn a_cycle_that_exists_only_through_an_inherited_edge_is_found() {
        // `check` reported "DAG acyclic" on a plan `critical-path` refused as
        // cyclic, and told the reader to run `check`. Two commands disagreeing,
        // with the one it names reporting clean (`A1-073`).
        let mut m0 = task("M-s0", Status::Todo, &["A0"]);
        m0.tags = vec!["milestone".into(), "s0".into()];
        let mut m1 = task("M-s1", Status::Todo, &["B1", "M-s0"]);
        m1.tags = vec!["milestone".into(), "s1".into()];
        // A0 is in cut 0 and waits on B1, which is in cut 1 and inherits M-s0.
        let tasks = [
            task("A0", Status::Todo, &["B1"]),
            task("B1", Status::Todo, &[]),
            m0,
            m1,
        ];
        let g = Graph::new(tasks.iter());
        let cycle = g.find_cycle().expect("the inherited edge closes a cycle");
        assert!(cycle.contains(&"M-s0".to_string()), "{cycle:?}");
    }

    /// `members` tasks gathered into `cuts` milestones, which is the shape
    /// `A1-227` measured: the task count is the same whatever `cuts` is, and
    /// only the width of one cut's membership changes.
    fn wide_cut_plan(members: usize, cuts: usize) -> Vec<Task> {
        let width = members / cuts;
        let mut tasks: Vec<Task> = (0..members)
            .map(|i| task(&format!("T-{i:05}"), Status::Todo, &[]))
            .collect();
        for c in 0..cuts {
            let ids: Vec<String> = (c * width..(c + 1) * width)
                .map(|i| format!("T-{i:05}"))
                .collect();
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            let mut m = task(&format!("M-{c:04}"), Status::Todo, &refs);
            m.tags = vec!["milestone".into()];
            tasks.push(m);
        }
        tasks
    }

    /// **What `find_cycle` costs does not depend on how wide a cut is**
    /// (`A1-227`).
    ///
    /// `aplan check` was quadratic in a single cut's membership: 25,000 tasks in
    /// ten cuts took 24,992ms, and the same 25,000 tasks in 2,500 cuts took
    /// 942ms. Timing this call put 2,234.8ms of a 2,347ms `validate` inside it.
    /// The walk re-enters a node once per blocker index and asked `blockers_of`
    /// on each of those entries, and a cut's `blocked_by` *is* its membership,
    /// so a milestone of M members rebuilt an M-element set M+1 times.
    ///
    /// A shape test rather than a budget, which is the only thing that can tell
    /// the exponent from the machine: the task count is held constant and only
    /// the number of cuts it is spread across changes, so every row does the
    /// same linear work and they differ only in the term this fixed. The old
    /// code fails it by two orders of magnitude rather than by a hair.
    #[test]
    fn finding_a_cycle_costs_the_same_however_wide_a_cut_is() {
        const MEMBERS: usize = 4_000;
        const SHAPES: [usize; 4] = [4, 16, 100, 1000];
        const ROUNDS: usize = 3;

        let plans: Vec<Vec<Task>> = SHAPES
            .into_iter()
            .map(|c| wide_cut_plan(MEMBERS, c))
            .collect();

        // **The fixtures are what they claim to be.** Everything below is a
        // timing, and a timing is just as green against a plan with no cuts in
        // it at all -- which is the shape that would make this test vacuous
        // while still reporting success. Checked for every row, not just one,
        // so a builder that degrades only at a wide or narrow end is caught.
        for (plan, cuts) in plans.iter().zip(SHAPES) {
            assert_eq!(plan.len(), MEMBERS + cuts, "{cuts} cuts: task count moved");
            let g = Graph::new(plan.iter());
            assert_eq!(
                g.blockers_of("M-0000").len(),
                MEMBERS / cuts,
                "{cuts} cuts: the milestone does not hold the membership that is \
                 the whole thing being measured"
            );
        }

        // Round-robin rather than shape by shape: a machine that gets busy for
        // one stretch then slows every row instead of only the row that happened
        // to be under measurement at the time. Best of `ROUNDS` for the same
        // reason.
        let mut best = [std::time::Duration::MAX; 4];
        for _ in 0..ROUNDS {
            for (i, plan) in plans.iter().enumerate() {
                let g = Graph::new(plan.iter());
                let start = std::time::Instant::now();
                let cycle = g.find_cycle();
                best[i] = best[i].min(start.elapsed());
                assert_eq!(
                    cycle, None,
                    "the fixture must be acyclic to be walked whole"
                );
            }
        }

        // Slowest row against fastest, rather than widest against narrowest, so
        // a middle shape going quadratic is caught as well and the comparison
        // does not depend on the order `SHAPES` happens to be written in. The
        // additive slack stops a sub-millisecond row from making the ratio
        // meaningless; the factor is what actually fails.
        let slowest = best.iter().max().expect("four shapes");
        let fastest = best.iter().min().expect("four shapes");
        let bound = *fastest * 6 + std::time::Duration::from_millis(20);
        let shown: Vec<String> = SHAPES
            .into_iter()
            .zip(best)
            .map(|(c, d)| format!("{c} cuts of {} = {d:?}", MEMBERS / c))
            .collect();
        assert!(
            *slowest <= bound,
            "the cost is following a cut's width, which is the quadratic back: {}",
            shown.join(", ")
        );
    }

    #[test]
    fn a_plan_whose_cuts_do_not_follow_one_another_is_unchanged() {
        // Opt-in by construction: this repository's own cuts are not sequential,
        // and nothing about them may move.
        let mut m0 = task("M-a", Status::Todo, &["A1"]);
        m0.tags = vec!["milestone".into(), "v0.1".into()];
        let mut m1 = task("M-b", Status::Todo, &["B1"]);
        m1.tags = vec!["milestone".into(), "v0.2".into()];
        let tasks = [task_e("A1", &[], 2.0), task_e("B1", &[], 3.0), m0, m1];
        let g = Graph::new(tasks.iter());
        // The claim here is membership, not order: both cuts being open is what
        // "unsequenced" means. The order is `Rank`'s and is asserted where that
        // is the subject (`A1-125`), so this sorts before comparing rather than
        // quietly becoming a second, weaker test of the ranking.
        let mut ready: Vec<&str> = g.ready().iter().map(|t| t.id.as_str()).collect();
        ready.sort_unstable();
        assert_eq!(ready, vec!["A1", "B1"], "both cuts are open");
    }

    #[test]
    fn a_task_blocked_only_by_an_unknown_id_is_still_not_ready() {
        // `blockers_of` filters to known ids so the inherited edges can be
        // resolved; a declared edge to a ghost must survive that filter, or a
        // task blocked solely by a typo would be offered as work.
        let tasks = [task("A", Status::Todo, &["GHOST"])];
        let g = Graph::new(tasks.iter());
        assert!(g.ready().is_empty(), "a ghost still blocks");
        assert_eq!(g.unmet_blockers("A"), vec!["GHOST"]);
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
        let why: Vec<_> = g
            .why_blocked("C", Walk::Complete)
            .iter()
            .map(|t| t.id.as_str())
            .collect();
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
    fn an_empty_path_is_positive_zero() {
        // `Sum` for floats folds from `-0.0`, so a cut whose every member was
        // dropped rendered `crit -0.0d`. Negative zero is arithmetically fine
        // and reads as a bug in a report.
        let tasks: [Task; 0] = [];
        let g = Graph::new(tasks.iter());
        let path = g
            .critical_path_within(&BTreeSet::new())
            .expect("an empty node set is trivially acyclic");
        assert!(
            path.total_effort_days.is_sign_positive(),
            "got {}",
            path.total_effort_days
        );
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

#[cfg(test)]
mod closeable {
    use super::*;
    use crate::task::{Status, Task};

    fn t(id: &str, status: Status, tags: &[&str], blocked_by: &[&str]) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            phase: 0,
            status,
            owner: None,
            effort_days: None,
            blocked_by: blocked_by.iter().map(|s| (*s).to_string()).collect(),
            prefer_after: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: tags.iter().map(|s| (*s).to_string()).collect(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
        }
    }

    /// Proves the named behaviour `ready.reports_a_closeable_cut` (`A1-211`).
    #[test]
    fn a_cut_whose_members_are_done_is_closeable() {
        let tasks = [
            t("M-s0", Status::Todo, &["milestone", "s0"], &["A-1"]),
            t("A-1", Status::Done, &[], &[]),
        ];
        let g = Graph::new(tasks.iter());
        let ids: Vec<&str> = g
            .closeable_milestones()
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(ids, vec!["M-s0"]);
    }

    #[test]
    fn a_cut_with_live_members_is_not() {
        let tasks = [
            t("M-s0", Status::Todo, &["milestone", "s0"], &["A-1", "A-2"]),
            t("A-1", Status::Done, &[], &[]),
            t("A-2", Status::Todo, &[], &[]),
        ];
        let g = Graph::new(tasks.iter());
        assert!(g.closeable_milestones().is_empty());
    }

    #[test]
    fn an_empty_cut_is_mis_shaped_rather_than_finished() {
        // `all()` over nothing is true, so an empty milestone read as "members all
        // done" and the reader was told to record the completion of a cut that
        // scoped no work. That is `APLAN021`'s finding, not a close.
        let tasks = [t("M-empty", Status::Todo, &["milestone", "s0"], &[])];
        let g = Graph::new(tasks.iter());
        assert!(g.closeable_milestones().is_empty());
    }

    #[test]
    fn a_cut_already_closed_is_not_offered_again() {
        for status in [Status::Done, Status::Dropped] {
            let tasks = [
                t("M-s0", status, &["milestone", "s0"], &["A-1"]),
                t("A-1", Status::Done, &[], &[]),
            ];
            let g = Graph::new(tasks.iter());
            assert!(g.closeable_milestones().is_empty(), "{status:?}");
        }
    }

    #[test]
    fn a_cut_waiting_on_the_cut_before_it_is_not_closeable() {
        // Sequencing (ADR-0013): the members of `M-s1` inherit the edge, and the
        // milestone itself waits too.
        let tasks = [
            t("M-s0", Status::Todo, &["milestone", "s0"], &["A-1"]),
            t("A-1", Status::Todo, &[], &[]),
            t("M-s1", Status::Todo, &["milestone", "s1"], &["B-1", "M-s0"]),
            t("B-1", Status::Done, &[], &[]),
        ];
        let g = Graph::new(tasks.iter());
        assert!(
            g.closeable_milestones().is_empty(),
            "cut 0 is unfinished, so neither cut can be closed"
        );
    }
}

#[cfg(test)]
mod ranking {
    use super::*;
    use crate::task::{Status, Task};

    /// A preference moves a task later in the offer and changes nothing else
    /// (`A1-150`, ADR-0025).
    /// Proves the named behaviour `ready.honours_a_declared_preference` (`A1-211`).
    #[test]
    fn a_preference_orders_the_offer_without_blocking_it() {
        // Same effort, no releases, no spine: derived order is id order, so the
        // preference is the only thing that can move them.
        let mut b = w("B", &[], Some(1.0));
        b.prefer_after = vec!["C".into()];
        let plan = [w("A", &[], Some(1.0)), b, w("C", &[], Some(1.0))];
        let g = Graph::new(plan.iter());
        let order: Vec<&str> = g
            .ready_ranked()
            .iter()
            .map(|(t, _)| t.id.as_str())
            .collect();
        assert_eq!(order, vec!["A", "C", "B"], "the preference did not order");
        // And nothing was withheld: a preference is not a blocker.
        assert_eq!(order.len(), 3);
    }

    /// The preference is inert when what it names is not on offer, because
    /// "offer me after that one" says nothing about a task nobody is offering.
    #[test]
    fn a_preference_naming_an_unoffered_task_changes_nothing() {
        let mut a = w("A", &[], Some(1.0));
        a.prefer_after = vec!["Z".into()];
        let mut done = w("Z", &[], Some(1.0));
        done.status = Status::Done;
        let plan = [a, w("B", &[], Some(1.0)), done];
        let g = Graph::new(plan.iter());
        let order: Vec<&str> = g
            .ready_ranked()
            .iter()
            .map(|(t, _)| t.id.as_str())
            .collect();
        assert_eq!(order, vec!["A", "B"], "a finished task was followed");
    }

    /// A cycle has no order, and `APLAN033` reports it. What must not happen is
    /// work disappearing from "what next" because the plan has a defect.
    #[test]
    fn a_preference_cycle_still_offers_every_task() {
        let mut a = w("A", &[], Some(1.0));
        a.prefer_after = vec!["B".into()];
        let mut b = w("B", &[], Some(1.0));
        b.prefer_after = vec!["A".into()];
        let plan = [a, b];
        let g = Graph::new(plan.iter());
        let order: Vec<&str> = g
            .ready_ranked()
            .iter()
            .map(|(t, _)| t.id.as_str())
            .collect();
        assert_eq!(order.len(), 2, "a cycle swallowed the work: {order:?}");
    }

    /// The derived rank still decides everything the preference does not touch,
    /// so a preference is a tiebreak on an order rather than a replacement for
    /// it.
    /// Proves the named behaviour `ready.order_is_derived` (`A1-211`).
    #[test]
    fn the_derived_rank_still_decides_the_rest() {
        let mut small = w("S", &[], Some(1.0));
        small.prefer_after = vec!["T".into()];
        let plan = [w("BIG", &[], Some(9.0)), small, w("T", &[], Some(2.0))];
        let g = Graph::new(plan.iter());
        let order: Vec<&str> = g
            .ready_ranked()
            .iter()
            .map(|(t, _)| t.id.as_str())
            .collect();
        // Effort still ranks BIG first; the preference only orders S after T.
        assert_eq!(order, vec!["BIG", "T", "S"]);
    }

    /// A work task: a title equal to its id, and an effort unless `None`.
    fn w(id: &str, blocked_by: &[&str], effort: Option<f64>) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            phase: 0,
            status: Status::Todo,
            owner: None,
            effort_days: effort,
            blocked_by: blocked_by.iter().map(|s| (*s).to_string()).collect(),
            prefer_after: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
        }
    }

    fn task_e(id: &str, blocked_by: &[&str], effort: f64) -> Task {
        w(id, blocked_by, Some(effort))
    }

    fn done(id: &str, blocked_by: &[&str], effort: f64) -> Task {
        let mut t = task_e(id, blocked_by, effort);
        t.status = Status::Done;
        t
    }

    fn cut(id: &str, status: Status, members: &[&str]) -> Task {
        let mut t = w(id, members, None);
        t.status = status;
        t.tags = vec!["milestone".to_string()];
        t
    }

    /// A fixture whose derived order is also its id order proves nothing about
    /// which of the two produced it (`A1-211`).
    fn not_alphabetical(order: &[&str]) {
        let mut by_id = order.to_vec();
        by_id.sort_unstable();
        assert_ne!(
            order, by_id,
            "this fixture's derived order is its alphabetical order, so the assertion \
             above holds on a build that ignores the rank entirely"
        );
    }

    /// The precedence, in one plan that exercises every level of it (`A1-125`).
    ///
    /// Id order and rank order disagree at every position, which is the point:
    /// `ready` used to answer alphabetically, and alphabetical is not an order.
    /// Proves the named behaviour `ready.order_is_derived` (`A1-211`).
    #[test]
    fn ready_is_ordered_by_what_the_plan_records_not_by_id() {
        let tasks = [
            // Releases `A-1`, which nothing else does. Named to sort *last*:
            // an earlier fixture used `A-9`, whose derived position happened to
            // be its alphabetical one, so this test passed unchanged on a build
            // that sorted by id -- the one thing its name says it rules out
            // (`A1-211`).
            task_e("Z-9", &[], 1.0),
            task_e("A-1", &["Z-9"], 1.0),
            // The critical path: 4 + 5 is the longest effort-weighted chain.
            done("B-0", &[], 4.0),
            task_e("B-1", &["B-0"], 5.0),
            // Off the spine, releasing nothing: effort orders them, id breaks the
            // tie.
            task_e("C-1", &[], 3.0),
            task_e("C-2", &[], 3.0),
            w("D-1", &[], None),
        ];
        let g = Graph::new(tasks.iter());
        let order: Vec<&str> = g.ready().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(order, vec!["Z-9", "B-1", "C-1", "C-2", "D-1"]);
        not_alphabetical(&order);

        let ranked = g.ready_ranked();
        assert_eq!(ranked[0].1.releases, 1, "Z-9 releases A-1");
        assert!(!ranked[0].1.critical, "Z-9 is not on the spine");
        assert!(ranked[1].1.critical, "B-1 is");
        assert_eq!(ranked[1].1.releases, 0);
        // The unestimated one says so rather than sitting somewhere unexplained.
        assert_eq!(
            ranked[4].1.because(),
            "nothing to rank it by; ordered by id"
        );
        // `Z-9` releases `A-1`, and `A-1` is the whole chain behind it, so the
        // second term says one as well (`A1-298`).
        assert_eq!(
            ranked[0].1.because(),
            "releases 1, a chain of 1 behind it, 1d"
        );

        // `B-1` is both the spine and the biggest, so the order above would hold
        // even if the spine counted for nothing. This is the case that separates
        // them: a one-day task on the critical path, and a three-day task off it.
        let tasks = [
            done("E-0", &[], 10.0),
            task_e("E-1", &["E-0"], 1.0),
            task_e("A-7", &[], 3.0),
        ];
        let g = Graph::new(tasks.iter());
        let order: Vec<&str> = g.ready().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            order,
            vec!["E-1", "A-7"],
            "the spine outranks effort, and effort would have reversed this"
        );
        not_alphabetical(&order);
    }

    /// Shuffling the input changes nothing, the way `toposort`'s test does it:
    /// `ready` is read by scripts, and an order that moves between runs is worse
    /// than no order at all.
    /// Proves the named behaviour `ready.order_is_derived` (`A1-211`).
    #[test]
    fn the_order_does_not_depend_on_how_the_plan_was_written() {
        let make = |order: [usize; 5]| {
            let all = [
                task_e("A-1", &[], 2.0),
                task_e("A-2", &[], 2.0),
                task_e("A-3", &[], 2.0),
                task_e("A-4", &["A-9"], 1.0),
                task_e("A-9", &[], 1.0),
            ];
            let picked: Vec<Task> = order.iter().map(|i| all[*i].clone()).collect();
            picked
        };
        let forwards = make([0, 1, 2, 3, 4]);
        let backwards = make([4, 3, 2, 1, 0]);
        let shuffled = make([2, 0, 4, 1, 3]);
        let ids = |ts: &[Task]| {
            Graph::new(ts.iter())
                .ready()
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&forwards), ids(&backwards));
        assert_eq!(ids(&forwards), ids(&shuffled));
        // And equal-rank tasks keep their id order, which is the stable tiebreak
        // rather than a claim about them.
        assert_eq!(ids(&forwards), vec!["A-9", "A-1", "A-2", "A-3"]);
    }

    /// **The chain behind a task does not run through the cut it belongs to**
    /// (`A1-298`).
    ///
    /// The edge that would inflate it is the one a member *inherits*: `B-1` sits
    /// in a cut that follows `M-s0` (ADR-0013), so it waits on that milestone,
    /// and `M-s0` in turn waits on its own members. Walked naively, every task in
    /// an earlier cut would have the whole of every later cut behind it -- a
    /// number about the plan's shape rather than about the task.
    ///
    /// Counting only edges where neither end is a milestone excludes it, along
    /// with membership in both directions, which is why the rule is stated that
    /// way rather than as three separate exclusions.
    ///
    /// The assertion below is on the *declared* case, `C-1` waiting on a whole
    /// cut, and that is deliberate: it is the one a mutation can reach. Reading
    /// `blockers_of` instead of `blocked_by` changes nothing here, because every
    /// inherited edge has a milestone at its tail and the same filter drops it --
    /// checked by mutating both. So there is one rule to keep, not two.
    #[test]
    fn the_chain_behind_a_task_does_not_run_through_its_cut() {
        let tasks = [
            // The first cut: `A-1` really does block `A-2`.
            task_e("A-1", &[], 1.0),
            task_e("A-2", &["A-1"], 1.0),
            cut("M-s0", Status::Todo, &["A-1", "A-2"]),
            // The second cut follows the first, so its members inherit `M-s0`.
            task_e("B-1", &[], 1.0),
            task_e("B-2", &["B-1"], 1.0),
            cut("M-s1", Status::Todo, &["B-1", "B-2", "M-s0"]),
            // And the other direction of the same rule: a task may wait on a
            // whole cut *declaring* it, which is a constraint rather than
            // membership. Closing `M-s0` does free `C-1`, but a checkpoint is not
            // credited with the work behind it any more than it is credited with
            // releasing it (`A1-233`).
            task_e("C-1", &["M-s0"], 1.0),
            task_e("C-2", &["C-1"], 1.0),
        ];
        let g = Graph::new(tasks.iter());
        let depth = g.downstream_depth_all();
        assert_eq!(
            depth.get("A-1").copied().unwrap_or(0),
            1,
            "`A-1` has only `A-2` behind it; walking the inherited cut edge would \
             say 3 by adding the whole of the next cut"
        );
        assert_eq!(depth.get("B-1").copied().unwrap_or(0), 1, "`B-2` only");
        assert_eq!(
            depth.get("M-s0").copied().unwrap_or(0),
            0,
            "a cut is not credited with the work that waits behind it either, and \
             `C-1` declares it as a blocker so this is the case that proves it"
        );
        assert_eq!(depth.get("C-1").copied().unwrap_or(0), 1, "`C-2` only");
    }

    /// The term counts the chain's *length*, so a longer tail outranks a shorter
    /// one when nothing else separates them (`A1-298`).
    /// Proves the named behaviour `ready.order_is_derived` (`A1-211`), whose
    /// description names this term.
    #[test]
    fn a_longer_chain_behind_a_task_ranks_it_higher() {
        let tasks = [
            // A third, much heavier chain takes the spine, so that neither head
            // below is on it. Without this the test passes on `critical` alone
            // and proves nothing about the term it is named for -- which is what
            // the first version of it did.
            task_e("S-1", &[], 20.0),
            task_e("S-2", &["S-1"], 20.0),
            // Two heads at identical effort, each releasing exactly one task, so
            // the first two terms tie and only the depth behind them can order
            // them.
            // The *deeper* chain is named last on purpose. Id ascending is the
            // final tiebreak, so a fixture where the deeper head sorts first
            // alphabetically passes with the term removed entirely -- which is
            // what the second version of this test did.
            task_e("A-1", &[], 2.0),
            task_e("A-2", &["A-1"], 2.0),
            task_e("Z-1", &[], 2.0),
            task_e("Z-2", &["Z-1"], 2.0),
            task_e("Z-3", &["Z-2"], 2.0),
        ];
        let g = Graph::new(tasks.iter());
        let ranked = g.ready_ranked();
        let by_id = |id: &str| {
            ranked
                .iter()
                .find(|(t, _)| t.id == id)
                .map(|(_, r)| *r)
                .expect("ranked")
        };
        let (deep, shallow) = (by_id("Z-1"), by_id("A-1"));
        assert_eq!(
            (deep.releases, deep.critical),
            (shallow.releases, shallow.critical),
            "the fixture must tie on every term above this one, or it is not testing it"
        );
        assert_eq!((deep.downstream_depth, shallow.downstream_depth), (2, 1));
        let order: Vec<&str> = ranked.iter().map(|(t, _)| t.id.as_str()).collect();
        let pos = |id: &str| order.iter().position(|x| *x == id).expect("offered");
        assert!(
            pos("Z-1") < pos("A-1"),
            "the deeper chain is offered first, against id order: {order:?}"
        );
        assert!(deep.because().contains("a chain of 2 behind it"));
    }

    /// **A cut is never counted as released work, even when a cut alone holds it
    /// up** (`A1-233`).
    ///
    /// This pins a decision rather than a mechanism. `A1-233` asked whether the
    /// membership question here should route through `edge_kind`, which is what
    /// `A1-216` built for it. It must not, and this case is why: `edge_kind`
    /// classifies a cut sequenced after another cut (ADR-0013) as a
    /// `Dependency`, both ends being milestones, so the predecessor would start
    /// being credited with releasing its successor. A checkpoint becoming
    /// closeable is not work becoming ready, and `closeable_cuts` already answers
    /// that question directly.
    ///
    /// So the refactor is declined, and this makes it fail a test rather than
    /// change a meaning quietly. `M-s0` alone holds up both `C-1`, a task, and
    /// `M-s1`, a cut whose own member is already finished. Only the task counts.
    #[test]
    fn a_cut_is_not_released_work_even_when_a_cut_alone_holds_it_up() {
        let tasks = [
            // `M-s0` stays open because `A-1` is unfinished.
            task_e("A-1", &[], 1.0),
            cut("M-s0", Status::Todo, &["A-1"]),
            // `M-s1` follows `M-s0`, and its own member is done, so the cut it
            // follows is its one remaining blocker.
            done("B-1", &[], 1.0),
            cut("M-s1", Status::Todo, &["B-1", "M-s0"]),
            // A real task in the same position: waiting on that cut alone.
            task_e("C-1", &["M-s0"], 1.0),
        ];
        let g = Graph::new(tasks.iter());
        assert_eq!(
            g.releases("M-s0"),
            1,
            "`M-s0` releases `C-1`, and must not also be credited with `M-s1`, \
             which is a checkpoint rather than work"
        );
    }

    /// **A cut of many members does not rank them all alike** (`A1-233`).
    ///
    /// The measurement that task asked to be turned into a test, and it pins the
    /// *consequence* rather than the mechanism: six members of one cut, identical
    /// effort so nothing else can discriminate, and only the two that really
    /// unblock something score.
    ///
    /// It is deliberately not the guard against membership counting --
    /// `releases_counts_only_dependents_it_would_really_release` is, and removing
    /// the milestone filter fails that one and not this one. The reason is worth
    /// knowing: a cut with six unfinished members is never any single member's
    /// release, because the sole-blocker rule already refuses a dependent that
    /// several tasks hold open. The filter bites only where a cut has exactly one
    /// unfinished member, which is the shape that test builds on purpose.
    #[test]
    fn a_cut_of_many_members_does_not_rank_them_all_alike() {
        let tasks = [
            // Identical effort throughout: if the ranks differ, `releases` is the
            // only thing that could have done it.
            task_e("A-1", &[], 2.0),
            task_e("A-2", &["A-1"], 2.0),
            task_e("A-3", &[], 2.0),
            task_e("A-4", &["A-3"], 2.0),
            task_e("A-5", &[], 2.0),
            task_e("A-6", &[], 2.0),
            cut(
                "M-s0",
                Status::Todo,
                &["A-1", "A-2", "A-3", "A-4", "A-5", "A-6"],
            ),
        ];
        let g = Graph::new(tasks.iter());
        let ranked = g.ready_ranked();
        let scores: Vec<(&str, usize)> = ranked
            .iter()
            .map(|(t, r)| (t.id.as_str(), r.releases))
            .collect();
        assert_eq!(
            scores,
            vec![("A-1", 1), ("A-3", 1), ("A-5", 0), ("A-6", 0)],
            "membership is scoring, so the cut ranks its members alike"
        );
        assert!(
            scores.iter().map(|(_, n)| n).collect::<BTreeSet<_>>().len() > 1,
            "every member of the cut scored the same, which is the defect"
        );
    }

    /// And the honest other half: with **no** dependency edges in the cut, every
    /// member does score alike -- at zero rather than at one.
    ///
    /// Excluding membership is necessary and not sufficient, which `A1-233` said
    /// plainly and this pins so nobody reads the test above as a claim that the
    /// order always discriminates. A flat zero is no more informative than a flat
    /// one; what it buys is that the term *can* mean something where real edges
    /// exist. Making the order discriminate on a plan that has no such edges is
    /// `A1-231`, and it is a different question.
    #[test]
    fn a_cut_whose_members_block_nothing_scores_them_all_zero() {
        let tasks = [
            task_e("A-1", &[], 2.0),
            task_e("A-2", &[], 2.0),
            task_e("A-3", &[], 2.0),
            cut("M-s0", Status::Todo, &["A-1", "A-2", "A-3"]),
        ];
        let g = Graph::new(tasks.iter());
        let ranked = g.ready_ranked();
        assert!(
            ranked.iter().all(|(_, r)| r.releases == 0),
            "a member released its own checkpoint"
        );
        // And the explanation does not pretend otherwise. It never claims a
        // release, and names only the terms actually doing the ordering -- here
        // the spine and the estimate, since with no chain at all the heaviest
        // task is trivially the longest one.
        assert!(
            ranked
                .iter()
                .all(|(_, r)| !r.because().contains("releases")),
            "an explanation claims a release where the term is inert"
        );
        assert_eq!(ranked[0].1.because(), "on the critical path, 2d");
    }

    /// `releases` counts what finishing this task would actually make workable,
    /// which is not the same as what points at it.
    #[test]
    fn releases_counts_only_dependents_it_would_really_release() {
        let tasks = [
            task_e("A", &[], 1.0),
            task_e("B", &[], 1.0),
            // Waiting on both, so neither releases it alone.
            task_e("C", &["A", "B"], 1.0),
            // Waiting only on `A`.
            task_e("D", &["A"], 1.0),
            // A cut whose only member is `A`: membership is an edge (ADR-0008),
            // so without the milestone filter finishing `A` would "release" its
            // own checkpoint, and nearly every task in a plan would score alike.
            // The cut is alone in listing `A` on purpose: a cut with four members
            // is held open by the other three, and would hide the filter behind
            // the one that asks whether the dependent is really released.
            cut("M-s0", Status::Todo, &["A"]),
        ];
        let g = Graph::new(tasks.iter());
        assert_eq!(g.releases("A"), 1, "only `D`, never the cut");
        assert_eq!(g.releases("B"), 0);

        // Once `B` is done, finishing `A` releases both.
        let tasks = [
            task_e("A", &[], 1.0),
            done("B", &[], 1.0),
            task_e("C", &["A", "B"], 1.0),
            task_e("D", &["A"], 1.0),
        ];
        assert_eq!(Graph::new(tasks.iter()).releases("A"), 2);

        // A task that blocks itself is `APLAN006`'s finding, and it is the one
        // task in a plan that is the sole thing holding itself up. Finishing it
        // releases nothing, and counting it would put a broken task at the top of
        // the answer to "what should I do next" -- where it is not even ready.
        let tasks = [task_e("A", &["A"], 1.0), task_e("B", &[], 1.0)];
        let g = Graph::new(tasks.iter());
        assert_eq!(g.releases("A"), 0);
        assert!(g.releases_all().is_empty());
        assert_eq!(
            g.ready().iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["B"]
        );
    }

    /// A plan with no estimates has no spine, and the rank says so instead of
    /// promoting whichever id sorts first (`A1-125`).
    ///
    /// `critical_path` picks the task with the greatest finish and breaks ties by
    /// id, so on a plan that estimates nothing it returns a zero-day path holding
    /// one arbitrary task. Marking that one "on the critical path" would be the
    /// arbitrary answer this rank exists to replace.
    #[test]
    fn a_spine_of_no_days_is_not_a_spine() {
        let none: Vec<&str> = Vec::new();
        let tasks = [
            w("A-2", &none, None),
            w("A-1", &none, None),
            w("A-3", &none, Some(0.0)),
        ];
        let g = Graph::new(tasks.iter());
        let ranked = g.ready_ranked();
        assert!(
            ranked.iter().all(|(_, r)| !r.critical),
            "a zero-day path is not a spine: {:?}",
            ranked
                .iter()
                .map(|(t, r)| (&t.id, r.critical))
                .collect::<Vec<_>>()
        );
        let order: Vec<&str> = ranked.iter().map(|(t, _)| t.id.as_str()).collect();
        assert_eq!(
            order,
            vec!["A-1", "A-2", "A-3"],
            "so it falls to id, stably"
        );
        assert_eq!(
            ranked[0].1.because(),
            "nothing to rank it by; ordered by id",
            "and it says so rather than implying a reason"
        );

        // One estimate is enough to make the longest chain mean something again.
        let tasks = [w("A-2", &none, None), w("A-1", &none, Some(2.0))];
        let g = Graph::new(tasks.iter());
        let ranked = g.ready_ranked();
        assert!(ranked[0].1.critical && ranked[0].0.id == "A-1");
    }

    /// The ranking must not make `ready` quadratic, which the first version of it
    /// did (`A1-125`).
    ///
    /// It asked "what does this release" once per ready task, and each answer
    /// scanned every task and rebuilt its blocker set. On a 2000-task plan that
    /// took `aplan ready` from 46ms to 765ms; inverting the graph once put it
    /// back to 49ms. A shape test rather than a benchmark: the bound is roughly
    /// twenty times what this costs when it is linear, so it fails on the
    /// quadratic version and not on a slow machine.
    #[test]
    fn ranking_a_large_plan_stays_linear() {
        let n = 4000;
        let tasks: Vec<Task> = (0..n)
            .map(|i| {
                let id = format!("T-{i:05}");
                let blocked: Vec<&str> = Vec::new();
                let mut t = w(&id, &blocked, Some(1.0 + f64::from(i % 5)));
                // A third of them wait on the one before, so the plan has edges
                // to invert rather than being a flat list.
                if i % 3 == 0 && i > 0 {
                    t.blocked_by = vec![format!("T-{:05}", i - 1)];
                }
                t
            })
            .collect();

        let start = std::time::Instant::now();
        let g = Graph::new(tasks.iter());
        let ready = g.ready_ranked();
        let elapsed = start.elapsed();

        assert!(!ready.is_empty());
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "ranking {n} tasks took {elapsed:?}, which is the quadratic shape back"
        );
    }

    /// Why cut sequence is not one of the rank's inputs, proved rather than
    /// asserted (`A1-125`).
    ///
    /// ADR-0017 named it, and it cannot discriminate: where cuts are sequenced,
    /// a later cut's members inherit the earlier milestone as a blocker and are
    /// not ready at all; and when that cut closes, its own members are done. So
    /// no two ready tasks are ever in cuts the plan has ordered.
    #[test]
    fn no_two_ready_tasks_are_in_cuts_the_plan_has_sequenced() {
        let tasks = [
            cut("M-s0", Status::Todo, &["A-1", "A-2"]),
            done("A-1", &[], 1.0),
            task_e("A-2", &[], 1.0),
            cut("M-s1", Status::Todo, &["B-1", "M-s0"]),
            task_e("B-1", &[], 1.0),
        ];
        let g = Graph::new(tasks.iter());
        let ready: Vec<&str> = g.ready().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ready,
            vec!["A-2"],
            "`B-1` waits on the cut before it, so the two cuts never compete"
        );

        // And with the first cut finished, its members are no longer workable.
        let tasks = [
            cut("M-s0", Status::Done, &["A-1", "A-2"]),
            done("A-1", &[], 1.0),
            done("A-2", &[], 1.0),
            cut("M-s1", Status::Todo, &["B-1", "M-s0"]),
            task_e("B-1", &[], 1.0),
        ];
        let g = Graph::new(tasks.iter());
        let ready: Vec<&str> = g.ready().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ready, vec!["B-1"]);
    }
}
