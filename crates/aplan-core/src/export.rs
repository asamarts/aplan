//! The plan as data, for a consumer that wants a rollup this tool does not have.
//!
//! `summary` originally rolled up by phase and nothing else. The fourth pilot
//! types every task build/measure/review, because 20 of its 105.5 days do not
//! shrink by working faster, and no command could answer that. A configured
//! family now covers one deliberate alternate view, while this surface keeps
//! every task available for any other rollup. Before either existed, the effort
//! table in that project's own plan stayed hand-maintained, a second copy of
//! exactly the fact `aplan` owns, and it was wrong twice during the pilot's own
//! audit (pre-public adoption evidence, item 5).
//!
//! The narrow fix was a `--group-by` flag, which answers one question and
//! invites the next. This emits the tasks instead, after which every rollup
//! anyone wants is a `jq` line and the class of request stops arriving
//! (`A1-075`).

use std::borrow::Cow;
use std::marker::PhantomData;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};

use crate::graph::Graph;
use crate::roadmap::Roadmap;
use crate::task::EvidenceKind;
use crate::task::Status;

/// A schema-only stand-in for an `Option<T>` that is nullable but not optional.
///
/// Schemars normally maps `Option<T>` to both a nullable value and an optional
/// property. Task-data rows serialize every key and use `null` only as the
/// value, so their schema needs to separate those two meanings (`A1-493`).
struct RequiredNullable<T>(PhantomData<T>);

impl<T: JsonSchema> JsonSchema for RequiredNullable<T> {
    fn inline_schema() -> bool {
        <Option<T>>::inline_schema()
    }

    fn schema_name() -> Cow<'static, str> {
        <Option<T>>::schema_name()
    }

    fn schema_id() -> Cow<'static, str> {
        <Option<T>>::schema_id()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        <Option<T>>::json_schema(generator)
    }
}

/// The document `aplan tasks --format json` emits.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TaskExport {
    /// The document's shape, so a consumer can pin it.
    pub schema: String,
    /// Local coordination boundary for every row's ephemeral claim.
    pub coordination: crate::ClaimScope,
    /// Clock at which claim ages can be derived.
    pub claim_observed_at_epoch_seconds: i64,
    pub tasks: Vec<TaskRow>,
}

impl TaskExport {
    /// Attach one coherent ephemeral-claim observation to every task row.
    #[must_use]
    pub fn with_claims(mut self, snapshot: &crate::ClaimSnapshot) -> Self {
        self.schema = "aplan/tasks/v2".to_string();
        self.coordination = snapshot.scope.clone();
        self.claim_observed_at_epoch_seconds = snapshot.observed_at_epoch_seconds;
        for row in &mut self.tasks {
            row.claim = snapshot.claim(&row.id).cloned();
        }
        self
    }
}

/// One task, with the facts a rollup needs already resolved.
///
/// `cut` and `ready` are computed rather than read off the file: a consumer that
/// had to derive them would be re-implementing the graph, which is the thing a
/// tool is for.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TaskRow {
    pub id: String,
    pub title: String,
    pub phase: u32,
    pub status: Status,
    /// `null` when the task carries no estimate, which `APLAN009` reports
    /// separately. Distinguished from `0` on purpose: an unestimated task is not
    /// a free one.
    #[schemars(with = "RequiredNullable<f64>")]
    pub effort_days: Option<f64>,
    #[schemars(with = "RequiredNullable<String>")]
    pub owner: Option<String>,
    /// Who is working on it now, distinct from durable `owner` responsibility.
    #[schemars(with = "RequiredNullable<crate::ClaimRecord>")]
    pub claim: Option<crate::ClaimRecord>,
    pub tags: Vec<String>,
    /// Declared blockers only. The cut-sequencing edges a task inherits
    /// (ADR-0013) are implied by `cut`, and repeating them here would make a
    /// rollup double-count what the plan states once.
    pub blocked_by: Vec<String>,
    /// The label of the cut this task belongs to, e.g. `v0.8` or `s3`.
    ///
    /// A task belongs to exactly one cut where the repository sets
    /// `require_cut_membership`, which `APLAN022` then enforces in both
    /// directions. Where it does not, a task may be listed by several
    /// milestones, and this reports the first in roadmap order: deterministic,
    /// but a rollup by cut on such a plan will under-count, and the plan has not
    /// asked the tool to stop it.
    #[schemars(with = "RequiredNullable<String>")]
    pub cut: Option<String>,
    /// The id of the milestone that owns it, since two cuts may share a label.
    #[schemars(with = "RequiredNullable<String>")]
    pub milestone: Option<String>,
    /// Whether every blocker is `done`, including the inherited cut edges. The
    /// same answer `aplan ready` gives, so a consumer cannot derive a different
    /// one.
    pub ready: bool,
    /// Milestones are checkpoints, not work. They are carried so a consumer can
    /// see the cut structure, and flagged so a rollup can exclude them the way
    /// every rollup here does.
    pub milestone_task: bool,
    /// Where `aplan ready` puts this task, `1` first, and `null` for anything
    /// that is not ready (`A1-125`).
    ///
    /// Carried because `A1-075` made this the machine surface a rollup is built
    /// on, and a rollup answering "the next five things" would otherwise have to
    /// reimplement the derivation and could get a different answer than the
    /// command. A rank on a task that is not ready would be an order over work
    /// nobody can start, so those are `null` rather than ranked at the end.
    #[schemars(with = "RequiredNullable<usize>")]
    pub rank: Option<usize>,

    // Everything a task *declares* that the derived half above does not cover.
    // `A1-075` made this the data surface and said any rollup the tool does not
    // have is a `jq` line downstream. That was true of thirteen fields, and the
    // claim was quoted for more: a consumer could ask which tasks are ready in a
    // cut and could not ask **which `done` tasks cite only `note` evidence** --
    // the question closest to this tool's thesis, and the first one an auditor
    // asks (`A1-220`).
    //
    // Everything declared, rather than the four that question needed: a surface
    // that emits what a task *is* can be explained in a sentence, and one that
    // emits a chosen subset has to be explained a field at a time and argued
    // about whenever somebody wants the next one.
    /// The proof cited for `status: done`, verbatim.
    ///
    /// The whole citation, not a count or a set of kinds: "cites only `note`"
    /// and "cites a commit that is not in this repository" are different
    /// questions, and a summarised field answers neither.
    pub evidence: Vec<EvidenceRow>,
    /// The globs this task declares it touches, which `verify` resolves a
    /// commit against and `APLAN019` reports when nothing matches.
    pub paths: Vec<String>,
    /// The document anchors this task traces to.
    pub traces: Vec<String>,
    /// The discretionary order somebody recorded, which is not a dependency
    /// (ADR-0025): `ready` uses it to break ties and the graph does not draw it.
    pub prefer_after: Vec<String>,
    /// Who must bless this task before it counts as finished, which is not a
    /// dependency either (ADR-0036): `ready` does not read it, and `APLAN046`
    /// refuses `done` while a ratifier is unfinished.
    pub ratified_by: Vec<String>,
    /// The checkpoints this one contains, whose work rolls up into it at any
    /// depth (ADR-0042). Not a dependency either: a milestone in `blocked_by`
    /// *sequences* two cuts (ADR-0013), and the two meanings are what this
    /// separates.
    pub contains: Vec<String>,
    /// The anchor fingerprints a closed task stamped, which is what `APLAN018`
    /// compares against to find a traced section that moved.
    pub trace_fingerprints: std::collections::BTreeMap<String, String>,
    /// The mutable evidence referents a closed task stamped, which `APLAN054`
    /// compares against their committed contents.
    pub evidence_fingerprints: std::collections::BTreeMap<String, String>,
    /// When a parked task is worth looking at again, as an RFC 3339 instant
    /// (`A1-264`).
    ///
    /// **Always present, `null` when there is none**, because this is a contract
    /// a consumer reads: a key that comes and goes hands it `undefined` where it
    /// expected a value, which is the rule the whole of this type follows. An
    /// external scheduler filters on it; `aplan due` is the same question asked
    /// with validated timestamp semantics.
    #[schemars(with = "RequiredNullable<String>")]
    pub revisit_at: Option<String>,
    /// What the task claims it does to the released version, `null` when it
    /// claims nothing (`A1-312`, ADR-0034).
    ///
    /// Null is the ordinary case and means "the level's floor applies", not
    /// "nothing": most tasks declare no impact and the plan still knows what
    /// they owe. A consumer asking what a cut owes reads this together with the
    /// repository's floors, which is why the claim is carried verbatim rather
    /// than resolved here -- the floors live in `.aplan.yml`, which this type
    /// has never read.
    #[schemars(with = "RequiredNullable<crate::task::VersionImpact>")]
    pub version_impact: Option<crate::task::VersionImpact>,
    /// Why that claim is lower than the level implies, `null` when there is
    /// none. This is the half a reviewer reads, and `APLAN044` is the rule that
    /// asks for it.
    #[schemars(with = "RequiredNullable<String>")]
    pub version_impact_reason: Option<String>,
    /// The version a **release** produced, written when it closed, `null`
    /// everywhere else (`A1-339`, ADR-0038).
    ///
    /// Carried because it is a *record* rather than a derivation: a consumer
    /// asking what a release contained reads this instead of re-deriving from a
    /// plan that has since moved, which is the whole argument for the field
    /// existing. `APLAN050` is the rule that keeps it on a release and readable.
    #[schemars(with = "RequiredNullable<String>")]
    pub version: Option<String>,
}

/// One citation, as the data surface carries it.
///
/// **Its own type, and not `task::Evidence`.** That type is the *file format*,
/// where `captured` is omitted when absent because a task file should not carry
/// `captured: null` on a commit. A contract a consumer reads is the opposite
/// case: a key that comes and goes hands it `undefined` where it expected a
/// value, and it would change whenever somebody adjusted the YAML for reasons
/// that have nothing to do with the surface. Every field here is always present
/// (`A1-220`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceRow {
    /// How to read `reference`.
    pub kind: EvidenceKind,
    /// The pointer: a commit SHA, a URL, a repo-relative path, a test name, or
    /// the note text itself.
    #[serde(rename = "ref")]
    pub reference: String,
    /// ISO-8601 date the inline assertion was made. Required for `note`, because
    /// an undated note is a rumour, and rejected on every machine-resolved kind:
    /// an author-entered date is not freshness proof (`APLAN055`).
    #[schemars(with = "RequiredNullable<String>")]
    pub captured: Option<String>,
}

impl From<&crate::task::Evidence> for EvidenceRow {
    fn from(e: &crate::task::Evidence) -> Self {
        Self {
            kind: e.kind,
            reference: e.reference.clone(),
            captured: e.captured.clone(),
        }
    }
}

impl Graph<'_> {
    /// Every task in the plan, in id order.
    #[must_use]
    pub fn task_rows(&self) -> TaskExport {
        let roadmap = self.roadmap();
        let rank: std::collections::BTreeMap<&str, usize> = self
            .ready()
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id.as_str(), i + 1))
            .collect();

        let tasks = self
            .by_id
            .values()
            .map(|t| {
                let (cut, milestone) = cut_of(&roadmap, &t.id);
                TaskRow {
                    id: t.id.clone(),
                    title: t.title.clone(),
                    phase: t.phase,
                    status: t.status,
                    effort_days: t.effort_days,
                    owner: t.owner.clone(),
                    claim: None,
                    tags: t.tags.clone(),
                    blocked_by: t.blocked_by.clone(),
                    cut,
                    milestone,
                    ready: rank.contains_key(t.id.as_str()),
                    milestone_task: t.is_milestone(),
                    rank: rank.get(t.id.as_str()).copied(),
                    evidence: t.evidence.iter().map(EvidenceRow::from).collect(),
                    paths: t.paths.clone(),
                    traces: t.traces.clone(),
                    prefer_after: t.prefer_after.clone(),
                    ratified_by: t.ratified_by.clone(),
                    contains: t.contains.clone(),
                    trace_fingerprints: t.trace_fingerprints.clone(),
                    evidence_fingerprints: t.evidence_fingerprints.clone(),
                    revisit_at: t.revisit_at.clone(),
                    version_impact: t.version_impact,
                    version_impact_reason: t.version_impact_reason.clone(),
                    version: t.version.clone(),
                }
            })
            .collect();

        TaskExport {
            schema: "aplan/tasks/v2".to_string(),
            coordination: crate::ClaimScope::unavailable("no local claim snapshot was supplied"),
            claim_observed_at_epoch_seconds: 0,
            tasks,
        }
    }
}

/// The cut a task belongs to, as `(label, milestone id)`.
///
/// **A release is never the answer.** ADR-0022 says a task stays in exactly one
/// cut "because a release claims none of them", and a release nonetheless
/// carries its cuts' members for the rollup (`A1-146`) -- so this walk returned
/// whichever of the two came first, and on `p00016` that was the release for
/// every task in the plan (`A1-240`). The model was right and the derivation
/// read the borrowed list as ownership.
///
/// **Nor is a container** (`A1-333`, ADR-0042), which is the same defect in a
/// second shape: a milestone declaring `contains` carries the work beneath it
/// for its rollup, and reading that as ownership gave a nested task the outer
/// cut rather than the one that lists it. ADR-0042's own stated test is that a
/// task belongs to exactly one cut at any depth, and it failed on the first
/// four-level plan built against it. `borrows` is what says which rows lend.
fn cut_of(roadmap: &Roadmap, id: &str) -> (Option<String>, Option<String>) {
    for cut in roadmap.cuts.iter().filter(|c| !c.borrows) {
        if cut.milestone_id == id {
            return (Some(cut.version.clone()), Some(cut.milestone_id.clone()));
        }
        if cut.members.iter().any(|m| m.id == id) {
            return (Some(cut.version.clone()), Some(cut.milestone_id.clone()));
        }
    }
    (None, None)
}

/// The schema for [`TaskExport`], derived from the type rather than written by
/// hand, and gated by `cargo xtask gen-schema --check` like the others.
#[must_use]
pub fn derived_schema() -> serde_json::Value {
    let schema = schemars::schema_for!(TaskExport);
    serde_json::to_value(schema).expect("schema is serialisable")
}

#[cfg(test)]
mod tests {

    /// `TaskExport`'s schema is a gated artefact (`gen-schema --check`) and was
    /// the one derived schema with no test that it is valid. `load.rs` has had
    /// this test since `A0-002`, which cites it as that task's proof.
    #[test]
    fn derived_schema_is_valid_json_schema() {
        let schema = derived_schema();
        assert!(jsonschema::validator_for(&schema).is_ok());
    }

    #[test]
    fn required_nullable_preserves_the_option_schema_identity() {
        assert_eq!(
            <RequiredNullable<String> as JsonSchema>::schema_name(),
            <Option<String> as JsonSchema>::schema_name()
        );
    }

    use super::*;
    use crate::task::Task;

    fn task(id: &str, phase: u32, status: Status, effort: Option<f64>, tags: &[&str]) -> Task {
        Task {
            id: id.to_string(),
            title: format!("{id} title"),
            phase,
            status,
            owner: None,
            effort_days: effort,
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

    /// **A release is never a task's cut.**
    ///
    /// ADR-0022: a task stays in exactly one cut, "because a release claims none
    /// of them". A release nonetheless carries its cuts' members, borrowed for
    /// its own rollup (`A1-146`), so this walk returned whichever of the two came
    /// first in roadmap order -- and on `p00016` that was the release, for every
    /// task in an eleven-cut plan. The model was right and the derivation read a
    /// borrowed list as ownership (`A1-240`).
    /// Proves the named behaviour `rollup.cut_excludes_releases` (`A1-211`).
    #[test]
    fn a_release_is_never_the_cut_a_task_is_in() {
        let mut cut = task("M-s0", 0, Status::Todo, None, &["milestone", "s0"]);
        cut.blocked_by = vec!["A1".into()];
        // Named first, so roadmap order alone would answer with it.
        let mut release = task(
            "M-v01",
            0,
            Status::Todo,
            None,
            &["milestone", "release", "v0.1"],
        );
        release.blocked_by = vec!["M-s0".into()];
        let tasks = [
            task("A1", 0, Status::Todo, Some(2.0), &["kind-b"]),
            release,
            cut,
        ];
        let export = Graph::new(tasks.iter()).task_rows();

        let a1 = export.tasks.iter().find(|t| t.id == "A1").expect("A1");
        assert_eq!(
            a1.cut.as_deref(),
            Some("s0"),
            "the release was reported as the cut the task is in"
        );
        assert_eq!(a1.milestone.as_deref(), Some("M-s0"));

        // And the release is in no cut, because it is not in one.
        let rel = export
            .tasks
            .iter()
            .find(|t| t.id == "M-v01")
            .expect("M-v01");
        assert_eq!(rel.cut, None, "a release reported itself as a cut");
        assert_eq!(rel.milestone, None);
    }

    #[test]
    fn a_task_carries_the_cut_it_belongs_to() {
        // A consumer that had to work out cut membership from `blocked_by` would
        // be re-implementing the graph, which is the thing a tool is for.
        let mut m = task("M-s0", 0, Status::Todo, None, &["milestone", "s0"]);
        m.blocked_by = vec!["A1".into()];
        let tasks = [task("A1", 0, Status::Todo, Some(2.0), &["kind-b"]), m];
        let g = Graph::new(tasks.iter());
        let export = g.task_rows();

        let a1 = export.tasks.iter().find(|t| t.id == "A1").expect("A1");
        assert_eq!(a1.cut.as_deref(), Some("s0"));
        assert_eq!(a1.milestone.as_deref(), Some("M-s0"));
        assert!(a1.ready, "no blockers");
        assert!(!a1.milestone_task);

        let m = export.tasks.iter().find(|t| t.id == "M-s0").expect("M-s0");
        assert!(m.milestone_task, "a checkpoint is flagged, not omitted");
    }

    #[test]
    fn ready_matches_the_ready_command_including_inherited_edges() {
        // Two answers to one question is the drift this tool exists to end, so
        // the exported flag is the graph's, not a second computation.
        let mut m0 = task("M-s0", 0, Status::Todo, None, &["milestone", "s0"]);
        m0.blocked_by = vec!["A1".into()];
        let mut m1 = task("M-s1", 1, Status::Todo, None, &["milestone", "s1"]);
        m1.blocked_by = vec!["B1".into(), "M-s0".into()];
        let tasks = [
            task("A1", 0, Status::Todo, Some(1.0), &[]),
            task("B1", 1, Status::Todo, Some(1.0), &[]),
            m0,
            m1,
        ];
        let g = Graph::new(tasks.iter());
        let export = g.task_rows();
        let ready: Vec<&str> = export
            .tasks
            .iter()
            .filter(|t| t.ready)
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(ready, vec!["A1"], "B1 waits for cut 0");
    }

    #[test]
    fn the_schema_string_is_what_a_consumer_pins() {
        let tasks: [Task; 0] = [];
        let g = Graph::new(tasks.iter());
        assert_eq!(g.task_rows().schema, "aplan/tasks/v2");
    }

    #[test]
    fn a_task_in_several_cuts_reports_the_first_in_roadmap_order() {
        // Legal only where `require_cut_membership` is off, and deterministic
        // there rather than dependent on which milestone was read first.
        let mut m1 = task("M-s1", 1, Status::Todo, None, &["milestone", "s1"]);
        m1.blocked_by = vec!["A1".into()];
        let mut m0 = task("M-s0", 0, Status::Todo, None, &["milestone", "s0"]);
        m0.blocked_by = vec!["A1".into()];
        let tasks = [task("A1", 0, Status::Todo, Some(1.0), &[]), m1, m0];
        let g = Graph::new(tasks.iter());
        let rows = g.task_rows();
        let a1 = rows.tasks.iter().find(|t| t.id == "A1").expect("A1");
        assert_eq!(
            a1.cut.as_deref(),
            Some("s0"),
            "roadmap order, not file order"
        );
    }

    #[test]
    fn a_dropped_task_is_present_and_distinguishable() {
        // A rollup must be able to honour `A1-074` without guessing.
        let tasks = [task("D1", 1, Status::Dropped, Some(3.0), &[])];
        let g = Graph::new(tasks.iter());
        let export = g.task_rows();
        assert_eq!(export.tasks[0].status, Status::Dropped);
    }

    #[test]
    fn an_unestimated_task_is_null_rather_than_zero() {
        // An unestimated task is not a free one, and a consumer summing a `0`
        // would report a plan as cheaper than anybody claimed.
        let tasks = [task("A1", 0, Status::Todo, None, &[])];
        let g = Graph::new(tasks.iter());
        assert_eq!(g.task_rows().tasks[0].effort_days, None);
    }
}
