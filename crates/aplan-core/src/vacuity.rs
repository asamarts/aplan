//! What each rule could actually have failed on (`A1-114`).
//!
//! A rule that evaluates to "pass" and a rule that had nothing to evaluate
//! produce the same output, and the second is the more common state. The second
//! pilot stated the general form of it while reporting something else -- a
//! capability check whose shell substitution had been eaten, so it asserted
//! nothing and exited `0` -- and observed that such a check "can be wrong in the
//! direction of silence, and the thing it guards is precisely the thing nobody
//! re-checks" (pre-public adoption evidence item 2.4).
//!
//! Four rules that catch a *check having quietly not checked anything* have
//! shipped, and each was written from a pilot finding the vacuity by hand. This
//! is the mechanism those are instances of, so the next one does not have to be
//! found in somebody's spreadsheet.
//!
//! **Derived from the rules, not from a list beside them.** Each rule declares
//! what a task must carry for it to have a subject, and what its predicate
//! compares that against, on the same row that generates its code, its severity
//! and its catalogue entry ([`Binding`]). A second table of "which rules can be
//! vacuous" would be the same defect one level up, which is what this task was
//! written to avoid.
//!
//! **Not a coverage percentage.** "`APLAN012` ran on 44 tasks" is not the
//! question; how many of those it could have failed is.

use crate::diagnostic::{Binding, Rule};
use crate::task::Task;

/// One rule's reach over a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleReach {
    pub rule: Rule,
    /// Tasks this rule has a subject in: it applies to them at all.
    pub subjects: usize,
    /// Subjects whose predicate had nothing to compare against, so the rule
    /// could not have failed however wrong the task was.
    pub unbound: usize,
    /// What the predicate compares against, for the report to name.
    pub binds: &'static [Binding],
    /// The `.aplan.yml` key that stopped this rule from running, if one did.
    ///
    /// `checked` counts subjects the rule *could* have failed on given what the
    /// task carried. It said nothing about whether the rule ran at all, so a
    /// rule the configuration had switched off was reported as having checked
    /// its subjects -- an answer, and the wrong one, from the command whose
    /// whole purpose is telling those two apart (`A1-269`).
    pub off: Option<&'static str>,
}

impl RuleReach {
    /// Subjects the rule could actually have failed on.
    ///
    /// Zero where the configuration switched the rule off, whatever the tasks
    /// carried: a rule that did not run checked nothing, and counting its
    /// subjects as checked is the claim `A1-269` was filed about.
    #[must_use]
    pub fn checked(&self) -> usize {
        if self.off.is_some() {
            return 0;
        }
        self.subjects.saturating_sub(self.unbound)
    }
}

/// A config switch, and the `.aplan.yml` key an adopter would set.
///
/// Each is a field of [`crate::validate::Options`] that can stop a rule from
/// firing at all. The list is of *switches*, not of rules: which rules each one
/// stops is measured rather than declared, so a rule that grows a dependency on
/// one joins without anybody remembering (`A1-269`).
type Switch = (&'static str, fn(&mut crate::validate::Options));

const SWITCHES: &[Switch] = &[
    ("require_effort", |o| o.require_effort = true),
    ("detect_drift", |o| o.detect_drift = true),
    ("require_cut_membership", |o| {
        o.require_cut_membership = true;
    }),
    // Any non-empty list turns `APLAN017` on; the strongest set is the one a
    // repository steers toward, so it is what "on" means here.
    ("evidence.require_min_strength", |o| {
        o.require_min_strength = Some(vec![
            crate::task::EvidenceKind::Commit,
            crate::task::EvidenceKind::File,
            crate::task::EvidenceKind::Test,
        ]);
    }),
    // On means the shipped floors, which is what an adopter who sets
    // `require_impact` and nothing else gets. Choosing stricter floors here
    // would report `APLAN044` as switched off for tasks the default would not
    // have named either.
    ("version.require_impact", |o| {
        o.require_impact = Some(crate::task::VersionFloors::default());
    }),
];

/// Which rules the configuration has switched off, and the key that did it.
///
/// Measured by asking: with this switch on and everything else as configured,
/// does any rule say something it does not say now? A rule that does was
/// prevented from speaking by that key. Nothing is hand-listed, so a rule that
/// starts reading a switch appears here on the commit that makes it do so, and
/// one that stops reading it disappears the same way.
///
/// **What it can and cannot see.** It reports a rule the configuration would
/// have changed the answer of *on this plan*, which is the actionable set: it is
/// exactly the case `p003` hit, where a fixture omitted `detect_drift` and the
/// drift it had deliberately introduced went unreported while `vacuity` called
/// the task checked. A rule switched off with subjects it would have passed
/// anyway is not distinguishable from one that ran, and cannot be without
/// asking the rules to declare their own dependencies -- a second registry that
/// would drift from the first.
#[must_use]
pub fn switched_off(
    plan: &crate::load::Plan,
    opts: &crate::validate::Options,
) -> std::collections::BTreeMap<&'static str, &'static str> {
    let codes = |o: &crate::validate::Options| -> std::collections::BTreeSet<String> {
        crate::validate::validate(plan, o)
            .diagnostics
            .into_iter()
            .map(|d| d.code.to_string())
            .collect()
    };
    let now = codes(opts);
    let mut out = std::collections::BTreeMap::new();
    for (key, turn_on) in SWITCHES {
        let mut with = opts.clone();
        turn_on(&mut with);
        // Already on: nothing to reveal, and the run would cost a full pass over
        // the plan to learn it. On a plan this repository's size that is most of
        // the command's time (`A1-269`'s audit).
        if with == *opts {
            continue;
        }
        for code in codes(&with).difference(&now) {
            // A rule already attributed keeps its first key: two switches that
            // both unblock it is a fact about the plan, and naming one is what
            // an adopter can act on.
            if let Some(rule) = Rule::ALL.iter().find(|r| r.code() == code) {
                out.entry(rule.code()).or_insert(*key);
            }
        }
    }
    out
}

/// Every rule whose reach is worth reporting, with the tasks it had nothing to
/// say about.
///
/// A rule that binds against nothing is left out: it has no vacuity to report,
/// and listing thirty rows of "0 unbound" would bury the six that matter.
#[must_use]
pub fn of(tasks: &[&Task]) -> Vec<RuleReach> {
    of_with(tasks, &std::collections::BTreeMap::new())
}

/// The same, told which rules the configuration switched off (`A1-269`).
///
/// A rule that binds against nothing is normally left out -- it has no vacuity
/// to report, and thirty rows of "0 unbound" would bury the six that matter. A
/// rule that was *switched off* earns a row anyway: that is not a nothing to
/// report, it is the report.
#[must_use]
pub fn of_with(
    tasks: &[&Task],
    off: &std::collections::BTreeMap<&'static str, &'static str>,
) -> Vec<RuleReach> {
    let mut out = Vec::new();
    for &rule in Rule::ALL {
        let binds = rule.binds_against();
        let switched_off = off.get(rule.code()).copied();
        if binds.is_empty() && switched_off.is_none() {
            continue;
        }
        let applies = rule.applies_to();
        // A rule that exempts a milestone declares `NotMilestone` and is filtered
        // by the line below like any other applicability. A blanket filter here
        // was the second copy of that idea, and it was wrong for the rules that
        // do *not* exempt one: a `done` milestone citing a commit is a real
        // `APLAN012` subject, and hiding it understated the count (`A1-157`).
        let subjects: Vec<&&Task> = tasks
            .iter()
            .filter(|t| applies.iter().all(|b| b.holds(t)))
            .collect();
        let unbound = subjects
            .iter()
            .filter(|t| !binds.iter().all(|b| b.holds(t)))
            .count();
        out.push(RuleReach {
            rule,
            subjects: subjects.len(),
            unbound,
            binds,
            off: switched_off,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{Evidence, EvidenceKind, Status};

    fn task(id: &str, status: Status, paths: &[&str], kinds: &[EvidenceKind]) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            phase: 1,
            status,
            owner: None,
            effort_days: Some(1.0),
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: kinds
                .iter()
                .map(|k| Evidence {
                    kind: *k,
                    reference: "x".to_string(),
                    captured: None,
                })
                .collect(),
            paths: paths.iter().map(|p| (*p).to_string()).collect(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            evidence_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    fn reach(tasks: &[Task], rule: Rule) -> RuleReach {
        let refs: Vec<&Task> = tasks.iter().collect();
        of(&refs)
            .into_iter()
            .find(|r| r.rule == rule)
            .expect("the rule reports its reach")
    }

    /// The p00025 case: every `done` task cites a commit and none declares
    /// `paths`, so `APLAN012` ran on all of them and could not have failed on
    /// one.
    #[test]
    fn a_plan_with_no_paths_reports_the_relevance_rule_as_unable_to_bind() {
        let tasks = vec![
            task("A", Status::Done, &[], &[EvidenceKind::Commit]),
            task("B", Status::Done, &[], &[EvidenceKind::Commit]),
        ];
        let r = reach(&tasks, Rule::CommitIrrelevant);
        assert_eq!(r.subjects, 2);
        assert_eq!(r.unbound, 2);
        assert_eq!(r.checked(), 0);
    }

    /// And the same plan with `paths` is the rule doing its job.
    #[test]
    fn declaring_paths_is_what_moves_a_subject_from_unbound_to_checked() {
        let tasks = vec![
            task("A", Status::Done, &["src/**"], &[EvidenceKind::Commit]),
            task("B", Status::Done, &[], &[EvidenceKind::Commit]),
        ];
        let r = reach(&tasks, Rule::CommitIrrelevant);
        assert_eq!((r.subjects, r.unbound, r.checked()), (2, 1, 1));
    }

    /// A rule has no subject in a task it does not apply to, which is not the
    /// same as having nothing to bind against. A `todo` task, or one citing only
    /// a test, is not a subject of the commit-relevance rule at all.
    #[test]
    fn a_task_the_rule_does_not_apply_to_is_not_a_subject() {
        let tasks = vec![
            task("A", Status::Todo, &[], &[EvidenceKind::Commit]),
            task("B", Status::Done, &[], &[EvidenceKind::Test]),
        ];
        let r = reach(&tasks, Rule::CommitIrrelevant);
        assert_eq!((r.subjects, r.unbound), (0, 0));
        // The test-relevance rule has the second one, and cannot bind it.
        let r = reach(&tasks, Rule::TestIrrelevant);
        assert_eq!((r.subjects, r.unbound), (1, 1));
    }

    /// A milestone is exempt exactly where the rule exempts it, and nowhere
    /// else. `APLAN023` declares `NotMilestone` because it really does skip one;
    /// `APLAN012` does not, so a `done` milestone citing a commit is one of its
    /// subjects and its missing `paths` are a real blind spot (`A1-157`).
    #[test]
    fn a_milestone_is_exempt_only_where_the_rule_exempts_it() {
        let mut cut = task("M-s1", Status::Done, &[], &[EvidenceKind::Commit]);
        cut.tags = vec!["milestone".to_string()];
        let r = reach(&[cut.clone()], Rule::CommitIrrelevant);
        assert_eq!(
            (r.subjects, r.unbound),
            (1, 1),
            "APLAN012 judges whatever cites a commit, milestone or not"
        );
        assert!(
            Rule::UncheckableEvidence
                .applies_to()
                .contains(&crate::diagnostic::Binding::NotMilestone),
            "and the rule that does exempt one says so"
        );
    }

    /// Only the rules that compare against something are reported. Thirty rows
    /// of "0 unbound" would bury the ones that matter.
    #[test]
    fn a_rule_that_binds_against_nothing_is_not_reported() {
        let tasks = [task("A", Status::Done, &[], &[EvidenceKind::Commit])];
        let refs: Vec<&Task> = tasks.iter().collect();
        let reported: Vec<Rule> = of(&refs).into_iter().map(|r| r.rule).collect();
        assert!(reported.contains(&Rule::CommitIrrelevant));
        assert!(
            !reported.contains(&Rule::DuplicateId),
            "a rule about ids compares against nothing a task carries: {reported:?}"
        );
        // Every reported rule declares what it binds against, which is what the
        // report names.
        for r in of(&refs) {
            assert!(!r.binds.is_empty(), "{:?}", r.rule);
        }
    }
}

#[cfg(test)]
mod switched_off_tests {
    use super::*;
    use crate::task::Status;

    fn plan_of(tasks: Vec<Task>) -> crate::load::Plan {
        crate::load::Plan {
            tasks: tasks
                .into_iter()
                .map(|task| crate::load::LoadedTask {
                    task,
                    path: std::path::PathBuf::from("plan/tasks/x.md"),
                    fields: crate::load::FieldLines::new(),
                    criteria: crate::load::Criteria::default(),
                })
                .collect(),
            ..crate::load::Plan::default()
        }
    }

    fn live(id: &str) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            phase: 0,
            status: Status::Todo,
            owner: None,
            // No estimate, so `require_effort` has something to say about it.
            effort_days: None,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            evidence_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    /// **Which rules a switch stops is measured, not listed** (`A1-269`).
    ///
    /// Nothing in this module names `APLAN009`. It appears because turning
    /// `require_effort` on makes that rule say something it does not say now --
    /// so a rule that starts reading a switch joins on the commit that makes it
    /// do so, and one that stops reading it leaves the same way. The hand-written
    /// part is the list of *switches*, which is `.aplan.yml`'s own vocabulary.
    #[test]
    fn a_switch_reveals_the_rules_it_was_stopping_without_naming_them() {
        let plan = plan_of(vec![live("A-1")]);
        // `require_effort` defaults to *on*, so switching it off is what puts
        // this plan in the state the rule is asked about.
        let without = crate::validate::Options {
            require_effort: false,
            ..crate::validate::Options::default()
        };
        let off = switched_off(&plan, &without);
        assert_eq!(
            off.get("APLAN009"),
            Some(&"require_effort"),
            "a live task with no estimate is exactly what that switch decides: {off:?}"
        );

        // And with it on -- the default -- the rule is running, so it is not
        // reported as off.
        let on = crate::validate::Options::default();
        assert!(!switched_off(&plan, &on).contains_key("APLAN009"));
    }

    /// A rule the configuration stopped checked nothing, whatever its subjects
    /// carried. Counting them as checked is the claim this task was filed about.
    #[test]
    fn a_switched_off_rule_reports_nothing_checked_and_says_why() {
        let plan = plan_of(vec![live("A-1")]);
        let tasks: Vec<&Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
        let without = crate::validate::Options {
            require_effort: false,
            ..crate::validate::Options::default()
        };
        let off = switched_off(&plan, &without);
        let report = of_with(&tasks, &off);

        let row = report
            .iter()
            .find(|r| r.rule.code() == "APLAN009")
            .expect("a rule with a subject gets a row even where it binds against nothing");
        assert_eq!(row.off, Some("require_effort"));
        assert_eq!(row.checked(), 0, "it did not run, so it checked nothing");
        assert!(
            row.subjects > 0,
            "and the subjects it would have had are real"
        );
    }

    /// **A switch already on is not measured**, because it can reveal nothing
    /// and the run costs a full pass over the plan (`A1-269`'s audit: five
    /// passes made `vacuity` four times the cost of `check`).
    ///
    /// Asserted through the answer rather than by counting passes: with every
    /// switch on, nothing is reported off, which is only true if the loop
    /// skipped them or ran them all to the same conclusion -- and the timing
    /// this protects is why it must be the first.
    #[test]
    fn a_switch_that_is_already_on_reveals_nothing_and_is_skipped() {
        let plan = plan_of(vec![live("A-1")]);
        let everything_on = crate::validate::Options {
            require_effort: true,
            detect_drift: true,
            require_cut_membership: true,
            require_min_strength: Some(vec![crate::task::EvidenceKind::Commit]),
            ..crate::validate::Options::default()
        };
        assert!(
            switched_off(&plan, &everything_on).is_empty(),
            "nothing is off when everything is on"
        );

        // And the skip is observable: flipping one back off is the only
        // difference, and exactly one key appears.
        let one_off = crate::validate::Options {
            require_effort: false,
            ..everything_on
        };
        let off = switched_off(&plan, &one_off);
        assert_eq!(
            off.values().collect::<std::collections::BTreeSet<_>>(),
            [&"require_effort"].into_iter().collect(),
            "{off:?}"
        );
    }

    /// The other half: a rule that ran is unchanged, so this reads as it did for
    /// every plan the switch does not touch.
    #[test]
    fn a_rule_that_ran_is_reported_exactly_as_before() {
        let plan = plan_of(vec![live("A-1")]);
        let tasks: Vec<&Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
        let bare = of(&tasks);
        let with_none = of_with(&tasks, &std::collections::BTreeMap::new());
        assert_eq!(bare, with_none);
        assert!(
            bare.iter().all(|r| r.off.is_none()),
            "nothing is off when nothing was measured as off"
        );
    }
}
