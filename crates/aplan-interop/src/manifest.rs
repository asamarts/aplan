//! A list of tasks to add as one change (`A1-241`).
//!
//! Two pilots arrived with a written plan and no way to get it in. `p00016`
//! turned a 1,400-line research document into 153 tasks, 11 cuts and 3 releases
//! in one pass, and `p00002` into 64 tasks with cuts, effort, two tag families,
//! paths, traces and blockers. Both wrote a script, because `aplan add` is one
//! task per invocation.
//!
//! `p00002`'s script shelled out 64 times and every task landed correctly.
//! `p00016`'s emitted frontmatter directly, which is the outcome worth
//! preventing: the plan was then authored by a program that knows the file
//! format, which is the coupling `import` exists to avoid for *other* tools'
//! formats, appearing for the case where the source is the author's own document
//! (pre-public adoption evidence item 3, pre-public adoption evidence item 2).
//!
//! **This is not an importer.** ADR-0012's honesty rules are about a format the
//! task model cannot hold, and there is no such loss here: an entry is the set
//! of arguments `aplan add` already takes, so the fields are the model's own.
//! What it removes is the invocation count and the second implementation of the
//! frontmatter contract, not a translation.

use aplan_core::task::VersionImpact;
use aplan_core::{Evidence, Status};
use serde::Deserialize;

/// Every accepted manifest key, audited against [`Entry`]'s generated schema.
///
/// This is documentation data rather than a second parser: the schema test
/// below refuses either a field added without a contract decision or a stale
/// name retained after the type changed.
pub const ENTRY_FIELDS: &[&str] = &[
    "title",
    "id",
    "phase",
    "cut",
    "effort",
    "owner",
    "status",
    "tags",
    "paths",
    "traces",
    "blocked_by",
    "blocks",
    "prefer_after",
    "ratified_by",
    "contains",
    "revisit_at",
    "version_impact",
    "version_impact_reason",
    "evidence",
    "body",
];

/// One task to add: what `aplan add` takes, and the body it cannot.
///
/// `deny_unknown_fields` for the reason `.aplan.yml` has it: a misspelled
/// `efort` that is silently ignored writes 153 tasks with no estimates and says
/// nothing, and the author finds out from `APLAN009` much later.
#[derive(Debug, Clone, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Title, as a person would read it. The one required field.
    pub title: String,
    /// Task id. Minted from the plan's convention when absent, exactly as
    /// `aplan add` mints it.
    #[serde(default)]
    pub id: Option<String>,
    /// `phase`. Defaults to the cut's, then to the highest in use.
    #[serde(default)]
    pub phase: Option<u32>,
    /// Milestone to join, by id or by the label the cut reports itself under.
    #[serde(default)]
    pub cut: Option<String>,
    /// Estimate in days. Refused when the entry is tagged `milestone`, because
    /// a checkpoint carries no effort.
    #[serde(default)]
    pub effort: Option<f64>,
    /// Who owns it.
    #[serde(default)]
    pub owner: Option<String>,
    /// Initial lifecycle state. Ordinary creation accepts only live states;
    /// `done` is reserved for the evidence-resolving ledger-correction path.
    /// `dropped` is never a creation state because it records a later decision
    /// not to do work that already existed.
    #[serde(default)]
    pub status: Option<Status>,
    /// Tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Globs this task is responsible for. Refused when the entry is tagged
    /// `milestone`, because a checkpoint owns no files.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Document sections this task answers to.
    #[serde(default)]
    pub traces: Vec<String>,
    /// Tasks this one waits on. May name a task the plan already has, or one any
    /// entry in the same file declares an `id` for -- including a later one, so
    /// a milestone can be written above the work it lists.
    #[serde(default)]
    pub blocked_by: Vec<String>,
    /// Tasks which wait on this one. This reverse spelling is useful when a
    /// creation batch needs to gate work that already exists; the target may
    /// also be another explicitly-named entry in the same atomic manifest.
    #[serde(default)]
    pub blocks: Vec<String>,
    /// Non-blocking ordering preferences. Like `blocked_by`, these may name an
    /// id declared later in the same atomic manifest.
    #[serde(default)]
    pub prefer_after: Vec<String>,
    /// Independent review tasks that must be complete before this task closes.
    #[serde(default)]
    pub ratified_by: Vec<String>,
    /// Cuts or tasks whose work this checkpoint borrows for rollups.
    #[serde(default)]
    pub contains: Vec<String>,
    /// RFC 3339 instant at which an initially parked task is worth reviewing.
    #[serde(default)]
    pub revisit_at: Option<String>,
    /// Effect on the released artifact.
    #[serde(default)]
    pub version_impact: Option<VersionImpact>,
    /// Required explanation when an impact is below the task's level floor.
    #[serde(default)]
    pub version_impact_reason: Option<String>,
    /// Proof for a `done` ledger-correction entry. Refused on live entries and
    /// resolved for the complete batch before any task or cut is written.
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    /// The task's body: everything after the frontmatter, as Markdown.
    ///
    /// Absent keeps the scaffold that `aplan add` creates. Present replaces the scaffold whole;
    /// the value is final task content rather than a template to merge.
    // The rationale is measured in pre-public adoption evidence item 2 and A1-260. Keep that
    // repository-local evidence out of the public JSON Schema description derived from this field.
    #[serde(default)]
    pub body: Option<String>,
}

/// What went wrong reading a manifest.
#[derive(Debug)]
pub enum ManifestError {
    /// The file could not be read.
    Io {
        /// The file.
        path: std::path::PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
    /// The file is not a list of entries.
    Parse {
        /// The file.
        path: std::path::PathBuf,
        /// What the parser said.
        said: String,
    },
    /// The file parsed and named no tasks.
    Empty {
        /// The file.
        path: std::path::PathBuf,
    },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "reading {}: {source}", path.display()),
            Self::Parse { path, said } => write!(
                f,
                "{} is not a list of tasks: {said}. Each entry is a mapping with a `title` \
                 and any of the fields `aplan add` takes as flags",
                path.display()
            ),
            Self::Empty { path } => write!(
                f,
                "{} names no tasks, so there is nothing to add",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { .. } | Self::Empty { .. } => None,
        }
    }
}

/// Read a manifest: a YAML list of [`Entry`].
///
/// # Errors
///
/// [`ManifestError`] when the file cannot be read, is not a list of entries, or
/// holds none.
pub fn read(path: &std::path::Path) -> Result<Vec<Entry>, ManifestError> {
    let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse(path, &text)
}

/// Parse a manifest from bytes already captured by a delivery adapter.
///
/// Keeping parsing separate from filesystem access lets an interactive client
/// preview exactly the text its file picker read, rather than naming a path and
/// silently re-reading newer bytes while it builds the guarded change.
///
/// # Errors
///
/// [`ManifestError::Parse`] when `text` is not a list of entries, or
/// [`ManifestError::Empty`] when the list contains no tasks.
pub fn parse(path: &std::path::Path, text: &str) -> Result<Vec<Entry>, ManifestError> {
    let entries: Vec<Entry> = yaml_compat::from_str(text).map_err(|e| ManifestError::Parse {
        path: path.to_path_buf(),
        said: e.to_string(),
    })?;
    if entries.is_empty() {
        return Err(ManifestError::Empty {
            path: path.to_path_buf(),
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::{ENTRY_FIELDS, Entry, ManifestError, parse, read};
    use aplan_core::Status;
    use aplan_core::task::VersionImpact;

    fn at(dir: &std::path::Path, text: &str) -> std::path::PathBuf {
        let p = dir.join("m.yml");
        std::fs::write(&p, text).expect("write");
        p
    }

    #[test]
    fn creation_field_contract_enumerates_the_manifest_type_system_surface() {
        let schema = serde_json::to_value(schemars::schema_for!(Entry)).expect("schema");
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("entry properties");
        let actual = properties
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        let declared = ENTRY_FIELDS
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, declared);
    }

    /// An entry carries the fields `add` takes, and no others.
    #[test]
    fn an_entry_reads_the_fields_add_takes() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let p = at(
            dir.path(),
            "- title: A first task\n  cut: M-s0\n  effort: 2.5\n  tags: [kind-build]\n  \
             paths: ['src/**']\n  blocked_by: [A-1]\n- title: A second\n",
        );
        let got = read(&p).expect("reads");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].title, "A first task");
        assert_eq!(got[0].cut.as_deref(), Some("M-s0"));
        assert_eq!(got[0].effort, Some(2.5));
        assert_eq!(got[0].blocked_by, vec!["A-1".to_string()]);
        assert_eq!(
            got[1],
            Entry {
                title: "A second".into(),
                id: None,
                phase: None,
                cut: None,
                effort: None,
                owner: None,
                status: None,
                tags: Vec::new(),
                paths: Vec::new(),
                traces: Vec::new(),
                blocked_by: Vec::new(),
                blocks: Vec::new(),
                prefer_after: Vec::new(),
                ratified_by: Vec::new(),
                contains: Vec::new(),
                revisit_at: None,
                version_impact: None,
                version_impact_reason: None,
                evidence: Vec::new(),
                body: None,
            },
            "everything but the title is optional"
        );
    }

    #[test]
    fn an_entry_carries_every_creation_time_relationship_and_live_state() {
        let entry = parse(
            std::path::Path::new("complete.yml"),
            concat!(
                "- title: Parked release checkpoint\n",
                "  id: M-release\n",
                "  status: blocked\n",
                "  prefer_after: [A-later]\n",
                "  ratified_by: [A-review]\n",
                "  contains: [M-cut]\n",
                "  revisit_at: 2026-12-01T00:00:00Z\n",
                "  version_impact: feature\n",
                "  version_impact_reason: Portfolio checkpoint\n",
                "- title: Later\n",
                "  id: A-later\n",
                "- title: Review\n",
                "  id: A-review\n",
                "- title: Cut\n",
                "  id: M-cut\n",
            ),
        )
        .expect("the manifest shape owns creation-time model fields");
        assert_eq!(entry[0].status, Some(Status::Blocked));
        assert_eq!(entry[0].prefer_after, ["A-later"]);
        assert_eq!(entry[0].ratified_by, ["A-review"]);
        assert_eq!(entry[0].contains, ["M-cut"]);
        assert_eq!(entry[0].revisit_at.as_deref(), Some("2026-12-01T00:00:00Z"));
        assert_eq!(entry[0].version_impact, Some(VersionImpact::Feature));
        assert_eq!(
            entry[0].version_impact_reason.as_deref(),
            Some("Portfolio checkpoint")
        );
    }

    #[test]
    fn captured_manifest_text_parses_without_reopening_its_label() {
        let label = std::path::Path::new("not-on-disk.yml");
        let entries = parse(label, "- title: Captured\n").expect("captured text parses");
        assert_eq!(entries[0].title, "Captured");
        assert!(matches!(parse(label, "[]"), Err(ManifestError::Empty { path }) if path == label));
    }

    /// The `A1-383` execution spike applied these three shapes through
    /// `add --from`. Keep the parser boundary explicit: both one-task shapes
    /// already fit, while a reusable graph must declare ids and a placeholder
    /// remains ordinary text. Parameters and local keys therefore belong to a
    /// distinct parsed blueprint document rather than being implied by a
    /// string convention in this manifest type.
    #[test]
    fn the_blueprint_spike_pins_the_plain_manifest_boundary() {
        let pilot = parse(
            std::path::Path::new("pilot.yml"),
            concat!(
                "- title: Pilot aplan in widget and record adoption friction\n",
                "  cut: M-spike\n",
                "  effort: 2\n",
                "  tags: [adoption, pilot]\n",
                "  paths: [docs/pilots/widget.md, 'aplan/**', .aplan.yml]\n",
                "  body: |\n",
                "    ## Why\n",
                "\n",
                "    Adopt aplan in `widget`.\n",
            ),
        )
        .expect("pilot is already an ordinary manifest");
        assert_eq!(pilot.len(), 1);
        assert_eq!(pilot[0].cut.as_deref(), Some("M-spike"));
        assert_eq!(pilot[0].paths.len(), 3);
        assert!(pilot[0].body.is_some());

        let rule = parse(
            std::path::Path::new("rule.yml"),
            concat!(
                "- title: Report live tasks with blank owners\n",
                "  cut: M-spike\n",
                "  effort: 1\n",
                "  tags: [rules]\n",
                "  paths: [crates/aplan-core/src/diagnostic.rs, docs/rule-catalogue.md, CHANGELOG.md]\n",
                "  body: |\n",
                "    ## Why\n",
                "\n",
                "    `APLAN999` reports a live task whose owner is blank.\n",
            ),
        )
        .expect("rule is already an ordinary manifest");
        assert_eq!(rule.len(), 1);
        assert_eq!(rule[0].tags, vec!["rules".to_string()]);
        assert_eq!(rule[0].paths.len(), 3);

        let feature = parse(
            std::path::Path::new("feature.yml"),
            concat!(
                "- id: S-100\n",
                "  title: Decide bulk archive semantics\n",
                "  cut: M-spike\n",
                "  effort: 1\n",
                "- id: S-101\n",
                "  title: Implement bulk archive\n",
                "  cut: M-spike\n",
                "  effort: 3\n",
                "  blocked_by: [S-100]\n",
                "- id: S-102\n",
                "  title: Prove and document bulk archive\n",
                "  cut: M-spike\n",
                "  effort: 1\n",
                "  blocked_by: [S-101]\n",
            ),
        )
        .expect("feature works only after choosing ids");
        assert_eq!(feature.len(), 3);
        assert_eq!(feature[1].blocked_by, vec!["S-100".to_string()]);
        assert_eq!(feature[2].blocked_by, vec!["S-101".to_string()]);

        let literal = parse(
            std::path::Path::new("literal.yml"),
            "- title: 'Pilot aplan in {{repository}}'\n  effort: 1\n",
        )
        .expect("placeholder syntax has no manifest semantics");
        assert_eq!(literal[0].title, "Pilot aplan in {{repository}}");

        let wrapper = parse(
            std::path::Path::new("blueprint.yml"),
            "parameters: [{ name: feature }]\ntasks: [{ title: Build it }]\n",
        )
        .expect_err("a blueprint wrapper is not a manifest");
        assert!(wrapper.to_string().contains("not a list of tasks"));

        let keyed = parse(
            std::path::Path::new("keyed.yml"),
            "- key: design\n  title: Decide it\n",
        )
        .expect_err("a local key is not a manifest field");
        assert!(keyed.to_string().contains("unknown field `key`"));

        let local_reference = parse(
            std::path::Path::new("local-reference.yml"),
            "- title: Implement it\n  blocked_by: [local:design]\n",
        )
        .expect("the manifest parser preserves rather than resolves ids");
        assert_eq!(
            local_reference[0].blocked_by,
            vec!["local:design".to_string()]
        );
    }

    /// The body reaches the entry as written, including the newlines a block
    /// scalar preserves: it becomes the file's Markdown byte for byte, so a
    /// parser that trimmed or rewrapped it would change what an author wrote
    /// (`A1-260`).
    #[test]
    fn an_entry_carries_the_body_verbatim() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        // Concatenated rather than continued: a `\` at the end of a Rust
        // string line eats the leading whitespace of the next, which in YAML is
        // the indentation that makes a block scalar (`A1-274` was found the same
        // way, and this test was written wrong once for the same reason).
        let p = at(
            dir.path(),
            concat!(
                "- title: With a body\n",
                "  body: |\n",
                "    ## Why\n",
                "\n",
                "    Two paragraphs, and\n",
                "\n",
                "    - [ ] one criterion\n",
                "- title: Without one\n",
            ),
        );
        let got = read(&p).expect("reads");
        assert_eq!(
            got[0].body.as_deref(),
            Some("## Why\n\nTwo paragraphs, and\n\n- [ ] one criterion\n")
        );
        assert_eq!(got[1].body, None, "absent is absent, not empty");
    }

    /// **A misspelled field is an error, not a silent omission.**
    ///
    /// The same stance `.aplan.yml` takes, for the same reason at a hundred and
    /// fifty times the scale: `efort` accepted quietly writes a plan with no
    /// estimates and says nothing until `APLAN009` does, long afterwards.
    #[test]
    fn a_misspelled_field_is_refused_by_name() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "- title: A task\n  efort: 2\n");
        let err = read(&p).expect_err("unknown field");
        let said = err.to_string();
        assert!(
            said.contains("efort"),
            "the name is what the author fixes: {said}"
        );
        assert!(
            said.contains("aplan add"),
            "and the message says where the field list comes from: {said}"
        );
    }

    /// A file that parses and holds nothing is refused, rather than reported as
    /// a successful run that added no tasks.
    #[test]
    fn an_empty_manifest_is_refused_rather_than_reported_as_done() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "[]\n");
        assert!(matches!(
            read(&p).expect_err("empty"),
            ManifestError::Empty { .. }
        ));
    }

    /// A mapping where a list belongs says so, naming the shape it wanted.
    #[test]
    fn a_document_that_is_not_a_list_says_what_one_looks_like() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "tasks:\n  - title: A task\n");
        let said = read(&p).expect_err("not a list").to_string();
        assert!(said.contains("is not a list of tasks"), "{said}");
    }

    /// A file that is not there names itself, and carries the cause for a
    /// caller that wants it.
    #[test]
    fn a_missing_manifest_names_the_file_and_keeps_the_cause() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let missing = dir.path().join("nope.yml");
        let err = read(&missing).expect_err("not there");
        assert!(matches!(err, ManifestError::Io { .. }), "{err:?}");
        let said = err.to_string();
        assert!(said.contains("nope.yml"), "{said}");
        assert!(
            std::error::Error::source(&err).is_some(),
            "the filesystem's own error is kept: {said}"
        );
    }

    /// The two errors that are their own explanation carry no source, so a
    /// caller printing the chain does not print the sentence twice.
    #[test]
    fn a_parse_or_empty_error_is_its_own_explanation() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        for text in ["[]\n", "- title: A task\n  efort: 2\n"] {
            let p = at(dir.path(), text);
            let err = read(&p).expect_err("refused");
            assert!(
                std::error::Error::source(&err).is_none(),
                "{}",
                err.to_string()
            );
            assert!(!err.to_string().is_empty());
        }
    }
}
