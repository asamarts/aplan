//! Output formats.
//!
//! Three consumers, three shapes, one report:
//!
//! - **human**, coloured, terminal-first, for a person reading a failure;
//! - **json**, stable machine shape, for deterministic automation;
//! - **agent**, every diagnostic carries an `agent_instruction` a coding agent
//!   can act on without reading the source;
//! - **github**, workflow-command annotations, so a PR shows the failure inline;
//! - **mermaid**, a *generated* dependency graph. Never hand-edit a view.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::io::Write as _;

use aplan_core::{
    CriticalPath, Cut, Diagnostic, Graph, Report, Roadmap, RoadmapTask, Severity, Summary,
};

aplan_core::str_enum! {
    /// Selects the rendering. Generated via [`str_enum!`](aplan_core::str_enum) so
    /// `Format::ALL` (which `facts.json` and the coverage audit read) can never
    /// omit a variant.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Format {
        Human = "human",
        Json = "json",
        Agent = "agent",
        Github = "github",
    }
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
enum Kind {
    Check,
    Verify,
}

impl Kind {
    /// The trailing clause of the human "ok:" line.
    fn clean_note(self) -> &'static str {
        match self {
            Kind::Check => "DAG acyclic",
            Kind::Verify => "evidence resolved",
        }
    }

    /// The `schema` field of the non-agent JSON document.
    fn json_schema(self) -> &'static str {
        match self {
            Kind::Check => "aplan/check-report/v1",
            Kind::Verify => "aplan/verify-report/v1",
        }
    }
}

/// Render a `check` report to stdout.
pub fn render(report: &Report, format: Format) -> std::io::Result<()> {
    render_kind(report, format, Kind::Check)
}

/// Render a `verify` report to stdout. Same diagnostics, a truthful summary.
pub fn render_verified(report: &Report, format: Format) -> std::io::Result<()> {
    render_kind(report, format, Kind::Verify)
}

fn render_kind(report: &Report, format: Format, kind: Kind) -> std::io::Result<()> {
    match format {
        Format::Human => render_human(report, kind.clean_note()),
        Format::Json => render_json(report, false, kind.json_schema()),
        Format::Agent => render_json(report, true, "aplan/agent-report/v1"),
        Format::Github => render_github(report),
    }
}

fn render_human(report: &Report, clean_note: &str) -> std::io::Result<()> {
    let mut out = anstream::stdout().lock();
    let red = anstyle::AnsiColor::Red.on_default().bold();
    let yellow = anstyle::AnsiColor::Yellow.on_default().bold();
    let dim = anstyle::Style::new().dimmed();
    let green = anstyle::AnsiColor::Green.on_default().bold();

    for d in &report.diagnostics {
        let (style, label) = match d.severity {
            Severity::Error => (red, "error"),
            Severity::Warning => (yellow, "warning"),
        };
        write!(out, "{style}{label}{style:#}[{}]", d.code)?;
        if let Some(t) = &d.task {
            write!(out, " {t}")?;
        }
        writeln!(out, ": {}", d.message)?;
        if let Some(f) = &d.file {
            writeln!(out, "  {dim}--> {}{dim:#}", f.display())?;
        }
    }

    let (e, w) = (report.errors(), report.warnings());
    if e == 0 {
        writeln!(
            out,
            "{green}ok{green:#}: {} tasks, {w} warning(s), {clean_note}",
            report.tasks_checked
        )?;
    } else {
        writeln!(
            out,
            "{red}failed{red:#}: {e} error(s), {w} warning(s) across {} tasks",
            report.tasks_checked
        )?;
    }
    Ok(())
}

fn render_json(report: &Report, agent: bool, schema: &str) -> std::io::Result<()> {
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
            });
            if agent && let Some(ai) = &d.agent_instruction {
                o["agent_instruction"] = serde_json::Value::String(ai.clone());
            }
            o
        })
        .collect();

    let doc = serde_json::json!({
        "schema": schema,
        "tasks_checked": report.tasks_checked,
        "errors": report.errors(),
        "warnings": report.warnings(),
        "diagnostics": diags,
    });
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &doc)?;
    writeln!(out)
}

fn render_github(report: &Report) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    for d in &report.diagnostics {
        writeln!(out, "{}", github_line(d))?;
    }
    Ok(())
}

/// One GitHub workflow-command annotation for a diagnostic.
///
/// Everything interpolated here is user-controlled and can arrive in a fork
/// pull request, so it is escaped per GitHub's rules (`@actions/core`): without
/// it a comma corrupts the property list and a newline injects a second, forged
/// command.
fn github_line(d: &Diagnostic) -> String {
    let level = match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    let file = d
        .file
        .as_ref()
        .map(|p| format!("file={},", escape_property(&p.display().to_string())))
        .unwrap_or_default();
    let task = d.task.as_deref().unwrap_or("-");
    let title = escape_property(&format!("{} {task}", d.code));
    format!("::{level} {file}title={title}::{}", escape_data(&d.message))
}

/// GitHub workflow-command *data* escaping (`@actions/core` `escapeData`).
fn escape_data(s: &str) -> String {
    s.replace('%', "%25")
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
    let bold = anstyle::Style::new().bold();

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
    writeln!(out, "ready: {}", summary.ready)
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
            })
        })
        .collect();
    let doc = serde_json::json!({
        "schema": "aplan/summary/v1",
        "phases": phases,
        "ready": summary.ready,
        "total_tasks": summary.total_tasks(),
        "total_done": summary.total_done(),
        "total_effort_days": summary.total_effort_days(),
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
    let bold = anstyle::Style::new().bold();
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
        writeln!(out, "  {:<8}  {:>5.1}d  {}", t.id, t.effort_days, t.title)?;
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

/// Render the dependency graph as Mermaid. A *generated* view, commit it only
/// behind a regenerate-and-diff check, never hand-edit it.
pub fn mermaid(graph: &Graph<'_>) -> String {
    let mut s = String::from("graph TD\n");
    for (id, t) in &graph.by_id {
        let key = node_key(id);
        let label = format!("{}: {}", escape(id), escape(&t.title));
        let shape = if t.status.satisfies_dependency() {
            format!("{key}([\"{label}\"])")
        } else {
            format!("{key}[\"{label}\"]")
        };
        let _ = writeln!(s, "    {shape}");
    }
    for (id, t) in &graph.by_id {
        let to = node_key(id);
        for b in &t.blocked_by {
            let _ = writeln!(s, "    {} --> {to}", node_key(b));
        }
    }
    s
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
    s.replace('"', "'")
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

fn roadmap_human(r: &Roadmap) -> String {
    let mut s = String::new();
    let total: usize = r.cuts.iter().map(|c| c.total).sum::<usize>() + r.cross_cutting.len();
    let _ = writeln!(s, "roadmap: {} cut(s), {total} task(s)", r.cuts.len());
    for c in &r.cuts {
        let ver = if c.version.is_empty() {
            "?"
        } else {
            c.version.as_str()
        };
        let _ = writeln!(
            s,
            "\n{ver}  {}  [{}]  {}/{} done, {} ready, {} blocked, crit {:.1}d",
            c.title,
            c.status,
            c.done,
            c.total,
            c.ready,
            c.blocked,
            c.critical_path.total_effort_days
        );
        for m in &c.members {
            let _ = writeln!(
                s,
                "  {:<10}  {:<12}  {}",
                m.id,
                m.status.to_string(),
                m.title
            );
        }
    }
    if !r.cross_cutting.is_empty() {
        let _ = writeln!(s, "\ncross-cutting (in no cut):");
        for m in &r.cross_cutting {
            let _ = writeln!(
                s,
                "  {:<10}  {:<12}  {}",
                m.id,
                m.status.to_string(),
                m.title
            );
        }
    }
    let _ = writeln!(
        s,
        "\n(statuses are as claimed in the plan and gated by `aplan verify` in CI)"
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
            "total": c.total,
            "done": c.done,
            "ready": c.ready,
            "blocked": c.blocked,
            "critical_path_days": c.critical_path.total_effort_days,
            "critical_path": c.critical_path.tasks.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
            "members": c.members.iter().map(roadmap_task_json).collect::<Vec<_>>(),
        })
    };
    let doc = serde_json::json!({
        "schema": "aplan/roadmap/v1",
        "cuts": r.cuts.iter().map(cut_json).collect::<Vec<_>>(),
        "cross_cutting": r.cross_cutting.iter().map(roadmap_task_json).collect::<Vec<_>>(),
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
pub fn render_roadmap_markdown(roadmap: &Roadmap) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "<!-- generated by `aplan roadmap --format markdown`; do not edit. -->"
    );
    let _ = writeln!(
        s,
        "<!-- regenerate: `cargo run -q -p aplan -- roadmap --format markdown > ROADMAP.status.md` -->"
    );
    let _ = writeln!(s, "\n# Roadmap\n\n## Cuts\n");
    let _ = writeln!(
        s,
        "| cut | status | done/total | ready | blocked | critical path |"
    );
    let _ = writeln!(s, "| --- | --- | --- | --- | --- | --- |");
    for c in &roadmap.cuts {
        let ver = if c.version.is_empty() {
            "?"
        } else {
            c.version.as_str()
        };
        let _ = writeln!(
            s,
            "| {} | {} | {}/{} | {} | {} | {:.1}d |",
            md_cell(ver),
            c.status,
            c.done,
            c.total,
            c.ready,
            c.blocked,
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
        roadmap_member_table(&mut s, &c.members);
    }
    if !roadmap.cross_cutting.is_empty() {
        let _ = writeln!(s, "\n## Cross-cutting (in no cut)\n");
        roadmap_member_table(&mut s, &roadmap.cross_cutting);
    }
    let _ = writeln!(
        s,
        "\n> Statuses are as claimed in the plan and gated by `aplan verify` in CI (ADR-0009)."
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
    use super::*;

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
        let line = github_line(&d);
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
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
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
            id: id.into(),
            title: title.into(),
            status: aplan_core::Status::Todo,
            blocked_by: vec![],
            evidence: vec![],
        }
    }

    fn one_cut(version: &str, title: &str, members: Vec<RoadmapTask>) -> Roadmap {
        Roadmap {
            cuts: vec![Cut {
                version: version.into(),
                title: title.into(),
                milestone_id: "M".into(),
                status: aplan_core::Status::Todo,
                total: members.len(),
                done: 0,
                ready: 0,
                blocked: members.len(),
                members,
                critical_path: CriticalPath {
                    total_effort_days: 0.0,
                    tasks: vec![],
                },
            }],
            cross_cutting: vec![],
        }
    }

    #[test]
    fn roadmap_markdown_escapes_pipes_and_newlines() {
        // A `|` or newline in an id/title/evidence would corrupt the gated table;
        // `check` accepts all of these, so the renderer must neutralise them.
        let m = RoadmapTask {
            id: "A|B".into(),
            title: "line1\nline2".into(),
            status: aplan_core::Status::Done,
            blocked_by: vec![],
            evidence: vec!["file:a|b.md".into()],
        };
        let md = render_roadmap_markdown(&one_cut("v0.1", "Core", vec![m]));
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
        let md = render_roadmap_markdown(&one_cut(
            "v0.1",
            "Core",
            vec![rt("A", "Short"), rt("A-really-quite-long-id", "Long")],
        ));
        assert!(md.contains("| task | title | status | blocked_by | evidence |"));
        assert!(
            md.contains("| A | Short | todo | - | - |"),
            "the short-id row is not padded:\n{md}"
        );
    }

    #[test]
    fn roadmap_markdown_marks_a_versionless_cut() {
        let md = render_roadmap_markdown(&one_cut("", "Loose", vec![rt("A", "T")]));
        assert!(
            md.contains("| ? |"),
            "versionless cut shows `?` in the overview"
        );
        assert!(md.contains("## ? Loose"), "and in the heading");
    }
}
