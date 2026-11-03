//! Kubernetes Enhancement Proposals (`kep.yaml`).
//!
//! The richest machine-readable proposal format in the wild, and the one
//! FIELD-RESEARCH names as the format to emulate: it separates *design
//! accepted* (`implementable`) from *shipped* (`implemented`), which is the
//! claimed-versus-resolved gap `aplan` exists to model.
//!
//! The mapping follows ADR-0012. A KEP's status becomes a `todo`/`dropped`
//! task plus a `kep:<status>` tag, never a new [`Status`] variant; the maturity
//! stage and owning SIG become tags; and every field the task model does not
//! hold is reported rather than dropped in silence.
//!
//! Unmodelled fields are collected by `#[serde(flatten)]` rather than matched
//! against a hand-written list, so a field added upstream after this code was
//! written is still reported. That is the difference between "we drop these
//! eleven fields" and "we drop whatever we cannot hold, and say which".

use std::collections::BTreeMap;

use aplan_core::{Evidence, EvidenceKind, Status, Task};
use serde::Deserialize;

use crate::{
    ImplementedAs, Import, ImportError, Notice, NoticeKind, Options, ProposalSource, Report,
};

/// The `kep.yaml` flavor.
#[derive(Debug, Default, Clone, Copy)]
pub struct Kep;

/// The status values the upstream template documents.
const KNOWN_STATUSES: &[&str] = &[
    "provisional",
    "implementable",
    "implemented",
    "deferred",
    "rejected",
    "withdrawn",
    "replaced",
];

/// The subset of `kep.yaml` the task model can hold. Everything else lands in
/// `extra` and is reported.
///
/// `extra` is a [`BTreeMap`], not a hash map, so the reported field order is
/// sorted and therefore deterministic (HLD section 14).
#[derive(Debug, Deserialize)]
struct KepYaml {
    title: Option<String>,
    #[serde(rename = "kep-number")]
    kep_number: Option<yaml_compat::Value>,
    authors: Option<Vec<String>>,
    #[serde(rename = "owning-sig")]
    owning_sig: Option<String>,
    status: Option<String>,
    stage: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, yaml_compat::Value>,
}

/// Whether a YAML value carries nothing worth reporting as lost.
///
/// A key present but empty (`see-also:` with no entries) lost nothing, so
/// reporting it would be noise that trains the reader to skim the report.
fn is_absent(v: &yaml_compat::Value) -> bool {
    match v {
        yaml_compat::Value::Null => true,
        yaml_compat::Value::String(s) => s.trim().is_empty(),
        yaml_compat::Value::Sequence(s) => s.is_empty(),
        yaml_compat::Value::Mapping(m) => m.is_empty(),
        _ => false,
    }
}

/// Render `kep-number` as it appears in an id, accepting the number the
/// template uses and the string some repositories quote it as.
fn kep_number(v: &yaml_compat::Value) -> Option<String> {
    match v {
        yaml_compat::Value::Number(n) => Some(n.to_string()),
        yaml_compat::Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

/// Trim a field to `Some(non-empty)`.
fn present(s: Option<&String>) -> Option<&str> {
    s.map(|v| v.trim()).filter(|v| !v.is_empty())
}

/// Map a KEP status onto the task model (ADR-0012 clauses 1 and 2), recording
/// why when the answer is not obvious from the value alone.
///
/// An unrecognised value is not fatal: it imports as `todo` and says so,
/// because refusing the whole file would serve nobody.
fn map_status(
    raw: Option<&str>,
    id: &str,
    source_name: &str,
    options: &Options,
    report: &mut Report,
) -> Status {
    let Some(raw) = raw else {
        report.push(Notice::new(
            source_name,
            NoticeKind::Provenance,
            format!("{id}: no `status`, imported as `todo`"),
        ));
        return Status::Todo;
    };

    let lower = raw.to_ascii_lowercase();
    if !KNOWN_STATUSES.contains(&lower.as_str()) {
        report.push(Notice::new(
            source_name,
            NoticeKind::Provenance,
            format!("{id}: unrecognised status `{raw}`, imported as `todo`"),
        ));
    }

    match lower.as_str() {
        // Clause 4: only an explicit opt-in turns a source's claim into `done`.
        "implemented" if matches!(options.implemented_as, ImplementedAs::Done { .. }) => {
            Status::Done
        }
        "rejected" | "withdrawn" | "replaced" => Status::Dropped,
        _ => Status::Todo,
    }
}

/// The stage vocabulary lives in tags, never in the status enum (clause 2).
fn tags_for(raw_status: Option<&str>, doc: &KepYaml) -> Vec<String> {
    let mut tags = Vec::new();
    if let Some(s) = raw_status {
        tags.push(format!("kep:{}", s.to_ascii_lowercase()));
    }
    if let Some(stage) = present(doc.stage.as_ref()) {
        tags.push(format!("kep:stage:{}", stage.to_ascii_lowercase()));
    }
    if let Some(sig) = present(doc.owning_sig.as_ref()) {
        // `sig-node` reads better as `sig:node` beside the other tags.
        let name = sig.strip_prefix("sig-").unwrap_or(sig);
        tags.push(format!("sig:{}", name.to_ascii_lowercase()));
    }
    tags
}

/// What backs an imported `implemented` claim (ADR-0012 clause 4).
///
/// Nothing, by default: the claim is recorded as a `todo` plus a tag, and the
/// report names the command that would prove it. Only the opt-in path attaches
/// evidence, and then only the weakest kind, dated by the caller.
fn evidence_for(
    raw_status: Option<&str>,
    id: &str,
    source_name: &str,
    options: &Options,
    report: &mut Report,
) -> Vec<Evidence> {
    if raw_status.map(str::to_ascii_lowercase).as_deref() != Some("implemented") {
        return Vec::new();
    }

    match &options.implemented_as {
        ImplementedAs::Done { as_of } => {
            report.push(Notice::new(
                source_name,
                NoticeKind::Provenance,
                format!(
                    "{id} imported `done` on note-level provenance only; attach real proof \
                     with `aplan close {id} --evidence commit:COMMIT_SHA`"
                ),
            ));
            vec![Evidence {
                kind: EvidenceKind::Note,
                reference: format!(
                    "imported from {source_name}: kep.yaml declared status `implemented`"
                ),
                captured: Some(as_of.clone()),
            }]
        }
        ImplementedAs::Todo => {
            report.push(Notice::new(
                source_name,
                NoticeKind::Provenance,
                format!(
                    "{id} claims `implemented`; imported as `todo` (the claim is not proof). \
                     Close it with `aplan close {id} --evidence commit:COMMIT_SHA`"
                ),
            ));
            Vec::new()
        }
    }
}

/// Name everything that did not survive the conversion (clause 3).
fn report_losses(doc: &KepYaml, source_name: &str, report: &mut Report) {
    for (field, value) in &doc.extra {
        if !is_absent(value) {
            report.dropped(source_name, field);
        }
    }
    if doc.authors.as_ref().is_some_and(|a| a.len() > 1) {
        report.push(Notice::new(
            source_name,
            NoticeKind::DroppedField,
            "`authors` lists more than one; the task model holds a single `owner`, \
             so only the first was kept",
        ));
    }
}

impl ProposalSource for Kep {
    fn flavor(&self) -> &'static str {
        "kep"
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

        let doc: KepYaml = yaml_compat::from_str(contents).map_err(|e| ImportError::Malformed {
            source_name: source_name.to_string(),
            message: e.to_string(),
        })?;

        let malformed = |message: &str| ImportError::Malformed {
            source_name: source_name.to_string(),
            message: message.to_string(),
        };

        let number = doc
            .kep_number
            .as_ref()
            .and_then(kep_number)
            .ok_or_else(|| malformed("`kep-number` is missing or not a number"))?;
        let id = format!("KEP-{number}");
        let title = present(doc.title.as_ref())
            .ok_or_else(|| malformed("`title` is missing"))?
            .to_string();

        let mut report = Report::new();
        let raw_status = present(doc.status.as_ref());

        let status = map_status(raw_status, &id, source_name, options, &mut report);
        let tags = tags_for(raw_status, &doc);
        let evidence = evidence_for(raw_status, &id, source_name, options, &mut report);
        report_losses(&doc, source_name, &mut report);

        let owner = doc
            .authors
            .as_ref()
            .and_then(|a| a.first())
            .map(|a| a.trim().trim_start_matches('@').to_string())
            .filter(|a| !a.is_empty());

        let task = Task {
            id,
            title,
            phase: options.phase,
            status,
            owner,
            effort_days: None,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence,
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: BTreeMap::new(),
            evidence_fingerprints: std::collections::BTreeMap::new(),
            tags,
            // An importer never invents one: a revisit is a decision about work
            // somebody parked, and no source format carries it.
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        };

        Ok(Import {
            bodies: vec![crate::emit::provenance_body(source_name, self.flavor())],
            tasks: vec![task],
            report,
        })
    }
}
