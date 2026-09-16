//! Output formats.
//!
//! Every name here is a [`Format`], declared in one place with the [`Surface`]
//! it can render, and `the_module_doc_lists_every_format` holds this list to
//! that table. The list drifted the moment `markdown` joined the vocabulary,
//! which is the same failure the table exists to stop: a second place saying
//! what the formats are.
//!
//! - **human**, coloured, terminal-first, for a person reading a failure;
//! - **json**, stable machine shape, for deterministic automation;
//! - **agent**, every diagnostic carries an `agent_instruction` a coding agent
//!   can act on without reading the source;
//! - **github**, workflow-command annotations, so a PR shows the failure inline;
//! - **sarif**, SARIF 2.1.0, so code-scanning dashboards ingest the same report;
//! - **markdown**, a document to commit beside the plan: the roadmap and the
//!   deprecation table;
//! - **mermaid**, a *generated* dependency graph, which renders in a README and
//!   in a browser. Never hand-edit a view;
//! - **dot**, the same graph as Graphviz input, which is what every other layout
//!   and analysis tool reads;
//! - **text**, the same graph drawn at a terminal, which is where a person is
//!   standing when they ask.
//!
//! `mermaid` was a rendering without being a choice until `graph` had a second
//! shape to choose between (`A1-193`).

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::Write as _;

use aplan_core::TaskExport;
use aplan_core::deprecation::{DEPRECATION_TAG, Deprecations, REMOVAL_TAG};
use aplan_core::diagnostic::Binding;
use aplan_core::vacuity::RuleReach;
use aplan_core::{
    CriticalPath, Cut, Diagnostic, EdgeKind, Graph, Report, Roadmap, RoadmapTask, Rule, Severity,
    Show, Status, Summary, Task, Verdict,
};

/// What a command prints, which is what decides the formats it can take.
///
/// The alternative was a hand-written list of format names per command, and
/// there were ten of them: nine calls carrying a literal `&["human", "json"]`
/// and one written out longhand inside `show`. Ten lists agree only for as long
/// as somebody keeps them agreeing, and adding a format meant finding all ten.
///
/// A surface is the thing they had in common. `sarif` and `github` are not
/// arbitrary omissions from `summary` -- they are shapes for *findings*, and a
/// rollup has none, so the question "may this command emit sarif" was never
/// really per command. It is per kind of output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// A set of findings: `check` and `verify`. The CI-facing formats mean
    /// something here and nowhere else.
    Diagnostics,
    /// A table or a rollup, read by a person or by a script.
    Report,
    /// A document meant to be committed beside the plan, which is why this is
    /// the one surface `markdown` renders.
    Document,
    /// A dependency graph: `graph`. The one surface with no `human` rendering,
    /// which is why a surface has to carry its own default.
    Graph,
}

impl Surface {
    /// A phrase for a refusal: "`aplan summary` renders *a report*".
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Surface::Diagnostics => "a set of findings",
            Surface::Report => "a report",
            Surface::Document => "a document",
            Surface::Graph => "a dependency graph",
        }
    }

    /// What `--format` defaults to on a command with this surface.
    ///
    /// Carried here rather than assumed to be `human`. It was `human` for every
    /// surface until `graph` joined the vocabulary, and a graph has no human
    /// rendering at all -- `mermaid` is what it has always printed with no flag.
    /// A surface that could not render its own default would leave a command
    /// refusing the value it defaults to (`A1-193`).
    #[must_use]
    pub const fn default_format(self) -> Format {
        match self {
            Surface::Diagnostics | Surface::Report | Surface::Document => Format::Human,
            Surface::Graph => Format::Mermaid,
        }
    }

    /// Every surface, so an audit can walk them.
    pub const ALL: &'static [Surface] = &[
        Surface::Diagnostics,
        Surface::Report,
        Surface::Document,
        Surface::Graph,
    ];

    /// The formats this surface can be rendered in.
    ///
    /// Derived by asking every [`Format`] rather than listed here, so a format
    /// joins the commands that can emit it by being declared, not by being
    /// added to a second table that might disagree with the first.
    #[must_use]
    pub fn formats(self) -> Vec<Format> {
        Format::ALL
            .iter()
            .copied()
            .filter(|f| f.renders(self))
            .collect()
    }
}

/// The format table: one row per format, declaring what it can render.
///
/// The same shape as `define_rules!`, and for the same reason. `Format::ALL`,
/// the parser, the error listing the valid names, every command's allow-list and
/// what `aplan facts` reports are all generated from these rows, so adding a
/// format is one row and cannot be half-done.
macro_rules! define_formats {
    ($(
        $(#[$meta:meta])*
        $variant:ident = $str:literal, renders: $renders:expr
    );+ $(;)?) => {
        /// Selects the rendering.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Format {
            $( $(#[$meta])* $variant ),+
        }

        impl Format {
            /// Every variant, in declaration order. Generated from the same
            /// table as the enum, so it can never omit one.
            pub const ALL: &'static [Format] = &[ $( Format::$variant ),+ ];

            /// The canonical string name of this variant.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $( Format::$variant => $str ),+ }
            }

            /// The surfaces this format can render.
            #[must_use]
            pub const fn surfaces(self) -> &'static [Surface] {
                match self { $( Format::$variant => $renders ),+ }
            }

            /// Whether this format can render `surface`.
            #[must_use]
            pub fn renders(self, surface: Surface) -> bool {
                self.surfaces().contains(&surface)
            }
        }
    };
}

define_formats! {
    /// Coloured, terminal-first, for a person reading a failure.
    Human = "human", renders: &[Surface::Diagnostics, Surface::Report, Surface::Document];
    /// A stable machine shape, for deterministic automation.
    Json = "json", renders: &[Surface::Diagnostics, Surface::Report, Surface::Document, Surface::Graph];
    /// Every diagnostic carries an `agent_instruction` a coding agent can act
    /// on without reading the source.
    Agent = "agent", renders: &[Surface::Diagnostics];
    /// Workflow-command annotations, so a PR shows the failure inline.
    Github = "github", renders: &[Surface::Diagnostics];
    /// SARIF 2.1.0, so code-scanning dashboards ingest the same report.
    Sarif = "sarif", renders: &[Surface::Diagnostics];
    /// A document to commit. Was never a `Format` at all: `roadmap` and
    /// `deprecations` compared the raw string against `"markdown"` by hand, so
    /// the one vocabulary the tool documents did not contain one of the names it
    /// accepts.
    Markdown = "markdown", renders: &[Surface::Document];
    /// A generated dependency graph that renders in a README and in a browser.
    /// It was `graph`'s only output, which is why that command had no
    /// `--format`: one shape is not a choice (`A1-193`).
    Mermaid = "mermaid", renders: &[Surface::Graph];
    /// The same graph as Graphviz input. Mermaid is a rendering for people, and
    /// a consumer that wants to lay the graph out or compute over it had to
    /// parse one; DOT is what every other tool reads (`A1-193`).
    Dot = "dot", renders: &[Surface::Graph];
    /// The graph drawn at a terminal, which is where a person is standing when
    /// they ask. Mermaid renders in a README and DOT feeds another tool; this
    /// is the one that needs nothing else (`A1-219`).
    Text = "text", renders: &[Surface::Graph];
}

impl std::str::FromStr for Format {
    type Err = String;

    /// Derived from `ALL` + `as_str`, so a new variant is parseable the moment it
    /// joins the table, no per-variant arm to forget, and the error lists the
    /// valid names from the same source.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Format::ALL
            .iter()
            .copied()
            .find(|f| f.as_str() == s)
            .ok_or_else(|| {
                let names = Format::ALL
                    .iter()
                    .map(|f| f.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("unknown format `{s}` (expected {names})")
            })
    }
}

/// What produced the report, so the affirmative summary tells the truth. A
/// clean `check` proved the DAG; a clean `verify` resolved the evidence. The
/// diagnostic lines are identical either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind<'a> {
    Check,
    /// `verify`, carrying how many citations no resolver was configured to open.
    /// The summary has to be able to say it: a consumer reading stdout alone
    /// would otherwise be told `evidence resolved` about citations nothing
    /// resolved, and the reason would be on stderr, which a pipe separates
    /// (`A1-253`, the same argument as [`Kind::failure_cause`]).
    ///
    /// Borrowed rather than owned so this stays `Copy`: the kinds are a `Vec`,
    /// and the renderer only reads them.
    Verify(&'a aplan_core::Unopened),
    /// The evidence a `close` or `amend` proposes, resolved before anything is
    /// written. Not a verdict on a plan: nothing here checked the graph.
    Evidence,
    /// `verify` on a plan whose files did not all load: it reports what loaded
    /// and resolves nothing. Carries how many files did not load, because that is
    /// the reason for the verdict and the summary has to be able to say it.
    VerifyPartial(usize),
    /// `check` on the same plan. Separate from [`Kind::Check`] for the same
    /// reason: a plan that did not load is not a verdict a policy can soften, and
    /// `ready`, `why` and `verify` all already refuse one.
    CheckPartial(usize),
}

impl Kind<'_> {
    /// The trailing clause of the human "ok:" line, where there can be one.
    ///
    /// [`None`] means this kind has no clean outcome to describe, which is also
    /// what makes it fail: a plan that did not load is not a verdict a policy can
    /// soften. Expressed as the absence of a note rather than as a second
    /// predicate beside it, so there is no unreachable string for the note to be
    /// wrong in (`A1-112`).
    fn clean_note(&self) -> Option<String> {
        match self {
            Kind::Check => Some("DAG acyclic".to_string()),
            // A run that opened some citations and had no resolver for others
            // resolved evidence, and did not resolve *this* evidence. The count
            // is the difference, said where the verdict is read.
            Kind::Verify(u) if u.any() => Some(format!(
                "evidence resolved, {} citation(s) unchecked",
                u.citations
            )),
            Kind::Verify(_) => Some("evidence resolved".to_string()),
            Kind::Evidence => Some("this evidence resolves".to_string()),
            Kind::VerifyPartial(_) | Kind::CheckPartial(_) => None,
        }
    }

    /// The `schema` field of the non-agent JSON document.
    fn json_schema(&self) -> &'static str {
        match self {
            Kind::Check | Kind::CheckPartial(_) => "aplan/check-report/v1",
            Kind::Verify(_) | Kind::VerifyPartial(_) | Kind::Evidence => "aplan/verify-report/v1",
        }
    }

    /// What this run had no resolver for, for the machine document.
    ///
    /// `None` where the question does not arise *and* where there is nothing to
    /// say, so the field's presence is itself the signal and a plan whose
    /// configuration covers everything reads exactly as it did. The same shape
    /// and the same name the MCP payload uses, because one fact described two
    /// ways is how two surfaces disagree about one plan (`A1-104`).
    fn evidence_unresolved(&self) -> Option<serde_json::Value> {
        match self {
            Kind::Verify(u) if u.any() => Some(serde_json::json!({
                "citations": u.citations,
                "tasks": u.tasks,
                "unconfigured_kinds": u.kind_names(),
            })),
            _ => None,
        }
    }

    /// Why the run failed, when the findings do not say it.
    ///
    /// A consumer reading stdout alone would otherwise see `failed: 0 error(s),
    /// 0 warning(s)` and have nowhere to look: the cause is on stderr, which a
    /// pipe separates. A verdict has to carry its own reason (`A1-091`).
    fn failure_cause(&self) -> String {
        match self {
            Kind::VerifyPartial(n) | Kind::CheckPartial(n) => {
                format!("{n} task file(s) did not load, ")
            }
            _ => String::new(),
        }
    }
}

/// Render a `check` report to stdout.
pub fn render(report: &Report, format: Format, policy: &Verdict) -> std::io::Result<()> {
    render_kind(report, format, Kind::Check, policy)
}

/// Render a `check` report for a plan whose files did not all load.
///
/// A file the loader could not read is not a finding a repository can be
/// tolerant of: every other rule's silence about that file means nothing, so no
/// policy makes the run `ok`. `verify`, `ready` and `why` already refuse such a
/// plan, and `check` said `ok` beside them once `APLAN001` could be downgraded
/// (`A1-112`).
pub fn render_partial(
    report: &Report,
    format: Format,
    policy: &Verdict,
    unloadable: usize,
) -> std::io::Result<()> {
    render_kind(report, format, Kind::CheckPartial(unloadable), policy)
}

/// Render a `verify` report to stdout. Same diagnostics, a truthful summary.
pub fn render_verified(
    report: &Report,
    format: Format,
    policy: &Verdict,
    unopened: &aplan_core::Unopened,
) -> std::io::Result<()> {
    render_kind(report, format, Kind::Verify(unopened), policy)
}

/// Render the evidence a `close` or `amend` proposes, before it is written.
pub fn render_proposed(report: &Report, policy: &Verdict) -> std::io::Result<()> {
    render_kind(report, Format::Human, Kind::Evidence, policy)
}

/// Render a `verify` report for a plan whose files did not all load.
///
/// Separate from [`render_verified`] because the outcome is structural rather
/// than a matter of policy: nothing was resolved, and the summary must not claim
/// otherwise however tolerant the repository's `advisory:` list is.
pub fn render_verified_partial(
    report: &Report,
    format: Format,
    policy: &Verdict,
    unloadable: usize,
) -> std::io::Result<()> {
    render_kind(report, format, Kind::VerifyPartial(unloadable), policy)
}

fn render_kind(
    report: &Report,
    format: Format,
    kind: Kind<'_>,
    policy: &Verdict,
) -> std::io::Result<()> {
    match format {
        Format::Human => render_human(report, kind, policy),
        Format::Json => render_json(report, false, kind.json_schema(), policy, &kind),
        Format::Agent => render_json(report, true, "aplan/agent-report/v1", policy, &kind),
        Format::Github => render_github(report, policy),
        Format::Sarif => render_sarif(report, policy),
        // These render a document and a graph; this renders findings. The CLI
        // refuses each pairing by surface before it can arrive here, so these
        // arms are the invariant stated rather than cases. An error, not
        // `unreachable!`: if the gate is ever bypassed, stopping beats printing
        // a set of findings under a name that promised something else.
        other @ (Format::Markdown | Format::Mermaid | Format::Dot | Format::Text) => {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("`{}` does not render a set of findings", other.as_str()),
            ))
        }
    }
}

/// SARIF 2.1.0, the interchange format static-analysis dashboards already read
/// (GitHub code scanning above all).
///
/// A pure formatter over the same [`Report`]: it adds no rule and no evidence,
/// only a consumer. The rule catalog is built from [`Rule::ALL`] rather than
/// from the diagnostics present in this run, so a result's `ruleId` always
/// resolves to a described rule, and a clean run still tells a dashboard what
/// this tool checks.
fn render_sarif(report: &Report, policy: &Verdict) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{}",
        serde_json::to_string_pretty(&sarif_document(report, policy))?
    )
}

/// The SARIF document for a report, split from the writer so a test can assert
/// against the value rather than scrape stdout.
fn sarif_document(report: &Report, policy: &Verdict) -> serde_json::Value {
    let rules: Vec<serde_json::Value> = Rule::ALL
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.code(),
                "name": r.code(),
                "shortDescription": { "text": r.title() },
                "defaultConfiguration": { "level": sarif_level(r.severity()) },
                "properties": { "tags": ["plan", "evidence"] },
            })
        })
        .collect();

    let results: Vec<serde_json::Value> = report
        .diagnostics
        .iter()
        .map(|d| {
            let mut result = serde_json::json!({
                "ruleId": d.code,
                "level": sarif_level(d.severity),
                "message": { "text": sarif_message(d) },
            });
            // A rule this repository made advisory is *suppressed*, which is the
            // word SARIF already has for a finding that is real and accepted. It
            // is the honest encoding: the `level` stays the rule's own, so a
            // dashboard still knows what the rule says, and the suppression says
            // the repository decided not to be stopped by it. Without this the
            // one surface a repository uploads to code scanning was the one
            // surface that could not tell the two apart (`A1-112`).
            if policy.is_advisory(d.code) {
                result["suppressions"] = serde_json::json!([{
                    "kind": "external",
                    "justification":
                        "listed under `advisory:` in .aplan.yml; reported, and not part of the verdict",
                }]);
            }
            // A result without a location is legal SARIF, and honest: some
            // diagnostics are about the plan as a whole, not one file.
            if let Some(file) = &d.file {
                result["locations"] = serde_json::json!([{
                    "physicalLocation": {
                        // SARIF URIs are forward-slash separated and
                        // repo-relative, whatever the host's separator is.
                        "artifactLocation": { "uri": sarif_uri(file) },
                    }
                }]);
                // A region only when the tool computed one (`A1-202`). This
                // used to be a hard-coded `startLine: 1` standing for "this
                // file", with the note that "claiming a precise line the tool
                // did not compute would be a lie a dashboard would render as
                // fact". It computes one now, and where it cannot the region is
                // omitted, which is SARIF's own way of saying the whole
                // artifact -- line 1 would be that same lie, just a quieter one.
                if let Some(location) = d.location {
                    result["locations"][0]["physicalLocation"]["region"] = serde_json::json!({
                        "startLine": location.line,
                        "startColumn": location.column,
                    });
                }
            }
            result
        })
        .collect();

    serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/main/sarif-2.1/schema/sarif-schema-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "aplan",
                    "informationUri": "https://github.com/asamarts/aplan",
                    "semanticVersion": env!("CARGO_PKG_VERSION"),
                    "version": env!("CARGO_PKG_VERSION"),
                    "rules": rules,
                }
            },
            "results": results,
        }],
    })
}

/// SARIF's level vocabulary. `note` and `none` exist but aplan has only two
/// severities, so mapping is total and needs no fallback.
fn sarif_level(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

/// The message a dashboard shows: the diagnostic, prefixed by the task it
/// concerns, since a SARIF result carries no `task` field of its own.
fn sarif_message(d: &Diagnostic) -> String {
    match &d.task {
        Some(task) => format!("{task}: {}", d.message),
        None => d.message.clone(),
    }
}

/// A SARIF artifact URI: forward slashes on every platform.
///
/// A relative path stays a relative URI reference, which is what a code-scanning
/// dashboard wants, because it maps results onto the repository by that path.
/// An absolute path becomes a proper `file://` URI rather than a bare path:
/// joining its components naively produced `//tmp/...`, a malformed URI that a
/// consumer would either reject or mis-locate.
fn sarif_uri(path: &std::path::Path) -> String {
    let joined = path
        .components()
        .filter(|c| !matches!(c, std::path::Component::RootDir))
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");

    if path.is_absolute() {
        format!("file:///{joined}")
    } else {
        joined
    }
}

fn render_human(report: &Report, kind: Kind<'_>, policy: &Verdict) -> std::io::Result<()> {
    let mut out = anstream::stdout().lock();
    let red = crate::style::ERROR;
    let yellow = crate::style::WARNING;
    let dim = crate::style::DIM;
    let green = crate::style::OK;

    // Every diagnostic carries the specific thing to do about it, and until
    // `A1-086` only `--format agent` printed it. The first pilot hit `APLAN018`,
    // read a finding with no fix, and reasoned its way to deleting the
    // fingerprint, which disables the guard permanently, while the command it
    // wanted had existed for ten days and this very field named it.
    //
    // Once per rule, not once per finding: the instruction is the same advice
    // for every finding of a code, so repeating it fifty times is what would
    // make a fifty-finding report unreadable, and showing it never is what made
    // it unactionable. The first occurrence carries a concrete instance, which
    // is a better template than a generic one.
    let mut instructed: BTreeSet<&str> = BTreeSet::new();
    let mut suppressed = 0usize;
    for d in &report.diagnostics {
        let (style, label) = match d.severity {
            Severity::Error => (red, "error"),
            Severity::Warning => (yellow, "warning"),
        };
        // Built as one string so the fold can measure what precedes the
        // message. Folding the message alone against `MAX_WIDTH` left the first
        // line as long as the prefix plus the width -- 103 columns on a real
        // finding, which the trycmd goldens caught (`A1-206`). Measured on the
        // generated catalogue: 42 of 61 messages and 44 of 61 fix lines are
        // wider than eighty columns, the longest fix line 404.
        let mut head = format!("{style}{label}{style:#}[{}]", d.code);
        if let Some(t) = &d.task {
            let _ = write!(head, " {t}");
        }
        head.push_str(": ");
        // A message quotes the plan: `APLAN032` prints the `paths` entry it
        // could not compile, and a dozen rules name a title, a tag or an
        // anchor. That is the widest route a plan has to its reader's terminal,
        // and the one the census found first (`A1-289`).
        writeln!(out, "{}", fold::row(&head, &sanitize::visible(&d.message)))?;
        if let Some(f) = &d.file {
            // Not folded: a path is one unbreakable token and the thing a
            // reader most wants to copy.
            // `path:line:col`, the convention `check-docs` already prints and
            // every editor and terminal knows how to open (`A1-202`). Without a
            // location it stays the bare path, which is what it has always been.
            // A path comes from the filesystem rather than from the plan, and
            // a filename may carry anything a filesystem allows.
            let at = sanitize::visible(&match d.location {
                Some(l) => format!("{}:{}:{}", f.display(), l.line, l.column),
                None => f.display().to_string(),
            })
            .into_owned();
            writeln!(out, "  {dim}--> {at}{dim:#}")?;
        }
        if let Some(ai) = &d.agent_instruction {
            if instructed.insert(d.code) {
                // `fold_after` rather than a fixed indent, so the prefix is
                // measured rather than assumed and the first line fits too.
                writeln!(
                    out,
                    "{}{dim:#}",
                    fold::row(&format!("  {dim}fix: "), &sanitize::visible(ai))
                )?;
            } else {
                suppressed += 1;
            }
        }
    }
    if suppressed > 0 {
        writeln!(
            out,
            // Not "repeat a fix": an instruction interpolates the ids of the
            // task it is about, so twenty-five `APLAN003` findings carry
            // twenty-five different lines. Telling a reader they repeat is
            // telling them to skip advice that differs.
            "{dim}({suppressed} further finding(s) carry their own fix; \
             `--format agent` prints every one){dim:#}"
        )?;
    }

    // Counted against the *verdict*, not the raw severities: a repository that
    // made a rule advisory passes, and a summary that said `failed` while the
    // command exited 0 would be a line contradicting its own exit code
    // (`A1-112`). The findings above still print at their declared severity,
    // because the severity is the rule's and only the tolerance is the
    // repository's.
    let (e, w) = (
        report.blocking(policy, Severity::Error),
        report.blocking(policy, Severity::Warning),
    );
    let advisory = report.advisory_count(policy);
    // Never silent: a build that passed because a rule was downgraded must not
    // read like one that had nothing to say.
    let heard = if advisory == 0 {
        String::new()
    } else {
        let mut codes: Vec<&str> = report
            .diagnostics
            .iter()
            .filter(|d| policy.is_advisory(d.code))
            .map(|d| d.code)
            .collect();
        codes.sort_unstable();
        codes.dedup();
        format!(", {advisory} advisory ({})", codes.join(", "))
    };
    // The word is the *verdict*, not a count: under `deny_warnings` a run with one
    // warning and no errors exits 1, and this line said `ok` (`A1-112`). A kind
    // with no clean note has no passing form at all, which is how a plan that did
    // not load reaches this.
    let Some(clean_note) = kind.clean_note().filter(|_| !report.denied_by(policy)) else {
        writeln!(
            out,
            "{red}failed{red:#}: {}{e} error(s), {w} warning(s){heard} across {} tasks",
            kind.failure_cause(),
            report.tasks_checked
        )?;
        return Ok(());
    };
    writeln!(
        out,
        "{green}ok{green:#}: {} tasks, {w} warning(s){heard}, {clean_note}",
        report.tasks_checked
    )?;
    Ok(())
}

fn render_json(
    report: &Report,
    agent: bool,
    schema: &str,
    policy: &Verdict,
    kind: &Kind<'_>,
) -> std::io::Result<()> {
    let diags: Vec<serde_json::Value> = report
        .diagnostics
        .iter()
        .map(|d| {
            let mut o = serde_json::json!({
                "code": d.code,
                "severity": d.severity.to_string(),
                "message": d.message,
                "task": d.task,
                "file": d.file.as_ref().map(|p| p.display().to_string()),
                // Null, not absent and not 1, when the finding has no single
                // line: a consumer can tell "the tool does not know" from "line
                // one" only if the two look different (`A1-202`).
                "line": d.location.map(|l| l.line),
                "column": d.location.map(|l| l.column),
            });
            if agent && let Some(ai) = &d.agent_instruction {
                o["agent_instruction"] = serde_json::Value::String(ai.clone());
            }
            o
        })
        .collect();

    let mut doc = serde_json::json!({
        "schema": schema,
        "tasks_checked": report.tasks_checked,
        // The counts a consumer scripts against are the blocking ones, for the
        // same reason the human summary uses them: they must agree with the exit
        // code. `advisory` says what was heard and not stopped for, so a passing
        // run is never silent about a downgraded rule (`A1-112`).
        "errors": report.blocking(policy, Severity::Error),
        "warnings": report.blocking(policy, Severity::Warning),
        "advisory": report.advisory_count(policy),
        "diagnostics": diags,
    });
    // Additive, and only where the question arises: a gate branches on this
    // rather than parsing the summary line, which is the whole reason the count
    // is published at all (`A1-253`).
    if let (Some(v), Some(obj)) = (kind.evidence_unresolved(), doc.as_object_mut()) {
        obj.insert("evidence_unresolved".to_string(), v);
    }
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &doc)?;
    writeln!(out)
}

fn render_github(report: &Report, policy: &Verdict) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    for d in &report.diagnostics {
        writeln!(out, "{}", github_line(d, policy.is_advisory(d.code)))?;
    }
    Ok(())
}

/// One GitHub workflow-command annotation for a diagnostic.
///
/// Everything interpolated here is user-controlled and can arrive in a fork
/// pull request, so it is escaped per GitHub's rules (`@actions/core`): without
/// it a comma corrupts the property list and a newline injects a second, forged
/// command.
fn github_line(d: &Diagnostic, advisory: bool) -> String {
    let level = match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    let file = d
        .file
        .as_ref()
        .map(|p| format!("file={},", escape_property(&p.display().to_string())))
        .unwrap_or_default();
    // `line`/`col` place the annotation on the field rather than on the file
    // header, which is where GitHub puts one that names no line (`A1-202`).
    // Omitted rather than defaulted when the finding has no single location.
    let at = match (d.file.as_ref(), d.location) {
        (Some(_), Some(l)) => format!("line={},col={},", l.line, l.column),
        _ => String::new(),
    };
    // A diagnostic about the plan as a whole carries no task, and the title is
    // then the code alone rather than the code and a dash: `APLAN029 -` reads
    // like a title that was cut off. `APLAN029` is the first rule to reach this
    // in practice, which is how the placeholder came to be looked at (`A1-085`).
    let task = d
        .task
        .as_deref()
        .map(|t| format!(" {t}"))
        .unwrap_or_default();
    // An annotation has no suppression concept, so the disclosure goes in the
    // title, where a reviewer sees it beside the rule. The level stays the rule's
    // own rather than being softened to `notice`: severity is what the rule says,
    // and tolerance is what this repository chose (`A1-112`).
    let note = if advisory { " (advisory)" } else { "" };
    let title = escape_property(&format!("{}{task}{note}", d.code));
    format!(
        "::{level} {file}{at}title={title}::{}",
        escape_data(&d.message)
    )
}

/// GitHub workflow-command *data* escaping (`@actions/core` `escapeData`).
fn escape_data(s: &str) -> String {
    // The workflow-command escaping is about `%`, `\r` and `\n` and says
    // nothing about the rest of the C0 range (`A1-289`, audit).
    sanitize::visible(s)
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// GitHub workflow-command *property* escaping (`@actions/core` `escapeProperty`).
fn escape_property(s: &str) -> String {
    escape_data(s).replace(':', "%3A").replace(',', "%2C")
}

/// Render a plan [`Summary`] to stdout: an aligned table for `human`, a stable
/// JSON document for every other format (there are no diagnostics to shape, so
/// `json`, `agent`, and `github` all collapse to the machine form).
pub fn render_summary(summary: &Summary, format: Format) -> std::io::Result<()> {
    match format {
        Format::Human => render_summary_human(summary),
        _ => render_summary_json(summary),
    }
}

fn render_summary_human(summary: &Summary) -> std::io::Result<()> {
    let mut out = anstream::stdout().lock();
    let bold = crate::style::HEADING;

    writeln!(
        out,
        "{bold}{:>5}  {:>5}  {:>4}  {:>6}{bold:#}",
        "phase", "tasks", "done", "effort"
    )?;
    for p in &summary.phases {
        writeln!(
            out,
            "{:>5}  {:>5}  {:>4}  {:>5.1}d",
            p.phase, p.tasks, p.done, p.effort_days
        )?;
    }
    writeln!(
        out,
        "{bold}{:>5}  {:>5}  {:>4}  {:>5.1}d{bold:#}",
        "total",
        summary.total_tasks(),
        summary.total_done(),
        summary.total_effort_days()
    )?;
    writeln!(out)?;
    // The derived duration (`A1-147`, ADR-0023). Printed only where the
    // repository declared a throughput, and printed *beside* the effort it comes
    // from rather than stored anywhere, so the two cannot drift apart: that is
    // the whole answer to the sixth pilot's hand-maintained calendar
    // (pre-public adoption evidence item 2). It says nothing about dates: a week here
    // is a unit of capacity, not a position in one.
    if let (Some(per_week), Some(weeks), Some(left)) = (
        summary.effort_days_per_week,
        summary.total_weeks(),
        summary.remaining_weeks(),
    ) {
        writeln!(
            out,
            "at {per_week:.1} effort days a week: {} weeks of work, {} remaining",
            weeks_cell(weeks),
            weeks_cell(left)
        )?;
    }
    // Cancelled work is named rather than merely omitted: a plan that drops a
    // cut made a decision, and a total that quietly leaves it out is how the
    // first version of this reported pre-cancellation scope forever (`A1-074`).
    let dropped = summary.total_dropped();
    if dropped > 0 {
        writeln!(
            out,
            "dropped: {dropped} task(s), {:.1}d, not counted above",
            summary.total_dropped_effort_days()
        )?;
    }
    writeln!(out, "ready: {}", summary.ready)
}

/// `aplan tasks`: the plan as data.
///
/// The JSON is the point; the human table exists so the command is not
/// write-only, and shows the fields a rollup most often groups by.
///
/// # Errors
/// Propagates any write failure on stdout.
pub fn render_tasks(export: &TaskExport, format: Format) -> std::io::Result<()> {
    if format == Format::Human {
        return render_tasks_human(export);
    }
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, export)?;
    writeln!(out)
}

/// The width a column should be printed at: the widest value in it, or `floor`,
/// whichever is larger.
///
/// The floor is what keeps a change like this invisible to every plan that did
/// not need it: a table whose values all fit renders exactly as it did before
/// the column could grow at all.
fn column<'a>(values: impl Iterator<Item = &'a str>, floor: usize) -> usize {
    values.map(str::len).max().unwrap_or(0).max(floor)
}

fn render_tasks_human(export: &TaskExport) -> std::io::Result<()> {
    let mut out = anstream::stdout().lock();
    let bold = crate::style::HEADING;
    // Wide enough for the widest value, never narrower than the width this table
    // has always had. A fixed 12 was fine while every id was `A1-047`; under
    // `id_strategy: ulid` an id is 26 characters plus the plan's prefix, and one
    // of those shifted every column of its own row while leaving the rest of the
    // table looking correct (`A1-047`, found by its audit). The floors keep the
    // output of every plan whose ids already fit byte-for-byte identical.
    let id_w = column(export.tasks.iter().map(|t| t.id.as_str()), 12);
    let cut_w = column(
        export.tasks.iter().map(|t| t.cut.as_deref().unwrap_or("-")),
        8,
    );
    writeln!(
        out,
        "{bold}{:<id_w$}  {:<10}  {:>5}  {:<cut_w$}  {:<6}  title{bold:#}",
        "id", "status", "effort", "cut", "ready"
    )?;
    for t in &export.tasks {
        let effort = match t.effort_days {
            Some(d) => format!("{d:.1}"),
            // Distinguished from `0.0`: an unestimated task is not a free one.
            None => "-".to_string(),
        };
        let ready = if t.milestone_task {
            "-"
        } else if t.ready {
            "yes"
        } else {
            "no"
        };
        writeln!(
            out,
            "{}",
            fold::row(
                // Padded first and styled second: a width specifier counts the
                // bytes of an escape sequence, so styling the value and then
                // padding it shifts every column of that row by the length of
                // its own colour. The padding lands inside the style, which is
                // invisible for a foreground-only palette and keeps the stripped
                // bytes identical to the uncoloured table.
                &format!(
                    "{id}{:<id_w$}{id:#}  {st}{:<10}{st:#}  {:>5}  {:<cut_w$}  {:<6}  ",
                    t.id,
                    t.status.to_string(),
                    effort,
                    t.cut.as_deref().unwrap_or("-"),
                    ready,
                    id = crate::style::ID,
                    st = crate::style::status(t.status.as_str()),
                ),
                // A title carrying a newline or tab would break the row, and a
                // table that data can corrupt is not a table. The JSON escapes
                // it; `roadmap --format markdown` already guards the same
                // thing.
                &one_line(&t.title)
            )
        )?;
    }
    Ok(())
}

/// Collapse the whitespace that would break a row into single spaces.
fn one_line(text: &str) -> String {
    // Safe to put in a row is both halves of the job: whitespace that would
    // break the row, and a control character that would steer the terminal
    // printing it (`A1-289`). Every caller wants both, and a caller that wanted
    // one and got the other is the bug this being one function prevents.
    sanitize::visible(text)
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_summary_json(summary: &Summary) -> std::io::Result<()> {
    let phases: Vec<serde_json::Value> = summary
        .phases
        .iter()
        .map(|p| {
            serde_json::json!({
                "phase": p.phase,
                "tasks": p.tasks,
                "done": p.done,
                "effort_days": p.effort_days,
                "dropped": p.dropped,
                "dropped_effort_days": p.dropped_effort_days,
                "done_effort_days": p.done_effort_days,
            })
        })
        .collect();
    let doc = serde_json::json!({
        // v2: `total_tasks` and `total_effort_days` changed meaning, from every
        // task to every task that can still happen. Field names that keep their
        // spelling while changing their number are how a consumer reads a wrong
        // answer confidently, so the version says it out loud (`A1-074`).
        "schema": "aplan/summary/v2",
        "phases": phases,
        "ready": summary.ready,
        "total_tasks": summary.total_tasks(),
        "total_done": summary.total_done(),
        "total_effort_days": summary.total_effort_days(),
        "total_dropped": summary.total_dropped(),
        "total_dropped_effort_days": summary.total_dropped_effort_days(),
        "total_done_effort_days": summary.total_done_effort_days(),
        // Additive, and `null` where the repository declared no throughput: a
        // consumer reading a duration gets one or gets nothing, never a zero
        // that reads as "no work left" (`A1-147`).
        "effort_days_per_week": summary.effort_days_per_week,
        "total_weeks": summary.total_weeks(),
        "remaining_weeks": summary.remaining_weeks(),
    });
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &doc)?;
    writeln!(out)
}

/// Render a [`CriticalPath`] to stdout: an aligned table for `human`, a stable
/// JSON document for every other format.
pub fn render_critical_path(path: &CriticalPath, format: Format) -> std::io::Result<()> {
    match format {
        Format::Human => render_critical_path_human(path),
        _ => render_critical_path_json(path),
    }
}

fn render_critical_path_human(path: &CriticalPath) -> std::io::Result<()> {
    let mut out = anstream::stdout().lock();
    let bold = crate::style::HEADING;
    if path.tasks.is_empty() {
        return writeln!(out, "critical path: none (no tasks)");
    }
    writeln!(
        out,
        "{bold}critical path: {:.1} days across {} task(s){bold:#}",
        path.total_effort_days,
        path.tasks.len()
    )?;
    for t in &path.tasks {
        // The title is the trailing column, so it folds under itself rather
        // than under the id (`A1-206`).
        writeln!(
            out,
            "{}",
            fold::row(
                &format!(
                    "  {id}{:<8}{id:#}  {dim}{:>5.1}d{dim:#}  ",
                    t.id,
                    t.effort_days,
                    id = crate::style::ID,
                    dim = crate::style::DIM
                ),
                &sanitize::visible(&t.title)
            )
        )?;
    }
    Ok(())
}

fn render_critical_path_json(path: &CriticalPath) -> std::io::Result<()> {
    let tasks: Vec<serde_json::Value> = path
        .tasks
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "title": t.title,
                "effort_days": t.effort_days,
                "status": t.status.to_string(),
            })
        })
        .collect();
    let doc = serde_json::json!({
        "schema": "aplan/critical-path/v1",
        "total_effort_days": path.total_effort_days,
        "tasks": tasks,
    });
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &doc)?;
    writeln!(out)
}

/// Render one task for a person (`A1-118`).
///
/// The projection is [`aplan_core::Show`], assembled once and serialised by two
/// surfaces; this is the only part that is text.
#[must_use]
/// The one-line summary under a task's title.
///
/// Its own function so [`render_show`] stays inside its line limit, which is
/// what keeps that renderer readable as a sequence of sections.
fn show_facts(show: &Show) -> Vec<String> {
    let mut facts = vec![format!("phase {}", show.phase)];
    if let Some(cut) = &show.cut {
        let m = show.milestone.as_deref().unwrap_or("");
        facts.push(format!("cut {cut} ({m})"));
    }
    if let Some(e) = show.effort_days {
        facts.push(format!("{e}d"));
    }
    if let Some(o) = &show.owner {
        facts.push(format!("owner {}", sanitize::visible(o)));
    }
    // Only for work that could be picked up. `ready` is a word about offered
    // work: a milestone is a checkpoint (ADR-0008) and a `done` task is over, so
    // "ready no" about either reads as "blocked", which is the wrong word `why`
    // was corrected for (`A1-105`).
    if show.workable {
        facts.push(format!("ready {}", if show.ready { "yes" } else { "no" }));
    }
    facts
}

/// `show`'s three list blocks: what the task declares it touches, traces and
/// cites.
///
/// Lifted out of `render_show` because that function is at its line ceiling, and
/// these three are one shape repeated.
fn show_lists(s: &mut String, show: &Show) {
    for (heading, items) in [
        ("paths", show.paths.clone()),
        ("traces", show.traces.clone()),
        (
            "evidence",
            show.evidence
                .iter()
                .map(|e| {
                    let captured = e
                        .captured
                        .as_ref()
                        .map_or_else(String::new, |c| format!(" ({c})"));
                    format!("{}:{}{captured}", e.kind, e.reference)
                })
                .collect(),
        ),
    ] {
        if items.is_empty() {
            continue;
        }
        let _ = writeln!(s, "  {hd}{heading}:{hd:#}", hd = crate::style::HEADING);
        for item in items {
            // A `paths` glob, a `traces` anchor and an evidence reference are
            // all strings the plan wrote (`A1-289`).
            let _ = writeln!(s, "    {}", sanitize::visible(&item));
        }
    }
}

pub fn render_show(show: &Show) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{}",
        fold::row(
            &format!(
                "{id}{}{id:#}  {st}{}{st:#}  ",
                show.id,
                show.status,
                id = crate::style::ID,
                st = crate::style::status(&show.status),
            ),
            &sanitize::visible(&show.title)
        )
    );

    let _ = writeln!(s, "  {}", show_facts(show).join("   "));
    if !show.tags.is_empty() {
        let _ = writeln!(
            s,
            "  {hd}tags:{hd:#} {}",
            sanitize::visible(&show.tags.join(", ")),
            hd = crate::style::HEADING
        );
    }
    // On its own line rather than in the facts above, and saying what it is:
    // a reader who sees only an instant will read it as a deadline, and it is
    // the opposite -- the earliest moment worth looking, which proves nothing
    // (`A1-264`).
    if let Some(at) = &show.revisit_at {
        let _ = writeln!(
            s,
            "  revisit at {}: parked until then is worth re-reading, not due",
            sanitize::visible(at)
        );
    }

    show_lists(&mut s, show);

    for (heading, items) in [("waits on", &show.waits_on), ("releases", &show.releases)] {
        if items.is_empty() {
            continue;
        }
        let _ = writeln!(s, "  {hd}{heading}:{hd:#}", hd = crate::style::HEADING);
        for r in items {
            let _ = writeln!(
                s,
                "    {id}{}{id:#}  {st}{}{st:#}",
                r.id,
                r.status,
                id = crate::style::ID,
                st = crate::style::status(&r.status),
            );
        }
    }

    if show.findings.is_empty() {
        // The distinction the whole command exists for: "nothing to report" and
        // "nothing was checked" are different answers, and a file cannot tell you
        // which one you have.
        let _ = writeln!(
            s,
            "  {hd}findings:{hd:#} none{}",
            if show.resolved {
                ""
            } else {
                ", and the evidence was not resolved (--no-resolve)"
            },
            hd = crate::style::HEADING
        );
    } else {
        let _ = writeln!(s, "  {hd}findings:{hd:#}", hd = crate::style::HEADING);
        for d in &show.findings {
            // The message is the trailing column and folds under itself; it did
            // not until `A1-291`, and `show` was measured against a task with no
            // findings so nothing said so.
            let _ = writeln!(
                s,
                "{}",
                fold::row(
                    &format!("    {}[{}] ", d.severity, d.code),
                    &sanitize::visible(&d.message),
                )
            );
            if let Some(fix) = &d.fix {
                let _ = writeln!(s, "{}", fold::row("      fix: ", &sanitize::visible(fix)));
            }
        }
        if !show.resolved {
            let _ = writeln!(
                s,
                "  (hermetic rules only; --no-resolve skipped the ladder)"
            );
        }
    }

    if let Some(body) = &show.body {
        let _ = writeln!(s, "\n{}", body.trim_end());
    }
    s
}

/// Render the whole graph as Mermaid, **grouping and all**.
///
/// A *generated* view: commit it only behind a regenerate-and-diff check, never
/// hand-edit it.
///
/// This draws a cut listing its members as an edge, which is what `graph` did
/// before `A1-216` and what it now does only under `--membership`. Three
/// quarters of a plan's edges can be grouping, and drawing them as constraint is
/// what made this diagram unreadable, so a caller that wants the dependency
/// graph should build a [`GraphView`] with [`GraphView::of_kinds`] and render
/// that.
pub fn mermaid(graph: &Graph<'_>) -> String {
    let all: BTreeSet<&str> = graph.by_id.keys().copied().collect();
    mermaid_of(graph, &all)
}

/// The same diagram over a subset of the plan (`A1-126`), grouping and all.
///
/// See [`mermaid`] for why "and all" is worth saying: use
/// [`GraphView::of_kinds`] and [`mermaid_of_view`] to draw dependencies alone.
///
/// The fourth pilot got 285 lines and 12.6 KB of Mermaid from a 105-task plan,
/// "a graph that size does not render usefully anywhere, so on a plan of this
/// shape the command is effectively unavailable" (pre-public adoption evidence item
/// 13). The whole-plan render is still the default, because a small plan wants
/// it and a scoped view of a small plan is the same picture.
///
/// An edge is drawn only when *both* ends are in view. A dangling arrow to a
/// node the reader cannot see would say "something outside blocks this" without
/// saying what, which is worse than the honest omission: the node's blockers are
/// a `why` or a `tree` away.
pub fn mermaid_of(graph: &Graph<'_>, include: &BTreeSet<&str>) -> String {
    mermaid_of_view(&GraphView::of(graph, include))
}

/// One node of a scoped graph.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub title: String,
    /// The status, so a consumer laying the graph out can style by it without a
    /// second lookup. Serialised by `Status` itself, so the word here is the
    /// word the task file uses rather than a fourth spelling of it.
    pub status: Status,
    /// Whether this status satisfies a dependency: what the Mermaid rounds the
    /// box for, said as a field rather than left to be inferred from a shape.
    pub done: bool,
}

/// One edge: `from` must finish before `to` can start.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    /// Whether this is a constraint or a cut listing a member (`A1-216`).
    ///
    /// Derived by [`aplan_core::edge_kind`], which is the one place that knows,
    /// and carried here so every shape reads the same answer instead of each
    /// re-deriving it from the ids.
    pub kind: EdgeKind,
}

/// The scoped graph as data, which every rendering of it is built from.
///
/// `graph` printed Mermaid and nothing else, so a consumer wanting to lay the
/// graph out or compute over it had to parse a rendering (`A1-193`). Three
/// shapes come from this one value rather than from three walks of the plan,
/// which is what stops them disagreeing about what is in view.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct GraphView {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// Edges with exactly one end in view, which no shape can draw.
    ///
    /// Reported rather than dropped: a scoped view that silently loses the edges
    /// leaving it answers a question the reader did not ask (`A1-126`). Mermaid
    /// and DOT say so in a comment; JSON carries the count.
    pub hidden_edges: usize,
    /// Cut-membership edges left out of this view, and the tasks that had none
    /// but membership (`A1-216`).
    ///
    /// Zero unless the view was narrowed to dependencies, which is what `graph`
    /// does by default. A default that drew grouping as constraint was nine
    /// layers deep and 177 nodes wide; a default that quietly omits three
    /// quarters of the plan's edges would be the same false completeness this
    /// repository refuses elsewhere, so both numbers are carried and every shape
    /// says them.
    #[serde(default)]
    pub grouping: Grouping,
    /// How this view was narrowed, in the words a reader can act on.
    ///
    /// A reachability scope has to say which graph it walked, because
    /// reachability over grouping answers "what is in this cut" and over
    /// dependencies answers what a reader means by "what does this unblock",
    /// and a view that does not say leaves them to assume (`A1-221`).
    /// Always serialised, empty or not: a field that disappears hands a
    /// consumer `null` where it expected a list (`A1-182`).
    #[serde(default)]
    pub scope: Vec<String>,
}

/// What narrowing a graph to its dependencies left behind.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Grouping {
    /// Membership edges not drawn.
    pub edges: usize,
    /// Tasks that had no edge left once membership came out, so are not drawn.
    pub tasks: usize,
}

impl Grouping {
    /// Whether anything was left out.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.edges > 0 || self.tasks > 0
    }
}

impl GraphView {
    /// The nodes and edges `include` scopes to, grouping and all.
    #[must_use]
    pub fn of(graph: &Graph<'_>, include: &BTreeSet<&str>) -> Self {
        Self::of_kinds(graph, include, true)
    }

    /// The same, keeping cut-membership edges only when `membership` is set.
    ///
    /// Narrowing here rather than in each renderer, so "what is in view" stays
    /// one decision (`A1-216`). A task left with no edge at all is dropped with
    /// it -- drawing 178 isolated boxes is not a dependency graph -- and both
    /// counts are recorded so no shape can omit them silently.
    #[must_use]
    pub fn of_kinds(graph: &Graph<'_>, include: &BTreeSet<&str>, membership: bool) -> Self {
        let whole = Self::build(graph, include);
        if membership {
            return whole;
        }

        let (kept, dropped): (Vec<GraphEdge>, Vec<GraphEdge>) = whole
            .edges
            .into_iter()
            .partition(|e| e.kind == EdgeKind::Dependency);
        let touched: BTreeSet<&str> = kept
            .iter()
            .flat_map(|e| [e.from.as_str(), e.to.as_str()])
            .collect();
        // A task is dropped only if narrowing is what isolated it. One that had
        // no edge to begin with was not a casualty of this: it was already
        // alone, the view already drew it, and the renderers already say "no
        // edges between them" (`A1-126`). Dropping it here would refuse to draw
        // a scope that has simply never had a dependency in it.
        let had_an_edge: BTreeSet<&str> = dropped
            .iter()
            .flat_map(|e| [e.from.as_str(), e.to.as_str()])
            .collect();
        let (nodes, orphans): (Vec<GraphNode>, Vec<GraphNode>) = whole
            .nodes
            .into_iter()
            .partition(|n| touched.contains(n.id.as_str()) || !had_an_edge.contains(n.id.as_str()));

        Self {
            nodes,
            edges: kept,
            scope: whole.scope,
            hidden_edges: whole.hidden_edges,
            grouping: Grouping {
                edges: dropped.len(),
                tasks: orphans.len(),
            },
        }
    }

    fn build(graph: &Graph<'_>, include: &BTreeSet<&str>) -> Self {
        let nodes = graph
            .by_id
            .iter()
            .filter(|(id, _)| include.contains(*id))
            .map(|(id, t)| GraphNode {
                id: (*id).to_string(),
                title: t.title.clone(),
                status: t.status,
                done: t.status.satisfies_dependency(),
            })
            .collect();

        // Every edge in the plan, not only those of the tasks in view: an edge
        // is hidden when *either* end is outside, and counting only one
        // direction would miss the tasks waiting on this scope from outside it.
        let mut edges = Vec::new();
        let mut hidden_edges = 0usize;
        for (id, t) in &graph.by_id {
            let dependent_in = include.contains(id);
            for b in &t.blocked_by {
                match (dependent_in, include.contains(b.as_str())) {
                    (true, true) => edges.push(GraphEdge {
                        from: b.clone(),
                        to: (*id).to_string(),
                        kind: graph
                            .get(b.as_str())
                            .map_or(EdgeKind::Dependency, |tail| aplan_core::edge_kind(tail, t)),
                    }),
                    (true, false) | (false, true) => hidden_edges += 1,
                    (false, false) => {}
                }
            }
        }
        Self {
            nodes,
            edges,
            hidden_edges,
            grouping: Grouping::default(),
            scope: Vec::new(),
        }
    }

    /// What a scoped view has to say beyond its nodes and edges.
    ///
    /// Shared by the two shapes that carry comments, so Mermaid and DOT cannot
    /// come to disagree about a view neither of them drew fully.
    pub(crate) fn notes(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.scope.is_empty() {
            out.push(format!("scoped to {}.", self.scope.join(", and ")));
        }
        if self.edges.is_empty() {
            let n = self.nodes.len();
            let plural = if n == 1 { "" } else { "s" };
            out.push(format!(
                "{n} task{plural} in view and no edges between them: nothing here waits on \
                 anything else here."
            ));
        }
        if self.grouping.any() {
            out.push(format!(
                "{} cut-membership edge(s) and {} task(s) with no dependency are not drawn: a cut \
                 listing its members is grouping, which `aplan roadmap` prints as a heading. \
                 `--membership` draws them.",
                self.grouping.edges, self.grouping.tasks
            ));
        }
        if self.hidden_edges > 0 {
            let said = if self.hidden_edges == 1 {
                "1 edge to a task outside this view is not drawn".to_string()
            } else {
                format!(
                    "{} edges to tasks outside this view are not drawn",
                    self.hidden_edges
                )
            };
            out.push(format!("{said}. `aplan tree <id>` follows them."));
        }
        out
    }
}

/// The view as Mermaid, which renders in a README and in a browser.
#[must_use]
pub fn mermaid_of_view(view: &GraphView) -> String {
    let mut s = String::from("graph TD\n");
    for n in &view.nodes {
        let key = node_key(&n.id);
        let label = format!("{}: {}", escape(&n.id), escape(&n.title));
        let shape = if n.done {
            format!("{key}([\"{label}\"])")
        } else {
            format!("{key}[\"{label}\"]")
        };
        let _ = writeln!(s, "    {shape}");
    }
    for e in &view.edges {
        // Dotted for grouping, solid for constraint: when both are drawn they
        // must not read as the same relationship, which is the confusion that
        // made this diagram unreadable in the first place (`A1-216`).
        let arrow = match e.kind {
            EdgeKind::Dependency => "-->",
            EdgeKind::Membership => "-.->",
        };
        let _ = writeln!(s, "    {} {arrow} {}", node_key(&e.from), node_key(&e.to));
    }
    // Comments, so the diagram a reader pastes anywhere is still a diagram.
    for note in view.notes() {
        let _ = writeln!(s, "%% {note}");
    }
    s
}

/// The view as Graphviz DOT, which is what every other layout and analysis tool
/// reads (`A1-193`).
#[must_use]
pub fn dot_of_view(view: &GraphView) -> String {
    let mut s = String::from("digraph plan {\n    rankdir=TB;\n");
    for n in &view.nodes {
        // A node keeps its id verbatim inside quotes, which DOT allows and
        // Mermaid does not: no sanitising here, so a consumer reading the DOT
        // gets the id the plan uses.
        let shape = if n.done { "ellipse" } else { "box" };
        let _ = writeln!(
            s,
            "    {} [label={}, shape={shape}];",
            dot_quote(&n.id),
            dot_quote(&format!("{}: {}", n.id, n.title))
        );
    }
    for e in &view.edges {
        let style = match e.kind {
            EdgeKind::Dependency => "",
            EdgeKind::Membership => " [style=dashed]",
        };
        let _ = writeln!(
            s,
            "    {} -> {}{style};",
            dot_quote(&e.from),
            dot_quote(&e.to)
        );
    }
    for note in view.notes() {
        let _ = writeln!(s, "    // {note}");
    }
    s.push_str("}\n");
    s
}

/// A DOT double-quoted string.
///
/// DOT escapes a quote and a backslash with a backslash, and a newline inside a
/// quoted string is legal but would break a reader scanning line by line, so it
/// becomes a space -- the same decision `escape` makes for Mermaid.
fn dot_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    // As for Mermaid above: DOT quotes a `"` and a `\`, and passes a control
    // character straight through to whatever reads it (`A1-289`, audit).
    for c in sanitize::visible(s).chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One task's neighbourhood in both directions, as text (`A1-126`).
///
/// `A1-125` made `ready`'s order derived rather than declared, and ADR-0017 is
/// explicit that a derived order nobody can inspect is an assertion the reader
/// has to take on trust. This is the inspection: **what must precede this task**
/// is why it is not offered yet, and **what it releases** is the rank's own first
/// input, so the two directions are exactly the two halves of that claim.
///
/// The blockers shown are `blockers_of`, so a cut-sequencing edge (ADR-0013)
/// appears here even though no task declares it. A reader asking "why is this
/// not ready" is owed the edge the tool acts on, not the one the file happens to
/// spell.
///
/// `depth` is `None` for the whole neighbourhood. A subtree cut short says so
/// rather than ending silently, and a task already shown is marked instead of
/// being walked again, so a cyclic plan (`APLAN004`, which `check` reports and
/// this command does not refuse) terminates.
pub fn tree<'a>(graph: &Graph<'a>, id: &str, depth: Option<usize>, live: bool) -> Option<String> {
    let (key, task) = graph.by_id.get_key_value(id).map(|(k, t)| (*k, *t))?;
    // Inverted once for the whole walk rather than per node: see
    // `Graph::dependents_index`.
    let dependents = graph.dependents_index();
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{}",
        fold::row(
            &format!(
                "{id}{}{id:#}  {st}{}{st:#}  ",
                task.id,
                task.status,
                id = crate::style::ID,
                st = crate::style::status(task.status.as_str()),
            ),
            &sanitize::visible(&task.title)
        )
    );

    if let Some((_, rank)) = graph.ready_ranked().iter().find(|(t, _)| t.id == id) {
        // Shown only for a task `ready` would offer, because the rank is an
        // order over *offered* work: printing it for a blocked task would invite
        // reading it as a queue position it does not have (`A1-126`).
        let _ = writeln!(
            s,
            "  {hd}rank:{hd:#} {}",
            rank.because(),
            hd = crate::style::HEADING
        );
    }

    for (heading, up) in [("waits on", true), ("releases", false)] {
        let mut lines = Vec::new();
        let mut shown: BTreeSet<&'a str> = BTreeSet::new();
        shown.insert(key);
        walk(
            graph,
            &dependents,
            key,
            up,
            Bounds { depth, live },
            &mut shown,
            &mut lines,
        );

        // What the status filter withheld, by difference: the same walk without
        // it, minus what this one printed. Counted this way rather than tallied
        // during the walk because the two filters overlap -- a satisfied task
        // below a `--depth` cut is withheld by both -- and a tally would either
        // double-count it or argue with the depth sentence about whose it was
        // (`A1-217`).
        let withheld = if live {
            let mut all_lines = Vec::new();
            let mut all: BTreeSet<&'a str> = BTreeSet::new();
            all.insert(key);
            walk(
                graph,
                &dependents,
                key,
                up,
                Bounds { depth, live: false },
                &mut all,
                &mut all_lines,
            );
            all.difference(&shown).count()
        } else {
            0
        };

        if lines.is_empty() && withheld == 0 {
            let _ = writeln!(
                s,
                "  {hd}{heading}:{hd:#} nothing",
                hd = crate::style::HEADING
            );
        } else {
            let _ = writeln!(s, "  {hd}{heading}:{hd:#}", hd = crate::style::HEADING);
            for l in lines {
                let _ = writeln!(s, "{l}");
            }
            if withheld > 0 {
                let plural = if withheld == 1 { "" } else { "s" };
                let _ = writeln!(
                    s,
                    "    ... {withheld} satisfied task{plural} not shown, past --live"
                );
            }
        }
    }
    Some(s)
}

/// One direction of [`tree`], depth-first in id order so the output is stable.
///
/// **Iterative on purpose.** The recursive version overflowed the stack and
/// aborted the process on a 3000-deep chain: not a hypothetical shape, since a
/// plan whose tasks are strictly sequenced is exactly a chain, and a *view*
/// command that kills the process on a large plan is worse than the 285-line
/// diagram this task exists to replace (`A1-126`, found while auditing it).
/// What bounds one direction of the walk.
///
/// Bundled because they are one idea -- how much of the ancestry this view is
/// for -- and because the two interact: a satisfied task below a `--depth` cut
/// is withheld by both, which is why the elision counts are taken by difference
/// rather than tallied (`A1-217`).
#[derive(Debug, Clone, Copy)]
struct Bounds {
    /// Levels to walk, unlimited when `None`.
    depth: Option<usize>,
    /// Show only what can still block.
    live: bool,
}

/// One row of `tree`: the id, the status, and the title as the trailing column.
///
/// Lifted out of `walk` because that function is at its line ceiling and this is
/// the half of it that is about rendering rather than traversal.
///
/// The note is styled here rather than at the call site so that an absent one
/// emits no bytes at all: wrapping an empty string in a dim span is a pair of
/// escapes around nothing, which strips to the same text and is still noise in
/// the stream.
///
/// The title is the trailing column, so it folds under itself rather than under
/// the id, and the indent is part of the prefix, so a deep row folds under its
/// own depth (`A1-206`).
fn node_row(indent: &str, task: &Task, membership: bool) -> String {
    let note = if membership {
        format!("{dim}  (the cut it is in){dim:#}", dim = crate::style::DIM)
    } else {
        String::new()
    };
    fold::row(
        &format!(
            "{indent}{id}{}{id:#}  {st}{}{st:#}  ",
            task.id,
            task.status,
            id = crate::style::ID,
            st = crate::style::status(task.status.as_str()),
        ),
        &format!("{}{note}", sanitize::visible(&task.title)),
    )
}

fn walk<'a>(
    graph: &Graph<'a>,
    dependents: &std::collections::BTreeMap<&'a str, Vec<&'a Task>>,
    root: &'a str,
    up: bool,
    bounds: Bounds,
    seen: &mut BTreeSet<&'a str>,
    out: &mut Vec<String>,
) {
    /// What is left to print. `Expand` holds a node whose children print one
    /// level deeper, so popping it after its own line keeps a subtree together.
    enum Step<'a> {
        Child {
            task: &'a Task,
            parent: &'a str,
            level: usize,
        },
        Ghost(&'a str, usize),
        Expand(&'a str, usize),
    }

    let Bounds { depth, live } = bounds;
    let mut stack = vec![Step::Expand(root, 1)];
    // `seen` is what the view *printed*, which is what the elision counts and
    // `hidden_below` both read. A node hidden by `--live` is walked through
    // without being printed, so it cannot use that set to avoid being expanded
    // twice; this is the set that does.
    let mut expanded: BTreeSet<&'a str> = BTreeSet::new();
    expanded.insert(root);
    // Where a `--depth` limit cut the view off, filled in once the walk knows
    // everything it printed.
    let mut pending: Vec<(usize, &'a str, usize)> = Vec::new();
    while let Some(step) = stack.pop() {
        match step {
            Step::Ghost(id, level) => {
                // An id nothing in the plan answers to is `APLAN003`'s finding,
                // and it is also the reason a task is not ready, so it is named
                // rather than dropped.
                out.push(format!("{}{id}  (no such task)", "  ".repeat(level + 1)));
            }
            Step::Child {
                task,
                parent,
                level,
            } => {
                let indent = "  ".repeat(level + 1);
                let Some((key, _)) = graph.by_id.get_key_value(task.id.as_str()) else {
                    continue;
                };
                // A task whose status satisfies a dependency cannot block
                // anything, which is what `ready` computes, so under `--live` it
                // is not printed. **Walked through, not skipped over**: a done
                // task can stand between the reader and live work below it, and
                // the first version suppressed the whole subtree -- on this
                // plan's own `A0-001` that hid 46 live tasks and called them
                // satisfied (`A1-217`).
                if live && task.status.satisfies_dependency() {
                    // Its children take the level it would have occupied, so
                    // nothing is indented under a row nobody can see.
                    if expanded.insert(key) {
                        stack.push(Step::Expand(key, level));
                    }
                    continue;
                }
                if !seen.insert(key) {
                    out.push(format!(
                        "{indent}{id}{}{id:#}{dim}  (shown above){dim:#}",
                        task.id,
                        id = crate::style::ID,
                        dim = crate::style::DIM
                    ));
                    continue;
                }
                // A cut listing this task is membership, not work waiting on it
                // (ADR-0008), and `A1-125`'s rank excludes it for exactly that
                // reason. Unlabelled it would read as a dependent the rank forgot
                // to count, and a reader checking the order against this view
                // would conclude the order is wrong (`A1-126`).
                let membership = graph
                    .get(parent)
                    .is_some_and(|p| aplan_core::edge_kind(p, task) == EdgeKind::Membership);
                out.push(node_row(&indent, task, !up && membership));
                if expanded.insert(key) {
                    stack.push(Step::Expand(key, level + 1));
                }
            }
            Step::Expand(id, level) => {
                let next: Vec<&'a Task> = if up {
                    graph
                        .blockers_of(id)
                        .into_iter()
                        .filter_map(|b| graph.get(b))
                        .collect()
                } else {
                    dependents.get(id).cloned().unwrap_or_default()
                };
                let ghosts: Vec<&str> = if up {
                    graph.direct_unknown_blockers(id)
                } else {
                    Vec::new()
                };

                if depth.is_some_and(|d| level > d) {
                    // Counted after the walk, not here: what is hidden is what
                    // this view never printed, and a task below the cut may still
                    // appear elsewhere in the tree. Counting now would call a
                    // sibling three lines down "hidden".
                    pending.push((out.len(), id, level));
                    out.push(String::new());
                    continue;
                }
                // Pushed in reverse so they pop in id order, and so each child's
                // own subtree is printed before its next sibling.
                for g in ghosts.into_iter().rev() {
                    stack.push(Step::Ghost(g, level));
                }
                for task in next.into_iter().rev() {
                    stack.push(Step::Child {
                        task,
                        parent: id,
                        level,
                    });
                }
            }
        }
    }

    // Everything below the cut that this view never printed, not just the
    // children: on a 4000-long chain "1 more task" would be true of the next
    // level and wrong about the view, which is the wrong-noun defect this
    // repository keeps finding in its own messages (`A1-126`).
    for (at, id, level) in pending {
        let n = hidden_below(graph, dependents, id, up, seen);
        if n > 0 {
            let plural = if n == 1 { "" } else { "s" };
            out[at] = format!(
                "{}... {n} more task{plural} below, past --depth",
                "  ".repeat(level + 1)
            );
        }
    }
    out.retain(|l| !l.is_empty());
}

/// How many tasks a `--depth` limit is hiding below `id`, transitively.
///
/// Seeded from what the walk has already printed, so a task shown elsewhere in
/// the tree is not counted as hidden, and its own visited set makes a cyclic plan
/// terminate here as it does in the walk itself.
fn hidden_below<'a>(
    graph: &Graph<'a>,
    dependents: &std::collections::BTreeMap<&'a str, Vec<&'a Task>>,
    id: &'a str,
    up: bool,
    seen: &BTreeSet<&'a str>,
) -> usize {
    let mut visited: BTreeSet<&'a str> = seen.clone();
    let mut queue = std::collections::VecDeque::from([id]);
    let mut n = 0;
    while let Some(cur) = queue.pop_front() {
        let next: Vec<&'a str> = if up {
            graph
                .blockers_of(cur)
                .into_iter()
                .chain(graph.direct_unknown_blockers(cur))
                .collect()
        } else {
            dependents
                .get(cur)
                .into_iter()
                .flatten()
                .filter_map(|t| graph.by_id.get_key_value(t.id.as_str()).map(|(k, _)| *k))
                .collect()
        };
        for b in next {
            if visited.insert(b) {
                n += 1;
                queue.push_back(b);
            }
        }
    }
    n
}

/// A Mermaid-safe node key. Ids of alphanumerics, `_`, and interior `-` pass
/// through unchanged, so conventional `A0-001` ids render as themselves;
/// anything with a space, comma, paren, etc. is sanitised so the graph is never
/// syntactically invalid. (Distinct ids can collide once sanitised, acceptable
/// for a view.)
fn node_key(id: &str) -> Cow<'_, str> {
    if !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Cow::Borrowed(id)
    } else {
        Cow::Owned(
            id.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect(),
        )
    }
}

/// Make `s` safe to sit inside a Mermaid `["..."]` label.
///
/// A quote would close the label early; a newline would end the statement
/// mid-label and produce a graph Mermaid cannot parse. Titles are free text and
/// YAML permits both, so neither can be assumed away.
fn escape(s: &str) -> String {
    // A control character too, and this is the line that distinguishes a format
    // that escapes from one that does not: `json` and `sarif` render an escape
    // as `\u001b`, which is inert wherever it lands, so they keep the plan's
    // bytes. Mermaid has no such spelling, and `aplan graph` prints straight to
    // a terminal by default, so a title carrying an erase-display would run
    // there. Found by widening the leak census past the human formats
    // (`A1-289`, audit).
    sanitize::visible(s)
        .replace('"', "'")
        .replace(['\n', '\r'], " ")
        .trim()
        .to_string()
}

/// Render the roadmap (ADR-0009) as human text or JSON, returned as a `String`
/// so both the CLI and the `gen-roadmap` xtask can use it. Markdown, the
/// committed artifact, is [`render_roadmap_markdown`]. Hermetic: shows the cited
/// status and evidence, never resolved.
pub fn render_roadmap(roadmap: &Roadmap, format: Format) -> String {
    match format {
        Format::Human => roadmap_human(roadmap),
        _ => roadmap_json(roadmap),
    }
}

/// The two lines a `roadmap` opens with: what the report holds, and the one
/// assumption every duration below it divides by.
fn roadmap_header(r: &Roadmap) -> String {
    let mut s = String::new();
    // Releases borrow their cuts' work for their own rollup, so counting them
    // here would count that work twice (`A1-146`).
    let total: usize = r
        .cuts
        .iter()
        .filter(|c| !c.release)
        .map(|c| c.total)
        .sum::<usize>()
        + r.cross_cutting.len();
    // The effort a reader gets by adding the rows up, said once at the top so
    // the column can be reconciled against `summary` without arithmetic
    // (`A1-147`). Releases are skipped here as they are in the task count, for
    // the same reason: they borrow their cuts' work.
    // Cuts and releases counted apart, for the reason the task total already
    // separates them: a release is a checkpoint over cuts, and folding the two
    // into one number told `p00016` it had fourteen cuts when it had eleven
    // (`A1-240`).
    let releases = r.cuts.iter().filter(|c| c.release).count();
    let _ = writeln!(
        s,
        "roadmap: {} cut(s){}, {total} task(s), {:.1}d effort",
        r.cuts.len() - releases,
        if releases == 0 {
            String::new()
        } else {
            format!(", {releases} release(s)")
        },
        r.total_effort_days()
    );
    // Said once, at the top: every `w` below is this number divided into the
    // effort on its own row (`A1-147`, ADR-0023).
    if let Some(per_week) = r.effort_days_per_week {
        let _ = writeln!(s, "throughput: {per_week:.1} effort days a week");
    }
    s
}

/// One member row: id, status, then the title as the trailing column.
///
/// Extracted because `roadmap_human` rendered it identically in three places --
/// flat members, grouped members, and the cross-cutting list -- and folding it
/// made each one four lines longer (`A1-206`). One function, so the three cannot
/// drift and only one of them fold.
fn member_row(indent: &str, id_w: usize, m: &aplan_core::roadmap::RoadmapTask) -> String {
    fold::row(
        &format!(
            "{indent}{id}{:<id_w$}{id:#}  {st}{:<12}{st:#}  ",
            m.id,
            m.status.to_string(),
            id = crate::style::ID,
            st = crate::style::status(m.status.as_str()),
        ),
        &sanitize::visible(&m.title),
    )
}

fn roadmap_human(r: &Roadmap) -> String {
    let mut s = String::new();
    // One width for every member of every cut, so the rows of one report line up
    // with each other rather than each cut inventing its own column. Floored at
    // the width this has always used, for the reason `column` gives.
    let id_w = column(
        r.cuts
            .iter()
            .flat_map(|c| {
                c.members
                    .iter()
                    .chain(c.groups.iter().flat_map(|g| g.members.iter()))
            })
            .chain(r.cross_cutting.iter())
            .map(|m| m.id.as_str()),
        10,
    );
    s.push_str(&roadmap_header(r));
    for c in &r.cuts {
        let ver = if c.version.is_empty() {
            "?"
        } else {
            c.version.as_str()
        };
        let kind = if c.release { " (release)" } else { "" };
        // A cut's summary is one sentence of counts, so it folds at a hanging
        // indent of two rather than under its own version (`A1-206`).
        let _ = writeln!(
            s,
            "\n{}",
            fold::fold(
                &format!(
                    // **The heading is the version and the title together, and
                    // the rollup behind it is metadata** (`A1-296`). Before this
                    // the bold reached four characters of a hundred-character
                    // line -- the version alone -- so the thing a reader scrolls
                    // for was the least visible part of the line it names.
                    // Contrast from both directions: the heading takes the
                    // weight, the counts give it up.
                    "{hd}{ver}{kind}  {}{hd:#}  [{st}{}{st:#}]  {dim}{}/{} done, {} ready, \
                     {} blocked, crit {:.1}d, {:.1}d effort{}{dim:#}",
                    c.title,
                    c.status,
                    c.done,
                    c.total,
                    c.ready,
                    c.blocked,
                    c.critical_path.total_effort_days,
                    c.effort_days,
                    // `w` only where a throughput was declared, and never a
                    // second copy of a number: it is this row's own effort,
                    // divided (`A1-147`).
                    match r.weeks_of(c) {
                        (Some(weeks), Some(left)) =>
                            format!(", {}w ({}w left)", weeks_cell(weeks), weeks_cell(left)),
                        _ => String::new(),
                    },
                    hd = crate::style::HEADING,
                    st = crate::style::status(&c.status.to_string()),
                    dim = crate::style::DIM,
                ),
                fold::out_width(),
                2
            )
        );
        // A release closes cuts; it does not hold work. Its rollup is its cuts'
        // work borrowed (`A1-146`), and re-listing those rows put every task in
        // the document twice -- 152 of them on `p00016` -- while saying nothing
        // about the relationship the release actually has (`A1-240`).
        if c.release {
            for named in closes_of(r, c) {
                let _ = writeln!(s, "  closes {}", named.replace('`', ""));
            }
            continue;
        }
        // Grouped when the repository named a family, flat otherwise, so a plan
        // that declares nothing renders exactly as it always did (`A1-078`).
        if c.groups.is_empty() {
            for m in &c.members {
                // Title is the trailing column, folded under itself
                // (`A1-206`).
                let _ = writeln!(s, "{}", member_row("  ", id_w, m));
            }
        } else {
            for g in &c.groups {
                let _ = writeln!(s, "  {}", g.label.as_deref().unwrap_or("(no group)"));
                for m in &g.members {
                    let _ = writeln!(s, "{}", member_row("    ", id_w, m));
                }
            }
        }
    }
    if !r.cross_cutting.is_empty() {
        // Cross-cutting work is work, and it used to be the one section with no
        // effort on it: 21.0 of this repository's 185.5 effort days were held by
        // tasks in no cut, so the column added up to less than the plan with
        // nothing on the page to say why (`A1-147`).
        let weeks = weeks_cells(r.cross_cutting_weeks())
            .map_or_else(String::new, |(w, left)| format!(", {w}w ({left}w left)"));
        let _ = writeln!(
            s,
            "\ncross-cutting (in no cut): {:.1}d effort{weeks}",
            r.cross_cutting_effort_days
        );
        for m in &r.cross_cutting {
            let _ = writeln!(s, "{}", member_row("  ", id_w, m));
        }
    }
    let _ = writeln!(
        s,
        "\n{}",
        fold::note("(statuses are as claimed in the plan and gated by `aplan verify` in CI)")
    );
    s
}

fn roadmap_task_json(t: &RoadmapTask) -> serde_json::Value {
    serde_json::json!({
        "id": t.id,
        "title": t.title,
        "status": t.status.to_string(),
        "blocked_by": t.blocked_by,
        "evidence": t.evidence,
    })
}

fn roadmap_json(r: &Roadmap) -> String {
    let cut_json = |c: &Cut| {
        serde_json::json!({
            "version": c.version,
            "title": c.title,
            "milestone": c.milestone_id,
            "status": c.status.to_string(),
            // A release borrows the work of the cuts it closes (`A1-146`,
            // ADR-0022), so a consumer summing `total` across cuts has to skip
            // these rows. The model knows; hiding it here would make the machine
            // surface the only one that cannot tell (`A1-104`).
            "release": c.release,
            "total": c.total,
            "done": c.done,
            "ready": c.ready,
            "blocked": c.blocked,
            "effort_days": c.effort_days,
            "done_effort_days": c.done_effort_days,
            // Derived here rather than stored, and `null` where no throughput
            // was declared (`A1-147`, ADR-0023). Emitted so the machine surface
            // can answer "how long is this cut" without the consumer
            // reimplementing the division -- which is how the number drifted in
            // the first place (pre-public adoption evidence item 2).
            "weeks": r.weeks_of(c).0,
            "weeks_remaining": r.weeks_of(c).1,
            "critical_path_days": c.critical_path.total_effort_days,
            "critical_path": c.critical_path.tasks.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
            "closes": c.closes.clone(),
            "members": c.members.iter().map(roadmap_task_json).collect::<Vec<_>>(),
            // Empty unless the repository named a grouping family, so this is
            // additive for every existing consumer. Present either way, because
            // the human and markdown views group and a machine reader seeing a
            // flat list would be a second answer to one question (`A1-078`).
            "groups": c.groups.iter().map(|g| serde_json::json!({
                "label": g.label,
                "members": g.members.iter().map(roadmap_task_json).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    };
    let doc = serde_json::json!({
        "schema": "aplan/roadmap/v1",
        "effort_days_per_week": r.effort_days_per_week,
        "cuts": r.cuts.iter().map(cut_json).collect::<Vec<_>>(),
        "cross_cutting": r.cross_cutting.iter().map(roadmap_task_json).collect::<Vec<_>>(),
        // Siblings rather than a nested object, so a consumer reading
        // `cross_cutting` as a list still does (`A1-147`). `total_effort_days`
        // is what the rows add up to -- every cut that is not a release, plus
        // these -- and it agrees with `summary`.
        "cross_cutting_effort_days": r.cross_cutting_effort_days,
        "cross_cutting_done_effort_days": r.cross_cutting_done_effort_days,
        "cross_cutting_weeks": r.cross_cutting_weeks().0,
        "cross_cutting_weeks_remaining": r.cross_cutting_weeks().1,
        "total_effort_days": r.total_effort_days(),
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&doc).expect("roadmap json serialises")
    )
}

/// Render the roadmap as the committed `ROADMAP.status.md` (ADR-0009). Tables are
/// **non-aligned** (single-space cells, no column padding), so a content change
/// never re-pads, hence re-diffs, a neighbouring cell, the whole-file
/// `gen-roadmap --check` gate stays stable. The output is the canonical form: no
/// markdown formatter may rewrite it.
/// The cuts a release closes, named the way a reader looks them up: by label
/// where the cut has one, and by milestone id where it does not.
///
/// One function because two renderers print it, which is the drift a shared
/// resolver ends (`A1-139`, `A1-240`).
fn closes_of(roadmap: &Roadmap, release: &Cut) -> Vec<String> {
    release
        .closes
        .iter()
        .map(|closed| {
            let label = roadmap
                .cuts
                .iter()
                .find(|other| &other.milestone_id == closed)
                .map_or("", |other| other.version.as_str());
            if label.is_empty() {
                closed.clone()
            } else {
                format!("{label} (`{closed}`)")
            }
        })
        .collect()
}

pub fn render_roadmap_markdown(roadmap: &Roadmap, regenerate: Option<&str>) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "<!-- generated by `aplan roadmap --format markdown`; do not edit. -->"
    );
    // The line a consumer runs in *their* repository. It used to be this
    // repository's own self-hosting invocation, `cargo run -q -p aplan -- ... >
    // ROADMAP.status.md`, which is wrong twice in anyone else's tree: there is no
    // `-p aplan` package to run, and the path is wherever they keep the file. A
    // generated artifact whose regenerate line does not work is the exact failure
    // the banner convention exists to prevent, arriving from the tool
    // (`A1-059`, pre-public adoption evidence).
    //
    // So: the plain command by default, naming no path rather than the wrong one,
    // and redirection left to whoever knows where the file goes. A repository
    // whose generator is a wrapper says so with `--regenerate-with`, which is how
    // this repository's own file names `cargo xtask gen-roadmap` truthfully.
    let _ = writeln!(
        s,
        "<!-- regenerate: `{}` -->",
        regenerate.unwrap_or("aplan roadmap --format markdown")
    );
    let _ = writeln!(s, "\n# Roadmap\n\n## Cuts\n");
    // `effort` is always known; `weeks` exists only where the repository
    // declared a throughput, so a plan that declares none renders the table it
    // always did plus one column of numbers it already had (`A1-147`,
    // ADR-0023). The first cell is untouched either way, which is what ADR-0015
    // resolves a `traces` row anchor by.
    let weeks_column = roadmap.effort_days_per_week.is_some();
    let (head, rule) = if weeks_column {
        (
            "| cut | status | done/total | ready | blocked | effort | weeks | critical path |",
            "| --- | --- | --- | --- | --- | --- | --- | --- |",
        )
    } else {
        (
            "| cut | status | done/total | ready | blocked | effort | critical path |",
            "| --- | --- | --- | --- | --- | --- | --- |",
        )
    };
    let _ = writeln!(s, "{head}");
    let _ = writeln!(s, "{rule}");
    for c in &roadmap.cuts {
        let ver = if c.version.is_empty() {
            "?"
        } else {
            c.version.as_str()
        };
        // A release row rolls up the cuts it closes, so a reader adding the
        // column up needs to know which rows those are (`A1-146`).
        let ver = if c.release {
            format!("{ver} (release)")
        } else {
            ver.to_string()
        };
        // The cell the sixth pilot maintained by hand, computed from the cell
        // beside it: "11.5 weeks (8.0 left)" is arithmetic over this row, not a
        // range on a calendar, and it rescales with the plan because nothing
        // wrote it down (`A1-147`).
        let weeks = weeks_cells(roadmap.weeks_of(c))
            .map_or_else(String::new, |(w, left)| format!(" {w} ({left} left) |"));
        let weeks = if weeks_column { weeks } else { String::new() };
        let _ = writeln!(
            s,
            "| {} | {} | {}/{} | {} | {} | {:.1}d |{} {:.1}d |",
            md_cell(&ver),
            c.status,
            c.done,
            c.total,
            c.ready,
            c.blocked,
            c.effort_days,
            weeks,
            c.critical_path.total_effort_days
        );
    }
    for c in &roadmap.cuts {
        let ver = if c.version.is_empty() {
            "?"
        } else {
            c.version.as_str()
        };
        let _ = writeln!(s, "\n## {} {}\n", md_line(ver), md_line(&c.title));
        // A release closes cuts and holds no work of its own, so it names them
        // rather than repeating their rows: this document carried every task
        // twice otherwise, once under its cut and once under the release that
        // reaches it (`A1-240`).
        if c.release {
            for named in closes_of(roadmap, c) {
                let _ = writeln!(s, "Closes {}.", md_line(&named));
            }
            continue;
        }
        // One table per group where the repository named a family, one for the
        // cut otherwise, so an ungrouped plan's file is byte-identical to the
        // one it already committed (`A1-078`).
        if c.groups.is_empty() {
            roadmap_member_table(&mut s, &c.members);
        } else {
            for g in &c.groups {
                let _ = writeln!(
                    s,
                    "### {}\n",
                    md_line(g.label.as_deref().unwrap_or("(no group)"))
                );
                roadmap_member_table(&mut s, &g.members);
                let _ = writeln!(s);
            }
        }
    }
    if !roadmap.cross_cutting.is_empty() {
        let _ = writeln!(s, "\n## Cross-cutting (in no cut)\n");
        let weeks = weeks_cells(roadmap.cross_cutting_weeks())
            .map_or_else(String::new, |(w, left)| {
                format!(", {w} weeks ({left} left)")
            });
        let _ = writeln!(
            s,
            "{:.1}d effort{weeks}, held by tasks in no cut and not counted in the table above.\n",
            roadmap.cross_cutting_effort_days
        );
        roadmap_member_table(&mut s, &roadmap.cross_cutting);
    }
    // No `ADR-NNNN` citation here, deliberately (`A1-044`): this file is committed
    // in the *consumer's* repository, whose `docs/adr/` does not hold this
    // project's decision records, so the number would be a reference its only
    // reader cannot resolve. What guarantees the statuses is the sentence itself.
    let _ = writeln!(
        s,
        "\n> Statuses are as claimed in the plan and gated by `aplan verify` in CI."
    );
    s
}

fn roadmap_member_table(s: &mut String, members: &[RoadmapTask]) {
    let _ = writeln!(s, "| task | title | status | blocked_by | evidence |");
    let _ = writeln!(s, "| --- | --- | --- | --- | --- |");
    for m in members {
        let blocked_by = if m.blocked_by.is_empty() {
            "-".to_string()
        } else {
            md_cell(&m.blocked_by.join(", "))
        };
        let evidence = if m.evidence.is_empty() {
            "-".to_string()
        } else {
            md_cell(&m.evidence.join(", "))
        };
        let _ = writeln!(
            s,
            "| {} | {} | {} | {blocked_by} | {evidence} |",
            md_cell(&m.id),
            md_cell(&m.title),
            m.status
        );
    }
}

/// Escape a value for a markdown table cell: a raw `|` would add a spurious
/// column and a newline would split the row. Nothing validates ids, titles, or
/// evidence refs against these, and the markdown is a gated artifact, so a green
/// plan must never emit a broken table.
/// A derived duration, at one decimal, with the rounding that would lie spelled
/// out instead (`A1-147`).
///
/// `{:.1}` renders 0.018 weeks as `0.0`, and the audit found what that means on
/// a real plan: a cut with a half-day of work left reported `0.0w (0.0w left)`,
/// which reads as *finished*. There is no honest one-decimal rendering of "some,
/// but less than a tenth of a week", so it says `<0.1` and the reader knows the
/// difference between a little and none. The JSON is unrounded and unaffected.
/// A row's duration as two rendered cells -- the whole and what is left -- or
/// `None` where the repository declared no throughput (`A1-147`).
///
/// Every surface goes through here, so the rounding decision `weeks_cell` makes
/// is made once. Each caller keeps its own wording, because a terminal row and a
/// markdown sentence do not read the same.
fn weeks_cells(weeks: (Option<f64>, Option<f64>)) -> Option<(String, String)> {
    match weeks {
        (Some(w), Some(left)) => Some((weeks_cell(w), weeks_cell(left))),
        _ => None,
    }
}

fn weeks_cell(weeks: f64) -> String {
    if weeks > 0.0 && weeks < 0.05 {
        "<0.1".to_string()
    } else {
        format!("{weeks:.1}")
    }
}

fn md_cell(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(['\n', '\r'], " ")
}

/// Collapse newlines so a value stays on one line (for headings, where a raw `|`
/// is harmless and need not be escaped).
fn md_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    /// The text a renderer produced, without the styling it produced it with.
    ///
    /// A renderer embeds its palette unconditionally and the *stream* decides
    /// whether the bytes survive (`A1-286`), so a unit test holding a `String`
    /// sees escapes a terminal reader never would. Every assertion below is
    /// about layout or wording, so every one of them compares the stripped text.
    fn plain(text: &str) -> String {
        anstream::adapter::strip_str(text).to_string()
    }

    use super::*;
    use aplan_core::TaskRow;

    #[test]
    fn every_format_round_trips_through_its_name() {
        use std::str::FromStr as _;
        for f in Format::ALL {
            assert_eq!(
                Format::from_str(f.as_str()).as_ref(),
                Ok(f),
                "{f:?} does not round-trip through {:?}",
                f.as_str()
            );
        }
    }

    #[test]
    fn escape_data_maps_percent_cr_lf() {
        assert_eq!(escape_data("a%b\r\nc"), "a%25b%0D%0Ac");
        // A newline can no longer inject a second workflow command.
        assert!(!escape_data("x\n::error::forged").contains('\n'));
    }

    #[test]
    fn escape_property_also_maps_colon_and_comma() {
        assert_eq!(escape_property("A,B:c"), "A%2CB%3Ac");
    }

    #[test]
    fn github_line_escapes_injection() {
        let d = Diagnostic::error("APLAN003", "unknown `a\nb`").with_task("X,Y");
        let line = github_line(&d, false);
        assert!(
            !line.contains('\n'),
            "message newline must be escaped: {line}"
        );
        assert!(line.contains("X%2CY"), "task comma must be escaped: {line}");
        assert!(line.contains("%0A"));
    }

    #[test]
    fn node_key_preserves_ids_and_sanitises() {
        assert_eq!(&*node_key("A0-001"), "A0-001");
        assert_eq!(&*node_key("A B"), "A_B");
        assert_eq!(&*node_key("x,(y)"), "x__y_");
    }

    /// Regression: a newline in a title (YAML permits one) was emitted raw
    /// inside the `["..."]` label, producing a graph Mermaid cannot parse.
    #[test]
    fn escape_neutralises_quotes_and_newlines() {
        assert_eq!(escape("line one\nline two"), "line one line two");
        assert_eq!(escape("say \"hi\""), "say 'hi'");
        assert!(!escape("a\r\nb").contains(['\n', '\r']));
    }

    fn tsk(id: &str, status: aplan_core::task::Status, blocked: &[&str], tags: &[&str]) -> Task {
        Task {
            id: id.into(),
            title: format!("task {id}"),
            phase: 0,
            status,
            owner: None,
            effort_days: Some(1.0),
            blocked_by: blocked.iter().map(|s| (*s).to_string()).collect(),
            prefer_after: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: tags.iter().map(|s| (*s).to_string()).collect(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
        }
    }

    /// Both directions, the depth limit, and the two things the view must not
    /// quietly do: walk a cycle forever, or let a cut's membership edge read as
    /// work waiting on the task (`A1-126`).
    #[test]
    fn a_tree_shows_both_directions_and_says_where_it_stopped() {
        use aplan_core::task::Status;
        let tasks = [
            tsk("A-1", Status::Done, &[], &[]),
            tsk("A-2", Status::Todo, &["A-1"], &[]),
            tsk("A-3", Status::Todo, &["A-2"], &[]),
            tsk("A-4", Status::Todo, &["A-3"], &[]),
            tsk(
                "M-s0",
                Status::Todo,
                &["A-1", "A-2", "A-3", "A-4"],
                &["milestone"],
            ),
        ];
        let g = Graph::new(tasks.iter());

        let out = plain(&tree(&g, "A-2", None, false).expect("A-2 exists"));
        assert!(out.contains("waits on:"), "{out}");
        assert!(out.contains("A-1"), "{out}");
        assert!(out.contains("releases:"), "{out}");
        // Downward through the chain, and the cut labelled for what it is.
        assert!(out.contains("A-3"), "{out}");
        assert!(out.contains("A-4"), "{out}");
        assert!(
            out.contains("M-s0  todo  task M-s0  (the cut it is in)"),
            "{out}"
        );

        // A task `ready` would offer carries the rank it is offered under, which
        // is what makes the derived order checkable rather than asserted.
        assert!(
            plain(&tree(&g, "A-2", None, false).expect("A-2")).contains("rank: "),
            "{out}"
        );
        assert!(
            !tree(&g, "A-4", None, false)
                .expect("A-4")
                .contains("rank: "),
            "a blocked task has no place in an order over offered work"
        );

        // The depth limit says how much it is not showing.
        let shallow = tree(&g, "A-2", Some(1), false).expect("A-2");
        assert!(shallow.contains("A-3"), "{shallow}");
        assert!(
            !shallow.contains("A-4"),
            "depth 1 stops after A-3:\n{shallow}"
        );
        // The count is what the view did not print, transitively: `A-4` is below
        // the cut and `M-s0` is not, because it appears as a sibling.
        assert!(
            shallow.contains("... 1 more task below, past --depth"),
            "{shallow}"
        );

        // Nothing either side, said rather than left blank.
        let alone = plain(&tree(&g, "A-1", Some(0), false).expect("A-1"));
        assert!(alone.contains("waits on: nothing"), "{alone}");

        assert!(tree(&g, "NOPE", None, false).is_none());
    }

    /// Walking the tree must not scan the plan per node, which the first version
    /// of it did (`A1-126`).
    ///
    /// `dependents_of` was a full scan, so a 4000-task chain took `aplan tree`
    /// 7.7 seconds; inverting the graph once put it at 0.15s. The same shape of
    /// defect `A1-125` was audited for, one commit later, which is why the index
    /// is shared rather than local. A shape test, not a benchmark: the bound is
    /// far above what this costs when it is linear and far below the quadratic
    /// version.
    #[test]
    fn walking_a_long_chain_stays_linear() {
        use aplan_core::task::Status;
        let n = 3000;
        let tasks: Vec<Task> = (0..n)
            .map(|i| {
                let blocked: Vec<String> = if i > 0 {
                    vec![format!("T-{:05}", i - 1)]
                } else {
                    Vec::new()
                };
                let mut t = tsk(&format!("T-{i:05}"), Status::Todo, &[], &[]);
                t.blocked_by = blocked;
                t
            })
            .collect();
        let g = Graph::new(tasks.iter());

        let start = std::time::Instant::now();
        let out = tree(&g, "T-00000", None, false).expect("the head of the chain");
        let elapsed = start.elapsed();

        assert_eq!(
            out.lines().count(),
            n + 3,
            "every task below it, plus the root and the two headings"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "walking {n} tasks took {elapsed:?}, which is the quadratic shape back"
        );
    }

    /// The view shows the edges the tool *acts on*, including the ones a cut
    /// implies and no task declares (ADR-0013).
    ///
    /// A reader asking "what does finishing this release" is owed the answer
    /// `ready` uses. Nothing in `B-1`'s file mentions `M-s0`; the edge exists
    /// because `M-s1` follows `M-s0`, and it is exactly why `B-1` is not offered
    /// yet (`A1-126`).
    #[test]
    fn the_tree_follows_the_edges_a_cut_implies() {
        use aplan_core::task::Status;
        let tasks = [
            tsk("A-1", Status::Todo, &[], &[]),
            tsk("M-s0", Status::Todo, &["A-1"], &["milestone"]),
            tsk("B-1", Status::Todo, &[], &[]),
            tsk("M-s1", Status::Todo, &["B-1", "M-s0"], &["milestone"]),
        ];
        let g = Graph::new(tasks.iter());

        let out = tree(&g, "A-1", None, false).expect("A-1");
        assert!(
            out.contains("B-1"),
            "finishing `A-1` closes `M-s0`, which is what `B-1` waits on:\n{out}"
        );

        // And upwards from the other end, the same edge is the reason.
        let up = tree(&g, "B-1", None, false).expect("B-1");
        assert!(up.contains("M-s0"), "{up}");
        assert!(
            !up.contains("rank: "),
            "`B-1` waits on the cut before it, so it is not offered:\n{up}"
        );
    }

    /// A cyclic plan is `APLAN004`, which `check` reports and this view does not
    /// refuse: rendering it is how a reader sees the cycle. It must terminate.
    #[test]
    fn a_cycle_is_marked_rather_than_walked_forever() {
        use aplan_core::task::Status;
        let tasks = [
            tsk("A", Status::Todo, &["C"], &[]),
            tsk("B", Status::Todo, &["A"], &[]),
            tsk("C", Status::Todo, &["B"], &[]),
        ];
        let g = Graph::new(tasks.iter());
        let out = tree(&g, "A", None, false).expect("A exists");
        assert!(out.contains("(shown above)"), "{out}");
        assert!(out.lines().count() < 20, "it kept walking:\n{out}");
    }

    /// Scoping is what makes `graph` usable on a plan of any size, and both ways
    /// it can mislead are answered in Mermaid's own comment syntax, so what a
    /// reader pastes is still a diagram (`A1-126`).
    #[test]
    fn a_scoped_graph_notes_what_it_is_not_showing() {
        use aplan_core::task::Status;
        let tasks = [
            tsk("A-1", Status::Todo, &[], &[]),
            tsk("A-2", Status::Todo, &["A-1"], &[]),
            tsk("B-1", Status::Todo, &["A-2"], &[]),
        ];
        let g = Graph::new(tasks.iter());

        let whole = mermaid(&g);
        assert!(
            !whole.contains("%%"),
            "the whole plan hides nothing:\n{whole}"
        );

        let scope: BTreeSet<&str> = ["A-1", "A-2"].into_iter().collect();
        let out = mermaid_of(&g, &scope);
        assert!(out.contains("A-1 --> A-2"), "{out}");
        assert!(!out.contains("B-1"), "{out}");
        assert!(
            out.contains("%% 1 edge to a task outside this view"),
            "the edge to B-1 left the view and must be named:\n{out}"
        );

        // Boxes and no arrows reads as "these are unrelated", which is a claim.
        let lonely: BTreeSet<&str> = ["A-1"].into_iter().collect();
        let out = mermaid_of(&g, &lonely);
        assert!(out.contains("%% 1 task in view and no edges"), "{out}");
        // Still a diagram: every non-comment line is Mermaid.
        assert!(
            out.lines()
                .filter(|l| !l.starts_with("%%"))
                .all(|l| l.starts_with("graph ") || l.starts_with("    ")),
            "{out}"
        );
    }

    #[test]
    fn mermaid_output_is_single_line_per_node() {
        use aplan_core::task::{Status, Task};
        let t = Task {
            id: "N-001".into(),
            title: "line one\nline two".into(),
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
        let g = Graph::new([&t]);
        for line in mermaid(&g).lines().skip(1).filter(|l| !l.trim().is_empty()) {
            assert!(
                line.matches('"').count() % 2 == 0,
                "unbalanced quotes, label broke across lines: {line}"
            );
        }
    }

    fn rt(id: &str, title: &str) -> RoadmapTask {
        RoadmapTask {
            tags: Vec::new(),
            id: id.into(),
            title: title.into(),
            status: aplan_core::Status::Todo,
            blocked_by: vec![],
            evidence: vec![],
        }
    }

    /// A column grows for the value that needs it and never shrinks below what
    /// this table always printed (`A1-047`, found by its audit).
    #[test]
    fn a_column_is_the_widest_value_or_the_floor() {
        assert_eq!(column(["A1-047", "M-v0.7"].into_iter(), 12), 12);
        assert_eq!(column(std::iter::empty(), 12), 12);
        assert_eq!(
            column(["A1-01K9ZBW7YQ8N4TQGX2H6RC3PVD"].into_iter(), 12),
            29
        );
    }

    /// And the report that reads it: one long id must not shift its own row out
    /// of a table whose other rows are still aligned. Under `id_strategy: ulid`
    /// every id is 26 characters plus a prefix, so this is the ordinary case for
    /// a repository that turned the key on, not a pathological one.
    #[test]
    fn a_long_id_does_not_shift_the_column_beside_it() {
        let long = "T-01K9ZBW7YQ8N4TQGX2H6RC3PVD";
        let r = one_cut(
            "v0.1",
            "First",
            vec![rt("T-01", "short"), rt(long, "long"), rt("T-02", "short")],
        );
        let out = plain(&roadmap_human(&r));
        let rows: Vec<&str> = out
            .lines()
            .filter(|l| l.contains("todo") && l.starts_with("  "))
            .collect();
        assert_eq!(rows.len(), 3, "{out}");
        let status_at: Vec<usize> = rows
            .iter()
            .map(|l| l.find("todo").expect("the status column"))
            .collect();
        assert!(
            status_at.windows(2).all(|w| w[0] == w[1]),
            "the status column starts at {status_at:?}, so a row is out of line:\n{out}"
        );

        // The floor, which is what keeps this invisible to every plan that never
        // had a long id: the same report without one is unchanged.
        let short = plain(&roadmap_human(&one_cut(
            "v0.1",
            "First",
            vec![rt("T-01", "short"), rt("T-02", "short")],
        )));
        assert!(
            short.contains("  T-01        todo"),
            "the width a plan of ordinary ids renders at moved:\n{short}"
        );
    }

    /// Every surface says which rows are releases (`A1-146`, found by its
    /// audit).
    ///
    /// A release borrows the work of the cuts it closes, so a reader adding the
    /// column up, or a `jq` line summing `total`, counts that work twice unless
    /// the row says what it is. The model carried the flag and two of the three
    /// renderers dropped it.
    #[test]
    fn a_release_is_labelled_on_every_surface() {
        let mut r = one_cut("v0.1", "The release", vec![rt("T-01", "borrowed")]);
        r.cuts[0].release = true;

        let human = roadmap_human(&r);
        assert!(human.contains("(release)"), "{human}");

        let markdown = render_roadmap_markdown(&r, None);
        assert!(markdown.contains("(release)"), "{markdown}");

        let json: serde_json::Value =
            serde_json::from_str(&roadmap_json(&r)).expect("the json parses");
        assert_eq!(
            json["cuts"][0]["release"], true,
            "a machine reader cannot tell which rows to skip: {json}"
        );

        // And an ordinary cut says so too, rather than saying nothing.
        let plain = one_cut("s0", "A cut", vec![rt("T-02", "its own")]);
        assert!(!roadmap_human(&plain).contains("(release)"));
        let json: serde_json::Value =
            serde_json::from_str(&roadmap_json(&plain)).expect("the json parses");
        assert_eq!(json["cuts"][0]["release"], false);
    }

    /// A derived duration reaches every surface, or the surfaces disagree
    /// (`A1-147`; the lesson `A1-146` learned when two of three renderers stayed
    /// quiet about a release).
    /// Proves the named behaviour `rollup.carries_effort` (`A1-211`).
    #[test]
    fn a_derived_duration_is_on_every_surface() {
        let mut r = one_cut("v0.9", "A cut", vec![rt("T-01", "work")]);
        r.cuts[0].effort_days = 20.0;
        r.cuts[0].done_effort_days = 5.0;
        let r = r.with_throughput(Some(10.0));

        let human = roadmap_human(&r);
        assert!(
            human.contains("throughput: 10.0 effort days a week"),
            "the assumption is stated once: {human}"
        );
        assert!(
            human.contains("20.0d effort, 2.0w (1.5w left)"),
            "the human row carries the effort and its division: {human}"
        );

        let md = render_roadmap_markdown(&r, None);
        assert!(md.contains("| effort | weeks |"), "the column exists: {md}");
        assert!(
            md.contains("| 20.0d | 2.0 (1.5 left) |"),
            "the cell the sixth pilot maintained by hand: {md}"
        );

        let json: serde_json::Value =
            serde_json::from_str(&roadmap_json(&r)).expect("the json parses");
        assert_eq!(json["effort_days_per_week"], 10.0);
        assert_eq!(json["cuts"][0]["effort_days"], 20.0);
        assert_eq!(json["cuts"][0]["weeks"], 2.0);
        assert_eq!(json["cuts"][0]["weeks_remaining"], 1.5);
    }

    /// Work in no cut is work, and the column has to add up to the plan
    /// (`A1-147`). Found by the audit on this repository: 21.0 of 185.5 effort
    /// days sat in cross-cutting tasks and appeared in no row.
    #[test]
    fn work_in_no_cut_is_on_every_surface_too() {
        let mut r = one_cut("v0.9", "A cut", vec![rt("T-01", "work")]);
        r.cuts[0].effort_days = 20.0;
        r.cross_cutting = vec![rt("X-01", "a gate")];
        r.cross_cutting_effort_days = 5.0;
        let r = r.with_throughput(Some(10.0));

        let human = roadmap_human(&r);
        assert!(
            human.contains("25.0d effort"),
            "the header reconciles against `summary`: {human}"
        );
        assert!(
            human.contains("cross-cutting (in no cut): 5.0d effort, 0.5w (0.5w left)"),
            "{human}"
        );

        let md = render_roadmap_markdown(&r, None);
        assert!(
            md.contains("5.0d effort, 0.5 weeks (0.5 left), held by tasks in no cut"),
            "{md}"
        );

        let json: serde_json::Value =
            serde_json::from_str(&roadmap_json(&r)).expect("the json parses");
        assert_eq!(json["cross_cutting_effort_days"], 5.0);
        assert_eq!(json["cross_cutting_weeks"], 0.5);
        assert_eq!(json["total_effort_days"], 25.0);
    }

    /// Declare nothing and every surface says nothing about time -- but still
    /// reports the effort, which was always knowable.
    #[test]
    fn no_declared_throughput_means_no_duration_anywhere() {
        let mut r = one_cut("v0.9", "A cut", vec![rt("T-01", "work")]);
        r.cuts[0].effort_days = 20.0;

        let human = roadmap_human(&r);
        assert!(!human.contains("throughput"), "{human}");
        assert!(!human.contains("w ("), "no duration: {human}");
        assert!(
            human.contains("20.0d effort"),
            "the effort is still said: {human}"
        );

        let md = render_roadmap_markdown(&r, None);
        assert!(!md.contains("weeks"), "no column invented: {md}");
        assert!(md.contains("| 20.0d |"), "{md}");

        let json: serde_json::Value =
            serde_json::from_str(&roadmap_json(&r)).expect("the json parses");
        // `null`, not `0`: a consumer must be able to tell "not declared" from
        // "no work left".
        assert!(json["effort_days_per_week"].is_null());
        assert!(json["cuts"][0]["weeks"].is_null());
        assert!(json["cuts"][0]["weeks_remaining"].is_null());
    }

    fn one_cut(version: &str, title: &str, members: Vec<RoadmapTask>) -> Roadmap {
        Roadmap {
            cuts: vec![Cut {
                closes: Vec::new(),
                release: false,
                groups: Vec::new(),
                version: version.into(),
                title: title.into(),
                milestone_id: "M".into(),
                status: aplan_core::Status::Todo,
                total: members.len(),
                done: 0,
                ready: 0,
                blocked: members.len(),
                effort_days: 0.0,
                done_effort_days: 0.0,
                members,
                critical_path: CriticalPath {
                    total_effort_days: 0.0,
                    tasks: vec![],
                },
            }],
            cross_cutting: vec![],
            cross_cutting_effort_days: 0.0,
            cross_cutting_done_effort_days: 0.0,
            effort_days_per_week: None,
        }
    }

    /// The same cut, with its members under two headings and an unlabelled one.
    fn grouped_cut() -> Roadmap {
        let a = rt("T-01", "first");
        let b = rt("T-02", "second");
        let c = rt("T-03", "ungrouped");
        let mut r = one_cut("s0", "Cut 0", vec![a.clone(), b.clone(), c.clone()]);
        r.cuts[0].groups = vec![
            aplan_core::roadmap::CutGroup {
                label: Some("epic-a".into()),
                members: vec![a],
            },
            aplan_core::roadmap::CutGroup {
                label: Some("epic-b".into()),
                members: vec![b],
            },
            aplan_core::roadmap::CutGroup {
                label: None,
                members: vec![c],
            },
        ];
        r
    }

    #[test]
    fn a_grouped_cut_renders_its_headings_in_both_views() {
        // The corpus runs the binary in another process and this repository
        // declares no family, so neither grouped branch is reachable from a
        // committed plan (`A1-078`).
        let r = grouped_cut();
        let md = render_roadmap_markdown(&r, None);
        assert!(md.contains("### epic-a"), "{md}");
        assert!(md.contains("### epic-b"), "{md}");
        // Shown, not dropped: a task carrying no member of the family still
        // appears, under a heading that does not claim it has one.
        assert!(md.contains("### (no group)"), "{md}");
        assert!(md.contains("T-03"), "{md}");

        let human = render_roadmap(&r, Format::Human);
        assert!(human.contains("  epic-a"), "{human}");
        assert!(human.contains("(no group)"), "{human}");
        assert!(human.contains("T-03"), "{human}");

        // And the machine view agrees. Human and markdown grouping while JSON
        // showed a flat list would be two answers to one question, which is the
        // defect this project keeps finding in its own output.
        let json: serde_json::Value =
            serde_json::from_str(&render_roadmap(&r, Format::Json)).expect("valid json");
        let groups = json["cuts"][0]["groups"].as_array().expect("groups");
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0]["label"], "epic-a");
        assert!(groups[2]["label"].is_null(), "the ungrouped bucket");
        assert_eq!(groups[2]["members"][0]["id"], "T-03");
    }

    #[test]
    fn an_ungrouped_plan_still_carries_an_empty_groups_array() {
        // Additive: a consumer that never asked for groups sees the field and an
        // empty list, not a shape that changes depending on the config.
        let r = one_cut("v0.1", "Core", vec![rt("T-01", "first")]);
        let json: serde_json::Value =
            serde_json::from_str(&render_roadmap(&r, Format::Json)).expect("valid json");
        assert_eq!(json["cuts"][0]["groups"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn an_ungrouped_cut_renders_the_flat_list_it_always_did() {
        let r = one_cut("v0.1", "Core", vec![rt("T-01", "first")]);
        let md = render_roadmap_markdown(&r, None);
        assert!(!md.contains("###"), "no group headings: {md}");
        assert!(md.contains("T-01"), "{md}");
    }

    fn a_summary(dropped: usize, dropped_days: f64) -> Summary {
        Summary {
            phases: vec![aplan_core::summary::PhaseSummary {
                phase: 0,
                tasks: 2,
                done: 1,
                effort_days: 3.0,
                dropped,
                dropped_effort_days: dropped_days,
                done_effort_days: 1.0,
            }],
            ready: 1,
            effort_days_per_week: None,
        }
    }

    /// A little is not none, on any surface (`A1-147`).
    ///
    /// The audit ran a real plan through this: a cut holding a half-day of work
    /// at 27 effort days a week rendered `0.0w (0.0w left)`, which is the answer
    /// a finished cut gives. One decimal cannot say "some, but under a tenth",
    /// so it stops pretending to.
    #[test]
    fn a_duration_too_small_to_round_says_so_rather_than_zero() {
        assert_eq!(weeks_cell(0.018), "<0.1", "half a day at 27 a week");
        assert_eq!(weeks_cell(0.0), "0.0", "nothing left is still nothing left");
        assert_eq!(weeks_cell(0.05), "0.1", "the first value that rounds up");
        assert_eq!(weeks_cell(2.0), "2.0");

        let mut r = one_cut("v0.9", "A cut", vec![rt("T-01", "work")]);
        r.cuts[0].effort_days = 0.5;
        let r = r.with_throughput(Some(27.0));
        let human = roadmap_human(&r);
        assert!(
            human.contains("<0.1w (<0.1w left)"),
            "the human row must not read as finished: {human}"
        );
        let md = render_roadmap_markdown(&r, None);
        assert!(md.contains("| <0.1 (<0.1 left) |"), "{md}");
        // The machine surface is unrounded, so a consumer computing its own
        // rendering is never handed the lie in the first place.
        let json: serde_json::Value =
            serde_json::from_str(&roadmap_json(&r)).expect("the json parses");
        let weeks = json["cuts"][0]["weeks"].as_f64().expect("a number");
        assert!(weeks > 0.0 && weeks < 0.05, "unrounded: {weeks}");
    }

    /// The same, for `summary`. The rendered bytes are pinned by the corpus,
    /// which runs the real binary in a repository that declares a throughput
    /// (`tests/cmd/summary.trycmd`); what is asserted here is the arithmetic the
    /// renderer prints and the branch that prints nothing.
    #[test]
    fn summary_derives_a_duration_only_when_one_was_declared() {
        let quiet = a_summary(0, 0.0);
        assert_eq!(quiet.total_weeks(), None, "nothing declared, nothing said");
        render_summary(&quiet, Format::Human).expect("human");
        render_summary(&quiet, Format::Json).expect("json");

        // 3.0d of effort, 1.0d of it done, at 1.5 effort days a week.
        let declared = a_summary(0, 0.0).with_throughput(Some(1.5));
        assert_eq!(declared.total_weeks(), Some(2.0));
        assert_eq!(declared.remaining_weeks(), Some(4.0 / 3.0));
        render_summary(&declared, Format::Human).expect("human");
        render_summary(&declared, Format::Json).expect("json");
    }

    #[test]
    fn a_title_cannot_break_a_row() {
        // The JSON escapes a newline; the human table used to print it, splitting
        // one task across two lines and corrupting anything reading the columns.
        assert_eq!(
            one_line("first line\nsecond line"),
            "first line second line"
        );
        assert_eq!(one_line("a\tb"), "a b");
        assert_eq!(one_line("  padded  "), "padded");
    }

    #[test]
    fn tasks_render_in_both_formats() {
        // The corpus runs the binary in another process, where coverage cannot
        // see these lines, and the human table has branches the JSON does not:
        // an unestimated task, a milestone, and a task with no cut.
        let export = TaskExport {
            schema: "aplan/tasks/v1".to_string(),
            tasks: vec![
                TaskRow {
                    id: "A1".into(),
                    title: "estimated, in a cut, ready".into(),
                    phase: 0,
                    status: aplan_core::task::Status::Todo,
                    effort_days: Some(2.0),
                    owner: None,
                    tags: vec!["kind-b".into()],
                    blocked_by: Vec::new(),
                    cut: Some("s0".into()),
                    milestone: Some("M-s0".into()),
                    ready: true,
                    milestone_task: false,
                    rank: Some(1),
                    evidence: Vec::new(),
                    paths: Vec::new(),
                    traces: Vec::new(),
                    prefer_after: Vec::new(),
                    trace_fingerprints: std::collections::BTreeMap::new(),
                    revisit_at: None,
                    version_impact: None,
                    version_impact_reason: None,
                },
                TaskRow {
                    id: "B1".into(),
                    title: "unestimated, no cut, blocked".into(),
                    phase: 1,
                    status: aplan_core::task::Status::Todo,
                    effort_days: None,
                    owner: None,
                    tags: Vec::new(),
                    blocked_by: vec!["A1".into()],
                    cut: None,
                    milestone: None,
                    ready: false,
                    milestone_task: false,
                    rank: None,
                    evidence: Vec::new(),
                    paths: Vec::new(),
                    traces: Vec::new(),
                    prefer_after: Vec::new(),
                    trace_fingerprints: std::collections::BTreeMap::new(),
                    revisit_at: None,
                    version_impact: None,
                    version_impact_reason: None,
                },
                TaskRow {
                    id: "M-s0".into(),
                    title: "a checkpoint".into(),
                    phase: 0,
                    status: aplan_core::task::Status::Todo,
                    effort_days: None,
                    owner: None,
                    tags: vec!["milestone".into()],
                    blocked_by: Vec::new(),
                    cut: Some("s0".into()),
                    milestone: Some("M-s0".into()),
                    ready: false,
                    milestone_task: true,
                    rank: None,
                    evidence: Vec::new(),
                    paths: Vec::new(),
                    traces: Vec::new(),
                    prefer_after: Vec::new(),
                    trace_fingerprints: std::collections::BTreeMap::new(),
                    revisit_at: None,
                    version_impact: None,
                    version_impact_reason: None,
                },
            ],
        };
        render_tasks(&export, Format::Human).expect("human");
        render_tasks(&export, Format::Json).expect("json");
    }

    #[test]
    fn a_summary_renders_with_and_without_cancelled_work() {
        // The `dropped` line is conditional, and the corpus that exercises it
        // runs the binary in another process, where coverage cannot see it. Both
        // arms are rendered here so neither can rot unnoticed (`A1-074`).
        for (dropped, days) in [(0usize, 0.0), (2, 6.0)] {
            render_summary(&a_summary(dropped, days), Format::Human).expect("human");
            render_summary(&a_summary(dropped, days), Format::Json).expect("json");
        }
    }

    #[test]
    fn roadmap_markdown_escapes_pipes_and_newlines() {
        // A `|` or newline in an id/title/evidence would corrupt the gated table;
        // `check` accepts all of these, so the renderer must neutralise them.
        let m = RoadmapTask {
            tags: Vec::new(),
            id: "A|B".into(),
            title: "line1\nline2".into(),
            status: aplan_core::Status::Done,
            blocked_by: vec![],
            evidence: vec!["file:a|b.md".into()],
        };
        let md = render_roadmap_markdown(&one_cut("v0.1", "Core", vec![m]), None);
        assert!(
            md.contains(r"| A\|B | line1 line2 | done | - | file:a\|b.md |"),
            "cells must escape `|` and collapse newlines:\n{md}"
        );
        assert!(
            !md.contains("line1\nline2"),
            "a newline must not split a row"
        );
    }

    #[test]
    fn roadmap_markdown_carries_the_title_and_is_reflow_proof() {
        // The member table includes `title` (ADR-0009), and cells are non-aligned:
        // a long-id row must not pad a short-id row (or every content change
        // re-diffs the whole column and the byte-exact gate thrashes).
        let md = render_roadmap_markdown(
            &one_cut(
                "v0.1",
                "Core",
                vec![rt("A", "Short"), rt("A-really-quite-long-id", "Long")],
            ),
            None,
        );
        assert!(md.contains("| task | title | status | blocked_by | evidence |"));
        assert!(
            md.contains("| A | Short | todo | - | - |"),
            "the short-id row is not padded:\n{md}"
        );
    }

    #[test]
    fn roadmap_markdown_marks_a_versionless_cut() {
        let md = render_roadmap_markdown(&one_cut("", "Loose", vec![rt("A", "T")]), None);
        assert!(
            md.contains("| ? |"),
            "versionless cut shows `?` in the overview"
        );
        assert!(md.contains("## ? Loose"), "and in the heading");
    }
    use aplan_core::deprecation::{Fault, Runway, RunwayTask};

    fn runway_task(id: &str, status: Status, cut: Option<&str>) -> RunwayTask {
        RunwayTask {
            id: id.to_string(),
            title: format!("{id} title"),
            status,
            cut: cut.map(str::to_string),
            cited: false,
        }
    }

    /// The report a person reads: where it was announced, what removes it, and
    /// what is wrong with the pair.
    #[test]
    fn a_runway_reads_as_two_ends_and_the_gap_between_them() {
        let report = Deprecations {
            runways: vec![Runway {
                announced: runway_task("T-ANN", Status::Done, Some("v0.3")),
                removals: vec![runway_task("T-REM", Status::Todo, Some("v0.5"))],
                faults: Vec::new(),
            }],
            unannounced: Vec::new(),
        };
        let text = plain(&deprecations_human(&report));
        assert!(text.contains("announced in v0.3 (done)"), "{text}");
        assert!(text.contains("removed in v0.5 by T-REM (todo)"), "{text}");
        assert!(!text.contains(" ! "), "nothing is wrong with it: {text}");
        assert!(text.contains("never fails a build"), "{text}");
    }

    /// A `done` removal citing nothing is a claim with no proof, and the report
    /// says so where it is read rather than leaving it to a rule the plan may
    /// have made advisory.
    #[test]
    fn a_removal_claimed_done_without_evidence_says_so() {
        let report = Deprecations {
            runways: vec![Runway {
                announced: runway_task("T-ANN", Status::Done, Some("v0.3")),
                removals: vec![runway_task("T-REM", Status::Done, Some("v0.5"))],
                faults: Vec::new(),
            }],
            unannounced: Vec::new(),
        };
        assert!(
            deprecations_human(&report).contains("no evidence cited"),
            "{}",
            deprecations_human(&report)
        );

        let cited = Deprecations {
            runways: vec![Runway {
                announced: runway_task("T-ANN", Status::Done, Some("v0.3")),
                removals: vec![RunwayTask {
                    cited: true,
                    ..runway_task("T-REM", Status::Done, Some("v0.5"))
                }],
                faults: Vec::new(),
            }],
            unannounced: Vec::new(),
        };
        assert!(!deprecations_human(&cited).contains("no evidence cited"));
    }

    /// A task in no cut, or in a cut with no version label, is undateable, and
    /// the two read the same to somebody asking "which release".
    #[test]
    fn a_task_no_release_dates_says_no_cut() {
        let report = Deprecations {
            runways: vec![Runway {
                announced: runway_task("T-ANN", Status::Todo, None),
                removals: Vec::new(),
                faults: vec![Fault::NoRemovalScheduled],
            }],
            unannounced: vec![runway_task("T-LOOSE", Status::Todo, Some(""))],
        };
        let text = deprecations_human(&report);
        assert_eq!(text.matches("no cut").count(), 1, "{text}");
        assert!(text.contains("the removal never lands"), "{text}");
        assert!(text.contains("nobody was told about"), "{text}");
        assert!(text.contains("1 runway(s), 1 removal(s)"), "{text}");
    }

    #[test]
    fn a_plan_with_no_runway_is_told_how_to_write_one() {
        let none = Deprecations {
            runways: Vec::new(),
            unannounced: Vec::new(),
        };
        let text = deprecations_human(&none);
        assert!(text.contains("declares no deprecation runway"), "{text}");
        assert!(text.contains("`blocked_by`"), "{text}");
        assert!(
            deprecations_markdown(&none).contains("None announced."),
            "the published table says so rather than printing empty headers"
        );
    }

    /// The table a project publishes. `Status` is derived from the two ends
    /// rather than declared, so it cannot disagree with them.
    #[test]
    fn the_published_table_derives_its_status_from_both_ends() {
        let rows = |announced: Status, removal: Option<Status>| {
            deprecations_markdown(&Deprecations {
                runways: vec![Runway {
                    announced: runway_task("T-ANN", announced, Some("v0.3")),
                    removals: removal
                        .map(|s| vec![runway_task("T-REM", s, Some("v0.5"))])
                        .unwrap_or_default(),
                    faults: Vec::new(),
                }],
                unannounced: Vec::new(),
            })
        };
        assert!(rows(Status::Done, Some(Status::Done)).contains("| removed |"));
        assert!(rows(Status::Done, Some(Status::Todo)).contains("| announced |"));
        assert!(rows(Status::Todo, Some(Status::Todo)).contains("| planned |"));
        // Announced, with nothing scheduled: the cell says that rather than
        // leaving the column blank.
        let unscheduled = rows(Status::Done, None);
        assert!(unscheduled.contains("| not scheduled |"), "{unscheduled}");
        assert!(unscheduled.contains("do not edit"), "{unscheduled}");
    }

    /// A removal done in pieces is several removals against one announcement,
    /// and every one of them is a release somebody has to know about.
    #[test]
    fn an_announcement_may_have_more_than_one_removal() {
        let report = Deprecations {
            runways: vec![Runway {
                announced: runway_task("T-ANN", Status::Done, Some("v0.3")),
                removals: vec![
                    runway_task("T-R1", Status::Done, Some("v0.5")),
                    runway_task("T-R2", Status::Todo, Some("v0.6")),
                ],
                faults: Vec::new(),
            }],
            unannounced: Vec::new(),
        };
        let text = deprecations_human(&report);
        assert!(text.contains("removed in v0.5 by T-R1"), "{text}");
        assert!(text.contains("removed in v0.6 by T-R2"), "{text}");
        let md = deprecations_markdown(&report);
        assert!(
            md.contains("| v0.5, v0.6 |"),
            "both releases in the row: {md}"
        );
        assert!(
            md.contains("| announced |"),
            "not `removed` until every piece is: {md}"
        );
    }

    /// Every fault carries its own sentence, and each says what the reader must
    /// do something about.
    #[test]
    fn every_fault_has_a_sentence_of_its_own() {
        // `Fault::ALL`, not a copy of it here: a variant added to the enum and
        // not to a hand-written array in a test is a variant nothing checks.
        let mut seen: Vec<&str> = Vec::new();
        for &f in Fault::ALL {
            let what = f.what();
            assert!(what.len() > 20, "{what}");
            assert!(!seen.contains(&what), "two faults share a sentence: {what}");
            assert!(
                !seen.contains(&f.kind()),
                "two faults share a machine name: {}",
                f.kind()
            );
            seen.push(what);
            seen.push(f.kind());
            let text = deprecations_human(&Deprecations {
                runways: vec![Runway {
                    announced: runway_task("T-ANN", Status::Todo, Some("v0.3")),
                    removals: Vec::new(),
                    faults: vec![f],
                }],
                unannounced: Vec::new(),
            });
            assert!(text.contains(what), "{text}");
        }
    }

    fn breadth(id: &str, hits: usize, percent: u32) -> PathBreadth {
        PathBreadth {
            id: id.to_string(),
            title: id.to_string(),
            hits,
            percent,
            first_path: "src/**".to_string(),
        }
    }

    /// A published field called `median_percent` has to be the median. The upper
    /// of the two middles is a different statistic, and on the even counts a
    /// small plan actually has it reads noticeably high.
    #[test]
    fn the_median_of_an_even_number_of_rows_is_between_the_two_middles() {
        let even = PathsReport {
            sampled: 100,
            asked_for: 100,
            tasks: vec![
                breadth("A", 0, 0),
                breadth("B", 20, 20),
                breadth("C", 40, 40),
                breadth("D", 100, 100),
            ],
            without_paths: 0,
            unmeasurable: 0,
        };
        assert_eq!(even.median_percent(), Some(30), "between 20 and 40");

        let odd = PathsReport {
            tasks: vec![
                breadth("A", 0, 0),
                breadth("B", 20, 20),
                breadth("C", 90, 90),
            ],
            ..even.clone()
        };
        assert_eq!(odd.median_percent(), Some(20), "the middle row itself");
    }

    /// Zero sampled is not a median of zero: there is no measurement to take a
    /// middle of, and a `0` here reads as "the typical task's paths are touched
    /// by nothing", which is a finding this never made.
    #[test]
    fn a_median_of_nothing_is_absent_rather_than_zero() {
        let nothing = PathsReport {
            sampled: 0,
            asked_for: 100,
            tasks: Vec::new(),
            without_paths: 0,
            unmeasurable: 0,
        };
        assert_eq!(nothing.median_percent(), None);
        let text = paths_human(&nothing);
        assert!(text.contains("no commit was sampled"), "{text}");
        assert!(!text.contains('%'), "{text}");
    }

    /// A plan whose every `paths` entry is a broken glob has rows -- it just has
    /// no measurable ones -- so "no task declares `paths`" would be false.
    #[test]
    fn broken_globs_are_not_reported_as_a_plan_without_paths() {
        let broken = PathsReport {
            sampled: 10,
            asked_for: 100,
            tasks: Vec::new(),
            without_paths: 0,
            unmeasurable: 3,
        };
        let text = paths_human(&broken);
        assert!(text.contains("this tool can compile"), "{text}");
        assert!(text.contains("APLAN032"), "{text}");
        assert!(
            !text.contains("no task declares"),
            "three tasks do declare `paths`: {text}"
        );
    }

    /// `weeks_cell`'s `<0.1`, for the same reason.
    #[test]
    fn a_share_that_rounds_to_nothing_is_not_printed_as_nothing() {
        assert_eq!(share_cell(0, 0), "0%");
        assert_eq!(share_cell(1, 0), "<1%");
        assert_eq!(share_cell(1, 1), "1%");
        assert_eq!(share_cell(100, 100), "100%");

        let report = PathsReport {
            sampled: 300,
            asked_for: 300,
            tasks: vec![breadth("T-RARE", 1, 0), breadth("T-NEVER", 0, 0)],
            without_paths: 0,
            unmeasurable: 0,
        };
        let text = paths_human(&report);
        assert!(text.contains("<1%"), "{text}");
        assert!(
            text.contains("0%"),
            "and a true zero still reads as zero: {text}"
        );
    }
}

#[cfg(test)]
mod sarif_tests {
    use super::*;
    use aplan_core::code;

    fn report() -> Report {
        Report {
            tasks_checked: 2,
            diagnostics: vec![
                Diagnostic::error(
                    code::DONE_WITHOUT_EVIDENCE,
                    "`status: done` with no `evidence`",
                )
                .with_task("S-1")
                .with_file(std::path::Path::new("plan/tasks/S-1.md")),
                Diagnostic::warning(code::MISSING_EFFORT, "no `effort_days` estimate")
                    .with_task("S-2"),
            ],
        }
    }

    /// A suppressed result is still valid SARIF, which is the whole reason to use
    /// the mechanism rather than invent a property: a dashboard already knows what
    /// `suppressions` means, so an advisory finding arrives as accepted rather
    /// than as an alert nobody can dismiss (`A1-112`).
    #[test]
    fn a_suppressed_result_is_valid_sarif_and_keeps_the_rule_severity() {
        let schema_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/sarif-2.1.0.schema.json");
        let schema: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&schema_path).expect("vendored schema"))
                .expect("schema is JSON");
        let validator = jsonschema::validator_for(&schema).expect("schema compiles");

        let policy = Verdict {
            deny_warnings: false,
            advisory: vec![code::DONE_WITHOUT_EVIDENCE.to_string()],
        };
        let doc = sarif_document(&report(), &policy);
        let errors: Vec<String> = validator.iter_errors(&doc).map(|e| e.to_string()).collect();
        assert!(
            errors.is_empty(),
            "not valid SARIF 2.1.0:\n{}",
            errors.join("\n")
        );

        let results = doc["runs"][0]["results"].as_array().expect("results");
        // Exactly the downgraded one, and not the finding beside it.
        let suppressed: Vec<&str> = results
            .iter()
            .filter(|r| r.get("suppressions").is_some())
            .map(|r| r["ruleId"].as_str().expect("ruleId"))
            .collect();
        assert_eq!(suppressed, vec![code::DONE_WITHOUT_EVIDENCE]);
        // The level is the rule's, suppressed or not: severity is what the rule
        // says and tolerance is what the repository chose.
        assert_eq!(results[0]["level"], "error");
        assert_eq!(
            doc["runs"][0]["tool"]["driver"]["rules"]
                .as_array()
                .expect("rules")
                .iter()
                .find(|r| r["id"] == code::DONE_WITHOUT_EVIDENCE)
                .expect("the rule")["defaultConfiguration"]["level"],
            "error",
            "the catalogue must not learn a repository's tolerance"
        );
    }

    /// A diagnostic about the plan as a whole carries no task, and the title
    /// then names the code alone. It used to read `APLAN029 -`, which looks like
    /// a title that was truncated; `APLAN029` is the first rule to produce a
    /// task-less finding in practice, so nothing had ever rendered it.
    #[test]
    fn an_annotation_for_a_plan_wide_finding_titles_the_code_alone() {
        let d = Diagnostic::warning(code::UNUSED_TAG_FAMILY, "no task carries the `epic` family");
        assert_eq!(
            github_line(&d, false),
            "::warning title=APLAN029::no task carries the `epic` family"
        );
        // And a finding that does name a task is unchanged.
        let with_task =
            Diagnostic::warning(code::MISSING_EFFORT, "no `effort_days` estimate").with_task("A-1");
        assert_eq!(
            github_line(&with_task, false),
            "::warning title=APLAN009 A-1::no `effort_days` estimate"
        );
    }

    /// The annotation has no suppression concept, so the disclosure is in the
    /// title. Asserted here rather than only end-to-end because the escaping is
    /// this function's job: a title is a workflow-command *property*, and an
    /// unescaped comma in it corrupts the whole annotation.
    #[test]
    fn an_advisory_annotation_says_so_in_its_title_and_keeps_its_level() {
        let d = Diagnostic::error(code::MEMBERSHIP_TOTALITY, "belongs to no cut").with_task("A-1");
        assert_eq!(
            github_line(&d, true),
            "::error title=APLAN022 A-1 (advisory)::belongs to no cut"
        );
        assert_eq!(
            github_line(&d, false),
            "::error title=APLAN022 A-1::belongs to no cut"
        );
    }

    /// The acceptance criterion, machine-checked rather than asserted: the
    /// document really is SARIF 2.1.0. The schema is vendored under
    /// `tests/data/` so this stays hermetic; a gate that needs the network is a
    /// gate that fails when the network does.
    #[test]
    fn the_document_validates_against_the_official_sarif_schema() {
        let schema_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/sarif-2.1.0.schema.json");
        let schema: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&schema_path).expect("vendored schema"))
                .expect("schema is JSON");
        let validator = jsonschema::validator_for(&schema).expect("schema compiles");

        let doc = sarif_document(&report(), &Verdict::default());
        let errors: Vec<String> = validator.iter_errors(&doc).map(|e| e.to_string()).collect();
        assert!(
            errors.is_empty(),
            "not valid SARIF 2.1.0:\n{}",
            errors.join("\n")
        );
    }

    /// An empty run is the common case in a healthy repository, and it must
    /// still be a valid document that tells a dashboard what this tool checks.
    #[test]
    fn a_clean_report_is_still_valid_and_still_carries_the_rule_catalog() {
        let clean = Report {
            diagnostics: vec![],
            tasks_checked: 5,
        };
        let doc = sarif_document(&clean, &Verdict::default());
        assert_eq!(doc["runs"][0]["results"].as_array().unwrap().len(), 0);
        assert_eq!(
            doc["runs"][0]["tool"]["driver"]["rules"]
                .as_array()
                .unwrap()
                .len(),
            Rule::ALL.len(),
            "the catalog is every rule, not just the ones that fired"
        );
    }

    #[test]
    fn every_result_rule_id_resolves_to_a_catalogued_rule() {
        // A `ruleId` with no matching rule renders in a dashboard as an
        // unexplained code, which is worse than no result at all.
        let doc = sarif_document(&report(), &Verdict::default());
        let ids: Vec<&str> = doc["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        for result in doc["runs"][0]["results"].as_array().unwrap() {
            let id = result["ruleId"].as_str().unwrap();
            assert!(ids.contains(&id), "ruleId {id} is not in the catalog");
        }
    }

    #[test]
    fn a_relative_path_stays_a_relative_uri_and_an_absolute_one_becomes_file() {
        // A dashboard maps a result onto the repository by a relative URI, so a
        // relative path must stay one. An absolute path is not repo-relative and
        // cannot be faked into one, so it is emitted as a well-formed `file://`
        // URI instead of the malformed `//tmp/...` a naive join produced.
        assert_eq!(
            sarif_uri(std::path::Path::new("aplan/tasks/A-1.md")),
            "aplan/tasks/A-1.md"
        );
        assert_eq!(
            sarif_uri(std::path::Path::new("/tmp/plan/tasks/A-1.md")),
            "file:///tmp/plan/tasks/A-1.md"
        );
        // The original bug: a bare `//tmp/...`, which is a network-path URI
        // reference, not the file location meant.
        assert!(!sarif_uri(std::path::Path::new("/tmp/a.md")).starts_with("//"));
    }

    #[test]
    fn severity_and_location_survive_the_mapping() {
        let doc = sarif_document(&report(), &Verdict::default());
        let results = doc["runs"][0]["results"].as_array().unwrap();

        assert_eq!(results[0]["level"], "error");
        assert_eq!(results[0]["ruleId"], code::DONE_WITHOUT_EVIDENCE);
        assert_eq!(
            results[0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
            "plan/tasks/S-1.md"
        );
        // The task id leads the message, because a SARIF result has no field for it.
        assert!(
            results[0]["message"]["text"]
                .as_str()
                .unwrap()
                .starts_with("S-1: ")
        );

        assert_eq!(results[1]["level"], "warning");
        // A diagnostic with no file is legal SARIF and honest: it is about the
        // plan, not a location in it.
        assert!(results[1].get("locations").is_none());
    }
}

/// `a`, `a` or `b`, `a`, `b` or `c` -- backticked, in the plan's own order.
fn list_of(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => format!("`{one}`"),
        [rest @ .., last] => format!(
            "{} or `{last}`",
            rest.iter()
                .map(|i| format!("`{i}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// What a repository's agent contract says, assembled from its policy rather
/// than from its work (`A1-038`).
///
/// Every field is a *policy* input: the plan's location, the evidence a `done`
/// task must carry, the runners that resolve a `test` citation, the tools an
/// agent has. None of it moves when a task is closed, which is the property that
/// makes the generated file gate-able -- a block that changed whenever work
/// happened would fail its own drift check daily and be turned off within a
/// week, which is what happened to the hand-maintained status this project
/// already had to delete (ADR-0009).
#[derive(Debug, Clone, Default)]
pub struct AgentContract {
    /// Where the task files live (`root`), relative to the repository, as an
    /// adopter would type it. `None` when they live at the repository root,
    /// which is a different *sentence* rather than a different path -- the
    /// audit found `the repository root/` inside backticks, which is what
    /// passing prose through a path field produces.
    pub plan_dir: Option<String>,
    /// The id shapes this plan currently mints, with the number left as `nn`.
    /// Empty when the plan teaches no convention, where inventing one would be
    /// worse than saying nothing (`A1-047`).
    ///
    /// A list because a plan may genuinely run two conventions at once, and
    /// naming the larger made this field move when ordinary work changed a count
    /// -- which is the invariant above, broken by the one field that was a
    /// tally rather than a policy (`A1-180`).
    pub id_examples: Vec<String>,
    /// `evidence.require_min_strength`, in the order the repository wrote it.
    pub min_strength: Vec<String>,
    /// `evidence.exempt_tags`.
    pub exempt_tags: Vec<String>,
    /// Evidence policy scoped to part of the plan, as `(subset, requirement)`
    /// pairs in the order the repository wrote them (`A1-183`).
    ///
    /// Empty for a repository whose policy is whole-plan, which is what every
    /// contract said before scoping existed. Carried because an agent told one
    /// sentence where two apply is told the wrong rule for the subset it is
    /// working in, and this file is the only thing it reads.
    pub scoped_evidence: Vec<(String, String)>,
    /// The policy switches this repository has on.
    ///
    /// A set rather than five `bool` fields: what the renderer asks is "is this
    /// one on", and five parallel booleans in a struct is the shape where a
    /// caller eventually passes two of them in the wrong order.
    pub switches: BTreeSet<Switch>,
    /// Rule codes this repository downgraded with `advisory`, in the order it
    /// wrote them.
    ///
    /// Needed because `deny_warnings` and `advisory` compose, and the sentence
    /// pair about the warning tier read as though they could not: *there is no
    /// advisory tier* is false for a repository running both, which is a
    /// supported migration posture with a test of its own (`A1-272`'s audit).
    pub advisory: Vec<String>,
    /// Configured test runner *names*, which is what a `test` citation resolves
    /// against. A repository with one runner may leave it unnamed, so this can
    /// be empty while [`runner_count`](Self::runner_count) is not.
    pub runners: Vec<String>,
    /// How many runners are configured at all.
    ///
    /// Kept separately because the audit found the single-runner form -- the one
    /// an adopter starts with -- producing a block that said nothing about how a
    /// `test` citation resolves, which is the sentence the whole file exists to
    /// carry.
    pub runner_count: usize,
    /// The MCP tools this build serves, `(name, one line)`. Empty when the
    /// binary has no MCP surface.
    pub tools: Vec<(String, String)>,
}

/// A policy switch a repository either has on or does not (`A1-038`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Switch {
    /// `deny_warnings`: every finding blocks.
    DenyWarnings,
    /// `detect_drift`: closing stamps `traces`, and a later edit reports.
    DetectDrift,
    /// `require_cut_membership`: every task is in exactly one cut.
    RequireCutMembership,
    /// `require_effort`: every live task carries an estimate. On by default.
    RequireEffort,
    /// `url.check` is configured, so a cited url is resolved rather than trusted.
    UrlCheck,
}

/// The width this tool renders to, for help and for its own output alike.
///
/// One number so `--help` and a finding cannot disagree about how wide the
/// terminal is (`A1-205`, and `A1-206` reads it for the human renderer).
///
/// **Ninety-five, matching the prose beside it.** The generated `AGENTS.md`
/// block already wraps here, and the documents in this repository are written to
/// about the same, so a reader moving between a task file, a rendered finding
/// and `--help` sees one measure rather than three.
///
/// It is a *cap*, not a fixed width: a narrower terminal gets its own width, and
/// a wider one stops here rather than running to the edge, because a line of
/// prose two hundred columns long is harder to read, not easier.
pub const MAX_WIDTH: usize = 95;

pub mod color;
pub mod fold;
pub mod lanes;
pub mod sanitize;
pub mod style;

/// The markers a generated block is fenced by. A repository keeps its own prose
/// outside them; everything between them is this command's (`A1-038`).
///
/// **Re-exported, not restated.** `aplan-core`'s `contract` module says in its
/// own header that it holds the markers because "a marker pair with two readers
/// is how the block a tool owns and the block it validates come to disagree" --
/// and there was a byte-identical second copy of both here, which is what the
/// writer and the `--check` gate actually read (`A1-294`, audit).
pub use aplan_core::contract::{BEGIN as AGENTS_BEGIN, END as AGENTS_END};

/// Wrap a paragraph at 95 columns, on word boundaries.
///
/// A generated block is a committed file with a drift gate on it, so the shape
/// of its lines is a diff decision rather than a cosmetic one: one 400-character
/// paragraph re-diffs whole when a single word changes, and a reviewer then
/// cannot see what changed. 95 is what the documents beside it use.
fn wrap(text: &str) -> String {
    let mut out = String::new();
    let mut width = 0;
    for word in text.split_whitespace() {
        let len = word.chars().count();
        if width > 0 && width + 1 + len > 95 {
            out.push('\n');
            width = 0;
        } else if width > 0 {
            out.push(' ');
            width += 1;
        }
        out.push_str(word);
        width += len;
    }
    out
}

/// The first sentence of a tool's description.
///
/// The list is a *surface*, not a manual: `aplan_close` describes itself in five
/// sentences, which is right in a tool listing an agent queries and wrong in a
/// file a human reads to learn the rules. The tool itself still says everything.
fn first_sentence(text: &str) -> &str {
    match text.find(". ") {
        Some(at) => &text[..=at],
        None => text,
    }
}

/// Render the agent contract as the block that goes in `AGENTS.md`.
///
/// Written to be read by an agent that has never seen this tool: the rule first,
/// the loop second, and the repository's own policy stated as *its* policy
/// rather than as the tool's defaults. What it deliberately does not carry is
/// anything a project knows and the plan does not -- build commands,
/// architecture, the CI matrix -- which is why it is fenced rather than being
/// the whole file (pre-public adoption evidence section 5).
#[must_use]
pub fn render_agents_md(c: &AgentContract) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "{AGENTS_BEGIN}");
    let _ = writeln!(s, "\n## The plan is the source of truth\n");
    let _ = writeln!(
        s,
        "{}\n",
        wrap(&format!(
            "Planning, tasking and status live {}, and what judges them is `aplan check` \
             followed by `aplan verify`. Not in issues, not in a TODO comment, not in prose: a \
             document that carries status becomes a second copy of it, and the second copy \
             drifts.",
            match c.plan_dir.as_deref().map(|d| d.trim_end_matches('/')) {
                Some(dir) if !dir.is_empty() => format!("under `{dir}/`"),
                _ => "at the repository root".to_string(),
            }
        ))
    );
    if !c.id_examples.is_empty() {
        let shapes = list_of(&c.id_examples);
        let convention = if c.id_examples.len() > 1 {
            format!("this plan's own conventions ({shapes})")
        } else {
            format!("this plan's own convention ({shapes})")
        };
        let _ = writeln!(
            s,
            "{}\n",
            wrap(&format!(
                "Task ids follow {convention}. If work does not fit an existing task, **write \
                 the task first**, then do the work: a change with no task is a change nobody \
                 can find later."
            ))
        );
    }

    let _ = writeln!(s, "## The loop\n");
    let _ = writeln!(s, "```sh");
    let _ = writeln!(
        s,
        "aplan ready                 # what can be started now: blockers all done, in a derived order"
    );
    // One shape for the command line, and the first is a stable choice because
    // the list is keyed by stem: a plan running two conventions still needs a
    // single word here, and the sentence above has already named both.
    let example = c.id_examples.first().map_or("<id>", String::as_str);
    let _ = writeln!(
        s,
        "aplan why {example:<17} # why something cannot be started yet"
    );
    let _ = writeln!(s, "# ... implement, commit ...");
    let _ = writeln!(
        s,
        "aplan close {example} --evidence commit:COMMIT_SHA --evidence test:REGISTERED_TEST_NAME"
    );
    let _ = writeln!(s, "aplan check && aplan verify # what CI will run");
    let _ = writeln!(s, "```\n");
    let _ = writeln!(
        s,
        "{}\n",
        wrap(
            "**A status is a claim, and `close` requires proof of it.** Evidence is a citation \
             something else resolves: a commit whose diff touched this task's declared `paths`, \
             or a test a runner can list and run. There is no \"mark it done\". If a task cannot \
             be closed honestly, it is not done, and saying so beats finding a weaker citation."
        )
    );

    let _ = writeln!(s, "## What closes a task here\n");
    let _ = writeln!(s, "{}", policy_lines(c));

    if !c.tools.is_empty() {
        let _ = writeln!(s, "## Through MCP\n");
        let _ = writeln!(
            s,
            "{}\n",
            wrap(
                "`aplan mcp` serves these tools, and they go through the same gates as the CLI, \
                 so they refuse what it refuses. Prefer them over shelling out."
            )
        );
        for (name, about) in &c.tools {
            let _ = writeln!(
                s,
                "{}",
                bullet(&format!("`{name}` -- {}", first_sentence(&one_line(about))))
            );
        }
        let _ = writeln!(s);
    }

    let _ = writeln!(s, "{AGENTS_END}");
    s
}

/// `a`, `a or b`, `a, b or c`. Written out because `join(" or ")` produced
/// "commit or test or file", which reads like a tool talking to itself.
fn english_list(items: &[String]) -> String {
    joined(items, "or")
}

/// The same, with the conjunction the sentence needs.
///
/// A list of alternatives takes *or*; a list of things that are all true takes
/// *and*. Every downgraded rule is downgraded, and "`A` or `B` are downgraded"
/// is a plural verb over a disjunction (`A1-272`'s second audit).
fn joined(items: &[String], conjunction: &str) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} {conjunction} {last}", rest.join(", ")),
    }
}

/// A bullet, wrapped and indented so its continuation lines sit under its text.
fn bullet(text: &str) -> String {
    let wrapped = wrap(text);
    let mut lines = wrapped.lines();
    let mut out = format!("- {}", lines.next().unwrap_or_default());
    for line in lines {
        out.push_str("\n  ");
        out.push_str(line);
    }
    out
}

/// The repository's own policy, in the terms a contributor meets it in.
fn policy_lines(c: &AgentContract) -> String {
    let mut s = evidence_lines(c);
    s.push_str(&rule_lines(c));
    // Two independent keys, so two clauses rather than four sentences: the tier
    // `deny_warnings` sets, and the exception `advisory` carves out of whatever
    // that tier is. Composed, because writing the combinations out is how the
    // first version came to claim there is no advisory tier for a repository
    // that had written one (`A1-272`'s audit).
    let mut warning_tier = if c.switches.contains(&Switch::DenyWarnings) {
        "**Every warning blocks** here (`deny_warnings`): a finding is a finding".to_string()
    } else {
        "Warnings do not block here: a finding in the advisory tier is reported and the \
         gate passes anyway, in CI as much as by hand, and `deny_warnings` is what makes \
         one stop a build"
            .to_string()
    };
    if c.advisory.is_empty() {
        warning_tier.push('.');
    } else {
        // Named, whichever tier it is carved out of. Without `deny_warnings` the
        // downgrade is invisible until it is an *error* that does not block,
        // which is the case an author is least likely to guess at.
        let _ = write!(
            warning_tier,
            ". {} {} downgraded with `advisory`, so {} nothing whatever {} severity.",
            joined(
                &c.advisory
                    .iter()
                    .map(|c| format!("`{c}`"))
                    .collect::<Vec<_>>(),
                "and"
            ),
            if c.advisory.len() == 1 { "is" } else { "are" },
            if c.advisory.len() == 1 {
                "it blocks"
            } else {
                "they block"
            },
            if c.advisory.len() == 1 {
                "its"
            } else {
                "their"
            },
        );
    }
    let _ = writeln!(s, "{}", bullet(&warning_tier));
    s
}

/// What a `done` task has to cite.
fn evidence_lines(c: &AgentContract) -> String {
    let mut s = String::new();
    if c.min_strength.is_empty() {
        let _ = writeln!(
            s,
            "{}",
            bullet(
                "Evidence is not gated by strength here: any citation closes a task, including a \
                 `note`. Cite the strongest thing that is true anyway."
            )
        );
    } else {
        let kinds: Vec<String> = c.min_strength.iter().map(|k| format!("`{k}`")).collect();
        let _ = writeln!(
            s,
            "{}",
            bullet(&format!(
                "A `done` task must cite {} evidence (`evidence.require_min_strength`).",
                english_list(&kinds)
            ))
        );
        for (subset, requirement) in &c.scoped_evidence {
            let _ = writeln!(
                s,
                "{}",
                bullet(&format!(
                    "In {subset}, {requirement} (`policy`). The narrowest scope that matches a \
                     task wins, and this line is one of them."
                ))
            );
        }
        if !c.exempt_tags.is_empty() {
            let tags: Vec<String> = c.exempt_tags.iter().map(|t| format!("`{t}`")).collect();
            let _ = writeln!(
                s,
                "{}",
                bullet(&format!(
                    "Tasks tagged {} are exempt, and a task claiming the exemption should say in \
                     its body why nothing stronger exists.",
                    tags.join(" or ")
                ))
            );
        }
    }
    s
}

/// The rules this repository turned on, and the one that is always true.
fn rule_lines(c: &AgentContract) -> String {
    let mut s = String::new();
    // True of every plan, and the one thing a task author forgets: `paths` is
    // what makes a `commit` citation mean anything, and its absence degrades
    // that citation from "a commit that touched this work" to "a commit exists"
    // (`APLAN023`, and the first pilot's own hand-written contract says the same
    // in its own words).
    //
    // "Every task that is not a milestone", on each of the three bullets whose
    // rule exempts one. A milestone is a checkpoint over work rather than work,
    // carrying no `paths` and no estimate by design, and stating these as
    // universal turned two configured rules into claims stronger than the
    // validator -- so a literal agent would "repair" a valid cut container with
    // a meaningless estimate and evidence boundary (`A1-157`, from the fifth
    // pilot). Said on each bullet rather than once at the end: a blanket
    // sentence is wrong the moment a bullet is added whose rule does apply to a
    // milestone, and this defect is exactly one general statement covering rules
    // with different subjects.
    let _ = writeln!(
        s,
        "{}",
        bullet(
            "Every task that is not a milestone declares `paths`, and a `done` one without them \
             is reported (`APLAN023`). That is what makes a `commit` citation checkable: `verify` \
             refuses one whose diff touched nothing the task claims, and a task without `paths` \
             has quietly weakened its own evidence to \"a commit exists\"."
        )
    );
    if c.switches.contains(&Switch::RequireEffort) {
        let _ = writeln!(
            s,
            "{}",
            bullet(
                "Every live task that is not a milestone carries `effort_days`; one without it \
                 is reported (`APLAN009`)."
            )
        );
    }
    if c.switches.contains(&Switch::RequireCutMembership) {
        let _ = writeln!(
            s,
            "{}",
            bullet(
                "Every task that is not a milestone belongs to exactly one cut, or is tagged \
                 `cross-cutting` out loud (`APLAN022`). Put a new task in its milestone's \
                 `blocked_by` when you write it."
            )
        );
    }
    if c.switches.contains(&Switch::DetectDrift) {
        let _ = writeln!(
            s,
            "{}",
            bullet(
                "Closing a task stamps the `traces` sections it names, and a later edit to one \
                 of them reports the task (`APLAN018`). Re-read the section, then `aplan close \
                 --restamp` if it still holds."
            )
        );
    }
    if c.runner_count > 0 {
        let names: Vec<String> = c.runners.iter().map(|r| format!("`{r}`")).collect();
        let which = if names.is_empty() {
            "the configured runner".to_string()
        } else {
            format!("the configured runners ({})", names.join(", "))
        };
        let _ = writeln!(
            s,
            "{}",
            bullet(&format!(
                "A `test` citation resolves against {which}: it has to be a name the runner \
                 lists, and where several runners list it the citation is refused rather than \
                 guessed (`APLAN013`)."
            ))
        );
    }
    if c.switches.contains(&Switch::UrlCheck) {
        let _ = writeln!(
            s,
            "{}",
            bullet(
                "A cited `url` is fetched and has to answer (`APLAN016`), so a link that needs \
                 credentials CI does not have is the wrong citation."
            )
        );
    }
    s
}

#[cfg(test)]
mod agents_tests {
    use super::*;

    fn contract() -> AgentContract {
        AgentContract {
            plan_dir: Some("plans/tasks".to_string()),
            id_examples: vec!["WH2-nn".to_string()],
            min_strength: vec!["commit".into(), "test".into(), "file".into()],
            exempt_tags: vec!["untestable".into()],
            scoped_evidence: Vec::new(),
            advisory: Vec::new(),
            switches: [
                Switch::DenyWarnings,
                Switch::DetectDrift,
                Switch::RequireCutMembership,
                Switch::RequireEffort,
            ]
            .into_iter()
            .collect(),
            runners: vec!["libs".into(), "cli".into()],
            runner_count: 2,
            tools: vec![(
                "aplan_ready".into(),
                "List tasks whose blockers are done. Longer prose follows.".into(),
            )],
        }
    }

    /// The block is the repository's *policy*, in its own terms (`A1-038`).
    #[test]
    fn the_block_states_this_repositorys_policy_not_the_tools_defaults() {
        let out = render_agents_md(&contract());
        assert!(out.starts_with(AGENTS_BEGIN), "fenced: {out}");
        assert!(out.trim_end().ends_with(AGENTS_END), "fenced: {out}");
        assert!(
            out.contains("`plans/tasks/`"),
            "where the plan lives: {out}"
        );
        assert!(out.contains("`WH2-nn`"), "the plan's own convention: {out}");
        assert!(
            out.contains("`commit`, `test` or `file` evidence"),
            "the declared policy, read as English: {out}"
        );
        assert!(out.contains("`untestable`"), "the exemption: {out}");
        assert!(
            out.contains("**Every warning blocks**"),
            "the policy: {out}"
        );
        assert!(out.contains("`libs`, `cli`"), "the runners: {out}");
        // One sentence per tool: the surface, not the manual.
        assert!(out.contains("- `aplan_ready` -- List tasks whose blockers are done."));
        assert!(!out.contains("Longer prose follows"), "truncated: {out}");
    }

    /// A plan without `deny_warnings` has an advisory tier, and the block used to
    /// tell its reader to read it -- true advice to a person, and advice to
    /// nobody once CI is the only thing running the gate (`A1-272`).
    #[test]
    fn the_advisory_tier_names_what_switches_it_off() {
        let mut c = contract();
        c.switches.remove(&Switch::DenyWarnings);
        let out = render_agents_md(&c);
        // `bullet` wraps, so a phrase longer than a few words straddles a line
        // break in the rendered block. Asserted against the unwrapped text: a
        // test that only passes while the wrap falls in a particular column is
        // measuring the terminal width, not the sentence.
        let flat = out.split_whitespace().collect::<Vec<_>>().join(" ");

        assert!(
            flat.contains("Warnings do not block here"),
            "the fact first: {out}"
        );
        // What actually happens, and the key that changes it. The line must not
        // instruct a reader instead: an automated gate has none, which is the
        // whole finding, and "read them" was the sentence that went stale.
        assert!(
            flat.contains("the gate passes anyway") && flat.contains("`deny_warnings`"),
            "names neither the consequence nor the key: {out}"
        );
        assert!(
            !flat.contains("read them"),
            "still addressing a reader that may not exist: {out}"
        );
        assert!(
            !out.contains("**Every warning blocks**"),
            "that is the other branch: {out}"
        );
    }

    /// The block for a repository's warning policy is two independent keys, so
    /// all four states are asserted (`A1-272`'s audits).
    ///
    /// The first version wrote the combinations out as whole sentences and got
    /// one wrong -- *there is no advisory tier*, told to a repository that had
    /// written one, which is a supported migration posture with a test of its own
    /// one crate over
    /// (`deny_warnings_does_not_block_on_a_rule_the_repository_made_advisory`).
    /// The second version handled `advisory` only under `deny_warnings`, leaving
    /// a *downgraded error* -- the case an author is least likely to guess at --
    /// unmentioned. A table is the shape that cannot have a hole.
    #[test]
    fn all_four_warning_policies_describe_themselves() {
        let policy = |deny: bool, advisory: &[&str]| {
            let mut c = contract();
            if !deny {
                c.switches.remove(&Switch::DenyWarnings);
            }
            c.advisory = advisory.iter().map(|s| (*s).to_string()).collect();
            let out = render_agents_md(&c);
            out.split_whitespace().collect::<Vec<_>>().join(" ")
        };

        let blocks = "**Every warning blocks**";
        let advisory_tier = "Warnings do not block here";
        let downgraded = "are downgraded with `advisory`, so they block nothing";

        let neither = policy(false, &[]);
        assert!(neither.contains(advisory_tier), "{neither}");
        assert!(
            !neither.contains("`advisory`,"),
            "nothing to name: {neither}"
        );

        let deny_only = policy(true, &[]);
        assert!(deny_only.contains(blocks), "{deny_only}");
        assert!(
            !deny_only.contains("`advisory`"),
            "nothing to name: {deny_only}"
        );

        // A downgraded *error* under no `deny_warnings`: the tier sentence is
        // about warnings and says nothing about this rule, so the clause is the
        // only thing that mentions it at all.
        let advisory_only = policy(false, &["APLAN022", "APLAN030"]);
        assert!(advisory_only.contains(advisory_tier), "{advisory_only}");
        assert!(
            advisory_only.contains(&format!("`APLAN022` and `APLAN030` {downgraded}")),
            "{advisory_only}"
        );

        let both = policy(true, &["APLAN009", "APLAN023"]);
        assert!(both.contains(blocks), "{both}");
        assert!(
            both.contains(&format!("`APLAN009` and `APLAN023` {downgraded}")),
            "{both}"
        );
        assert!(
            !both.contains("there is no advisory tier"),
            "the repository wrote one: {both}"
        );
    }

    /// One downgraded rule is *is* and *it*, not *are* and *they*.
    #[test]
    fn a_single_downgraded_rule_reads_as_one() {
        let out = render_agents_md(&AgentContract {
            advisory: vec!["APLAN022".into()],
            ..contract()
        });
        let flat = out.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains(
                "`APLAN022` is downgraded with `advisory`, so it blocks nothing \
                           whatever its severity."
            ),
            "{out}"
        );
    }

    /// One runner, unnamed, is the shape an adopter starts with, and it used to
    /// produce a block that never mentioned tests (this task's audit).
    #[test]
    fn a_single_unnamed_runner_is_still_described() {
        let one = AgentContract {
            runner_count: 1,
            ..contract()
        };
        let out = render_agents_md(&AgentContract {
            runners: Vec::new(),
            ..one
        });
        assert!(
            out.contains("resolves against the configured runner:"),
            "the sentence survives having no name to print: {out}"
        );
    }

    /// A repository that declares nothing gets a block that claims nothing.
    #[test]
    fn nothing_declared_means_nothing_asserted() {
        let bare = AgentContract {
            plan_dir: None,
            switches: [Switch::RequireEffort].into_iter().collect(),
            ..AgentContract::default()
        };
        let out = render_agents_md(&bare);
        assert!(
            !out.contains("`APLAN022`"),
            "no cut rule was declared: {out}"
        );
        assert!(
            !out.contains("`APLAN018`"),
            "no drift rule was declared: {out}"
        );
        assert!(
            !out.contains("Through MCP"),
            "no tools in this build: {out}"
        );
        assert!(
            out.contains("live at the repository root,"),
            "a plan at the root is a sentence, not a path: {out}"
        );
        assert!(
            out.contains("Evidence is not gated by strength here"),
            "says what the absence means rather than staying silent: {out}"
        );
        // Two things hold for every plan, and both must survive an empty config.
        // Scoped to the subject the rule has: a milestone declares no `paths` by
        // design, and saying otherwise is a claim stronger than the validator
        // makes (`A1-157`).
        assert!(
            out.contains("Every task that is not a milestone declares `paths`"),
            "{out}"
        );
        assert!(out.contains("A status is a claim"), "{out}");
    }

    /// The generated contract must not state a rule more strongly than the rule
    /// states itself (`A1-157`, from the fifth pilot).
    ///
    /// Three bullets said "every task" about rules that exempt a milestone, so a
    /// valid cut container read as noncompliant and a literal agent would
    /// "repair" it with a meaningless estimate and evidence boundary. The gate
    /// derives from the rules rather than from a list of the three: any bullet
    /// citing a rule whose `applies_to` declares `NotMilestone` has to say so,
    /// which is a claim a future bullet cannot quietly break.
    #[test]
    fn a_bullet_scopes_itself_to_the_subject_its_rule_has() {
        let exempting: Vec<&str> = aplan_core::Rule::ALL
            .iter()
            .filter(|r| r.applies_to().contains(&Binding::NotMilestone))
            .map(|r| r.code())
            .collect();
        assert!(
            exempting.len() >= 3,
            "the sweep found {exempting:?}, so it has stopped matching and passes vacuously"
        );

        // `contract()` already turns on every switch whose bullet cites one of
        // these rules, which is what the count below insists on.
        let block = render_agents_md(&contract());
        let mut cited = 0usize;
        for code in &exempting {
            if !block.contains(code) {
                continue;
            }
            cited += 1;
            let bullet = bullet_of(&block, code);
            assert!(
                bullet.contains("not a milestone"),
                "the bullet citing {code} does not say the rule exempts a milestone:\n  {bullet}"
            );
        }
        assert!(
            cited >= 3,
            "only {cited} of the exempting rules is cited in the block, so this checks almost \
             nothing"
        );
    }

    /// The whole bullet a code appears in: bullets wrap, so a line is not one.
    fn bullet_of(block: &str, code: &str) -> String {
        let mut current = String::new();
        for line in block.lines() {
            if line.starts_with("- ") {
                if current.contains(code) {
                    return current;
                }
                current = line.to_string();
            } else {
                current.push(' ');
                current.push_str(line.trim());
            }
        }
        current
    }

    /// Every line fits, because the block is committed and diffed (`A1-038`).
    #[test]
    fn no_line_is_wider_than_the_documents_beside_it() {
        let out = render_agents_md(&contract());
        for line in out.lines() {
            // The markers are one long line each by nature, and a fenced command
            // is copied rather than read.
            if line.starts_with("<!--") {
                continue;
            }
            assert!(
                line.chars().count() <= 100,
                "a generated line re-diffs whole when a word changes: {line}"
            );
        }
    }

    #[test]
    fn a_list_reads_as_english() {
        assert_eq!(english_list(&["a".to_string()]), "a");
        assert_eq!(english_list(&["a".to_string(), "b".to_string()]), "a or b");
        assert_eq!(
            english_list(&["a".to_string(), "b".to_string(), "c".to_string()]),
            "a, b or c"
        );
    }
}

/// One live task that has not been touched for a while (`A1-020`).
#[derive(Debug, Clone, PartialEq)]
pub struct StaleTask {
    pub id: String,
    pub title: String,
    pub status: String,
    /// Days since the task's file last changed, as of `HEAD`.
    pub idle_days: i64,
    /// Whether every blocker is `done`, so nothing but a decision is stopping it.
    ///
    /// The least-proxy case FIELD-RESEARCH names and this report leads with: a
    /// task that *can* start and has not is drift a tracker cannot see, because
    /// the tracker never knew when the last blocker cleared.
    pub ready: bool,
}

/// One parked task whose revisit instant has arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueTask {
    /// The task.
    pub id: String,
    /// Its title.
    pub title: String,
    /// The instant it named, as written.
    pub revisit_at: String,
    /// How long ago that was, from the clock the report used.
    pub overdue_seconds: i64,
}

/// What `aplan due` found (`A1-264`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueReport {
    /// Parked tasks whose revisit has arrived, longest overdue first.
    pub tasks: Vec<DueTask>,
    /// Parked tasks with a revisit still in the future, so a reader knows the
    /// denominator and an empty answer is not mistaken for "nothing is parked".
    pub waiting: usize,
    /// The instant the report answered as of, when the caller named one.
    /// `None` means the system clock, which is the answer that cannot be
    /// reproduced and is therefore the one worth saying out loud.
    pub as_of: Option<String>,
}

/// Render `aplan due`.
///
/// # Errors
///
/// When the output cannot be written.
pub fn render_due(report: &DueReport, format: Format) -> std::io::Result<()> {
    if format != Format::Human {
        let doc = serde_json::json!({
            "schema": "aplan/due/v1",
            "as_of": report.as_of,
            "waiting": report.waiting,
            "tasks": report.tasks.iter().map(|t| serde_json::json!({
                "id": t.id,
                "title": t.title,
                "revisit_at": t.revisit_at,
                "overdue_seconds": t.overdue_seconds,
            })).collect::<Vec<_>>(),
        });
        let mut out = anstream::stdout().lock();
        return writeln!(out, "{}", serde_json::to_string_pretty(&doc)?);
    }
    let mut out = anstream::stdout().lock();
    write!(out, "{}", due_human(report))
}

/// The human report as text, pure so a test reads what a reader sees.
#[must_use]
pub fn due_human(report: &DueReport) -> String {
    let mut s = String::new();
    if report.tasks.is_empty() {
        let _ = writeln!(
            s,
            "no parked task is due for a look; {} carry a revisit still in the future",
            report.waiting
        );
        push_due_clock(&mut s, report);
        return s;
    }
    let id_w = column(report.tasks.iter().map(|t| t.id.as_str()), 10);
    let bold = crate::style::HEADING;
    let _ = writeln!(
        s,
        "{bold}{:<id_w$}  {:<22}  title{bold:#}",
        "task", "revisit due"
    );
    for t in &report.tasks {
        let _ = writeln!(
            s,
            "{}",
            fold::row(
                &format!(
                    "{id}{:<id_w$}{id:#}  {dim}{:<22}{dim:#}  ",
                    t.id,
                    t.revisit_at,
                    id = crate::style::ID,
                    dim = crate::style::DIM
                ),
                &one_line(&t.title),
            )
        );
    }
    let _ = writeln!(s);
    // The sentence the whole feature turns on. A scheduler that read this as
    // "the wait is over" would unpark work on a clock, which is the one thing
    // this field must never be able to say (`A1-264`).
    let _ = writeln!(
        s,
        "{}",
        fold::note(
            "These are due for a look, not unblocked: reaching the instant says nothing about \
             the condition each is parked on, which is in its body. Nothing here changes a \
             status."
        )
    );
    if report.waiting > 0 {
        let _ = writeln!(
            s,
            "{} more parked task(s) carry a revisit still in the future.",
            report.waiting
        );
    }
    push_due_clock(&mut s, report);
    s
}

/// Which clock, always: a reader told a task is due has to know due by when.
fn push_due_clock(s: &mut String, report: &DueReport) {
    let _ = match &report.as_of {
        Some(at) => writeln!(s, "Answered as of {at}, as asked (`--as-of`)."),
        None => writeln!(
            s,
            "{}",
            fold::note(
                "Answered against this machine's clock. Pass `--as-of <instant>` in RFC 3339 -- \
                 when a scheduler woke -- so two runs of one plan agree."
            )
        ),
    };
}

/// What `aplan stale` found.
#[derive(Debug, Clone, PartialEq)]
pub struct StaleReport {
    /// The window asked for, in days.
    pub window_days: i64,
    /// Live tasks over the window, readiest and idlest first.
    pub tasks: Vec<StaleTask>,
    /// Live tasks that are inside the window, so a reader knows the denominator.
    pub fresh: usize,
    /// Live tasks git has never seen, which have no history to have sat idle in.
    pub untracked: usize,
    /// The date the report measured *from*, when the caller named one
    /// (`--as-of`). `None` means `HEAD`'s own commit time, which is the default
    /// and the reproducible one.
    ///
    /// Carried so the report can say which clock it used: the same window
    /// against two different "now"s is two different questions, and a reader who
    /// cannot tell them apart cannot act on either (`A1-162`).
    pub as_of: Option<String>,
}

/// Render the staleness report.
///
/// **Advice, and it says so.** Every other report in this tool leads to a
/// verdict; this one cannot, because a task file's date is a *proxy* for the
/// work's state -- a file can sit while the work moves, or move while the work
/// sits. What it is good for is a question to ask at a standup, and the wording
/// stays inside that (`A1-020`, FIELD-RESEARCH 7.3).
///
/// # Errors
/// Propagates any write failure on stdout.
pub fn render_stale(report: &StaleReport, format: Format) -> std::io::Result<()> {
    if format != Format::Human {
        return render_stale_json(report);
    }
    let mut out = anstream::stdout().lock();
    write!(out, "{}", stale_human(report))
}

/// The human report as text.
///
/// Pure, like `roadmap_human`, so a test asserts on what a reader sees rather
/// than on a copy of it: the first version of this task's audit tested a
/// reimplementation of these sentences, which proves only that the test agrees
/// with itself.
#[must_use]
pub fn stale_human(report: &StaleReport) -> String {
    let mut s = String::new();
    // A report that can see nothing must not read as a clean bill of health.
    // Pointed at the wrong repository -- `--repo` elsewhere, or a plan checked
    // out beside one rather than inside it -- every live task has no history
    // here, and the first version answered "nothing has sat", which is the false
    // green this project exists to refuse (`A1-020`, found by its audit).
    if report.fresh == 0 && report.untracked > 0 && report.tasks.is_empty() {
        let _ = writeln!(
            s,
            "no live task has any history in this repository ({} of them), so this says nothing \
             about staleness. Either the plan is new, or `--repo` is not the repository that \
             holds it.",
            report.untracked
        );
        return s;
    }
    if report.tasks.is_empty() {
        let _ = writeln!(
            s,
            "nothing live has sat longer than {} day(s); {} live task(s) are inside the window",
            report.window_days, report.fresh
        );
        push_untracked(&mut s, report);
        return s;
    }
    let id_w = column(report.tasks.iter().map(|t| t.id.as_str()), 10);
    let bold = crate::style::HEADING;
    let _ = writeln!(
        s,
        "{bold}{:<id_w$}  {:>4}  {:<12}  title{bold:#}",
        "task", "idle", "status"
    );
    for t in &report.tasks {
        // The marker rather than a column: a reader scanning for what to pick up
        // wants the ready ones to stand out, and a second column of mostly
        // blanks does not do that.
        let ready = if t.ready { " (ready)" } else { "" };
        let _ = writeln!(
            s,
            "{}",
            fold::row(
                &format!(
                    "{id}{:<id_w$}{id:#}  {dim}{:>3}d{dim:#}  {st}{:<12}{st:#}  ",
                    t.id,
                    t.idle_days,
                    t.status,
                    id = crate::style::ID,
                    dim = crate::style::DIM,
                    st = crate::style::status(&t.status)
                ),
                &format!("{}{ready}", one_line(&t.title)),
            )
        );
    }
    let _ = writeln!(s);
    let ready = report.tasks.iter().filter(|t| t.ready).count();
    if ready > 0 {
        let _ = writeln!(
            s,
            "{}",
            fold::note(&format!(
                "{ready} of these can be started now: every blocker is done, and they \
                 have sat anyway."
            ))
        );
    }
    // What a `blocked` row means, said only when there is one, and said in a
    // way that is true of every such row (`A1-275`, audit).
    //
    // *Not waiting on the plan* was the first wording and it is false for a task
    // that is parked **and** lists unfinished blockers -- a shape nothing
    // refuses. What holds either way is that the status, not the graph, is what
    // keeps it out of `ready`: clearing every blocker would not offer it. The
    // count is phrased without a verb that has to agree, because one parked task
    // is the common case and "1 of these are parked" is the sort of thing a
    // reader notices instead of the report.
    let parked = report
        .tasks
        .iter()
        .filter(|t| t.status == "blocked")
        .count();
    if parked > 0 {
        let _ = writeln!(
            s,
            "{}",
            fold::fold(
                &format!(
                    "{parked} parked (`blocked`): set aside by a decision rather than by the \
                     graph, so clearing blockers does not make one ready. The age is how long \
                     the park has held, and the task body says what it waits on."
                ),
                fold::out_width(),
                0
            )
        );
    }
    let _ = writeln!(
        s,
        "{}",
        fold::note(&format!(
            "advice, not a verdict: a task file's date is a proxy for the work's state. \
             {} live task(s) are inside the window.",
            report.fresh
        ))
    );
    push_untracked(&mut s, report);
    s
}

/// What could not be dated, said wherever the report ends.
fn push_untracked(s: &mut String, report: &StaleReport) {
    // Which clock, always, and not only when it is the unusual one. A reader
    // told "idle 42 days" has to know what that is 42 days *before*, and the
    // default -- `HEAD`'s own commit time -- is the surprising answer for a
    // repository that has stopped moving: every age freezes with it.
    let _ = match &report.as_of {
        Some(date) => writeln!(s, "Ages measured from {date}, as asked (`--as-of`)."),
        None => writeln!(
            s,
            "{}",
            fold::note(
                "Ages measured from `HEAD`'s commit time, so two runs at one commit agree. \
                 Pass `--as-of <YYYY-MM-DD>` to measure from a date instead."
            )
        ),
    };
    if report.untracked > 0 {
        let _ = writeln!(
            s,
            "{} live task(s) have no commit in this history, so they have no age to report.",
            report.untracked
        );
    }
}

fn render_stale_json(report: &StaleReport) -> std::io::Result<()> {
    let doc = serde_json::json!({
        "schema": "aplan/stale/v1",
        "window_days": report.window_days,
        "fresh": report.fresh,
        "untracked": report.untracked,
        // `null` means `HEAD`'s commit time, which is the default and the
        // reproducible one.
        "as_of": report.as_of,
        "tasks": report.tasks.iter().map(|t| serde_json::json!({
            "id": t.id,
            "title": t.title,
            "status": t.status,
            "idle_days": t.idle_days,
            "ready": t.ready,
        })).collect::<Vec<_>>(),
    });
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &doc)?;
    writeln!(out)
}

#[cfg(test)]
mod stale_tests {
    use super::*;

    fn task(id: &str, idle_days: i64, ready: bool) -> StaleTask {
        StaleTask {
            id: id.to_string(),
            title: format!("{id} title"),
            status: "todo".to_string(),
            idle_days,
            ready,
        }
    }

    /// The three branches a reader can meet, rendered where coverage can see
    /// them: the corpus runs the binary in another process (`A1-074`'s reason).
    #[test]
    fn every_branch_of_the_report_renders() {
        for report in [
            // Something to report, one of it ready.
            StaleReport {
                as_of: None,
                window_days: 30,
                tasks: vec![task("A-01", 90, true), task("A-02", 40, false)],
                fresh: 3,
                untracked: 1,
            },
            // Nothing over the window, and nothing hidden either.
            StaleReport {
                as_of: None,
                window_days: 30,
                tasks: Vec::new(),
                fresh: 4,
                untracked: 0,
            },
            // The blind case: no history at all for anything live.
            StaleReport {
                as_of: None,
                window_days: 30,
                tasks: Vec::new(),
                fresh: 0,
                untracked: 5,
            },
        ] {
            render_stale(&report, Format::Human).expect("human");
            render_stale(&report, Format::Json).expect("json");
        }
    }

    /// A report that can see nothing must not read as a clean bill of health
    /// (`A1-020`, found by its audit). Asserted on the string rather than by
    /// eye, because this is the sentence the defect lived in.
    #[test]
    fn a_blind_report_does_not_say_the_reassuring_thing() {
        let blind = StaleReport {
            as_of: None,
            window_days: 30,
            tasks: Vec::new(),
            fresh: 0,
            untracked: 5,
        };
        let text = capture(&blind);
        assert!(text.contains("no live task has any history"), "{text}");
        assert!(!text.contains("nothing live has sat"), "{text}");

        // And the ordinary empty case still answers, while naming what it could
        // not date rather than dropping it.
        let quiet = StaleReport {
            as_of: None,
            window_days: 30,
            tasks: Vec::new(),
            fresh: 4,
            untracked: 2,
        };
        let text = capture(&quiet);
        assert!(
            text.contains("nothing live has sat longer than 30"),
            "{text}"
        );
        assert!(text.contains("2 live task(s) have no commit"), "{text}");
    }

    /// The ready marker is what a reader scans for, so it is on the row.
    #[test]
    fn a_task_that_could_start_is_marked_on_its_row() {
        let report = StaleReport {
            as_of: None,
            window_days: 30,
            tasks: vec![task("A-01", 90, true), task("A-02", 40, false)],
            fresh: 0,
            untracked: 0,
        };
        let text = capture(&report);
        assert!(text.contains("A-01"), "{text}");
        assert!(text.contains("(ready)"), "{text}");
        assert!(
            text.contains("1 of these can be started now"),
            "and counted in the summary line: {text}"
        );
        // The row that is not ready carries no marker.
        let row = text
            .lines()
            .find(|l| l.contains("A-02"))
            .expect("the second row");
        assert!(!row.contains("(ready)"), "{row}");
    }

    fn capture(report: &StaleReport) -> String {
        stale_human(report)
    }
}

/// One task's `paths`, measured against recent history (`A1-151`).
#[derive(Debug, Clone, PartialEq)]
pub struct PathBreadth {
    pub id: String,
    pub title: String,
    /// Sampled commits that touch these paths. The count rather than the
    /// percentage is what orders the report and what the JSON carries, because
    /// one commit in two hundred rounds to the same `0%` as none at all, and
    /// those are opposite findings (`A1-151`'s audit).
    pub hits: usize,
    /// Percentage of sampled commits that touch these paths, rounded down.
    pub percent: u32,
    /// The first declared entry, which is usually the one doing the work.
    pub first_path: String,
}

/// What `aplan paths` found.
#[derive(Debug, Clone, PartialEq)]
pub struct PathsReport {
    /// How many commits were actually sampled, which is not always what was
    /// asked for: a repository shorter than the sample is common.
    pub sampled: usize,
    pub asked_for: usize,
    /// Every task carrying `paths`, broadest first.
    pub tasks: Vec<PathBreadth>,
    /// Tasks with no `paths` at all, whose `commit` evidence is weaker still.
    pub without_paths: usize,
    /// Tasks whose `paths` do not compile to globs, so nothing was measured for
    /// them. `APLAN032` is the place that reports the broken glob; a `0%` row
    /// here would be this report claiming a measurement it never made.
    pub unmeasurable: usize,
}

impl PathsReport {
    /// The plan's own middle, which is what any single number has to be read
    /// against. `None` when there is nothing to take a middle of: no task
    /// carries `paths`, or no commit was sampled.
    ///
    /// The mean of the two middle rows on an even count, rather than the upper
    /// one. A published field called `median_percent` has to be the median.
    #[must_use]
    pub fn median_percent(&self) -> Option<u32> {
        if self.sampled == 0 {
            return None;
        }
        median_of(&self.tasks)
    }
}

/// The median share, or `None` when there are no rows to take one of.
fn median_of(tasks: &[PathBreadth]) -> Option<u32> {
    if tasks.is_empty() {
        return None;
    }
    let mut all: Vec<u32> = tasks.iter().map(|t| t.percent).collect();
    all.sort_unstable();
    let mid = all.len() / 2;
    Some(if all.len() % 2 == 1 {
        all[mid]
    } else {
        (all[mid - 1] + all[mid]).div_ceil(2)
    })
}

/// A share small enough to round to nothing, said as that.
///
/// `weeks_cell`'s `<0.1` for the same reason: a cell that reads `0%` when the
/// answer is "one commit in two hundred" hides the narrowest paths in the plan
/// among the ones no commit has ever touched.
fn share_cell(hits: usize, percent: u32) -> String {
    if hits > 0 && percent == 0 {
        "<1%".to_string()
    } else {
        format!("{percent}%")
    }
}

/// Render what each rule could actually have failed on (`A1-114`).
///
/// **A measurement, and never a verdict.** A repository mid-adoption is vacuous
/// nearly everywhere and must not be blocked for it; what an adopter needs is to
/// watch the number come down. Same shape as `paths` and `stale`: it exits `0`
/// whatever it finds.
///
/// # Errors
/// Propagates any write failure on stdout.
pub fn render_vacuity(report: &[RuleReach], format: Format) -> std::io::Result<()> {
    if format != Format::Human {
        return render_vacuity_json(report);
    }
    let mut out = anstream::stdout().lock();
    write!(out, "{}", vacuity_human(report))
}

/// The human report as text, pure so a test reads what a person reads.
#[must_use]
pub fn vacuity_human(report: &[RuleReach]) -> String {
    let mut s = String::new();
    let with_subjects: Vec<&RuleReach> = report.iter().filter(|r| r.subjects > 0).collect();
    if with_subjects.is_empty() {
        let _ = writeln!(
            s,
            "no rule that binds evidence to a task has a subject in this plan, so there is \
             nothing here to be blind about yet."
        );
        return s;
    }

    let blind: usize = with_subjects.iter().map(|r| r.unbound).sum();
    let subjects: usize = with_subjects.iter().map(|r| r.subjects).sum();
    let _ = writeln!(
        s,
        "{}",
        fold::fold(
            &format!(
                "{blind} of {subjects} rule/task pair(s) had nothing to check against: the rule \
                 ran and could not have failed."
            ),
            fold::out_width(),
            0
        )
    );
    let _ = writeln!(s);

    let bold = crate::style::HEADING;
    let _ = writeln!(
        s,
        "{bold}{:<10}  {:>8}  {:>7}  {:>7}  needs{bold:#}",
        "rule", "subjects", "checked", "blind"
    );
    for r in &with_subjects {
        // A rule the configuration switched off says so in the column that
        // would otherwise say what it needed from the task: what it needed is
        // not the reason it said nothing (`A1-269`).
        let needed = match r.off {
            // Short enough for the column: the footnote below says where these
            // keys live, and repeating `.aplan.yml` on every row pushed the
            // longest one past a 95-column terminal (`A1-269`'s audit).
            Some(key) => format!("not run: `{key}` is off"),
            None => needs(r.binds),
        };
        let _ = writeln!(
            s,
            "{}",
            fold::row(
                &format!(
                    "{id}{:<10}{id:#}  {:>8}  {:>7}  {:>7}  ",
                    r.rule.code(),
                    r.subjects,
                    r.checked(),
                    r.unbound,
                    id = crate::style::ID,
                ),
                &needed,
            )
        );
    }
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "{}",
        fold::note(
            "A blind subject is one the rule ran on and could not have failed, however wrong the \
             task was. It is not a finding: it is the reason a finding could not exist. A rule \
             marked *not run* checked nothing at all, whatever its subjects carried."
        )
    );
    let _ = writeln!(
        s,
        "{}",
        fold::note(
            "This counts what the *task* carried, and what `.aplan.yml` switched off: a rule \
             stopped by a config key is marked *not run* and checked nothing. What is \
             still not measured is a rule a *runner's* configuration leaves nothing for \
             -- no runner declaring `covers`, say -- which needs the repository open and \
             this report is hermetic."
        )
    );
    s
}

/// What a rule's predicate compares against, in a reader's words.
fn needs(binds: &[Binding]) -> String {
    binds
        .iter()
        .map(|b| match b {
            Binding::Done => "`done`",
            Binding::NotMilestone => "not a milestone",
            Binding::Paths => "`paths`",
            Binding::Traces => "`traces`",
            Binding::TraceFingerprints => "`trace_fingerprints`",
            Binding::Evidence(_) => "evidence",
        })
        .collect::<Vec<_>>()
        .join(" and ")
}

fn render_vacuity_json(report: &[RuleReach]) -> std::io::Result<()> {
    let doc = serde_json::json!({
        "schema": "aplan/vacuity/v1",
        "rules": report.iter().map(|r| serde_json::json!({
            "rule": r.rule.code(),
            "title": r.rule.title(),
            "subjects": r.subjects,
            "checked": r.checked(),
            "blind": r.unbound,
            // Absent where the rule ran, rather than present-and-null: a
            // consumer reading this document should not have to tell "did not
            // run" from "ran" by inspecting a null (`A1-253`'s shape,
            // `A1-269`).
            "not_run": r.off,
            "needs": r.binds.iter().map(|b| match b {
                Binding::Done => "done",
                Binding::NotMilestone => "not_milestone",
                Binding::Paths => "paths",
                Binding::Traces => "traces",
                Binding::TraceFingerprints => "trace_fingerprints",
                Binding::Evidence(_) => "evidence",
            }).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &doc)?;
    writeln!(out)
}

/// Render the `paths` breadth report.
///
/// **A measurement, not a verdict, and deliberately not a rule** (`A1-151`).
/// Measured across five real repositories first: the median task's paths are
/// matched by 8% of recent commits in one plan and 55% in another, so no
/// threshold means the same thing in both, and at any threshold that catches the
/// pilot's case two thirds of that pilot's tasks are flagged -- correctly, and
/// unactionably, because in a single-crate project one crate really is the work.
/// What travels between repositories is the *ranking* and the distance from the
/// plan's own median, so that is what this prints.
///
/// # Errors
/// Propagates any write failure on stdout.
pub fn render_paths(report: &PathsReport, format: Format) -> std::io::Result<()> {
    if format != Format::Human {
        return render_paths_json(report);
    }
    let mut out = anstream::stdout().lock();
    write!(out, "{}", paths_human(report))
}

/// The human report as text, pure so a test reads what a person reads.
#[must_use]
pub fn paths_human(report: &PathsReport) -> String {
    let mut s = String::new();
    // `sampled` first, because it decides whether anything below is a
    // measurement at all. A plan full of `paths` in a repository with no history
    // must not be told that no task declares any.
    if report.sampled == 0 {
        // A table of `0%` rows here would be this command saying "no recent
        // commit touches these paths" about a history it never read
        // (`A1-151`'s audit).
        let _ = writeln!(
            s,
            "no commit was sampled, so there is nothing to measure `paths` against: this \
             repository has no history yet."
        );
        return s;
    }
    if report.tasks.is_empty() {
        // "no task declares `paths`" would be a lie about a plan whose every
        // `paths` entry is a broken glob, which is a different problem with a
        // different fix.
        if report.unmeasurable > 0 {
            let _ = writeln!(
                s,
                "no task has `paths` this tool can compile, so nothing here is measurable: \
                 `aplan check` reports each entry that will not compile (APLAN032)."
            );
        } else {
            let _ = writeln!(
                s,
                "no task declares `paths`, so nothing here is measurable: a `commit` citation \
                 on such a task proves only that the commit exists."
            );
        }
        return s;
    }
    // Non-empty by the guard above, so there is a middle.
    let median = median_of(&report.tasks).unwrap_or_default();
    let _ = writeln!(
        s,
        "{}",
        fold::fold(
            &format!(
                "{} commit(s) sampled of the {} asked for; the median task's `paths` are touched \
                 by {}% of them.",
                report.sampled, report.asked_for, median
            ),
            fold::out_width(),
            0
        )
    );
    let _ = writeln!(s);

    let id_w = column(report.tasks.iter().map(|t| t.id.as_str()), 10);
    let bold = crate::style::HEADING;
    let _ = writeln!(
        s,
        "{bold}{:<id_w$}  {:>5}  first declared path{bold:#}",
        "task", "share"
    );
    for t in &report.tasks {
        let _ = writeln!(
            s,
            "{id}{:<id_w$}{id:#}  {:>5}  {}",
            t.id,
            share_cell(t.hits, t.percent),
            one_line(&t.first_path),
            id = crate::style::ID
        );
    }
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "{}",
        fold::note(
            "Read a row against that median, not against a fixed number: the same share means \
             \"almost any commit satisfies this\" in a plan whose median is low, and nothing at \
             all in a plan where one component is the work."
        )
    );
    if report.without_paths > 0 {
        let _ = writeln!(
            s,
            "{}",
            fold::note(&format!(
                "{} task(s) declare no `paths`; a `commit` citation there proves the \
                 commit exists and no more.",
                report.without_paths
            ))
        );
    }
    if report.unmeasurable > 0 {
        let _ = writeln!(
            s,
            "{} task(s) declare `paths` that do not compile to a glob, so they are not measured \
             here at all; `aplan check` reports each one (APLAN032).",
            report.unmeasurable
        );
    }
    s
}

/// Render the deprecation-runway report.
///
/// **A report, not a rule** (`A1-021`). Every runway shape it names is decidable
/// from the plan alone, but a plan is not wrong for having none, and across the
/// six repositories this tool has been pointed at exactly zero declare one: the
/// artifact the research found in GitLab, systemd, Kubernetes and OpenSSL is a
/// property of projects with public API promises, not of every project. So it
/// exits `0` whatever it finds, and lives where somebody who has a runway goes
/// looking, rather than in everybody else's `check` output.
///
/// # Errors
/// Propagates any write failure on stdout.
pub fn render_deprecations(report: &Deprecations, format: Format) -> std::io::Result<()> {
    if format != Format::Human {
        return render_deprecations_json(report);
    }
    let mut out = anstream::stdout().lock();
    write!(out, "{}", deprecations_human(report))
}

/// The human report as text, pure so a test reads what a person reads.
#[must_use]
pub fn deprecations_human(report: &Deprecations) -> String {
    let mut s = String::new();
    if report.runways.is_empty() && report.unannounced.is_empty() {
        let _ = writeln!(
            s,
            "{}",
            fold::note(&format!(
                "no task is tagged `{DEPRECATION_TAG}` or `{REMOVAL_TAG}`, so this plan \
                     declares no deprecation runway. A runway is two ordinary tasks: one \
                     announcing the removal, one doing it in a later cut and listing the first \
                     in its `blocked_by`."
            ))
        );
        return s;
    }

    for r in &report.runways {
        let _ = writeln!(
            s,
            "{}",
            fold::row(
                &format!("{id}{}{id:#}  ", r.announced.id, id = crate::style::ID),
                &one_line(&r.announced.title),
            )
        );
        let _ = writeln!(
            s,
            "  announced in {} ({st}{}{st:#})",
            cut_or_none(r.announced.cut.as_deref()),
            r.announced.status,
            st = crate::style::status(&r.announced.status.to_string())
        );
        for removal in &r.removals {
            let _ = writeln!(
                s,
                "  removed in {} by {} ({}{})",
                cut_or_none(removal.cut.as_deref()),
                removal.id,
                removal.status,
                if removal.status == Status::Done && !removal.cited {
                    ", no evidence cited"
                } else {
                    ""
                }
            );
        }
        for fault in &r.faults {
            let _ = writeln!(s, "  ! {}", fault.what());
        }
        let _ = writeln!(s);
    }

    for r in &report.unannounced {
        let _ = writeln!(
            s,
            "{}  {}\n  ! tagged `{REMOVAL_TAG}` and blocked by no `{DEPRECATION_TAG}` task: a \
             removal nobody was told about",
            r.id,
            one_line(&r.title)
        );
        let _ = writeln!(s);
    }

    let _ = writeln!(
        s,
        "{}",
        fold::fold(
            &format!(
                "{} runway(s), {} removal(s) with nothing announcing them. A runway is the gap \
                 between the two cuts; this reports its shape and never fails a build.",
                report.runways.len(),
                report.unannounced.len()
            ),
            fold::out_width(),
            0
        )
    );
    s
}

/// The same report as a markdown table, which is the shape a project publishes.
///
/// `markdown` is not a [`Format`] variant, for the reason `roadmap` gives: it
/// would make `check --format markdown` a thing. The command parses the word and
/// calls this.
#[must_use]
pub fn deprecations_markdown(report: &Deprecations) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "<!-- generated by `aplan deprecations --format markdown`; do not edit. -->"
    );
    let _ = writeln!(s, "\n# Deprecations\n");
    if report.runways.is_empty() {
        let _ = writeln!(s, "None announced.");
        return s;
    }
    // "Announcement", not "Feature": what this column holds is the announcing
    // task's own title, and calling it the feature would imply the tool knows
    // the feature's name. Write that title as the notice you want published.
    let _ = writeln!(s, "| Announcement | Announced in | Removed in | Status |");
    let _ = writeln!(s, "|---|---|---|---|");
    for r in &report.runways {
        let removed = if r.removals.is_empty() {
            "not scheduled".to_string()
        } else {
            r.removals
                .iter()
                .map(|x| cut_or_none(x.cut.as_deref()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let status =
            if r.removals.iter().all(|x| x.status == Status::Done) && !r.removals.is_empty() {
                "removed"
            } else if r.announced.status == Status::Done {
                "announced"
            } else {
                "planned"
            };
        let _ = writeln!(
            s,
            "| {} | {} | {} | {status} |",
            md_cell(&r.announced.title),
            md_cell(&cut_or_none(r.announced.cut.as_deref())),
            md_cell(&removed)
        );
    }
    s
}

fn cut_or_none(cut: Option<&str>) -> String {
    match cut {
        Some(c) if !c.is_empty() => c.to_string(),
        // A cut with no version label, or no cut at all. Both mean the same
        // thing to a reader asking "which release": nothing here dates it.
        _ => "no cut".to_string(),
    }
}

fn render_deprecations_json(report: &Deprecations) -> std::io::Result<()> {
    let task = |t: &aplan_core::deprecation::RunwayTask| {
        serde_json::json!({
            "id": t.id,
            "title": t.title,
            "status": t.status.to_string(),
            "cut": t.cut,
            "cited": t.cited,
        })
    };
    let doc = serde_json::json!({
        "schema": "aplan/deprecations/v1",
        "runways": report.runways.iter().map(|r| serde_json::json!({
            "announced": task(&r.announced),
            "removals": r.removals.iter().map(&task).collect::<Vec<_>>(),
            "faults": r.faults.iter().map(|f| serde_json::json!({
                "kind": f.kind(),
                "what": f.what(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "unannounced_removals": report.unannounced.iter().map(&task).collect::<Vec<_>>(),
    });
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &doc)?;
    writeln!(out)
}

fn render_paths_json(report: &PathsReport) -> std::io::Result<()> {
    let doc = serde_json::json!({
        "schema": "aplan/paths/v1",
        "sampled": report.sampled,
        "asked_for": report.asked_for,
        "median_percent": report.median_percent(),
        "without_paths": report.without_paths,
        "unmeasurable": report.unmeasurable,
        "tasks": report.tasks.iter().map(|t| serde_json::json!({
            "id": t.id,
            "title": t.title,
            // Both: `percent` is what a person reads, `hits` is what a consumer
            // can divide by `sampled` without losing the rows that round to
            // nothing.
            "hits": t.hits,
            "percent": t.percent,
            "first_path": t.first_path,
        })).collect::<Vec<_>>(),
    });
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &doc)?;
    writeln!(out)
}

#[cfg(test)]
mod format_table_tests {
    use super::*;

    /// The module doc above is a list of formats, and a list of formats beside
    /// the table is exactly the thing that drifts: it still named five after
    /// `markdown` joined the vocabulary. Holding it to `Format::ALL` costs one
    /// test and removes the second source.
    #[test]
    fn the_module_doc_lists_every_format() {
        let source = include_str!("lib.rs");
        let doc: String = source
            .lines()
            .take_while(|l| l.starts_with("//!") || l.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        let missing: Vec<&str> = Format::ALL
            .iter()
            .map(|f| f.as_str())
            .filter(|name| !doc.contains(&format!("**{name}**")))
            .collect();
        assert!(
            missing.is_empty(),
            "the module doc does not mention {missing:?}, so it is a second and wrong \
             answer about what this tool can render"
        );
    }

    /// A format renders at least one surface, or nothing can ask for it and it
    /// is a name the parser accepts and no command takes.
    #[test]
    fn every_format_renders_something() {
        for format in Format::ALL {
            assert!(
                !format.surfaces().is_empty(),
                "`{}` renders no surface, so it can be parsed and never used",
                format.as_str()
            );
        }
    }

    /// Every surface has a format, or a command declaring it accepts nothing.
    #[test]
    fn every_surface_has_a_format() {
        for surface in Surface::ALL {
            assert!(
                !surface.formats().is_empty(),
                "{surface:?} has no format, so a command declaring it takes no --format value"
            );
        }
    }

    /// A surface must render the value its commands default to, or they refuse
    /// the flag they were given by not passing one.
    ///
    /// This asked about `human` until `graph` arrived, because `human` was every
    /// command's default. A graph has no human rendering, which is what moved
    /// the default onto the surface (`A1-193`).
    #[test]
    fn every_surface_renders_its_own_default() {
        for surface in Surface::ALL {
            assert!(
                surface.default_format().renders(*surface),
                "{surface:?} cannot render {:?}, which its commands default to",
                surface.default_format()
            );
        }
    }
}
