//! The task model. These Rust types are the single source of truth: the JSON
//! Schema in `schemas/v1/task.json` is *derived* from them (`cargo xtask
//! gen-schema`), and CI fails if the committed schema and the types disagree.
//! Prose never becomes a regenerable source of truth.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// Generated via `str_enum!`, as `EvidenceKind` below already was, so
// `Status::ALL` cannot omit a variant and `as_str` agrees with the `snake_case`
// serde name by construction. It was a hand-written five-arm `Display` and a
// hand-written list wherever anything needed the set; `A1-214` needed the set
// for `--status` and adding a sixth by hand would have been the drift this
// repository refuses for rules and formats.
crate::str_enum! {
    /// Lifecycle of a task. `Done` is the only status that requires evidence.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Status {
        /// Not started. Eligible to be worked once its blockers clear.
        Todo = "todo",
        /// Actively being worked.
        InProgress = "in_progress",
        /// Explicitly parked for a reason outside the dependency graph.
        Blocked = "blocked",
        /// Complete. Requires at least one `evidence` entry.
        Done = "done",
        /// Abandoned. Kept for the record rather than deleted.
        Dropped = "dropped",
    }
}

impl std::fmt::Display for Status {
    /// Renders as the serialised (`snake_case`) form. Diagnostics must show the
    /// value the user typed, not the Rust variant name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Status {
    /// A task that will never be worked again.
    pub fn is_terminal(self) -> bool {
        matches!(self, Status::Done | Status::Dropped)
    }

    /// Only `Done` satisfies a dependency. `Dropped` does not, a dropped
    /// blocker means its dependents can never legitimately start.
    pub fn satisfies_dependency(self) -> bool {
        self == Status::Done
    }

    /// Whether a task in this status is available to be picked up.
    ///
    /// `Blocked` is not: it means *explicitly parked for a reason outside the
    /// dependency graph*, so it is workable by no one until a human unparks it.
    /// Answering "what can I work on now" with a task whose status is literally
    /// `blocked` is the tool contradicting itself.
    pub fn is_workable(self) -> bool {
        matches!(self, Status::Todo | Status::InProgress)
    }
}

// Generated via `str_enum!` so `EvidenceKind::ALL` (which `facts.json` reads)
// can never omit a variant, and `as_str` agrees with the `snake_case` serde name
// by construction. The doc comment below is the schema description, kept free of
// codegen detail.
crate::str_enum! {
    /// What kind of proof closes a task.
    ///
    /// The point of typing this is that "I finished it" is not evidence. A commit,
    /// a URL, a committed artefact, a named test, or a dated note is.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum EvidenceKind {
        /// A git commit SHA in this repository.
        Commit = "commit",
        /// An external URL (a filing receipt, a regulator page, a vendor quote).
        Url = "url",
        /// A repo-relative path to a committed artefact (a saved PDF, a transcript), optionally at a rev (`path@rev`) and optionally naming the section that proves it (`path#anchor`).
        File = "file",
        /// A named test that must pass.
        Test = "test",
        /// A dated human note, the weakest kind. Use when nothing better exists,
        /// e.g. the outcome of a phone call.
        Note = "note",
    }
}

/// Split a `file` reference into `(path, rev)`. `path@rev` yields a rev; a bare
/// path (or one whose `@` leaves either side empty) resolves at `HEAD`.
///
/// A rev is required to contain no `/`, so a path with an `@` segment (a scoped
/// npm package, `pkg/@scope/x.js`) is not misread as `path=pkg/`,
/// `rev=scope/x.js`.
///
/// One definition, in the crate that owns the reference format: `attest` had a
/// second copy that lacked these guards and truncated a path containing an `@`
/// (`A1-159`'s audit of its own change). A suffix is a rev only when both halves
/// are non-empty and it holds no `/`, so `node_modules/@scope/doc.md` stays one
/// path.
#[must_use]
pub fn split_rev(reference: &str) -> (&str, Option<&str>) {
    match reference.rsplit_once('@') {
        Some((p, r)) if !p.is_empty() && !r.is_empty() && !r.contains('/') => (p, Some(r)),
        _ => (reference, None),
    }
}

/// The `#section` a `file` citation may name, split off from the path.
///
/// An anchor is a place *in* a file rather than a second file (ADR-0015), so
/// every caller that wants the path -- resolution, relevance, the blob a
/// document is attested by -- strips it the same way.
#[must_use]
pub fn split_anchor(reference: &str) -> (&str, Option<&str>) {
    match reference.split_once('#') {
        Some((p, a)) => (p, Some(a)),
        None => (reference, None),
    }
}

/// A single piece of proof attached to a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// How to interpret `reference`.
    pub kind: EvidenceKind,
    /// The pointer: a commit SHA, a URL, a repo-relative path, a test name, or
    /// the note text itself.
    #[serde(rename = "ref")]
    pub reference: String,
    /// ISO-8601 date the evidence was captured. Required for `note`, because an
    /// undated note is a rumour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured: Option<String>,
}

/// One unit of work. Serialised as YAML frontmatter in a markdown file under
/// the configured tasks glob; the markdown body is free prose and is not parsed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Task {
    /// Stable identifier, unique across the plan. Convention: `P<phase>-<nnn>`.
    pub id: String,
    /// One line. What done looks like, not what the work is.
    ///
    /// Non-empty, because a task nobody can name is a task nobody can find: it
    /// renders as a blank row in `ready`, `roadmap`, and the generated companion,
    /// and there is no reading of an empty title that was intended.
    #[schemars(length(min = 1))]
    pub title: String,
    /// Phase number. Used for rollups and ordering, not for gating.
    pub phase: u32,
    /// Lifecycle status.
    pub status: Status,
    /// Who owns it. Absent means unassigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Estimated effort in days. Typed so phases can be rolled up. Non-negative:
    /// a negative estimate is meaningless and would corrupt a rollup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0))]
    pub effort_days: Option<f64>,
    /// Task IDs that must be `done` before this one can start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    /// Task IDs this one prefers to follow: **discretionary** order, which
    /// `ready` honours and nothing else reads (`A1-150`, ADR-0025).
    ///
    /// Not a dependency, and the distinction is the whole point. A preference
    /// never blocks: a task whose preference is unmet is still ready and still
    /// offered, only offered later. `critical-path`, `why` and `APLAN007` do not
    /// read it, because a reader asking "why can I not start this" must never be
    /// shown an opinion. Expressing preference as `blocked_by` instead corrupts
    /// all three, which ADR-0017 measured before deferring this field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefer_after: Vec<String>,
    /// Proof of completion. Must be non-empty when `status: done`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
    /// Repo-relative globs naming the files this task is responsible for.
    ///
    /// This is what lets `aplan verify` ask whether a cited `commit` actually
    /// touches anything the task claims (`APLAN012`, L2 "Relevant"), without it,
    /// a commit SHA is a citation no one can bind to the task (HLD §5.2). Empty
    /// means relevance is unverifiable, not that everything is fair game.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Anchors this task resolves, as `path/to/doc.md#anchor`. Closing the task
    /// obliges you to update that anchor, this is the link that stops a
    /// research document and a plan from silently diverging.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traces: Vec<String>,
    /// Content fingerprints of `traces` anchors, captured when the task was closed
    /// (`aplan close`, with `detect_drift` on). Keyed by the full trace string.
    /// `APLAN018` fires when a `done` task's traced section no longer matches its
    /// stamp. Absent means unverifiable, not a violation (zero false positives).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub trace_fingerprints: std::collections::BTreeMap<String, String>,
    /// Free-form labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// When a parked task is worth re-evaluating, as an RFC 3339 instant
    /// (`A1-264`).
    ///
    /// **It says when to look again, never that the wait is over.** `p00013`'s
    /// `DN0-29` waits on GitHub creating a real weekday `schedule` event: the
    /// earliest useful revisit is computable from a committed cron and a
    /// measured delay, and reaching it proves nothing about whether the event
    /// exists. So this changes no status, satisfies no dependency, and is read
    /// by `aplan due` and nothing else -- `check`, `ready` and `verify` never
    /// consult a clock.
    ///
    /// This does not reverse `A1-175`, which declined a structured *reason*:
    /// all 22 measured parks waited on an unfalsifiable human judgement, and a
    /// field holding one would be a claim nothing could check. A revisit
    /// instant is the narrower claim *do not look before this*, which is
    /// checkable, and the reason still lives in the body.
    ///
    /// Meaningful only while the task is parked, which `APLAN042` reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revisit_at: Option<String>,
    /// What this task did to the released artifact (`A1-312`, ADR-0034).
    ///
    /// **The change, not the arithmetic.** `breaking` stays true at every
    /// version; `major` does not, because cargo formalises `0.x.y` as
    /// `0.INCOMPATIBLE.COMPATIBLE`, so below 1.0 a breaking change bumps the
    /// *minor*. An author writing the word for the bump would have to know that
    /// rule and get it right; an author writing the word for the change does
    /// not, and the scheme does the arithmetic.
    ///
    /// Usually absent. A task's level already supplies a floor -- a unit of work
    /// `fix`, a cut `feature` -- so this is for saying what the floor cannot: a
    /// `breaking` change inside one small task, or a `none` that is genuinely
    /// none. Declaring *below* the floor needs [`Task::version_impact_reason`],
    /// which `APLAN044` reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_impact: Option<VersionImpact>,
    /// Why this task claims less than its level implies (`A1-312`).
    ///
    /// A field rather than a paragraph, because a rule cannot read the body: the
    /// model carries what the loader derived and the prose is not parsed. The
    /// same reason `APLAN021` counts members rather than reading the narrative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_impact_reason: Option<String>,
}

/// What a change did to the thing that ships (`A1-312`, ADR-0034).
///
/// Ordered, and the order is the whole type: aggregation takes the strongest of
/// what a level floors and what its members claim, so `None < Fix < Feature <
/// Breaking` is what makes a `Breaking` task raise the cut above it.
///
/// It describes the **change**, and each scheme maps it to its own arithmetic:
/// semver at or past 1.0 sends `Breaking` to major, semver below 1.0 sends it to
/// minor, and `CalVer` takes its number from the calendar and keeps only the
/// compatibility claim.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum VersionImpact {
    /// Changes nothing that ships. A pilot log, a plan edit, a note.
    None,
    /// A compatible correction.
    Fix,
    /// A compatible addition.
    Feature,
    /// Something a consumer has to react to.
    Breaking,
}

/// The floor each level of a plan carries, already resolved (`A1-312`,
/// ADR-0034).
///
/// Resolved rather than configured, because this crate must not know what
/// `.aplan.yml` looks like: `aplan_config` turns the adopter's map into this,
/// defaults and all, and the rule below reads only the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionFloors {
    /// A unit of work.
    pub task: VersionImpact,
    /// A milestone holding work (ADR-0022).
    pub cut: VersionImpact,
    /// A milestone closing cuts (ADR-0022).
    pub release: VersionImpact,
}

impl Default for VersionFloors {
    /// The shipped floors: a unit of work is a `fix`, a cut is a `feature`, and
    /// a release claims nothing of its own because its cuts already do.
    ///
    /// Here rather than in `aplan_config` so that the rule, the scaffold and
    /// `vacuity`'s "what would this switch reveal" probe all read one answer.
    fn default() -> Self {
        Self {
            task: VersionImpact::Fix,
            cut: VersionImpact::Feature,
            release: VersionImpact::None,
        }
    }
}

impl VersionFloors {
    /// The floor this task's level carries.
    ///
    /// Read from the `milestone` and `release` tags, which is the only place a
    /// level is written down. **That is a known weakness and not this task's to
    /// fix**: `A1-212` reports those five tags as an undeclared vocabulary
    /// `APLAN026` never checks, so a task tagged `releaes` is a work task
    /// silently -- and here, that silently lowers the floor it is held to.
    #[must_use]
    pub fn of(&self, task: &Task) -> VersionImpact {
        if !task.is_milestone() {
            return self.task;
        }
        if task.is_release() {
            self.release
        } else {
            self.cut
        }
    }
}

impl std::fmt::Display for VersionImpact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::None => "none",
            Self::Fix => "fix",
            Self::Feature => "feature",
            Self::Breaking => "breaking",
        })
    }
}

/// The tag that marks a task as a cut checkpoint rather than work.
pub const MILESTONE_TAG: &str = "milestone";

/// The tag that marks a milestone as a *delivery over cuts* rather than a cut
/// (`A1-146`, ADR-0022).
///
/// A release closes cuts; a cut holds work. Both are milestones, and both use
/// `blocked_by`, so the edge alone cannot say which is meant: ADR-0013 already
/// reads a milestone naming a milestone as "follows that cut". This tag is the
/// author saying which they meant, and it is read by exactly the rules that
/// would otherwise be wrong about it.
pub const RELEASE_TAG: &str = "release";

impl Task {
    /// A task is *ready* when its status is workable and every blocker is `done`.
    ///
    /// Note the status test is [`Status::is_workable`], not `!is_terminal`: a
    /// `blocked` task has no unmet *graph* blockers, so a `!is_terminal` test
    /// reported it as ready, advertising work the plan explicitly parked.
    pub fn is_ready(&self, is_done: impl Fn(&str) -> bool) -> bool {
        self.status.is_workable() && self.blocked_by.iter().all(|b| is_done(b))
    }

    /// Whether anything this task cites is judged against its `paths`.
    ///
    /// `APLAN012` binds a `commit` by asking whether its diff touched them,
    /// `APLAN024` measures how much of that commit falls outside them, `APLAN015`
    /// and `APLAN023` bind a `file` to them, and `APLAN025` asks whether a cited
    /// `test` covers them. A `note` or a `url` is judged against nothing, and a
    /// task citing neither has nothing bound to its paths at all.
    ///
    /// This is `APLAN019`'s scope (`A1-138`). That rule reports a glob matching
    /// no tracked file, and explains itself with "so every rule that judges
    /// evidence against it has nothing to bind" -- which, for a task with no such
    /// citation, is a harm that is not yet possible. The fourth pilot met it at
    /// scale: 140 findings across 71 `todo` tasks whose globs name trees a later
    /// cut creates, under `deny_warnings`, and it declined the upgrade rather
    /// than take them (pre-public adoption evidence, item 24 and its correction).
    ///
    /// A vacuous `ref` does not count, by the same reasoning `APLAN005` uses: a
    /// citation of a single zero-width space is not a citation.
    #[must_use]
    pub fn binds_paths(&self) -> bool {
        self.evidence
            .iter()
            .filter(|e| !crate::validate::is_vacuous_ref(&e.reference))
            .any(|e| {
                matches!(
                    e.kind,
                    EvidenceKind::Commit | EvidenceKind::File | EvidenceKind::Test
                )
            })
    }

    /// Whether this task is a *milestone*: a checkpoint over other tasks rather
    /// than a unit of work. A milestone is `blocked_by` the tasks in its cut, so
    /// it cannot be `done` while any listed member is undone (`APLAN007`), and its
    /// body carries the cut's narrative. The `milestone` tag is the marker; the
    /// work-views (`ready`, `critical_path`, `summary`, and the `APLAN009` effort
    /// warning) exclude it, because a checkpoint is not itself workable, weighs
    /// nothing, and is not on the plan's spine. `APLAN021` keeps the tag honest (a
    /// milestone must have members and no effort or paths). No new field or edge
    /// type: a milestone is an ordinary task.
    pub fn is_milestone(&self) -> bool {
        self.tags.iter().any(|t| t == MILESTONE_TAG)
    }

    /// Whether this milestone is a *release*: a checkpoint over cuts rather than
    /// over work (`A1-146`, ADR-0022).
    ///
    /// A release is a milestone first -- it is a checkpoint, it carries no
    /// effort and no paths, and every work-view excludes it -- so this is a
    /// narrowing rather than a second kind of node. The tag alone is not enough:
    /// a task that is not a milestone is not a release however it is tagged,
    /// because a release that could hold work would be the second grouping axis
    /// ADR-0014 refused.
    pub fn is_release(&self) -> bool {
        self.is_milestone() && self.tags.iter().any(|t| t == RELEASE_TAG)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A blank work task, for the level cases below.
    fn work_task() -> Task {
        Task {
            id: "A-1".into(),
            title: "work".into(),
            phase: 0,
            status: Status::Done,
            owner: None,
            effort_days: None,
            blocked_by: Vec::new(),
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

    /// The order is the type: aggregation takes the strongest, so a `Breaking`
    /// member has to outrank every floor above it (`A1-312`, ADR-0034).
    #[test]
    fn the_ladder_runs_from_none_to_breaking() {
        use VersionImpact::{Breaking, Feature, Fix, None as NoImpact};
        assert!(NoImpact < Fix && Fix < Feature && Feature < Breaking);
        assert_eq!(
            [Feature, NoImpact, Breaking, Fix].iter().max(),
            Some(&Breaking),
            "the strongest claim is what a level ends up owing"
        );
        // And the words are the change rather than the arithmetic, which is what
        // lets one ladder serve semver, 0.x semver and CalVer.
        assert_eq!(Breaking.to_string(), "breaking");
        assert_eq!(NoImpact.to_string(), "none");
    }

    /// A task's floor comes from its level, and the level comes from its tags.
    #[test]
    fn each_level_carries_its_own_floor() {
        use VersionImpact::{Feature, Fix, None as NoImpact};
        let floors = VersionFloors {
            task: Fix,
            cut: Feature,
            release: NoImpact,
        };

        let work = work_task();
        assert_eq!(floors.of(&work), Fix);

        let mut cut = work_task();
        cut.tags = vec![MILESTONE_TAG.to_string()];
        assert_eq!(floors.of(&cut), Feature);

        let mut release = work_task();
        release.tags = vec![MILESTONE_TAG.to_string(), RELEASE_TAG.to_string()];
        assert_eq!(floors.of(&release), NoImpact);

        // A release is a milestone first: the tag alone does not make one, so a
        // work task carrying it is still held to the work floor. That narrowing
        // is load-bearing here, because the release floor is the *lowest* of the
        // three -- reading it wrongly would quietly let a task off.
        let mut mislabelled = work_task();
        mislabelled.tags = vec![RELEASE_TAG.to_string()];
        assert_eq!(floors.of(&mislabelled), Fix);
    }

    /// A release is a milestone first (`A1-146`, ADR-0022).
    ///
    /// The call sites all ask inside `is_milestone()`, so the narrowing is
    /// belt-and-braces there -- and the predicate is what a future call site
    /// will trust: a work task tagged `release` would otherwise be a checkpoint
    /// carrying effort and paths, which is the second grouping axis ADR-0014
    /// refused.
    #[test]
    fn a_task_that_is_not_a_milestone_is_not_a_release() {
        let mut t = Task {
            id: "A-1".into(),
            title: "work".into(),
            phase: 0,
            status: Status::Todo,
            owner: None,
            effort_days: Some(1.0),
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: vec![RELEASE_TAG.to_string()],
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
        };
        assert!(
            !t.is_release(),
            "a work task carrying the tag is still work"
        );
        t.tags.push(MILESTONE_TAG.to_string());
        assert!(t.is_release(), "a milestone carrying it is a release");
        t.tags.retain(|tag| tag != RELEASE_TAG);
        assert!(!t.is_release(), "and a cut is not one");
        assert!(t.is_milestone());
    }

    /// Regression: `ready` and `why` both announced an explicitly parked task
    /// (`status: blocked`) as ready, because the test was `!is_terminal` and a
    /// parked task has no unmet graph blockers.
    #[test]
    fn parked_task_is_never_ready() {
        let parked = Task {
            id: "X".into(),
            title: "parked".into(),
            phase: 0,
            status: Status::Blocked,
            owner: None,
            effort_days: None,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
        };
        assert!(
            !parked.is_ready(|_| true),
            "a `blocked` task must never be reported as ready"
        );
        for s in [Status::Todo, Status::InProgress] {
            let mut t = parked.clone();
            t.status = s;
            assert!(t.is_ready(|_| true), "{s} with no blockers is ready");
        }
        for s in [Status::Done, Status::Dropped] {
            let mut t = parked.clone();
            t.status = s;
            assert!(!t.is_ready(|_| true), "{s} is terminal, not ready");
        }
    }

    /// The predicate `APLAN019` is scoped by, tested where it lives rather than
    /// only through the rule (`A1-138`).
    #[test]
    fn binds_paths_is_about_the_kinds_judged_against_them() {
        let ev = |kind: EvidenceKind, r: &str| Evidence {
            kind,
            reference: r.to_string(),
            captured: None,
        };
        let with = |evidence: Vec<Evidence>| {
            let mut t = Task {
                id: "A".into(),
                title: "t".into(),
                phase: 0,
                status: Status::Done,
                owner: None,
                effort_days: None,
                blocked_by: Vec::new(),
                prefer_after: Vec::new(),
                evidence: Vec::new(),
                paths: vec!["src/**".into()],
                traces: Vec::new(),
                trace_fingerprints: std::collections::BTreeMap::new(),
                tags: Vec::new(),
                revisit_at: None,
                version_impact: None,
                version_impact_reason: None,
            };
            t.evidence = evidence;
            t.binds_paths()
        };

        assert!(with(vec![ev(EvidenceKind::Commit, "abc")]));
        assert!(with(vec![ev(EvidenceKind::File, "src/a.rs")]));
        assert!(with(vec![ev(EvidenceKind::Test, "m::t")]));

        // Judged against nothing: `APLAN012`, `APLAN015`, `APLAN024` and
        // `APLAN025` all ask about paths, and none of them looks at these.
        assert!(!with(vec![ev(EvidenceKind::Note, "reviewed")]));
        assert!(!with(vec![ev(EvidenceKind::Url, "https://x.invalid/")]));
        assert!(!with(Vec::new()));

        // A citation of nothing is not a citation, by `APLAN005`'s own predicate:
        // a single zero-width space would otherwise bind the paths.
        assert!(!with(vec![ev(EvidenceKind::Commit, "\u{200b}")]));
        assert!(!with(vec![ev(EvidenceKind::Commit, "   ")]));
        // And one real citation among vacuous ones still binds.
        assert!(with(vec![
            ev(EvidenceKind::Commit, " "),
            ev(EvidenceKind::File, "src/a.rs"),
        ]));
    }

    #[test]
    fn is_milestone_reads_the_milestone_tag() {
        let mut t = Task {
            id: "M".into(),
            title: "cut".into(),
            phase: 0,
            status: Status::Todo,
            owner: None,
            effort_days: None,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
        };
        assert!(!t.is_milestone(), "no tags: not a milestone");
        t.tags = vec!["cli".into(), "milestone".into()];
        assert!(t.is_milestone(), "the `milestone` tag marks a milestone");
        t.tags = vec!["milestones".into()];
        assert!(!t.is_milestone(), "only the exact `milestone` tag counts");
    }
    /// Both splitters, at the edges that produced a divergence: `attest` kept a
    /// copy of the rev rule without the guards, and truncated a path holding an
    /// `@` (`A1-159`).
    #[test]
    fn a_reference_splits_the_same_way_everywhere() {
        assert_eq!(split_rev("docs/x.md"), ("docs/x.md", None));
        assert_eq!(split_rev("docs/x.md@abc123"), ("docs/x.md", Some("abc123")));
        // A scoped package path is one path, not a rev.
        assert_eq!(
            split_rev("node_modules/@scope/doc.md"),
            ("node_modules/@scope/doc.md", None)
        );
        assert_eq!(
            split_rev("@abc"),
            ("@abc", None),
            "an empty path is not a split"
        );
        assert_eq!(split_rev("docs/x.md@"), ("docs/x.md@", None));

        assert_eq!(split_anchor("docs/x.md"), ("docs/x.md", None));
        assert_eq!(
            split_anchor("docs/x.md#result"),
            ("docs/x.md", Some("result"))
        );
        assert_eq!(split_anchor("docs/x.md#"), ("docs/x.md", Some("")));
        // The first `#`, as `traces` splits it, so the two agree on a document
        // whose anchor contains one.
        assert_eq!(split_anchor("docs/x.md#a#b"), ("docs/x.md", Some("a#b")));

        // Written together, the rev comes off first.
        let (reference, rev) = split_rev("docs/x.md#result@abc123");
        assert_eq!(rev, Some("abc123"));
        assert_eq!(split_anchor(reference), ("docs/x.md", Some("result")));
    }
}
