//! Reusable state and review projections for guided plan mutations.
//!
//! Concrete commands own their typed requests and domain validation. This
//! module owns only interaction mechanics: bounded Unicode editing, local
//! completeness, repeated values, predictable focus, faithful `ChangeSet`
//! review and a confirmation boundary that rejects key repeats.

use std::collections::{BTreeMap, BTreeSet};

use aplan_app::{
    ChangeDisposition, ChangeEvidence, ChangeSet, CommitOutcome, PublicationFileState,
    PublicationReport, PublicationState, SemanticEffect, observe_publication,
    snapshot_observes_publication,
};
use ratatui::crossterm::event::KeyEventKind;

use crate::text;

const MAX_VALUE_CELLS: usize = 2_048;
const MAX_VALUE_GRAPHEMES: usize = 4_096;
const MAX_VALUE_BYTES: usize = 16_384;

/// Whether a field accepts one value or an ordered list of values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    One,
    Many,
}

impl Cardinality {
    const fn label(self) -> &'static str {
        match self {
            Self::One => "one value",
            Self::Many => "ordered list",
        }
    }
}

/// Static presentation and completeness rules for one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSpec {
    key: String,
    label: String,
    help: String,
    required: bool,
    cardinality: Cardinality,
    initial_values: Vec<String>,
}

impl FieldSpec {
    /// Define an optional, single-value field.
    pub fn new(key: impl Into<String>, label: impl Into<String>, help: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            help: help.into(),
            required: false,
            cardinality: Cardinality::One,
            initial_values: Vec::new(),
        }
    }

    /// Require at least one non-blank value.
    #[must_use]
    pub const fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Accept an ordered list rather than one value.
    #[must_use]
    pub const fn repeated(mut self) -> Self {
        self.cardinality = Cardinality::Many;
        self
    }

    /// Prepopulate one scalar value or the first member of a repeated field.
    #[must_use]
    pub fn initial_value(mut self, value: impl Into<String>) -> Self {
        self.initial_values = vec![value.into()];
        self
    }

    /// Prepopulate an ordered repeated field with concrete values.
    #[must_use]
    pub fn initial_values(mut self, values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.initial_values = values.into_iter().map(Into::into).collect();
        self
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn help(&self) -> &str {
        &self.help
    }

    pub const fn is_required(&self) -> bool {
        self.required
    }

    pub const fn cardinality(&self) -> Cardinality {
        self.cardinality
    }
}

/// A local input problem, before repository-aware validation runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalIssue {
    Required,
    IncompleteEntry { index: usize },
    Invalid { message: String },
}

impl std::fmt::Display for LocalIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Required => write!(f, "a value is required"),
            Self::IncompleteEntry { index } => {
                write!(f, "list entry {} is incomplete", index + 1)
            }
            Self::Invalid { message } => write!(f, "{message}"),
        }
    }
}

/// A typed-service refusal which is deliberately separate from local input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainIssue {
    pub summary: String,
    pub details: Vec<String>,
}

impl DomainIssue {
    pub fn new(summary: impl Into<String>, details: Vec<String>) -> Self {
        Self {
            summary: summary.into(),
            details,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Editor {
    value: String,
    cursor: usize,
}

impl Editor {
    fn empty() -> Self {
        Self {
            value: String::new(),
            cursor: 0,
        }
    }
}

/// Mutable value state for one form field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldState {
    spec: FieldSpec,
    editors: Vec<Editor>,
    selected: usize,
    issue: Option<LocalIssue>,
}

impl FieldState {
    fn new(spec: FieldSpec) -> Self {
        let editors = if spec.initial_values.is_empty() {
            vec![Editor::empty()]
        } else {
            spec.initial_values
                .iter()
                .map(|value| Editor {
                    value: value.clone(),
                    cursor: value.len(),
                })
                .collect()
        };
        Self {
            spec,
            editors,
            selected: 0,
            issue: None,
        }
    }

    pub const fn spec(&self) -> &FieldSpec {
        &self.spec
    }

    pub fn values(&self) -> impl ExactSizeIterator<Item = &str> {
        self.editors.iter().map(|editor| editor.value.as_str())
    }

    pub const fn selected(&self) -> usize {
        self.selected
    }

    pub const fn issue(&self) -> Option<&LocalIssue> {
        self.issue.as_ref()
    }

    fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editors[self.selected]
    }

    fn automatic_issue(&self) -> Option<LocalIssue> {
        let nonblank = self
            .editors
            .iter()
            .filter(|editor| !editor.value.trim().is_empty())
            .count();
        if self.spec.required && nonblank == 0 {
            return Some(LocalIssue::Required);
        }
        self.editors
            .iter()
            .enumerate()
            .find(|(_, editor)| {
                editor.value.trim().is_empty() && (self.editors.len() > 1 || nonblank > 0)
            })
            .map(|(index, _)| LocalIssue::IncompleteEntry { index })
    }

    fn submission_values(&self) -> Vec<String> {
        self.editors
            .iter()
            .map(|editor| editor.value.clone())
            .filter(|value| !value.trim().is_empty())
            .collect()
    }
}

/// Stable keyboard focus inside a form or preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormFocus {
    Field(usize),
    Cancel,
    Review,
    Recompute,
    Confirm,
}

/// Key lifecycle for a confirmation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Activation {
    fresh_press: bool,
}

impl Activation {
    /// Preserve the terminal's actual key lifecycle at the reducer boundary.
    pub const fn from_key_kind(kind: KeyEventKind) -> Self {
        Self {
            fresh_press: matches!(kind, KeyEventKind::Press),
        }
    }
}

/// Semantic editing action independent of Crossterm key bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormAction {
    NextFocus,
    PreviousFocus,
    NextItem,
    PreviousItem,
    Insert(String),
    Backspace,
    Delete,
    CursorLeft,
    CursorRight,
    CursorHome,
    CursorEnd,
    AddItem,
    RemoveItem,
    MoveItemUp,
    MoveItemDown,
    FocusField(usize),
    FocusItem { field: usize, item: usize },
    Review,
    Recompute,
    Scroll { delta: i16, from: usize },
    Cancel,
    Confirm(Activation),
    PointerConfirm(Activation),
}

/// Values handed to the operation-specific typed preview service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormSubmission {
    request: u64,
    input_sha256: String,
    subject_id: Option<String>,
    subject_sha256: Option<String>,
    fields: BTreeMap<String, Vec<String>>,
}

impl FormSubmission {
    pub const fn request(&self) -> u64 {
        self.request
    }

    pub fn input_sha256(&self) -> &str {
        &self.input_sha256
    }

    /// Immutable operation-specific subject observed when the form was
    /// populated. Unlike editable fields, this cannot be redirected by input.
    pub fn subject_sha256(&self) -> Option<&str> {
        self.subject_sha256.as_deref()
    }

    pub fn subject_id(&self) -> Option<&str> {
        self.subject_id.as_deref()
    }

    pub fn values(&self, key: &str) -> Option<&[String]> {
        self.fields.get(key).map(Vec::as_slice)
    }

    pub fn value(&self, key: &str) -> Option<&str> {
        self.values(key)
            .and_then(|values| values.first())
            .map(String::as_str)
    }
}

/// What the surrounding operation should do after reducing an action.
#[derive(Debug)]
pub enum FormTransition {
    None,
    PreviewRequested(FormSubmission),
    CommitRequested(Box<ChangeSet>),
    Cancelled,
}

/// Why a form definition itself is unusable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DefinitionError {
    #[error("a mutation form title must be non-blank")]
    BlankTitle,
    #[error("a mutation form input identity must be non-blank")]
    BlankInput,
    #[error("a mutation form needs at least one field")]
    NoFields,
    #[error("form field keys must be non-blank")]
    BlankKey,
    #[error("form field key `{0}` is declared more than once")]
    DuplicateKey(String),
    #[error("form field key `{0}` has surrounding whitespace")]
    UntrimmedKey(String),
    #[error("form field `{0}` needs a non-blank label")]
    BlankLabel(String),
    #[error("single-value form field `{0}` has more than one initial value")]
    TooManyInitialValues(String),
    #[error("form field key `{0}` is not declared")]
    UnknownKey(String),
}

#[derive(Debug)]
struct Preview {
    change: ChangeSet,
    stale_actual: Option<String>,
    commit: Option<CommitStatus>,
}

impl Preview {
    fn is_confirmable(&self) -> bool {
        self.stale_actual.is_none()
            && self.commit.is_none()
            && self.change.disposition() == ChangeDisposition::Ready
    }
}

/// Visible publication state retained after a confirmation attempt.
#[derive(Debug)]
pub enum CommitStatus {
    /// The publisher returned success, but a fresh load has not yet observed the
    /// reviewed after-state. The interface must not call this committed yet.
    AwaitingReload,
    /// A no-op or denied value reached the commit boundary without a write.
    NotPublished { reason: &'static str },
    /// Publication returned an error and the affected paths were observed.
    Failed {
        error: String,
        report: PublicationReport,
    },
    /// A fresh load reconciled every affected path with the reviewed bytes.
    Reconciled {
        input_sha256: String,
        report: PublicationReport,
    },
}

impl CommitStatus {
    #[must_use]
    pub const fn report(&self) -> Option<&PublicationReport> {
        match self {
            Self::Failed { report, .. } | Self::Reconciled { report, .. } => Some(report),
            Self::AwaitingReload | Self::NotPublished { .. } => None,
        }
    }
}

/// A reusable mutation form and its immutable preview boundary.
#[derive(Debug)]
pub struct MutationForm {
    title: String,
    input_sha256: String,
    subject_id: Option<String>,
    subject_sha256: Option<String>,
    destructive: bool,
    guidance: Vec<String>,
    external_note: Option<String>,
    review_rows: Vec<String>,
    fields: Vec<FieldState>,
    focus: FormFocus,
    domain_issue: Option<DomainIssue>,
    preview: Option<Preview>,
    scroll: usize,
    /// Manual scrolling may intentionally move away from the focused field.
    /// Without it, the responsive renderer follows semantic focus and derives
    /// a presentation offset for the current width and height.
    scroll_explicit: bool,
    next_request: u64,
    pending_request: Option<u64>,
    confirmation_armed: bool,
}

impl MutationForm {
    pub fn new(
        title: impl Into<String>,
        input_sha256: impl Into<String>,
        destructive: bool,
        specs: Vec<FieldSpec>,
    ) -> Result<Self, DefinitionError> {
        let title = title.into();
        let input_sha256 = input_sha256.into();
        if title.trim().is_empty() {
            return Err(DefinitionError::BlankTitle);
        }
        if input_sha256.trim().is_empty() {
            return Err(DefinitionError::BlankInput);
        }
        validate_specs(&specs)?;
        Ok(Self {
            title,
            input_sha256,
            subject_id: None,
            subject_sha256: None,
            destructive,
            guidance: Vec::new(),
            external_note: None,
            review_rows: Vec::new(),
            fields: specs.into_iter().map(FieldState::new).collect(),
            focus: FormFocus::Field(0),
            domain_issue: None,
            preview: None,
            scroll: 0,
            scroll_explicit: false,
            next_request: 1,
            pending_request: None,
            confirmation_armed: false,
        })
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn input_sha256(&self) -> &str {
        &self.input_sha256
    }

    /// Bind this form to an exact domain subject observed while its editable
    /// defaults were populated. The binding is carried outside the field map,
    /// so editing a visible identity cannot replace the reviewed subject.
    #[must_use]
    pub fn with_subject(mut self, id: impl Into<String>, sha256: impl Into<String>) -> Self {
        self.subject_id = Some(id.into());
        self.subject_sha256 = Some(sha256.into());
        self
    }

    pub const fn is_destructive(&self) -> bool {
        self.destructive
    }

    /// Add read-only context that is visible before the user requests a
    /// preview. Operation-specific forms use this for policy that should guide
    /// input rather than arrive only after an invalid submission.
    #[must_use]
    pub fn with_guidance(mut self, guidance: Vec<String>) -> Self {
        self.guidance = guidance;
        self
    }

    /// Append operation-specific boundary guidance without replacing domain
    /// policy already supplied by the form owner.
    #[must_use]
    pub fn with_additional_guidance(mut self, guidance: impl IntoIterator<Item = String>) -> Self {
        self.guidance.extend(guidance);
        self
    }

    pub fn fields(&self) -> &[FieldState] {
        &self.fields
    }

    #[cfg(test)]
    pub(crate) fn guidance(&self) -> &[String] {
        &self.guidance
    }

    /// Read the first current value of one field without manufacturing a
    /// submission. Terminal handoffs use this to seed an external editor while
    /// preview tokens remain owned by the ordinary review action.
    pub(crate) fn value(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|field| field.spec.key == key)
            .and_then(|field| field.editors.first())
            .map(|editor| editor.value.as_str())
    }

    /// Record context returned by a terminal-independent handoff.
    ///
    /// The note participates in the same wrapped/pointer row geometry as all
    /// other form context. Changing it invalidates an older preview because the
    /// surrounding workflow may prefer the external value over a visible
    /// short field.
    pub(crate) fn set_external_note(&mut self, note: Option<String>) {
        if self.external_note == note {
            return;
        }
        self.external_note = note;
        self.invalidate_preview();
    }

    pub const fn focus(&self) -> FormFocus {
        self.focus
    }

    pub const fn is_editing(&self) -> bool {
        self.preview.is_none()
    }

    pub fn can_confirm(&self) -> bool {
        self.preview.as_ref().is_some_and(Preview::is_confirmable)
    }

    pub fn confirm_label(&self) -> &'static str {
        let Some(preview) = self
            .preview
            .as_ref()
            .filter(|preview| preview.is_confirmable())
        else {
            return "Confirm unavailable";
        };
        if is_broad(&preview.change) {
            if self.confirmation_armed {
                "Confirm broad change"
            } else {
                "Arm broad change"
            }
        } else {
            "Confirm"
        }
    }

    /// Whether a confirmable preview uses proportional two-step confirmation,
    /// and whether that boundary has already been armed.
    pub fn broad_confirmation_armed(&self) -> Option<bool> {
        self.preview
            .as_ref()
            .filter(|preview| preview.is_confirmable() && is_broad(&preview.change))
            .map(|_| self.confirmation_armed)
    }

    /// Replace one field with trusted values supplied by another typed UI.
    ///
    /// This is deliberately not a parser bypass: the ordinary local and
    /// domain validation still run when the user requests a preview. It only
    /// lets selectors seed an exact value without simulating key presses.
    pub fn prefill(
        &mut self,
        key: &str,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<(), DefinitionError> {
        let index = self
            .fields
            .iter()
            .position(|field| field.spec.key == key)
            .ok_or_else(|| DefinitionError::UnknownKey(key.to_string()))?;
        let values = values.into_iter().map(Into::into).collect::<Vec<_>>();
        if self.fields[index].spec.cardinality == Cardinality::One && values.len() > 1 {
            return Err(DefinitionError::TooManyInitialValues(key.to_string()));
        }
        self.fields[index].editors = if values.is_empty() {
            vec![Editor::empty()]
        } else {
            values
                .into_iter()
                .map(|value| Editor {
                    cursor: value.len(),
                    value,
                })
                .collect()
        };
        self.fields[index].selected = 0;
        self.fields[index].issue = None;
        self.focus = FormFocus::Field(index);
        self.invalidate_preview();
        Ok(())
    }

    pub const fn domain_issue(&self) -> Option<&DomainIssue> {
        self.domain_issue.as_ref()
    }

    pub fn change_set(&self) -> Option<&ChangeSet> {
        self.preview.as_ref().map(|preview| &preview.change)
    }

    pub fn commit_status(&self) -> Option<&CommitStatus> {
        self.preview
            .as_ref()
            .and_then(|preview| preview.commit.as_ref())
    }

    pub fn set_local_issue(&mut self, key: &str, issue: LocalIssue) -> Result<(), DefinitionError> {
        let index = self
            .fields
            .iter()
            .position(|field| field.spec.key == key)
            .ok_or_else(|| DefinitionError::UnknownKey(key.to_string()))?;
        self.fields[index].issue = Some(issue);
        self.invalidate_preview();
        self.focus = FormFocus::Field(index);
        Ok(())
    }

    /// Install the typed service's answer. This never performs a write.
    pub fn accept_preview(&mut self, request: u64, result: Result<ChangeSet, DomainIssue>) -> bool {
        self.accept_preview_with_rows(request, result, Vec::new())
    }

    /// Install a typed preview and its operation-specific review projection.
    ///
    /// Supplemental rows are bound to the same request id as the immutable
    /// change, so a late worker answer cannot pair current fields with older
    /// readiness, import or selection context.
    pub fn accept_preview_with_rows(
        &mut self,
        request: u64,
        result: Result<ChangeSet, DomainIssue>,
        review_rows: Vec<String>,
    ) -> bool {
        if !self.accept_preview_request(request) {
            return false;
        }
        match result {
            Ok(change) => {
                let stale_actual =
                    (change.input().sha256 != self.input_sha256).then(|| self.input_sha256.clone());
                self.preview = Some(Preview {
                    change,
                    stale_actual,
                    commit: None,
                });
                self.review_rows = review_rows
                    .into_iter()
                    .map(|row| text::safe(&row))
                    .collect();
                self.domain_issue = None;
                // A held Enter that requested review therefore lands on the
                // safe control even before key-kind filtering is considered.
                self.focus = FormFocus::Cancel;
            }
            Err(issue) => {
                self.preview = None;
                self.review_rows.clear();
                self.domain_issue = Some(issue);
                self.focus = FormFocus::Review;
            }
        }
        true
    }

    /// Consume the token for a typed preview whose review state is owned by
    /// the surrounding workflow rather than by this generic form.
    ///
    /// Claim is currently that workflow: its immutable application preview is
    /// not a [`ChangeSet`], but an edit made while it is in flight must still
    /// invalidate the response exactly as it does for plan mutations.
    pub(crate) fn accept_preview_request(&mut self, request: u64) -> bool {
        if self.pending_request != Some(request) {
            return false;
        }
        self.pending_request = None;
        self.scroll = 0;
        self.scroll_explicit = false;
        true
    }

    /// Accept a reconciliation already proved by the repository worker after
    /// it loaded a fresh application context.
    pub fn accept_reconciliation(
        &mut self,
        input_sha256: impl Into<String>,
        report: PublicationReport,
    ) -> bool {
        let input_sha256 = input_sha256.into();
        if input_sha256.trim().is_empty() || report.state() != PublicationState::Complete {
            return false;
        }
        let Some(preview) = self.preview.as_mut() else {
            return false;
        };
        if !matches!(preview.commit, Some(CommitStatus::AwaitingReload)) {
            return false;
        }
        preview.commit = Some(CommitStatus::Reconciled {
            input_sha256,
            report,
        });
        self.focus = FormFocus::Cancel;
        self.scroll = 0;
        self.scroll_explicit = false;
        true
    }

    /// Retain a refused preview for inspection while disabling confirmation.
    pub fn accept_stale(&mut self, change: ChangeSet, actual_sha256: impl Into<String>) {
        let actual_sha256 = actual_sha256.into();
        self.input_sha256.clone_from(&actual_sha256);
        self.pending_request = None;
        self.preview = Some(Preview {
            change,
            stale_actual: Some(actual_sha256),
            commit: None,
        });
        self.review_rows.clear();
        self.domain_issue = None;
        self.focus = FormFocus::Cancel;
        self.scroll = 0;
        self.scroll_explicit = false;
    }

    /// Retain the exact reviewed change and its publication result.
    ///
    /// Fields are deliberately left intact. A denied, stale, or failed write
    /// can therefore be inspected and safely recomputed without re-entering its
    /// scope. Even `Applied` remains reload-pending: only
    /// a worker-proved reconciliation can turn it into a visible committed
    /// result.
    pub fn accept_commit_outcome(&mut self, outcome: CommitOutcome) {
        let (change, stale_actual, commit) = match outcome {
            CommitOutcome::Applied(change) => (change, None, CommitStatus::AwaitingReload),
            CommitOutcome::NoOp(change) => {
                let reason = if change.focused_test_execution() {
                    "the reviewed state already existed; `aplan` published no plan bytes; effects of the configured test command are not rolled back"
                } else {
                    "the reviewed state already existed; nothing was written"
                };
                (change, None, CommitStatus::NotPublished { reason })
            }
            CommitOutcome::Denied(change) => {
                let reason = if change.focused_test_execution() {
                    "policy denied the reviewed state; `aplan` published no plan bytes; effects of the configured test command are not rolled back"
                } else {
                    "policy denied the reviewed state; nothing was written"
                };
                (change, None, CommitStatus::NotPublished { reason })
            }
            CommitOutcome::Stale { change, actual } => {
                self.input_sha256.clone_from(&actual.sha256);
                let reason = if change.focused_test_execution() {
                    "the identified input changed; `aplan` published no plan bytes; effects of the configured test command are not rolled back"
                } else {
                    "the identified input changed; nothing was written"
                };
                (
                    change,
                    Some(actual.sha256),
                    CommitStatus::NotPublished { reason },
                )
            }
            CommitOutcome::PublicationFailed {
                change,
                report,
                error,
            } => {
                let error = if change.focused_test_execution() {
                    let publication = if report.state() == PublicationState::Unchanged {
                        "`aplan` published no plan bytes"
                    } else {
                        "`aplan` could not confirm complete publication; inspect every reported path"
                    };
                    format!(
                        "{publication}. Effects of the configured test command are not rolled back. Writer error: {error}"
                    )
                } else {
                    error.to_string()
                };
                (change, None, CommitStatus::Failed { error, report })
            }
        };
        self.preview = Some(Preview {
            change,
            stale_actual,
            commit: Some(commit),
        });
        self.pending_request = None;
        self.confirmation_armed = false;
        self.domain_issue = None;
        self.focus = FormFocus::Cancel;
        self.scroll = 0;
        self.scroll_explicit = false;
    }

    /// Reconcile a prior publication attempt after the surrounding shell has
    /// completed a fresh load. The report reads every reviewed path again, so
    /// an interruption after commit has the same visible answer as an
    /// uninterrupted session.
    pub fn reconcile_reload(&mut self, context: &aplan_app::PlanContext) -> bool {
        let Some(preview) = self.preview.as_mut() else {
            return false;
        };
        if !matches!(
            preview.commit,
            Some(CommitStatus::AwaitingReload | CommitStatus::Failed { .. })
        ) {
            return false;
        }
        let input = context.identity();
        if input.plan_root != preview.change.input().plan_root
            || input.repository_root != preview.change.input().repository_root
            || input.configuration.path != preview.change.input().configuration.path
            || input.trust != preview.change.input().trust
            || !snapshot_observes_publication(context, &preview.change)
        {
            return false;
        }
        let report = observe_publication(&preview.change);
        preview.commit = Some(CommitStatus::Reconciled {
            input_sha256: input.sha256.clone(),
            report,
        });
        self.focus = FormFocus::Cancel;
        self.scroll = 0;
        self.scroll_explicit = false;
        true
    }

    pub fn reduce(&mut self, action: FormAction) -> FormTransition {
        match action {
            FormAction::NextFocus => self.move_focus(true),
            FormAction::PreviousFocus => self.move_focus(false),
            FormAction::NextItem => self.move_item(true),
            FormAction::PreviousItem => self.move_item(false),
            FormAction::Insert(value) => self.edit(|editor| {
                text::insert_bounded(
                    &mut editor.value,
                    &mut editor.cursor,
                    &value,
                    MAX_VALUE_CELLS,
                    MAX_VALUE_GRAPHEMES,
                    MAX_VALUE_BYTES,
                )
            }),
            FormAction::Backspace => self.edit(|editor| {
                text::delete_previous_grapheme(&mut editor.value, &mut editor.cursor)
            }),
            FormAction::Delete => self
                .edit(|editor| text::delete_next_grapheme(&mut editor.value, &mut editor.cursor)),
            FormAction::CursorLeft => self.move_cursor(false),
            FormAction::CursorRight => self.move_cursor(true),
            FormAction::CursorHome => self.set_cursor(false),
            FormAction::CursorEnd => self.set_cursor(true),
            FormAction::AddItem => self.add_item(),
            FormAction::RemoveItem => self.remove_item(),
            FormAction::MoveItemUp => self.reorder_item(false),
            FormAction::MoveItemDown => self.reorder_item(true),
            FormAction::FocusField(index) => self.focus_field(index),
            FormAction::FocusItem { field, item } => self.focus_item(field, item),
            FormAction::Review | FormAction::Recompute => return self.request_preview(),
            FormAction::Scroll { delta, from } => self.scroll(delta, from),
            FormAction::Cancel => return FormTransition::Cancelled,
            FormAction::Confirm(activation) => return self.confirm(activation),
            FormAction::PointerConfirm(activation) => {
                if activation.fresh_press {
                    self.focus = FormFocus::Confirm;
                }
                return self.confirm(activation);
            }
        }
        FormTransition::None
    }

    pub fn render(&self, width: usize, height: usize) -> RenderedForm {
        let rows = self.all_rows();
        RenderedForm::window(&rows, width, height, self.resolved_scroll(width, height))
    }

    /// Semantic pointer targets for the exact editing rows returned by
    /// [`Self::render`]. Wrapped rows retain their field/item identity; rows
    /// outside the current scroll window produce no target.
    pub fn pointer_rows(&self, width: usize, height: usize) -> Vec<(usize, FormAction)> {
        if !self.is_editing() {
            return Vec::new();
        }
        let tagged = editing_rows_tagged(self);
        let rows = tagged
            .iter()
            .map(|(row, _)| row.clone())
            .collect::<Vec<_>>();
        let window =
            RenderedForm::window(&rows, width, height, self.resolved_scroll(width, height));
        let physical = tagged
            .into_iter()
            .flat_map(|(row, action)| {
                text::wrap(&row, width)
                    .into_iter()
                    .map(move |_| action.clone())
            })
            .collect::<Vec<_>>();
        physical[window.first_row..window.last_row]
            .iter()
            .enumerate()
            .filter_map(|(row, action)| action.clone().map(|action| (row, action)))
            .collect()
    }

    fn request_preview(&mut self) -> FormTransition {
        if self.pending_request.is_some() {
            return FormTransition::None;
        }
        self.preview = None;
        self.review_rows.clear();
        self.domain_issue = None;
        self.confirmation_armed = false;
        let mut first_invalid = None;
        for (index, field) in self.fields.iter_mut().enumerate() {
            if field.issue.is_none() {
                field.issue = field.automatic_issue();
            }
            if field.issue.is_some() && first_invalid.is_none() {
                first_invalid = Some(index);
            }
        }
        if let Some(index) = first_invalid {
            self.focus = FormFocus::Field(index);
            return FormTransition::None;
        }
        self.focus = FormFocus::Review;
        let request = self.next_request;
        self.next_request = self.next_request.saturating_add(1);
        self.pending_request = Some(request);
        FormTransition::PreviewRequested(self.submission(request))
    }

    fn confirm(&mut self, activation: Activation) -> FormTransition {
        if !activation.fresh_press || self.focus != FormFocus::Confirm {
            return FormTransition::None;
        }
        let Some(preview) = self.preview.as_ref() else {
            return FormTransition::None;
        };
        if !preview.is_confirmable() {
            return FormTransition::None;
        }
        if is_broad(&preview.change) && !self.confirmation_armed {
            self.confirmation_armed = true;
            // Moving back to the safe control makes the second confirmation a
            // deliberate navigation plus press even on terminals which cannot
            // distinguish an auto-repeat from another `Press` event.
            self.focus = FormFocus::Cancel;
            return FormTransition::None;
        }
        let preview = self
            .preview
            .take()
            .expect("the preview was inspected above");
        FormTransition::CommitRequested(Box::new(preview.change))
    }

    fn submission(&self, request: u64) -> FormSubmission {
        FormSubmission {
            request,
            input_sha256: self.input_sha256.clone(),
            subject_id: self.subject_id.clone(),
            subject_sha256: self.subject_sha256.clone(),
            fields: self
                .fields
                .iter()
                .map(|field| (field.spec.key.clone(), field.submission_values()))
                .collect(),
        }
    }

    fn editing(&self) -> bool {
        self.preview.is_none()
    }

    fn focus_ring(&self) -> Vec<FormFocus> {
        if self.editing() {
            self.fields
                .iter()
                .enumerate()
                .map(|(index, _)| FormFocus::Field(index))
                .chain([FormFocus::Cancel, FormFocus::Review])
                .collect()
        } else {
            vec![FormFocus::Cancel, FormFocus::Recompute, FormFocus::Confirm]
        }
    }

    fn move_focus(&mut self, forward: bool) {
        let ring = self.focus_ring();
        let current = ring
            .iter()
            .position(|candidate| *candidate == self.focus)
            .unwrap_or_default();
        let next = if forward {
            (current + 1) % ring.len()
        } else {
            current.checked_sub(1).unwrap_or(ring.len() - 1)
        };
        self.focus = ring[next];
        self.scroll_explicit = false;
    }

    fn focus_field(&mut self, index: usize) {
        if index < self.fields.len() && self.editing() {
            self.focus = FormFocus::Field(index);
            self.scroll_explicit = false;
        }
    }

    fn focus_item(&mut self, field: usize, item: usize) {
        if !self.editing() {
            return;
        }
        let Some(state) = self.fields.get_mut(field) else {
            return;
        };
        if item < state.editors.len() {
            state.selected = item;
            self.focus = FormFocus::Field(field);
            self.scroll_explicit = false;
        }
    }

    fn focused_field_mut(&mut self) -> Option<&mut FieldState> {
        let FormFocus::Field(index) = self.focus else {
            return None;
        };
        self.fields.get_mut(index)
    }

    fn edit(&mut self, operation: impl FnOnce(&mut Editor) -> bool) {
        let changed = self
            .focused_field_mut()
            .is_some_and(|field| operation(field.editor_mut()));
        if changed {
            self.clear_focused_issue();
            self.invalidate_preview();
            self.scroll_explicit = false;
        }
    }

    fn move_cursor(&mut self, forward: bool) {
        if let Some(field) = self.focused_field_mut() {
            let editor = field.editor_mut();
            text::move_cursor(&editor.value, &mut editor.cursor, forward);
            self.scroll_explicit = false;
        }
    }

    fn set_cursor(&mut self, end: bool) {
        if let Some(field) = self.focused_field_mut() {
            let editor = field.editor_mut();
            editor.cursor = if end { editor.value.len() } else { 0 };
            self.scroll_explicit = false;
        }
    }

    fn move_item(&mut self, forward: bool) {
        let Some(field) = self.focused_field_mut() else {
            return;
        };
        let count = field.editors.len();
        if count < 2 {
            return;
        }
        field.selected = if forward {
            (field.selected + 1) % count
        } else {
            field.selected.checked_sub(1).unwrap_or(count - 1)
        };
        self.scroll_explicit = false;
    }

    fn add_item(&mut self) {
        let Some(field) = self.focused_field_mut() else {
            return;
        };
        if field.spec.cardinality != Cardinality::Many {
            return;
        }
        field.selected += 1;
        field.editors.insert(field.selected, Editor::empty());
        field.issue = None;
        self.invalidate_preview();
        self.scroll_explicit = false;
    }

    fn remove_item(&mut self) {
        let Some(field) = self.focused_field_mut() else {
            return;
        };
        if field.spec.cardinality != Cardinality::Many {
            return;
        }
        if field.editors.len() == 1 {
            field.editors[0] = Editor::empty();
        } else {
            field.editors.remove(field.selected);
            field.selected = field.selected.min(field.editors.len() - 1);
        }
        field.issue = None;
        self.invalidate_preview();
        self.scroll_explicit = false;
    }

    fn reorder_item(&mut self, forward: bool) {
        let Some(field) = self.focused_field_mut() else {
            return;
        };
        if field.spec.cardinality != Cardinality::Many || field.editors.len() < 2 {
            return;
        }
        let target = if forward {
            (field.selected + 1).min(field.editors.len() - 1)
        } else {
            field.selected.saturating_sub(1)
        };
        if target != field.selected {
            field.editors.swap(field.selected, target);
            field.selected = target;
            field.issue = None;
            self.invalidate_preview();
            self.scroll_explicit = false;
        }
    }

    fn clear_focused_issue(&mut self) {
        if let Some(field) = self.focused_field_mut() {
            field.issue = None;
        }
    }

    fn invalidate_preview(&mut self) {
        self.preview = None;
        self.review_rows.clear();
        self.domain_issue = None;
        self.pending_request = None;
        self.confirmation_armed = false;
        self.scroll = 0;
        self.scroll_explicit = false;
    }

    fn scroll(&mut self, delta: i16, from: usize) {
        self.scroll = if delta.is_negative() {
            from.saturating_sub(usize::from(delta.unsigned_abs()))
        } else {
            from.saturating_add(usize::from(delta.unsigned_abs()))
        };
        self.scroll_explicit = true;
    }

    pub(crate) fn resolved_scroll(&self, width: usize, height: usize) -> usize {
        if self.scroll_explicit || !self.editing() || height == 0 {
            return self.scroll;
        }
        let tagged = editing_rows_tagged(self);
        let target = editing_focus_logical_row(self, &tagged);
        let physical = tagged
            .iter()
            .take(target)
            .map(|(row, _)| text::wrap(row, width).len())
            .sum::<usize>();
        let focus_row = physical.saturating_add(editing_focus_wrapped_row(self, width, target));
        let total = tagged
            .iter()
            .map(|(row, _)| text::wrap(row, width).len())
            .sum::<usize>();
        let capacity = height.saturating_sub(usize::from(total > height)).max(1);
        focus_row.saturating_add(1).saturating_sub(capacity)
    }

    fn all_rows(&self) -> Vec<String> {
        if let Some(preview) = &self.preview {
            preview_rows(self, preview)
        } else {
            editing_rows(self)
        }
    }
}

fn editing_focus_wrapped_row(form: &MutationForm, width: usize, logical_row: usize) -> usize {
    let FormFocus::Field(field) = form.focus else {
        return editing_rows_tagged(form)
            .get(logical_row)
            .map_or(0, |(row, _)| text::wrap(row, width).len().saturating_sub(1));
    };
    let Some(state) = form.fields.get(field) else {
        return 0;
    };
    let item = state.selected.min(state.editors.len().saturating_sub(1));
    let Some(editor) = state.editors.get(item) else {
        return 0;
    };
    let cursor = editor.cursor.min(editor.value.len());
    let prefix = format!("    * {}: {}|", item + 1, &editor.value[..cursor]);
    text::wrap(&prefix, width).len().saturating_sub(1)
}

fn editing_focus_logical_row(form: &MutationForm, rows: &[(String, Option<FormAction>)]) -> usize {
    match form.focus {
        FormFocus::Field(field) => {
            let item = form.fields.get(field).map_or(0, |state| state.selected);
            rows.iter()
                .position(|(_, action)| *action == Some(FormAction::FocusItem { field, item }))
                .or_else(|| {
                    rows.iter()
                        .position(|(_, action)| *action == Some(FormAction::FocusField(field)))
                })
                .unwrap_or_default()
        }
        FormFocus::Cancel | FormFocus::Review | FormFocus::Recompute | FormFocus::Confirm => {
            rows.len().saturating_sub(1)
        }
    }
}

fn validate_specs(specs: &[FieldSpec]) -> Result<(), DefinitionError> {
    if specs.is_empty() {
        return Err(DefinitionError::NoFields);
    }
    let mut keys = std::collections::BTreeSet::new();
    for spec in specs {
        if spec.key.trim().is_empty() {
            return Err(DefinitionError::BlankKey);
        }
        if spec.key.trim() != spec.key {
            return Err(DefinitionError::UntrimmedKey(spec.key.clone()));
        }
        if spec.label.trim().is_empty() {
            return Err(DefinitionError::BlankLabel(spec.key.clone()));
        }
        if spec.cardinality == Cardinality::One && spec.initial_values.len() > 1 {
            return Err(DefinitionError::TooManyInitialValues(spec.key.clone()));
        }
        if !keys.insert(spec.key.as_str()) {
            return Err(DefinitionError::DuplicateKey(spec.key.clone()));
        }
    }
    Ok(())
}

fn editing_rows(form: &MutationForm) -> Vec<String> {
    editing_rows_tagged(form)
        .into_iter()
        .map(|(row, _)| row)
        .collect()
}

fn editing_rows_tagged(form: &MutationForm) -> Vec<(String, Option<FormAction>)> {
    let mut rows = vec![(format!("FORM: {}", form.title), None)];
    if !form.guidance.is_empty() || form.external_note.is_some() {
        rows.push(("Context".to_string(), None));
        rows.extend(form.guidance.iter().map(|line| (format!("  {line}"), None)));
        if let Some(note) = &form.external_note {
            rows.push((format!("  {note}"), None));
        }
    }
    for (field_index, field) in form.fields.iter().enumerate() {
        let focus = if form.focus == FormFocus::Field(field_index) {
            ">"
        } else {
            " "
        };
        let required = if field.spec.required {
            "required"
        } else {
            "optional"
        };
        rows.push((
            format!(
                "{focus} {} [{required}; {}] - {}",
                field.spec.label,
                field.spec.cardinality.label(),
                field.spec.help
            ),
            Some(FormAction::FocusField(field_index)),
        ));
        for (value_index, editor) in field.editors.iter().enumerate() {
            let selected = if field.selected == value_index {
                "*"
            } else {
                " "
            };
            let value = editor_value(
                editor,
                form.focus == FormFocus::Field(field_index) && field.selected == value_index,
            );
            rows.push((
                format!("    {selected} {}: {value}", value_index + 1),
                Some(FormAction::FocusItem {
                    field: field_index,
                    item: value_index,
                }),
            ));
        }
        if let Some(issue) = &field.issue {
            rows.push((format!("    LOCAL {}: {issue}", field.spec.key), None));
        }
    }
    if let Some(issue) = &form.domain_issue {
        rows.push((format!("PLAN: {}", issue.summary), None));
        rows.extend(
            issue
                .details
                .iter()
                .map(|detail| (format!("  {detail}"), None)),
        );
    }
    rows.push((
        format!(
            "Controls: {}Cancel  {}Review",
            focus_marker(form.focus, FormFocus::Cancel),
            focus_marker(form.focus, FormFocus::Review)
        ),
        None,
    ));
    rows
}

fn editor_value(editor: &Editor, focused: bool) -> String {
    if !focused {
        return editor.value.clone();
    }
    let mut value = editor.value.clone();
    value.insert(editor.cursor, '|');
    value
}

fn preview_rows(form: &MutationForm, preview: &Preview) -> Vec<String> {
    let change = &preview.change;
    let mut rows = vec![format!("PREVIEW: {}", form.title)];
    let verdict = publication_verdict(preview);
    rows.push(format!(
        "Verdict: {verdict}{}",
        if form.destructive {
            "; destructive"
        } else {
            ""
        }
    ));
    rows.push(format!("Preview input: {}", change.input().sha256));
    if let Some(actual) = &preview.stale_actual {
        rows.push(format!("Current input: {actual}"));
        rows.push("Confirmation disabled: recompute from the current snapshot.".to_string());
    }
    append_impact(&mut rows, change);
    append_files(&mut rows, change);
    append_effects(&mut rows, change);
    append_findings(&mut rows, change);
    append_evidence(&mut rows, change);
    if !form.review_rows.is_empty() {
        rows.push(format!("Operation review ({})", form.review_rows.len()));
        rows.extend(form.review_rows.iter().cloned());
    }
    append_commit_status(&mut rows, preview.commit.as_ref());
    let confirm = if preview.is_confirmable() {
        if is_broad(change) {
            if form.confirmation_armed {
                "Confirm broad change"
            } else {
                "Arm broad change"
            }
        } else {
            "Confirm"
        }
    } else {
        "Confirm disabled"
    };
    rows.push(format!(
        "Controls: {}Cancel  {}Recompute  {}{confirm}",
        focus_marker(form.focus, FormFocus::Cancel),
        focus_marker(form.focus, FormFocus::Recompute),
        focus_marker(form.focus, FormFocus::Confirm)
    ));
    rows
}

fn publication_verdict(preview: &Preview) -> &'static str {
    if preview.stale_actual.is_some() {
        return "STALE";
    }
    match preview.commit.as_ref() {
        Some(CommitStatus::AwaitingReload) => "RELOAD REQUIRED",
        Some(CommitStatus::NotPublished { .. }) => "NOT PUBLISHED",
        Some(CommitStatus::Failed { report, .. }) => match report.state() {
            PublicationState::Unchanged => "FAILED - UNCHANGED",
            PublicationState::Complete => "FAILED - COMPLETE",
            PublicationState::Partial => "RECOVERY REQUIRED - PARTIAL",
            PublicationState::Diverged => "RECOVERY REQUIRED - DIVERGED",
        },
        Some(CommitStatus::Reconciled { report, .. }) => match report.state() {
            PublicationState::Complete => "COMMITTED",
            PublicationState::Unchanged => "NOT PUBLISHED",
            PublicationState::Partial => "RECOVERY REQUIRED - PARTIAL",
            PublicationState::Diverged => "RECOVERY REQUIRED - DIVERGED",
        },
        None => match preview.change.disposition() {
            ChangeDisposition::Ready => "READY",
            ChangeDisposition::NoOp => "NO-OP",
            ChangeDisposition::Denied => "DENIED",
        },
    }
}

fn append_impact(rows: &mut Vec<String>, change: &ChangeSet) {
    let tasks = affected_tasks(change);
    rows.push(format!("Affected tasks ({})", tasks.len()));
    for (index, task) in tasks.iter().enumerate() {
        rows.push(format!("TASK {}/{}: {task}", index + 1, tasks.len()));
    }
    let scale = if is_broad(change) {
        "broad; confirmation requires arming, refocusing, and a second fresh key press"
    } else if change.files().is_empty() {
        "none; confirmation is disabled"
    } else {
        "focused; confirmation requires one fresh key press"
    };
    rows.push(format!(
        "Impact: {} task(s), {} file(s); {scale}",
        tasks.len(),
        change.files().len()
    ));
}

fn is_broad(change: &ChangeSet) -> bool {
    change.files().len() > 1 || affected_tasks(change).len() > 1
}

fn affected_tasks(change: &ChangeSet) -> Vec<String> {
    let mut tasks = BTreeSet::new();
    for effect in change.effects() {
        match effect {
            SemanticEffect::TaskStatusChanged { task, .. }
            | SemanticEffect::DependencyAdded { task, .. }
            | SemanticEffect::DependencyRemoved { task, .. }
            | SemanticEffect::PreferenceAdded { task, .. }
            | SemanticEffect::PreferenceRemoved { task, .. }
            | SemanticEffect::TagAdded { task, .. }
            | SemanticEffect::TagRemoved { task, .. }
            | SemanticEffect::TaskCreated { task, .. }
            | SemanticEffect::TaskPhaseChanged { task, .. }
            | SemanticEffect::TraceFingerprintsRestamped { task, .. }
            | SemanticEffect::TaskReferenceRekeyed { task, .. }
            | SemanticEffect::TaskClosed { task, .. }
            | SemanticEffect::EvidenceAdded { task, .. }
            | SemanticEffect::EvidenceRemoved { task, .. }
            | SemanticEffect::EvidenceReplaced { task, .. }
            | SemanticEffect::EvidenceAndTraceFingerprintsRestamped { task, .. }
            | SemanticEffect::VerificationRepairApplied { task, .. } => {
                tasks.insert(task.clone());
            }
            SemanticEffect::TagRenamed { tasks: renamed, .. }
            | SemanticEffect::CutMembershipAdded { tasks: renamed, .. }
            | SemanticEffect::CutMembershipRemoved { tasks: renamed, .. } => {
                tasks.extend(renamed.iter().cloned());
            }
            SemanticEffect::TaskRekeyed { old, new } => {
                tasks.insert(format!("{old} -> {new}"));
            }
            _ => {}
        }
    }
    tasks.into_iter().collect()
}

fn append_commit_status(rows: &mut Vec<String>, status: Option<&CommitStatus>) {
    let Some(status) = status else {
        return;
    };
    match status {
        CommitStatus::AwaitingReload => rows.push(
            "Publication returned; reload before claiming success. If interrupted now, reload and compare every listed file with this preview."
                .to_string(),
        ),
        CommitStatus::NotPublished { reason } => {
            rows.push(format!("Publication result: {reason}. Form input was preserved."));
        }
        CommitStatus::Failed { error, report } => {
            rows.push(format!("Publication failed: {error}"));
            append_recovery(rows, report);
        }
        CommitStatus::Reconciled {
            input_sha256,
            report,
        } => {
            rows.push(format!("Reloaded input: {input_sha256}"));
            append_recovery(rows, report);
        }
    }
}

fn append_recovery(rows: &mut Vec<String>, report: &PublicationReport) {
    rows.push(format!(
        "Observed publication: {}",
        publication_state_label(report.state())
    ));
    for (index, file) in report.files().iter().enumerate() {
        let source = file.source_path().map_or(String::new(), |source| {
            format!(" from {}", source.display())
        });
        rows.push(format!(
            "OBSERVED {}/{} [{}]: {}{source}",
            index + 1,
            report.files().len(),
            publication_file_state_label(file.state()),
            file.path().display()
        ));
    }
    rows.push(match report.state() {
        PublicationState::Unchanged =>
            "Recovery: no reviewed after-state was observed. Reload current input, review again, then retry."
                .to_string(),
        PublicationState::Complete =>
            "Recovery: every reviewed after-state is present. Reload and do not repeat the mutation."
                .to_string(),
        PublicationState::Partial =>
            "Recovery: do not retry the whole mutation. Keep the COMPLETE paths and either apply the reviewed after bytes to every UNCHANGED path or restore every COMPLETE path to its reviewed before bytes; reload before another write."
                .to_string(),
        PublicationState::Diverged =>
            "Recovery: do not retry or overwrite. Inspect every DIVERGED path against the reviewed before/after bytes, restore one complete consistent state, then reload."
                .to_string(),
    });
}

const fn publication_state_label(state: PublicationState) -> &'static str {
    match state {
        PublicationState::Unchanged => "unchanged",
        PublicationState::Complete => "complete",
        PublicationState::Partial => "partial",
        PublicationState::Diverged => "diverged",
    }
}

const fn publication_file_state_label(state: PublicationFileState) -> &'static str {
    match state {
        PublicationFileState::Unchanged => "UNCHANGED",
        PublicationFileState::Complete => "COMPLETE",
        PublicationFileState::Diverged => "DIVERGED",
    }
}

fn focus_marker(actual: FormFocus, expected: FormFocus) -> &'static str {
    if actual == expected { ">" } else { " " }
}

fn append_files(rows: &mut Vec<String>, change: &ChangeSet) {
    rows.push(format!("Files ({})", change.files().len()));
    for (index, file) in change.files().iter().enumerate() {
        let relative = file
            .path()
            .strip_prefix(&change.input().plan_root)
            .unwrap_or(file.path());
        let operation = if file.source_path().is_some() {
            "relocate"
        } else if file.before().is_some() {
            "replace"
        } else {
            "create"
        };
        rows.push(format!(
            "FILE {}/{} [{}]: {}",
            index + 1,
            change.files().len(),
            operation,
            relative.display()
        ));
        if let Some(source) = file.source_path() {
            let source = source
                .strip_prefix(&change.input().plan_root)
                .unwrap_or(source);
            rows.push(format!("  from: {}", source.display()));
        }
        append_text(rows, "---", file.before());
        append_text(rows, "+++", Some(file.after()));
    }
}

fn append_text(rows: &mut Vec<String>, prefix: &str, content: Option<&str>) {
    let Some(content) = content else {
        rows.push(format!("{prefix} <absent>"));
        return;
    };
    rows.push(format!("{prefix} {} byte(s)", content.len()));
    if content.is_empty() {
        rows.push(format!("{prefix} <empty>"));
        return;
    }
    for (index, line) in content.split_terminator('\n').enumerate() {
        rows.push(format!("{prefix}{:04}|{line}", index + 1));
    }
    rows.push(format!(
        "{prefix} {}",
        if content.ends_with('\n') {
            "ends with newline"
        } else {
            "no newline at EOF"
        }
    ));
}

fn append_effects(rows: &mut Vec<String>, change: &ChangeSet) {
    rows.push(format!("Semantic changes ({})", change.effects().len()));
    for effect in change.effects() {
        let summary = match effect {
            SemanticEffect::DependencyAdded { task, after } => {
                format!("add dependency: {task} after {after}")
            }
            SemanticEffect::DependencyRemoved { task, after } => {
                format!("remove dependency: {task} after {after}")
            }
            SemanticEffect::PreferenceAdded { task, after } => {
                format!("add preference: {task} after {after}")
            }
            SemanticEffect::PreferenceRemoved { task, after } => {
                format!("remove preference: {task} after {after}")
            }
            SemanticEffect::TagAdded { task, tag } => format!("add tag to {task}: {tag}"),
            SemanticEffect::TagRemoved { task, tag } => {
                format!("remove tag from {task}: {tag}")
            }
            SemanticEffect::TagRenamed { from, to, tasks } => {
                format!("rename tag {from} -> {to} on: {}", tasks.join(", "))
            }
            SemanticEffect::TaskCreated { task, title } => {
                format!("create task: {task} ({title})")
            }
            SemanticEffect::CutMembershipAdded { cut, tasks } => {
                format!("add to cut {cut}: {}", tasks.join(", "))
            }
            SemanticEffect::CutMembershipRemoved { cut, tasks } => {
                format!("remove from cut {cut}: {}", tasks.join(", "))
            }
            SemanticEffect::TaskPhaseChanged { task, from, to } => {
                format!("change phase for {task}: {from} -> {to}")
            }
            SemanticEffect::TraceFingerprintsRestamped { task, count } => {
                format!("re-stamp {count} trace fingerprint(s): {task}")
            }
            SemanticEffect::TaskRekeyed { old, new } => {
                format!("rekey task: {old} -> {new}")
            }
            SemanticEffect::TaskReferenceRekeyed { task, old, new } => {
                format!("rekey reference in {task}: {old} -> {new}")
            }
            SemanticEffect::PolicyReferenceRekeyed { path, old, new } => {
                format!(
                    "rekey policy reference in {}: {old} -> {new}",
                    path.display()
                )
            }
            SemanticEffect::ImportNotice {
                source,
                kind,
                message,
            } => format!("import {kind} from {source}: {message}"),
            SemanticEffect::TaskClosed { task, evidence } => {
                format!("close {task} with {} citation(s)", evidence.len())
            }
            SemanticEffect::EvidenceAdded { task, evidence } => format!(
                "add evidence to {task}: {}:{}",
                evidence.kind.as_str(),
                evidence.reference
            ),
            SemanticEffect::EvidenceRemoved { task, evidence } => format!(
                "remove evidence from {task}: {}:{}",
                evidence.kind.as_str(),
                evidence.reference
            ),
            SemanticEffect::EvidenceReplaced { task, from, to }
            | SemanticEffect::VerificationRepairApplied { task, from, to } => format!(
                "replace evidence on {task}: {}:{} -> {}:{}",
                from.kind.as_str(),
                from.reference,
                to.kind.as_str(),
                to.reference
            ),
            SemanticEffect::EvidenceAndTraceFingerprintsRestamped {
                task,
                trace_changes,
                evidence_changes,
            } => format!(
                "restamp {task}: {trace_changes} trace and {evidence_changes} evidence change(s)"
            ),
            other => format!("{other:?}"),
        };
        rows.push(format!("  {summary}"));
    }
}

fn append_findings(rows: &mut Vec<String>, change: &ChangeSet) {
    let findings = &change.diagnostics().added;
    rows.push(format!("Findings ({})", findings.len()));
    for finding in findings {
        let task = finding
            .task
            .as_deref()
            .map_or(String::new(), |task| format!(" {task}"));
        rows.push(format!(
            "  {} {}{task}: {}",
            finding.severity, finding.code, finding.message
        ));
        if let Some(instruction) = &finding.agent_instruction {
            rows.push(format!("    fix: {instruction}"));
        }
        rows.extend(
            finding
                .detail
                .iter()
                .map(|detail| format!("    detail: {detail}")),
        );
    }
}

fn append_evidence(rows: &mut Vec<String>, change: &ChangeSet) {
    let review = change.evidence_review();
    append_evidence_requirement(rows, review.requirement.as_ref());
    match change.evidence() {
        ChangeEvidence::NotRequired => rows.push("Evidence: not required".to_string()),
        ChangeEvidence::Resolution {
            unresolved,
            observed,
        } => {
            rows.push(format!(
                "Evidence: {} observed; {} unresolved",
                observed.len(),
                unresolved.len()
            ));
            for citation in observed {
                rows.push(format!(
                    "  {} {}:{} - {} via {} ({})",
                    citation.task,
                    citation.kind.as_str(),
                    citation.reference,
                    citation.resolution.outcome.label(),
                    citation.resolution.resolver,
                    citation.resolution.strength.label()
                ));
                if let Some(detail) = &citation.resolution.detail {
                    rows.push(format!("    detail: {detail}"));
                }
            }
            for citation in unresolved {
                rows.push(format!(
                    "  unresolved {}:{}",
                    citation.kind.as_str(),
                    citation.reference
                ));
            }
        }
        ChangeEvidence::Skipped { citations, reason } => {
            rows.push(format!(
                "Evidence: resolution explicitly skipped for {} citation(s)",
                citations.len()
            ));
            rows.push(format!("  reason: {reason}"));
            for citation in citations {
                rows.push(format!(
                    "  unchecked {}:{}",
                    citation.kind.as_str(),
                    citation.reference
                ));
            }
        }
        other => rows.push(format!("Evidence: {other:?}")),
    }
    append_evidence_findings(rows, &review.findings);
    for notice in &review.notices {
        rows.push(format!("Evidence notice: {notice}"));
    }
}

fn append_evidence_requirement(
    rows: &mut Vec<String>,
    requirement: Option<&aplan_app::EvidenceRequirement>,
) {
    let Some(requirement) = requirement else {
        return;
    };
    let required = match &requirement.required {
        None => "not configured".to_string(),
        Some(kinds) if kinds.is_empty() => "explicitly none".to_string(),
        Some(kinds) => kinds
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    };
    rows.push(format!("Evidence policy: required kinds: {required}"));
    rows.push(format!(
        "  scope: cut {}; milestone {}",
        requirement.cut.as_deref().unwrap_or("<none>"),
        requirement.milestone.as_deref().unwrap_or("<none>")
    ));
    rows.push(format!(
        "  exempt tags: {}",
        list_or_none(&requirement.exempt_tags)
    ));
    rows.push(format!(
        "  matched exemptions: {}",
        list_or_none(&requirement.matched_exemptions)
    ));
}

fn list_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "<none>".to_string()
    } else {
        values.join(", ")
    }
}

fn append_evidence_findings(rows: &mut Vec<String>, findings: &[aplan_core::Diagnostic]) {
    if findings.is_empty() {
        return;
    }
    rows.push(format!("Evidence resolver findings ({})", findings.len()));
    for finding in findings {
        let task = finding
            .task
            .as_deref()
            .map_or(String::new(), |task| format!(" {task}"));
        rows.push(format!(
            "  {} {}{task}: {}",
            finding.severity, finding.code, finding.message
        ));
        if let Some(instruction) = &finding.agent_instruction {
            rows.push(format!("    fix: {instruction}"));
        }
        rows.extend(
            finding
                .detail
                .iter()
                .map(|detail| format!("    detail: {detail}")),
        );
    }
}

/// A vertically windowed, width-wrapped form representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedForm {
    pub rows: Vec<String>,
    pub first_row: usize,
    pub last_row: usize,
    pub total_rows: usize,
    pub hidden_above: usize,
    pub hidden_below: usize,
}

impl RenderedForm {
    pub(crate) fn window(source: &[String], width: usize, height: usize, requested: usize) -> Self {
        let wrapped = source
            .iter()
            .flat_map(|row| text::wrap(row, width))
            .collect::<Vec<_>>();
        let total_rows = wrapped.len();
        let needs_disclosure = total_rows > height;
        let capacity = height.saturating_sub(usize::from(needs_disclosure));
        let max_start = total_rows.saturating_sub(capacity);
        let first_row = requested.min(max_start);
        let last_row = first_row.saturating_add(capacity).min(total_rows);
        let hidden_above = first_row;
        let hidden_below = total_rows.saturating_sub(last_row);
        let mut rows = wrapped[first_row..last_row].to_vec();
        if needs_disclosure && height > 0 {
            rows.push(if capacity == 0 {
                format!("Rows 0/{total_rows}")
            } else {
                format!(
                    "Rows {}-{last_row}/{total_rows}",
                    first_row.saturating_add(1).min(total_rows)
                )
            });
        }
        Self {
            rows,
            first_row,
            last_row,
            total_rows,
            hidden_above,
            hidden_below,
        }
    }

    pub fn snapshot(&self) -> String {
        self.rows.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use aplan_app::{
        CommitOutcome, ContextInputs, DependencyOperation, DependencyRequest, PlanContext, Trust,
        commit_change_set, preview_dependency,
    };
    use unicode_segmentation::UnicodeSegmentation as _;

    use super::*;

    #[test]
    fn field_metadata_accessors_preserve_the_complete_definition() {
        let field = FieldSpec::new("paths", "Paths", "Owned repository globs")
            .required()
            .repeated()
            .initial_values(["src/**", "tests/**"]);
        assert_eq!(field.key(), "paths");
        assert_eq!(field.label(), "Paths");
        assert_eq!(field.help(), "Owned repository globs");
        assert!(field.is_required());
        assert_eq!(field.cardinality(), Cardinality::Many);
        assert_eq!(field.initial_values, ["src/**", "tests/**"]);
    }

    fn plan() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path().join("aplan/tasks");
        std::fs::create_dir_all(&root).expect("tasks");
        std::fs::write(
            root.join("A-1.md"),
            "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\neffort_days: 1\n---\n",
        )
        .expect("first");
        std::fs::write(
            root.join("A-2.md"),
            "---\nid: A-2\ntitle: second\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [A-1]\n---\n",
        )
        .expect("second");
        dir
    }

    fn broad_plan() -> tempfile::TempDir {
        let dir = plan();
        for name in ["A-1.md", "A-2.md"] {
            let path = dir.path().join("aplan/tasks").join(name);
            let content = std::fs::read_to_string(&path).expect("task");
            let content = content.replace("status: todo\n", "status: todo\ntags: [old]\n");
            std::fs::write(path, content).expect("tag task");
        }
        std::fs::write(
            dir.path().join("aplan/tasks/M-1.md"),
            "---\nid: M-1\ntitle: first cut\nphase: 1\nstatus: todo\nblocked_by: [A-1, A-2]\ntags: [milestone, v1]\n---\n",
        )
        .expect("milestone");
        dir
    }

    fn context(root: &Path) -> PlanContext {
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

    fn repository_context(root: &Path) -> PlanContext {
        let output = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(root)
            .output()
            .expect("git init");
        assert!(output.status.success());
        for args in [
            ["config", "user.email", "test@example.com"].as_slice(),
            ["config", "user.name", "Test"].as_slice(),
            ["add", "."].as_slice(),
            ["commit", "-qm", "fixture"].as_slice(),
        ] {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .expect("git fixture command");
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        PlanContext::new(
            aplan_core::load_plan_identified(&root.join("aplan")),
            ContextInputs {
                repository_root: root.to_path_buf(),
                configuration_file: None,
                trust: Trust::RepositoryRead,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
    }

    fn form() -> MutationForm {
        bound_form("test-input")
    }

    fn bound_form(input_sha256: &str) -> MutationForm {
        MutationForm::new(
            "Edit task",
            input_sha256,
            false,
            vec![
                FieldSpec::new("title", "Title", "What needs doing").required(),
                FieldSpec::new("ids", "Dependencies", "Task ids").repeated(),
                FieldSpec::new("tags", "Tags", "Classification").repeated(),
                FieldSpec::new("paths", "Paths", "Owned files").repeated(),
                FieldSpec::new("traces", "Traces", "Design facts").repeated(),
                FieldSpec::new("evidence", "Evidence", "Proof citations").repeated(),
            ],
        )
        .expect("form")
    }

    fn insert(form: &mut MutationForm, value: &str) {
        assert!(matches!(
            form.reduce(FormAction::Insert(value.to_string())),
            FormTransition::None
        ));
    }

    fn request(form: &mut MutationForm) -> u64 {
        if form.fields[0].values().next() == Some("") {
            insert(form, "A complete title");
        }
        let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
            panic!("complete form did not request a preview")
        };
        submission.request()
    }

    fn rendered_scope(context: &PlanContext, change: ChangeSet) -> String {
        let mut form = bound_form(&context.identity().sha256);
        let request = request(&mut form);
        assert!(form.accept_preview(request, Ok(change)));
        form.render(180, 400).snapshot()
    }

    fn assert_scope_lists(context: &PlanContext, change: ChangeSet, expected_tasks: &[&str]) {
        let mut expected_task_ids = expected_tasks
            .iter()
            .map(|task| (*task).to_string())
            .collect::<Vec<_>>();
        expected_task_ids.sort();
        assert_eq!(
            affected_tasks(&change),
            expected_task_ids,
            "the semantic impact projection itself omitted or invented a task"
        );
        let file_paths = change
            .files()
            .iter()
            .map(|file| {
                file.path()
                    .strip_prefix(&change.input().plan_root)
                    .unwrap_or(file.path())
                    .display()
                    .to_string()
            })
            .collect::<Vec<_>>();
        let rendered = rendered_scope(context, change);
        assert!(
            rendered.contains(&format!("Affected tasks ({})", expected_tasks.len())),
            "{rendered}"
        );
        for task in expected_tasks {
            assert!(rendered.contains(&format!(": {task}")), "{rendered}");
        }
        for path in file_paths {
            assert!(rendered.contains(&format!(": {path}")), "{rendered}");
        }
        assert!(rendered.contains("Arm broad change"), "{rendered}");
    }

    fn confirm_broad_change(form: &mut MutationForm) -> Box<ChangeSet> {
        form.reduce(FormAction::PreviousFocus);
        assert_eq!(form.focus(), FormFocus::Confirm);
        assert!(matches!(
            form.reduce(FormAction::Confirm(Activation::from_key_kind(
                KeyEventKind::Press,
            ))),
            FormTransition::None
        ));
        assert!(form.change_set().is_some());
        assert_eq!(form.focus(), FormFocus::Cancel);
        assert!(
            form.render(120, 200)
                .snapshot()
                .contains("Confirm broad change")
        );
        assert!(matches!(
            form.reduce(FormAction::Confirm(Activation::from_key_kind(
                KeyEventKind::Repeat,
            ))),
            FormTransition::None
        ));
        form.reduce(FormAction::PreviousFocus);
        assert_eq!(form.focus(), FormFocus::Confirm);
        let FormTransition::CommitRequested(change) = form.reduce(FormAction::Confirm(
            Activation::from_key_kind(KeyEventKind::Press),
        )) else {
            panic!("second fresh press did not request the broad commit")
        };
        change
    }

    fn broad_change(context: &PlanContext) -> ChangeSet {
        aplan_app::preview_planning(
            context,
            &aplan_app::PlanningRequest::Tag(aplan_app::TagRequest::Rename {
                from: "old".to_string(),
                to: "new".to_string(),
            }),
        )
        .expect("broad rename")
        .into_change_set()
    }

    fn render_failed_publication(state: PublicationState) -> String {
        let dir = broad_plan();
        let context = context(dir.path());
        let change = broad_change(&context);
        match state {
            PublicationState::Unchanged => {}
            PublicationState::Complete => {
                for file in change.files() {
                    std::fs::write(file.path(), file.after()).expect("publish reviewed bytes");
                }
            }
            PublicationState::Diverged => {
                std::fs::write(change.files()[0].path(), "third-party bytes\n")
                    .expect("diverge first path");
            }
            PublicationState::Partial => panic!("partial has its own fault-injection journey"),
        }
        let report = observe_publication(&change);
        assert_eq!(report.state(), state);
        let mut form = bound_form(&context.identity().sha256);
        let _ = request(&mut form);
        form.accept_commit_outcome(CommitOutcome::PublicationFailed {
            error: aplan_core::save::SaveError::Io {
                path: change.files()[0].path().to_path_buf(),
                source: std::io::Error::other("injected publication failure"),
            },
            change,
            report,
        });
        form.render(180, 400).snapshot()
    }

    fn stable_snapshot(output: &str) -> String {
        let mut normalized = Vec::new();
        let mut digest_continuation = false;
        for line in output.lines() {
            if line.starts_with("Preview input: ") {
                normalized.push("Preview input: <input-digest>".to_string());
                digest_continuation = true;
            } else if digest_continuation
                && !line.is_empty()
                && line.chars().all(|character| character.is_ascii_hexdigit())
            {
                digest_continuation = false;
            } else {
                digest_continuation = false;
                normalized.push(line.replace('\\', "/"));
            }
        }
        normalized.join("\n")
    }

    #[test]
    fn selector_prefill_replaces_values_without_bypassing_form_rules() {
        let mut form = form();
        form.prefill("ids", ["A-1", "A-2"])
            .expect("repeatable selector values");
        let ids = form
            .fields()
            .iter()
            .find(|field| field.spec().key() == "ids")
            .expect("ids field");
        assert_eq!(ids.values().collect::<Vec<_>>(), vec!["A-1", "A-2"]);
        assert_eq!(form.focus(), FormFocus::Field(1));

        form.prefill("ids", std::iter::empty::<String>())
            .expect("empty selection restores one editable row");
        let ids = form
            .fields()
            .iter()
            .find(|field| field.spec().key() == "ids")
            .expect("ids field");
        assert_eq!(ids.values().collect::<Vec<_>>(), vec![""]);

        assert!(matches!(
            form.prefill("title", ["one", "two"]),
            Err(DefinitionError::TooManyInitialValues(key)) if key == "title"
        ));
        assert!(matches!(
            form.prefill("missing", ["value"]),
            Err(DefinitionError::UnknownKey(key)) if key == "missing"
        ));
    }

    #[test]
    fn forms_edit_unicode_and_repeated_values_with_predictable_focus() {
        let mut form = form();
        insert(&mut form, "Cafe\u{301} plan");
        assert_eq!(form.fields[0].values().next(), Some("Cafe\u{301} plan"));
        assert_eq!(form.fields[0].editors[0].cursor, "Cafe\u{301} plan".len());
        form.reduce(FormAction::CursorHome);
        for _ in 0..4 {
            form.reduce(FormAction::CursorRight);
        }
        form.reduce(FormAction::Backspace);
        assert_eq!(
            form.fields[0].values().next(),
            Some("Caf plan"),
            "backspace must remove one extended grapheme"
        );

        for expected_key in ["ids", "tags", "paths", "traces", "evidence"] {
            form.reduce(FormAction::NextFocus);
            let FormFocus::Field(index) = form.focus() else {
                panic!("focus left the fields before `{expected_key}`")
            };
            assert_eq!(form.fields[index].spec.key, expected_key);
            insert(&mut form, &format!("{expected_key}-one"));
            form.reduce(FormAction::AddItem);
            insert(&mut form, &format!("{expected_key}-two"));
            form.reduce(FormAction::MoveItemUp);
            assert_eq!(
                form.fields[index].values().collect::<Vec<_>>(),
                [format!("{expected_key}-two"), format!("{expected_key}-one")]
            );
            form.reduce(FormAction::RemoveItem);
            assert_eq!(
                form.fields[index].values().collect::<Vec<_>>(),
                [format!("{expected_key}-one")]
            );
        }
        form.reduce(FormAction::NextFocus);
        assert_eq!(form.focus(), FormFocus::Cancel);
        form.reduce(FormAction::NextFocus);
        assert_eq!(form.focus(), FormFocus::Review);
        form.reduce(FormAction::NextFocus);
        assert_eq!(form.focus(), FormFocus::Field(0));
        assert_eq!("Caf plan".graphemes(true).count(), 8);
    }

    #[test]
    fn definitions_reject_ambiguous_fields_and_preserve_initial_values() {
        let defined_form = MutationForm::new(
            "Move task",
            "snapshot",
            false,
            vec![
                FieldSpec::new("cut", "Cut", "Destination")
                    .required()
                    .initial_value("M-next"),
                FieldSpec::new("tags", "Tags", "Classification")
                    .repeated()
                    .initial_values(["area:tui", "priority:high"]),
            ],
        )
        .expect("valid prepopulated form");
        assert_eq!(
            defined_form.fields[0].values().collect::<Vec<_>>(),
            ["M-next"]
        );
        assert_eq!(
            defined_form.fields[1].values().collect::<Vec<_>>(),
            ["area:tui", "priority:high"]
        );

        assert!(matches!(
            MutationForm::new(
                "Bad",
                "snapshot",
                false,
                vec![FieldSpec::new("owner", "Owner", "one").initial_values(["a", "b"])]
            ),
            Err(DefinitionError::TooManyInitialValues(key)) if key == "owner"
        ));
        assert!(matches!(
            MutationForm::new(
                "Bad",
                "snapshot",
                false,
                vec![
                    FieldSpec::new("tag", "Tag", "first"),
                    FieldSpec::new("tag", "Tag", "second"),
                ]
            ),
            Err(DefinitionError::DuplicateKey(key)) if key == "tag"
        ));

        let mut form = form();
        form.set_local_issue(
            "paths",
            LocalIssue::Invalid {
                message: "path must stay inside the plan".to_string(),
            },
        )
        .expect("known field");
        assert_eq!(form.focus(), FormFocus::Field(3));
        assert!(matches!(
            form.fields[3].issue(),
            Some(LocalIssue::Invalid { message }) if message.contains("inside")
        ));
    }

    #[test]
    fn definitions_reject_every_vacuous_and_unknown_shape() {
        let one_field = || vec![FieldSpec::new("tag", "Tag", "classification")];
        assert!(matches!(
            MutationForm::new("  ", "snapshot", false, one_field()),
            Err(DefinitionError::BlankTitle)
        ));
        assert!(matches!(
            MutationForm::new("Bad", "\t", false, one_field()),
            Err(DefinitionError::BlankInput)
        ));
        assert!(matches!(
            MutationForm::new("Bad", "snapshot", false, vec![]),
            Err(DefinitionError::NoFields)
        ));
        assert!(matches!(
            MutationForm::new(
                "Bad",
                "snapshot",
                false,
                vec![FieldSpec::new(" ", "Tag", "classification")]
            ),
            Err(DefinitionError::BlankKey)
        ));
        assert!(matches!(
            MutationForm::new(
                "Bad",
                "snapshot",
                false,
                vec![FieldSpec::new(" tag", "Tag", "classification")]
            ),
            Err(DefinitionError::UntrimmedKey(key)) if key == " tag"
        ));
        assert!(matches!(
            MutationForm::new(
                "Bad",
                "snapshot",
                false,
                vec![FieldSpec::new("tag", "\n", "classification")]
            ),
            Err(DefinitionError::BlankLabel(key)) if key == "tag"
        ));

        let mut form = form();
        assert!(matches!(
            form.set_local_issue("missing", LocalIssue::Required),
            Err(DefinitionError::UnknownKey(key)) if key == "missing"
        ));
    }

    #[test]
    fn form_snapshots_distinguish_local_domain_ready_noop_denied_and_stale_states() {
        let mut output = String::new();

        let mut local = form();
        local.reduce(FormAction::NextFocus);
        insert(&mut local, "A-1");
        local.reduce(FormAction::AddItem);
        assert!(matches!(
            local.reduce(FormAction::Review),
            FormTransition::None
        ));
        output.push_str("== LOCAL PARTIAL ==\n");
        output.push_str(&local.render(72, 40).snapshot());

        let mut domain = form();
        let domain_request = request(&mut domain);
        assert!(domain.accept_preview(
            domain_request,
            Err(DomainIssue::new(
                "typed service refused the request",
                vec!["the selected task no longer exists".to_string()],
            ))
        ));
        output.push_str("\n== DOMAIN INVALID ==\n");
        output.push_str(&domain.render(72, 40).snapshot());

        let dir = plan();
        let context = context(dir.path());
        let ready = preview_dependency(
            &context,
            &DependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Remove,
            },
        )
        .expect("ready preview")
        .into_change_set();
        let mut accepted = bound_form(&context.identity().sha256);
        let accepted_request = request(&mut accepted);
        assert!(accepted.accept_preview(accepted_request, Ok(ready)));
        assert!(
            accepted
                .render(100, 200)
                .snapshot()
                .contains(&context.identity().sha256)
        );
        output.push_str("\n== ACCEPTED ==\n");
        output.push_str(&accepted.render(72, 200).snapshot());

        let noop_change = preview_dependency(
            &context,
            &DependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Add,
            },
        )
        .expect("no-op preview")
        .into_change_set();
        let mut noop_form = bound_form(&context.identity().sha256);
        let no_op_request = request(&mut noop_form);
        assert!(noop_form.accept_preview(no_op_request, Ok(noop_change)));
        output.push_str("\n== NO-OP ==\n");
        output.push_str(&noop_form.render(72, 40).snapshot());

        let denied = preview_dependency(
            &context,
            &DependencyRequest {
                id: "A-1".to_string(),
                after: "A-2".to_string(),
                operation: DependencyOperation::Add,
            },
        )
        .expect("denied preview")
        .into_change_set();
        let mut denied_form = bound_form(&context.identity().sha256);
        let denied_request = request(&mut denied_form);
        assert!(denied_form.accept_preview(denied_request, Ok(denied)));
        assert_eq!(
            denied_form.fields()[0].values().next(),
            Some("A complete title")
        );
        output.push_str("\n== DENIED ==\n");
        output.push_str(&denied_form.render(72, 40).snapshot());

        let stale = preview_dependency(
            &context,
            &DependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Remove,
            },
        )
        .expect("stale preview")
        .into_change_set();
        let mut stale_form = form();
        stale_form.accept_stale(stale, "new-input-digest");
        output.push_str("\n== STALE WINDOW ==\n");
        let rendered = stale_form.render(48, 10);
        assert!(rendered.hidden_below > 0);
        output.push_str(&rendered.snapshot());

        let output = stable_snapshot(&output.replace('\u{2192}', "->"));
        assert_eq!(
            output,
            stable_snapshot(include_str!("snapshots/form-states-ascii.txt"))
        );
    }

    #[test]
    fn confirmation_requires_a_fresh_press_on_the_focused_control() {
        let dir = plan();
        let context = context(dir.path());
        let change = || {
            preview_dependency(
                &context,
                &DependencyRequest {
                    id: "A-2".to_string(),
                    after: "A-1".to_string(),
                    operation: DependencyOperation::Remove,
                },
            )
            .expect("preview")
            .into_change_set()
        };
        let mut ready_form = bound_form(&context.identity().sha256);
        let preview_request = request(&mut ready_form);
        assert!(ready_form.accept_preview(preview_request, Ok(change())));
        let target = dir.path().join("aplan/tasks/A-2.md");
        let before = std::fs::read_to_string(&target).expect("before confirmation");
        let after = ready_form.change_set().expect("reviewed change").files()[0]
            .after()
            .to_string();
        assert_eq!(ready_form.focus(), FormFocus::Cancel);
        assert!(matches!(
            ready_form.reduce(FormAction::Confirm(Activation::from_key_kind(
                KeyEventKind::Press,
            ))),
            FormTransition::None
        ));
        ready_form.reduce(FormAction::PreviousFocus);
        assert_eq!(ready_form.focus(), FormFocus::Confirm);
        for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
            assert!(matches!(
                ready_form.reduce(FormAction::Confirm(Activation::from_key_kind(kind))),
                FormTransition::None
            ));
            assert!(ready_form.change_set().is_some());
        }
        assert_eq!(
            std::fs::read_to_string(&target).expect("before fresh press"),
            before,
            "opening and navigating the preview must not write"
        );
        let FormTransition::CommitRequested(committed_change) = ready_form.reduce(
            FormAction::Confirm(Activation::from_key_kind(KeyEventKind::Press)),
        ) else {
            panic!("fresh press on focused confirmation did not yield the reviewed change")
        };
        assert!(ready_form.change_set().is_none());
        assert!(matches!(
            commit_change_set(&context, *committed_change),
            CommitOutcome::Applied(_)
        ));
        assert_eq!(
            std::fs::read_to_string(&target).expect("after confirmation"),
            after,
            "confirmation did not publish the exact reviewed bytes"
        );

        let mut stale = form();
        let _ = request(&mut stale);
        stale.accept_stale(change(), "changed");
        stale.reduce(FormAction::PreviousFocus);
        assert_eq!(stale.focus(), FormFocus::Confirm);
        assert!(matches!(
            stale.reduce(FormAction::Confirm(Activation::from_key_kind(
                KeyEventKind::Press,
            ))),
            FormTransition::None
        ));
        let FormTransition::PreviewRequested(recomputed) = stale.reduce(FormAction::Recompute)
        else {
            panic!("stale preview could not request recomputation")
        };
        assert_eq!(recomputed.input_sha256(), "changed");
    }

    /// Proof for `tui.overlay_pointer_activation_is_semantic_and_guarded`:
    /// pointer confirmation enters through the same freshness and broad-change
    /// arm boundary as keyboard confirmation, without depending on prior focus.
    #[test]
    fn pointer_confirmation_preserves_fresh_press_and_broad_change_arming() {
        let dir = broad_plan();
        let context = context(dir.path());
        let mut form = bound_form(&context.identity().sha256);
        let preview_request = request(&mut form);
        assert!(form.accept_preview(preview_request, Ok(broad_change(&context))));
        assert_eq!(form.focus(), FormFocus::Cancel);
        assert_eq!(form.confirm_label(), "Arm broad change");
        assert_eq!(form.broad_confirmation_armed(), Some(false));

        for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
            assert!(matches!(
                form.reduce(FormAction::PointerConfirm(Activation::from_key_kind(kind))),
                FormTransition::None
            ));
            assert_eq!(form.focus(), FormFocus::Cancel);
            assert_eq!(form.confirm_label(), "Arm broad change");
        }

        assert!(matches!(
            form.reduce(FormAction::PointerConfirm(Activation::from_key_kind(
                KeyEventKind::Press,
            ))),
            FormTransition::None
        ));
        assert_eq!(form.focus(), FormFocus::Cancel);
        assert_eq!(form.confirm_label(), "Confirm broad change");
        assert_eq!(form.broad_confirmation_armed(), Some(true));
        assert!(form.change_set().is_some());

        let FormTransition::CommitRequested(change) = form.reduce(FormAction::PointerConfirm(
            Activation::from_key_kind(KeyEventKind::Press),
        )) else {
            panic!("second fresh pointer press did not yield the reviewed broad change")
        };
        assert_eq!(change.files().len(), 2);
        assert!(form.change_set().is_none());
    }

    #[test]
    fn every_clipped_preview_discloses_the_complete_row_range() {
        let dir = plan();
        let context = context(dir.path());
        let change = preview_dependency(
            &context,
            &DependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Remove,
            },
        )
        .expect("preview")
        .into_change_set();
        let mut form = bound_form(&context.identity().sha256);
        let request = request(&mut form);
        assert!(form.accept_preview(request, Ok(change)));

        let top = form.render(32, 6);
        assert_eq!(top.rows.len(), 6);
        assert_eq!(top.hidden_above, 0);
        assert!(top.hidden_below > 0);
        assert!(top.rows.last().is_some_and(|row| row.contains('/')));

        let disclosure_only = form.render(32, 1);
        assert_eq!(disclosure_only.rows, [format!("Rows 0/{}", top.total_rows)]);
        assert_eq!(disclosure_only.first_row, disclosure_only.last_row);
        assert_eq!(
            disclosure_only.hidden_above + disclosure_only.hidden_below,
            disclosure_only.total_rows,
            "a disclosure-only viewport must not claim that it rendered a content row"
        );

        form.reduce(FormAction::Scroll {
            delta: i16::MAX,
            from: top.first_row,
        });
        let bottom = form.render(32, 6);
        assert!(bottom.hidden_above > 0);
        assert_eq!(bottom.hidden_below, 0);
        assert!(bottom.rows.last().is_some_and(|row| row.contains('/')));
    }

    /// Proof for `tui.shell_adapts_by_route_and_available_rows`: a short form
    /// follows the exact wrapped cursor row, pointer geometry uses that same
    /// viewport, and an explicit page starts from the rows actually painted.
    #[test]
    fn short_form_viewport_follows_cursor_and_manual_scroll_starts_from_the_painted_rows() {
        let mut form = MutationForm::new(
            "Responsive edit",
            "input",
            false,
            vec![
                FieldSpec::new("title", "Title", "required value")
                    .required()
                    .initial_value(format!("alpha-{}-omega", "\u{754c}e\u{301}".repeat(8))),
            ],
        )
        .expect("form")
        .with_guidance((0..6).map(|index| format!("context {index}")).collect());
        let width = 12;
        let height = 5;

        let at_end = form.render(width, height);
        let cursor_row = at_end
            .rows
            .iter()
            .position(|row| row.contains('|'))
            .expect("the wrapped cursor is visible at the end");
        assert!(
            form.pointer_rows(width, height)
                .contains(&(cursor_row, FormAction::FocusItem { field: 0, item: 0 }))
        );

        form.reduce(FormAction::CursorHome);
        let at_start = form.render(width, height);
        assert!(at_start.snapshot().contains("* 1: |"), "{at_start:?}");
        assert!(
            at_start.first_row < at_end.first_row,
            "moving the cursor home must reveal its earlier wrapped row"
        );

        form.reduce(FormAction::Scroll {
            delta: 2,
            from: at_start.first_row,
        });
        let manual = form.render(width, height);
        assert_eq!(manual.first_row, at_start.first_row + 2);

        form.reduce(FormAction::CursorHome);
        assert_eq!(form.render(width, height).first_row, at_start.first_row);
    }

    #[test]
    fn an_old_preview_response_cannot_replace_newer_form_input() {
        let dir = plan();
        let context = context(dir.path());
        let change = preview_dependency(
            &context,
            &DependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Remove,
            },
        )
        .expect("preview")
        .into_change_set();
        let mut form = bound_form(&context.identity().sha256);
        let old = request(&mut form);
        form.focus = FormFocus::Field(0);
        insert(&mut form, " changed");
        assert!(!form.accept_preview(old, Ok(change)));
        assert!(form.change_set().is_none());
        assert!(matches!(
            form.reduce(FormAction::Review),
            FormTransition::PreviewRequested(_)
        ));
    }

    #[test]
    fn supplemental_review_rows_are_request_bound_sanitized_and_invalidated() {
        let dir = plan();
        let context = context(dir.path());
        let change = preview_dependency(
            &context,
            &DependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Remove,
            },
        )
        .expect("preview")
        .into_change_set();
        let mut form = bound_form(&context.identity().sha256);
        let request = request(&mut form);
        assert!(form.accept_preview_with_rows(
            request,
            Ok(change),
            vec!["selected task: A-2\u{1b}[2J".to_string()],
        ));
        let rendered = form.render(120, 200).snapshot();
        assert!(rendered.contains("Operation review (1)"), "{rendered}");
        assert!(rendered.contains("selected task: A-2"), "{rendered}");
        assert!(rendered.contains("[2J"), "{rendered}");
        assert!(!rendered.contains('\u{1b}'), "{rendered:?}");

        form.focus = FormFocus::Field(0);
        insert(&mut form, " changed");
        let rendered = form.render(120, 200).snapshot();
        assert!(!rendered.contains("selected task: A-2"), "{rendered}");
        assert!(form.change_set().is_none());
        assert!(!form.accept_preview_with_rows(
            request,
            Err(DomainIssue::new("late", Vec::new())),
            vec!["stale context".to_string()],
        ));
        assert!(!form.render(120, 200).snapshot().contains("stale context"));
    }

    #[test]
    fn a_preview_for_another_input_is_inspectable_but_never_confirmable() {
        let dir = plan();
        let context = context(dir.path());
        let change = preview_dependency(
            &context,
            &DependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Remove,
            },
        )
        .expect("preview")
        .into_change_set();
        let mut form = form();
        let request = request(&mut form);
        assert!(form.accept_preview(request, Ok(change)));
        assert!(form.render(100, 20).snapshot().contains("Verdict: STALE"));
        form.reduce(FormAction::PreviousFocus);
        assert_eq!(form.focus(), FormFocus::Confirm);
        assert!(matches!(
            form.reduce(FormAction::Confirm(Activation::from_key_kind(
                KeyEventKind::Press,
            ))),
            FormTransition::None
        ));
    }

    /// Proof for `tui.broad_mutations_are_recoverable`: a broad confirmation
    /// enumerates every task and file, needs two distinct presses, and a late
    /// injected failure retains both form input and per-path recovery guidance.
    #[test]
    fn broad_confirmation_and_partial_failure_name_every_path_and_recovery_step() {
        let dir = broad_plan();
        let context = context(dir.path());
        let preview = aplan_app::preview_planning(
            &context,
            &aplan_app::PlanningRequest::Tag(aplan_app::TagRequest::Rename {
                from: "old".to_string(),
                to: "new".to_string(),
            }),
        )
        .expect("broad rename");
        let mut form = bound_form(&context.identity().sha256);
        let request = request(&mut form);
        assert!(form.accept_preview(request, Ok(preview.into_change_set())));
        let review = form.render(120, 200).snapshot();
        assert!(review.contains("Affected tasks (2)"), "{review}");
        assert!(review.contains("TASK 1/2: A-1"), "{review}");
        assert!(review.contains("TASK 2/2: A-2"), "{review}");
        for (index, task) in ["A-1.md", "A-2.md"].into_iter().enumerate() {
            let path = Path::new("tasks").join(task);
            assert!(
                review.contains(&format!(
                    "FILE {}/2 [replace]: {}",
                    index + 1,
                    path.display()
                )),
                "{review}"
            );
        }
        assert!(review.contains("Arm broad change"), "{review}");

        let change = confirm_broad_change(&mut form);

        let interrupted_preview = observe_publication(&change);
        assert_eq!(interrupted_preview.state(), PublicationState::Unchanged);
        assert!(
            interrupted_preview
                .files()
                .iter()
                .all(|file| file.state() == PublicationFileState::Unchanged),
            "an interruption before publication changed reviewed files"
        );

        let first = &change.files()[0];
        std::fs::write(first.path(), first.after()).expect("inject first rename");
        let report = observe_publication(&change);
        assert_eq!(report.state(), PublicationState::Partial);
        assert_eq!(
            std::fs::read_to_string(change.files()[0].path()).expect("first"),
            change.files()[0].after()
        );
        assert_eq!(
            std::fs::read_to_string(change.files()[1].path()).expect("second"),
            change.files()[1].before().expect("second before")
        );
        form.accept_commit_outcome(CommitOutcome::PublicationFailed {
            error: aplan_core::save::SaveError::Io {
                path: change.files()[1].path().to_path_buf(),
                source: std::io::Error::other("injected between renames"),
            },
            change: *change,
            report,
        });
        assert_eq!(
            form.commit_status()
                .and_then(CommitStatus::report)
                .map(PublicationReport::state),
            Some(PublicationState::Partial)
        );
        assert_eq!(
            form.fields()[0].values().next(),
            Some("A complete title"),
            "failed publication discarded the submitted form"
        );
        let failure = form.render(160, 300).snapshot();
        assert!(
            failure.contains("Verdict: RECOVERY REQUIRED - PARTIAL"),
            "{failure}"
        );
        assert!(failure.contains("[COMPLETE]:"), "{failure}");
        assert!(failure.contains("[UNCHANGED]:"), "{failure}");
        assert!(
            failure.contains("do not retry the whole mutation"),
            "{failure}"
        );
        assert!(!failure.contains("Verdict: COMMITTED"), "{failure}");
    }

    /// Proof for `tui.broad_mutations_are_recoverable`: every aggregate
    /// failure state has a distinct verdict and safe next action.
    #[test]
    fn unchanged_complete_and_diverged_failures_render_distinct_recovery() {
        let unchanged = render_failed_publication(PublicationState::Unchanged);
        assert!(
            unchanged.contains("Verdict: FAILED - UNCHANGED"),
            "{unchanged}"
        );
        assert!(
            unchanged.contains("review again, then retry"),
            "{unchanged}"
        );

        let complete = render_failed_publication(PublicationState::Complete);
        assert!(
            complete.contains("Verdict: FAILED - COMPLETE"),
            "{complete}"
        );
        assert!(
            complete.contains("do not repeat the mutation"),
            "{complete}"
        );

        let diverged = render_failed_publication(PublicationState::Diverged);
        assert!(
            diverged.contains("Verdict: RECOVERY REQUIRED - DIVERGED"),
            "{diverged}"
        );
        assert!(diverged.contains("[DIVERGED]:"), "{diverged}");
        assert!(diverged.contains("do not retry or overwrite"), "{diverged}");
    }

    #[test]
    fn every_nonpublication_outcome_preserves_fields_and_disables_confirmation() {
        let dir = plan();
        let plan_context = context(dir.path());
        let preview = |id: &str, after: &str, operation| {
            preview_dependency(
                &plan_context,
                &DependencyRequest {
                    id: id.to_string(),
                    after: after.to_string(),
                    operation,
                },
            )
            .expect("dependency preview")
            .into_change_set()
        };

        for outcome in [
            CommitOutcome::NoOp(preview("A-2", "A-1", DependencyOperation::Add)),
            CommitOutcome::Denied(preview("A-1", "A-2", DependencyOperation::Add)),
        ] {
            let mut form = bound_form(&plan_context.identity().sha256);
            let _ = request(&mut form);
            form.accept_commit_outcome(outcome);
            let rendered = form.render(120, 200).snapshot();
            assert!(rendered.contains("Verdict: NOT PUBLISHED"), "{rendered}");
            assert!(rendered.contains("Confirm disabled"), "{rendered}");
            assert_eq!(form.fields()[0].values().next(), Some("A complete title"));
        }

        let change = preview("A-2", "A-1", DependencyOperation::Remove);
        let task = dir.path().join("aplan/tasks/A-2.md");
        let bytes = std::fs::read_to_string(&task)
            .expect("task")
            .replace("title: second", "title: concurrent");
        std::fs::write(task, bytes).expect("concurrent edit");
        let actual = context(dir.path()).identity().clone();
        let mut form = bound_form(&plan_context.identity().sha256);
        let _ = request(&mut form);
        form.accept_commit_outcome(CommitOutcome::Stale { change, actual });
        let rendered = form.render(120, 200).snapshot();
        assert!(rendered.contains("Verdict: STALE"), "{rendered}");
        assert!(rendered.contains("Confirm disabled"), "{rendered}");
        assert_eq!(form.fields()[0].values().next(), Some("A complete title"));
    }

    /// Proof for `tui.broad_mutations_are_recoverable`: the generic impact
    /// projection names every task and file for each currently broad family.
    #[test]
    fn every_broad_operation_family_lists_its_complete_scope() {
        let dir = broad_plan();
        let context = repository_context(dir.path());

        let dropped = aplan_app::preview_planning(
            &context,
            &aplan_app::PlanningRequest::Drop(aplan_app::DropRequest {
                ids: vec!["M-1".to_string()],
                by_cut: true,
            }),
        )
        .expect("cut drop")
        .into_change_set();
        assert_scope_lists(&context, dropped, &["A-1", "A-2", "M-1"]);

        let renamed = aplan_app::preview_planning(
            &context,
            &aplan_app::PlanningRequest::Tag(aplan_app::TagRequest::Rename {
                from: "old".to_string(),
                to: "new".to_string(),
            }),
        )
        .expect("tag rename")
        .into_change_set();
        assert_scope_lists(&context, renamed, &["A-1", "A-2"]);

        let imported = aplan_app::preview_import(
            &context,
            &aplan_app::ImportRequest {
                source_name: "captured.md".to_string(),
                contents: "## Imported\n\n- [ ] First\n- [ ] Second\n".to_string(),
                format: "checklist".to_string(),
                phase: 1,
                implemented_as: aplan_app::ImplementedAs::Todo,
                cut: None,
                id_prefix: Some("C".to_string()),
                tags: Vec::new(),
            },
        )
        .expect("import preview");
        let imported_ids = imported.created().to_vec();
        let imported_refs = imported_ids.iter().map(String::as_str).collect::<Vec<_>>();
        assert_scope_lists(&context, imported.into_change_set(), &imported_refs);

        let rekeyed = aplan_app::preview_rekey(
            &context,
            &aplan_app::RekeyRequest {
                old: "A-1".to_string(),
                new: "A-9".to_string(),
                allow_terminal: false,
            },
        )
        .expect("rekey preview")
        .into_change_set();
        assert_scope_lists(&context, rekeyed, &["A-1 -> A-9", "A-2", "M-1"]);
    }

    /// Proof for `tui.broad_mutations_are_recoverable`: publisher success is
    /// explicitly provisional until a fresh load observes the reviewed bytes.
    #[test]
    fn applied_publication_is_not_success_until_reload_reconciles_exact_bytes() {
        let dir = plan();
        let plan_context = context(dir.path());
        let preview = preview_dependency(
            &plan_context,
            &DependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Remove,
            },
        )
        .expect("preview")
        .into_change_set();
        let mut form = bound_form(&plan_context.identity().sha256);
        let request = request(&mut form);
        assert!(form.accept_preview(request, Ok(preview)));
        form.reduce(FormAction::PreviousFocus);
        let FormTransition::CommitRequested(change) = form.reduce(FormAction::Confirm(
            Activation::from_key_kind(KeyEventKind::Press),
        )) else {
            panic!("focused confirmation did not request commit")
        };
        form.accept_commit_outcome(commit_change_set(&plan_context, *change));
        let pending = form.render(120, 200).snapshot();
        assert!(pending.contains("Verdict: RELOAD REQUIRED"), "{pending}");
        assert!(
            pending.contains("reload before claiming success"),
            "{pending}"
        );
        assert!(!pending.contains("Verdict: COMMITTED"), "{pending}");
        assert_eq!(form.fields()[0].values().next(), Some("A complete title"));

        assert!(
            !form.reconcile_reload(&plan_context),
            "the pre-publication in-memory snapshot impersonated a reload"
        );
        assert!(
            form.render(120, 200)
                .snapshot()
                .contains("Verdict: RELOAD REQUIRED")
        );

        let reloaded = context(dir.path());
        assert!(form.reconcile_reload(&reloaded));
        let complete = form.render(120, 200).snapshot();
        assert!(complete.contains("Verdict: COMMITTED"), "{complete}");
        assert!(
            complete.contains("Observed publication: complete"),
            "{complete}"
        );
        assert!(
            complete.contains("do not repeat the mutation"),
            "{complete}"
        );
        assert!(complete.contains("Confirm disabled"), "{complete}");
        assert!(!form.reconcile_reload(&reloaded));
    }

    #[test]
    fn worker_reconciliation_requires_complete_observation_and_fresh_identity() {
        let dir = plan();
        let plan_context = context(dir.path());
        let preview = preview_dependency(
            &plan_context,
            &DependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Remove,
            },
        )
        .expect("preview")
        .into_change_set();
        let mut form = bound_form(&plan_context.identity().sha256);
        let request = request(&mut form);
        assert!(form.accept_preview(request, Ok(preview)));
        form.reduce(FormAction::PreviousFocus);
        let FormTransition::CommitRequested(change) = form.reduce(FormAction::Confirm(
            Activation::from_key_kind(KeyEventKind::Press),
        )) else {
            panic!("focused confirmation did not request commit")
        };
        let unchanged = observe_publication(&change);
        form.accept_commit_outcome(commit_change_set(&plan_context, *change));
        let complete = observe_publication(form.change_set().expect("retained change"));
        assert_eq!(unchanged.state(), PublicationState::Unchanged);
        assert_eq!(complete.state(), PublicationState::Complete);
        assert!(!form.accept_reconciliation("fresh", unchanged));
        assert!(!form.accept_reconciliation("  ", complete.clone()));
        assert!(form.accept_reconciliation("fresh", complete));
        let rendered = form.render(120, 200).snapshot();
        assert!(rendered.contains("Verdict: COMMITTED"), "{rendered}");
        assert!(!form.accept_reconciliation(
            "newer",
            observe_publication(form.change_set().expect("retained change"))
        ));
    }
}
