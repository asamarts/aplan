//! The roadmap view: the plan grouped by cut (the milestone meta-tasks,
//! ADR-0008), with per-cut rollups and critical path, plus the tasks in no cut.
//!
//! This is the model `aplan roadmap` renders and `gen-roadmap --check` gates
//! (ADR-0009). It is hermetic, it reads the plan and never git, and renders the
//! *cited* evidence, not its resolution (`aplan verify` owns resolution). It is
//! advisory: a plan is not wrong for being in progress.

use std::collections::BTreeSet;

use crate::graph::{CriticalPath, Graph};
use crate::task::{EvidenceKind, Status, Task};

/// The whole plan as a roadmap: cuts in version order, then the cross-cutting
/// tasks that belong to no cut.
#[derive(Debug, Clone, PartialEq)]
pub struct Roadmap {
    pub cuts: Vec<Cut>,
    /// Real tasks that are a member of no cut (e.g. a cross-cutting quality gate).
    pub cross_cutting: Vec<RoadmapTask>,
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
    pub total: usize,
    pub done: usize,
    pub ready: usize,
    pub blocked: usize,
    /// The cut's member tasks, in id order.
    pub members: Vec<RoadmapTask>,
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
        }
    }
}

/// Parse a `vN.N` cut tag into `(major, minor)` for numeric ordering, so `v0.10`
/// sorts *after* `v0.9`. `None` for a tag that is not a cut version.
pub fn parse_cut_version(tag: &str) -> Option<(u32, u32)> {
    let (major, minor) = tag.strip_prefix('v')?.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
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

        for (&id, &m) in &self.by_id {
            if !m.is_milestone() {
                continue;
            }
            let version = m
                .tags
                .iter()
                .find(|t| parse_cut_version(t).is_some())
                .cloned()
                .unwrap_or_default();

            // Members: the milestone's blockers that exist and are not themselves
            // milestones (a milestone is never its own member).
            let member_ids: BTreeSet<&str> = m
                .blocked_by
                .iter()
                .map(String::as_str)
                .filter(|b| self.get(b).is_some_and(|t| !t.is_milestone()))
                .collect();

            // The rollup and critical path count only members that can still
            // happen: a `dropped` member is shown in the table (for transparency)
            // but excluded from `total`/`done`/`ready`/`blocked` and from the
            // path, exactly as the whole-plan `critical_path` excludes dropped, so
            // `done + ready + blocked == total` always reconciles.
            let mut members = Vec::with_capacity(member_ids.len());
            let (mut total, mut done, mut ready, mut blocked) = (0usize, 0usize, 0usize, 0usize);
            let mut active: BTreeSet<&str> = BTreeSet::new();
            for &mid in &member_ids {
                claimed.insert(mid);
                let t = self.get(mid).expect("member exists (filtered above)");
                if t.status != Status::Dropped {
                    total += 1;
                    active.insert(mid);
                    if t.status == Status::Done {
                        done += 1;
                    } else if t.is_ready(|b| self.is_done(b)) {
                        ready += 1;
                    } else {
                        blocked += 1;
                    }
                }
                members.push(RoadmapTask::of(t));
            }

            let critical_path = self.critical_path_within(&active).unwrap_or(CriticalPath {
                total_effort_days: 0.0,
                tasks: Vec::new(),
            });

            cuts.push(Cut {
                version,
                title: m.title.clone(),
                milestone_id: id.to_string(),
                status: m.status,
                total,
                done,
                ready,
                blocked,
                members,
                critical_path,
            });
        }

        cuts.sort_by(|a, b| {
            match (parse_cut_version(&a.version), parse_cut_version(&b.version)) {
                (Some(x), Some(y)) => x.cmp(&y),
                // A versioned cut sorts before an unversioned one.
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| a.milestone_id.cmp(&b.milestone_id))
        });

        let cross_cutting = self
            .by_id
            .iter()
            .filter(|(id, t)| !t.is_milestone() && !claimed.contains(*id))
            .map(|(_, t)| RoadmapTask::of(t))
            .collect();

        Roadmap {
            cuts,
            cross_cutting,
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
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: tags.iter().map(|s| (*s).to_string()).collect(),
        }
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
