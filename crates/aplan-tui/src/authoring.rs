//! Guided structural-authoring forms and external-editor handoff.
//!
//! These definitions describe the interactive contract. Domain validation and
//! exact mutation previews live in [`aplan_app::authoring`]; this module does
//! not duplicate plan rules or parse rendered command output.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use aplan_app::PlanContext;

use crate::form::{DefinitionError, FieldSpec, FormSubmission, MutationForm};

/// Structural authoring workflows delivered by this cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthoringWorkflow {
    Add,
    Manifest,
    Import,
    Supersede,
    Rekey,
}

impl AuthoringWorkflow {
    /// Exhaustive inventory used by the form parity test.
    pub const ALL: &'static [Self] = &[
        Self::Add,
        Self::Manifest,
        Self::Import,
        Self::Supersede,
        Self::Rekey,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Add => "Add task",
            Self::Manifest => "Add tasks from manifest",
            Self::Import => "Import proposal tasks",
            Self::Supersede => "Supersede completed outcome",
            Self::Rekey => "Rekey task",
        }
    }

    pub const fn destructive(self) -> bool {
        matches!(self, Self::Supersede | Self::Rekey)
    }

    /// Explicit command boundaries displayed beside the native workflow.
    pub const fn cli_only_help(self) -> &'static [&'static str] {
        match self {
            Self::Add => &["Writing outside this plan stays CLI-only: aplan add TITLE --out DIR"],
            Self::Manifest => {
                &["Writing outside this plan stays CLI-only: aplan add --from FILE --out DIR"]
            }
            Self::Import => &[
                "Writing outside this plan stays CLI-only: aplan import FILE --format FORMAT --out DIR",
            ],
            Self::Supersede => &[],
            Self::Rekey => &["Machine output stays CLI-only: aplan rekey OLD NEW --format json"],
        }
    }
}

/// Construct one workflow's reusable form for the current immutable snapshot.
pub fn form(
    context: &PlanContext,
    workflow: AuthoringWorkflow,
    selected_task: Option<&str>,
) -> Result<MutationForm, DefinitionError> {
    MutationForm::new(
        workflow.title(),
        &context.identity().sha256,
        workflow.destructive(),
        fields(workflow, selected_task),
    )
}

fn fields(workflow: AuthoringWorkflow, selected_task: Option<&str>) -> Vec<FieldSpec> {
    match workflow {
        AuthoringWorkflow::Add => add_fields(),
        AuthoringWorkflow::Manifest => manifest_fields(),
        AuthoringWorkflow::Import => import_fields(),
        AuthoringWorkflow::Supersede => supersede_fields(selected_task),
        AuthoringWorkflow::Rekey => vec![
            selected("old", "Current ID", "Stable id to migrate", selected_task).required(),
            FieldSpec::new("new", "New ID", "New stable id and conventional file stem").required(),
            FieldSpec::new(
                "allow_terminal",
                "Allow terminal",
                "Explicitly type yes to migrate done or dropped history",
            ),
        ],
    }
}

/// Shared task-creation vocabulary. Capture promotion selects the admitted
/// placement fields from this exact list rather than maintaining a second
/// spelling or help contract.
pub(crate) fn add_fields() -> Vec<FieldSpec> {
    vec![
        FieldSpec::new("title", "Title", "Required one-line outcome").required(),
        FieldSpec::new("id", "ID", "Blank mints from the plan convention"),
        FieldSpec::new("phase", "Phase", "Blank derives from the selected cut"),
        FieldSpec::new("cut", "Cut", "Milestone id or displayed label"),
        FieldSpec::new("effort", "Effort days", "Finite non-negative estimate"),
        FieldSpec::new("owner", "Owner", "Optional canonical owner"),
        FieldSpec::new(
            "status",
            "Status",
            "todo (default), in_progress, or blocked; terminal history uses a manifest ledger",
        ),
        FieldSpec::new("blocked_by", "Blocked by", "Required predecessor ids").repeated(),
        FieldSpec::new("prefer_after", "Prefer after", "Advisory predecessor ids").repeated(),
        FieldSpec::new("ratified_by", "Ratified by", "Decision-record task ids").repeated(),
        FieldSpec::new("contains", "Contains", "Release/container member ids").repeated(),
        FieldSpec::new(
            "members",
            "Move members",
            "Existing live task ids moved into a new milestone",
        )
        .repeated(),
        FieldSpec::new("tags", "Tags", "Task classification").repeated(),
        FieldSpec::new("paths", "Paths", "Owned repository globs").repeated(),
        FieldSpec::new("traces", "Traces", "Document anchors this task resolves").repeated(),
        FieldSpec::new(
            "revisit_at",
            "Revisit at",
            "RFC 3339 instant for an initially blocked task",
        ),
        FieldSpec::new(
            "version_impact",
            "Version impact",
            "none, fix, feature, or breaking",
        ),
        FieldSpec::new(
            "version_impact_reason",
            "Version reason",
            "Why the declared version impact applies",
        ),
        FieldSpec::new(
            "body",
            "Body",
            "Optional Markdown; use the external-editor handoff for a long body",
        ),
    ]
}

fn manifest_fields() -> Vec<FieldSpec> {
    vec![
        FieldSpec::new(
            "source",
            "Manifest",
            "YAML task list; its captured bytes are previewed as one batch",
        )
        .required(),
    ]
}

fn import_fields() -> Vec<FieldSpec> {
    vec![
        FieldSpec::new("source", "Source", "Proposal document to capture").required(),
        FieldSpec::new(
            "format",
            "Format",
            "kep, checklist, roadmap-json, spec-kit, or openspec",
        )
        .required()
        .initial_value("kep"),
        FieldSpec::new("phase", "Phase", "Destination reporting coordinate")
            .required()
            .initial_value("1"),
        FieldSpec::new(
            "implemented_as",
            "Implemented as",
            "todo (default) or done with an explicit as-of date",
        )
        .required()
        .initial_value("todo"),
        FieldSpec::new("as_of", "As of", "YYYY-MM-DD; only for implemented-as done"),
        FieldSpec::new("cut", "Cut", "Milestone id or displayed label"),
        FieldSpec::new(
            "id_prefix",
            "ID prefix",
            "Meaning depends on the source format",
        ),
        FieldSpec::new("tags", "Tags", "Classification applied to every task").repeated(),
    ]
}

fn supersede_fields(selected_task: Option<&str>) -> Vec<FieldSpec> {
    vec![
        selected(
            "predecessor",
            "Predecessor",
            "Completed task id",
            selected_task,
        )
        .required(),
        FieldSpec::new("title", "Successor title", "What now needs doing").required(),
        FieldSpec::new("cut", "Cut", "Optional successor placement"),
        FieldSpec::new("effort", "Effort days", "Finite non-negative estimate"),
        FieldSpec::new("owner", "Owner", "Blank inherits the predecessor owner"),
        FieldSpec::new(
            "tags",
            "Tags",
            "Blank inherits subject tags but never structural tags",
        )
        .repeated(),
        FieldSpec::new(
            "paths",
            "Replacement paths",
            "Omit to inherit; values replace the predecessor paths",
        )
        .repeated(),
        FieldSpec::new(
            "clear_paths",
            "Clear paths",
            "Type yes to inherit no paths; conflicts with replacement values",
        ),
        FieldSpec::new(
            "traces",
            "Replacement traces",
            "Omit to inherit; values replace the predecessor traces",
        )
        .repeated(),
        FieldSpec::new(
            "clear_traces",
            "Clear traces",
            "Type yes to inherit no traces; conflicts with replacement values",
        ),
        FieldSpec::new("body", "Successor body", "Omit for the generated template"),
    ]
}

fn selected(key: &str, label: &str, help: &str, selected: Option<&str>) -> FieldSpec {
    selected.map_or_else(
        || FieldSpec::new(key, label, help),
        |value| FieldSpec::new(key, label, help).initial_value(value),
    )
}

/// Exact non-form input captured before a preview request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthoringAssets {
    /// Source document selected by manifest/import forms.
    pub source: Option<CapturedSource>,
    /// Long Markdown returned by [`ExternalEditorHandoff`]. It takes precedence
    /// over the form's short body field.
    pub edited_body: Option<String>,
}

/// One source picker result, including the bytes that were actually read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedSource {
    pub name: String,
    pub contents: String,
}

/// Typed application request decoded from an authoring form.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthoringRequest {
    Add(Box<aplan_app::AddRequest>),
    Manifest(aplan_app::ManifestRequest),
    Import(aplan_app::ImportRequest),
    Supersede(aplan_app::SupersedeRequest),
    Rekey(aplan_app::RekeyRequest),
}

impl AuthoringRequest {
    /// Run the matching application preview without writing.
    pub fn preview(
        &self,
        context: &PlanContext,
    ) -> Result<aplan_app::AuthoringPreview, aplan_app::AuthoringError> {
        match self {
            Self::Add(request) => aplan_app::preview_add(context, request),
            Self::Manifest(request) => aplan_app::preview_manifest(context, request),
            Self::Import(request) => aplan_app::preview_import(context, request),
            Self::Supersede(request) => aplan_app::preview_supersede(context, request),
            Self::Rekey(request) => aplan_app::preview_rekey(context, request),
        }
    }
}

/// Decode one complete form submission into the typed application operation.
pub fn request(
    workflow: AuthoringWorkflow,
    submission: &FormSubmission,
    assets: AuthoringAssets,
) -> Result<AuthoringRequest, InputError> {
    match workflow {
        AuthoringWorkflow::Add => decode_add(submission, assets),
        AuthoringWorkflow::Manifest => {
            let source = captured_source(submission, assets.source)?;
            Ok(AuthoringRequest::Manifest(aplan_app::ManifestRequest {
                source_name: source.name,
                contents: source.contents,
            }))
        }
        AuthoringWorkflow::Import => decode_import(submission, assets.source),
        AuthoringWorkflow::Supersede => {
            let paths = replacement_or_clear(submission, "paths", "clear_paths")?;
            let traces = replacement_or_clear(submission, "traces", "clear_traces")?;
            Ok(AuthoringRequest::Supersede(aplan_app::SupersedeRequest {
                predecessor: required(submission, "predecessor")?,
                title: required(submission, "title")?,
                cut: optional(submission, "cut"),
                effort_days: optional_number(submission, "effort")?,
                owner: optional(submission, "owner"),
                tags: repeated(submission, "tags"),
                paths,
                traces,
                body: assets.edited_body.or_else(|| optional(submission, "body")),
            }))
        }
        AuthoringWorkflow::Rekey => Ok(AuthoringRequest::Rekey(aplan_app::RekeyRequest {
            old: required(submission, "old")?,
            new: required(submission, "new")?,
            allow_terminal: yes_no(submission, "allow_terminal")?,
        })),
    }
}

fn decode_add(
    submission: &FormSubmission,
    assets: AuthoringAssets,
) -> Result<AuthoringRequest, InputError> {
    let body = assets.edited_body.or_else(|| optional(submission, "body"));
    Ok(AuthoringRequest::Add(Box::new(aplan_app::AddRequest {
        title: required(submission, "title")?,
        id: optional(submission, "id"),
        phase: optional_number(submission, "phase")?,
        cut: optional(submission, "cut"),
        effort_days: optional_number(submission, "effort")?,
        owner: optional(submission, "owner"),
        status: creation_status(submission)?,
        blocked_by: repeated(submission, "blocked_by"),
        prefer_after: repeated(submission, "prefer_after"),
        ratified_by: repeated(submission, "ratified_by"),
        contains: repeated(submission, "contains"),
        members: repeated(submission, "members"),
        tags: repeated(submission, "tags"),
        paths: repeated(submission, "paths"),
        traces: repeated(submission, "traces"),
        revisit_at: optional(submission, "revisit_at"),
        version_impact: creation_version_impact(submission)?,
        version_impact_reason: optional(submission, "version_impact_reason"),
        evidence: Vec::new(),
        body,
    })))
}

fn decode_import(
    submission: &FormSubmission,
    source: Option<CapturedSource>,
) -> Result<AuthoringRequest, InputError> {
    let source = captured_source(submission, source)?;
    let implemented = required(submission, "implemented_as")?;
    let implemented_as = match implemented.as_str() {
        "todo" => aplan_app::ImplementedAs::Todo,
        "done" => aplan_app::ImplementedAs::Done {
            as_of: required(submission, "as_of")?,
        },
        _ => {
            return Err(InputError::Invalid {
                field: "implemented_as",
                message: "expected `todo` or `done`".to_string(),
            });
        }
    };
    Ok(AuthoringRequest::Import(aplan_app::ImportRequest {
        source_name: source.name,
        contents: source.contents,
        format: required(submission, "format")?,
        phase: required_number(submission, "phase")?,
        implemented_as,
        cut: optional(submission, "cut"),
        id_prefix: optional(submission, "id_prefix"),
        tags: repeated(submission, "tags"),
    }))
}

fn captured_source(
    submission: &FormSubmission,
    captured: Option<CapturedSource>,
) -> Result<CapturedSource, InputError> {
    let selected = required(submission, "source")?;
    let captured = captured.ok_or(InputError::MissingSource)?;
    if captured.name != selected {
        return Err(InputError::SourceMoved {
            selected,
            captured: captured.name,
        });
    }
    Ok(captured)
}

fn required(submission: &FormSubmission, field: &'static str) -> Result<String, InputError> {
    optional(submission, field).ok_or(InputError::Required(field))
}

fn optional(submission: &FormSubmission, field: &str) -> Option<String> {
    submission
        .value(field)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn repeated(submission: &FormSubmission, field: &str) -> Vec<String> {
    submission
        .values(field)
        .into_iter()
        .flatten()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn replacement_or_clear(
    submission: &FormSubmission,
    values_field: &'static str,
    clear_field: &'static str,
) -> Result<Option<Vec<String>>, InputError> {
    let values = repeated(submission, values_field);
    let clear = yes_no(submission, clear_field)?;
    if clear && !values.is_empty() {
        return Err(InputError::Invalid {
            field: clear_field,
            message: format!("cannot be `yes` when `{values_field}` has replacement values"),
        });
    }
    Ok(if clear {
        Some(Vec::new())
    } else if values.is_empty() {
        None
    } else {
        Some(values)
    })
}

fn creation_status(submission: &FormSubmission) -> Result<Option<aplan_core::Status>, InputError> {
    optional(submission, "status")
        .map(|value| match value.as_str() {
            "todo" => Ok(aplan_core::Status::Todo),
            "in_progress" => Ok(aplan_core::Status::InProgress),
            "blocked" => Ok(aplan_core::Status::Blocked),
            "done" | "dropped" => Err(InputError::Invalid {
                field: "status",
                message: "terminal history must use the evidence-resolving manifest ledger"
                    .to_string(),
            }),
            _ => Err(InputError::Invalid {
                field: "status",
                message: "expected `todo`, `in_progress`, or `blocked`".to_string(),
            }),
        })
        .transpose()
}

fn creation_version_impact(
    submission: &FormSubmission,
) -> Result<Option<aplan_core::task::VersionImpact>, InputError> {
    optional(submission, "version_impact")
        .map(|value| match value.as_str() {
            "none" => Ok(aplan_core::task::VersionImpact::None),
            "fix" => Ok(aplan_core::task::VersionImpact::Fix),
            "feature" => Ok(aplan_core::task::VersionImpact::Feature),
            "breaking" => Ok(aplan_core::task::VersionImpact::Breaking),
            _ => Err(InputError::Invalid {
                field: "version_impact",
                message: "expected `none`, `fix`, `feature`, or `breaking`".to_string(),
            }),
        })
        .transpose()
}

fn required_number<T>(submission: &FormSubmission, field: &'static str) -> Result<T, InputError>
where
    T: std::str::FromStr,
{
    required(submission, field)?
        .parse()
        .map_err(|_| InputError::Invalid {
            field,
            message: "not a valid number".to_string(),
        })
}

fn optional_number<T>(
    submission: &FormSubmission,
    field: &'static str,
) -> Result<Option<T>, InputError>
where
    T: std::str::FromStr,
{
    optional(submission, field)
        .map(|value| {
            value.parse().map_err(|_| InputError::Invalid {
                field,
                message: "not a valid number".to_string(),
            })
        })
        .transpose()
}

fn yes_no(submission: &FormSubmission, field: &'static str) -> Result<bool, InputError> {
    match optional(submission, field).as_deref() {
        None | Some("no") => Ok(false),
        Some("yes") => Ok(true),
        Some(_) => Err(InputError::Invalid {
            field,
            message: "expected `yes` or `no`".to_string(),
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
    #[error("field `{0}` is required")]
    Required(&'static str),
    #[error("field `{field}` is invalid: {message}")]
    Invalid {
        field: &'static str,
        message: String,
    },
    #[error("the form source has not been captured")]
    MissingSource,
    #[error("the selected source `{selected}` does not match captured source `{captured}`")]
    SourceMoved { selected: String, captured: String },
}

const MAX_BODY_BYTES: u64 = 4 * 1024 * 1024;
static NEXT_DRAFT: AtomicU64 = AtomicU64::new(1);

/// A private draft file that the terminal host can hand to an external editor.
///
/// Preparing and completing are separate so the shell can leave raw mode and
/// the alternate screen before it launches the user's configured program. This
/// type never invokes a shell and never interprets editor configuration.
#[derive(Debug)]
pub struct ExternalEditorHandoff {
    path: PathBuf,
    complete: bool,
}

impl ExternalEditorHandoff {
    /// Create a unique, private draft containing the exact initial Markdown.
    pub fn prepare(directory: &Path, initial: &str) -> Result<Self, EditorError> {
        let bytes = u64::try_from(initial.len()).unwrap_or(u64::MAX);
        if bytes > MAX_BODY_BYTES {
            return Err(EditorError::TooLarge {
                bytes,
                maximum: MAX_BODY_BYTES,
            });
        }
        std::fs::create_dir_all(directory).map_err(|source| EditorError::Io {
            operation: "creating draft directory",
            path: directory.to_path_buf(),
            source,
        })?;
        for _ in 0..100 {
            let sequence = NEXT_DRAFT.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!("aplan-body-{}-{sequence}.md", std::process::id()));
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(mut file) => {
                    use std::io::Write as _;
                    if let Err(source) = file
                        .write_all(initial.as_bytes())
                        .and_then(|()| file.sync_all())
                    {
                        drop(file);
                        let _ = std::fs::remove_file(&path);
                        return Err(EditorError::Io {
                            operation: "writing body draft",
                            path: path.clone(),
                            source,
                        });
                    }
                    return Ok(Self {
                        path,
                        complete: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(source) => {
                    return Err(EditorError::Io {
                        operation: "creating body draft",
                        path,
                        source,
                    });
                }
            }
        }
        Err(EditorError::Exhausted)
    }

    /// Path to append as the final argument of the configured editor command.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the edited UTF-8 Markdown and remove the draft.
    pub fn complete(self) -> Result<String, EditorError> {
        self.complete_after_inspection(|_| {})
    }

    fn complete_after_inspection(
        mut self,
        after_inspection: impl FnOnce(&Path),
    ) -> Result<String, EditorError> {
        // Open first, then compare the opened object with the name. Reading by
        // path after a metadata check leaves a symlink-swap window; reading the
        // handle makes a later rename harmless, and the Unix identity check
        // rejects a replacement between open and inspection.
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .open(&self.path)
            .map_err(|error| editor_io("opening edited body", &self.path, error))?;
        let opened = file
            .metadata()
            .map_err(|error| editor_io("inspecting opened body", &self.path, error))?;
        let named = std::fs::symlink_metadata(&self.path)
            .map_err(|error| editor_io("inspecting edited body", &self.path, error))?;
        if !opened.file_type().is_file() || !named.file_type().is_file() {
            return Err(EditorError::NotRegular {
                path: self.path.clone(),
            });
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if (opened.dev(), opened.ino()) != (named.dev(), named.ino()) {
                return Err(EditorError::Replaced {
                    path: self.path.clone(),
                });
            }
        }
        // The seam is a no-op in production. Tests use it to make changes in
        // the otherwise tiny interval after inspection deterministic, proving
        // that both a growing file and an atomic editor replacement fail
        // closed rather than relying on scheduler luck.
        after_inspection(&self.path);
        if opened.len() > MAX_BODY_BYTES {
            return Err(EditorError::TooLarge {
                bytes: opened.len(),
                maximum: MAX_BODY_BYTES,
            });
        }
        let mut bytes = Vec::with_capacity(usize::try_from(opened.len()).unwrap_or(0));
        file.by_ref()
            .take(MAX_BODY_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| editor_io("reading edited body", &self.path, error))?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BODY_BYTES {
            return Err(EditorError::TooLarge {
                bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                maximum: MAX_BODY_BYTES,
            });
        }
        let current = std::fs::symlink_metadata(&self.path)
            .map_err(|error| editor_io("reinspecting edited body", &self.path, error))?;
        if !current.file_type().is_file() {
            return Err(EditorError::NotRegular {
                path: self.path.clone(),
            });
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if (opened.dev(), opened.ino()) != (current.dev(), current.ino()) {
                return Err(EditorError::Replaced {
                    path: self.path.clone(),
                });
            }
        }
        let body = String::from_utf8(bytes).map_err(|error| {
            editor_io(
                "reading edited body as UTF-8",
                &self.path,
                std::io::Error::new(std::io::ErrorKind::InvalidData, error),
            )
        })?;
        drop(file);
        std::fs::remove_file(&self.path)
            .map_err(|error| editor_io("removing body draft", &self.path, error))?;
        self.complete = true;
        Ok(body)
    }
}

fn editor_io(operation: &'static str, path: &Path, source: std::io::Error) -> EditorError {
    EditorError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

impl Drop for ExternalEditorHandoff {
    fn drop(&mut self) {
        if !self.complete {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EditorError {
    #[error("{operation} at {}: {source}", path.display())]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("edited body is {bytes} bytes; the safety limit is {maximum}")]
    TooLarge { bytes: u64, maximum: u64 },
    #[error("edited body at {} is not a regular file", path.display())]
    NotRegular { path: PathBuf },
    #[error("edited body at {} was replaced while it was being inspected", path.display())]
    Replaced { path: PathBuf },
    #[error("could not allocate a unique body draft after 100 attempts")]
    Exhausted,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::{FormAction, FormTransition};
    use aplan_app::{ContextInputs, Trust};

    fn context(root: &Path) -> PlanContext {
        let plan = root.join("aplan/tasks");
        std::fs::create_dir_all(&plan).expect("tasks");
        std::fs::write(
            plan.join("A-1.md"),
            "---\nid: A-1\ntitle: one\nphase: 1\nstatus: todo\neffort_days: 1\n---\n",
        )
        .expect("task");
        PlanContext::new(
            aplan_core::load_plan_identified(&root.join("aplan")),
            ContextInputs {
                repository_root: root.to_path_buf(),
                configuration_file: None,
                trust: Trust::Hermetic,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
    }

    #[test]
    fn every_authoring_workflow_has_a_complete_form_and_names_cli_only_boundaries() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let context = context(dir.path());
        let cases = [
            (AuthoringWorkflow::Add, 19, true),
            (AuthoringWorkflow::Manifest, 1, true),
            (AuthoringWorkflow::Import, 8, true),
            (AuthoringWorkflow::Supersede, 11, false),
            (AuthoringWorkflow::Rekey, 3, false),
        ];
        assert_eq!(
            cases.iter().map(|case| case.0).collect::<Vec<_>>(),
            AuthoringWorkflow::ALL,
            "a workflow cannot be registered without a driven form case"
        );
        for (workflow, field_count, has_out_boundary) in cases {
            let form = form(&context, workflow, Some("A-1")).expect("valid definition");
            assert_eq!(form.title(), workflow.title());
            assert_eq!(form.fields().len(), field_count);
            assert_eq!(form.is_destructive(), workflow.destructive());
            let help = workflow.cli_only_help().join("\n");
            assert_eq!(help.contains("--out"), has_out_boundary);
            assert!(
                !help.contains("$SHELL") && !help.contains("sh -c"),
                "contextual help must never offer an embedded shell"
            );
        }
        let add = form(&context, AuthoringWorkflow::Add, None).expect("add form");
        for repeated in [
            "blocked_by",
            "prefer_after",
            "ratified_by",
            "contains",
            "members",
            "tags",
            "paths",
            "traces",
        ] {
            let field = add
                .fields()
                .iter()
                .find(|field| field.spec().key() == repeated)
                .expect("repeated field");
            assert_eq!(field.spec().cardinality(), crate::form::Cardinality::Many);
        }
    }

    fn submission(mut form: MutationForm, edits: &[(&str, &str)]) -> FormSubmission {
        for (key, value) in edits {
            let index = form
                .fields()
                .iter()
                .position(|field| field.spec().key() == *key)
                .expect("field exists");
            while form.focus() != crate::form::FormFocus::Field(index) {
                let _ = form.reduce(FormAction::NextFocus);
            }
            let _ = form.reduce(FormAction::Insert((*value).to_string()));
        }
        let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
            panic!("complete form did not yield a submission")
        };
        submission
    }

    fn replacement_submission(
        mut form: MutationForm,
        replacements: &[(&str, &str)],
    ) -> FormSubmission {
        for (key, value) in replacements {
            let index = form
                .fields()
                .iter()
                .position(|field| field.spec().key() == *key)
                .expect("field exists");
            while form.focus() != crate::form::FormFocus::Field(index) {
                let _ = form.reduce(FormAction::NextFocus);
            }
            for _ in 0..64 {
                let _ = form.reduce(FormAction::Backspace);
            }
            let _ = form.reduce(FormAction::Insert((*value).to_string()));
        }
        let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
            panic!("complete form did not yield a submission")
        };
        submission
    }

    #[test]
    fn add_form_decodes_external_body_and_reaches_the_typed_preview() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let context = context(dir.path());
        let add = submission(
            form(&context, AuthoringWorkflow::Add, None).expect("add form"),
            &[("title", "new task"), ("effort", "2.5")],
        );
        let decoded = request(
            AuthoringWorkflow::Add,
            &add,
            AuthoringAssets {
                edited_body: Some("## Why\n\nEdited outside.\n".to_string()),
                ..AuthoringAssets::default()
            },
        )
        .expect("add request");
        assert_eq!(
            decoded
                .preview(&context)
                .expect("typed add preview")
                .created(),
            ["A-2"]
        );
        let AuthoringRequest::Add(add) = decoded else {
            panic!("wrong add request")
        };
        assert_eq!(add.title, "new task");
        assert_eq!(add.effort_days, Some(2.5));
        assert_eq!(add.body.as_deref(), Some("## Why\n\nEdited outside.\n"));
    }

    #[test]
    fn add_and_supersede_forms_decode_every_new_creation_control() {
        // Native coverage for `add.accepts_creation_time_model_fields` and
        // `supersede.controls_inherited_successor_content`: the interactive
        // adapter must not silently replace typed values with defaults while
        // the CLI and manifest accept them.
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let context = context(dir.path());
        let add = submission(
            form(&context, AuthoringWorkflow::Add, None).expect("add form"),
            &[
                ("title", "new container"),
                ("id", "M-new"),
                ("phase", "2"),
                ("status", "blocked"),
                ("blocked_by", "M-v1"),
                ("prefer_after", "A-1"),
                ("ratified_by", "A-2"),
                ("contains", "M-v1"),
                ("tags", "milestone"),
                ("revisit_at", "2030-01-02T03:04:05Z"),
                ("version_impact", "feature"),
                ("version_impact_reason", "new delivery surface"),
            ],
        );
        let AuthoringRequest::Add(add) =
            request(AuthoringWorkflow::Add, &add, AuthoringAssets::default()).expect("typed add")
        else {
            panic!("wrong request")
        };
        assert_eq!(add.status, Some(aplan_core::Status::Blocked));
        assert_eq!(add.prefer_after, ["A-1"]);
        assert_eq!(add.ratified_by, ["A-2"]);
        assert_eq!(add.contains, ["M-v1"]);
        assert_eq!(add.revisit_at.as_deref(), Some("2030-01-02T03:04:05Z"));
        assert_eq!(
            add.version_impact,
            Some(aplan_core::task::VersionImpact::Feature)
        );
        assert_eq!(
            add.version_impact_reason.as_deref(),
            Some("new delivery surface")
        );

        let supersede = submission(
            form(&context, AuthoringWorkflow::Supersede, Some("A-1")).expect("supersede form"),
            &[
                ("title", "successor"),
                ("paths", "src/new.rs"),
                ("clear_traces", "yes"),
                ("body", "Replacement body."),
            ],
        );
        let AuthoringRequest::Supersede(supersede) = request(
            AuthoringWorkflow::Supersede,
            &supersede,
            AuthoringAssets::default(),
        )
        .expect("typed supersede") else {
            panic!("wrong request")
        };
        assert_eq!(supersede.paths, Some(vec!["src/new.rs".to_string()]));
        assert_eq!(supersede.traces, Some(Vec::new()));
        assert_eq!(supersede.body.as_deref(), Some("Replacement body."));

        let terminal = submission(
            form(&context, AuthoringWorkflow::Add, None).expect("add form"),
            &[("title", "history"), ("status", "done")],
        );
        assert!(matches!(
            request(
                AuthoringWorkflow::Add,
                &terminal,
                AuthoringAssets::default()
            ),
            Err(InputError::Invalid {
                field: "status",
                ..
            })
        ));

        let conflicting = submission(
            form(&context, AuthoringWorkflow::Supersede, Some("A-1")).expect("supersede form"),
            &[
                ("title", "successor"),
                ("paths", "src/new.rs"),
                ("clear_paths", "yes"),
            ],
        );
        assert!(matches!(
            request(
                AuthoringWorkflow::Supersede,
                &conflicting,
                AuthoringAssets::default()
            ),
            Err(InputError::Invalid {
                field: "clear_paths",
                ..
            })
        ));
    }

    #[test]
    fn add_form_moves_an_existing_member_through_the_native_typed_preview() {
        // Proves `add.creates_cut_around_existing_members` through the native
        // interactive form rather than a command escape hatch.
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let tasks = dir.path().join("aplan/tasks");
        std::fs::create_dir_all(&tasks).expect("tasks");
        std::fs::write(
            tasks.join("A-2.md"),
            "---\nid: A-2\ntitle: two\nphase: 1\nstatus: todo\neffort_days: 1\n---\n",
        )
        .expect("second task");
        std::fs::write(
            tasks.join("M-v1.md"),
            "---\nid: M-v1\ntitle: first cut\nphase: 1\nstatus: todo\n\
             blocked_by: [A-1, A-2]\ntags: [milestone, v1]\n---\n",
        )
        .expect("source cut");
        let context = context(dir.path());
        let submission = submission(
            form(&context, AuthoringWorkflow::Add, None).expect("add form"),
            &[
                ("title", "second cut"),
                ("id", "M-v2"),
                ("phase", "2"),
                ("blocked_by", "M-v1"),
                ("members", "A-2"),
                ("tags", "milestone"),
            ],
        );
        let decoded = request(
            AuthoringWorkflow::Add,
            &submission,
            AuthoringAssets::default(),
        )
        .expect("typed request");
        let preview = decoded.preview(&context).expect("native preview");
        assert_eq!(preview.created(), ["M-v2"]);
        assert_eq!(preview.change_set().files().len(), 2);
        assert!(preview.change_set().effects().iter().any(|effect| matches!(
            effect,
            aplan_app::SemanticEffect::CutMembershipRemoved { cut, tasks }
                if cut == "M-v1" && tasks == &["A-2"]
        )));
    }

    #[test]
    fn manifest_and_import_bind_the_selected_name_to_captured_bytes() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let context = context(dir.path());
        let manifest = submission(
            form(&context, AuthoringWorkflow::Manifest, None).expect("manifest form"),
            &[("source", "tasks.yml")],
        );
        assert!(matches!(
            request(
                AuthoringWorkflow::Manifest,
                &manifest,
                AuthoringAssets {
                    source: Some(CapturedSource {
                        name: "tasks.yml".to_string(),
                        contents: "- title: task\n".to_string(),
                    }),
                    ..AuthoringAssets::default()
                }
            ),
            Ok(AuthoringRequest::Manifest(_))
        ));

        let import = submission(
            form(&context, AuthoringWorkflow::Import, None).expect("import form"),
            &[("source", "kep.yaml")],
        );
        assert!(matches!(
            request(
                AuthoringWorkflow::Import,
                &import,
                AuthoringAssets {
                    source: Some(CapturedSource {
                        name: "kep.yaml".to_string(),
                        contents: "title: task\nkep-number: 9\n".to_string(),
                    }),
                    ..AuthoringAssets::default()
                }
            ),
            Ok(AuthoringRequest::Import(_))
        ));
        assert!(matches!(
            request(
                AuthoringWorkflow::Import,
                &import,
                AuthoringAssets {
                    source: Some(CapturedSource {
                        name: "newer.yaml".to_string(),
                        contents: String::new(),
                    }),
                    ..AuthoringAssets::default()
                }
            ),
            Err(InputError::SourceMoved { .. })
        ));
    }

    #[test]
    fn typed_authoring_decode_distinguishes_done_and_each_invalid_value_class() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let context = context(dir.path());
        let source = || AuthoringAssets {
            source: Some(CapturedSource {
                name: "kep.yaml".to_string(),
                contents: "title: task\nkep-number: 9\n".to_string(),
            }),
            ..AuthoringAssets::default()
        };

        let done = replacement_submission(
            form(&context, AuthoringWorkflow::Import, None).expect("import form"),
            &[
                ("source", "kep.yaml"),
                ("implemented_as", "done"),
                ("as_of", "2026-10-27"),
            ],
        );
        assert!(matches!(
            request(AuthoringWorkflow::Import, &done, source()),
            Ok(AuthoringRequest::Import(aplan_app::ImportRequest {
                implemented_as: aplan_app::ImplementedAs::Done { as_of },
                ..
            })) if as_of == "2026-10-27"
        ));

        for (field, value) in [("implemented_as", "later"), ("phase", "many")] {
            let invalid = replacement_submission(
                form(&context, AuthoringWorkflow::Import, None).expect("import form"),
                &[("source", "kep.yaml"), (field, value)],
            );
            assert!(matches!(
                request(AuthoringWorkflow::Import, &invalid, source()),
                Err(InputError::Invalid { field: rejected, .. }) if rejected == field
            ));
        }

        let bad_effort = replacement_submission(
            form(&context, AuthoringWorkflow::Add, None).expect("add form"),
            &[("title", "new task"), ("effort", "many")],
        );
        assert!(matches!(
            request(
                AuthoringWorkflow::Add,
                &bad_effort,
                AuthoringAssets::default()
            ),
            Err(InputError::Invalid {
                field: "effort",
                ..
            })
        ));

        let bad_boolean = replacement_submission(
            form(&context, AuthoringWorkflow::Rekey, Some("A-1")).expect("rekey form"),
            &[("new", "A-9"), ("allow_terminal", "maybe")],
        );
        assert!(matches!(
            request(
                AuthoringWorkflow::Rekey,
                &bad_boolean,
                AuthoringAssets::default()
            ),
            Err(InputError::Invalid {
                field: "allow_terminal",
                ..
            })
        ));
    }

    #[test]
    fn supersede_and_rekey_forms_decode_selected_task_and_destructive_override() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let context = context(dir.path());
        let supersede = submission(
            form(&context, AuthoringWorkflow::Supersede, Some("A-1")).expect("supersede form"),
            &[("title", "successor")],
        );
        assert!(matches!(
            request(
                AuthoringWorkflow::Supersede,
                &supersede,
                AuthoringAssets::default()
            ),
            Ok(AuthoringRequest::Supersede(_))
        ));

        let rekey = submission(
            form(&context, AuthoringWorkflow::Rekey, Some("A-1")).expect("rekey form"),
            &[("new", "A-9"), ("allow_terminal", "yes")],
        );
        assert!(matches!(
            request(AuthoringWorkflow::Rekey, &rekey, AuthoringAssets::default()),
            Ok(AuthoringRequest::Rekey(aplan_app::RekeyRequest {
                allow_terminal: true,
                ..
            }))
        ));
    }

    #[test]
    fn external_editor_handoff_is_private_exact_bounded_and_always_cleaned_up() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        assert!(matches!(
            ExternalEditorHandoff::prepare(
                dir.path(),
                &"x".repeat(usize::try_from(MAX_BODY_BYTES).expect("body limit") + 1),
            ),
            Err(EditorError::TooLarge { .. })
        ));
        assert_eq!(
            std::fs::read_dir(dir.path())
                .expect("empty draft directory")
                .count(),
            0,
            "an oversized initial value must be refused before a file is created"
        );

        let handoff = ExternalEditorHandoff::prepare(dir.path(), "initial\n").expect("draft");
        let path = handoff.path().to_path_buf();
        assert_eq!(std::fs::read_to_string(&path).expect("seed"), "initial\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&path)
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::write(&path, "edited body\n").expect("editor write");
        assert_eq!(handoff.complete().expect("complete"), "edited body\n");
        assert!(!path.exists());

        let cancelled = ExternalEditorHandoff::prepare(dir.path(), "cancel").expect("draft");
        let cancelled_path = cancelled.path().to_path_buf();
        drop(cancelled);
        assert!(!cancelled_path.exists());

        let invalid = ExternalEditorHandoff::prepare(dir.path(), "valid").expect("draft");
        let invalid_path = invalid.path().to_path_buf();
        std::fs::write(&invalid_path, [0xff]).expect("invalid UTF-8");
        assert!(matches!(invalid.complete(), Err(EditorError::Io { .. })));
        assert!(
            !invalid_path.exists(),
            "an error must not strand the private draft"
        );

        let oversized = ExternalEditorHandoff::prepare(dir.path(), "small").expect("draft");
        let oversized_path = oversized.path().to_path_buf();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&oversized_path)
            .expect("open draft")
            .set_len(MAX_BODY_BYTES + 1)
            .expect("enlarge draft");
        assert!(matches!(
            oversized.complete(),
            Err(EditorError::TooLarge { .. })
        ));
        assert!(
            !oversized_path.exists(),
            "an oversized draft must be removed"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let target = dir.path().join("outside.md");
            std::fs::write(&target, "must not be read or removed").expect("target");
            let swapped = ExternalEditorHandoff::prepare(dir.path(), "draft").expect("draft");
            let swapped_path = swapped.path().to_path_buf();
            std::fs::remove_file(&swapped_path).expect("replace draft");
            symlink(&target, &swapped_path).expect("swap in symlink");
            assert!(matches!(
                swapped.complete(),
                Err(EditorError::NotRegular { .. })
            ));
            assert!(!swapped_path.exists(), "the symlink must be removed");
            assert_eq!(
                std::fs::read_to_string(target).expect("target survives"),
                "must not be read or removed"
            );
        }
    }

    #[test]
    fn external_editor_handoff_refuses_growth_and_replacement_after_inspection() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let growing = ExternalEditorHandoff::prepare(dir.path(), "small").expect("draft");
        let growing_path = growing.path().to_path_buf();
        assert!(matches!(
            growing.complete_after_inspection(|path| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(path)
                    .expect("open inspected draft")
                    .set_len(MAX_BODY_BYTES + 1)
                    .expect("grow inspected draft");
            }),
            Err(EditorError::TooLarge {
                bytes,
                maximum: MAX_BODY_BYTES,
            }) if bytes == MAX_BODY_BYTES + 1
        ));
        assert!(
            !growing_path.exists(),
            "a draft that grows after inspection must be removed"
        );

        #[cfg(unix)]
        {
            let replaced =
                ExternalEditorHandoff::prepare(dir.path(), "opened body").expect("draft");
            let replaced_path = replaced.path().to_path_buf();
            assert!(matches!(
                replaced.complete_after_inspection(|path| {
                    let replacement = path.with_extension("replacement");
                    std::fs::write(&replacement, "different regular file")
                        .expect("replacement body");
                    std::fs::rename(replacement, path).expect("replace after inspection");
                }),
                Err(EditorError::Replaced { .. })
            ));
            assert!(
                !replaced_path.exists(),
                "a regular replacement must not survive a refused handoff"
            );
        }
    }
}
