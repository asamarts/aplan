//! Reading `p0000019`'s hand-maintained `roadmap.json` status contract (`A1-031`).
//!
//! The file describes itself as a "Status and dependency projection", and that is
//! exactly the shape this tool validates: 21 phases, a workstream whose
//! milestones carry a real `depends_on` graph, and a status on every one of them.
//! What it does *not* have is anything checking that a `done` is done, which is
//! the measurement the pilot (`A1-036`) exists to take.
//!
//! Three mappings carry the weight, and each is stated in the report rather than
//! left for a reader to infer:
//!
//! - **`depends_on` becomes `blocked_by`**, so the graph is validated on arrival.
//!   A reference to a milestone that does not exist is `APLAN003`, and a cycle is
//!   `APLAN004`; neither was checkable before, because the source's own linter
//!   proves identifiers resolve to files, not that the graph is sound.
//! - **The status vocabulary is mapped explicitly**, not by name-matching luck. A
//!   source's `done` arrives as `todo` carrying the claim as a tag, because a
//!   hand-maintained status is the claim this tool exists to doubt (ADR-0012
//!   clause 4).
//! - **The workstream becomes a milestone task** whose `blocked_by` is its own
//!   milestones (ADR-0008), so the cut it already describes is a cut here too,
//!   and `APLAN007` then refuses to let it be `done` while a member is not.

use std::collections::{BTreeMap, BTreeSet};

use aplan_core::task::{Evidence, EvidenceKind, Status, Task};
use serde::Deserialize;

use crate::report::{Notice, NoticeKind, Report};
use crate::{ImplementedAs, Import, ImportError, Options, ProposalSource};

/// The `roadmap.json` flavor.
#[derive(Debug)]
pub struct RoadmapJson;

#[derive(Debug, Deserialize)]
struct Document {
    #[serde(default)]
    phases: Vec<Phase>,
    #[serde(default)]
    workstreams: Vec<Workstream>,
    /// Everything else the file carries, so a field added upstream after this
    /// code was written is still reported rather than silently ignored.
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Phase {
    id: String,
    title: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    blurb: Option<String>,
    /// What this phase waits on, which a phase may carry as well as a milestone.
    ///
    /// Read rather than left to `extra`, for the reason the milestone graph is:
    /// the model holds a dependency edge -- that is what `blocked_by` is -- so
    /// dropping these under "not held by the task model" lost five real edges
    /// from the pilot's plan and told the reader there had been nothing to lose
    /// (`A1-161`, from `A1-036`).
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Workstream {
    id: String,
    title: String,
    #[serde(default)]
    artifact: Option<String>,
    #[serde(default)]
    milestones: Vec<Milestone>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Milestone {
    id: String,
    title: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    /// Documents the source cites as proof, `path#anchor`.
    ///
    /// Read rather than left to `extra`, because dropping it was the sharpest
    /// loss this importer had: a tool whose thesis is that evidence settles a
    /// claim, discarding the evidence a source brought and then telling the
    /// adopter to go and find some (`A1-080`).
    #[serde(default)]
    evidence: Vec<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

/// How one source status becomes an `aplan` one.
///
/// A table rather than a `match` on strings scattered through the code, because
/// the mapping is the part a reader of the import has to be able to audit: it is
/// printed in the report, and this is the single place it is decided.
const STATUS_MAP: &[(&str, Status, &str)] = &[
    ("not_started", Status::Todo, "not started here either"),
    ("in_progress", Status::InProgress, "carried across as-is"),
    ("blocked", Status::Blocked, "carried across as-is"),
    ("dropped", Status::Dropped, "carried across as-is"),
    (
        "done",
        Status::Todo,
        "a hand-maintained `done` is a claim, so it arrives `todo` with the claim as a tag",
    ),
    // A requirement that turned out not to apply is a decision, not outstanding
    // work, and `todo` said the opposite about three of the pilot's milestones
    // (`A1-160`). `dropped` is aplan's word for "abandoned, kept for the record",
    // and importing it manufactures nothing: this tool doubts a claim that work
    // was *finished*, which is why `done` still arrives `todo`, and a decision
    // not to do something needs no proof.
    //
    // The source treats it as satisfying a dependency; this model does not, and
    // the difference is reported rather than papered over. See the notice below.
    (
        "not_applicable",
        Status::Dropped,
        "a requirement that turned out not to apply is a decision, so it arrives `dropped`",
    ),
];

/// Write the plan back into the shape `roadmap.json` consumers read (`A1-033`).
///
/// **A merge, not a render, and it has to be.** That file carries fields this
/// model does not hold -- `project`, `note`, `adrs`, `specs`, a workstream's
/// `authority` and `status_validity`, a phase's `blurb` and `blockers` -- and a
/// web build and a spec linter read them. Generating from the plan alone would
/// produce a file that validates here and breaks there, which is the opposite of
/// what generating it is for. So the existing document is the template: the plan
/// supplies the half that drifts, and every other byte of meaning is carried
/// through untouched.
///
/// **The template is authoritative for what exists; the plan for status and
/// dependencies.** A milestone the plan has and the file does not is *reported*
/// rather than invented, because where it belongs in a foreign schema is not
/// something this can know -- the same refusal to guess that `A1-080` made of an
/// imported `traces` path. Adding it once is a human's job; keeping its status
/// true is this function's.
///
/// # Errors
///
/// [`ImportError::Malformed`] when the template is not a JSON object.
pub fn render(
    template: &serde_json::Value,
    tasks: &[&Task],
    source_name: &str,
    report: &mut Report,
) -> Result<serde_json::Value, ImportError> {
    let by_id: BTreeMap<&str, &Task> = tasks.iter().map(|t| (t.id.as_str(), *t)).collect();
    let mut out = template.clone();
    let mut placed: BTreeSet<String> = BTreeSet::new();
    // Every id this importer renames, mapped back to the source's own spelling,
    // built once and used for both halves. The first version passed it to the
    // phases and not to the milestones, so a milestone that came to depend on a
    // phase wrote `PHASE-00` into a document that spells it `0` -- an id that
    // repository's own linter reports as a missing dependency (`A1-033`'s
    // audit).
    let spelling = spellings(template);
    // What the document declares and the plan does not, so a status nothing
    // backs is not left standing in a generated file.
    let mut orphaned: Vec<String> = Vec::new();

    let Some(root) = out.as_object_mut() else {
        return Err(ImportError::Malformed {
            source_name: source_name.to_string(),
            message: "the document is not a JSON object, so there is nothing to write into"
                .to_string(),
        });
    };

    if let Some(phases) = root.get_mut("phases").and_then(|p| p.as_array_mut()) {
        for phase in phases.iter_mut() {
            let Some(raw) = phase.get("id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let id = phase_id(raw);
            match by_id.get(id.as_str()).copied() {
                Some(task) => {
                    update(phase, task, &spelling);
                    placed.insert(id);
                }
                None => orphaned.push(id),
            }
        }
    }

    if let Some(streams) = root.get_mut("workstreams").and_then(|w| w.as_array_mut()) {
        for ws in streams.iter_mut() {
            let Some(milestones) = ws.get_mut("milestones").and_then(|m| m.as_array_mut()) else {
                continue;
            };
            for milestone in milestones.iter_mut() {
                let Some(id) = milestone
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                else {
                    continue;
                };
                match by_id.get(id.as_str()).copied() {
                    Some(task) => {
                        update(milestone, task, &spelling);
                        placed.insert(id);
                    }
                    None => orphaned.push(id),
                }
            }
        }
    }

    // What the plan has and the file has nowhere to put. Reported once, with the
    // ids, because a generated contract silently missing a milestone is the
    // drift this command exists to retire arriving from the other direction.
    let missing: Vec<&str> = tasks
        .iter()
        .filter(|t| !t.is_milestone() && !placed.contains(&t.id))
        .map(|t| t.id.as_str())
        .collect();
    if !missing.is_empty() {
        report.push(Notice::new(
            source_name,
            NoticeKind::DroppedField,
            format!(
                "{} task(s) are in the plan and not in this document, so their status is not \
                 written: {}. Add them to `roadmap.json` once, by hand, and this will keep them \
                 true afterwards",
                missing.len(),
                missing.join(", ")
            ),
        ));
    }
    if !orphaned.is_empty() {
        report.push(Notice::new(
            source_name,
            NoticeKind::DroppedField,
            format!(
                "this document declares {} id(s) the plan does not, so their status is left \
                 exactly as it was and nothing here backs it: {}. Import them, or delete them \
                 from `roadmap.json`",
                orphaned.len(),
                orphaned.join(", ")
            ),
        ));
    }
    Ok(out)
}

/// Every id this importer renames, mapped back to the source's own spelling.
///
/// A phase (`0` becomes `PHASE-00`) and a workstream (`W` becomes `W-CUT`), from
/// the document's own lists rather than by unpicking the shape of a string: a
/// milestone genuinely named `FOO-CUT` keeps its name.
fn spellings(template: &serde_json::Value) -> BTreeMap<String, String> {
    let list = |key: &str| {
        template
            .get(key)
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| p.get("id").and_then(serde_json::Value::as_str))
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let phases = list("phases").into_iter().map(|raw| (phase_id(&raw), raw));
    let streams = list("workstreams")
        .into_iter()
        .map(|raw| (format!("{raw}-CUT"), raw));
    phases.chain(streams).collect()
}

/// Set the two fields the plan owns on one phase or milestone.
fn update(entry: &mut serde_json::Value, task: &Task, spelling: &BTreeMap<String, String>) {
    let Some(object) = entry.as_object_mut() else {
        return;
    };
    object.insert(
        "status".to_string(),
        serde_json::Value::String(source_status(task)),
    );
    // Only where the document already carries the key: adding `depends_on` to a
    // phase that never had one changes the shape of a file another build reads,
    // for a task that has no edges anyway.
    if object.contains_key("depends_on") || !task.blocked_by.is_empty() {
        object.insert(
            "depends_on".to_string(),
            serde_json::Value::Array(
                task.blocked_by
                    .iter()
                    .map(|b| {
                        serde_json::Value::String(
                            spelling
                                .get(b.as_str())
                                .cloned()
                                .unwrap_or_else(|| b.clone()),
                        )
                    })
                    .collect(),
            ),
        );
    }
}

/// A task's status in the source's own vocabulary.
///
/// The plan's verdict, in their words -- except for the one distinction the
/// import preserved: `not_applicable` is a requirement discharged rather than
/// work abandoned (`A1-160`), and the tag is where that word was kept. A source
/// `done` that arrived `todo` stays `not_started`, deliberately: this tool has
/// not seen proof of it, and writing `done` back would launder the claim through
/// a file that is supposed to have stopped being hand-maintained.
fn source_status(task: &Task) -> String {
    let tagged = |word: &str| {
        task.tags
            .iter()
            .any(|g| g.as_str() == format!("roadmap-json:{word}"))
    };
    match task.status {
        Status::Done => "done",
        Status::InProgress => "in_progress",
        Status::Blocked => "blocked",
        Status::Todo => "not_started",
        Status::Dropped if tagged("not_applicable") => "not_applicable",
        Status::Dropped => "dropped",
    }
    .to_string()
}

/// Map a source status, reporting anything the table does not know.
fn map_status(
    raw: Option<&str>,
    id: &str,
    source_name: &str,
    options: &Options,
    report: &mut Report,
) -> (Status, Vec<String>, Vec<Evidence>) {
    let Some(raw) = raw else {
        report.push(Notice::new(
            source_name,
            NoticeKind::Provenance,
            format!("{id} carries no status; imported as `todo`"),
        ));
        return (Status::Todo, Vec::new(), Vec::new());
    };
    let tag = format!("roadmap-json:{raw}");

    let Some((_, mapped, _)) = STATUS_MAP.iter().find(|(name, _, _)| *name == raw) else {
        report.push(Notice::new(
            source_name,
            NoticeKind::Provenance,
            format!(
                "{id} has status `{raw}`, which this importer does not know; imported as \
                 `todo` with the source's word kept as a tag"
            ),
        ));
        return (Status::Todo, vec![tag], Vec::new());
    };

    if raw == "not_applicable" {
        // The one place the two models genuinely differ, said once per task
        // rather than left for `check` to raise as a surprise. `p0000019`'s own
        // linter counts `not_applicable` as satisfying a dependency; here a
        // `dropped` blocker stops its dependents, deliberately -- a prerequisite
        // somebody abandoned is a plan its author has to resolve, and `A1-081`
        // exempts only a cut's own member, because a checkpoint is reached when
        // its live work is.
        report.push(Notice::new(
            source_name,
            NoticeKind::Provenance,
            format!(
                "{id} has status `not_applicable`; imported as `dropped`, a decision recorded \
                 rather than work outstanding. If a live task still lists it in `blocked_by`, \
                 `APLAN020` will say so: drop that edge, because a requirement that no longer \
                 applies is not something to wait for"
            ),
        ));
        return (*mapped, vec![tag], Vec::new());
    }
    if raw != "done" {
        return (*mapped, vec![tag], Vec::new());
    }
    match &options.implemented_as {
        ImplementedAs::Done { as_of } => {
            report.push(Notice::new(
                source_name,
                NoticeKind::Provenance,
                format!(
                    "{id} imported `done` on the source's say-so, which is note-level \
                     provenance; attach real proof with `aplan close {id} --evidence \
                     commit:COMMIT_SHA`"
                ),
            ));
            (
                Status::Done,
                vec![tag],
                vec![Evidence {
                    kind: EvidenceKind::Note,
                    reference: format!("imported from {source_name}: roadmap.json said `done`"),
                    captured: Some(as_of.clone()),
                }],
            )
        }
        ImplementedAs::Todo => {
            report.push(Notice::new(
                source_name,
                NoticeKind::Provenance,
                format!(
                    "{id} claims `done`; imported as `todo` (the claim is what this tool is \
                     here to check). Close it with `aplan close {id} --evidence commit:COMMIT_SHA`"
                ),
            ));
            (Status::Todo, vec![tag], Vec::new())
        }
    }
}

/// A phase id (`0`) as a task id (`PHASE-00`).
///
/// Prefixed and padded because `0` is neither a usable file name nor something a
/// reader would recognise in a diagnostic, and because sorting `PHASE-2` before
/// `PHASE-10` would misrepresent the order the source is written in.
fn phase_id(raw: &str) -> String {
    match raw.parse::<u32>() {
        Ok(n) => format!("PHASE-{n:02}"),
        Err(_) => format!("PHASE-{raw}"),
    }
}

/// Name every field the task model does not hold (ADR-0012 clause 3).
fn report_losses(doc: &Document, source_name: &str, report: &mut Report) {
    for (field, value) in &doc.extra {
        match (field.as_str(), value) {
            // Reference registries, not work: importing them as tasks would put
            // a status on a record that describes a document's state, and the
            // roadmap already tracks the work that changes them.
            ("adrs" | "specs", serde_json::Value::Array(items)) => {
                report.push(Notice::new(
                    source_name,
                    NoticeKind::DroppedField,
                    format!(
                        "`{field}` lists {} record(s) with their own statuses; not imported, \
                         because they are reference documents rather than work",
                        items.len()
                    ),
                ));
            }
            _ => report.dropped(source_name, field),
        }
    }
    for phase in &doc.phases {
        for field in phase.extra.keys() {
            report.dropped(source_name, &format!("phases[{}].{field}", phase.id));
        }
    }
    for ws in &doc.workstreams {
        for field in ws.extra.keys() {
            // `status_validity` is not one more field this model does not hold:
            // it is a wall-clock control the source's own gate enforces, and
            // saying only "not held" leaves it sixth in a list of six with
            // nothing to mark it. `A1-162` decided not to model a declared
            // expiry -- the only clock that could read one is the one this tool
            // deliberately does not have -- so the honest thing is to say whose
            // job it stays.
            if field == "status_validity" {
                report.push(Notice::new(
                    source_name,
                    NoticeKind::DroppedField,
                    format!(
                        "`workstreams[{}].status_validity` declares how long that status is good \
                         for; not imported, because nothing here reads a wall clock. Keep the \
                         gate that enforces it: `aplan stale --as-of <date>` asks a related \
                         question from git, and it is a different one",
                        ws.id
                    ),
                ));
                continue;
            }
            report.dropped(source_name, &format!("workstreams[{}].{field}", ws.id));
        }
        for m in &ws.milestones {
            for field in m.extra.keys() {
                report.dropped(source_name, &format!("milestones[{}].{field}", m.id));
            }
        }
    }
}

/// Cross-references named in prose, which stay prose.
///
/// `SPEC-0009` identifies a document the JSON never gives a path for, and a
/// `traces` entry has to resolve or `APLAN008` fails the plan. Inventing
/// `specs/0009-something.md` from a convention this importer cannot see would be
/// a guess that breaks the import when it is wrong, so the references are left
/// in the body where they were and named here.
fn report_references(body: &str, id: &str, source_name: &str, report: &mut Report) {
    let mut found: Vec<String> = Vec::new();
    for word in body.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        let is_ref = ["SPEC-", "ADR-", "RFC-"]
            .iter()
            .any(|p| word.starts_with(p) && word.len() > p.len());
        // A task naming itself is not a cross-reference, and reporting it as one
        // is noise in a report whose whole value is that every line means something.
        if is_ref && word != id && !found.iter().any(|f| f == word) {
            found.push(word.to_string());
        }
    }
    if !found.is_empty() {
        report.push(Notice::new(
            source_name,
            NoticeKind::DroppedField,
            format!(
                "{id} names {} in its prose; kept in the body but not converted to `traces`, \
                 because the source gives ids rather than paths and a trace has to resolve",
                found.join(", ")
            ),
        ));
    }
}

fn body_for(source_name: &str, kind: &str, title: &str, blurb: Option<&str>) -> String {
    use std::fmt::Write as _;

    let mut body = format!("## Why\n\nImported from `{source_name}` ({kind} \"{title}\").\n");
    if let Some(blurb) = blurb.map(str::trim).filter(|b| !b.is_empty()) {
        let _ = write!(body, "\n{blurb}\n");
    }
    // The constant, not a copy of its text: a rule reads it to refuse the
    // prompt back as an answer, and a copy here would drift out of that the
    // day somebody reworded one of them (`A1-271`).
    let _ = write!(
        body,
        "\n## Acceptance criteria\n\n- [ ] {}\n",
        aplan_core::scaffold::criterion_prompt()
    );
    body
}

impl ProposalSource for RoadmapJson {
    fn flavor(&self) -> &'static str {
        "roadmap-json"
    }

    fn parse(
        &self,
        source_name: &str,
        contents: &str,
        options: &Options,
    ) -> Result<Import, ImportError> {
        if contents.trim().is_empty() {
            return Err(ImportError::Empty {
                source_name: source_name.to_string(),
                detail: None,
            });
        }
        let doc: Document = serde_json::from_str(contents).map_err(|e| ImportError::Malformed {
            source_name: source_name.to_string(),
            message: e.to_string(),
        })?;
        if doc.phases.is_empty() && doc.workstreams.iter().all(|w| w.milestones.is_empty()) {
            return Err(ImportError::Empty {
                source_name: source_name.to_string(),
                detail: None,
            });
        }

        let mut report = Report::new();
        let mut tasks: Vec<Task> = Vec::new();
        let mut bodies: Vec<String> = Vec::new();

        // The mapping is printed, not implied: a reader of the import has to be
        // able to audit what happened to each of the source's words.
        report.push(Notice::new(
            source_name,
            NoticeKind::Provenance,
            format!(
                "status mapping: {}",
                STATUS_MAP
                    .iter()
                    .map(|(name, mapped, why)| format!("`{name}` -> `{mapped}` ({why})"))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        ));
        report_losses(&doc, source_name, &mut report);

        // A phase may depend on another phase or on a workstream, both of which
        // this importer renames (`0` -> `PHASE-00`, `W` -> `W-CUT`), or on a
        // milestone, whose id it keeps. Resolved against the source's own lists
        // rather than guessed from the shape of the string -- and resolved at
        // all, because renaming a thing and then reporting the edge to it as
        // naming something unknown is the tool contradicting itself
        // (`A1-161`'s audit).
        let renamed: BTreeMap<&str, String> = doc
            .phases
            .iter()
            .map(|p| (p.id.as_str(), phase_id(&p.id)))
            .chain(
                doc.workstreams
                    .iter()
                    .map(|w| (w.id.as_str(), format!("{}-CUT", w.id))),
            )
            .collect();

        for phase in &doc.phases {
            let id = phase_id(&phase.id);
            let (status, tags, evidence) = map_status(
                phase.status.as_deref(),
                &id,
                source_name,
                options,
                &mut report,
            );
            let body = body_for(source_name, "phase", &phase.title, phase.blurb.as_deref());
            report_references(&body, &id, source_name, &mut report);
            tasks.push(Task {
                id,
                title: phase.title.clone(),
                phase: options.phase,
                status,
                owner: None,
                effort_days: None,
                blocked_by: phase
                    .depends_on
                    .iter()
                    .map(|d| {
                        renamed
                            .get(d.as_str())
                            .cloned()
                            .unwrap_or_else(|| d.clone())
                    })
                    .collect(),
                prefer_after: Vec::new(),
                evidence,
                paths: Vec::new(),
                traces: Vec::new(),
                trace_fingerprints: BTreeMap::new(),
                tags,
                revisit_at: None,
                version_impact: None,
                version_impact_reason: None,
            });
            bodies.push(body);
        }

        for ws in &doc.workstreams {
            read_workstream(
                ws,
                source_name,
                options,
                &mut report,
                &mut tasks,
                &mut bodies,
            );
        }

        Ok(Import {
            tasks,
            report,
            bodies,
        })
    }
}

/// One workstream: its milestones, then the cut over them.
///
/// Split out of `parse` to keep each half readable, not because they are
/// different concerns: the cut exists only to group the milestones written just
/// above it, so the two belong in one place.
fn read_workstream(
    ws: &Workstream,
    source_name: &str,
    options: &Options,
    report: &mut Report,
    tasks: &mut Vec<Task>,
    bodies: &mut Vec<String>,
) {
    {
        {
            for m in &ws.milestones {
                let (status, tags, evidence) =
                    map_status(m.status.as_deref(), &m.id, source_name, options, report);
                let body = body_for(source_name, "milestone", &m.title, None);
                // `traces`, not `evidence`. The citation is a document, and
                // `APLAN008` then checks the anchor resolves on every run, which
                // is a stronger guarantee than the source had. Importing it as
                // `file` evidence would let `close` use it, and manufacturing
                // evidence during an import is exactly what ADR-0012 refused
                // when it declined to import a `done` (`A1-080`).
                //
                // The notice names the citation the reader can attach *when they
                // close it*: `A1-159` taught `file` to hold a section, so the
                // source'"'"'s own proof is now attachable, and an adopter who is
                // told only that it became provenance will not find that out.
                if !m.evidence.is_empty() {
                    report.push(Notice::new(
                        source_name,
                        NoticeKind::Provenance,
                        format!(
                            "{}: {} `evidence` citation(s) became `traces`; `check` fails \
                             (APLAN008) if an anchor does not resolve, which is the point of \
                             recording them. Cite the same reference as `file:` evidence when you \
                             close the task, e.g. `--evidence file:{}`",
                            m.id,
                            m.evidence.len(),
                            m.evidence[0]
                        ),
                    ));
                }
                tasks.push(Task {
                    id: m.id.clone(),
                    title: m.title.clone(),
                    phase: options.phase,
                    status,
                    owner: None,
                    effort_days: None,
                    // The graph, carried across so `check` can validate it: an
                    // unknown reference is APLAN003 and a cycle is APLAN004,
                    // neither of which anything checked before.
                    blocked_by: m.depends_on.clone(),
                    prefer_after: Vec::new(),
                    evidence,
                    paths: Vec::new(),
                    traces: m.evidence.clone(),
                    trace_fingerprints: BTreeMap::new(),
                    tags,
                    revisit_at: None,
                    version_impact: None,
                    version_impact_reason: None,
                });
                bodies.push(body);
            }

            // The workstream is a cut: it groups milestones and does no work of
            // its own (ADR-0008), which is exactly a `milestone`-tagged task.
            // `APLAN007` then refuses to let it be `done` while a member is not,
            // which is a guarantee the source could not express.
            if !ws.milestones.is_empty() {
                let body = body_for(source_name, "workstream", &ws.title, None);
                report_references(
                    &format!("{} {}", ws.id, ws.title),
                    &ws.id,
                    source_name,
                    report,
                );
                let traces = ws.artifact.clone().into_iter().collect::<Vec<_>>();
                if !traces.is_empty() {
                    report.push(Notice::new(
                        source_name,
                        NoticeKind::Provenance,
                        format!(
                            "{}: `artifact` became a `traces` entry; `check` fails (APLAN008) \
                             if that file is not there, which is the point of recording it",
                            ws.id
                        ),
                    ));
                }
                tasks.push(Task {
                    id: format!("{}-CUT", ws.id),
                    title: ws.title.clone(),
                    phase: options.phase,
                    status: Status::Todo,
                    owner: None,
                    effort_days: None,
                    blocked_by: ws.milestones.iter().map(|m| m.id.clone()).collect(),
                    prefer_after: Vec::new(),
                    evidence: Vec::new(),
                    paths: Vec::new(),
                    traces,
                    trace_fingerprints: BTreeMap::new(),
                    tags: vec!["milestone".to_string()],
                    revisit_at: None,
                    version_impact: None,
                    version_impact_reason: None,
                });
                bodies.push(body);
            }
        }
    }
}

#[cfg(test)]
mod tests {

    /// A milestone that cites the documents proving it, which is what the real
    /// source does on 16 of its 25.
    const WITH_EVIDENCE: &str = r#"{
      "format_version": 1,
      "phases": [],
      "workstreams": [{
        "id": "RFC-0001",
        "title": "A workstream",
        "milestones": [{
          "id": "M-1",
          "title": "A milestone",
          "status": "done",
          "evidence": ["docs/decisions/0010-hold.md#status", "docs/decisions/0011-hold.md#status"]
        }]
      }]
    }"#;

    const WITH_NOT_APPLICABLE: &str = r#"{
      "format_version": 1,
      "phases": [],
      "workstreams": [{
        "id": "RFC-0001",
        "title": "A workstream",
        "milestones": [
          {"id": "M-NA", "title": "A requirement that stopped applying", "status": "not_applicable"},
          {"id": "M-LIVE", "title": "Still to do", "status": "not_started", "depends_on": ["M-NA"]}
        ]
      }]
    }"#;

    /// A requirement that turned out not to apply is a decision, not outstanding
    /// work. It arrived `todo`, which said the opposite about three of the
    /// pilot's milestones (`A1-160`, from `A1-036`).
    ///
    /// Importing it as `dropped` manufactures nothing: this tool doubts a claim
    /// that work was *finished*, which is why a source's `done` still arrives
    /// `todo`, and a decision not to do something needs no proof.
    #[test]
    fn a_requirement_that_stopped_applying_arrives_as_a_decision() {
        let imported = RoadmapJson
            .parse("roadmap.json", WITH_NOT_APPLICABLE, &Options::default())
            .expect("imports");
        let na = imported
            .tasks
            .iter()
            .find(|t| t.id == "M-NA")
            .expect("the milestone");
        assert_eq!(na.status, Status::Dropped);
        assert!(
            na.tags.iter().any(|g| g == "roadmap-json:not_applicable"),
            "the source's own word is kept: {:?}",
            na.tags
        );
        assert!(
            na.evidence.is_empty(),
            "a cancellation is not a claim needing proof: {:?}",
            na.evidence
        );
    }

    /// The two models differ here and the notice says so rather than leaving
    /// `check` to surprise the reader. `p0000019`'s linter counts
    /// `not_applicable` as satisfying a dependency; here a `dropped` blocker
    /// stops its dependents, and the remedy is to drop the edge.
    #[test]
    fn the_notice_names_the_edge_the_reader_will_have_to_resolve() {
        let imported = RoadmapJson
            .parse("roadmap.json", WITH_NOT_APPLICABLE, &Options::default())
            .expect("imports");
        let notice = imported
            .report
            .notices()
            .iter()
            .find(|n| n.message.contains("M-NA has status `not_applicable`"))
            .expect("a provenance notice");
        assert!(notice.message.contains("`dropped`"), "{}", notice.message);
        assert!(notice.message.contains("APLAN020"), "{}", notice.message);
        assert!(
            notice.message.contains("drop that edge"),
            "the remedy, not just the rule: {}",
            notice.message
        );

        // And the edge really is there to resolve: the importer keeps the
        // source's graph rather than quietly deleting the dependency for it.
        let live = imported
            .tasks
            .iter()
            .find(|t| t.id == "M-LIVE")
            .expect("the dependent");
        assert_eq!(live.blocked_by, vec!["M-NA".to_string()]);
    }

    /// A status this importer has never heard of still arrives `todo` with the
    /// word kept, which is the fallback and must not have moved.
    #[test]
    fn an_unknown_status_still_arrives_as_outstanding_work() {
        let doc = WITH_NOT_APPLICABLE.replace("not_applicable", "mothballed");
        let imported = RoadmapJson
            .parse("roadmap.json", &doc, &Options::default())
            .expect("imports");
        let task = imported
            .tasks
            .iter()
            .find(|t| t.id == "M-NA")
            .expect("the milestone");
        assert_eq!(task.status, Status::Todo);
        assert!(task.tags.iter().any(|g| g == "roadmap-json:mothballed"));
    }

    /// The notice has to name the way out, not only the loss. `A1-159` taught
    /// `file` evidence to hold a section, so the source's own proof is
    /// attachable at close time -- and an adopter told only that it became
    /// provenance has no way to learn that.
    #[test]
    fn the_notice_names_the_citation_to_attach_when_closing() {
        let imported = RoadmapJson
            .parse("roadmap.json", WITH_EVIDENCE, &Options::default())
            .expect("imports");
        let notice = imported
            .report
            .notices()
            .iter()
            .find(|n| n.message.contains("became `traces`"))
            .expect("a provenance notice");
        assert!(
            notice.message.contains("--evidence file:"),
            "{}",
            notice.message
        );
        assert!(
            notice
                .message
                .contains("docs/decisions/0010-hold.md#status"),
            "and the reference itself: {}",
            notice.message
        );
    }

    #[test]
    fn milestone_evidence_becomes_traces_rather_than_being_dropped() {
        // A tool whose thesis is that evidence settles a claim, discarding the
        // evidence a source brought, was the sharpest loss it had (`A1-080`).
        let imported = RoadmapJson
            .parse("roadmap.json", WITH_EVIDENCE, &Options::default())
            .expect("imports");
        let m = imported
            .tasks
            .iter()
            .find(|t| t.id == "M-1")
            .expect("the milestone");
        assert_eq!(
            m.traces,
            vec![
                "docs/decisions/0010-hold.md#status".to_string(),
                "docs/decisions/0011-hold.md#status".to_string()
            ]
        );
    }

    #[test]
    fn no_message_claims_the_model_cannot_hold_evidence() {
        // The old report said "`evidence` is not held by the task model", which
        // was false: it is, in exactly that shape, and the same importer already
        // mapped `artifact` to a `traces` entry.
        let imported = RoadmapJson
            .parse("roadmap.json", WITH_EVIDENCE, &Options::default())
            .expect("imports");
        let msgs = messages(&imported);
        assert!(
            !msgs.iter().any(|m| m.contains("evidence` is not held")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter().any(|m| m.contains("became `traces`")),
            "the mapping must be named: {msgs:?}"
        );
        // ... and the reader is warned what will happen on their first check.
        assert!(msgs.iter().any(|m| m.contains("APLAN008")), "{msgs:?}");
    }

    #[test]
    fn a_milestone_with_no_evidence_gains_no_traces_and_no_notice() {
        const PLAIN: &str = r#"{
          "format_version": 1,
          "phases": [],
          "workstreams": [{
            "id": "RFC-0001",
            "title": "A workstream",
            "milestones": [{"id": "M-1", "title": "A milestone", "status": "done"}]
          }]
        }"#;
        let imported = RoadmapJson
            .parse("roadmap.json", PLAIN, &Options::default())
            .expect("imports");
        let m = imported.tasks.iter().find(|t| t.id == "M-1").expect("m");
        assert!(m.traces.is_empty());
        let msgs = messages(&imported);
        assert!(
            !msgs.iter().any(|m| m.contains("became `traces`")),
            "{msgs:?}"
        );
    }
    use super::*;

    const DOC: &str = r#"{
      "format_version": 1,
      "project": "p0000019",
      "note": "Status and dependency projection only.",
      "phases": [
        {"id": "0", "title": "Research", "status": "done", "blurb": "Cited research (ADR-0003, SPEC-0009)."},
        {"id": "13", "title": "Web explorer", "status": "in_progress", "blurb": "The SPA."}
      ],
      "workstreams": [
        {
          "id": "RFC-0001",
          "title": "Route remediation",
          "artifact": "rfcs/0001-route.md",
          "authority": "nonnormative",
          "milestones": [
            {"id": "RFC-0001-A", "title": "Containment", "status": "not_started", "depends_on": []},
            {"id": "RFC-0001-B", "title": "Restoration", "status": "not_started", "depends_on": ["RFC-0001-A"]}
          ]
        }
      ],
      "specs": [{"id": "SPEC-0001", "title": "s", "status": "draft"}],
      "adrs": [{"id": "ADR-0001", "title": "a", "status": "accepted"}]
    }"#;

    fn parse(options: &Options) -> Import {
        RoadmapJson.parse("roadmap.json", DOC, options).unwrap()
    }

    fn messages(import: &Import) -> Vec<String> {
        import
            .report
            .notices()
            .iter()
            .map(|n| n.message.clone())
            .collect()
    }

    const PHASE_DEPS: &str = r#"{
      "format_version": 1,
      "phases": [
        {"id": "0", "title": "Foundations", "status": "done"},
        {"id": "19", "title": "Refresh", "status": "in_progress",
         "depends_on": ["0", "RFC-0001-A"],
         "blockers": [{"id": "contract_unaccepted", "evidence": "rfcs/0001.md#phase-19"}]}
      ],
      "workstreams": [{
        "id": "RFC-0001",
        "title": "A workstream",
        "milestones": [{"id": "RFC-0001-A", "title": "A milestone", "status": "not_started"}]
      }]
    }"#;

    /// A phase may wait on things too, and those edges were dropped under
    /// "not held by the task model" -- which is untrue of a dependency edge, and
    /// cost the pilot's plan five of them (`A1-161`, from `A1-036`).
    #[test]
    fn a_phases_dependencies_become_blocked_by_like_a_milestones() {
        let imported = RoadmapJson
            .parse("roadmap.json", PHASE_DEPS, &Options::default())
            .expect("imports");
        let phase = imported
            .tasks
            .iter()
            .find(|t| t.id == "PHASE-19")
            .expect("the phase");
        assert_eq!(
            phase.blocked_by,
            vec!["PHASE-00".to_string(), "RFC-0001-A".to_string()],
            "a phase dependency is rewritten to the task id this importer gives it; a milestone \
             keeps its own"
        );
        // Both targets exist as tasks, so the graph `check` validates is whole.
        for id in &phase.blocked_by {
            assert!(
                imported.tasks.iter().any(|t| &t.id == id),
                "{id} is named but not imported"
            );
        }
    }

    /// A phase may also name the *workstream* it waits on, which this importer
    /// renames to the cut task (`W` -> `W-CUT`). Reporting that edge as naming
    /// something unknown, when the importer is the thing that renamed it, is the
    /// tool contradicting itself (`A1-161`'s audit).
    #[test]
    fn an_edge_to_a_renamed_workstream_resolves_to_the_cut_it_became() {
        let doc = r#"{
          "format_version": 1,
          "phases": [{"id": "1", "title": "P", "status": "not_started", "depends_on": ["W"]}],
          "workstreams": [{
            "id": "W",
            "title": "w",
            "milestones": [{"id": "M-A", "title": "m", "status": "not_started"}]
          }]
        }"#;
        let imported = RoadmapJson
            .parse("roadmap.json", doc, &Options::default())
            .expect("imports");
        let phase = imported
            .tasks
            .iter()
            .find(|t| t.id == "PHASE-01")
            .expect("the phase");
        assert_eq!(phase.blocked_by, vec!["W-CUT".to_string()]);
        assert!(
            imported.tasks.iter().any(|t| t.id == "W-CUT"),
            "and that is a task, so the edge resolves"
        );
    }

    /// The message has to mean something again: a field the model really cannot
    /// hold keeps the wording, and one it can no longer claims otherwise.
    #[test]
    fn only_a_field_the_model_cannot_hold_is_reported_as_unholdable() {
        let imported = RoadmapJson
            .parse("roadmap.json", PHASE_DEPS, &Options::default())
            .expect("imports");
        let dropped: Vec<&str> = imported
            .report
            .notices()
            .iter()
            .filter(|n| n.message.contains("is not held by the task model"))
            .map(|n| n.message.as_str())
            .collect();
        assert!(
            dropped.iter().any(|m| m.contains("phases[19].blockers")),
            "prose with its own anchors genuinely is not holdable: {dropped:?}"
        );
        assert!(
            !dropped.iter().any(|m| m.contains("depends_on")),
            "a dependency edge is exactly what `blocked_by` is: {dropped:?}"
        );
    }

    /// Validated on arrival, which is the point of reading the graph rather than
    /// dropping it: a phase naming something that does not exist is `APLAN003`
    /// on the first `check`, where before it was silence.
    #[test]
    fn a_phase_naming_something_absent_arrives_as_a_dangling_edge() {
        let doc = PHASE_DEPS.replace("\"RFC-0001-A\"]", "\"RFC-0001-NOPE\"]");
        let imported = RoadmapJson
            .parse("roadmap.json", &doc, &Options::default())
            .expect("imports");
        let phase = imported
            .tasks
            .iter()
            .find(|t| t.id == "PHASE-19")
            .expect("the phase");
        assert!(
            phase.blocked_by.contains(&"RFC-0001-NOPE".to_string()),
            "kept, so the plan says what the source said: {:?}",
            phase.blocked_by
        );
        assert!(
            !imported.tasks.iter().any(|t| t.id == "RFC-0001-NOPE"),
            "and nothing was invented to satisfy it"
        );
    }

    #[test]
    fn phases_and_milestones_both_become_tasks() {
        let import = parse(&Options::default());
        let ids: Vec<&str> = import.tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "PHASE-00",
                "PHASE-13",
                "RFC-0001-A",
                "RFC-0001-B",
                "RFC-0001-CUT"
            ]
        );
        // Zero-padded, so PHASE-02 sorts before PHASE-13 the way the source reads.
        assert!(ids[0] < ids[1]);
    }

    #[test]
    fn depends_on_becomes_blocked_by_so_the_graph_is_checked_on_arrival() {
        let import = parse(&Options::default());
        let b = import.tasks.iter().find(|t| t.id == "RFC-0001-B").unwrap();
        assert_eq!(b.blocked_by, vec!["RFC-0001-A"]);
    }

    #[test]
    fn a_hand_maintained_done_is_a_claim_not_a_status() {
        let import = parse(&Options::default());
        let phase = &import.tasks[0];
        assert_eq!(phase.status, Status::Todo, "the source said `done`");
        assert!(phase.tags.iter().any(|t| t == "roadmap-json:done"));
        assert!(phase.evidence.is_empty());
        assert!(
            messages(&import)
                .iter()
                .any(|m| m.contains("claims `done`")),
            "{:?}",
            messages(&import)
        );
    }

    #[test]
    fn in_progress_survives_because_it_claims_nothing_finished() {
        let import = parse(&Options::default());
        assert_eq!(import.tasks[1].status, Status::InProgress);
    }

    #[test]
    fn the_status_mapping_is_printed_rather_than_implied() {
        let import = parse(&Options::default());
        let mapping = messages(&import)
            .into_iter()
            .find(|m| m.starts_with("status mapping:"))
            .expect("the mapping must be in the report");
        for (name, _, _) in STATUS_MAP {
            assert!(mapping.contains(name), "{name} missing from: {mapping}");
        }
    }

    #[test]
    fn implemented_as_done_takes_the_weaker_record_and_dates_it() {
        let options = Options {
            implemented_as: ImplementedAs::Done {
                as_of: "2026-08-04".into(),
            },
            ..Options::default()
        };
        let import = parse(&options);
        let phase = &import.tasks[0];
        assert_eq!(phase.status, Status::Done);
        assert_eq!(phase.evidence.len(), 1);
        assert_eq!(phase.evidence[0].captured.as_deref(), Some("2026-08-04"));
    }

    #[test]
    fn the_workstream_becomes_a_cut_over_its_own_milestones() {
        let import = parse(&Options::default());
        let cut = import
            .tasks
            .iter()
            .find(|t| t.id == "RFC-0001-CUT")
            .unwrap();
        assert!(cut.tags.iter().any(|t| t == "milestone"));
        assert_eq!(cut.blocked_by, vec!["RFC-0001-A", "RFC-0001-B"]);
        // APLAN021: a milestone carries no effort and no paths.
        assert!(cut.effort_days.is_none() && cut.paths.is_empty());
        assert_eq!(cut.traces, vec!["rfcs/0001-route.md"]);
    }

    #[test]
    fn every_unmodelled_field_is_named() {
        let import = parse(&Options::default());
        let msgs = messages(&import);
        for expected in ["format_version", "project", "note", "authority"] {
            assert!(
                msgs.iter().any(|m| m.contains(expected)),
                "{expected} was dropped in silence: {msgs:?}"
            );
        }
        // The reference registries are named with their size, not imported.
        assert!(
            msgs.iter().any(|m| m.contains("`specs` lists 1 record")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter().any(|m| m.contains("`adrs` lists 1 record")),
            "{msgs:?}"
        );
    }

    #[test]
    fn prose_references_are_kept_and_reported_rather_than_guessed_into_traces() {
        let import = parse(&Options::default());
        assert!(
            import.bodies[0].contains("ADR-0003, SPEC-0009"),
            "the blurb must survive: {}",
            import.bodies[0]
        );
        assert!(
            messages(&import)
                .iter()
                .any(|m| m.contains("SPEC-0009") && m.contains("not converted to `traces`")),
            "{:?}",
            messages(&import)
        );
        // A path invented from an id would break the plan when it is wrong.
        assert!(import.tasks[0].traces.is_empty());
    }

    #[test]
    fn a_task_naming_itself_is_not_a_cross_reference() {
        let import = parse(&Options::default());
        assert!(
            !messages(&import)
                .iter()
                .any(|m| m.starts_with("RFC-0001 names RFC-0001")),
            "{:?}",
            messages(&import)
        );
    }

    #[test]
    fn an_unknown_status_word_is_kept_and_flagged_rather_than_guessed() {
        let doc = r#"{"phases":[{"id":"1","title":"t","status":"mostly_done"}]}"#;
        let import = RoadmapJson
            .parse("r.json", doc, &Options::default())
            .unwrap();
        assert_eq!(import.tasks[0].status, Status::Todo);
        assert!(
            import.tasks[0]
                .tags
                .iter()
                .any(|t| t == "roadmap-json:mostly_done")
        );
        assert!(
            messages(&import)
                .iter()
                .any(|m| m.contains("does not know")),
            "{:?}",
            messages(&import)
        );
    }

    #[test]
    fn a_file_that_is_not_this_shape_is_refused_rather_than_half_read() {
        let err = RoadmapJson
            .parse("r.json", "{ not json", &Options::default())
            .unwrap_err();
        assert!(matches!(err, ImportError::Malformed { .. }), "{err:?}");
        // Valid JSON with nothing importable is not a successful import of zero.
        let err = RoadmapJson
            .parse("r.json", r#"{"phases":[]}"#, &Options::default())
            .unwrap_err();
        assert!(matches!(err, ImportError::Empty { .. }), "{err:?}");
    }
}
