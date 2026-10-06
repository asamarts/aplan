//! The roadmap view: the plan grouped by cut (the milestone meta-tasks,
//! ADR-0008), with per-cut rollups and critical path, plus the tasks in no cut.
//!
//! This is the model `aplan roadmap` renders and `gen-roadmap --check` gates
//! (ADR-0009). It is hermetic, it reads the plan and never git, and renders the
//! *cited* evidence, not its resolution (`aplan verify` owns resolution). It is
//! advisory: a plan is not wrong for being in progress.

use std::collections::BTreeSet;

use crate::graph::{CriticalPath, Graph};
use crate::task::{EvidenceKind, MILESTONE_TAG, Status, Task};

/// The whole plan as a roadmap: cuts in version order, then the cross-cutting
/// tasks that belong to no cut.
#[derive(Debug, Clone, PartialEq)]
pub struct Roadmap {
    pub cuts: Vec<Cut>,
    /// Effort days a week absorbs, if the repository declared one (`A1-147`,
    /// ADR-0023). Carried rather than folded in: a duration is *derived* at the
    /// point it is rendered, so no surface can print one that disagrees with the
    /// effort beside it.
    pub effort_days_per_week: Option<f64>,
    /// Real tasks that are a member of no cut (e.g. a cross-cutting quality gate).
    pub cross_cutting: Vec<RoadmapTask>,
    /// Sum of `effort_days` over those, live only, with the part already done
    /// (`A1-147`).
    ///
    /// Carried because the effort column has to add up to the plan. Measured on
    /// this repository when it did not: 21.0 of 185.5 effort days sat in 18
    /// cross-cutting tasks and appeared in no row, so a reader adding the column
    /// got 164.5 with nothing on the page to say why. Cuts plus this is the
    /// plan, and the header line says the total.
    pub cross_cutting_effort_days: f64,
    pub cross_cutting_done_effort_days: f64,
}

/// One release cut: a milestone meta-task and the tasks it groups.
#[derive(Debug, Clone, PartialEq)]
pub struct Cut {
    /// The cut version, e.g. `v0.3` (the `vN.N` tag on the milestone). Empty if
    /// the milestone carries no version tag.
    pub version: String,
    /// The milestone's title, the cut's theme.
    pub title: String,
    pub milestone_id: String,
    /// The milestone's own status (`done` only when every listed member is done).
    pub status: Status,
    /// Whether this is a *release*: a checkpoint over cuts rather than a cut
    /// (`A1-146`, ADR-0022). Its members are the work of the cuts it closes,
    /// borrowed for the rollup, so a consumer summing every cut's `total` must
    /// skip these or count that work twice.
    pub release: bool,
    /// Whether this row's members are **borrowed** from beneath it rather than
    /// its own, so a consumer summing rows must skip it or count that work
    /// twice (`A1-333`, ADR-0042).
    ///
    /// `release` used to answer this, and stopped being able to: a release
    /// borrows, and so does any milestone that declares `contains`, and the two
    /// are no longer the same set. Measured the moment containment worked -- a
    /// four-level plan with two real tasks reported `4 task(s), 10.0d` in the
    /// header, because the cut and the sub-cut both legitimately carried them.
    ///
    /// `release` keeps its other jobs: the label, `closes`, and which kind the
    /// row is. This one answers only the arithmetic question.
    pub borrows: bool,
    pub total: usize,
    pub done: usize,
    pub ready: usize,
    pub blocked: usize,
    /// Sum of `effort_days` over the cut's live members (`A1-147`). Members
    /// without an estimate contribute nothing, and `APLAN009` reports them.
    ///
    /// A **release** borrows the work of the cuts it closes, exactly as `total`
    /// does, so this is their effort and a consumer adding this up across cuts
    /// must skip the rows where `release` is set or count that work twice. The
    /// same trap `A1-146` documented for the task counts, one field along.
    pub effort_days: f64,
    /// Of that, the part already claimed `done`.
    pub done_effort_days: f64,
    /// The cuts a **release** closes, by milestone id, in the order it names
    /// them. Empty for a cut, which closes nothing.
    ///
    /// A release "is a checkpoint over cuts" (ADR-0022), and until this it had
    /// no way to say *which*: it carried its cuts' members instead, so every
    /// surface rendered the same task rows twice and a reader had to infer the
    /// relationship from the tags (`A1-240`).
    pub closes: Vec<String>,
    /// The cut's member tasks, in id order.
    ///
    /// For a **release** these are borrowed from the cuts it closes, for the
    /// rollup's sake (`A1-146`), and they are not this milestone's own work.
    /// Nothing may read them as "the tasks in this cut": `closes` is what a
    /// release holds.
    pub members: Vec<RoadmapTask>,
    /// The same members, under their group headings.
    ///
    /// Empty unless the repository named a grouping family
    /// (`roadmap.group_by`), so a plan that declares nothing renders exactly as
    /// it did before this existed. `members` stays the flat list either way, so
    /// a consumer that never asked for groups is unaffected (`A1-078`).
    pub groups: Vec<CutGroup>,
    /// The longest effort-weighted chain among the cut's members (its own spine).
    pub critical_path: CriticalPath,
}

/// A task as it appears in the roadmap: its claimed status and cited evidence,
/// never resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadmapTask {
    pub id: String,
    pub title: String,
    pub status: Status,
    pub blocked_by: Vec<String>,
    /// Cited evidence, rendered `kind:ref` (a `commit` ref is shortened; a `note`
    /// shows only its kind, since its text is free-form). Not resolved.
    pub evidence: Vec<String>,
    /// The task's tags, so the view can group by a declared family without a
    /// second pass over the plan (`A1-078`).
    pub tags: Vec<String>,
}

impl RoadmapTask {
    fn of(t: &Task) -> Self {
        let evidence = t
            .evidence
            .iter()
            .map(|e| {
                let kind = e.kind.as_str();
                match e.kind {
                    // A short prefix identifies the commit without a 40-char cell;
                    // this is display only (the roadmap never resolves it). Take
                    // 12 *chars*, not bytes: nothing validates a `ref` is ASCII
                    // hex, and byte-slicing mid-char would panic.
                    EvidenceKind::Commit => {
                        let short: String = e.reference.chars().take(12).collect();
                        format!("{kind}:{short}")
                    }
                    // A note's `ref` is free-form prose, possibly long; show the kind.
                    EvidenceKind::Note => kind.to_string(),
                    _ => format!("{kind}:{}", e.reference),
                }
            })
            .collect();
        RoadmapTask {
            id: t.id.clone(),
            title: t.title.clone(),
            status: t.status,
            blocked_by: t.blocked_by.clone(),
            evidence,
            tags: t.tags.clone(),
        }
    }
}

/// The label a cut reports itself under: its `vN.N` tag if it has one, else the
/// first tag that is not the `milestone` marker, else empty.
///
/// One function because two things read it (`A1-139`, found by its audit): this
/// module renders it, and `--cut` accepts it as a name. Two copies of the rule
/// would let a plan render one label and answer to another, which is the drift
/// the shared resolver was built to end.
///
/// A project that organises by `phase-2` or `s3` is not missing a version, it
/// has a different vocabulary, and `?` reads as "the tool does not understand my
/// project" (`A1-037`).
#[must_use]
pub fn cut_label(t: &Task) -> &str {
    t.tags
        .iter()
        .find(|tag| parse_cut_version(tag).is_some())
        .or_else(|| t.tags.iter().find(|tag| tag.as_str() != MILESTONE_TAG))
        .map_or("", String::as_str)
}

/// Parse a `vN.N` cut tag into `(major, minor)` for numeric ordering, so `v0.10`
/// sorts *after* `v0.9`. `None` for a tag that is not a cut version.
pub fn parse_cut_version(tag: &str) -> Option<(u32, u32)> {
    let (major, minor) = tag.strip_prefix('v')?.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Members: the milestone's blockers that exist and are not themselves
/// milestones (a milestone is never its own member).
///
/// A **release** closes cuts rather than holding work (`A1-146`,
/// ADR-0022), so its members are the work of the cuts it names. Read
/// one level through, not recursively: a release of releases is not a
/// shape the model has, and the rollup would then double-count.
/// Without this a release rendered `0/0 done` -- true of its own
/// `blocked_by` and useless as an answer to "how is v0.1 going".
/// "What this cut lists" is a membership edge, asked for by name
/// rather than re-derived from whether the tail is a milestone
/// (`A1-216`). A release lists cuts, so its members are its cuts'
/// members.
fn members_of<'a>(graph: &'a Graph, m: &'a Task) -> BTreeSet<&'a str> {
    let mut out = BTreeSet::new();
    let mut seen = BTreeSet::new();
    collect_members(graph, m, &mut out, &mut seen);
    out
}

/// The work beneath a milestone, at any depth, each task once (ADR-0042).
///
/// Two edges reach a child milestone and they mean different things. `contains`
/// is containment, and its work rolls up here. A milestone in `blocked_by`
/// *sequences* the two cuts (ADR-0013) and its work is its own -- which is why
/// this walk descends the first and not the second, and why a plan written
/// before `contains` existed rolls up exactly as it did.
///
/// **A release descends its `blocked_by` milestones too**, which is ADR-0022's
/// "read one level through" kept for compatibility: a release written before
/// this ADR names its cuts there and must keep working. Declaring `contains` is
/// the way to say it now, and the two are unioned rather than chosen between,
/// so a release part-way through the migration is not half-counted.
///
/// `seen` is what makes ADR-0022's double-counting worry unnecessary: a
/// descendant reachable by two routes is visited once, so the recursion that
/// supersedes "one level through" cannot count it twice.
fn collect_members<'a>(
    graph: &'a Graph,
    m: &'a Task,
    out: &mut BTreeSet<&'a str>,
    seen: &mut BTreeSet<&'a str>,
) {
    if !seen.insert(m.id.as_str()) {
        return;
    }
    for b in &m.blocked_by {
        if graph
            .get(b)
            .is_some_and(|t| crate::edge_kind(t, m) == crate::EdgeKind::Membership)
        {
            out.insert(b.as_str());
        }
    }
    let descend = m.contains.iter().chain(
        m.is_release()
            .then(|| m.blocked_by.iter())
            .into_iter()
            .flatten(),
    );
    for child in descend {
        if let Some(c) = graph.get(child)
            && c.is_milestone()
        {
            collect_members(graph, c, out, seen);
        }
    }
}

/// The cuts a release closes, by milestone id, in the order it names them.
///
/// Asked of the model rather than inferred from the members a release borrows
/// for its rollup: those are the *cuts'* work, and reading them as the release's
/// own is what made every surface repeat them (`A1-240`).
fn closed_by(graph: &Graph, m: &Task) -> Vec<String> {
    if !m.is_release() {
        return Vec::new();
    }
    m.blocked_by
        .iter()
        .filter(|b| graph.get(b).is_some_and(Task::is_milestone))
        .cloned()
        .collect()
}

impl Roadmap {
    /// Attach the repository's declared throughput, so every surface can derive
    /// a duration from the effort it is already printing (`A1-147`, ADR-0023).
    #[must_use]
    pub fn with_throughput(mut self, effort_days_per_week: Option<f64>) -> Self {
        self.effort_days_per_week = effort_days_per_week;
        self
    }

    /// This cut's weeks of work at the declared throughput, and the part of them
    /// still to do. `(None, None)` when the repository declared no throughput.
    #[must_use]
    pub fn weeks_of(&self, cut: &Cut) -> (Option<f64>, Option<f64>) {
        self.weeks(cut.effort_days, cut.done_effort_days)
    }

    /// The same for the tasks in no cut.
    #[must_use]
    pub fn cross_cutting_weeks(&self) -> (Option<f64>, Option<f64>) {
        self.weeks(
            self.cross_cutting_effort_days,
            self.cross_cutting_done_effort_days,
        )
    }

    /// The plan's own effort: every cut plus the tasks in none, with releases
    /// skipped because they borrow the work of the cuts they close (ADR-0022),
    /// as is any row that borrows from beneath it (`A1-333`, ADR-0042).
    /// This is what the effort column adds up to, and it agrees with `summary`.
    #[must_use]
    pub fn total_effort_days(&self) -> f64 {
        self.cuts
            .iter()
            .filter(|c| !c.borrows)
            .map(|c| c.effort_days)
            .sum::<f64>()
            + self.cross_cutting_effort_days
    }

    fn weeks(&self, effort: f64, done: f64) -> (Option<f64>, Option<f64>) {
        (
            crate::summary::weeks_of_work(effort, self.effort_days_per_week),
            crate::summary::weeks_of_work(effort - done, self.effort_days_per_week),
        )
    }

    /// Group each cut's members by the declared family's members.
    ///
    /// Headings follow the order the repository declared, because that is the
    /// author's intent and any other order would be one nobody chose. Tasks
    /// carrying no member of the family land in a final unlabelled group:
    /// rendering them under a heading they do not have would be a lie, and
    /// dropping them would be worse (`A1-078`).
    #[must_use]
    pub fn grouped_by(mut self, family_members: &[String]) -> Self {
        for cut in &mut self.cuts {
            let mut groups: Vec<CutGroup> = Vec::new();
            for label in family_members {
                let members: Vec<RoadmapTask> = cut
                    .members
                    .iter()
                    .filter(|m| m.tags.iter().any(|t| t == label))
                    .cloned()
                    .collect();
                if !members.is_empty() {
                    groups.push(CutGroup {
                        label: Some(label.clone()),
                        members,
                    });
                }
            }
            let ungrouped: Vec<RoadmapTask> = cut
                .members
                .iter()
                .filter(|m| !family_members.iter().any(|l| m.tags.iter().any(|t| t == l)))
                .cloned()
                .collect();
            if !ungrouped.is_empty() {
                groups.push(CutGroup {
                    label: None,
                    members: ungrouped,
                });
            }
            cut.groups = groups;
        }
        self
    }
}

/// Compare two labels the way a reader does, so a run of digits sorts by value
/// rather than by character: `s2` before `s10`, `phase-2` before `phase-10`.
///
/// An order nobody can predict is worse than an obvious one, and plain string
/// order is the one that surprises: it is why `v0.10` needed the numeric parse
/// above, and a project numbering its own cuts hits the same wall at ten.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(cx), Some(cy)) => {
                if cx.is_ascii_digit() && cy.is_ascii_digit() {
                    let nx: String =
                        std::iter::from_fn(|| x.next_if(char::is_ascii_digit)).collect();
                    let ny: String =
                        std::iter::from_fn(|| y.next_if(char::is_ascii_digit)).collect();
                    // Both are runs of digits, so a parse failure means a run
                    // longer than u128; fall back to length then text.
                    let ord = match (nx.parse::<u128>(), ny.parse::<u128>()) {
                        (Ok(vx), Ok(vy)) => vx.cmp(&vy),
                        _ => nx.len().cmp(&ny.len()).then_with(|| nx.cmp(&ny)),
                    };
                    if ord != std::cmp::Ordering::Equal {
                        return ord;
                    }
                } else {
                    x.next();
                    y.next();
                    let ord = cx.cmp(&cy);
                    if ord != std::cmp::Ordering::Equal {
                        return ord;
                    }
                }
            }
        }
    }
}

/// A cut's members under one heading of the grouping family.
#[derive(Debug, Clone, PartialEq)]
pub struct CutGroup {
    /// The tag that names this group, or `None` for the tasks carrying none.
    pub label: Option<String>,
    pub members: Vec<RoadmapTask>,
}

/// Effort over tasks that can still happen, and the part of it already `done`
/// (`A1-147`).
///
/// `dropped` is excluded here as it is everywhere else a rollup counts: a
/// cancelled task is not time, and a total that quietly included it reported
/// pre-cancellation scope forever (`A1-074`). A task with no estimate
/// contributes nothing rather than a guess, and `APLAN009` is what says so.
fn live_effort(tasks: &[&Task]) -> (f64, f64) {
    let (mut effort, mut done) = (0.0, 0.0);
    for t in tasks {
        if t.status == Status::Dropped {
            continue;
        }
        let days = t.effort_days.unwrap_or(0.0);
        effort += days;
        if t.status == Status::Done {
            done += days;
        }
    }
    (effort, done)
}

impl Graph<'_> {
    /// Group the plan by cut for the roadmap view. Cuts are the milestone
    /// meta-tasks (ADR-0008), ordered by their `vN.N` tag parsed *numerically*
    /// (unversioned milestones sort last, by id); each cut's members are its
    /// milestone's `blocked_by` tasks (the milestone is excluded from its own
    /// member list). Tasks in no cut are collected as cross-cutting. Fully
    /// deterministic: members iterate in id order, cuts by `(version, id)`.
    pub fn roadmap(&self) -> Roadmap {
        let mut claimed: BTreeSet<&str> = BTreeSet::new();
        let mut cuts: Vec<Cut> = Vec::new();

        // **One walk for every bounded row** (`A1-345`, audit). The chain that
        // ends at a borrowing row is a trace back through a whole-plan DP, and
        // the DP is the same for all of them: asking per row cost this report
        // 472ms against 214ms on a 4,870-task plan with ten releases. Empty
        // where the walk is cyclic, which is a plan `check` rejects.
        let borrowing: BTreeSet<&str> = self
            .by_id
            .iter()
            .filter(|(_, m)| m.is_milestone() && (m.is_release() || !m.contains.is_empty()))
            .map(|(id, _)| *id)
            .collect();
        let bounded = self.chains_to(&borrowing).unwrap_or_default();

        for (&id, &m) in &self.by_id {
            if !m.is_milestone() {
                continue;
            }
            // The cut's own name for itself. `vN.N` first, so a plan that uses
            // release versions keeps exactly the label it had; otherwise the
            // first tag that is not the `milestone` marker, because a project
            // that organises by `phase-2` or `s3` is not missing a version, it
            // has a different vocabulary and `?` reads as "the tool does not
            // understand my project" (`A1-037`).
            let version = cut_label(m).to_string();

            let member_ids = members_of(self, m);

            // The rollup and critical path count only members that can still
            // happen: a `dropped` member is shown in the table (for transparency)
            // but excluded from `total`/`done`/`ready`/`blocked` and from the
            // path, exactly as the whole-plan `critical_path` excludes dropped, so
            // `done + ready + blocked == total` always reconciles.
            let mut members = Vec::with_capacity(member_ids.len());
            let (mut total, mut done, mut ready, mut blocked) = (0usize, 0usize, 0usize, 0usize);
            let mut active: BTreeSet<&str> = BTreeSet::new();
            // Collected so the effort comes from `live_effort` here too: what
            // counts as a cut's time and what counts as the plan's must be one
            // definition, or they drift the way the numbers they summarise do.
            let mut member_tasks: Vec<&Task> = Vec::with_capacity(member_ids.len());
            for &mid in &member_ids {
                // A release borrows its cuts' work for the rollup, and marking it
                // claimed here changes nothing: the cut it lives in claims it in
                // its own turn, and `claimed` is a set. Said rather than branched
                // on, because the branch read as though it mattered (`A1-146`).
                claimed.insert(mid);
                let t = self.get(mid).expect("member exists (filtered above)");
                member_tasks.push(t);
                if t.status != Status::Dropped {
                    total += 1;
                    active.insert(mid);
                    if t.status == Status::Done {
                        done += 1;
                    } else if t.status.is_workable()
                        && self.blockers_of(&t.id).iter().all(|b| !self.blocks(t, b))
                    {
                        ready += 1;
                    } else {
                        blocked += 1;
                    }
                }
                members.push(RoadmapTask::of(t));
            }
            let (effort_days, done_effort_days) = live_effort(&member_tasks);

            // **A borrowing row's `crit` is the chain that ends at it**
            // (`A1-345`). Over its members it was the longest chain among the
            // work it borrows, which excludes the zero-effort milestone nodes
            // carrying cut order -- so on the sixth pilot's plan the release row
            // said `82.0d` while the chain that actually controls the release ran
            // through cuts it does not hold. Two quantities under one word, and
            // the smaller one was the one on the page.
            //
            // Keyed on `borrows`, not on `release`: a container declaring
            // `contains` holds the same relationship to the work under it
            // (ADR-0042), and keying on the narrower word would leave the
            // defect in place for every plan that migrated to the new field.
            //
            // A *cut*'s number stays the member-induced one, because that is the
            // question a cut's row asks: how much work is in this bucket. A row
            // that borrows holds no work of its own, so the only chain it has is
            // the one leading to it.
            let borrows = m.is_release() || !m.contains.is_empty();
            let critical_path = if borrows {
                bounded.get(id).cloned()
            } else {
                self.critical_path_within(&active)
            }
            .unwrap_or(CriticalPath {
                total_effort_days: 0.0,
                tasks: Vec::new(),
            });

            let closes = closed_by(self, m);

            cuts.push(Cut {
                version,
                title: m.title.clone(),
                milestone_id: id.to_string(),
                status: m.status,
                release: m.is_release(),
                borrows,
                closes,
                total,
                done,
                ready,
                blocked,
                effort_days,
                done_effort_days,
                members,
                critical_path,
                groups: Vec::new(),
            });
        }

        cuts.sort_by(|a, b| {
            match (parse_cut_version(&a.version), parse_cut_version(&b.version)) {
                (Some(x), Some(y)) => x.cmp(&y),
                // A versioned cut sorts before an unversioned one.
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                // Neither is a release version, so compare the labels the way a
                // reader does: `s2` before `s10`, `phase-2` before `phase-10`.
                // Ordering by id was the old fallback and put `s10` second
                // (`A1-037`).
                (None, None) => natural_cmp(&a.version, &b.version),
            }
            .then_with(|| a.milestone_id.cmp(&b.milestone_id))
        });

        let loose: Vec<&Task> = self
            .by_id
            .iter()
            .filter(|(id, t)| !t.is_milestone() && !claimed.contains(*id))
            .map(|(_, t)| *t)
            .collect();
        let (cross_cutting_effort_days, cross_cutting_done_effort_days) = live_effort(&loose);
        let cross_cutting = loose.iter().map(|t| RoadmapTask::of(t)).collect();

        Roadmap {
            cuts,
            cross_cutting,
            cross_cutting_effort_days,
            cross_cutting_done_effort_days,
            // Filled in by `with_throughput`, for the same reason as `summary`:
            // the graph knows the plan, not the repository's configuration.
            effort_days_per_week: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Evidence;

    fn task(
        id: &str,
        status: Status,
        effort: Option<f64>,
        blocked_by: &[&str],
        tags: &[&str],
    ) -> Task {
        Task {
            id: id.into(),
            title: format!("{id} title"),
            phase: 0,
            status,
            owner: None,
            effort_days: effort,
            blocked_by: blocked_by.iter().map(|s| (*s).to_string()).collect(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: tags.iter().map(|s| (*s).to_string()).collect(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    /// A cut's effort is its live members', and its duration is that effort
    /// divided by the declared throughput -- computed here, stored nowhere
    /// (`A1-147`, ADR-0023).
    /// Proves the named behaviour `rollup.carries_effort` (`A1-211`).
    #[test]
    fn a_cut_carries_the_effort_of_its_live_members() {
        let mut m = task(
            "M",
            Status::Todo,
            None,
            &["A", "B", "C"],
            &["milestone", "v0.9"],
        );
        m.title = "A cut".into();
        let a = task("A", Status::Done, Some(4.0), &[], &[]);
        let b = task("B", Status::Todo, Some(6.0), &[], &[]);
        // Cancelled work is not time, and a member with no estimate contributes
        // nothing rather than a guess (`APLAN009` reports it).
        let gone = task("C", Status::Dropped, Some(90.0), &[], &[]);
        let noestimate = task("D", Status::Todo, None, &[], &["cross-cutting"]);
        let r = Graph::new([&m, &a, &b, &gone, &noestimate])
            .roadmap()
            .with_throughput(Some(5.0));
        let cut = &r.cuts[0];
        assert!(
            (cut.effort_days - 10.0).abs() < 1e-9,
            "4 done + 6 left, and not the dropped 90: {}",
            cut.effort_days
        );
        assert!((cut.done_effort_days - 4.0).abs() < 1e-9);
        let (weeks, left) = r.weeks_of(cut);
        assert!((weeks.expect("declared") - 2.0).abs() < 1e-9);
        assert!((left.expect("declared") - 1.2).abs() < 1e-9);
    }

    /// Undeclared throughput means no duration anywhere, rather than a zero a
    /// reader would take for "no work left".
    #[test]
    fn without_a_declared_throughput_a_cut_has_no_duration() {
        let m = task("M", Status::Todo, None, &["A"], &["milestone", "v0.9"]);
        let a = task("A", Status::Todo, Some(4.0), &[], &[]);
        let r = Graph::new([&m, &a]).roadmap();
        assert_eq!(r.effort_days_per_week, None);
        assert!(
            (r.cuts[0].effort_days - 4.0).abs() < 1e-9,
            "the effort is known either way"
        );
        assert_eq!(r.weeks_of(&r.cuts[0]), (None, None));
    }

    /// The effort on the page is the effort in the plan (`A1-147`).
    ///
    /// This is the invariant the audit found broken: cuts held 164.5 of this
    /// repository's 185.5 effort days and the other 21.0 sat in tasks belonging
    /// to no cut, invisible on every `roadmap` surface. Asserted against
    /// `summary`, which is the other rollup a reader compares it to, on a plan
    /// carrying all four shapes at once: a cut, a release borrowing it, work in
    /// no cut, and a cancelled task that is nobody's time.
    #[test]
    fn the_effort_column_adds_up_to_the_plan() {
        let rel = task(
            "R",
            Status::Todo,
            None,
            &["M1"],
            &["milestone", "release", "v1.0"],
        );
        let m1 = task("M1", Status::Todo, None, &["A"], &["milestone", "v0.1"]);
        let a = task("A", Status::Done, Some(4.0), &[], &[]);
        let loose = task("L", Status::Todo, Some(3.0), &[], &["cross-cutting"]);
        let gone = task("G", Status::Dropped, Some(90.0), &[], &["cross-cutting"]);
        let tasks = [&rel, &m1, &a, &loose, &gone];

        let r = Graph::new(tasks).roadmap().with_throughput(Some(7.0));
        let s = Graph::new(tasks).summary();
        assert!(
            (r.total_effort_days() - s.total_effort_days()).abs() < 1e-9,
            "roadmap says {}, summary says {}",
            r.total_effort_days(),
            s.total_effort_days()
        );
        assert!(
            (r.cross_cutting_effort_days - 3.0).abs() < 1e-9,
            "the dropped 90 is not time"
        );
        assert_eq!(
            r.cross_cutting_weeks().0.map(|w| (w * 100.0).round()),
            Some(43.0)
        );
    }

    /// A release's effort is the work of the cuts it closes, which is the same
    /// borrowing `A1-146` gave `total` -- and the same double count for anyone
    /// summing the column (`A1-147`).
    /// Proves the named behaviour `milestone.release_holds_cuts` (`A1-211`).
    #[test]
    fn a_release_borrows_the_effort_of_its_cuts() {
        let rel = task(
            "R",
            Status::Todo,
            None,
            &["M1", "M2"],
            &["milestone", "release", "v1.0"],
        );
        let m1 = task("M1", Status::Todo, None, &["A"], &["milestone", "v0.1"]);
        let m2 = task("M2", Status::Todo, None, &["B"], &["milestone", "v0.2"]);
        let a = task("A", Status::Done, Some(4.0), &[], &[]);
        let b = task("B", Status::Todo, Some(6.0), &[], &[]);
        let r = Graph::new([&rel, &m1, &m2, &a, &b])
            .roadmap()
            .with_throughput(Some(5.0));

        let release = r.cuts.iter().find(|c| c.release).expect("the release");
        assert!(
            (release.effort_days - 10.0).abs() < 1e-9,
            "the release holds its cuts' work: {}",
            release.effort_days
        );
        assert!((release.done_effort_days - 4.0).abs() < 1e-9);

        // The plan has 10 effort days in it, and adding every row up says 20.
        // `release` is the flag that makes the difference tellable, which is why
        // it is on all three surfaces.
        let every_row: f64 = r.cuts.iter().map(|c| c.effort_days).sum();
        let cuts_only: f64 = r
            .cuts
            .iter()
            .filter(|c| !c.release)
            .map(|c| c.effort_days)
            .sum();
        assert!(
            (every_row - 20.0).abs() < 1e-9,
            "double counted, as documented"
        );
        assert!(
            (cuts_only - 10.0).abs() < 1e-9,
            "skipping releases is the answer"
        );
    }

    /// The sixth pilot's second drift: an effort total that implied a two-person
    /// team in a one-person project (pre-public adoption evidence item 2).
    ///
    /// It is a claim about throughput, and this is where it becomes visible: the
    /// same plan under two declared capacities gives two durations, and the
    /// capacity is the one number a repository has to state and own. Nothing here
    /// asserts the declaration is *true* -- no tool can -- but it can no longer
    /// hide inside a schedule that was arrived at by dividing by it silently.
    #[test]
    fn the_throughput_is_the_only_assumption_and_it_is_visible() {
        let m = task("M", Status::Todo, None, &["A"], &["milestone", "v0.9"]);
        let a = task("A", Status::Todo, Some(20.0), &[], &[]);
        let plan = Graph::new([&m, &a]).roadmap();
        let one = plan.clone().with_throughput(Some(5.0));
        let two = plan.with_throughput(Some(10.0));
        assert!((one.weeks_of(&one.cuts[0]).0.expect("declared") - 4.0).abs() < 1e-9);
        assert!((two.weeks_of(&two.cuts[0]).0.expect("declared") - 2.0).abs() < 1e-9);
    }

    fn tagged(id: &str, tags: &[&str]) -> Task {
        task(id, Status::Todo, Some(1.0), &[], tags)
    }

    #[test]
    fn a_cut_groups_by_the_declared_family_in_declared_order() {
        // Headings follow the order the repository declared, because that is the
        // author's intent; any other order is one nobody chose (`A1-078`).
        let tasks = [
            tagged("T-01", &["epic-b"]),
            tagged("T-02", &["epic-a"]),
            tagged("T-03", &[]),
            task(
                "M-s0",
                Status::Todo,
                None,
                &["T-01", "T-02", "T-03"],
                &["milestone", "s0"],
            ),
        ];
        let g = Graph::new(tasks.iter());
        let family = vec!["epic-a".to_string(), "epic-b".to_string()];
        let r = g.roadmap().grouped_by(&family);
        let cut = &r.cuts[0];

        let labels: Vec<Option<&str>> = cut.groups.iter().map(|g| g.label.as_deref()).collect();
        assert_eq!(labels, vec![Some("epic-a"), Some("epic-b"), None]);
        assert_eq!(cut.groups[0].members[0].id, "T-02");
        // A task carrying no member of the family is shown, not dropped:
        // rendering it under a heading it does not have would be a lie.
        assert_eq!(cut.groups[2].members[0].id, "T-03");
        // The flat list is untouched, so a consumer that never asked for groups
        // sees exactly what it saw before.
        assert_eq!(cut.members.len(), 3);
    }

    #[test]
    fn an_ungrouped_plan_has_no_groups_at_all() {
        // The default, and this repository's own roadmap: byte-identical output.
        let tasks = [
            tagged("T-01", &["epic-a"]),
            task("M-s0", Status::Todo, None, &["T-01"], &["milestone", "s0"]),
        ];
        let g = Graph::new(tasks.iter());
        assert!(g.roadmap().cuts[0].groups.is_empty());
    }

    #[test]
    fn a_group_with_no_members_is_not_rendered_as_an_empty_heading() {
        // A family lists every epic the project has; a given cut uses a few.
        let tasks = [
            tagged("T-01", &["epic-b"]),
            task("M-s0", Status::Todo, None, &["T-01"], &["milestone", "s0"]),
        ];
        let g = Graph::new(tasks.iter());
        let family = vec![
            "epic-a".to_string(),
            "epic-b".to_string(),
            "epic-c".to_string(),
        ];
        let r = g.roadmap().grouped_by(&family);
        let labels: Vec<Option<&str>> = r.cuts[0]
            .groups
            .iter()
            .map(|g| g.label.as_deref())
            .collect();
        assert_eq!(labels, vec![Some("epic-b")], "no empty headings");
    }

    #[test]
    fn a_task_in_two_groups_appears_under_each() {
        // Tags are a vocabulary, not a containment (ADR-0014): nothing says a
        // task carries one member of a family, so the view must not pretend it
        // does by silently choosing one.
        let tasks = [
            tagged("T-01", &["epic-a", "epic-b"]),
            task("M-s0", Status::Todo, None, &["T-01"], &["milestone", "s0"]),
        ];
        let g = Graph::new(tasks.iter());
        let family = vec!["epic-a".to_string(), "epic-b".to_string()];
        let r = g.roadmap().grouped_by(&family);
        assert_eq!(r.cuts[0].groups.len(), 2);
        assert_eq!(r.cuts[0].groups[0].members[0].id, "T-01");
        assert_eq!(r.cuts[0].groups[1].members[0].id, "T-01");
    }

    #[test]
    fn a_cut_that_is_not_a_release_version_keeps_its_own_label() {
        // Two pilots, two non-release vocabularies (`phase-N` and `sN`), the
        // same `?`. The second occurrence inverts the reading of the first:
        // `vN.N` is the unusual case among the projects adopting (`A1-037`).
        let tasks = [
            task("A1", Status::Todo, Some(1.0), &[], &[]),
            task("M-s0", Status::Todo, None, &["A1"], &["milestone", "s0"]),
        ];
        let g = Graph::new(tasks.iter());
        let r = g.roadmap();
        assert_eq!(r.cuts[0].version, "s0", "the cut's own name for itself");
    }

    #[test]
    fn cuts_order_by_number_not_by_text() {
        // An order nobody can predict is worse than an obvious one, and plain
        // string order is the one that surprises: it puts `s10` second.
        let tasks = [
            task("A1", Status::Todo, Some(1.0), &[], &[]),
            task("M-s2", Status::Todo, None, &["A1"], &["milestone", "s2"]),
            task("M-s10", Status::Todo, None, &["A1"], &["milestone", "s10"]),
            task("M-s1", Status::Todo, None, &["A1"], &["milestone", "s1"]),
        ];
        let g = Graph::new(tasks.iter());
        let r = g.roadmap();
        let labels: Vec<&str> = r.cuts.iter().map(|c| c.version.as_str()).collect();
        assert_eq!(labels, vec!["s1", "s2", "s10"]);
    }

    #[test]
    fn natural_order_is_a_consistent_total_order() {
        use std::cmp::Ordering::{Equal, Greater, Less};
        // The comparison feeds a sort, so an inconsistent one is a panic waiting
        // for the right input, not merely a surprising order.
        assert_eq!(natural_cmp("s2", "s10"), Less);
        assert_eq!(natural_cmp("s100", "s99"), Greater);
        assert_eq!(natural_cmp("phase-2", "phase-10"), Less);
        assert_eq!(natural_cmp("s01", "s1"), Equal, "numerically equal");
        assert_eq!(natural_cmp("", "a"), Less);
        // A run of digits longer than `u128` must still order, not panic.
        assert_eq!(natural_cmp(&format!("s{}", "9".repeat(60)), "s2"), Greater);
        let words = [
            "s0", "s1", "s2", "s10", "phase-1", "v0.9", "v0.10", "", "s01",
        ];
        for a in words {
            for b in words {
                assert_eq!(
                    natural_cmp(a, b),
                    natural_cmp(b, a).reverse(),
                    "antisymmetry: {a:?} {b:?}"
                );
            }
        }
    }

    #[test]
    fn a_release_versioned_plan_orders_exactly_as_before() {
        // `v0.10` after `v0.9` is the case the numeric parse exists for, and
        // nothing about it may move.
        let tasks = [
            task("A1", Status::Todo, Some(1.0), &[], &[]),
            task("M-b", Status::Todo, None, &["A1"], &["milestone", "v0.10"]),
            task("M-a", Status::Todo, None, &["A1"], &["milestone", "v0.9"]),
        ];
        let g = Graph::new(tasks.iter());
        let r = g.roadmap();
        let labels: Vec<&str> = r.cuts.iter().map(|c| c.version.as_str()).collect();
        assert_eq!(labels, vec!["v0.9", "v0.10"]);
    }

    #[test]
    fn a_cuts_rollup_agrees_with_ready() {
        // The rollup counted a member of a sequenced cut as ready while `ready`
        // did not list it. Two answers to one question is the drift this tool
        // exists to end, and it was in the tool.
        let mut m0 = task("M-s0", Status::Todo, None, &["A1"], &["milestone", "s0"]);
        m0.tags = vec!["milestone".into(), "s0".into()];
        let m1 = task(
            "M-s1",
            Status::Todo,
            None,
            &["B1", "M-s0"],
            &["milestone", "s1"],
        );
        let tasks = [
            task("A1", Status::Todo, Some(1.0), &[], &[]),
            task("B1", Status::Todo, Some(1.0), &[], &[]),
            m0,
            m1,
        ];
        let g = Graph::new(tasks.iter());
        let r = g.roadmap();
        let ready_ids: Vec<&str> = g.ready().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ready_ids, vec!["A1"]);
        let s1 = r.cuts.iter().find(|c| c.version == "s1").expect("cut s1");
        assert_eq!(s1.ready, 0, "cut 1 has no ready work until cut 0 is done");
        assert_eq!(s1.blocked, 1);
    }

    #[test]
    fn parse_cut_version_is_numeric_not_lexicographic() {
        assert_eq!(parse_cut_version("v0.5"), Some((0, 5)));
        assert_eq!(parse_cut_version("v0.10"), Some((0, 10)));
        assert!(
            parse_cut_version("v0.10") > parse_cut_version("v0.9"),
            "v0.10 must sort after v0.9"
        );
        assert_eq!(parse_cut_version("milestone"), None);
        assert_eq!(parse_cut_version("v1"), None);
    }

    #[test]
    fn roadmap_groups_by_cut_orders_numerically_and_rolls_up() {
        let mut a1 = task("A1", Status::Done, Some(1.0), &[], &[]);
        a1.evidence = vec![Evidence {
            kind: EvidenceKind::Commit,
            reference: "9eabb17ce7b19c49".into(),
            captured: None,
        }];
        let a2 = task("A2", Status::Todo, Some(2.0), &["A1"], &[]); // ready: A1 done
        let a3 = task("A3", Status::Todo, Some(3.0), &[], &[]);
        let m1 = task(
            "M-v0.1",
            Status::Done,
            None,
            &["A1"],
            &["milestone", "v0.1"],
        );
        let m2 = task(
            "M-v0.2",
            Status::Todo,
            None,
            &["A2"],
            &["milestone", "v0.2"],
        );
        // Authored out of order, two-digit minor: proves numeric (not string) sort.
        let m10 = task(
            "M-v0.10",
            Status::Todo,
            None,
            &["A3"],
            &["milestone", "v0.10"],
        );
        let x = task("X", Status::Todo, Some(1.0), &[], &[]); // in no cut

        let r = Graph::new([&a1, &a2, &a3, &m1, &m2, &m10, &x]).roadmap();

        let versions: Vec<&str> = r.cuts.iter().map(|c| c.version.as_str()).collect();
        assert_eq!(
            versions,
            vec!["v0.1", "v0.2", "v0.10"],
            "numeric version order"
        );

        assert_eq!((r.cuts[0].total, r.cuts[0].done), (1, 1));
        assert_eq!(r.cuts[0].status, Status::Done);
        assert!((r.cuts[0].critical_path.total_effort_days - 1.0).abs() < 1e-9);
        assert_eq!(
            (r.cuts[1].total, r.cuts[1].done, r.cuts[1].ready),
            (1, 0, 1)
        );

        let cross: Vec<&str> = r.cross_cutting.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(cross, vec!["X"], "only the task in no cut is cross-cutting");
    }

    #[test]
    fn roadmap_task_shortens_a_commit_and_hides_note_text() {
        let mut t = task("A", Status::Done, Some(1.0), &[], &[]);
        t.evidence = vec![
            Evidence {
                kind: EvidenceKind::Commit,
                reference: "0123456789abcdef0123".into(),
                captured: None,
            },
            Evidence {
                kind: EvidenceKind::Note,
                reference: "a long free-form note".into(),
                captured: Some("2026-07-01".into()),
            },
        ];
        assert_eq!(
            RoadmapTask::of(&t).evidence,
            vec!["commit:0123456789ab".to_string(), "note".to_string()]
        );
    }

    #[test]
    fn roadmap_task_shortens_a_non_ascii_ref_without_panicking() {
        // Nothing validates a commit `ref` is ASCII hex, so the shortening must
        // take 12 *chars*, not bytes: here a multibyte char straddles byte 12,
        // which a byte slice would panic on (regression for a real crash).
        let mut t = task("A", Status::Done, Some(1.0), &[], &[]);
        t.evidence = vec![Evidence {
            kind: EvidenceKind::Commit,
            reference: "12345678901\u{e9}234".into(),
            captured: None,
        }];
        assert_eq!(
            RoadmapTask::of(&t).evidence,
            vec!["commit:12345678901\u{e9}".to_string()]
        );
    }

    #[test]
    fn roadmap_excludes_dropped_members_from_the_rollup_and_path() {
        // A dropped member is shown in the member table but excluded from the
        // rollup and the critical path (mirroring the whole-plan `critical_path`),
        // so `done + ready + blocked == total` reconciles and a huge dropped
        // effort does not inflate the cut's spine.
        let a = task("A", Status::Done, Some(1.0), &[], &[]);
        let z = task("Z", Status::Dropped, Some(100.0), &[], &[]);
        let m = task(
            "M-v0.1",
            Status::Todo,
            None,
            &["A", "Z"],
            &["milestone", "v0.1"],
        );
        let r = Graph::new([&a, &z, &m]).roadmap();
        let cut = &r.cuts[0];

        assert_eq!((cut.total, cut.done, cut.ready, cut.blocked), (1, 1, 0, 0));
        assert_eq!(cut.done + cut.ready + cut.blocked, cut.total);
        assert!(
            (cut.critical_path.total_effort_days - 1.0).abs() < 1e-9,
            "the 100d dropped member must not be on the path"
        );
        let ids: Vec<&str> = cut.members.iter().map(|t| t.id.as_str()).collect();
        assert!(
            ids.contains(&"Z"),
            "the dropped member is still shown in the table"
        );
    }
}

#[cfg(test)]
mod release_tests {
    use crate::graph::Graph;
    use crate::task::{Status, Task};

    fn task(id: &str, blocked_by: &[&str], tags: &[&str], effort: Option<f64>) -> Task {
        Task {
            id: id.into(),
            title: id.into(),
            phase: 0,
            status: Status::Todo,
            owner: None,
            effort_days: effort,
            blocked_by: blocked_by.iter().map(|s| (*s).to_string()).collect(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: tags.iter().map(|s| (*s).to_string()).collect(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    /// A release says which cuts it closes, and a cut closes nothing.
    ///
    /// Until this it had no way to say. It carried its cuts' members instead --
    /// which the rollup above needs and every *reader* then had to interpret, so
    /// the roadmap rendered those rows twice and `cut_of` reported the release as
    /// the cut a task is in (`A1-240`). The test above was right about what it
    /// checked; nobody asked what the borrowed list meant to anything else.
    #[test]
    fn a_release_names_the_cuts_it_closes_and_a_cut_names_none() {
        let plan = [
            task("A-1", &[], &[], Some(1.0)),
            task("B-1", &[], &[], Some(2.0)),
            task("M-s0", &["A-1"], &["milestone", "s0"], None),
            // Sequenced after the first cut (ADR-0013), so this cut has a
            // milestone among its blockers. Without that the fixture cannot
            // tell "a release names its cuts" from "any milestone names the
            // milestones it waits on", and the difference is the whole point.
            task("M-s1", &["B-1", "M-s0"], &["milestone", "s1"], None),
            task(
                "R-v0.1",
                &["M-s0", "M-s1"],
                &["milestone", "release", "v0.1"],
                None,
            ),
        ];
        let r = Graph::new(plan.iter()).roadmap();

        let rel = r
            .cuts
            .iter()
            .find(|c| c.milestone_id == "R-v0.1")
            .expect("the release");
        assert_eq!(rel.closes, vec!["M-s0".to_string(), "M-s1".to_string()]);
        assert_eq!(rel.total, 2, "it still borrows their work for the rollup");

        for id in ["M-s0", "M-s1"] {
            let cut = r
                .cuts
                .iter()
                .find(|c| c.milestone_id == id)
                .expect("the cut");
            assert!(
                cut.closes.is_empty(),
                "`{id}` is a cut and closed something: {:?}",
                cut.closes
            );
        }
    }

    /// A release's `crit` is the chain that ends at the release, not the longest
    /// chain among the work it borrows (`A1-345`).
    ///
    /// The two differ exactly where cuts are sequenced. `critical_path_within`
    /// sees only the release's members, so the milestone nodes that carry cut
    /// order are not in the induced set and their edges vanish: B-1's dependency
    /// on the whole of cut s0 disappears, and two tasks that run one after the
    /// other read as running side by side. The sixth pilot's release row said
    /// `82.0d` for this reason while the chain that controls the release was
    /// longer. Same word, two quantities, and the smaller was on the page.
    #[test]
    fn a_release_reports_the_chain_that_ends_at_it() {
        let a = task("A-1", &[], &[], Some(1.0));
        // Sequenced after the first cut (ADR-0013): this is the edge the
        // member-induced answer cannot see.
        let b = task("B-1", &["M-s0"], &[], Some(2.0));
        let s0 = task("M-s0", &["A-1"], &["milestone", "s0"], None);
        let s1 = task("M-s1", &["B-1", "M-s0"], &["milestone", "s1"], None);
        let release = task(
            "R-v0.1",
            &["M-s0", "M-s1"],
            &["milestone", "release", "v0.1"],
            None,
        );
        let plan = [a, b, s0, s1, release];
        let r = Graph::new(plan.iter()).roadmap();

        let rel = r
            .cuts
            .iter()
            .find(|c| c.milestone_id == "R-v0.1")
            .expect("the release");
        assert_eq!(
            rel.critical_path
                .tasks
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            vec!["A-1", "B-1"],
            "the release's chain runs through the cut it waits on"
        );
        assert!(
            (rel.critical_path.total_effort_days - 3.0).abs() < 1e-9,
            "got {}, and 2.0 is the member-induced answer this replaced",
            rel.critical_path.total_effort_days
        );

        // A *cut* keeps the member-induced number, because its row asks how much
        // work is in this bucket rather than what leads to it: s1 holds B-1 only.
        let s1 = r
            .cuts
            .iter()
            .find(|c| c.milestone_id == "M-s1")
            .expect("the cut");
        assert!(
            (s1.critical_path.total_effort_days - 2.0).abs() < 1e-9,
            "got {}",
            s1.critical_path.total_effort_days
        );
    }

    /// A container's row is derived the same way a release's is (`A1-345`,
    /// ADR-0042), because it holds the same relationship to the work beneath it.
    ///
    /// Keying on `release` instead would have left the defect in place for
    /// exactly the plans that took up the newer field: containment is not an
    /// edge, so the member-induced chain over a container's rolled-up work loses
    /// the same sequencing it loses for a release.
    #[test]
    fn a_container_reports_the_chain_that_ends_at_it_too() {
        let a = task("A-1", &[], &[], Some(1.0));
        let b = task("B-1", &["M-s0"], &[], Some(2.0));
        let s0 = task("M-s0", &["A-1"], &["milestone", "s0"], None);
        let s1 = task("M-s1", &["B-1"], &["milestone", "s1"], None);
        // Holds both cuts *and* waits on them, which is the only shape
        // `APLAN047` accepts (`A1-345`, audit: the first draft of this test
        // wrote `contains` alone, which `check` refuses).
        let mut container = task("C-top", &["M-s0", "M-s1"], &["milestone", "top"], None);
        container.contains = vec!["M-s0".into(), "M-s1".into()];
        let plan = [a, b, s0, s1, container];
        let r = Graph::new(plan.iter()).roadmap();

        let top = r
            .cuts
            .iter()
            .find(|c| c.milestone_id == "C-top")
            .expect("the container");
        assert!(top.borrows, "a container borrows what it holds");
        assert!(!top.release, "and it is not a release");
        assert_eq!(
            top.critical_path
                .tasks
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            vec!["A-1", "B-1"],
            "the chain descends `contains` and keeps the sequencing edge"
        );
        assert!(
            (top.critical_path.total_effort_days - 3.0).abs() < 1e-9,
            "got {}, and 2.0 is the member-induced answer this replaced",
            top.critical_path.total_effort_days
        );
    }

    /// A plan whose containment closes a loop still renders a roadmap.
    ///
    /// `APLAN047` rejects it and `critical-path` refuses to name a number, but
    /// this is a report and a report that panics on a bad plan is worse than one
    /// that says `0.0d` beside a rollup the reader can see is larger. The walk
    /// has no defensible answer, so the row carries the absence rather than an
    /// invented figure (`A1-345`, audit).
    #[test]
    fn a_containment_loop_renders_rather_than_panicking() {
        let a = task("A-1", &[], &[], Some(1.0));
        let mut one = task("M-s0", &["A-1", "M-s1"], &["milestone", "s0"], None);
        one.contains = vec!["M-s1".into()];
        let mut two = task("M-s1", &["M-s0"], &["milestone", "s1"], None);
        two.contains = vec!["M-s0".into()];
        let plan = [a, one, two];
        let r = Graph::new(plan.iter()).roadmap();
        for cut in &r.cuts {
            assert!(cut.borrows, "both rows hold the other");
            assert!(
                (cut.critical_path.total_effort_days - 0.0).abs() < 1e-9,
                "`{}` invented a number over a loop: {}",
                cut.milestone_id,
                cut.critical_path.total_effort_days
            );
        }
    }

    /// A release's chain never counts its checkpoint nodes as work, so the
    /// number stays an effort total rather than a hop count.
    #[test]
    fn a_release_chain_holds_no_milestones() {
        let a = task("A-1", &[], &[], Some(1.0));
        let s0 = task("M-s0", &["A-1"], &["milestone", "s0"], None);
        let release = task("R-v0.1", &["M-s0"], &["milestone", "release", "v0.1"], None);
        let plan = [a, s0, release];
        let r = Graph::new(plan.iter()).roadmap();
        let rel = r
            .cuts
            .iter()
            .find(|c| c.milestone_id == "R-v0.1")
            .expect("the release");
        assert_eq!(
            rel.critical_path
                .tasks
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            vec!["A-1"]
        );
    }

    /// A release rolls up the work of the cuts it closes (`A1-146`, ADR-0022).
    ///
    /// Reading only its own `blocked_by` rendered `0/0 done`, which is true of
    /// the milestone and useless as an answer to "how is v0.1 going".
    #[test]
    fn a_release_rolls_up_the_cuts_it_closes_without_double_counting() {
        let a = task("A-1", &[], &[], Some(1.0));
        let b = task("B-1", &[], &[], Some(2.0));
        let s0 = task("M-s0", &["A-1"], &["milestone", "s0"], None);
        let s1 = task("M-s1", &["B-1"], &["milestone", "s1"], None);
        let release = task(
            "R-v0.1",
            &["M-s0", "M-s1"],
            &["milestone", "release", "v0.1"],
            None,
        );
        let plan = [a, b, s0, s1, release];
        let r = Graph::new(plan.iter()).roadmap();

        let rel = r
            .cuts
            .iter()
            .find(|c| c.milestone_id == "R-v0.1")
            .expect("the release is a cut in the view");
        assert!(rel.release, "the view must say which kind it is");
        assert_eq!(rel.total, 2, "both cuts' work: {:?}", rel.members);

        // And the cuts still hold their own work, so the task appears in its cut
        // as well as in the release that closes it.
        let s0 = r
            .cuts
            .iter()
            .find(|c| c.milestone_id == "M-s0")
            .expect("cut");
        assert_eq!(s0.total, 1);
        assert!(!s0.release);

        // Nothing is cross-cutting: the release borrowing a task must not stop
        // its cut from claiming it, nor claim it itself.
        assert!(
            r.cross_cutting.is_empty(),
            "{:?}",
            r.cross_cutting.iter().map(|t| &t.id).collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod label_tests {
    use super::cut_label;
    use crate::task::{Status, Task};

    fn milestone(tags: &[&str]) -> Task {
        Task {
            id: "M-1".into(),
            title: "A cut".into(),
            phase: 0,
            status: Status::Todo,
            owner: None,
            effort_days: None,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: tags.iter().map(|t| (*t).to_string()).collect(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    /// The one derivation two things read: what `roadmap` renders, and what
    /// `--cut` answers to (`A1-139`).
    #[test]
    fn a_version_tag_wins_and_any_other_tag_serves() {
        assert_eq!(cut_label(&milestone(&["milestone", "v0.8"])), "v0.8");
        // A project with its own vocabulary is not missing a version.
        assert_eq!(cut_label(&milestone(&["milestone", "s3"])), "s3");
        // The version wins wherever it sits among the others.
        assert_eq!(cut_label(&milestone(&["milestone", "s3", "v1.0"])), "v1.0");
        // Nothing but the marker: no label, and `--cut ""` must not match it by
        // accident, which is why the resolver refuses a name nobody answers to.
        assert_eq!(cut_label(&milestone(&["milestone"])), "");
    }
}
