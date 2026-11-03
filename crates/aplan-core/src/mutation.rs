//! What a mutation would do to the plan, judged before it does it (`A1-113`).
//!
//! Outside this crate's own tests the rule engine had four call sites, and all
//! four were read commands: `check` and `verify`, on both surfaces. **No mutating
//! command ran the rules on the plan it was about to write.** Each hand-rolled a
//! partial pre-check instead, so `cut::check_target` and `mv`'s both-ends check
//! were two different partial answers to "is this placement legal", and the
//! fourth pilot's complaint about `mv` was never that the plan stayed broken but
//! that the command which broke it said nothing (pre-public adoption evidence).
//!
//! The check is a **delta**, not a verdict on the whole plan. A plan may already
//! be dirty, and reporting everything would make every mutation on an imperfect
//! plan refuse, which no adopter would tolerate: it would also be right about the
//! rule and wrong about the cause, which is the shape `p00025` item 14 names.
//!
//! It lives here rather than in the binary crate because the agent surface would
//! otherwise keep the old behaviour, which is the fifth time shared behaviour has
//! been found on the wrong side of that boundary (`A1-100`, `A1-101`, `A1-102`,
//! `A1-103`).

use std::collections::BTreeMap;

use crate::diagnostic::{Diagnostic, Severity, Verdict};
use crate::load::{LoadedTask, Plan};
use crate::task::Task;
use crate::validate::{Options, validate};

/// What the hermetic rules say about a mutation that has not happened yet.
#[derive(Debug, Default)]
pub struct Delta {
    /// Diagnostics the post-state has that the pre-state did not.
    pub added: Vec<Diagnostic>,
}

/// Which half of the bootstrap deadlock a refusal is, when it is one (`A1-292`).
///
/// A plan with no cuts cannot be built up a step at a time wherever an empty cut
/// blocks: a first task belongs to no cut (`APLAN022`) and a first cut has no
/// members (`APLAN021`), so neither can be added first. The way through is one
/// atomic change holding both.
///
/// Classified here rather than at the surface that speaks, because the question
/// is which rule fired and whether this repository's policy stops it -- both
/// facts about the rules. The wording belongs to the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bootstrap {
    /// A cut was added with no members, and this policy refuses that.
    EmptyCut,
    /// A task was added into a plan with no cut to join, and an empty cut would
    /// be refused too, so there is no order that works.
    NoCutAndNoneCanBeEmpty,
    /// A task was added into a plan with no cut to join, and an empty cut is a
    /// warning this policy allows: the cut can be made first.
    NoCut,
    /// Not a bootstrap problem.
    No,
}

impl Delta {
    /// Whether this refusal is half of the bootstrap deadlock, and which half.
    ///
    /// `before` and `after` rather than booleans, because all three questions
    /// this asks are easy to get wrong at a call site: whether the plan had any
    /// cut, whether the milestone is one this change is *creating*, and whether
    /// that milestone actually has live work rather than merely sharing
    /// `APLAN021` with a different shape.
    #[must_use]
    pub fn bootstrap(&self, before: &Plan, after: &Plan, policy: &Verdict) -> Bootstrap {
        use crate::diagnostic::code;

        let fired = |c: &str| {
            self.added
                .iter()
                .any(|d| d.code == c && !policy.is_advisory(c))
        };
        let known = |id: &str| before.tasks.iter().any(|lt| lt.task.id == id);
        // **Only a cut this change creates.** `mv` emptying an existing cut
        // raises the same `APLAN021`, and telling its author that "a cut is
        // created with its work" is advice about an operation they are not
        // performing -- the rule's own remedy, move work in or cancel the cut,
        // is the complete answer there (`A1-292`, audit).
        let creates_an_empty_cut = self.added.iter().any(|d| {
            d.code == code::MILESTONE_HYGIENE
                && !policy.is_advisory(d.code)
                && d.task.as_deref().is_some_and(|id| {
                    !known(id)
                        && after.tasks.iter().any(|lt| {
                            let cut = &lt.task;
                            cut.id == id
                                && cut.is_milestone()
                                && !cut.is_release()
                                && cut.contains.is_empty()
                                && cut.status != crate::task::Status::Dropped
                                && !cut.blocked_by.iter().any(|member| {
                                    after.tasks.iter().any(|candidate| {
                                        candidate.task.id == *member
                                            && !candidate.task.is_milestone()
                                            && candidate.task.status != crate::task::Status::Dropped
                                    })
                                })
                        })
                })
        });
        if creates_an_empty_cut {
            return Bootstrap::EmptyCut;
        }
        if !fired(code::MEMBERSHIP_TOTALITY) || before.tasks.iter().any(|lt| lt.task.is_milestone())
        {
            return Bootstrap::No;
        }
        // `APLAN021` is a warning, so an empty cut is stopped only where warnings
        // block. That is the whole of why the advice cannot be one sentence: the
        // two-step is the simpler answer wherever it works.
        if policy.deny_warnings && !policy.is_advisory(code::MILESTONE_HYGIENE) {
            Bootstrap::NoCutAndNoneCanBeEmpty
        } else {
            Bootstrap::NoCut
        }
    }

    /// Whether the mutation must be refused rather than written.
    ///
    /// A new *error* refuses: the mutation would produce a plan `check` rejects,
    /// and writing it would leave the author to discover that on the next run. A
    /// new *warning* is reported and lets `deny_warnings` decide, because
    /// refusing one would block a legitimate two-step rearrangement: `mv`
    /// emptying a cut is `APLAN021`, and emptying it is how you refill it
    /// somewhere else.
    #[must_use]
    pub fn refuses(&self, policy: &Verdict) -> bool {
        self.added
            .iter()
            .any(|d| !policy.is_advisory(d.code) && d.severity == Severity::Error)
            || (policy.deny_warnings
                && self
                    .added
                    .iter()
                    .any(|d| !policy.is_advisory(d.code) && d.severity == Severity::Warning))
    }

    /// Whether an `add` must be refused, including explicit work fields on a
    /// checkpoint this add creates (`A1-369`, `A1-407`).
    ///
    /// APLAN021 remains warning-shaped for ordinary mutations: moving work can
    /// temporarily empty a cut, and warning policy decides whether that
    /// two-step repair is allowed. An add already has all of its input, so a new
    /// milestone carrying `effort_days` or `paths` is invalid author input and
    /// is refused under every policy. The milestone must be absent from
    /// `before`: adding valid work to an existing malformed cut can change its
    /// diagnostic text, but did not introduce its work fields (`A1-406`).
    #[must_use]
    pub fn refuses_add(&self, before: &Plan, after: &Plan, policy: &Verdict) -> bool {
        use crate::diagnostic::code;

        self.refuses(policy)
            || self.added.iter().any(|finding| {
                finding.code == code::MILESTONE_HYGIENE
                    && finding.task.as_deref().is_some_and(|id| {
                        !before.tasks.iter().any(|lt| lt.task.id == id)
                            && after.tasks.iter().any(|lt| {
                                lt.task.id == id
                                    && lt.task.is_milestone()
                                    && (lt.task.effort_days.is_some() || !lt.task.paths.is_empty())
                            })
                    })
            })
    }

    /// The findings, one per line, each carrying the remedy it was written with.
    ///
    /// The `agent_instruction` comes along because a refusal that names a rule
    /// and not the way out is a diagnostic with its most useful half removed,
    /// which is the defect `A1-086` fixed for `check` (`A1-113`).
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.added
            .iter()
            .map(|d| {
                let named = match &d.task {
                    Some(t) => format!("{} {t}: {}", d.code, d.message),
                    None => format!("{}: {}", d.code, d.message),
                };
                match &d.agent_instruction {
                    Some(fix) => format!("{named}\n      fix: {fix}"),
                    None => named,
                }
            })
            .collect()
    }
}

/// What this mutation adds to the plan's diagnostics.
///
/// Counted as a multiset rather than a set: a mutation that produces a *second*
/// instance of a code the plan already had is adding one, and a set difference
/// would call that unchanged.
#[must_use]
pub fn delta(before: &Plan, after: &Plan, opts: &Options) -> Delta {
    Delta {
        added: added(
            &validate(before, opts).diagnostics,
            validate(after, opts).diagnostics,
        ),
    }
}

/// The findings in `after` that `before` did not already have, counting
/// duplicates.
///
/// Split out because it is the claim worth testing directly: no rule in this
/// build produces two findings with the same code, task, file *and* message, so
/// the multiset behaviour cannot be reached through `validate`, and a test that
/// went through it would be exercising a set difference while claiming
/// otherwise. That is what the first version of its test did (`A1-113`).
#[must_use]
pub fn added(before: &[Diagnostic], after: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut had: BTreeMap<String, usize> = BTreeMap::new();
    for d in before {
        *had.entry(key(d)).or_default() += 1;
    }
    let mut out = Vec::new();
    for d in after {
        let k = key(&d);
        match had.get_mut(&k) {
            Some(n) if *n > 0 => *n -= 1,
            _ => out.push(d),
        }
    }
    out
}

/// What makes two diagnostics the same finding.
fn key(d: &Diagnostic) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        d.code,
        d.task.as_deref().unwrap_or(""),
        d.file
            .as_ref()
            .map_or(String::new(), |p| p.display().to_string()),
        d.message
    )
}

/// The task a produced file would load as.
///
/// Parsed from the frontmatter a mutation actually built rather than from a
/// second model of what it meant to build, so a command whose write is wrong is
/// judged on what it would really write.
///
/// # Errors
/// When the content has no frontmatter, or the frontmatter would not load.
pub fn task_of(content: &str) -> Result<Task, String> {
    let (frontmatter, _) = crate::load::split_frontmatter(content)
        .ok_or_else(|| "the rewritten file has no frontmatter".to_string())?;
    yaml_compat::from_str(frontmatter).map_err(|e| format!("the rewritten frontmatter: {e}"))
}

/// A copy of this plan with one task's content replaced.
///
/// The post-state is built from the *frontmatter the mutation produced*, not
/// from a second model of what it meant to do, so a command whose write is wrong
/// is judged on what it would actually write.
#[must_use]
pub fn with_replaced(plan: &Plan, id: &str, task: Task) -> Plan {
    let mut out = clone_of(plan);
    if let Some(slot) = out.tasks.iter_mut().find(|lt| lt.task.id == id) {
        slot.task = task;
    }
    out
}

/// Why this edge is a cut's *membership* rather than a task's ordering, if it is.
///
/// A milestone's `blocked_by` is the list of tasks in that cut (ADR-0008), so an
/// edge whose head is a milestone and whose tail is work adds or removes a
/// member. `mv` maintains that field as a pair of edits with guards on both ends;
/// editing one end of it directly is `mv` with every guard removed, and the two
/// holes were measured rather than assumed (`A1-124`):
///
/// - removing the last work member of a **sequenced** cut leaves `blocked_by`
///   holding its predecessor. `APLAN021` used to see a non-empty list and call
///   that clean; since `A1-134` it counts live work, so the rule reports it and
///   `mv`'s hand-rolled refusal is gone.
/// - adding a member without removing the one the task already has puts it in two
///   cuts, which only `APLAN022` catches, and that rule is off unless
///   `require_cut_membership` is set. In the default configuration, nothing.
///
/// Sequencing one cut after another is the same field and is *not* refused: it is
/// a milestone naming a milestone, `A1-073` made it the supported way to order
/// cuts, and `mv` sends you here to do it.
#[must_use]
pub fn membership_edit(plan: &Plan, id: &str, after: &str, adding: bool) -> Option<String> {
    let head = plan.tasks.iter().find(|lt| lt.task.id == id)?;
    if !head.task.is_milestone() {
        return None;
    }
    // An `after` this plan does not have is not a membership edit: adding one is
    // `APLAN003`, which the delta reports, and removing one is repairing a
    // dangling member, which must stay possible.
    let tail = plan.tasks.iter().find(|lt| lt.task.id == after)?;
    if tail.task.is_milestone() {
        return None;
    }
    Some(if adding {
        format!(
            "`{id}` is a cut, and its `blocked_by` is which tasks are in it, not what it comes \
             after: this would put `{after}` in a second cut, which only `APLAN022` catches and \
             only when `require_cut_membership` is set. Run `aplan mv {after} --cut {id}`, which \
             removes it from the cut it is in now."
        )
    } else {
        format!(
            "`{id}` is a cut, and its `blocked_by` is which tasks are in it: this would take \
             `{after}` out of it without putting it anywhere, and a sequenced cut emptied of work \
             keeps its predecessor in `blocked_by`, so `APLAN021` stays silent. Run `aplan mv \
             {after} --cut <other>`, or `aplan drop` whichever of the two is finished with."
        )
    })
}

/// A copy of this plan with a task added at `path`.
#[must_use]
pub fn with_added(plan: &Plan, path: std::path::PathBuf, task: Task) -> Plan {
    let mut out = clone_of(plan);
    // No `fields`: this task has never been written, so there is no text to
    // point a finding at. A finding about it names the file and stops there,
    // which is the truth (`A1-202`).
    out.tasks.push(LoadedTask {
        task,
        path,
        fields: crate::load::FieldLines::new(),
        // No criteria either, for the same reason: every caller adds a task
        // whose file does not exist yet (`add`, `add --from`, and the successor
        // `supersede` writes), so there is no body to have counted. `APLAN038`
        // reads a count of the body, and counting nothing is what this says
        // (`A1-224`).
        criteria: crate::load::Criteria::default(),
    });
    out
}

/// Refuse a mutation whose target id more than one task declares (`A1-135`).
///
/// Every mutating command finds its target with a linear search for the id, and
/// nothing said that id was unique. Measured on a plan declaring `A-2` twice:
/// `dep add` edited one file and `drop` edited the other, neither said anything,
/// and the plan afterwards held a `dropped` task and a live one sharing an id.
///
/// **Refused rather than disambiguated**, which is the shape the rest of this
/// tool takes: a mutation that cannot know which task it is editing must not
/// guess. It costs an author one `check` run to find out, and `APLAN002` already
/// names both files. A `--file` selector would be machinery for a state that is
/// an error anyway.
///
/// Not something the post-state check can catch: `APLAN002` fires in the
/// *pre*-state too, so it is not a finding the mutation added, and a mutation on
/// an already-dirty plan is allowed by design (`A1-113`).
#[must_use]
pub fn ambiguous_id(plan: &Plan, id: &str) -> Option<String> {
    let files: Vec<String> = plan
        .tasks
        .iter()
        .filter(|lt| lt.task.id == id)
        .map(|lt| lt.path.display().to_string())
        .collect();
    if files.len() < 2 {
        return None;
    }
    Some(format!(
        "`{id}` is declared by {} files ({}), so this cannot know which task it would edit. \
         `APLAN002` reports the duplicate; fix it first, then run this again.",
        files.len(),
        files.join(", ")
    ))
}

/// A plan with the same tasks, carrying no load diagnostics of its own.
///
/// Public so a command applying several edits can build one post-state and judge
/// it once: dropping a cut drops its members too, and judging each file alone
/// would ask the rules about a plan half-cancelled.
///
/// The load half is deliberately dropped: a mutation is judged on the rules, and
/// a file that did not parse is refused by every mutating command before it gets
/// here, so carrying those diagnostics into both sides of the delta would only
/// cancel them out.
#[must_use]
pub fn clone_of(plan: &Plan) -> Plan {
    Plan {
        tasks: plan
            .tasks
            .iter()
            .map(|lt| LoadedTask {
                task: lt.task.clone(),
                path: lt.path.clone(),
                fields: lt.fields.clone(),
                // Carried: a post-state is the plan as it would be, and the
                // bodies are not what the mutation changed (`A1-224`).
                criteria: lt.criteria,
            })
            .collect(),
        load_diagnostics: Vec::new(),
        unloadable: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::code;
    use crate::task::Status;

    /// A finding as a delta carries it, against `task`, for the classification
    /// below.
    fn one_finding(code: &'static str, severity: Severity, on: &str) -> Delta {
        Delta {
            added: vec![Diagnostic {
                severity,
                ..Diagnostic::error(code, "a finding").with_task(on)
            }],
        }
    }

    /// A milestone, for a plan that has one.
    fn milestone(id: &str) -> Task {
        let mut m = task(id, Status::Todo);
        m.tags = vec!["milestone".to_string()];
        m.effort_days = None;
        m
    }

    /// **The deadlock is classified by rule and by policy, not by rule alone.**
    ///
    /// `APLAN021` is a warning, so a first cut can be made empty wherever
    /// warnings do not block, and the two-step is the simpler answer there. Where
    /// they do block, neither half of a new plan can be added first, and telling
    /// the reader to make the cut sends them at a command their own policy
    /// declines (`A1-292`).
    #[test]
    fn the_first_task_is_told_whether_a_cut_can_be_made_first() {
        let lenient = Verdict {
            deny_warnings: false,
            advisory: Vec::new(),
        };
        let strict = Verdict {
            deny_warnings: true,
            advisory: Vec::new(),
        };
        let empty = plan_of(Vec::new());
        let has_a_cut = plan_of(vec![milestone("M-a")]);
        let first_task = one_finding(code::MEMBERSHIP_TOTALITY, Severity::Error, "T-001");
        let first_after = plan_of(vec![task("T-001", Status::Todo)]);
        assert_eq!(
            first_task.bootstrap(&empty, &first_after, &lenient),
            Bootstrap::NoCut
        );
        assert_eq!(
            first_task.bootstrap(&empty, &first_after, &strict),
            Bootstrap::NoCutAndNoneCanBeEmpty
        );
        // A plan that has a cut is not in this situation at all: "add it to one"
        // is complete advice.
        assert_eq!(
            first_task.bootstrap(&has_a_cut, &first_after, &strict),
            Bootstrap::No
        );
    }

    #[test]
    fn an_empty_cut_is_its_own_half_of_the_deadlock() {
        let strict = Verdict {
            deny_warnings: true,
            advisory: Vec::new(),
        };
        let new_cut = one_finding(code::MILESTONE_HYGIENE, Severity::Warning, "M-new");
        let after = plan_of(vec![milestone("M-new")]);
        assert_eq!(
            new_cut.bootstrap(&plan_of(Vec::new()), &after, &strict),
            Bootstrap::EmptyCut
        );
        // And on a plan that already has cuts, because the trap is not confined
        // to an empty plan: an empty milestone is the finding wherever it is
        // added.
        let before = plan_of(vec![milestone("M-a")]);
        let after = plan_of(vec![milestone("M-a"), milestone("M-new")]);
        assert_eq!(
            new_cut.bootstrap(&before, &after, &strict),
            Bootstrap::EmptyCut
        );
    }

    /// **A cut this change emptied is not a cut this change created.**
    ///
    /// `mv` moving the last member out raises the same `APLAN021`, and the
    /// paragraph about creating a cut with its work is advice about an operation
    /// the author is not performing. The rule's own remedy -- move work in, or
    /// cancel the cut -- is complete there (`A1-292`, audit).
    #[test]
    fn emptying_a_cut_that_already_existed_is_not_the_deadlock() {
        let strict = Verdict {
            deny_warnings: true,
            advisory: Vec::new(),
        };
        let emptied = one_finding(code::MILESTONE_HYGIENE, Severity::Warning, "M-a");
        let plan = plan_of(vec![milestone("M-a")]);
        assert_eq!(emptied.bootstrap(&plan, &plan, &strict), Bootstrap::No);
    }

    /// A warning about work fields is not evidence that the cut has no work.
    ///
    /// `APLAN021` covers both shapes. Classifying by code alone sent a newly
    /// added, populated milestone carrying `effort_days` to the empty-cut
    /// remedy, which told the author to add members it already had (`A1-369`).
    #[test]
    fn milestone_work_fields_are_not_misclassified_as_an_empty_cut() {
        let strict = Verdict {
            deny_warnings: true,
            advisory: Vec::new(),
        };
        let work = task("T-001", Status::Todo);
        let mut cut = milestone("M-new");
        cut.blocked_by.push(work.id.clone());
        cut.effort_days = Some(0.0);
        let after = plan_of(vec![work, cut]);
        let work_fields = one_finding(code::MILESTONE_HYGIENE, Severity::Warning, "M-new");

        assert_eq!(
            work_fields.bootstrap(&plan_of(Vec::new()), &after, &strict),
            Bootstrap::No
        );
    }

    /// The add-specific refusal is about input introduced by this add, not any
    /// milestone named by a diagnostic whose wording changed (`A1-406`,
    /// `A1-407`). Keeping this at the shared predicate pins the answer for every
    /// caller, independently of either surface's wiring test.
    #[test]
    fn add_refusal_distinguishes_a_new_bad_milestone_from_an_existing_one() {
        let lenient = Verdict {
            deny_warnings: false,
            advisory: Vec::new(),
        };
        let advisory = Verdict {
            deny_warnings: true,
            advisory: vec![code::MILESTONE_HYGIENE.to_string()],
        };
        let finding = one_finding(code::MILESTONE_HYGIENE, Severity::Warning, "M-bad");

        let mut bad = milestone("M-bad");
        bad.effort_days = Some(0.0);
        let after = plan_of(vec![bad.clone()]);
        assert!(finding.refuses_add(&plan_of(Vec::new()), &after, &lenient));
        assert!(finding.refuses_add(&plan_of(Vec::new()), &after, &advisory));

        // A changed APLAN021 message makes the finding new in the delta, but
        // the malformed milestone and its work field both predate this add.
        let before = plan_of(vec![bad]);
        assert!(!finding.refuses_add(&before, &after, &lenient));

        // A new, clean milestone can still have an APLAN021 for being empty;
        // warning policy, not the work-field exception, owns that case.
        let clean_after = plan_of(vec![milestone("M-bad")]);
        assert!(!finding.refuses_add(&plan_of(Vec::new()), &clean_after, &lenient));
    }

    #[test]
    fn a_rule_this_repository_hears_without_blocking_is_not_the_deadlock() {
        // `advisory` is the knob that takes a rule out of the verdict, so an
        // advisory `APLAN021` means an empty cut is writable and the two-step
        // works again.
        let hears = Verdict {
            deny_warnings: true,
            advisory: vec![code::MILESTONE_HYGIENE.to_string()],
        };
        let empty = plan_of(Vec::new());
        let first_after = plan_of(vec![task("T-001", Status::Todo)]);
        assert_eq!(
            one_finding(code::MEMBERSHIP_TOTALITY, Severity::Error, "T-001").bootstrap(
                &empty,
                &first_after,
                &hears
            ),
            Bootstrap::NoCut
        );
        let cut_after = plan_of(vec![milestone("M-new")]);
        assert_eq!(
            one_finding(code::MILESTONE_HYGIENE, Severity::Warning, "M-new")
                .bootstrap(&empty, &cut_after, &hears),
            Bootstrap::No
        );
    }

    fn task(id: &str, status: Status) -> Task {
        Task {
            id: id.to_string(),
            title: format!("A task called {id}"),
            phase: 1,
            status,
            owner: Some("kaminsod".into()),
            effort_days: Some(1.0),
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: BTreeMap::new(),
            evidence_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    fn plan_of(tasks: Vec<Task>) -> Plan {
        Plan {
            tasks: tasks
                .into_iter()
                .map(|t| LoadedTask {
                    path: std::path::PathBuf::from(format!("aplan/tasks/{}.md", t.id)),
                    fields: crate::load::FieldLines::new(),
                    criteria: crate::load::Criteria::default(),
                    task: t,
                })
                .collect(),
            load_diagnostics: Vec::new(),
            unloadable: BTreeMap::new(),
        }
    }

    /// The whole point: a plan that is already dirty does not make every mutation
    /// refuse. Attributing what was already there to whoever touched the plan
    /// next is right about the rule and wrong about the cause.
    #[test]
    fn diagnostics_the_plan_already_had_are_not_attributed_to_the_mutation() {
        let mut broken = task("A", Status::Todo);
        broken.blocked_by = vec!["GHOST".into()];
        let before = plan_of(vec![broken.clone(), task("B", Status::Todo)]);

        // The pre-state already fails `APLAN003`.
        let existing = validate(&before, &Options::default());
        assert!(
            existing
                .diagnostics
                .iter()
                .any(|d| d.code == code::UNKNOWN_DEP)
        );

        // A mutation that changes something else entirely adds nothing.
        let mut b = task("B", Status::Todo);
        b.title = "A task called B, retitled".into();
        let after = with_replaced(&before, "B", b);
        let d = delta(&before, &after, &Options::default());
        assert!(d.added.is_empty(), "{:?}", d.added);
        assert!(!d.refuses(&Verdict::default()));
    }

    /// The multiset, tested where it can be: two findings identical in every
    /// field. A set difference calls the second one unchanged, which is how a
    /// mutation that doubles a fault reads as harmless.
    ///
    /// No rule in this build produces two such findings, so this cannot be
    /// reached through `validate`; the first version of this test used two
    /// different task ids and would have passed against a set.
    #[test]
    fn a_finding_identical_to_an_existing_one_is_still_an_addition() {
        let one = || Diagnostic::error(code::CYCLE, "dependency cycle: A -> A").with_task("A");
        assert!(
            added(&[one()], vec![one()]).is_empty(),
            "unchanged is not added"
        );
        assert_eq!(
            added(&[one()], vec![one(), one()]).len(),
            1,
            "the second instance is an addition"
        );
        assert_eq!(added(&[], vec![one(), one()]).len(), 2);
        // And the fields that make two findings the same one.
        let elsewhere = Diagnostic::error(code::CYCLE, "dependency cycle: A -> A").with_task("B");
        assert_eq!(
            added(&[one()], vec![elsewhere]).len(),
            1,
            "task is part of it"
        );
        let other_message =
            Diagnostic::error(code::CYCLE, "dependency cycle: A -> B -> A").with_task("A");
        assert_eq!(
            added(&[one()], vec![other_message]).len(),
            1,
            "message is part of it"
        );
        let other_file = Diagnostic::error(code::CYCLE, "dependency cycle: A -> A")
            .with_task("A")
            .with_file(std::path::Path::new("elsewhere.md"));
        assert_eq!(
            added(&[one()], vec![other_file]).len(),
            1,
            "file is part of it"
        );
    }

    /// The same through the plan, which is what the commands use.
    #[test]
    fn a_second_task_with_the_same_fault_is_an_addition() {
        let mut one = task("A", Status::Todo);
        one.blocked_by = vec!["GHOST".into()];
        let before = plan_of(vec![one, task("B", Status::Todo)]);

        let mut also_broken = task("B", Status::Todo);
        also_broken.blocked_by = vec!["GHOST".into()];
        let after = with_replaced(&before, "B", also_broken);

        let d = delta(&before, &after, &Options::default());
        assert_eq!(d.added.len(), 1, "{:?}", d.added);
        assert_eq!(d.added[0].code, code::UNKNOWN_DEP);
        assert_eq!(d.added[0].task.as_deref(), Some("B"));
        assert!(d.refuses(&Verdict::default()));
    }

    /// A new error refuses; a new warning does not, unless the repository said
    /// warnings block.
    #[test]
    fn an_error_refuses_and_a_warning_is_left_to_the_policy() {
        let before = plan_of(vec![task("A", Status::Todo)]);

        // `APLAN009`: an active task with no estimate is a warning.
        let mut no_effort = task("A", Status::Todo);
        no_effort.effort_days = None;
        let warned = delta(
            &before,
            &with_replaced(&before, "A", no_effort),
            &Options::default(),
        );
        assert_eq!(warned.added.len(), 1, "{:?}", warned.added);
        assert!(!warned.refuses(&Verdict::default()));
        assert!(warned.refuses(&Verdict {
            deny_warnings: true,
            advisory: Vec::new()
        }));

        // `APLAN005`: `done` with no evidence is an error.
        let errored = delta(
            &before,
            &with_replaced(&before, "A", task("A", Status::Done)),
            &Options::default(),
        );
        assert!(errored.refuses(&Verdict::default()));
        let line = &errored.lines()[0];
        assert!(line.contains("APLAN005 A:"), "{line}");
        // And the way out, because a refusal naming a rule and not the remedy is
        // a diagnostic with its useful half removed.
        assert!(
            line.contains("fix: Add at least one evidence entry"),
            "{line}"
        );
    }

    /// A rule the repository made advisory does not refuse a mutation either: the
    /// verdict is one policy, and a second one here would be a second opinion
    /// about the same plan (`A1-112`).
    #[test]
    fn an_advisory_rule_does_not_refuse_a_mutation() {
        let before = plan_of(vec![task("A", Status::Todo)]);
        let d = delta(
            &before,
            &with_replaced(&before, "A", task("A", Status::Done)),
            &Options::default(),
        );
        assert!(d.refuses(&Verdict::default()));
        assert!(!d.refuses(&Verdict {
            deny_warnings: false,
            advisory: vec![code::DONE_WITHOUT_EVIDENCE.to_string()],
        }));
    }

    /// The two shapes a line can take that the cases above do not reach: a finding
    /// about the plan rather than a task, and one whose rule carries no remedy.
    #[test]
    fn a_line_names_what_it_has_and_no_more() {
        let plan_wide = Delta {
            added: vec![Diagnostic::warning(
                code::UNUSED_TAG_FAMILY,
                "nothing carries `epic`",
            )],
        };
        assert_eq!(plan_wide.lines(), vec!["APLAN029: nothing carries `epic`"]);

        let with_fix = Delta {
            added: vec![
                Diagnostic::error(code::CYCLE, "dependency cycle: A -> B -> A")
                    .with_task("A")
                    .with_agent_instruction("Break the cycle."),
            ],
        };
        assert_eq!(
            with_fix.lines(),
            vec!["APLAN004 A: dependency cycle: A -> B -> A\n      fix: Break the cycle."]
        );
    }

    /// Replacing a task the plan does not have changes nothing, rather than
    /// inventing one: a mutation naming an unknown id is refused by its command,
    /// and this must not quietly add a second task under that id.
    #[test]
    fn replacing_an_unknown_id_leaves_the_plan_alone() {
        let before = plan_of(vec![task("A", Status::Todo)]);
        let after = with_replaced(&before, "NOPE", task("NOPE", Status::Todo));
        assert_eq!(after.tasks.len(), 1);
        assert_eq!(after.tasks[0].task.id, "A");
        assert!(delta(&before, &after, &Options::default()).added.is_empty());
    }

    /// A task added by the mutation is judged like any other.
    #[test]
    fn an_added_task_is_part_of_the_post_state() {
        let before = plan_of(vec![task("A", Status::Todo)]);
        let mut new = task("B", Status::Todo);
        new.blocked_by = vec!["GHOST".into()];
        let after = with_added(&before, "aplan/tasks/B.md".into(), new);

        let d = delta(&before, &after, &Options::default());
        assert_eq!(d.added.len(), 1, "{:?}", d.added);
        assert_eq!(d.added[0].code, code::UNKNOWN_DEP);
    }
}
