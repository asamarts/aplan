//! What a subset of a plan is, in one place (`A1-214`).
//!
//! Selection was one dimension on six commands and the plan has five. `--tag`
//! existed on exactly one command; `ready`, `why`, `summary`, `stale`,
//! `vacuity`, `paths`, `tasks`, `critical-path` and `roadmap` answered about the
//! whole plan or about one task with nothing in between. Adding them a flag at a
//! time is forty-five flags, each with its own parsing, its own help line and
//! its own way of being slightly different.
//!
//! **Here rather than in the CLI**, because `A1-183` asks the same question of
//! `.aplan.yml` -- policy scoped to a subset -- and two vocabularies for one idea
//! is the drift this cut exists to remove. One model, and as many renderings as
//! there are surfaces.
//!
//! **Selection narrows what is *shown*, never what is *computed*.** A task whose
//! blocker is outside the subset is still blocked: filtering the plan and then
//! asking what is ready would answer that it is, which is a false answer rather
//! than a narrow one. Every consumer computes over the whole plan and filters
//! the rows.
//!
//! **This is not a query language.** A conjunction of equality tests over
//! declared dimensions, and nothing else: no negation, no comparison, no boolean
//! combination. `ADR-0026` records that boundary and `A1-221`'s reachability
//! terms are the one extension it anticipates.

use crate::task::{Status, Task};

/// A subset of a plan, as a conjunction of equality tests.
///
/// Empty means the whole plan, so a command can always build one and apply it
/// without asking whether the user selected anything.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Selector {
    /// Cut labels or milestone ids. A task matches if it is in any of them.
    pub cuts: Vec<String>,
    /// Numeric phases. A task matches if it carries any of them.
    pub phases: Vec<u32>,
    /// Tags. A task matches if it carries any of them.
    pub tags: Vec<String>,
    /// Owners. A task matches if any is its owner.
    pub owners: Vec<String>,
    /// Statuses. A task matches if it is in any of them.
    pub statuses: Vec<Status>,
    /// Only tasks with no observed ephemeral claim (`A1-531`).
    ///
    /// This is a separate dimension from durable `owners`: a task can be the
    /// responsibility of team `x` while nobody is actively working on it.
    /// Callers evaluating this dimension must use [`Self::matches_claimed`].
    pub unclaimed: bool,
}

impl Selector {
    /// Whether this selects the whole plan.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cuts.is_empty()
            && self.phases.is_empty()
            && self.tags.is_empty()
            && self.owners.is_empty()
            && self.statuses.is_empty()
            && !self.unclaimed
    }

    /// Whether `task` is in the subset, given the cut it belongs to.
    ///
    /// **Within a dimension, any; across dimensions, all.** `--tag a --tag b` is
    /// "either tag", because a reader listing two tags is widening; `--tag a
    /// --owner b` is "both", because a reader naming two dimensions is
    /// narrowing. That is the reading `graph` already had and the one every
    /// other tool with these flags uses.
    #[must_use]
    pub fn matches(&self, task: &Task, cut: Option<&str>, milestone: Option<&str>) -> bool {
        self.matches_fields(
            task.phase,
            task.status,
            task.owner.as_deref(),
            &task.tags,
            cut,
            milestone,
        )
    }

    /// Whether a task matches when its ephemeral claim state is known.
    ///
    /// `owner` remains durable plan responsibility. `claimed` is deliberately
    /// separate: collapsing the two is the worktree-coordination defect
    /// `A1-531` removes. `None` means the local coordination scope was not
    /// observable and therefore never satisfies `unclaimed`.
    #[must_use]
    pub fn matches_claimed(
        &self,
        task: &Task,
        cut: Option<&str>,
        milestone: Option<&str>,
        claimed: Option<bool>,
    ) -> bool {
        self.matches_fields_claimed(
            task.phase,
            task.status,
            task.owner.as_deref(),
            &task.tags,
            cut,
            milestone,
            claimed,
        )
    }

    /// Whether resolved task fields are in the subset.
    ///
    /// Views whose rows already came from [`crate::Graph`] use this form so
    /// they do not need to recover a second [`Task`] model merely to apply the
    /// selector. It is deliberately the implementation behind [`Self::matches`]:
    /// interactive and command-line consumers cannot acquire subtly different
    /// readings of a selector dimension.
    #[must_use]
    pub fn matches_fields(
        &self,
        phase: u32,
        status: Status,
        owner: Option<&str>,
        tags: &[String],
        cut: Option<&str>,
        milestone: Option<&str>,
    ) -> bool {
        let in_cut = self.cuts.is_empty()
            || self
                .cuts
                .iter()
                .any(|want| Some(want.as_str()) == cut || Some(want.as_str()) == milestone);
        let has_tag = self.tags.is_empty() || tags.iter().any(|tag| self.tags.contains(tag));
        let is_phase = self.phases.is_empty() || self.phases.contains(&phase);
        let has_requested_owner = self.owners.is_empty()
            || owner.is_some_and(|candidate| self.owners.iter().any(|want| want == candidate));
        let is_status = self.statuses.is_empty() || self.statuses.contains(&status);
        // No claim observation was supplied to this compatibility entry point.
        // Fail closed for `unclaimed`; absence of an observation is not an
        // observation that the task is free.
        in_cut && is_phase && has_tag && has_requested_owner && is_status && !self.unclaimed
    }

    /// Apply the selector with an independently observed ephemeral claim.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn matches_fields_claimed(
        &self,
        phase: u32,
        status: Status,
        owner: Option<&str>,
        tags: &[String],
        cut: Option<&str>,
        milestone: Option<&str>,
        claimed: Option<bool>,
    ) -> bool {
        let mut durable = self.clone();
        durable.unclaimed = false;
        durable.matches_fields(phase, status, owner, tags, cut, milestone)
            && (!self.unclaimed || claimed == Some(false))
    }

    /// How a view says what it was narrowed to.
    ///
    /// One wording, so nine commands cannot describe the same subset nine ways.
    #[must_use]
    pub fn describe(&self) -> Vec<String> {
        let mut out = Vec::new();
        {
            let mut say = |what: &str, values: Vec<String>| {
                if !values.is_empty() {
                    out.push(format!("{what} `{}`", values.join("`, `")));
                }
            };
            say("cut", self.cuts.clone());
            say(
                "phase",
                self.phases.iter().map(ToString::to_string).collect(),
            );
            say("tag", self.tags.clone());
            say("owner", self.owners.clone());
            say(
                "status",
                self.statuses.iter().map(ToString::to_string).collect(),
            );
        }
        // Not a value in a dimension, so it is said rather than listed: there
        // is no name to print. Say activity rather than ownership because a
        // durable owner may still match the same selector (`A1-531`).
        if self.unclaimed {
            out.push("not actively claimed".to_string());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, owner: Option<&str>, tags: &[&str], status: Status) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            phase: 0,
            status,
            owner: owner.map(str::to_string),
            effort_days: Some(1.0),
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            evidence_fingerprints: std::collections::BTreeMap::new(),
            tags: tags.iter().map(|s| (*s).to_string()).collect(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    #[test]
    fn an_empty_selector_is_the_whole_plan() {
        let s = Selector::default();
        assert!(s.is_empty());
        assert!(s.matches(&task("A", None, &[], Status::Todo), None, None));
    }

    /// Within a dimension, any: two tags widen.
    #[test]
    fn two_values_in_one_dimension_widen() {
        let s = Selector {
            tags: vec!["a".into(), "b".into()],
            ..Selector::default()
        };
        assert!(s.matches(&task("A", None, &["a"], Status::Todo), None, None));
        assert!(s.matches(&task("B", None, &["b"], Status::Todo), None, None));
        assert!(!s.matches(&task("C", None, &["c"], Status::Todo), None, None));
    }

    #[test]
    fn phase_values_are_a_real_selector_dimension() {
        let s = Selector {
            phases: vec![2, 4],
            ..Selector::default()
        };
        let mut in_phase = task("A", None, &[], Status::Todo);
        in_phase.phase = 4;
        let mut outside = task("B", None, &[], Status::Todo);
        outside.phase = 3;
        assert!(s.matches(&in_phase, None, None));
        assert!(!s.matches(&outside, None, None));
    }

    /// Across dimensions, all: two dimensions narrow.
    #[test]
    fn two_dimensions_narrow() {
        let s = Selector {
            tags: vec!["a".into()],
            owners: vec!["kim".into()],
            ..Selector::default()
        };
        assert!(s.matches(&task("A", Some("kim"), &["a"], Status::Todo), None, None));
        assert!(!s.matches(&task("B", Some("sam"), &["a"], Status::Todo), None, None));
        assert!(!s.matches(&task("C", Some("kim"), &["z"], Status::Todo), None, None));
    }

    /// A cut is named by its label or by its milestone id, because `roadmap`
    /// prints the label and `blocked_by` carries the id.
    #[test]
    fn a_cut_matches_by_label_or_by_milestone_id() {
        let s = Selector {
            cuts: vec!["v0.9".into()],
            ..Selector::default()
        };
        let t = task("A", None, &[], Status::Todo);
        assert!(s.matches(&t, Some("v0.9"), Some("M-v0.9")));
        assert!(!s.matches(&t, Some("v0.8"), Some("M-v0.8")));

        let by_id = Selector {
            cuts: vec!["M-v0.9".into()],
            ..Selector::default()
        };
        assert!(by_id.matches(&t, Some("v0.9"), Some("M-v0.9")));
    }

    #[test]
    fn a_task_with_no_owner_is_not_selected_by_owner() {
        let s = Selector {
            owners: vec!["kim".into()],
            ..Selector::default()
        };
        assert!(!s.matches(&task("A", None, &[], Status::Todo), None, None));
    }

    #[test]
    fn stable_owner_and_no_ephemeral_claim_are_compatible_dimensions() {
        let s = Selector {
            owners: vec!["kim".into()],
            unclaimed: true,
            ..Selector::default()
        };
        let owned = task("A", Some("kim"), &[], Status::Todo);
        assert!(s.matches_claimed(&owned, None, None, Some(false)));
        assert!(!s.matches_claimed(&owned, None, None, Some(true)));
        assert!(
            !s.matches(&owned, None, None),
            "a caller with no claim observation must fail closed"
        );
    }

    #[test]
    fn the_description_names_every_dimension_given() {
        let s = Selector {
            cuts: vec!["v0.9".into()],
            phases: vec![2],
            tags: vec!["cli".into(), "output".into()],
            owners: Vec::new(),
            statuses: vec![Status::Todo],
            unclaimed: false,
        };
        assert_eq!(
            s.describe(),
            [
                "cut `v0.9`",
                "phase `2`",
                "tag `cli`, `output`",
                "status `todo`"
            ]
        );
        // Said rather than listed, and last, because it names no value
        // (`A1-359`).
        assert_eq!(
            Selector {
                unclaimed: true,
                ..s.clone()
            }
            .describe(),
            [
                "cut `v0.9`",
                "phase `2`",
                "tag `cli`, `output`",
                "status `todo`",
                "not actively claimed"
            ]
        );
        assert!(
            !Selector {
                unclaimed: true,
                ..Selector::default()
            }
            .is_empty(),
            "`--unclaimed` alone is a narrowing, so it is not the whole plan"
        );
        assert!(Selector::default().describe().is_empty());
    }
}
