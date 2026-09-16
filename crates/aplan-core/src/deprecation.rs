//! The deprecation runway: announced in one cut, removed in a later one.
//!
//! FIELD-RESEARCH section 6 P2 found this the cleanest checkable planning
//! promise in the wild -- GitLab's deprecation rows, systemd's "Announcements of
//! Future Feature Removals", Kubernetes removal milestones and OpenSSL's EOL
//! tables all encode the same pair -- and the reason it is checkable is that
//! both halves are dated by a *release*, not by a calendar.
//!
//! It needs no schema. A runway is two ordinary tasks: one tagged
//! [`DEPRECATION_TAG`] that announces, one tagged [`REMOVAL_TAG`] that removes
//! and lists the announcement in its `blocked_by`. Each is a member of its cut
//! the way any task is, so "announced in v0.3, removed in v0.5" is already
//! written down, and this module only reads it.
//!
//! Hermetic, like the roadmap it is built on: it reads the plan and never git.

use std::collections::BTreeMap;

use crate::roadmap::{Roadmap, parse_cut_version};
use crate::task::{Status, Task};

/// The tag on the task that announces a removal.
pub const DEPRECATION_TAG: &str = "deprecation";

/// The tag on the task that performs one.
pub const REMOVAL_TAG: &str = "removal";

/// One end of a runway, as the report needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunwayTask {
    pub id: String,
    pub title: String,
    pub status: Status,
    /// The cut this task is a member of, by the cut's own label. `None` when it
    /// belongs to no cut, which is what makes a runway undateable.
    pub cut: Option<String>,
    /// Whether the task cites any evidence at all. Whether that evidence
    /// *resolves* is `aplan verify`'s answer, not this report's.
    pub cited: bool,
}

impl RunwayTask {
    fn of(t: &Task, cut: Option<String>) -> Self {
        Self {
            id: t.id.clone(),
            title: t.title.clone(),
            status: t.status,
            cut,
            cited: !t.evidence.is_empty(),
        }
    }
}

/// What is wrong with a runway's shape, when something is.
///
/// Each is a promise the plan makes and does not keep, and each is decidable
/// from the plan alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// Announced, with no task scheduled to do the removing. The most common
    /// real-world deprecation failure: the notice ships and the removal never
    /// gets planned.
    NoRemovalScheduled,
    /// Announced and removed in the same cut -- or by the same task -- which
    /// gives a user no release to migrate in. The runway *is* the gap, so a
    /// runway of zero is not one.
    RemovedWhereAnnounced,
    /// Removed in a cut that ships *before* the one announcing it.
    RemovedBeforeAnnounced,
    /// The announcement was cancelled and the removal was not, so a removal is
    /// still scheduled for something nobody will have been told about.
    AnnouncementCancelled,
}

impl Fault {
    /// Every shape this reports, so a consumer and a test iterate one list
    /// rather than each keeping a copy.
    pub const ALL: &'static [Self] = &[
        Self::NoRemovalScheduled,
        Self::RemovedWhereAnnounced,
        Self::RemovedBeforeAnnounced,
        Self::AnnouncementCancelled,
    ];

    /// The stable name a machine reads. Exhaustive on purpose: a new variant
    /// fails to compile here until it has one, which is what stops the JSON
    /// growing a shape nothing documents.
    #[must_use]
    pub fn kind(self) -> &'static str {
        match self {
            Self::NoRemovalScheduled => "no_removal_scheduled",
            Self::RemovedWhereAnnounced => "removed_where_announced",
            Self::RemovedBeforeAnnounced => "removed_before_announced",
            Self::AnnouncementCancelled => "announcement_cancelled",
        }
    }

    /// One line, addressed to somebody reading their own plan.
    #[must_use]
    pub fn what(self) -> &'static str {
        match self {
            Self::NoRemovalScheduled => {
                "announced, and no task is scheduled to remove it: the notice ships and the \
                 removal never lands"
            }
            Self::RemovedWhereAnnounced => {
                "announced and removed together, with no release in between for anybody \
                 depending on it to migrate in"
            }
            Self::RemovedBeforeAnnounced => {
                "removed in a cut that ships before the one announcing it, so the notice arrives \
                 after the removal"
            }
            Self::AnnouncementCancelled => {
                "the announcement was cancelled and the removal was not, so this is still \
                 scheduled to remove something that will never have been announced"
            }
        }
    }
}

/// One announcement and whatever removes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runway {
    pub announced: RunwayTask,
    /// Every task tagged [`REMOVAL_TAG`] that lists the announcement as a
    /// blocker. Usually one; more than one is a removal done in pieces, and
    /// fewer than one is [`Fault::NoRemovalScheduled`].
    pub removals: Vec<RunwayTask>,
    pub faults: Vec<Fault>,
}

/// Every runway in a plan, and the removals that announce nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deprecations {
    pub runways: Vec<Runway>,
    /// Tagged [`REMOVAL_TAG`] and blocked by no announcement: a removal nobody
    /// was told about, which is the pair broken from the other end.
    pub unannounced: Vec<RunwayTask>,
}

/// Read the runways out of a plan.
///
/// The roadmap supplies cut membership, so "which release is this in" has one
/// definition here and in every other rollup.
#[must_use]
pub fn of(tasks: &[&Task], roadmap: &Roadmap) -> Deprecations {
    let cut_by_id = cut_index(roadmap);
    // A cut whose milestone carries no version label dates nothing, so it is
    // `None` here rather than an empty string: one representation of "no
    // release names this", not two for a consumer to handle.
    let cut_of = |t: &Task| {
        cut_by_id
            .get(t.id.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| (*s).to_string())
    };

    // A milestone is not work, exactly as every other rollup reads it.
    let removals: Vec<&Task> = tasks
        .iter()
        .copied()
        .filter(|t| has_tag(t, REMOVAL_TAG) && !t.is_milestone())
        .collect();
    // A `dropped` announcement is a deprecation somebody called off, and
    // reporting it as an announcement with no removal scheduled would be a
    // finding about a decision already taken -- *unless* a removal still names
    // it, which is the decision half-taken and the thing worth saying. Dropping
    // it from the list unconditionally made the removal read as one nobody was
    // told about, which sent the reader looking for an announcement sitting
    // right there in the plan (found by this task's audit).
    let announced_at_all: Vec<&str> = removals
        .iter()
        .flat_map(|r| r.blocked_by.iter().map(String::as_str))
        .collect();
    let announcements: Vec<&Task> = tasks
        .iter()
        .copied()
        .filter(|t| has_tag(t, DEPRECATION_TAG) && !t.is_milestone())
        .filter(|t| t.status != Status::Dropped || announced_at_all.contains(&t.id.as_str()))
        .collect();

    let mut runways = Vec::with_capacity(announcements.len());
    for a in &announcements {
        let announced = RunwayTask::of(a, cut_of(a));
        let mine: Vec<&&Task> = removals
            .iter()
            .filter(|r| r.blocked_by.iter().any(|b| b == &a.id))
            .collect();
        let removal_tasks: Vec<RunwayTask> =
            mine.iter().map(|r| RunwayTask::of(r, cut_of(r))).collect();

        let live_removal = removal_tasks.iter().any(|r| r.status != Status::Dropped);
        let mut faults = Vec::new();
        if a.status == Status::Dropped {
            // The announcement was called off. If nothing live still points at
            // it the whole runway was cancelled cleanly, which is a decision
            // recorded rather than a shape broken, and it earns no fault: the
            // two statuses on the rows say it. If a live removal survives it,
            // that is the decision half-taken.
            if live_removal {
                faults.push(Fault::AnnouncementCancelled);
            }
        } else if has_tag(a, REMOVAL_TAG) {
            // One task carrying both tags is a runway of zero length: announced
            // and removed in one go, which is the `RemovedWhereAnnounced` case
            // said another way. Reported once, rather than as an announcement
            // with no removal *and* a removal nobody announced, which is what
            // two independent scans would say about one honest mistake.
            faults.push(Fault::RemovedWhereAnnounced);
        } else if !live_removal {
            // A removal that was dropped is not scheduled. It stays in the list
            // so the reader can see it was cancelled rather than never written.
            faults.push(Fault::NoRemovalScheduled);
        }

        // Ordering, only for a runway still meant to happen: on a cancelled
        // announcement the cuts it would have spanned are not the finding.
        if a.status != Status::Dropped {
            for r in &removal_tasks {
                // Only where both ends carry a version this tool can order. A
                // cut labelled `s3` is a vocabulary, not a missing version
                // (`A1-037`), and guessing an order for it would invent a
                // finding.
                let (Some(from), Some(to)) = (
                    announced.cut.as_deref().and_then(parse_cut_version),
                    r.cut.as_deref().and_then(parse_cut_version),
                ) else {
                    continue;
                };
                let fault = match to.cmp(&from) {
                    std::cmp::Ordering::Equal => Fault::RemovedWhereAnnounced,
                    std::cmp::Ordering::Less => Fault::RemovedBeforeAnnounced,
                    std::cmp::Ordering::Greater => continue,
                };
                // `contains`, not `dedup`: with three removals the same fault
                // can arrive either side of a different one, and `dedup` only
                // collapses neighbours.
                if !faults.contains(&fault) {
                    faults.push(fault);
                }
            }
        }
        runways.push(Runway {
            announced,
            removals: removal_tasks,
            faults,
        });
    }
    runways.sort_by(|a, b| a.announced.id.cmp(&b.announced.id));

    let announced_ids: Vec<&str> = announcements.iter().map(|a| a.id.as_str()).collect();
    let mut unannounced: Vec<RunwayTask> = removals
        .iter()
        .filter(|r| !has_tag(r, DEPRECATION_TAG))
        .filter(|r| {
            !r.blocked_by
                .iter()
                .any(|b| announced_ids.contains(&b.as_str()))
        })
        .map(|r| RunwayTask::of(r, cut_of(r)))
        .collect();
    unannounced.sort_by(|a, b| a.id.cmp(&b.id));

    Deprecations {
        runways,
        unannounced,
    }
}

fn has_tag(t: &Task, tag: &str) -> bool {
    t.tags.iter().any(|x| x == tag)
}

/// Task id to the label of the cut holding it.
fn cut_index(roadmap: &Roadmap) -> BTreeMap<&str, &str> {
    let mut out = BTreeMap::new();
    for cut in &roadmap.cuts {
        // A release borrows its cuts' members for its rollup (ADR-0022), so
        // claiming them here would overwrite the cut that actually ships them.
        if cut.release {
            continue;
        }
        for m in &cut.members {
            out.insert(m.id.as_str(), cut.version.as_str());
        }
        out.insert(cut.milestone_id.as_str(), cut.version.as_str());
    }
    out
}
