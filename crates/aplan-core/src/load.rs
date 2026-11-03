//! Loading tasks from markdown files with YAML frontmatter.
//!
//! Frontmatter is validated against the schema *derived from* [`crate::task::Task`]
//! before deserialisation, so a malformed field produces a schema diagnostic
//! rather than an opaque serde error.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::diagnostic::{Diagnostic, code};
use crate::task::Task;

/// A task plus the file it came from.
#[derive(Debug, Clone)]
pub struct LoadedTask {
    pub task: Task,
    pub path: PathBuf,
    /// Where each top-level frontmatter field sits in `path`, so a finding can
    /// name the line it is about rather than only the file (`A1-202`).
    ///
    /// Built by the loader, which is the only place that still has the text the
    /// author typed: YAML is parsed into a `Task` and the layout is gone.
    pub fields: FieldLines,
    /// How many acceptance criteria the body states, and how many are ticked
    /// (`A1-224`).
    ///
    /// A count, and here rather than on [`Task`], for the same reason
    /// [`FieldLines`] is: the body is prose the model deliberately does not
    /// carry, and a rule needs one fact out of it rather than the text. Keeping
    /// it off `Task` keeps it out of the schema, the importers and the
    /// exporters, none of which have anything to say about a checkbox.
    pub criteria: Criteria,
}

/// The acceptance criteria a task's body states, counted (`A1-224`).
///
/// `aplan add` scaffolds `- [ ] state what done means for this task`, and
/// `close` writes the status, the evidence and the fingerprints and touches
/// nothing in the body: the tool authors the obligation and never reads it. A
/// task claiming `done` over its own unticked definition of done is the shape
/// this tool exists to refuse, one rung above evidence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Criteria {
    /// Checkboxes in the body, outside fenced code.
    pub total: usize,
    /// How many of them are ticked.
    pub ticked: usize,
    /// The file line of the first unticked one, so a finding names it rather
    /// than the file.
    pub first_unticked: Option<usize>,
    /// The prompt text that was ticked, so a finding can quote the one it found
    /// rather than the one it looked for first (`A1-271`'s audit: there are two,
    /// and `import` writes the other).
    pub scaffold_prompt: Option<&'static str>,
    /// The file line of the scaffold's own criterion, **ticked** (`A1-271`).
    ///
    /// `aplan add` writes `- [ ] state what done means for this task, in this
    /// repository's terms`. Unticked it is `APLAN038`'s business. Ticked, the
    /// task asserts it met the instruction to write a criterion, and that is a
    /// claim this tool can call false because it wrote the sentence.
    pub scaffold_ticked: Option<usize>,
    /// The file line of the scaffold's own criterion, **unticked** (`A1-299`).
    ///
    /// Unticked and alone it is what `add` asked for and nobody answered, which
    /// is between the author and `APLAN038`. Unticked *beside* criteria the
    /// author wrote is a different thing: the scaffold left behind, which is what
    /// appending a second `## Acceptance criteria` to a scaffolded file produces.
    pub scaffold_unticked: Option<usize>,
    /// How many of `total` are a prompt this tool wrote, ticked or not.
    ///
    /// Counted so a rule can ask what is left when the prompts are set aside,
    /// rather than subtracting one and being wrong about a body carrying both.
    pub prompts: usize,
    /// Whether the scaffold's `## Why` prompt is still there, word for word.
    ///
    /// Not a finding of its own: it is the second sentence of one, because a
    /// task carrying both has answered neither half of what `add` asked.
    pub why_untouched: bool,
}

impl Criteria {
    /// Whether the body states criteria and some are unanswered.
    #[must_use]
    pub const fn unanswered(&self) -> bool {
        self.total > 0 && self.ticked < self.total
    }
}

/// Count the checkboxes in a task body, ignoring fenced code.
///
/// **Fenced code is excluded because a task may quote the scaffold**, and
/// `A1-224`'s own body is the plan's only instance: it shows
/// `- [ ] state what done means for this task` inside a fence while stating five
/// criteria of its own, so a naive counter reads six and reports the task
/// against text it was only citing.
///
/// The fence skipping is [`crate::anchor::unfenced_lines`] rather than a toggle
/// of its own, which is the whole point of that function existing: a fence
/// closes only on its own marker, at its own length or longer, and only when
/// bare, and a reader that gets any of those wrong *reopens a block that is
/// still open* and then reads the sample as content. `A1-165` shipped exactly
/// that mistake and `A1-224`'s audit shipped it again, which is the argument
/// for one reader rather than a careful comment on each.
///
/// Every checkbox in the body counts, not only those under a heading called
/// "Acceptance criteria": the plan spells that heading at least two ways
/// (`## Acceptance criteria`, `## Exit criteria`), and a rule that keyed on the
/// wording would go silent the first time somebody wrote a third.
///
/// A mark that is neither `x`, `X` nor a space is counted as neither ticked nor
/// stated. Conventions differ on what `- [-]` and `- [~]` mean -- cancelled,
/// deferred, not applicable -- and a rule that guessed would report a `done`
/// task over a criterion its author had already decided about.
fn criteria_in(body: &str, first_line: usize) -> Criteria {
    let mut out = Criteria::default();
    for (n, line) in crate::anchor::unfenced_lines(body) {
        let trimmed = line.trim_start();
        // Any of the three list markers, because a task list is a list and
        // CommonMark spells one three ways. This plan writes `-` and `aplan add`
        // scaffolds `-`, so the other two are here for an adopter's convention
        // rather than this one's: a rule that only knew `-` would be silent in a
        // repository that writes `*`, which is worse than absent because the
        // gate would appear to hold.
        let Some(rest) = ["- [", "* [", "+ ["]
            .iter()
            .find_map(|m| trimmed.strip_prefix(m))
        else {
            continue;
        };
        let mut chars = rest.chars();
        let (Some(mark), Some(']')) = (chars.next(), chars.next()) else {
            continue;
        };
        match mark {
            'x' | 'X' => {
                out.total += 1;
                out.ticked += 1;
                // The scaffold's own criterion, ticked (`A1-271`). Exact after
                // trailing whitespace, never a prefix: a criterion that merely
                // begins the way the prompt does is somebody's real sentence,
                // and guessing about prose is what this rule refuses to do.
                if let Some(prompt) = crate::scaffold::criterion_prompts()
                    .into_iter()
                    .find(|p| *p == rest[2..].trim())
                {
                    out.prompts += 1;
                    out.scaffold_ticked.get_or_insert(first_line + n);
                    out.scaffold_prompt.get_or_insert(prompt);
                }
            }
            ' ' => {
                out.total += 1;
                if out.first_unticked.is_none() {
                    out.first_unticked = Some(first_line + n);
                }
                // The same exact match the ticked arm makes, for the same
                // reason: this is the one sentence the tool wrote, so
                // recognising it is not a guess about prose (`A1-299`).
                if let Some(prompt) = crate::scaffold::criterion_prompts()
                    .into_iter()
                    .find(|p| *p == rest[2..].trim())
                {
                    out.prompts += 1;
                    out.scaffold_unticked.get_or_insert(first_line + n);
                    out.scaffold_prompt.get_or_insert(prompt);
                }
            }
            _ => {}
        }
    }
    // Read over the unfenced body once more rather than inside the loop above,
    // which is only looking at list items.
    out.why_untouched = crate::anchor::unfenced_lines(body)
        .into_iter()
        .any(|(_, l)| l.trim() == crate::scaffold::why_prompt());
    out
}

/// Where a YAML parser gave up, as a location in the *file*.
///
/// The only findings that can be placed exactly. A file that did not parse has
/// no fields to point at, but the parser knows the line it stopped on, and that
/// is strictly better than the file header a reader would otherwise get
/// (`A1-202`).
fn parser_location(e: &yaml_compat::Error) -> Option<crate::diagnostic::Location> {
    let mark = e.location()?;
    Some(crate::diagnostic::Location {
        // The parser counts from the start of the frontmatter it was handed; a
        // reader counts from the start of the file.
        line: mark.line() + FRONTMATTER_FIRST_LINE - 1,
        column: mark.column(),
    })
}

/// The file line the frontmatter starts on: the opening `---` is line one.
pub const FRONTMATTER_FIRST_LINE: usize = 2;

/// The line and column of each top-level frontmatter field.
///
/// A field that appears more than once is absent from this map rather than
/// present with one of its two lines. That is the rule the loader already
/// applies to `source_literal`: two candidates mean the answer is a guess, and
/// a guess about where an author meant is worse than not offering one.
pub type FieldLines = BTreeMap<String, crate::diagnostic::Location>;

/// Index the top-level fields of `frontmatter`, whose first line is line
/// `first_line` of the file.
///
/// Deliberately a text scan and not a second parse. The question is where the
/// author's bytes are, and a parsed document has already thrown that away --
/// which is the whole reason this task existed. A top-level field is a line
/// beginning at column one with `key:`; anything indented belongs to one.
#[must_use]
pub fn field_lines(frontmatter: &str, first_line: usize) -> FieldLines {
    let mut found: BTreeMap<String, crate::diagnostic::Location> = BTreeMap::new();
    let mut twice: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for (n, line) in frontmatter.lines().enumerate() {
        // Column one only: an indented `ref:` is part of an evidence entry, not
        // a field of the task, and pointing a reader at it would be a lie about
        // what the finding concerns.
        if line.starts_with(char::is_whitespace) || line.starts_with('#') {
            continue;
        }
        let Some((key, _)) = line.split_once(':') else {
            continue;
        };
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let location = crate::diagnostic::Location {
            line: first_line + n,
            column: 1,
        };
        if found.insert(key.to_string(), location).is_some() {
            twice.insert(key.to_string());
        }
    }

    for key in twice {
        found.remove(&key);
    }
    found
}

/// Everything a validation run needs.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub tasks: Vec<LoadedTask>,
    /// Diagnostics raised during loading (schema, parse).
    pub load_diagnostics: Vec<Diagnostic>,
    /// Ids whose defining file was found but could not be loaded, and that file.
    ///
    /// A task that failed to load is absent from `tasks`, so every edge pointing
    /// at it looks like a dangling reference. It is not: the file is right there.
    /// Reporting the two identically buries the real fault under a cascade
    /// proportional to how well connected the plan is, which is how one unquoted
    /// colon produced fifty-two phantom errors in the fourth pilot (`A1-072`).
    pub unloadable: BTreeMap<String, PathBuf>,
}

/// One filesystem candidate considered by a plan load, with the exact input
/// that load observed.
///
/// This includes readable Markdown that is not presently a task. Such a file
/// can become a task after one edit, so omitting it would make an input identity
/// blind to a change in plan membership. An unreadable candidate carries the
/// stable error kind rather than pretending no input existed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanFileInput {
    pub path: PathBuf,
    pub content: PlanFileContent,
}

/// The bytes or read failure observed for one plan candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanFileContent {
    Bytes(Vec<u8>),
    /// An exact content identity retained without retaining source bytes.
    Sha256(String),
    ReadError {
        kind: std::io::ErrorKind,
        message: String,
    },
}

/// A parsed plan paired with the exact filesystem inputs from which it was
/// derived.
///
/// This is an identified load, not a filesystem-wide atomic snapshot: files
/// are still read in deterministic walk order. Every parsed task and load
/// diagnostic does, however, come from the bytes recorded beside the plan.
#[derive(Debug, Clone)]
pub struct IdentifiedPlan {
    /// The plan root selected for this load, made absolute without resolving
    /// symlinks after the read.
    pub root: PathBuf,
    pub plan: Plan,
    pub inputs: Vec<PlanFileInput>,
}

/// Deterministic work performed by one working-tree load.
///
/// These counters are a regression contract, not a profiler: unlike elapsed
/// time they do not change when a shared runner is busy. Bytes count successful
/// reads, while candidates also include a Markdown path whose read failed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadStats {
    pub candidate_files: usize,
    pub readable_files: usize,
    pub bytes_read: u64,
    pub frontmatter_candidates: usize,
    pub task_files_parsed: usize,
    pub retained_files: usize,
    pub retained_bytes: u64,
}

/// What `check` left unopened: the citations only `verify` resolves (`A1-173`).
///
/// `check` is structural, so a `done` task whose cited commit or test stopped
/// resolving passes it clean. Both surfaces have to be able to *say* that, and
/// they have to say the same thing -- a second count of one fact drifts, which
/// is the lesson `report_payload` already carries from `A1-100` and `A1-101`.
/// So the count lives here and the CLI note and the MCP payload read it.
///
/// `note` evidence is excluded because nothing resolves one by design; the other
/// four kinds are exactly what `verify` opens, against git or against a
/// configured program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Unresolved {
    /// Citations `verify` would open.
    pub citations: usize,
    /// Tasks carrying at least one of them.
    pub tasks: usize,
}

impl Unresolved {
    /// Whether there is anything for a run to have left unopened.
    #[must_use]
    pub const fn any(self) -> bool {
        self.citations > 0
    }
}

/// What a `verify` run had no resolver for, and which kinds it had none of.
///
/// The twin of [`Unresolved`], one rung up. That one counts what a *hermetic*
/// run leaves for `verify`; this counts what `verify` itself leaves, and the
/// reason is the configuration rather than the command: a resolver joins a run
/// only when `.aplan.yml` declares it, so a `test` citation under no runner is
/// recorded and never opened (`A1-253`).
///
/// Here rather than in `aplan-evidence`, which computes it, because all three
/// surfaces report it and only one of them can see that crate: the CLI's
/// renderer depends on `aplan-core` alone, and a second type for the same fact
/// is how two surfaces start describing one plan differently (`A1-104`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Unopened {
    /// Citations no resolver covers.
    pub citations: usize,
    /// Tasks carrying at least one of them.
    pub tasks: usize,
    /// The kinds nothing is configured to check, first seen first.
    pub kinds: Vec<crate::task::EvidenceKind>,
}

impl Unopened {
    /// Whether this run left anything unopened.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.citations > 0
    }

    /// The kinds as a comma-separated list, for a message that names them.
    #[must_use]
    pub fn kinds_said(&self) -> String {
        self.kinds
            .iter()
            .map(|k| format!("`{}`", k.as_str()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The kinds as plain names, for a machine document.
    #[must_use]
    pub fn kind_names(&self) -> Vec<&'static str> {
        self.kinds.iter().map(|k| k.as_str()).collect()
    }
}

/// Count what a structural run did not resolve.
#[must_use]
pub fn unresolved_by_check(plan: &Plan) -> Unresolved {
    let mut out = Unresolved::default();
    for lt in &plan.tasks {
        let n = lt
            .task
            .evidence
            .iter()
            .filter(|ev| {
                !matches!(
                    ev.kind.capabilities().resolution,
                    crate::task::EvidenceResolution::None
                )
            })
            .count();
        if n > 0 {
            out.tasks += 1;
            out.citations += n;
        }
    }
    out
}

/// A file that could not be loaded, and the id it appears to define.
struct LoadFailure {
    /// Recovered from the file rather than parsed, since parsing is what failed.
    id: Option<String>,
    diagnostics: Vec<Diagnostic>,
}

impl From<Vec<Diagnostic>> for LoadFailure {
    fn from(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            id: None,
            diagnostics,
        }
    }
}

/// Split `---\n<yaml>\n---\n<body>` into its frontmatter and body.
///
/// Returns `None` when the source does not open with a `---\n` / `---\r\n`
/// fence, or opens with one that is never closed. The caller ([`load_plan`])
/// pre-filters non-fence files as "not a task", so a `None` it sees always
/// means an unterminated fence.
pub fn split_frontmatter(source: &str) -> Option<(&str, &str)> {
    // Tolerate one or more leading UTF-8 BOMs (Windows editors, PowerShell
    // redirection). Without this a BOM-prefixed task file is treated as "not a
    // task" and its invariants, including `done`-needs-evidence, never run.
    let source = source.trim_start_matches('\u{feff}');
    let rest = source
        .strip_prefix("---\n")
        .or_else(|| source.strip_prefix("---\r\n"))?;
    // Find a line that is exactly `---`.
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" {
            let fm = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return Some((fm, body));
        }
        offset += line.len();
    }
    None
}

/// The id and status a task file declared, read from its frontmatter alone.
///
/// For asking what a task *was* at some earlier commit (`A1-313`, ADR-0038):
/// the window a release closes is the tasks that are `done` now and were not
/// `done` at the last release tag, and answering that means reading a blob
/// rather than a working tree.
///
/// Deliberately not [`load_plan`] against an old checkout. Two fields are
/// wanted, the file may be from a build whose schema this one does not share,
/// and a whole load would report *the old plan's* diagnostics as though they
/// were this run's. `None` for anything that is not a fenced task file with
/// both fields.
#[must_use]
pub fn id_and_status(source: &str) -> Option<(String, crate::task::Status)> {
    #[derive(serde::Deserialize)]
    struct Front {
        id: String,
        status: crate::task::Status,
    }
    let (frontmatter, _) = split_frontmatter(source)?;
    // Unknown keys are ignored on purpose, which is the opposite of the loader's
    // contract and right here: this reads a file written by an older build, and
    // a field that build had is not this one's business to refuse.
    let front: Front = yaml_compat::from_str(frontmatter).ok()?;
    Some((front.id, front.status))
}

/// The JSON Schema derived from the Rust types. This is the same value the
/// `gen-schema` xtask writes to `schemas/v1/task.json`.
pub fn derived_schema() -> serde_json::Value {
    let schema = schemars::schema_for!(Task);
    serde_json::to_value(schema).expect("schema is serialisable")
}

/// A value YAML retyped, where quoting is the whole fix (`A1-158`).
///
/// `ref: 12345678` is a commit ref an author wrote and YAML read as an integer,
/// so the schema refuses it with `12345678 is not of type "string"` -- true, and
/// no help to somebody looking at a line that plainly says a ref. About one
/// short sha in forty-three is all digits: rare enough to baffle, common enough
/// to meet. It took a CI run where seven fixtures shared one unlucky commit to
/// find it.
///
/// The suggestion quotes what the *author* wrote, recovered from the
/// frontmatter, not what YAML made of it: `ref: 0x123456` arrives here as
/// `1193046`, and telling somebody to write `ref: "1193046"` would replace their
/// ref with a different string that happens to load. Where the line cannot be
/// identified unambiguously the sentence still says what happened and stops
/// short of dictating a value.
fn retyped_scalar(
    error: &jsonschema::ValidationError<'_>,
    at: &str,
    frontmatter: &str,
) -> Option<(String, String)> {
    let (literal, kind) = match error.instance().as_ref() {
        serde_json::Value::Number(n) => (n.to_string(), "number"),
        serde_json::Value::Bool(b) => (b.to_string(), "boolean"),
        _ => return None,
    };
    // Only where a string was wanted: every other type error is its own problem
    // and reads clearly already.
    if !error.to_string().contains("of type \"string\"") {
        return None;
    }

    let mut segments = at.rsplit('/').filter(|s| !s.is_empty());
    let last = segments.next().unwrap_or(at);
    // A path can end in an array index (`/paths/0`), where the line is a `-`
    // item and the field name is the segment above it.
    let (field, in_sequence) = if last.chars().all(|c| c.is_ascii_digit()) {
        (segments.next().unwrap_or(last), true)
    } else {
        (last, false)
    };

    let subject = if in_sequence {
        format!("`{field}` is a list of strings")
    } else {
        format!("`{field}` is a string field")
    };
    let Some(source) = source_literal(frontmatter, field, in_sequence, error.instance().as_ref())
    else {
        // Two lines could have produced this value, so which one the reader is
        // looking at is a guess. Say what happened and let them quote it.
        return Some((
            format!(
                "{subject}, and YAML read `{literal}` as a {kind} (at `{at}`). \
                 Quote the value exactly as it is written in the file"
            ),
            format!("Quote the value at `{at}` exactly as it is written in the file."),
        ));
    };
    let write = if in_sequence {
        format!("- \"{source}\"")
    } else {
        format!("{field}: \"{source}\"")
    };
    Some((
        format!(
            "{subject}, and YAML read `{literal}` as a {kind} (at `{at}`). Quote it: `{write}`"
        ),
        format!("Quote the value at `{at}`: write `{write}`."),
    ))
}

/// The text the author typed for a retyped scalar, when exactly one line in the
/// frontmatter could have produced it.
///
/// Matching by value rather than by position: the instance path leads to the
/// JSON, and YAML spans are not on offer, so this asks which lines parse to the
/// same number. Two candidates mean the answer is a guess, and a guess about
/// which ref somebody meant is worse than not offering one.
fn source_literal(
    frontmatter: &str,
    field: &str,
    in_sequence: bool,
    instance: &serde_json::Value,
) -> Option<String> {
    let mut found: Option<String> = None;
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        let rest = if in_sequence {
            match trimmed.strip_prefix("- ") {
                Some(v) => v.trim(),
                None => continue,
            }
        } else {
            let after_dash = trimmed.strip_prefix("- ").unwrap_or(trimmed);
            match after_dash.strip_prefix(&format!("{field}:")) {
                Some(v) => v.trim(),
                None => continue,
            }
        };
        // YAML ends a plain scalar at whitespace-then-`#`, and so must this:
        // suggesting `ref: "12345678 # the fix commit"` would hand somebody a
        // ref that is not a ref, which is worse than the message they had.
        let rest = comment_stripped(rest);
        if rest.is_empty() {
            continue;
        }
        let Ok(parsed) = yaml_compat::from_str::<serde_json::Value>(rest) else {
            continue;
        };
        if &parsed != instance {
            continue;
        }
        match &found {
            Some(seen) if seen != rest => return None,
            Some(_) => {}
            None => found = Some(rest.to_string()),
        }
    }
    found
}

/// A plain scalar without its trailing comment.
///
/// YAML starts a comment at a `#` preceded by whitespace (or at the start), and
/// nowhere else: `abc#def` is the scalar `abc#def`.
fn comment_stripped(value: &str) -> &str {
    if value.starts_with('#') {
        return "";
    }
    let mut chars = value.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c.is_whitespace()
            && let Some((j, '#')) = chars.peek().copied()
        {
            let _ = i;
            return value[..j].trim_end();
        }
    }
    value.trim_end()
}

/// The id a file appears to define, for a file that did not parse.
///
/// The `id:` line first, because that is the file's own claim and survives a
/// syntax error elsewhere in the frontmatter; then the file stem, which is what
/// `aplan add` writes and what a generated plan almost always uses.
///
/// **`text` must be the frontmatter, not the whole file.** A recovered id
/// suppresses `APLAN003` for edges naming it, so reading one out of prose that
/// merely begins a line with `id:` silences a genuinely dangling reference,
/// which is the one risk `A1-072` set out not to take. Only the unterminated
/// fence case passes the whole source, and there every line was meant to be
/// frontmatter.
fn recover_id(text: &str, path: &Path) -> Option<String> {
    for line in text.lines().take(40) {
        let line = line.trim_start_matches('\u{feff}');
        if let Some(rest) = line.strip_prefix("id:") {
            let id = rest.trim().trim_matches(['\'', '"']).trim();
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    file_stem_id(path)
}

/// The id a file's name claims, which is what `aplan add` writes.
///
/// Safe as a guess in a way an id read from prose is not: if it collides with a
/// task that did load, the graph resolves the edge and nothing is suppressed;
/// if it collides with nothing, only edges naming that exact id are affected,
/// and those are edges to the task this file was supposed to define.
fn file_stem_id(path: &Path) -> Option<String> {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
}

/// Turn a YAML parse failure into something that names the mistake.
///
/// `mapping values are not allowed in this context` is what an unquoted `: `
/// inside a value produces, and it is the most common authoring error there is:
/// a title that names a file and then says what it does. The raw message reports
/// a line and column inside the value and never mentions quoting, which is the
/// entire fix (`A1-072`).
fn yaml_error_message(err: &yaml_compat::Error, frontmatter: &str) -> (String, String) {
    let raw = err.to_string();
    if !raw.contains("mapping values are not allowed") {
        return (
            format!("invalid YAML frontmatter: {raw}"),
            "Fix the YAML syntax in the frontmatter.".to_string(),
        );
    }
    // Name the field when the reported line has one, so the reader does not have
    // to count columns to find out which value is unquoted.
    let field = err
        .location()
        .and_then(|loc| frontmatter.lines().nth(loc.line().saturating_sub(1)))
        .and_then(|line| line.split_once(':'))
        .map(|(key, _)| key.trim().to_string())
        .filter(|key| !key.is_empty() && !key.contains(' '));
    let named = match &field {
        Some(f) => format!("the value of `{f}`"),
        None => "a value".to_string(),
    };
    (
        format!(
            "invalid YAML frontmatter: {raw}: {named} contains `: `, which starts a nested \
             mapping unless the value is quoted"
        ),
        match &field {
            Some(f) => format!("Quote the value: `{f}: 'text with: a colon'`."),
            None => "Quote any value containing `: `, for example `title: 'a: b'`.".to_string(),
        },
    )
}

/// Load one task from bytes supplied by a repository snapshot rather than the
/// filesystem. Both entry points meet here so a historical report accepts and
/// rejects exactly the same frontmatter as an ordinary checkout (`A1-194`).
fn load_one_source(
    path: &Path,
    source: &str,
    validator: &jsonschema::Validator,
) -> Result<LoadedTask, LoadFailure> {
    let Some((frontmatter, body)) = split_frontmatter(source) else {
        // The walker only routes files that open with a `---` fence to
        // `load_one`, so a `None` split means the fence was opened but never
        // closed.
        return Err(LoadFailure {
            // The stem only. With no closing fence there is no way to tell
            // frontmatter from prose, so scanning for an `id:` line here reads
            // the body by definition, and a wrong id silences a real edge.
            id: file_stem_id(path),
            diagnostics: vec![
                Diagnostic::error(
                    code::SCHEMA,
                    "frontmatter fence (`---`) opened but never closed",
                )
                .with_file(path)
                .with_agent_instruction(
                    "Close the YAML frontmatter with a `---` line, or remove the opening fence."
                        .to_string(),
                ),
            ],
        });
    };

    // YAML -> JSON so we can schema-check before typed deserialisation.
    let value: serde_json::Value = match yaml_compat::from_str(frontmatter) {
        Ok(v) => v,
        Err(e) => {
            let (message, instruction) = yaml_error_message(&e, frontmatter);
            let mut d = Diagnostic::error(code::SCHEMA, message)
                .with_file(path)
                .with_agent_instruction(instruction);
            if let Some(location) = parser_location(&e) {
                d = d.at(location);
            }
            return Err(LoadFailure {
                id: recover_id(frontmatter, path),
                diagnostics: vec![d],
            });
        }
    };

    let schema_errors: Vec<Diagnostic> = validator
        .iter_errors(&value)
        .map(|e| {
            let at = e.instance_path();
            let (message, instruction) = retyped_scalar(&e, &at.to_string(), frontmatter)
                .unwrap_or_else(|| {
                    (
                        format!("{e} (at `{at}`)"),
                        format!(
                            "Fix the frontmatter field at `{at}` to satisfy schemas/v1/task.json."
                        ),
                    )
                });
            Diagnostic::error(code::SCHEMA, message)
                .with_file(path)
                .with_agent_instruction(instruction)
        })
        .collect();
    if !schema_errors.is_empty() {
        return Err(LoadFailure {
            id: recover_id(frontmatter, path),
            diagnostics: schema_errors,
        });
    }

    // Deserialise straight from the YAML (not from the schema-checked `Value`):
    // the `serde_json::Value` step silently keeps the last of any duplicate keys,
    // so `id: A` twice would pass unnoticed. Serde rejects a duplicate field, the
    // way `.aplan.yml` loading already does.
    match yaml_compat::from_str::<Task>(frontmatter) {
        Ok(task) => {
            check_finite_effort(&task, path)?;
            Ok(LoadedTask {
                task,
                path: path.to_path_buf(),
                // Two: `split_frontmatter` consumed the opening `---` on line
                // one, so the frontmatter's first line is the file's second.
                fields: field_lines(frontmatter, FRONTMATTER_FIRST_LINE),
                // The body begins after the opening fence, the frontmatter and
                // the closing fence.
                criteria: criteria_in(
                    body,
                    FRONTMATTER_FIRST_LINE + frontmatter.lines().count() + 1,
                ),
            })
        }
        Err(e) => {
            // The parser's own line, and the only case where a finding can be
            // placed exactly (`A1-202`): the file did not parse, so it has no
            // fields to point at, but the parser knows where it gave up.
            let mut d = Diagnostic::error(
                code::SCHEMA,
                format!("frontmatter does not deserialise: {e}"),
            )
            .with_file(path)
            .with_agent_instruction(
                "Correct the frontmatter to match the task schema (`aplan schema`).".to_string(),
            );
            if let Some(location) = parser_location(&e) {
                d = d.at(location);
            }
            Err(LoadFailure {
                id: recover_id(frontmatter, path),
                diagnostics: vec![d],
            })
        }
    }
}

/// Reject an `effort_days` of `.inf` or `.nan`, which the schema cannot see.
///
/// The two halves of loading disagree here, and the disagreement is the bug. The
/// schema is checked against a `serde_json::Value` copy of the frontmatter, and
/// JSON has no infinity or NaN, so `yaml_compat` renders both as `null`, which
/// satisfies `["number", "null"]`. The typed deserialisation below reads the same
/// YAML and produces `Some(f64::INFINITY)`. So a hand-written `effort_days: .inf`
/// passed `check` cleanly and then poisoned every rollup that adds it up: the
/// phase totals in `summary`, and the critical path, which is a sum of exactly
/// this field. A negative estimate has been refused since the field existed,
/// for the same reason and by the schema; this closes the hole beside it.
fn check_finite_effort(task: &Task, path: &Path) -> Result<(), Vec<Diagnostic>> {
    match task.effort_days {
        Some(days) if !days.is_finite() => Err(vec![
            Diagnostic::error(
                code::SCHEMA,
                format!(
                    "`effort_days` is `{days}`, which is not a number of days; a rollup that \
                     adds it up reports nothing meaningful"
                ),
            )
            .with_file(path)
            .with_agent_instruction(
                "Give `effort_days` a finite, non-negative estimate, or remove it.".to_string(),
            ),
        ]),
        _ => Ok(()),
    }
}

/// Walk `root` for `*.md` files and load every one that has frontmatter.
///
/// A plan validator must *see every task file*, so the walk deliberately does
/// not honour `.gitignore`/`.ignore`, nor skip dotfiles, nor care about the
/// case of the `.md` extension: a gitignored, hidden, or `TASK.MD` file is still
/// a task, and silently dropping one is the exact failure this tool exists to
/// catch. It skips only well-known non-plan directories (VCS metadata, build
/// output) so pointing `--root` at a repository root does not descend into
/// `.git` or `target`; the default root is the plan directory, which contains
/// none of these, so that skip is belt-and-suspenders.
///
/// Whether `frontmatter` is something this loader would accept, judged by the
/// schema it actually uses.
///
/// **For a writer checking its own output** (`A1-363`). Deserialising a rewrite
/// into a typed struct is weaker than loading it: `blocked_by` is
/// `#[serde(default)] Vec<String>`, and a key present but empty is YAML `null`,
/// which serde takes and the schema refuses. `aplan mv` wrote exactly that and
/// its read-back guard passed, so the tool produced a plan it could not read.
///
/// The order is the loader's own -- YAML to JSON, schema, then types -- because
/// a guard that checks something else is the defect it is there to prevent.
///
/// # Errors
/// The first schema violation, or the YAML error, as the loader would word it.
pub fn frontmatter_is_loadable(frontmatter: &str) -> Result<(), String> {
    let validator = task_validator().map_err(|plan| {
        plan.load_diagnostics.first().map_or_else(
            || "internal: task schema is unavailable".to_string(),
            |d| d.message.clone(),
        )
    })?;
    let value: serde_json::Value = yaml_compat::from_str(frontmatter).map_err(|e| e.to_string())?;
    match validator.iter_errors(&value).next() {
        // With the instance path, because the caller is a writer refusing its
        // own output and "null is not of type array" alone does not say which
        // field it rewrote wrongly. Worded as the loader words it.
        Some(error) => Err(format!("{error} (at `{}`)", error.instance_path())),
        None => Ok(()),
    }
}

/// Files whose first line is not a bare `---` fence are skipped silently; a
/// candidate that fails to load (unterminated fence, invalid YAML, schema
/// violation) produces a diagnostic; an unreadable file is reported, never
/// silently dropped.
pub fn load_plan(root: &Path) -> Plan {
    load_plan_measured(root).0
}

/// Load a working-tree plan and report deterministic work counters.
#[must_use]
pub fn load_plan_measured(root: &Path) -> (Plan, LoadStats) {
    let (loaded, stats) = load_plan_worktree(root, false);
    (loaded.plan, stats)
}

/// Load a working-tree plan and retain the exact candidate bytes it used.
///
/// Each candidate is read once. The former two-read loader first inspected a
/// fence and then reopened the file for parsing, so a concurrent editor could
/// make the candidate decision on one version and the parsed task on another.
/// Keeping the input beside the result also lets an asynchronous caller bind a
/// result to what was actually parsed instead of re-reading a newer tree.
#[must_use]
pub fn load_plan_identified(root: &Path) -> IdentifiedPlan {
    load_plan_identified_measured(root).0
}

/// Load an identified working-tree plan and report deterministic work counters.
#[must_use]
pub fn load_plan_identified_measured(root: &Path) -> (IdentifiedPlan, LoadStats) {
    load_plan_worktree(root, true)
}

struct WorktreeCandidate {
    content: PlanFileContent,
    parsed: Option<Result<LoadedTask, LoadFailure>>,
    read_diagnostic: Option<Diagnostic>,
}

/// The parsed meaning of one Markdown candidate supplied by a caller.
///
/// This is the incremental-loader seam used by the application store. It is
/// deliberately the ordinary loader's result rather than a second cache model:
/// callers may retain a successful [`LoadedTask`] or the fact that a file is
/// not a task, but load failures remain the loader's diagnostics.
#[derive(Debug, Clone)]
pub enum ParsedCandidate {
    /// The bytes do not open with a task frontmatter fence.
    NotTask,
    /// The bytes are one valid task.
    Task(Box<LoadedTask>),
    /// The bytes look like a task but cannot be loaded.
    Failed {
        /// The id recovered from invalid frontmatter, when one was available.
        id: Option<String>,
        /// The same diagnostics a full working-tree load would report.
        diagnostics: Vec<Diagnostic>,
    },
}

/// Parse one already-read working-tree candidate with the authoritative task
/// schema and loader semantics.
///
/// The persistent application cache calls this only for bytes whose digest it
/// has not seen. Keeping fence detection, UTF-8 handling, schema validation and
/// typed deserialisation here prevents an incremental path from becoming a
/// second loader that merely resembles this one.
#[must_use]
pub fn parse_plan_candidate(path: &Path, bytes: &[u8]) -> ParsedCandidate {
    let Ok(source) = std::str::from_utf8(bytes) else {
        return ParsedCandidate::Failed {
            id: None,
            diagnostics: vec![
                Diagnostic::error(
                    code::SCHEMA,
                    "cannot read: stream did not contain valid UTF-8",
                )
                .with_file(path)
                .with_agent_instruction(
                    "Make the file readable, or remove it from the plan directory.".to_string(),
                ),
            ],
        };
    };
    let opens_fence = source
        .trim_start_matches('\u{feff}')
        .lines()
        .next()
        .is_some_and(|line| line.trim_end_matches('\r') == "---");
    if !opens_fence {
        return ParsedCandidate::NotTask;
    }
    let validator = match task_validator() {
        Ok(validator) => validator,
        Err(plan) => {
            return ParsedCandidate::Failed {
                id: None,
                diagnostics: plan.load_diagnostics,
            };
        }
    };
    match load_one_source(path, source, validator) {
        Ok(task) => ParsedCandidate::Task(Box::new(task)),
        Err(failure) => ParsedCandidate::Failed {
            id: failure.id,
            diagnostics: failure.diagnostics,
        },
    }
}

/// Enumerate the exact filesystem candidate set the working-tree loader uses.
///
/// Returned paths are absolute only when `root` is absolute, and are sorted by
/// native path order. Traversal errors retain the historical loader behaviour:
/// an unreadable file that can be enumerated is a candidate and will produce a
/// read diagnostic; an entry the filesystem cannot enumerate is not invented.
#[must_use]
pub fn plan_candidate_paths(root: &Path) -> Vec<PathBuf> {
    plan_walk(root)
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap_or(path);
            (path.is_file() && is_plan_file_path(relative)).then(|| path.to_path_buf())
        })
        .collect()
}

fn plan_walk(root: &Path) -> ignore::Walk {
    ignore::WalkBuilder::new(root)
        .hidden(false)
        .parents(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_str();
            if matches!(name, Some(".git" | ".jj" | ".hg" | ".svn")) {
                return false;
            }
            !(e.depth() == 1
                && matches!(
                    name,
                    Some("target" | "node_modules" | crate::capture::INBOX_DIR)
                ))
        })
        .sort_by_file_path(Path::cmp)
        .build()
}

fn load_worktree_candidate(path: &Path, validator: &jsonschema::Validator) -> WorktreeCandidate {
    let read = std::fs::read(path);
    let Ok(bytes) = read else {
        let error = read.expect_err("the immediately preceding read failed");
        return WorktreeCandidate {
            content: PlanFileContent::ReadError {
                kind: error.kind(),
                message: error.to_string(),
            },
            parsed: None,
            read_diagnostic: Some(
                Diagnostic::error(code::SCHEMA, format!("cannot read: {error}"))
                    .with_file(path)
                    .with_agent_instruction(
                        "Make the file readable, or remove it from the plan directory.".to_string(),
                    ),
            ),
        };
    };
    let Ok(source) = std::str::from_utf8(&bytes) else {
        let diagnostic = Diagnostic::error(
            code::SCHEMA,
            "cannot read: stream did not contain valid UTF-8",
        )
        .with_file(path)
        .with_agent_instruction(
            "Make the file readable, or remove it from the plan directory.".to_string(),
        );
        return WorktreeCandidate {
            content: PlanFileContent::Bytes(bytes),
            parsed: Some(Err(LoadFailure::from(vec![diagnostic]))),
            read_diagnostic: None,
        };
    };
    let opens_fence = source
        .trim_start_matches('\u{feff}')
        .lines()
        .next()
        .is_some_and(|line| line.trim_end_matches('\r') == "---");
    let parsed = opens_fence.then(|| load_one_source(path, source, validator));
    WorktreeCandidate {
        content: PlanFileContent::Bytes(bytes),
        parsed,
        read_diagnostic: None,
    }
}

fn load_plan_worktree(root: &Path, retain_inputs: bool) -> (IdentifiedPlan, LoadStats) {
    let identified_root = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
    let mut stats = LoadStats::default();
    let validator = match task_validator() {
        Ok(v) => v,
        Err(plan) => {
            return (
                IdentifiedPlan {
                    root: identified_root,
                    plan,
                    inputs: Vec::new(),
                },
                stats,
            );
        }
    };

    let mut plan = Plan::default();
    let mut inputs = Vec::new();

    // Sort the walk so load order, and therefore the order of any tie-broken
    // diagnostics, is identical across filesystems and CI runners, not the
    // OS `readdir` order.
    for entry in plan_walk(root).flatten() {
        let path = entry.path();
        // Case-insensitive: `.MD`/`.Md` (a Windows editor, a careless rename) is
        // still a task file, not something to silently drop.
        let relative = path.strip_prefix(root).unwrap_or(path);
        if !path.is_file() || !is_plan_file_path(relative) {
            continue;
        }
        stats.candidate_files += 1;
        // Pre-filter: a task candidate is a file whose first line is a bare
        // `---` fence (after any leading BOMs). A `----` rule or a `---foo`
        // line is not a fence and is skipped silently; an unreadable file is
        // *reported*, not skipped, silently dropping a file is the failure
        // this tool exists to catch.
        let WorktreeCandidate {
            content,
            parsed,
            read_diagnostic,
        } = load_worktree_candidate(path, validator);
        if let PlanFileContent::Bytes(bytes) = &content {
            stats.readable_files += 1;
            stats.bytes_read = stats
                .bytes_read
                .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        }
        if let Some(diagnostic) = read_diagnostic {
            plan.load_diagnostics.push(diagnostic);
        }
        if let Some(parsed) = parsed {
            stats.frontmatter_candidates += 1;
            stats.task_files_parsed += 1;
            match parsed {
                Ok(t) => plan.tasks.push(t),
                Err(mut f) => {
                    if let Some(id) = f.id {
                        plan.unloadable.insert(id, path.to_path_buf());
                    }
                    plan.load_diagnostics.append(&mut f.diagnostics);
                }
            }
        }
        if retain_inputs {
            stats.retained_files += 1;
            if let PlanFileContent::Bytes(bytes) = &content {
                stats.retained_bytes = stats
                    .retained_bytes
                    .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
            }
            inputs.push(PlanFileInput {
                path: path.to_path_buf(),
                content,
            });
        }
    }

    plan.tasks.sort_by(|a, b| a.task.id.cmp(&b.task.id));
    (
        IdentifiedPlan {
            root: identified_root,
            plan,
            inputs,
        },
        stats,
    )
}

/// Whether a path relative to a plan root can be a task file.
///
/// Shared with repository-tree loading so a historical report does not read
/// every blob under `--root .` into memory and cannot drift from the filesystem
/// loader's extension and pruning rules (`A1-194`). Frontmatter remains a byte
/// decision made after the candidate is read.
#[must_use]
pub fn is_plan_file_path(relative: &Path) -> bool {
    relative.parent().is_none_or(is_plan_directory_path)
        && relative
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

/// Whether the filesystem plan walk would enter a directory relative to its
/// root.
///
/// This is separate from [`is_plan_file_path`] because a repository tree has
/// to decide whether to descend before it can see a candidate file. Keeping the
/// boundary shared prevents an ignored build directory (including a gitlink)
/// from making `summary --at HEAD` disagree with an ordinary summary.
#[must_use]
pub fn is_plan_directory_path(relative: &Path) -> bool {
    let components: Vec<_> = relative.components().collect();
    if components.iter().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some(".git" | ".jj" | ".hg" | ".svn")
        )
    }) || (!components.is_empty()
        && matches!(
            components[0].as_os_str().to_str(),
            Some("target" | "node_modules" | crate::capture::INBOX_DIR)
        ))
    {
        return false;
    }
    true
}

/// Load a committed plan from its repository-relative file contents.
///
/// `files` names paths relative to `root`. It is intentionally bytes rather
/// than text: an invalid UTF-8 task is a load diagnostic on disk and must remain
/// one in a historical tree, not disappear before the loader sees it. The same
/// extension, frontmatter-candidate and directory rules as [`load_plan`] apply.
///
/// This is the input seam for revision-addressed reporting (`A1-194`). Git does
/// not belong in `aplan-core`; the repository facade supplies immutable bytes
/// and this loader gives them the same task semantics as a checkout.
pub fn load_plan_files(root: &Path, files: impl IntoIterator<Item = (PathBuf, Vec<u8>)>) -> Plan {
    let validator = match task_validator() {
        Ok(v) => v,
        Err(plan) => return plan,
    };

    let mut files: Vec<_> = files.into_iter().collect();
    files.sort_by(|(a, _), (b, _)| a.cmp(b));
    let mut plan = Plan::default();
    for (relative, bytes) in files {
        let path = root.join(&relative);
        if !is_plan_file_path(&relative) {
            continue;
        }
        let Ok(source) = String::from_utf8(bytes) else {
            plan.load_diagnostics.push(
                Diagnostic::error(
                    code::SCHEMA,
                    "cannot read: stream did not contain valid UTF-8",
                )
                .with_file(&path)
                .with_agent_instruction(
                    "Make the file readable, or remove it from the plan directory.".to_string(),
                ),
            );
            continue;
        };
        let opens_fence = source
            .trim_start_matches('\u{feff}')
            .lines()
            .next()
            .is_some_and(|line| line.trim_end_matches('\r') == "---");
        if !opens_fence {
            continue;
        }
        match load_one_source(&path, &source, validator) {
            Ok(task) => plan.tasks.push(task),
            Err(mut failure) => {
                if let Some(id) = failure.id {
                    plan.unloadable.insert(id, path.clone());
                }
                plan.load_diagnostics.append(&mut failure.diagnostics);
            }
        }
    }
    plan.tasks.sort_by(|a, b| a.task.id.cmp(&b.task.id));
    plan
}

/// Compile the one derived task schema used by both filesystem and committed
/// inputs. A generated schema failure is an internal plan-loading diagnostic;
/// keeping that construction here avoids two untestable copies of the same
/// impossible-under-the-type-system branch.
fn task_validator() -> Result<&'static jsonschema::Validator, Plan> {
    static VALIDATOR: std::sync::OnceLock<Result<jsonschema::Validator, String>> =
        std::sync::OnceLock::new();
    match VALIDATOR.get_or_init(|| {
        jsonschema::validator_for(&derived_schema()).map_err(|error| error.to_string())
    }) {
        Ok(validator) => Ok(validator),
        Err(error) => Err(Plan {
            tasks: Vec::new(),
            unloadable: BTreeMap::new(),
            load_diagnostics: vec![
                Diagnostic::error(
                    code::SCHEMA,
                    format!("internal: derived schema is not a valid JSON Schema: {error}"),
                )
                .with_agent_instruction(
                    "This is an internal error in aplan; please file a bug.".to_string(),
                ),
            ],
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        ParsedCandidate, PlanFileContent, is_plan_file_path, load_plan_files, load_plan_identified,
        load_plan_identified_measured, load_plan_measured, parse_plan_candidate,
        plan_candidate_paths,
    };

    #[test]
    fn incremental_candidate_seam_keeps_the_authoritative_loader_boundaries() {
        let task = b"---\nid: A-1\ntitle: valid\nphase: 1\nstatus: todo\n---\n";
        let ParsedCandidate::Task(loaded) =
            parse_plan_candidate(std::path::Path::new("task.md"), task)
        else {
            panic!("valid task was not parsed")
        };
        assert_eq!(loaded.task.id, "A-1");

        assert!(matches!(
            parse_plan_candidate(std::path::Path::new("notes.md"), b"ordinary prose\n"),
            ParsedCandidate::NotTask
        ));
        let ParsedCandidate::Failed { id, diagnostics } = parse_plan_candidate(
            std::path::Path::new("bad.md"),
            b"---\nid: A-2\ntitle: [\n---\n",
        ) else {
            panic!("invalid YAML was not retained as a load failure")
        };
        assert_eq!(id.as_deref(), Some("A-2"));
        assert_eq!(diagnostics[0].code, crate::diagnostic::code::SCHEMA);

        let ParsedCandidate::Failed { id, diagnostics } =
            parse_plan_candidate(std::path::Path::new("binary.md"), &[0xff])
        else {
            panic!("invalid UTF-8 was not retained as a load failure")
        };
        assert!(id.is_none());
        assert!(diagnostics[0].message.contains("valid UTF-8"));

        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join("tasks/target")).expect("nested target");
        std::fs::create_dir_all(dir.path().join("target")).expect("root target");
        std::fs::create_dir_all(dir.path().join(".git")).expect("vcs metadata");
        std::fs::create_dir_all(dir.path().join(crate::capture::INBOX_DIR)).expect("inbox");
        for relative in [
            "task.md",
            "tasks/target/nested.MD",
            "target/build.md",
            ".git/hidden.md",
            ".inbox/not-a-task.md",
        ] {
            let path = dir.path().join(relative);
            std::fs::write(path, task).expect("candidate fixture");
        }
        let candidates = plan_candidate_paths(dir.path());
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().any(|path| path.ends_with("task.md")));
        assert!(
            candidates
                .iter()
                .any(|path| path.ends_with("tasks/target/nested.MD"))
        );
        assert!(
            !is_plan_file_path(std::path::Path::new(".inbox/not-a-task.md")),
            "committed-tree discovery must share the inbox pruning boundary"
        );
        assert!(
            candidates
                .iter()
                .all(|path| !path.starts_with(dir.path().join(crate::capture::INBOX_DIR))),
            "the reserved inbox must be pruned before Markdown discovery"
        );
    }

    #[test]
    fn measured_load_counts_work_and_retains_bytes_only_when_requested() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let task = "---\nid: A-1\ntitle: measured\nphase: 1\nstatus: todo\n---\n";
        let prose = "ordinary prose\n";
        std::fs::write(dir.path().join("task.md"), task).expect("task");
        std::fs::write(dir.path().join("notes.md"), prose).expect("notes");
        std::fs::write(dir.path().join("ignored.txt"), "not a candidate\n").expect("ignored");

        let (plain, plain_stats) = load_plan_measured(dir.path());
        assert_eq!(plain.tasks.len(), 1);
        assert_eq!(plain_stats.candidate_files, 2);
        assert_eq!(plain_stats.readable_files, 2);
        assert_eq!(plain_stats.frontmatter_candidates, 1);
        assert_eq!(plain_stats.task_files_parsed, 1);
        assert_eq!(
            plain_stats.bytes_read,
            u64::try_from(task.len() + prose.len()).expect("fixture length")
        );
        assert_eq!(plain_stats.retained_files, 0);
        assert_eq!(plain_stats.retained_bytes, 0);

        let (identified, identified_stats) = load_plan_identified_measured(dir.path());
        assert_eq!(identified.inputs.len(), 2);
        assert_eq!(
            identified_stats.candidate_files,
            plain_stats.candidate_files
        );
        assert_eq!(identified_stats.bytes_read, plain_stats.bytes_read);
        assert_eq!(identified_stats.retained_files, 2);
        assert_eq!(identified_stats.retained_bytes, plain_stats.bytes_read);
    }

    /// Proof for `application.input_identity_matches_loaded_inputs`.
    #[test]
    fn an_identified_load_keeps_every_candidate_and_the_bytes_it_parsed() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let task = dir.path().join("task.md");
        let notes = dir.path().join("notes.md");
        let original = "---\nid: A-1\ntitle: before\nphase: 1\nstatus: todo\neffort_days: 1\n---\n";
        std::fs::write(&task, original).expect("task");
        std::fs::write(&notes, "ordinary prose\n").expect("notes");

        let loaded = load_plan_identified(dir.path());
        std::fs::write(&task, original.replace("before", "after")).expect("external edit");

        assert_eq!(loaded.plan.tasks[0].task.title, "before");
        assert_eq!(
            loaded.inputs.len(),
            2,
            "non-task Markdown is still an input"
        );
        let captured = loaded
            .inputs
            .iter()
            .find(|input| input.path == task)
            .expect("task input");
        assert_eq!(
            captured.content,
            PlanFileContent::Bytes(original.as_bytes().to_vec())
        );
        assert!(
            loaded.inputs.iter().any(|input| input.path == notes),
            "a later fence can make this file part of the plan"
        );
    }

    #[test]
    fn committed_plan_candidates_share_the_filesystem_walks_boundary() {
        for path in [
            "task.md",
            "TASK.MD",
            "tasks/target/deep.md",
            "a/node_modules/deep.md",
        ] {
            assert!(super::is_plan_file_path(Path::new(path)), "{path}");
        }
        for path in [
            "notes.txt",
            "target/generated.md",
            "node_modules/generated.md",
            ".inbox/not-a-task.md",
            "tasks/.git/copied.md",
            "a/.svn/copied.md",
        ] {
            assert!(!super::is_plan_file_path(Path::new(path)), "{path}");
        }
        for path in ["", "tasks", "tasks/target", "a/node_modules"] {
            assert!(super::is_plan_directory_path(Path::new(path)), "{path}");
        }
        for path in [
            "target",
            "node_modules",
            ".inbox",
            ".inbox/nested",
            "tasks/.git",
            "a/.svn",
        ] {
            assert!(!super::is_plan_directory_path(Path::new(path)), "{path}");
        }
    }

    /// A repository tree supplies bytes rather than filesystem entries. The
    /// alternate seam still has to make the loader's two easy-to-lose
    /// distinctions: prose markdown is not a task, and invalid UTF-8 in a task
    /// candidate is a finding rather than an absent task (`A1-194`).
    #[test]
    fn committed_bytes_keep_candidate_and_utf8_rules() {
        let root = PathBuf::from("snapshot");
        let plan = load_plan_files(
            &root,
            [
                (PathBuf::from("notes.md"), b"prose only\n".to_vec()),
                (
                    PathBuf::from("tasks/T-1.md"),
                    b"---\nid: T-1\ntitle: One\nphase: 1\nstatus: todo\n---\n".to_vec(),
                ),
                (
                    PathBuf::from("tasks/T-2.md"),
                    vec![b'-', b'-', b'-', b'\n', 0xff],
                ),
            ],
        );
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].task.id, "T-1");
        assert_eq!(plan.load_diagnostics.len(), 1);
        assert_eq!(
            plan.load_diagnostics[0].file.as_deref(),
            Some(root.join("tasks/T-2.md").as_path())
        );
        assert!(plan.load_diagnostics[0].message.contains("valid UTF-8"));
    }

    #[test]
    fn an_id_is_recovered_from_a_file_that_did_not_parse() {
        // Parsing is what failed, so the id has to come out of the text. The
        // file's own `id:` line is its claim and survives a syntax error
        // elsewhere in the frontmatter.
        let path = Path::new("plans/tasks/RD0-08.md");
        let source = "---\nid: RD0-08\ntitle: a: b\n---\n";
        assert_eq!(recover_id(source, path).as_deref(), Some("RD0-08"));

        // Quoted ids, and a leading BOM, are the same claim.
        assert_eq!(
            recover_id("---\nid: 'RD0-08'\n", path).as_deref(),
            Some("RD0-08")
        );
        assert_eq!(
            recover_id("\u{feff}id: \"RD0-08\"\n", path).as_deref(),
            Some("RD0-08")
        );
    }

    #[test]
    fn an_id_is_never_read_out_of_a_files_prose() {
        // A recovered id suppresses `APLAN003` for edges naming it, so reading
        // one out of a body that merely begins a line with `id:` would silence a
        // genuinely dangling reference. That is the single risk `A1-072` set out
        // not to take, and scanning the whole file once took it.
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(
            dir.path().join("nameless.md"),
            "---\ntitle: no id here\nphase: 0\nstatus: todo\n---\n\nProse:\nid: RD9-99\n",
        )
        .expect("write");

        let plan = load_plan(dir.path());
        assert!(
            !plan.unloadable.contains_key("RD9-99"),
            "an id in the body is not this file's claim: {:?}",
            plan.unloadable
        );
        // It still records what it can, so the edge case does not cost the fix.
        assert!(
            plan.unloadable.contains_key("nameless"),
            "the file stem is still recovered: {:?}",
            plan.unloadable
        );
    }

    #[test]
    fn an_unclosed_fence_claims_only_its_file_name() {
        // With no closing fence there is no way to tell frontmatter from prose,
        // so an `id:` line found here is read out of the body by definition.
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(
            dir.path().join("openfence.md"),
            "---\ntitle: never closed\n\nProse:\nid: GHOST2\n",
        )
        .expect("write");

        let plan = load_plan(dir.path());
        assert!(
            !plan.unloadable.contains_key("GHOST2"),
            "{:?}",
            plan.unloadable
        );
        assert!(
            plan.unloadable.contains_key("openfence"),
            "{:?}",
            plan.unloadable
        );
    }

    #[test]
    fn a_file_with_no_readable_id_falls_back_to_its_name() {
        // What `aplan add` writes, and what a generated plan almost always uses.
        let path = Path::new("plans/tasks/RD0-08.md");
        assert_eq!(
            recover_id("---\ntitle: x\n", path).as_deref(),
            Some("RD0-08")
        );
    }

    #[test]
    fn an_unquoted_colon_is_named_as_such_and_the_field_identified() {
        // The most common authoring mistake there is: a title that names a file
        // and then says what it does. The raw serde message reports a column
        // inside the value and never mentions quoting, which is the whole fix.
        let frontmatter = "id: RD0-08\ntitle: ci/scripts/plan.sh: check, verify\n";
        let err = yaml_compat::from_str::<serde_json::Value>(frontmatter)
            .expect_err("an unquoted colon must not parse");
        let (message, instruction) = yaml_error_message(&err, frontmatter);
        assert!(message.contains("`title`"), "{message}");
        assert!(message.contains("quoted"), "{message}");
        assert!(instruction.contains("title:"), "{instruction}");
    }

    #[test]
    fn an_unrelated_yaml_error_keeps_its_own_message() {
        // The special case must not swallow every other syntax error.
        let frontmatter = "id: [unclosed\n";
        let err = yaml_compat::from_str::<serde_json::Value>(frontmatter)
            .expect_err("unclosed bracket must not parse");
        let (message, _) = yaml_error_message(&err, frontmatter);
        assert!(!message.contains("quoted"), "{message}");
        assert!(
            message.starts_with("invalid YAML frontmatter:"),
            "{message}"
        );
    }
    use super::*;

    #[test]
    fn splits_frontmatter() {
        let src = "---\nid: X\n---\nbody here\n";
        let (fm, body) = split_frontmatter(src).expect("has frontmatter");
        assert_eq!(fm, "id: X\n");
        assert_eq!(body, "body here\n");
    }

    #[test]
    fn no_fence_is_none() {
        assert!(split_frontmatter("# just a doc\n").is_none());
    }

    #[test]
    fn unterminated_fence_is_none() {
        assert!(split_frontmatter("---\nid: X\nnever closed\n").is_none());
    }

    /// **The writer's check is as strong as the loader's** (`A1-363`).
    ///
    /// `blocked_by` is `#[serde(default)] Vec<String>`, so a key present with no
    /// value -- YAML `null` -- deserialises happily and fails the schema. A
    /// rewrite that emptied a list produced exactly that, and the guard meant to
    /// catch it read the result with serde and passed. This asserts the two
    /// really do disagree, so the reason for validating against the schema does
    /// not quietly stop being true.
    #[test]
    fn a_null_list_passes_serde_and_fails_the_schema() {
        let frontmatter =
            "id: M-a\ntitle: a cut\nphase: 1\nstatus: todo\ntags:\n  - milestone\nblocked_by:";

        let typed: Result<crate::task::Task, _> = yaml_compat::from_str(frontmatter);
        assert!(
            typed.is_ok(),
            "serde is supposed to be the weaker of the two here: {typed:?}"
        );

        let loadable = frontmatter_is_loadable(frontmatter);
        assert!(
            loadable.is_err(),
            "the schema accepted a null where it wants an array, so the guard built on \
             it would not have caught the rewrite that wrote one"
        );
        assert!(
            loadable.unwrap_err().contains("blocked_by"),
            "the refusal should name the field"
        );

        // And the same frontmatter without the empty key is fine, so the test is
        // about the null and not about the rest of it.
        let without = "id: M-a\ntitle: a cut\nphase: 1\nstatus: todo\ntags:\n  - milestone";
        assert!(frontmatter_is_loadable(without).is_ok());
    }

    #[test]
    fn derived_schema_is_valid_json_schema() {
        let schema = derived_schema();
        assert!(jsonschema::validator_for(&schema).is_ok());
    }

    #[test]
    fn splits_frontmatter_with_bom() {
        let src = "\u{feff}---\nid: X\n---\nbody\n";
        let (fm, body) = split_frontmatter(src).expect("BOM tolerated");
        assert_eq!(fm, "id: X\n");
        assert_eq!(body, "body\n");
    }

    /// Regression: a BOM-prefixed task (common from Windows editors) was
    /// silently skipped, so a `done` with no evidence slipped through.
    #[test]
    fn bom_prefixed_task_is_loaded_not_skipped() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(
            dir.path().join("t.md"),
            "\u{feff}---\nid: T\ntitle: t\nphase: 0\nstatus: todo\n---\nx\n",
        )
        .expect("write");
        let plan = load_plan(dir.path());
        assert_eq!(plan.tasks.len(), 1, "BOM file must not be skipped");
        assert!(
            plan.load_diagnostics.is_empty(),
            "{:?}",
            plan.load_diagnostics
        );
    }

    /// Regression: the walk was unsorted, so diagnostics that tie on
    /// (code, task) came out in filesystem order, divergent across runners.
    #[test]
    fn load_order_is_deterministic() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        for name in ["c.md", "a.md", "b.md"] {
            std::fs::write(
                dir.path().join(name),
                "---\nid: X\ntitle: t\nphase: 0\nstatus: bogus\n---\nx\n",
            )
            .expect("write");
        }
        let plan = load_plan(dir.path());
        let files: Vec<_> = plan
            .load_diagnostics
            .iter()
            .filter_map(|d| d.file.as_ref())
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        assert_eq!(files, vec!["a.md", "b.md", "c.md"]);
    }

    #[test]
    fn strips_multiple_leading_boms() {
        let src = "\u{feff}\u{feff}---\nid: X\n---\nbody\n";
        let (fm, _) = split_frontmatter(src).expect("double BOM tolerated");
        assert_eq!(fm, "id: X\n");
    }

    /// Regression: a `----` rule or a `---foo` line is not a `---` fence and
    /// must be skipped silently, not turned into a false `APLAN001`.
    #[test]
    fn non_fence_markdown_is_skipped_not_errored() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(dir.path().join("notes.md"), "----\n\nnotes\n").expect("write");
        std::fs::write(dir.path().join("other.md"), "---foo\nbar\n").expect("write");
        let plan = load_plan(dir.path());
        assert_eq!(plan.tasks.len(), 0);
        assert!(
            plan.load_diagnostics.is_empty(),
            "non-fence markdown must be skipped, not errored: {:?}",
            plan.load_diagnostics
        );
    }

    /// Regression: a file that opens with a `---` fence but never closes it is
    /// reported with an accurate message (not "no fence at start of file").
    #[test]
    fn unterminated_fence_reports_accurately() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(
            dir.path().join("t.md"),
            "---\n# heading, no close\n\nprose\n",
        )
        .expect("write");
        let plan = load_plan(dir.path());
        assert_eq!(plan.tasks.len(), 0);
        let d = plan
            .load_diagnostics
            .iter()
            .find(|d| d.code == code::SCHEMA)
            .expect("a diagnostic");
        assert!(
            d.message.contains("never closed"),
            "message should name the unterminated fence, got: {}",
            d.message
        );
    }

    /// Regression: an unreadable task file must be reported, not silently
    /// dropped (silent skip is the failure the tool exists to catch).
    #[cfg(unix)]
    #[test]
    fn unreadable_task_is_reported_not_skipped() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = dir.path().join("u.md");
        std::fs::write(&p, "---\nid: U\ntitle: t\nphase: 0\nstatus: done\n---\nx\n")
            .expect("write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        let plan = load_plan(dir.path());
        let dropped = plan.load_diagnostics.is_empty() && plan.tasks.is_empty();
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644));
        assert!(
            !dropped,
            "an unreadable task file must not be silently dropped"
        );
    }

    /// Regression: the VCS/build-output prune must not drop a task in a *nested*
    /// directory that happens to be named `target`/`node_modules`. Only VCS
    /// metadata (any depth) and a root-level build dir are skipped.
    #[test]
    fn a_task_in_a_nested_build_named_dir_is_not_dropped() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let task = "---\nid: {ID}\ntitle: t\nphase: 0\nstatus: done\n---\nx\n";
        for (rel, id) in [
            ("tasks/target/deep.md", "NEST1"),
            ("a/node_modules/x.md", "NEST2"),
            ("normal.md", "NORMAL"),
        ] {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, task.replace("{ID}", id)).unwrap();
        }
        let plan = load_plan(dir.path());
        let ids: Vec<&str> = plan.tasks.iter().map(|t| t.task.id.as_str()).collect();
        for id in ["NEST1", "NEST2", "NORMAL"] {
            assert!(ids.contains(&id), "{id} was dropped: {ids:?}");
        }
    }

    /// A duplicate top-level key in task frontmatter must be rejected, not
    /// silently deduped to the last value (as the `Value` step used to do).
    #[test]
    fn duplicate_frontmatter_keys_are_rejected() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(
            dir.path().join("t.md"),
            "---\nid: A\nid: B\ntitle: t\nphase: 0\nstatus: todo\n---\nx\n",
        )
        .expect("write");
        let plan = load_plan(dir.path());
        assert!(plan.tasks.is_empty(), "a duplicate key must not load");
        assert!(
            plan.load_diagnostics
                .iter()
                .any(|d| d.message.contains("duplicate")),
            "expected a duplicate-field error: {:?}",
            plan.load_diagnostics
        );
    }

    /// A negative `effort_days` is meaningless and would corrupt a rollup; the
    /// schema's `minimum: 0` rejects it at load (APLAN001).
    #[test]
    fn a_negative_effort_is_a_schema_error() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(
            dir.path().join("t.md"),
            "---\nid: T\ntitle: t\nphase: 0\nstatus: todo\neffort_days: -5\n---\nx\n",
        )
        .expect("write");
        let plan = load_plan(dir.path());
        assert!(plan.tasks.is_empty(), "a negative effort must not load");
        assert!(
            plan.load_diagnostics.iter().any(|d| d.code == code::SCHEMA),
            "expected a schema diagnostic: {:?}",
            plan.load_diagnostics
        );
    }

    #[test]
    fn a_non_finite_effort_is_a_schema_error() {
        // The schema cannot catch these: JSON has no infinity or NaN, so the
        // `serde_json::Value` the validator sees carries `null` while the typed
        // value carries `f64::INFINITY`. Both spellings, and both signs, because
        // a rollup is equally meaningless for all of them.
        for value in [".inf", "-.inf", ".nan", ".Inf", ".NaN"] {
            let dir = tempfile::Builder::new()
                .tempdir_in(crate::fixture::fixture_root())
                .expect("tempdir");
            std::fs::write(
                dir.path().join("t.md"),
                format!(
                    "---\nid: T\ntitle: t\nphase: 0\nstatus: todo\neffort_days: {value}\n---\n"
                ),
            )
            .expect("write");
            let plan = load_plan(dir.path());
            assert!(
                plan.tasks.is_empty(),
                "`effort_days: {value}` must not load"
            );
            assert!(
                plan.load_diagnostics.iter().any(|d| d.code == code::SCHEMA),
                "expected a schema diagnostic for {value}: {:?}",
                plan.load_diagnostics
            );
        }
    }

    #[test]
    fn a_finite_effort_still_loads() {
        // The near-miss: the guard must not reject the estimates people write.
        for value in ["0", "0.5", "3", "1e2"] {
            let dir = tempfile::Builder::new()
                .tempdir_in(crate::fixture::fixture_root())
                .expect("tempdir");
            std::fs::write(
                dir.path().join("t.md"),
                format!(
                    "---\nid: T\ntitle: t\nphase: 0\nstatus: todo\neffort_days: {value}\n---\n"
                ),
            )
            .expect("write");
            let plan = load_plan(dir.path());
            assert_eq!(
                plan.tasks.len(),
                1,
                "`{value}` must load: {:?}",
                plan.load_diagnostics
            );
        }
    }

    /// Regression: `.gitignore`/`.ignore` rules, hidden dotfiles, and an
    /// uppercase `.MD` extension must NOT hide a task from the gate. A silently
    /// dropped `done`-without-evidence task is the exact failure aplan exists to
    /// catch, so the walk sees every `.md` regardless of ignore rules or case.
    #[test]
    fn ignore_rules_case_and_dotfiles_do_not_hide_tasks() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        // A `.ignore` (honoured with no git repo, unlike `.gitignore`) that would
        // match a task file:
        std::fs::write(dir.path().join(".ignore"), "secret*.md\n").expect("write");
        let task = "---\nid: {ID}\ntitle: t\nphase: 0\nstatus: done\nevidence:\n  - kind: commit\n    ref: abc1234\n---\nx\n";
        std::fs::write(dir.path().join("secret-a.md"), task.replace("{ID}", "IGN")).expect("write");
        std::fs::write(dir.path().join("UPPER.MD"), task.replace("{ID}", "UP")).expect("write");
        std::fs::write(dir.path().join(".hidden.md"), task.replace("{ID}", "HID")).expect("write");
        let plan = load_plan(dir.path());
        let ids: Vec<&str> = plan.tasks.iter().map(|t| t.task.id.as_str()).collect();
        assert!(
            ids.contains(&"IGN"),
            "ignored task hidden from the gate: {ids:?}"
        );
        assert!(ids.contains(&"UP"), "uppercase `.MD` task hidden: {ids:?}");
        assert!(ids.contains(&"HID"), "hidden dotfile task dropped: {ids:?}");
    }

    /// The `agent` output format promises *every* diagnostic carries an
    /// `agent_instruction`; the parse/load diagnostics must honour that too.
    #[test]
    fn parse_error_diagnostics_carry_an_agent_instruction() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(dir.path().join("bad.md"), "---\nid: [unclosed\n---\nx\n").expect("write");
        let plan = load_plan(dir.path());
        let d = plan
            .load_diagnostics
            .iter()
            .find(|d| d.code == code::SCHEMA)
            .expect("a diagnostic");
        assert!(
            d.agent_instruction.is_some(),
            "load diagnostics must carry an agent_instruction: {d:?}"
        );
    }
    /// A short sha that happens to be all digits is a string YAML retypes, and
    /// the schema's own words for that (`12345678 is not of type "string"`) send
    /// the reader looking at a line that plainly says a ref (`A1-158`).
    #[test]
    fn a_numeric_looking_ref_is_told_it_was_read_as_a_number() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(
            dir.path().join("A-01.md"),
            "---\nid: A-01\ntitle: t\nphase: 1\nstatus: done\neffort_days: 1\n\
             evidence:\n  - kind: commit\n    ref: 12345678\n---\n\nbody\n",
        )
        .expect("write");
        let plan = load_plan(dir.path());
        let d = plan
            .load_diagnostics
            .iter()
            .find(|d| d.code == code::SCHEMA)
            .expect("a diagnostic");
        assert!(
            d.message.contains("YAML read `12345678` as a number"),
            "{}",
            d.message
        );
        assert!(
            d.message.contains(r#"`ref: "12345678"`"#),
            "the fix is quoting, and the message says so: {}",
            d.message
        );
        assert!(
            d.agent_instruction
                .as_deref()
                .unwrap_or_default()
                .contains(r#"ref: "12345678""#),
            "{d:?}"
        );
    }

    /// The suggestion has to be what the *author* wrote. `ref: 0x123456` reaches
    /// the validator as `1193046`, and "write `ref: \"1193046\"`" would swap a
    /// ref for a different string that merely loads.
    #[test]
    fn the_suggestion_quotes_the_text_in_the_file_not_the_converted_value() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(
            dir.path().join("A-01.md"),
            "---\nid: A-01\ntitle: t\nphase: 1\nstatus: todo\neffort_days: 1\n\
             paths:\n  - 0x123456\n---\n\nbody\n",
        )
        .expect("write");
        let plan = load_plan(dir.path());
        let d = plan
            .load_diagnostics
            .iter()
            .find(|d| d.code == code::SCHEMA)
            .expect("a diagnostic");
        assert!(d.message.contains(r#"`- "0x123456"`"#), "{}", d.message);
        assert!(
            !d.message.contains(r#""1193046""#),
            "the converted value must not be offered as the ref: {}",
            d.message
        );
        assert!(
            d.message.contains("list of strings"),
            "a sequence entry is not a field: {}",
            d.message
        );
    }

    /// Where two lines could have produced the value, which one the reader is
    /// looking at is a guess, so the message stops short of dictating one.
    #[test]
    fn an_ambiguous_line_is_not_given_a_value_to_write() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(
            dir.path().join("A-01.md"),
            "---\nid: A-01\ntitle: t\nphase: 1\nstatus: done\neffort_days: 1\nevidence:\n  \
             - kind: commit\n    ref: 0x123456\n  - kind: commit\n    ref: 1193046\n---\n\nbody\n",
        )
        .expect("write");
        let plan = load_plan(dir.path());
        let d = plan
            .load_diagnostics
            .iter()
            .find(|d| d.code == code::SCHEMA)
            .expect("a diagnostic");
        assert!(
            d.message.contains("exactly as it is written in the file"),
            "{}",
            d.message
        );
        assert!(!d.message.contains("Quote it:"), "{}", d.message);
    }

    /// A trailing comment is not part of the value, and a suggestion that
    /// swallows it hands somebody a ref that is not a ref -- strictly worse than
    /// the message they started with.
    #[test]
    fn a_trailing_comment_is_not_quoted_into_the_suggestion() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(
            dir.path().join("A-01.md"),
            "---\nid: A-01\ntitle: t\nphase: 1\nstatus: done\neffort_days: 1\n\
             evidence:\n  - kind: commit\n    ref: 12345678 # the fix commit\n---\n\nbody\n",
        )
        .expect("write");
        let plan = load_plan(dir.path());
        let d = plan
            .load_diagnostics
            .iter()
            .find(|d| d.code == code::SCHEMA)
            .expect("a diagnostic");
        assert!(d.message.contains(r#"`ref: "12345678"`"#), "{}", d.message);
        assert!(
            !d.message.contains("fix commit"),
            "the comment is not part of the ref: {}",
            d.message
        );
    }

    /// YAML starts a comment at whitespace-then-`#` and nowhere else, so a `#`
    /// inside a scalar has to survive.
    #[test]
    fn a_hash_inside_a_value_is_not_a_comment() {
        assert_eq!(comment_stripped("12345678 # note"), "12345678");
        assert_eq!(comment_stripped("12345678\t# note"), "12345678");
        assert_eq!(comment_stripped("abc#def"), "abc#def");
        assert_eq!(comment_stripped("# only a comment"), "");
        assert_eq!(comment_stripped("0o17  "), "0o17");
    }

    /// Only the type errors quoting fixes. A boolean where an integer was
    /// wanted is a scalar too, and quoting it would not help: the rewrite must
    /// not reach it, or every schema error starts claiming a string was wanted.
    #[test]
    fn other_schema_errors_are_left_alone() {
        for (frontmatter, why) in [
            (
                "---\nid: A-01\ntitle: t\nphase: true\nstatus: todo\neffort_days: 1\n---\n\nb\n",
                "a boolean where an integer was wanted",
            ),
            (
                "---\nid: A-01\ntitle: t\nphase: 1\nstatus: todo\neffort_days: nope\n---\n\nb\n",
                "a string where a number was wanted",
            ),
        ] {
            let dir = tempfile::Builder::new()
                .tempdir_in(crate::fixture::fixture_root())
                .expect("tempdir");
            std::fs::write(dir.path().join("A-01.md"), frontmatter).expect("write");
            let plan = load_plan(dir.path());
            let d = plan
                .load_diagnostics
                .iter()
                .find(|d| d.code == code::SCHEMA)
                .unwrap_or_else(|| panic!("{why}: expected a schema diagnostic"));
            assert!(!d.message.contains("YAML read"), "{why}: {}", d.message);
            assert!(!d.message.contains("string field"), "{why}: {}", d.message);
        }
    }
}

#[cfg(test)]
mod field_line_tests {
    use super::*;

    const FM: &str = "id: T-001\ntitle: a task\nblocked_by:\n  - T-002\n  - T-003\nevidence:\n  \
                      - kind: note\n    ref: seeded\n";

    #[test]
    fn a_top_level_field_gets_the_line_it_is_on() {
        let fields = field_lines(FM, FRONTMATTER_FIRST_LINE);
        // The frontmatter's first line is the file's second, so `id` is line 2.
        assert_eq!(fields["id"].line, 2);
        assert_eq!(fields["title"].line, 3);
        assert_eq!(fields["blocked_by"].line, 4);
        assert_eq!(fields["evidence"].line, 7);
        for f in fields.values() {
            assert_eq!(f.column, 1, "a top-level field starts at column one");
        }
    }

    /// An indented `ref:` belongs to an evidence entry, not to the task. A
    /// finding about `evidence` that pointed at it would be naming a different
    /// thing from the one it is about.
    #[test]
    fn an_indented_key_is_not_a_field_of_the_task() {
        let fields = field_lines(FM, FRONTMATTER_FIRST_LINE);
        assert!(
            !fields.contains_key("ref"),
            "an entry's key was taken for a field"
        );
        assert!(
            !fields.contains_key("kind"),
            "an entry's key was taken for a field"
        );
        assert!(
            !fields.contains_key("- kind"),
            "a sequence item was taken for a field"
        );
    }

    /// Two candidates mean the answer is a guess, and the loader's rule
    /// throughout is that a guess is worse than no answer.
    #[test]
    fn a_field_written_twice_has_no_line_rather_than_one_of_them() {
        let fields = field_lines("id: A\nstatus: todo\nid: B\n", FRONTMATTER_FIRST_LINE);
        assert!(
            !fields.contains_key("id"),
            "a duplicated field was given one of its two lines"
        );
        assert_eq!(fields["status"].line, 3, "its neighbours still resolve");
    }

    /// Closes the loop the `points_at` audit leaves open. That audit checks
    /// every declared field against the schema; this checks that the scanner
    /// can actually *find* a field the schema declares. Without it, a future
    /// field outside the scanner's key charset -- a hyphen, a dot -- would pass
    /// the first audit, be declared by a rule, and never once produce a line.
    #[test]
    fn the_scanner_finds_every_field_the_schema_declares() {
        let schema = derived_schema();
        let properties = schema["properties"]
            .as_object()
            .expect("the derived schema has properties");

        // One line per field, so every one has a line to be found on.
        let mut frontmatter = String::new();
        for key in properties.keys() {
            use std::fmt::Write as _;
            let _ = writeln!(frontmatter, "{key}: x");
        }
        let fields = field_lines(&frontmatter, FRONTMATTER_FIRST_LINE);

        let missed: Vec<&String> = properties
            .keys()
            .filter(|k| !fields.contains_key(*k))
            .collect();
        assert!(
            missed.is_empty(),
            "the scanner cannot find {} field(s) the schema declares, so a rule pointing at one \
             would never produce a line: {missed:?}",
            missed.len()
        );
        assert!(
            properties.len() > 8,
            "only {} fields, so this proves little",
            properties.len()
        );
    }

    /// Windows editors write `\r\n`, and a line counted by splitting on `\n`
    /// must still be the line an editor opens.
    #[test]
    fn carriage_returns_do_not_shift_the_count() {
        let real = "---\r\nid: T-001\r\nstatus: todo\r\n---\r\n";
        let (fm, _) = split_frontmatter(real).expect("frontmatter");
        let fields = field_lines(fm, FRONTMATTER_FIRST_LINE);
        assert_eq!(fields["status"].line, 3);
        assert_eq!(
            real.lines()
                .nth(fields["status"].line - 1)
                .map(str::trim_end),
            Some("status: todo")
        );
    }

    /// A BOM sits on line one with the opening fence, so it moves nothing.
    #[test]
    fn a_byte_order_mark_does_not_shift_the_count() {
        let real = "\u{feff}---\nid: T-001\nstatus: todo\n---\n";
        let (fm, _) = split_frontmatter(real).expect("frontmatter");
        let fields = field_lines(fm, FRONTMATTER_FIRST_LINE);
        assert_eq!(fields["status"].line, 3);
        assert_eq!(
            real.lines().nth(fields["status"].line - 1),
            Some("status: todo")
        );
    }

    #[test]
    fn a_comment_is_not_a_field() {
        let fields = field_lines("# id: not-a-field\nid: T-001\n", FRONTMATTER_FIRST_LINE);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields["id"].line, 3);
    }

    /// The offset is the whole reason this is not `lines().position()`: a line
    /// number that is right about the frontmatter and wrong about the file
    /// sends every reader, and every editor, one line off.
    #[test]
    fn the_line_is_counted_from_the_start_of_the_file() {
        let real = "---\nid: T-001\nstatus: todo\n---\n\n## Why\n";
        let (fm, _) = split_frontmatter(real).expect("frontmatter");
        let fields = field_lines(fm, FRONTMATTER_FIRST_LINE);
        let line = fields["status"].line;
        assert_eq!(
            real.lines().nth(line - 1),
            Some("status: todo"),
            "line {line} of the file is not the field it was said to be"
        );
    }

    /// **A checkbox inside a fence is quoted, not stated** (`A1-224`).
    ///
    /// `A1-224`'s own body is the plan's only instance: it shows the scaffold
    /// line `- [ ] state what done means for this task` inside a fence while
    /// stating four criteria of its own, so a counter that read the fence would
    /// report the task against text it was citing.
    #[test]
    fn a_checkbox_inside_a_fence_is_not_a_criterion() {
        let body = "\n## Why\n\nThe scaffold writes:\n\n```markdown\n- [ ] state what done \
                    means\n```\n\n## Acceptance criteria\n\n- [x] one\n- [ ] two\n";
        let c = criteria_in(body, 10);
        assert_eq!(
            (c.total, c.ticked),
            (2, 1),
            "the fenced line is quoted and the two below it are stated"
        );
    }

    /// **A fence closes only on its own marker, and only when bare.**
    ///
    /// The two shapes a hand-rolled toggle gets wrong, and both make the same
    /// mistake in the same direction: they reopen a block that is still open,
    /// so the sample text below reads as the task's own criteria and the rule
    /// reports a `done` task against text it was only quoting.
    #[test]
    fn a_fence_is_not_closed_by_the_other_marker_or_by_an_info_string() {
        let crossed =
            "\n~~~\n- [ ] a sample\n```\n- [ ] still the sample\n~~~\n\n- [x] the real one\n";
        assert_eq!(
            (
                criteria_in(crossed, 1).total,
                criteria_in(crossed, 1).ticked
            ),
            (1, 1),
            "a backtick run does not close a tilde fence"
        );

        let infostring = "\n```markdown\n- [ ] a sample\n```rust\n- [ ] still the sample\n```\n\n- [x] the real one\n";
        assert_eq!(
            (
                criteria_in(infostring, 1).total,
                criteria_in(infostring, 1).ticked
            ),
            (1, 1),
            "a run carrying an info string is content, not a close (CommonMark)"
        );
    }

    /// **A task list is a list, and `CommonMark` spells one three ways.**
    ///
    /// `aplan add` scaffolds `-` and this plan writes `-` everywhere, so this
    /// is about the repositories this rule ships to: one writing `*` would get
    /// a gate that never fires, which is worse than not having it.
    #[test]
    fn a_star_or_plus_bullet_states_a_criterion_too() {
        let c = criteria_in("\n* [x] one\n+ [ ] two\n- [ ] three\n", 1);
        assert_eq!((c.total, c.ticked), (3, 1));
        assert_eq!(
            c.first_unticked,
            Some(3),
            "and the line is the first unticked one whatever marker it carries"
        );
        // Not emphasis: a marker needs the space that makes it a list.
        let none = criteria_in("\n*[x] not a list item\n", 1);
        assert_eq!((none.total, none.ticked), (0, 0));
    }

    /// The count is of the whole body, and a tilde fence closes like a backtick
    /// one.
    #[test]
    fn a_tilde_fence_hides_a_checkbox_too() {
        let c = criteria_in("~~~\n- [ ] quoted\n~~~\n- [ ] stated\n", 1);
        assert_eq!((c.total, c.ticked), (1, 0));
    }

    /// **The line is the first unticked one**, so a reader opens the file where
    /// the work is rather than at the top of it.
    #[test]
    fn the_reported_line_is_the_first_unanswered_criterion() {
        // Body starts at file line 10: `- [ ] two` is the body's third line.
        let c = criteria_in("- [x] one\n- [ ] two\n- [ ] three\n", 10);
        assert_eq!(
            c.first_unticked,
            Some(11),
            "the second body line is file 11"
        );
        assert!(c.unanswered());
    }

    /// **A body that states none is not a body that failed.** Ten of this plan's
    /// closed tasks state no criteria, and the rule has to be silent about them
    /// or it demands a markdown convention rather than checking a claim.
    #[test]
    fn a_body_with_no_checkboxes_is_answered_by_default() {
        let c = criteria_in("## Why\n\nProse only.\n", 1);
        assert_eq!((c.total, c.ticked), (0, 0));
        assert!(
            !c.unanswered(),
            "nothing was claimed, so nothing is unanswered"
        );
    }

    /// A fully ticked body is answered, which is the 196-task majority.
    #[test]
    fn a_fully_ticked_body_is_answered() {
        let c = criteria_in("- [x] one\n- [X] two\n", 1);
        assert_eq!((c.total, c.ticked), (2, 2));
        assert!(!c.unanswered());
        assert_eq!(c.first_unticked, None);
    }
}
