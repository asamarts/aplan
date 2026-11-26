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

use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostic::{Binding, ResolverSwitch, Rule, SwitchDisposition, ValidationSwitch};
use crate::graph::Graph;
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

/// One rule entry audited by [`audit_switch_dispositions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchCatalogEntry {
    pub code: &'static str,
    pub disposition: SwitchDisposition,
}

/// Audit rule switch dispositions against the declared switch catalogues and
/// `.aplan.yml`'s accepted key schema (`A2-135`).
///
/// Returns concrete failure descriptions for every defect found:
/// - missing rule dispositions (a rule in [`Rule::ALL`] absent from `entries`);
/// - duplicate rule dispositions (a rule code listed more than once);
/// - unknown rule codes in `entries`;
/// - duplicate `config_key` or `options_field` entries across switches;
/// - stale switch config keys absent from `accepted_config_keys`;
/// - stale/orphan switches that no rule in `entries` references.
#[must_use]
pub fn audit_switch_dispositions(
    entries: &[SwitchCatalogEntry],
    validation_switches: &[ValidationSwitch],
    resolver_switches: &[ResolverSwitch],
    accepted_config_keys: &[String],
) -> Vec<String> {
    let mut failures = Vec::new();
    let accepted: BTreeSet<&str> = accepted_config_keys.iter().map(String::as_str).collect();

    let mut seen_codes = BTreeSet::new();
    let mut referenced_validation = BTreeSet::new();
    let mut referenced_resolver = BTreeSet::new();
    for entry in entries {
        if !seen_codes.insert(entry.code) {
            failures.push(format!("duplicate switch disposition for `{}`", entry.code));
        }
        if Rule::from_code(entry.code).is_none() {
            failures.push(format!(
                "stale switch disposition names unknown rule `{}`",
                entry.code
            ));
        }
        match entry.disposition {
            SwitchDisposition::ValidationSwitch(sw) => {
                referenced_validation.insert(sw);
                if !validation_switches.contains(&sw) {
                    failures.push(format!(
                        "`{}` references undeclared validation switch `{}`",
                        entry.code,
                        sw.config_key()
                    ));
                }
            }
            SwitchDisposition::ResolverSwitch(sw) => {
                referenced_resolver.insert(sw);
                if !resolver_switches.contains(&sw) {
                    failures.push(format!(
                        "`{}` references undeclared resolver switch `{sw:?}`",
                        entry.code
                    ));
                }
            }
            SwitchDisposition::Unconditional | SwitchDisposition::External => {}
        }
    }

    for &rule in Rule::ALL {
        if !seen_codes.contains(rule.code()) {
            failures.push(format!(
                "missing switch disposition for `{}` ({})",
                rule.code(),
                rule.title()
            ));
        }
    }

    let mut seen_validation_keys = BTreeSet::new();
    let mut seen_options_fields = BTreeSet::new();
    for &sw in validation_switches {
        let key = sw.config_key();
        if !seen_validation_keys.insert(key) {
            failures.push(format!("duplicate validation switch config key `{key}`"));
        }
        let field = sw.options_field();
        if !seen_options_fields.insert(field) {
            failures.push(format!(
                "duplicate validation switch options field `{field}`"
            ));
        }
        if !accepted.contains(key) {
            failures.push(format!(
                "stale validation switch key `{key}` is not in `.aplan.yml`'s accepted keys"
            ));
        }
        if !referenced_validation.contains(&sw) {
            failures.push(format!(
                "stale validation switch `{key}` is referenced by no rule in the catalogue"
            ));
        }
    }

    let mut seen_resolvers = BTreeSet::new();
    for &sw in resolver_switches {
        if !seen_resolvers.insert(sw) {
            failures.push(format!("duplicate resolver switch `{sw:?}`"));
        }
        for &key in sw.config_keys() {
            if !accepted.contains(key) {
                failures.push(format!(
                    "stale resolver switch key `{key}` is not in `.aplan.yml`'s accepted keys"
                ));
            }
        }
        if !referenced_resolver.contains(&sw) {
            failures.push(format!(
                "stale resolver switch `{sw:?}` is referenced by no rule in the catalogue"
            ));
        }
    }

    failures
}

/// Which rules the configuration has switched off, and the key that did it,
/// derived from one single exhaustive validation pass (`A1-269`, `A2-135`).
///
/// Builds one dependency graph and parses each distinct traced document at most
/// once, then projects the pass's typed [`crate::validate::ValidationObservations`]
/// onto the rule catalogue's [`SwitchDisposition`] mapping.
#[must_use]
pub fn switched_off(
    plan: &crate::load::Plan,
    opts: &crate::validate::Options,
) -> BTreeMap<&'static str, &'static str> {
    crate::validate::validate_with_observations(plan, opts)
        .1
        .switched_off()
}

/// Same as [`switched_off`], reusing graph state the enclosing operation
/// already constructed so commands such as `aplan vacuity --cut ...` construct
/// exactly one graph per snapshot (`A2-135`).
#[must_use]
pub fn switched_off_with_graph(
    plan: &crate::load::Plan,
    opts: &crate::validate::Options,
    graph: &Graph<'_>,
) -> BTreeMap<&'static str, &'static str> {
    crate::validate::validate_with_graph_and_observations(plan, opts, graph)
        .1
        .switched_off()
}

/// Which `verify`-tier rules a repository's resolver configuration leaves
/// inactive, derived directly from [`Rule::ALL`] and [`Rule::switch_disposition`]
/// (`A1-274`, `A2-135`).
#[must_use]
pub fn resolver_switched_off(
    has_test_runner: bool,
    test_claims_locality: bool,
    has_url_checker: bool,
) -> BTreeMap<&'static str, &'static str> {
    let test_key = crate::task::EvidenceConfiguredResolver::Test.config_key();
    let url_key = crate::task::EvidenceConfiguredResolver::Url.config_key();
    let mut out = BTreeMap::new();
    for &rule in Rule::ALL {
        match rule.switch_disposition() {
            SwitchDisposition::ResolverSwitch(ResolverSwitch::Test) if !has_test_runner => {
                out.insert(rule.code(), test_key);
            }
            SwitchDisposition::ResolverSwitch(ResolverSwitch::TestLocality) => {
                if !has_test_runner {
                    out.insert(rule.code(), test_key);
                } else if !test_claims_locality {
                    out.insert(rule.code(), "test.covers");
                }
            }
            SwitchDisposition::ResolverSwitch(ResolverSwitch::UrlCheck) if !has_url_checker => {
                out.insert(rule.code(), url_key);
            }
            SwitchDisposition::ResolverSwitch(_)
            | SwitchDisposition::ValidationSwitch(_)
            | SwitchDisposition::Unconditional
            | SwitchDisposition::External => {}
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
            closed_at: None,
            closed_at_source: None,
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
            closed_at: None,
            closed_at_source: None,
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

    /// Flipping `evidence.require_min_strength` on during a vacuity probe must
    /// not fabricate an `APLAN059` finding for a repository that has no `test`
    /// runner configured and never asked for `test` evidence (`A2-025`).
    #[test]
    fn probing_min_strength_does_not_report_policy_evidence_unresolvable_as_off() {
        let plan = plan_of(vec![live("A-1")]);
        let opts = crate::validate::Options {
            configured_resolvers: Some(std::collections::BTreeSet::new()),
            ..crate::validate::Options::default()
        };
        let off = switched_off(&plan, &opts);
        assert!(
            !off.contains_key("APLAN059"),
            "APLAN059 is not switched off by evidence.require_min_strength: {off:?}"
        );
    }

    type OracleSwitch = (&'static str, fn(&mut crate::validate::Options));

    /// Reference multi-pass implementation (the pre-`A2-135` path) retained in
    /// tests only as an equivalence oracle and as the mutation target proving
    /// the work-count check fails if repeated validation is restored.
    fn repeated_validator_switched_off(
        plan: &crate::load::Plan,
        opts: &crate::validate::Options,
    ) -> BTreeMap<&'static str, &'static str> {
        let switches: &[OracleSwitch] = &[
            ("require_effort", |o| o.require_effort = true),
            ("detect_drift", |o| o.detect_drift = true),
            ("require_cut_membership", |o| {
                o.require_cut_membership = true;
            }),
            ("evidence.require_min_strength", |o| {
                if o.require_min_strength.as_deref()
                    != Some(crate::validate::VACUITY_MIN_STRENGTH_ON)
                {
                    o.require_min_strength =
                        Some(crate::validate::VACUITY_MIN_STRENGTH_ON.to_vec());
                    o.configured_resolvers = None;
                }
            }),
            ("version.require_impact", |o| {
                o.require_impact = Some(crate::task::VersionFloors::default());
            }),
        ];
        let codes = |o: &crate::validate::Options| -> BTreeSet<String> {
            crate::validate::validate(plan, o)
                .diagnostics
                .into_iter()
                .map(|d| d.code.to_string())
                .collect()
        };
        let now = codes(opts);
        let mut out = BTreeMap::new();
        for (key, turn_on) in switches {
            let mut with = opts.clone();
            turn_on(&mut with);
            if with == *opts {
                continue;
            }
            for code in codes(&with).difference(&now) {
                if let Some(rule) = Rule::ALL.iter().find(|r| r.code() == code) {
                    out.entry(rule.code()).or_insert(*key);
                }
            }
        }
        out
    }

    fn verify_single_pass_work(
        plan: &crate::load::Plan,
        opts: &crate::validate::Options,
        max_markdown_parses: usize,
        max_structured_parses: usize,
        compute: impl FnOnce(
            &crate::load::Plan,
            &crate::validate::Options,
        ) -> BTreeMap<&'static str, &'static str>,
    ) -> Result<BTreeMap<&'static str, &'static str>, String> {
        let graphs_before = crate::graph_constructions();
        let traces_before = crate::trace_document_work();
        let off = compute(plan, opts);
        let graph_builds = crate::graph_constructions() - graphs_before;
        let traces_after = crate::trace_document_work();
        let markdown_parses = traces_after.markdown_parses - traces_before.markdown_parses;
        let structured_parses = traces_after.structured_parses - traces_before.structured_parses;
        let mut problems = Vec::new();
        if graph_builds != 1 {
            problems.push(format!("expected 1 graph build, observed {graph_builds}"));
        }
        if markdown_parses > max_markdown_parses {
            problems.push(format!(
                "expected at most {max_markdown_parses} Markdown parse(s), observed \
                 {markdown_parses}"
            ));
        }
        if structured_parses > max_structured_parses {
            problems.push(format!(
                "expected at most {max_structured_parses} structured parse(s), observed \
                 {structured_parses}"
            ));
        }
        if problems.is_empty() {
            Ok(off)
        } else {
            Err(problems.join("; "))
        }
    }

    fn shared_trace_corpus() -> (tempfile::TempDir, crate::load::Plan) {
        let dir = tempfile::Builder::new()
            .prefix("vacuity-single-pass-")
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let md_path = dir.path().join("docs/spec.md");
        let yaml_path = dir.path().join("docs/spec.yaml");
        std::fs::create_dir_all(dir.path().join("docs")).expect("docs");
        std::fs::write(
            &md_path,
            "# Spec\n\n## Alpha\n\nChanged body.\n\n| Item | Value |\n|---|---|\n| beta | row |\n",
        )
        .expect("md");
        std::fs::write(&yaml_path, "service:\n  mode: strict\n").expect("yaml");

        // Task 1: live, missing effort (APLAN009), in no cut (APLAN022).
        let t_live = live("A-1");

        // Task 2: done with note evidence only (APLAN017), unexplained `none`
        // version impact (APLAN044), drifted Markdown stamp (APLAN018).
        let mut t_drift = live("A-2");
        t_drift.status = Status::Done;
        t_drift.effort_days = Some(1.0);
        t_drift.evidence = vec![crate::task::Evidence {
            kind: crate::task::EvidenceKind::Note,
            reference: "reviewed".to_string(),
            captured: Some("2026-10-01".to_string()),
        }];
        t_drift.version_impact = Some(crate::task::VersionImpact::None);
        t_drift.traces = vec![
            "docs/spec.md#alpha".to_string(),
            "docs/spec.yaml#/service".to_string(),
        ];
        t_drift.trace_fingerprints.insert(
            "docs/spec.md#alpha".to_string(),
            "deadbeefdeadbeef".to_string(),
        );
        t_drift.trace_fingerprints.insert(
            "docs/spec.yaml#/service".to_string(),
            "0000000000000000".to_string(),
        );

        // Task 3: done, shares docs/spec.md with unstamped trace (APLAN028) and
        // kind-retargeted stamp on `#beta` (`heading:` stamp now resolving to a
        // table row -> APLAN060).
        let mut t_multi = live("A-3");
        t_multi.status = Status::Done;
        t_multi.effort_days = Some(1.0);
        t_multi.evidence = vec![crate::task::Evidence {
            kind: crate::task::EvidenceKind::File,
            reference: "docs/spec.md".to_string(),
            captured: None,
        }];
        t_multi.traces = vec![
            "docs/spec.md#alpha".to_string(),
            "docs/spec.md#beta".to_string(),
            "docs/spec.yaml#/service".to_string(),
        ];
        t_multi.trace_fingerprints.insert(
            "docs/spec.md#beta".to_string(),
            "heading:deadbeefdeadbeef".to_string(),
        );

        (dir, plan_of(vec![t_live, t_drift, t_multi]))
    }

    /// `A2-135` AC 1: one validation pass distinguishes inapplicable, enabled
    /// clean, enabled with findings, and disabled (both clean and suppressed).
    #[test]
    fn single_pass_observations_distinguish_all_four_activation_states() {
        use crate::validate::{CheckActivation, validate_with_observations};

        // Empty plan: every switch-gated rule is inapplicable.
        let empty = plan_of(Vec::new());
        let (_, empty_obs) =
            validate_with_observations(&empty, &crate::validate::Options::default());
        for &switch in ValidationSwitch::ALL {
            for rule in switch.rules() {
                assert_eq!(
                    empty_obs.activation(rule),
                    Some(CheckActivation::Inapplicable),
                    "{} should be Inapplicable on an empty plan",
                    rule.code()
                );
            }
        }

        // Enabled clean vs enabled with findings vs disabled clean vs disabled
        // with suppressed findings.
        let mut clean_live = live("C-1");
        clean_live.effort_days = Some(2.0);
        let mut cut = live("M-1");
        cut.tags = vec!["milestone".to_string()];
        cut.blocked_by = vec!["C-1".to_string()];
        let clean_plan = plan_of(vec![clean_live, cut]);

        let all_on = crate::validate::Options {
            require_effort: true,
            require_cut_membership: true,
            ..crate::validate::Options::default()
        };
        let (_, on_obs) = validate_with_observations(&clean_plan, &all_on);
        assert_eq!(
            on_obs.activation(Rule::MissingEffort),
            Some(CheckActivation::EnabledNoFindings)
        );
        assert_eq!(
            on_obs.activation(Rule::MembershipTotality),
            Some(CheckActivation::EnabledNoFindings)
        );

        let all_off = crate::validate::Options {
            require_effort: false,
            require_cut_membership: false,
            ..crate::validate::Options::default()
        };
        let (_, off_clean_obs) = validate_with_observations(&clean_plan, &all_off);
        assert_eq!(
            off_clean_obs.activation(Rule::MissingEffort),
            Some(CheckActivation::Disabled {
                suppressed_findings: 0
            })
        );
        assert_eq!(
            off_clean_obs.activation(Rule::MembershipTotality),
            Some(CheckActivation::Disabled {
                suppressed_findings: 0
            })
        );
        assert!(off_clean_obs.switched_off().is_empty());

        let dirty_plan = plan_of(vec![live("D-1")]);
        let (on_report, on_dirty_obs) = validate_with_observations(&dirty_plan, &all_on);
        assert!(on_report.diagnostics.iter().any(|d| d.code == "APLAN009"));
        assert_eq!(
            on_dirty_obs.activation(Rule::MissingEffort),
            Some(CheckActivation::EnabledWithFindings { findings: 1 })
        );
        assert_eq!(
            on_dirty_obs.activation(Rule::MembershipTotality),
            Some(CheckActivation::EnabledWithFindings { findings: 1 })
        );

        let (off_report, off_dirty_obs) = validate_with_observations(&dirty_plan, &all_off);
        assert!(
            off_report
                .diagnostics
                .iter()
                .all(|d| d.code != "APLAN009" && d.code != "APLAN022"),
            "disabled checks must not leak diagnostics into the report: {:?}",
            off_report.diagnostics
        );
        assert_eq!(
            off_dirty_obs.activation(Rule::MissingEffort),
            Some(CheckActivation::Disabled {
                suppressed_findings: 1
            })
        );
        assert_eq!(
            off_dirty_obs.activation(Rule::MembershipTotality),
            Some(CheckActivation::Disabled {
                suppressed_findings: 1
            })
        );
        assert_eq!(
            off_dirty_obs.switched_off().get("APLAN009"),
            Some(&"require_effort")
        );
        assert_eq!(
            off_dirty_obs.switched_off().get("APLAN022"),
            Some(&"require_cut_membership")
        );
    }

    /// `A2-135` AC 3, AC 4 and AC 5: single-pass `switched_off` matches the
    /// repeated-validator oracle across all switches (including three rules
    /// behind `detect_drift`), performs one graph build and one parse per
    /// shared trace document, and restoring the repeated-validator path fails
    /// the exact same work-count proof.
    #[test]
    fn single_pass_vacuity_proves_work_bounds_and_rejects_repeated_validator_mutant() {
        let (dir, plan) = shared_trace_corpus();
        let all_off = crate::validate::Options {
            require_effort: false,
            detect_drift: false,
            require_cut_membership: false,
            require_min_strength: None,
            require_impact: None,
            traces_root: Some(dir.path().to_path_buf()),
            ..crate::validate::Options::default()
        };

        let single_pass = verify_single_pass_work(&plan, &all_off, 1, 1, switched_off)
            .expect("single-pass vacuity must stay within 1 graph build and 1 parse per trace doc");
        let expected_oracle = repeated_validator_switched_off(&plan, &all_off);
        assert_eq!(
            single_pass, expected_oracle,
            "single-pass vacuity must agree with the reference oracle across all 7 switch-gated \
             rules (including APLAN018, APLAN028 and APLAN060 under detect_drift)"
        );
        for (code, key) in [
            ("APLAN009", "require_effort"),
            ("APLAN017", "evidence.require_min_strength"),
            ("APLAN018", "detect_drift"),
            ("APLAN022", "require_cut_membership"),
            ("APLAN028", "detect_drift"),
            ("APLAN044", "version.require_impact"),
            ("APLAN060", "detect_drift"),
        ] {
            assert_eq!(
                single_pass.get(code),
                Some(&key),
                "missing expected rule `{code}` -> `{key}` in {single_pass:?}"
            );
        }

        // Mutation test (AC 5): restoring the repeated-validator path fails
        // the exact same work-count proof on both graph builds and trace parses.
        let mutant_err =
            verify_single_pass_work(&plan, &all_off, 1, 1, repeated_validator_switched_off)
                .expect_err("restoring the repeated-validator path must fail the work-count proof");
        assert!(
            mutant_err.contains("graph build")
                && mutant_err.contains("Markdown parse")
                && mutant_err.contains("structured parse"),
            "mutant should trip graph and trace work counters, got: {mutant_err}"
        );
    }

    /// Scoped evidence policies and custom version floors preserve exact
    /// differential parity with the reference oracle (`A2-135` AC 4).
    #[test]
    fn single_pass_matches_oracle_under_scoped_policies_and_custom_floors() {
        let (dir, plan) = shared_trace_corpus();
        let scoped = crate::validate::EvidenceScope {
            selector: crate::select::Selector {
                tags: vec!["scoped".to_string()],
                ..crate::select::Selector::default()
            },
            named_dimensions: 1,
            require_min_strength: Some(vec![crate::task::EvidenceKind::Test]),
            exempt_tags: None,
        };
        let configurations = [
            crate::validate::Options {
                traces_root: Some(dir.path().to_path_buf()),
                ..crate::validate::Options::default()
            },
            crate::validate::Options {
                require_effort: false,
                detect_drift: true,
                require_cut_membership: true,
                require_min_strength: Some(vec![crate::task::EvidenceKind::Note]),
                require_impact: Some(crate::task::VersionFloors {
                    task: crate::task::VersionImpact::None,
                    cut: crate::task::VersionImpact::None,
                    release: crate::task::VersionImpact::None,
                }),
                traces_root: Some(dir.path().to_path_buf()),
                evidence_scopes: vec![scoped.clone()],
                ..crate::validate::Options::default()
            },
            crate::validate::Options {
                require_effort: true,
                detect_drift: true,
                require_cut_membership: true,
                require_min_strength: Some(vec![crate::task::EvidenceKind::Test]),
                require_impact: Some(crate::task::VersionFloors::default()),
                traces_root: Some(dir.path().to_path_buf()),
                evidence_scopes: vec![scoped],
                ..crate::validate::Options::default()
            },
        ];
        for opts in &configurations {
            assert_eq!(
                switched_off(&plan, opts),
                repeated_validator_switched_off(&plan, opts),
                "single-pass disagreed with oracle for {opts:?}"
            );
        }
    }

    fn clean_all_switch_plan() -> (tempfile::TempDir, crate::load::Plan) {
        let dir = tempfile::Builder::new()
            .prefix("vacuity-clean-states-")
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join("docs")).expect("docs");
        let md = "# Spec\n\n## Alpha\n\nStable.\n";
        std::fs::write(dir.path().join("docs/spec.md"), md).expect("md");
        let stamp = crate::stamp_in("docs/spec.md", md, "alpha").expect("stamp");

        let mut cut = live("M-1");
        cut.tags = vec!["milestone".to_string()];
        cut.blocked_by = vec!["C-1".to_string(), "C-2".to_string()];

        let mut c_live = live("C-1");
        c_live.effort_days = Some(1.0);

        let mut c_done = live("C-2");
        c_done.status = Status::Done;
        c_done.effort_days = Some(1.0);
        c_done.version_impact = Some(crate::task::VersionImpact::Fix);
        c_done.evidence = vec![crate::task::Evidence {
            kind: crate::task::EvidenceKind::Test,
            reference: "unit_ok".to_string(),
            captured: None,
        }];
        c_done.traces = vec!["docs/spec.md#alpha".to_string()];
        c_done
            .trace_fingerprints
            .insert("docs/spec.md#alpha".to_string(), stamp);

        (dir, plan_of(vec![cut, c_live, c_done]))
    }

    /// A scoped empty-list waiver (`policy[].evidence.require_min_strength: []`)
    /// makes `APLAN017` inapplicable on that task rather than reporting the
    /// whole-plan switch as disabled, and clean subjects across all 7
    /// switch-gated rules cleanly distinguish `EnabledNoFindings` from
    /// `Disabled { suppressed_findings: 0 }`.
    #[test]
    fn clean_subjects_and_scoped_waivers_report_exact_activation_states() {
        use crate::validate::{CheckActivation, validate_with_observations};

        let (dir, clean_plan) = clean_all_switch_plan();
        let all_on = crate::validate::Options {
            require_effort: true,
            detect_drift: true,
            require_cut_membership: true,
            require_min_strength: Some(crate::validate::VACUITY_MIN_STRENGTH_ON.to_vec()),
            require_impact: Some(crate::task::VersionFloors::default()),
            traces_root: Some(dir.path().to_path_buf()),
            ..crate::validate::Options::default()
        };
        let (_, obs_on) = validate_with_observations(&clean_plan, &all_on);
        for &switch in ValidationSwitch::ALL {
            for rule in switch.rules() {
                assert_eq!(
                    obs_on.activation(rule),
                    Some(CheckActivation::EnabledNoFindings),
                    "{} should be EnabledNoFindings on clean subjects",
                    rule.code()
                );
            }
        }

        let all_off = crate::validate::Options {
            require_effort: false,
            detect_drift: false,
            require_cut_membership: false,
            require_min_strength: None,
            require_impact: None,
            traces_root: Some(dir.path().to_path_buf()),
            ..crate::validate::Options::default()
        };
        let (_, obs_off) = validate_with_observations(&clean_plan, &all_off);
        for &switch in ValidationSwitch::ALL {
            for rule in switch.rules() {
                assert_eq!(
                    obs_off.activation(rule),
                    Some(CheckActivation::Disabled {
                        suppressed_findings: 0
                    }),
                    "{} should be Disabled(0) on clean subjects with switch off",
                    rule.code()
                );
            }
        }
        assert!(obs_off.switched_off().is_empty());

        // Scoped empty-list waiver with whole-plan switch enabled -> Inapplicable.
        let mut waived = live("W-1");
        waived.status = Status::Done;
        waived.effort_days = Some(1.0);
        waived.tags = vec!["waived".to_string()];
        waived.evidence = vec![crate::task::Evidence {
            kind: crate::task::EvidenceKind::Note,
            reference: "manual signoff".to_string(),
            captured: Some("2026-10-01".to_string()),
        }];
        let waived_plan = plan_of(vec![waived]);
        let waived_opts = crate::validate::Options {
            require_min_strength: Some(crate::validate::VACUITY_MIN_STRENGTH_ON.to_vec()),
            evidence_scopes: vec![crate::validate::EvidenceScope {
                selector: crate::select::Selector {
                    tags: vec!["waived".to_string()],
                    ..crate::select::Selector::default()
                },
                named_dimensions: 1,
                require_min_strength: Some(Vec::new()),
                exempt_tags: None,
            }],
            ..crate::validate::Options::default()
        };
        let (_, waived_obs) = validate_with_observations(&waived_plan, &waived_opts);
        assert_eq!(
            waived_obs.activation(Rule::WeakEvidence),
            Some(CheckActivation::Inapplicable),
            "scoped empty-list waiver must not misreport an enabled switch as Disabled"
        );
    }

    /// `A2-135` AC 2 negative tests: missing, duplicate, unknown, stale and
    /// orphan dispositions all fail `audit_switch_dispositions`.
    #[test]
    fn switch_catalogue_audit_rejects_missing_duplicate_stale_and_orphan_entries() {
        let live: Vec<SwitchCatalogEntry> = Rule::ALL
            .iter()
            .map(|&rule| SwitchCatalogEntry {
                code: rule.code(),
                disposition: rule.switch_disposition(),
            })
            .collect();
        let accepted: Vec<String> = ValidationSwitch::ALL
            .iter()
            .map(|sw| sw.config_key().to_string())
            .chain(
                ResolverSwitch::ALL
                    .iter()
                    .flat_map(|sw| sw.config_keys().iter().map(|k| (*k).to_string())),
            )
            .collect();
        assert!(
            audit_switch_dispositions(&live, ValidationSwitch::ALL, ResolverSwitch::ALL, &accepted)
                .is_empty()
        );

        let missing = audit_switch_dispositions(
            &live[1..],
            ValidationSwitch::ALL,
            ResolverSwitch::ALL,
            &accepted,
        );
        assert!(missing.iter().any(|m| m.contains("missing")), "{missing:?}");

        let mut dup = live.clone();
        dup.push(live[0]);
        let dup_err =
            audit_switch_dispositions(&dup, ValidationSwitch::ALL, ResolverSwitch::ALL, &accepted);
        assert!(
            dup_err.iter().any(|m| m.contains("duplicate")),
            "{dup_err:?}"
        );

        let dup_switches = [
            ValidationSwitch::RequireEffort,
            ValidationSwitch::RequireEffort,
            ValidationSwitch::DetectDrift,
            ValidationSwitch::RequireCutMembership,
            ValidationSwitch::RequireMinStrength,
            ValidationSwitch::RequireImpact,
        ];
        let dup_sw_err =
            audit_switch_dispositions(&live, &dup_switches, ResolverSwitch::ALL, &accepted);
        assert!(
            dup_sw_err.iter().any(|m| m.contains("duplicate")),
            "{dup_sw_err:?}"
        );

        let mut unknown = live.clone();
        unknown.push(SwitchCatalogEntry {
            code: "APLAN999",
            disposition: SwitchDisposition::Unconditional,
        });
        let unknown_err = audit_switch_dispositions(
            &unknown,
            ValidationSwitch::ALL,
            ResolverSwitch::ALL,
            &accepted,
        );
        assert!(
            unknown_err.iter().any(|m| m.contains("stale")),
            "{unknown_err:?}"
        );

        let orphan_entries: Vec<SwitchCatalogEntry> = live
            .iter()
            .map(|entry| {
                if entry.disposition
                    == SwitchDisposition::ValidationSwitch(ValidationSwitch::RequireEffort)
                {
                    SwitchCatalogEntry {
                        code: entry.code,
                        disposition: SwitchDisposition::Unconditional,
                    }
                } else {
                    *entry
                }
            })
            .collect();
        let orphan_err = audit_switch_dispositions(
            &orphan_entries,
            ValidationSwitch::ALL,
            ResolverSwitch::ALL,
            &accepted,
        );
        assert!(
            orphan_err
                .iter()
                .any(|m| m.contains("stale") && m.contains("require_effort")),
            "{orphan_err:?}"
        );
    }

    #[test]
    fn switch_catalogue_audit_rejects_undeclared_duplicate_stale_and_orphan_resolver_switches() {
        let live: Vec<SwitchCatalogEntry> = Rule::ALL
            .iter()
            .map(|&rule| SwitchCatalogEntry {
                code: rule.code(),
                disposition: rule.switch_disposition(),
            })
            .collect();
        let accepted: Vec<String> = ValidationSwitch::ALL
            .iter()
            .map(|sw| sw.config_key().to_string())
            .chain(
                ResolverSwitch::ALL
                    .iter()
                    .flat_map(|sw| sw.config_keys().iter().map(|k| (*k).to_string())),
            )
            .collect();

        // Undeclared validation switch, undeclared resolver switch, duplicate resolver switch,
        // stale resolver switch key, and orphan resolver switch must all be reported.
        let undeclared_val = audit_switch_dispositions(&live, &[], ResolverSwitch::ALL, &accepted);
        assert!(
            undeclared_val
                .iter()
                .any(|m| m.contains("undeclared validation switch")),
            "{undeclared_val:?}"
        );
        let undeclared_res =
            audit_switch_dispositions(&live, ValidationSwitch::ALL, &[], &accepted);
        assert!(
            undeclared_res
                .iter()
                .any(|m| m.contains("undeclared resolver switch")),
            "{undeclared_res:?}"
        );
        let dup_res = audit_switch_dispositions(
            &live,
            ValidationSwitch::ALL,
            &[
                ResolverSwitch::Test,
                ResolverSwitch::Test,
                ResolverSwitch::TestLocality,
                ResolverSwitch::UrlCheck,
            ],
            &accepted,
        );
        assert!(
            dup_res
                .iter()
                .any(|m| m.contains("duplicate resolver switch")),
            "{dup_res:?}"
        );
        let no_url_keys: Vec<String> = accepted
            .iter()
            .filter(|k| k.as_str() != "url.check")
            .cloned()
            .collect();
        let stale_res_key = audit_switch_dispositions(
            &live,
            ValidationSwitch::ALL,
            ResolverSwitch::ALL,
            &no_url_keys,
        );
        assert!(
            stale_res_key
                .iter()
                .any(|m| m.contains("stale resolver switch key") && m.contains("url.check")),
            "{stale_res_key:?}"
        );
        let no_url_entries: Vec<SwitchCatalogEntry> = live
            .iter()
            .map(|entry| {
                if entry.disposition == SwitchDisposition::ResolverSwitch(ResolverSwitch::UrlCheck)
                {
                    SwitchCatalogEntry {
                        code: entry.code,
                        disposition: SwitchDisposition::External,
                    }
                } else {
                    *entry
                }
            })
            .collect();
        let orphan_res = audit_switch_dispositions(
            &no_url_entries,
            ValidationSwitch::ALL,
            ResolverSwitch::ALL,
            &accepted,
        );
        assert!(
            orphan_res
                .iter()
                .any(|m| m.contains("stale resolver switch") && m.contains("UrlCheck")),
            "{orphan_res:?}"
        );
    }
}
