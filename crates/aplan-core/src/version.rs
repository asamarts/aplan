//! What the plan says the next version must be (`A1-313`, ADR-0034).
//!
//! **The plan decides the number; the scheme spells it.** A task declares what
//! it did to the released artifact -- `none`, `fix`, `feature`, `breaking` --
//! and a level supplies a floor for the tasks that declare nothing. The answer
//! for a node is the strongest of what its members owe, which is why a floor
//! composes and an assertion would not: a `breaking` task raises the cut that
//! holds it and the release that closes the cut.
//!
//! Nothing here reads git or the artifact. This module answers "what does this
//! node owe", which is a question about the plan alone, and that is why
//! `aplan version next <ID>` resolves in `check` (ADR-0041). The bare form
//! needs a window over history and lives where git does.

use crate::graph::Graph;
use crate::task::{Task, VersionFloors, VersionImpact};

/// What a node owes, and the member that made it owe that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owed {
    /// The strongest impact anything beneath this node claims.
    pub impact: VersionImpact,
    /// The id that raised it, when a *member* did rather than this node's own
    /// floor. `None` when the floor is the answer, which is the common case for
    /// a plan that declares nothing.
    ///
    /// Named because "this release owes a minor" is not actionable and "this
    /// release owes a minor because `A1-306` is breaking" is: the whole reason
    /// the arithmetic is worth having is that it points at the claim.
    pub raised_by: Option<String>,
}

/// What one task owes: its declaration, else the floor for its level.
///
/// A declaration wins over a floor in both directions. A task claiming `none`
/// owes nothing even though the task floor is `fix` -- that is the case
/// `APLAN044` asks for a reason about -- and a task claiming `breaking` owes
/// that even though no floor would have.
#[must_use]
pub fn owed_by_task(t: &Task, floors: VersionFloors) -> VersionImpact {
    // `VersionFloors::of` already answers "which floor is this task's level",
    // and a second copy here would be two answers to one question -- the drift
    // this project gates elsewhere. It is the only thing this function adds to
    // the declaration, so the function is thin on purpose.
    t.version_impact.unwrap_or_else(|| floors.of(t))
}

/// What a node owes, aggregating over everything beneath it.
///
/// The walk is the membership one: a cut's members are its work, a checkpoint's
/// are the work beneath it at any depth (ADR-0042). A node's own declaration or
/// floor is included, so a cut that declares `breaking` owes that even when its
/// members are all fixes.
///
/// **Not a maximum over a flat list.** Each member contributes what *it* owes,
/// which for a milestone is its own aggregate, so the answer composes up the
/// hierarchy rather than being recomputed from the leaves at each level. That is
/// what makes "what this cut owes" and "the release's arithmetic restricted to
/// that cut" the same number rather than two that happen to agree.
#[must_use]
pub fn owed(graph: &Graph<'_>, floors: VersionFloors, id: &str) -> Option<Owed> {
    let mut seen = std::collections::BTreeSet::new();
    owed_walk(graph, floors, id, &mut seen)
}

/// The walk, carrying the set of nodes already counted.
///
/// **`seen` is threaded through the recursion, not rebuilt per level**, and that
/// is the whole of the termination argument. An earlier cut kept it local to the
/// member listing, which deduplicated one node's list and did nothing across
/// levels: two checkpoints containing each other recursed until the stack ran
/// out, and `aplan version` dumped core on a plan `check` merely refuses. A
/// crash is a worse answer than a diagnostic, and it is not this function's job
/// to have opinions about a plan that did not pass `APLAN047`.
///
/// A node already counted returns `None` rather than its impact, so the caller
/// skips it: its contribution is in the total once already.
fn owed_walk(
    graph: &Graph<'_>,
    floors: VersionFloors,
    id: &str,
    seen: &mut std::collections::BTreeSet<String>,
) -> Option<Owed> {
    let task = graph.get(id)?;
    if !seen.insert(id.to_string()) {
        return None;
    }
    let mut best = owed_by_task(task, floors);
    let mut raised_by = None;
    for member in members(graph, task) {
        let Some(m) = graph.get(&member) else {
            continue;
        };
        // A dropped task is not work that ships, and counting it would let a
        // cancelled breaking change raise a release forever (`A1-074`'s rule,
        // one arithmetic along).
        if m.status == crate::task::Status::Dropped {
            continue;
        }
        let sub = if m.is_milestone() {
            match owed_walk(graph, floors, &member, seen) {
                Some(o) => o,
                // Already counted somewhere else in this walk.
                None => continue,
            }
        } else if seen.insert(member.clone()) {
            Owed {
                impact: owed_by_task(m, floors),
                raised_by: m.version_impact.is_some().then(|| member.clone()),
            }
        } else {
            continue;
        };
        if sub.impact > best {
            best = sub.impact;
            // **The claim, not the level above it.** A release whose cut owes
            // `breaking` used to name the *cut*, so a reader told "`M-C` is
            // breaking" had to walk again to find the task that said so -- and
            // the message reads as though the cut had declared it. The member's
            // own attribution is carried up where it has one, and the member is
            // named where the answer was its floor (`A1-348`, audit).
            raised_by = sub.raised_by.or_else(|| Some(member.clone()));
        }
    }
    Some(Owed {
        impact: best,
        raised_by,
    })
}

/// The ids directly beneath a node: its cut members, and the checkpoints it
/// contains.
///
/// Shallow on purpose -- `owed_walk` recurses through milestones itself, so that
/// each level's answer is built from the level below rather than from the
/// leaves.
///
/// **A release descends its `blocked_by` milestones too**, which is ADR-0022's
/// "read one level through" kept for every plan written before `contains`
/// existed. Without it a release in that shape aggregates nothing and answers
/// `none` -- silently, on the shape ADR-0022 endorses. The roadmap's own walk
/// unions the two for the same reason; this one has to agree with it or the
/// version and the rollup describe different plans.
fn members(graph: &Graph<'_>, m: &Task) -> Vec<String> {
    let mut out = Vec::new();
    let mut listed = std::collections::BTreeSet::new();
    for b in m.blocked_by.iter().chain(m.contains.iter()) {
        if !listed.insert(b.as_str()) {
            continue;
        }
        let Some(t) = graph.get(b) else { continue };
        let membership = crate::edge_kind(t, m) == crate::EdgeKind::Membership;
        let contained = m.contains.iter().any(|c| c == b);
        let borrowed_by_release = m.is_release() && t.is_milestone();
        if membership || contained || borrowed_by_release {
            out.push(b.clone());
        }
    }
    out
}

/// Every id the aggregation for `id` counts, including `id` itself.
///
/// **The same walk, asked what it touched** (`A1-348`). A release's members are
/// cuts (ADR-0022) and a task no cut claims is in none of them, so the walk
/// never reaches it: a plan with a `cross-cutting` task declaring `breaking`
/// answered `fix` for the release that shipped it, which is the one failure this
/// arithmetic exists to prevent, on a shape the model explicitly endorses.
///
/// Answering "what did the walk miss" needs the walk's own definition of what it
/// reaches, and deriving that separately would be a second copy of "what a
/// release contains" -- the two would disagree the first time either changed.
///
/// Empty for an id this plan does not hold.
///
/// The floors are taken rather than assumed, even though nothing in the walk
/// branches on them today: a caller asking "what did *my* aggregation reach"
/// and getting an answer computed under different floors would be a divergence
/// nothing else would notice.
#[must_use]
pub fn reached(
    graph: &Graph<'_>,
    floors: VersionFloors,
    id: &str,
) -> std::collections::BTreeSet<String> {
    let mut seen = std::collections::BTreeSet::new();
    let _ = owed_walk(graph, floors, id, &mut seen);
    seen
}

/// What a set of closed tasks owes between them, and the one that demanded most.
///
/// The window form's arithmetic (ADR-0038): with no release node, what ships is
/// everything closed since the last release tag, which is a *set* rather than a
/// node, so there is no hierarchy to walk and no floor of its own to add. Each
/// member contributes what it declared or what its level implies, exactly as it
/// would inside a cut.
///
/// **`raised_by` names a *declaration*, never a floor.** A window of seventeen
/// ordinary tasks owes `fix` because seventeen floors say so, and naming
/// whichever one the walk reached first would read as an accusation of a task
/// that claimed nothing. What is worth pointing at is the claim that raised the
/// answer above what the levels implied, which is the whole reason the
/// arithmetic is worth having. `None` therefore means "the levels decided this",
/// which is the common case and an answer in itself.
#[must_use]
pub fn owed_among<'a>(tasks: impl IntoIterator<Item = &'a Task>, floors: VersionFloors) -> Owed {
    let mut best = VersionImpact::None;
    let mut raised_by = None;
    for t in tasks {
        // A dropped task is not work that ships, the same refusal `owed_walk`
        // makes: `closed` is built from what went `done`, but a caller asking
        // about an arbitrary set should not be able to smuggle one in.
        if t.status == crate::task::Status::Dropped {
            continue;
        }
        let owes = owed_by_task(t, floors);
        if owes > best {
            best = owes;
            raised_by = t.version_impact.is_some().then(|| t.id.clone());
        }
    }
    Owed {
        impact: best,
        raised_by,
    }
}

/// The `major.minor.patch` of a version, with any pre-release or build metadata
/// dropped. `None` for a scheme this does not understand.
///
/// Public because comparing two versions is the other half of the arithmetic:
/// `APLAN048` asks whether the artifact has moved as far as the plan requires,
/// and a second parser written for that question would answer a different one
/// the first time either changed. Ordering the triples is ordering the
/// releases -- the pre-release is deliberately not in it, because `0.4.0-dev`
/// and `0.4.0` are the same *release* being prepared and shipped.
#[must_use]
pub fn triple(version: &str) -> Option<(u64, u64, u64)> {
    /// One component, refusing a leading zero.
    ///
    /// Semver forbids leading zeroes in a numeric identifier, and enforcing it
    /// is what keeps `CalVer` out: `2026.09.1` parses perfectly well as
    /// `(2026, 9, 1)`, and the answer would be handed back spelled `2026.9.1`,
    /// which is a *different version*. Found by testing the refusal ADR-0034
    /// made rather than by reading the parser (`A1-313`).
    fn part(text: &str) -> Option<u64> {
        if text.len() > 1 && text.starts_with('0') {
            return None;
        }
        text.parse().ok()
    }
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let mut parts = core.split('.');
    let major = part(parts.next()?)?;
    let minor = part(parts.next()?)?;
    let patch = part(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// The next version, given the one that ships now and what the plan owes.
///
/// **`0.x` is not `1.x` with a smaller number.** Cargo formalises `0.MINOR.PATCH`
/// as `0.INCOMPATIBLE.COMPATIBLE`, so below 1.0 a `breaking` change raises the
/// *minor* and a `feature` raises the *patch*. That is the deciding argument in
/// ADR-0034 for a ladder of `none < fix < feature < breaking` rather than
/// semver's own words: an author writing `minor` for a breaking change would
/// have to know this rule for the file to mean what it says.
///
/// `None` for a version this does not understand, which is honest rather than a
/// guess: a `CalVer` or a four-part version is somebody else's scheme, and
/// ADR-0034 already refused to tell a project its date is a patch number.
#[must_use]
pub fn next_version(current: &str, owes: VersionImpact) -> Option<String> {
    let (major, minor, patch) = triple(current)?;
    // A pre-release of the version being asked about releases *as* that version:
    // `0.4.0-dev` owing a fix is `0.4.0`, not `0.4.1`. The `-dev` is how this
    // repository says "not released yet" (`A1-060`), and the number beside it is
    // already the one being prepared.
    let pre = current.contains('-');
    Some(match (major, owes) {
        (_, VersionImpact::None) if pre => format!("{major}.{minor}.{patch}"),
        (_, VersionImpact::None) => current.to_string(),
        (0, VersionImpact::Breaking) => format!("0.{}.0", minor + 1),
        (0, _) if pre => format!("0.{minor}.{patch}"),
        (0, VersionImpact::Feature | VersionImpact::Fix) => format!("0.{minor}.{}", patch + 1),
        (_, VersionImpact::Breaking) => format!("{}.0.0", major + 1),
        (_, _) if pre => format!("{major}.{minor}.{patch}"),
        (_, VersionImpact::Feature) => format!("{major}.{}.0", minor + 1),
        (_, VersionImpact::Fix) => format!("{major}.{minor}.{}", patch + 1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::Plan;

    fn t(id: &str, impact: Option<VersionImpact>, milestone: bool) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            phase: 0,
            status: crate::task::Status::Todo,
            owner: None,
            effort_days: if milestone { None } else { Some(1.0) },
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            evidence_fingerprints: std::collections::BTreeMap::new(),
            tags: if milestone {
                vec![crate::task::MILESTONE_TAG.to_string()]
            } else {
                Vec::new()
            },
            revisit_at: None,
            version_impact: impact,
            version_impact_reason: None,
            version: None,
        }
    }

    fn plan_of(tasks: Vec<Task>) -> Plan {
        Plan {
            tasks: tasks
                .into_iter()
                .map(|task| crate::load::LoadedTask {
                    task,
                    path: std::path::PathBuf::from("plan/tasks/x.md"),
                    fields: crate::load::FieldLines::new(),
                    criteria: crate::load::Criteria::default(),
                })
                .collect(),
            load_diagnostics: Vec::new(),
            unloadable: std::collections::BTreeMap::new(),
        }
    }

    /// **A breaking task raises the cut that holds it and the release above**,
    /// and the answer says which member did it (`A1-313`).
    ///
    /// The whole reason a level supplies a *floor* rather than an assertion: a
    /// floor composes upward and an assertion would stop the claim at the level
    /// that made it.
    #[test]
    fn a_breaking_task_raises_its_cut_and_its_release() {
        let mut cut = t("M-C", None, true);
        cut.blocked_by = vec!["W-1".to_string(), "W-2".to_string()];
        let mut rel = t("M-R", None, true);
        rel.tags.push(crate::task::RELEASE_TAG.to_string());
        rel.contains = vec!["M-C".to_string()];
        rel.blocked_by = vec!["M-C".to_string()];
        let tasks = vec![
            t("W-1", None, false),
            t("W-2", Some(VersionImpact::Breaking), false),
            cut,
            rel,
        ];
        let plan = plan_of(tasks);
        let refs: Vec<&Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
        let g = Graph::new(refs);
        let f = VersionFloors::default();

        let at_cut = owed(&g, f, "M-C").expect("the cut");
        assert_eq!(at_cut.impact, VersionImpact::Breaking);
        assert_eq!(
            at_cut.raised_by.as_deref(),
            Some("W-2"),
            "the cut must name the claim"
        );

        let at_release = owed(&g, f, "M-R").expect("the release");
        assert_eq!(
            at_release.impact,
            VersionImpact::Breaking,
            "a release whose cut owes breaking owes it too"
        );
    }

    /// **A release written before `contains` existed still aggregates its cuts**
    /// (`A1-313`, audit).
    ///
    /// ADR-0022's shape is a release whose `blocked_by` names the cuts it
    /// closes, and every plan written before ADR-0042 uses it. The first cut of
    /// this walk required `contains` and answered `none` for such a release --
    /// silently, on the shape the model endorses. The roadmap's own walk unions
    /// the two, and this has to agree with it or the version and the rollup
    /// describe different plans.
    #[test]
    fn a_release_that_predates_contains_still_aggregates_its_cuts() {
        let mut cut = t("M-C", None, true);
        cut.blocked_by = vec!["W-1".to_string()];
        let mut rel = t("M-R", None, true);
        rel.tags.push(crate::task::RELEASE_TAG.to_string());
        rel.blocked_by = vec!["M-C".to_string()];
        let plan = plan_of(vec![
            t("W-1", Some(VersionImpact::Breaking), false),
            cut,
            rel,
        ]);
        let refs: Vec<&Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
        let g = Graph::new(refs);
        assert_eq!(
            owed(&g, VersionFloors::default(), "M-R").map(|o| o.impact),
            Some(VersionImpact::Breaking),
            "a release with no `contains` aggregated nothing"
        );
    }

    /// **A containment cycle terminates rather than exhausting the stack**
    /// (`A1-313`, audit).
    ///
    /// `APLAN047` refuses such a plan, but this function does not run the rules
    /// and must not crash on one that failed them: the first cut kept the seen
    /// set local to the member listing, so two checkpoints containing each other
    /// recursed until `aplan version` dumped core. A crash is a worse answer
    /// than a diagnostic.
    #[test]
    fn a_containment_cycle_terminates() {
        let mut a = t("M-A", None, true);
        a.tags.push("a".to_string());
        a.contains = vec!["M-B".to_string()];
        a.blocked_by = vec!["M-B".to_string()];
        let mut b = t("M-B", None, true);
        b.tags.push("b".to_string());
        b.contains = vec!["M-A".to_string()];
        b.blocked_by = vec!["M-A".to_string(), "W-1".to_string()];
        let plan = plan_of(vec![a, b, t("W-1", None, false)]);
        let refs: Vec<&Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
        let g = Graph::new(refs);
        // The assertion is that this returns at all. The value is the cut floor,
        // since the work beneath owes only a fix.
        assert_eq!(
            owed(&g, VersionFloors::default(), "M-A").map(|o| o.impact),
            Some(VersionImpact::Feature)
        );
    }

    /// A task reachable by two routes is counted once, which is what makes the
    /// answer independent of the order the walk happens to take.
    #[test]
    fn a_task_reached_twice_is_counted_once() {
        let mut cut = t("M-C", None, true);
        cut.blocked_by = vec!["W-1".to_string()];
        let mut rel = t("M-R", None, true);
        rel.tags.push(crate::task::RELEASE_TAG.to_string());
        rel.contains = vec!["M-C".to_string()];
        rel.blocked_by = vec!["M-C".to_string()];
        let plan = plan_of(vec![
            t("W-1", Some(VersionImpact::Breaking), false),
            cut,
            rel,
        ]);
        let refs: Vec<&Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
        let g = Graph::new(refs);
        let f = VersionFloors::default();
        assert_eq!(
            owed(&g, f, "M-R").map(|o| o.impact),
            Some(VersionImpact::Breaking),
            "the release reaches W-1 through both `contains` and `blocked_by`"
        );
    }

    /// Asking a cut gives the same answer as the release restricted to it, so
    /// the two cannot disagree -- which is `A1-313`'s fourth criterion and the
    /// reason the walk recurses through milestones rather than flattening.
    #[test]
    fn a_cut_and_the_release_above_it_agree() {
        let mut cut = t("M-C", None, true);
        cut.blocked_by = vec!["W-1".to_string()];
        let mut rel = t("M-R", None, true);
        rel.tags.push(crate::task::RELEASE_TAG.to_string());
        rel.contains = vec!["M-C".to_string()];
        rel.blocked_by = vec!["M-C".to_string()];
        let plan = plan_of(vec![
            t("W-1", Some(VersionImpact::Feature), false),
            cut,
            rel,
        ]);
        let refs: Vec<&Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
        let g = Graph::new(refs);
        let f = VersionFloors::default();
        assert_eq!(
            owed(&g, f, "M-C").map(|o| o.impact),
            owed(&g, f, "M-R").map(|o| o.impact)
        );
    }

    /// **The walk says what it reached**, which is how the caller learns what it
    /// missed (`A1-348`).
    ///
    /// A release's members are cuts, so a task no cut claims is in none of them.
    /// Deriving that set separately would be a second copy of "what a release
    /// contains", and the two would disagree the first time either changed.
    #[test]
    fn the_walk_reports_what_it_counted() {
        let mut cut = t("M-C", None, true);
        cut.blocked_by = vec!["W-1".to_string()];
        let mut rel = t("M-R", None, true);
        rel.tags.push(crate::task::RELEASE_TAG.to_string());
        rel.contains = vec!["M-C".to_string()];
        rel.blocked_by = vec!["M-C".to_string()];
        let mut dropped = t("W-3", Some(VersionImpact::Breaking), false);
        dropped.status = crate::task::Status::Dropped;
        cut.blocked_by.push("W-3".to_string());
        let plan = plan_of(vec![
            t("W-1", None, false),
            // In no cut at all: the shape `APLAN022` calls `cross-cutting`.
            t("W-2", Some(VersionImpact::Breaking), false),
            dropped,
            cut,
            rel,
        ]);
        let refs: Vec<&Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
        let g = Graph::new(refs);

        let counted = reached(&g, VersionFloors::default(), "M-R");
        assert!(counted.contains("M-R") && counted.contains("M-C") && counted.contains("W-1"));
        assert!(
            !counted.contains("W-2"),
            "no cut claims it, so the release's members do not reach it"
        );
        assert!(
            !counted.contains("W-3"),
            "a dropped task is not work that ships, so the walk does not count it"
        );
        // And the answer the walk gives is the one that misses the break, which
        // is the defect this exists to make measurable rather than arguable.
        assert_eq!(
            owed(&g, VersionFloors::default(), "M-R").map(|o| o.impact),
            Some(VersionImpact::Feature)
        );
        assert!(reached(&g, VersionFloors::default(), "no-such-id").is_empty());
    }

    /// **`raised_by` points at the claim, not at the level above it**
    /// (`A1-348`, audit).
    ///
    /// A release whose cut owes `breaking` used to name the *cut*, so a reader
    /// told "`M-C` is `breaking`" had to walk again to find the task that said
    /// so -- and the sentence reads as though the cut had declared it. The
    /// member's own attribution is carried up where it has one.
    #[test]
    fn the_attribution_is_the_claim_and_not_the_cut_holding_it() {
        let mut cut = t("M-C", None, true);
        cut.blocked_by = vec!["W-1".to_string()];
        let mut rel = t("M-R", None, true);
        rel.tags.push(crate::task::RELEASE_TAG.to_string());
        rel.blocked_by = vec!["M-C".to_string()];
        let plan = plan_of(vec![
            t("W-1", Some(VersionImpact::Breaking), false),
            cut,
            rel,
        ]);
        let refs: Vec<&Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
        let g = Graph::new(refs);
        let f = VersionFloors::default();
        assert_eq!(
            owed(&g, f, "M-R").and_then(|o| o.raised_by).as_deref(),
            Some("W-1"),
            "the release must name the task, not the cut between them"
        );
        assert_eq!(
            owed(&g, f, "M-C").and_then(|o| o.raised_by).as_deref(),
            Some("W-1")
        );
    }

    /// And a member whose answer is its *floor* is named itself, because there
    /// is no claim underneath to point at.
    #[test]
    fn a_member_raised_by_its_own_floor_is_named_itself() {
        let mut cut = t("M-C", None, true);
        cut.blocked_by = vec!["W-1".to_string()];
        let mut rel = t("M-R", None, true);
        rel.tags.push(crate::task::RELEASE_TAG.to_string());
        rel.blocked_by = vec!["M-C".to_string()];
        let plan = plan_of(vec![t("W-1", None, false), cut, rel]);
        let refs: Vec<&Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
        let g = Graph::new(refs);
        // The cut's floor is `feature`, above the release's `none`, and nothing
        // beneath it declared anything.
        assert_eq!(
            owed(&g, VersionFloors::default(), "M-R")
                .and_then(|o| o.raised_by)
                .as_deref(),
            Some("M-C")
        );
    }

    /// A cancelled task is not work that ships.
    ///
    /// `A1-074`'s rule, one arithmetic along: counting it would let a dropped
    /// breaking change raise every release after it, forever.
    #[test]
    fn a_dropped_task_owes_nothing() {
        let mut cut = t("M-C", None, true);
        cut.blocked_by = vec!["W-1".to_string()];
        let mut dropped = t("W-1", Some(VersionImpact::Breaking), false);
        dropped.status = crate::task::Status::Dropped;
        let plan = plan_of(vec![dropped, cut]);
        let refs: Vec<&Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
        let g = Graph::new(refs);
        let owed = owed(&g, VersionFloors::default(), "M-C").expect("the cut");
        assert_eq!(
            owed.impact,
            VersionImpact::Feature,
            "the cut's own floor, not the dropped claim"
        );
    }

    /// A declaration wins over a floor in both directions (ADR-0034).
    #[test]
    fn a_declaration_beats_the_floor_either_way() {
        let f = VersionFloors::default();
        assert_eq!(
            owed_by_task(&t("W", Some(VersionImpact::None), false), f),
            VersionImpact::None
        );
        assert_eq!(owed_by_task(&t("W", None, false), f), VersionImpact::Fix);
        assert_eq!(owed_by_task(&t("M", None, true), f), VersionImpact::Feature);
    }

    /// `0.x` is not `1.x` with a smaller number (ADR-0034).
    ///
    /// Cargo formalises `0.MINOR.PATCH` as `0.INCOMPATIBLE.COMPATIBLE`, so a
    /// breaking change below 1.0 raises the minor. Getting this wrong is the
    /// whole reason the ladder is not spelled in semver's own words.
    #[test]
    fn below_one_a_breaking_change_raises_the_minor() {
        assert_eq!(
            next_version("0.3.1", VersionImpact::Breaking).as_deref(),
            Some("0.4.0")
        );
        assert_eq!(
            next_version("0.3.1", VersionImpact::Feature).as_deref(),
            Some("0.3.2")
        );
        assert_eq!(
            next_version("0.3.1", VersionImpact::Fix).as_deref(),
            Some("0.3.2")
        );
        // At or above 1.0 the ordinary arithmetic applies.
        assert_eq!(
            next_version("1.2.3", VersionImpact::Breaking).as_deref(),
            Some("2.0.0")
        );
        assert_eq!(
            next_version("1.2.3", VersionImpact::Feature).as_deref(),
            Some("1.3.0")
        );
        assert_eq!(
            next_version("1.2.3", VersionImpact::Fix).as_deref(),
            Some("1.2.4")
        );
    }

    /// A pre-release releases *as* the number beside it.
    ///
    /// `0.4.0-dev` is how this repository says "not released yet" (`A1-060`),
    /// and the number is already the one being prepared -- so owing a fix makes
    /// it `0.4.0`, not `0.4.1`. Getting this wrong would skip a version on
    /// every release this project has ever cut.
    #[test]
    fn a_pre_release_ships_as_the_number_beside_it() {
        for owes in [
            VersionImpact::Fix,
            VersionImpact::Feature,
            VersionImpact::None,
        ] {
            assert_eq!(
                next_version("0.4.0-dev", owes).as_deref(),
                Some("0.4.0"),
                "{owes:?}"
            );
        }
        // Breaking still moves, because `0.4.0` would then be claiming
        // compatibility it does not have.
        assert_eq!(
            next_version("0.4.0-dev", VersionImpact::Breaking).as_deref(),
            Some("0.5.0")
        );

        // At or above 1.0, compatible work likewise finishes the version that
        // is already being prepared; only a breaking change advances it.
        for owes in [
            VersionImpact::Fix,
            VersionImpact::Feature,
            VersionImpact::None,
        ] {
            assert_eq!(
                next_version("1.2.3-dev", owes).as_deref(),
                Some("1.2.3"),
                "{owes:?}"
            );
        }
        assert_eq!(
            next_version("1.2.3-dev", VersionImpact::Breaking).as_deref(),
            Some("2.0.0")
        );
    }

    /// A scheme this does not understand gets no answer rather than a guess.
    ///
    /// ADR-0034 refused to tell a `CalVer` project that its date is a patch
    /// number, and this is where that refusal is enforced.
    #[test]
    fn a_version_this_does_not_understand_is_refused() {
        for v in [
            "2026.09",
            "2026.09.1",
            "1.2.3.4",
            "v1.2.3",
            "",
            "not-a-version",
        ] {
            assert_eq!(next_version(v, VersionImpact::Fix), None, "{v}");
        }
    }

    /// A window is a set, so its answer is the strongest member and there is no
    /// node's own floor in it (ADR-0038).
    #[test]
    fn a_window_owes_the_strongest_thing_in_it() {
        let floors = VersionFloors::default();
        let tasks = [
            t("W-1", None, false),
            t("W-2", Some(VersionImpact::Breaking), false),
            t("W-3", Some(VersionImpact::None), false),
        ];
        let owed = owed_among(tasks.iter(), floors);
        assert_eq!(owed.impact, VersionImpact::Breaking);
        assert_eq!(owed.raised_by.as_deref(), Some("W-2"));
        // An empty window owes nothing and blames nobody, which is the answer
        // for a release that closes no task.
        let none = owed_among(std::iter::empty(), floors);
        assert_eq!(none.impact, VersionImpact::None);
        assert_eq!(none.raised_by, None);
    }

    /// **`raised_by` names a declaration, never a floor** (`A1-313`).
    ///
    /// Seventeen ordinary tasks owe `fix` because seventeen floors say so.
    /// Naming whichever the walk reached first reads as an accusation of a task
    /// that claimed nothing, and it was doing exactly that until this was
    /// measured against this repository's own window.
    #[test]
    fn a_floor_blames_nobody() {
        let floors = VersionFloors::default();
        let tasks = [t("W-1", None, false), t("W-2", None, false)];
        let owed = owed_among(tasks.iter(), floors);
        assert_eq!(owed.impact, VersionImpact::Fix, "the task floor");
        assert_eq!(
            owed.raised_by, None,
            "no task declared this; the levels decided it"
        );
        // And a declaration at the same height as the floor still names itself,
        // because it *is* the thing that reached the answer.
        let declared = [t("W-1", Some(VersionImpact::Fix), false)];
        assert_eq!(
            owed_among(declared.iter(), floors).raised_by.as_deref(),
            Some("W-1")
        );
    }

    /// A dropped task cannot be smuggled into a window either (`A1-074`).
    #[test]
    fn a_dropped_task_owes_nothing_in_a_window_too() {
        let mut dropped = t("W-1", Some(VersionImpact::Breaking), false);
        dropped.status = crate::task::Status::Dropped;
        let tasks = [dropped];
        assert_eq!(
            owed_among(tasks.iter(), VersionFloors::default()).impact,
            VersionImpact::None
        );
    }

    /// The triple is the ordering, and the pre-release is deliberately not in
    /// it: `0.4.0-dev` and `0.4.0` are one release being prepared and shipped.
    #[test]
    fn the_triple_orders_releases_and_ignores_the_pre_release() {
        assert_eq!(triple("0.4.0-dev"), triple("0.4.0"));
        assert_eq!(triple("1.2.3+build.5"), Some((1, 2, 3)));
        assert!(triple("0.3.1") < triple("0.4.0"));
        // A leading zero is `CalVer`, and it parses too well to be left alone:
        // `2026.09.1` reads as `(2026, 9, 1)` and would be handed back spelled
        // `2026.9.1`, which is a different version.
        for v in [
            "2026.09",
            "2026.09.1",
            "1.02.3",
            "1.2.3.4",
            "v1.2.3",
            "",
            "not-a-version",
        ] {
            assert_eq!(triple(v), None, "{v}");
        }
        // The zero that is exactly zero still parses, which is most of this
        // project's own history.
        assert_eq!(triple("0.4.0"), Some((0, 4, 0)));
    }

    /// `none` leaves the version alone, which is what makes a plan edit or a
    /// pilot log cost nothing.
    #[test]
    fn owing_nothing_moves_nothing() {
        assert_eq!(
            next_version("1.2.3", VersionImpact::None).as_deref(),
            Some("1.2.3")
        );
    }
}
