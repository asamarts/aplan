//! Spec-driven task lists: spec-kit's `specs/NNN/tasks.md` and `openspec`'s
//! `openspec/changes/<name>/tasks.md` (`A1-039`).
//!
//! RESEARCH section 15 ranked this camp first: spec-kit discussion #2726 asks
//! for "a deterministic gate... no LLM, `exit 0/1`", which is this tool's exact
//! shape. Both projects generate the same artifact -- a markdown checklist whose
//! items carry ids -- and the difference between them is two tokens, which is
//! why they are one parser and two names rather than two flavors.
//!
//! **The formats, read from the real documents** (fetched 2026-08-17, not
//! reconstructed):
//!
//! ```text
//! spec-kit   - [ ] T012 [P] [US1] Create Entity1 model in src/models/entity1.py
//! OpenSpec   - [x] 1.1 Create `src/core/diff.ts` with diff logic
//! ```
//!
//! What the model can hold: the id, the title, whether the box was ticked, and
//! the heading it sat under. What it cannot, and what this reports rather than
//! guesses:
//!
//! - **Order.** spec-kit writes a `## Dependencies & Execution Order` section in
//!   English -- "Models before services", "User Story 2 may integrate with US1".
//!   Turning that into `blocked_by` would be a guess dressed as a dependency,
//!   which is the one thing an evidence-gated planner must not do. It is
//!   reported, with the line count, and `aplan dep add` is the answer.
//! - **`[P]`, the parallel marker.** It says an item shares no files with its
//!   neighbours. That is an assertion about the code, not about the plan, and
//!   nothing here can check it; it becomes a tag.
//! - **File paths in descriptions.** spec-kit's own format section says "include
//!   exact file paths in descriptions", which reads like `paths` waiting to be
//!   extracted. Measured before declining: **zero of the 34 items** in the
//!   upstream template carry a resolvable path (they are placeholders like
//!   `src/models/[entity1].py`), and the one real item in that repository writes
//!   "in hello.py" inside an English sentence. Extracting those would mean
//!   parsing prose to produce the field that makes `commit` evidence checkable,
//!   so a wrong guess would weaken exactly what this tool exists to strengthen.
//!   Declined, reported, and left to `aplan` the human.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use aplan_core::task::{Status, Task};

use crate::report::{Notice, NoticeKind, Report};
use crate::{Import, ImportError, Options, ProposalSource, checklist};

/// The `spec-kit` flavor: `T001`-style ids, `[P]` and `[US1]` markers.
#[derive(Debug, Default, Clone, Copy)]
pub struct SpecKit;

/// The `openspec` flavor: `1.1`-style ids, no markers.
///
/// A separate name because that is what an adopter types, and the same parser
/// because that is what the documents are. `A1-039` asked for "a second flavor,
/// or a recorded finding that its shape is close enough that a second flavor
/// would be duplication": this type is the finding, written as code.
#[derive(Debug, Default, Clone, Copy)]
pub struct OpenSpec;

/// What differs between the two documents, which is all that differs.
#[derive(Debug, Clone, Copy)]
struct Dialect {
    flavor: &'static str,
    /// How an item's id is written, immediately after the checkbox.
    id: IdShape,
    /// Whether `[P]` and `[US<n>]` markers are expected.
    markers: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdShape {
    /// `T001`, `T12`.
    Prefixed,
    /// `1.1`, `2.10`, `3`.
    Dotted,
}

const SPEC_KIT: Dialect = Dialect {
    flavor: "spec-kit",
    id: IdShape::Prefixed,
    markers: true,
};
const OPEN_SPEC: Dialect = Dialect {
    flavor: "openspec",
    id: IdShape::Dotted,
    markers: false,
};

impl ProposalSource for SpecKit {
    fn flavor(&self) -> &'static str {
        SPEC_KIT.flavor
    }
    fn parse(
        &self,
        source: &str,
        contents: &str,
        options: &Options,
    ) -> Result<Import, ImportError> {
        parse_tasks(SPEC_KIT, source, contents, options)
    }
}

impl ProposalSource for OpenSpec {
    fn flavor(&self) -> &'static str {
        OPEN_SPEC.flavor
    }
    fn parse(
        &self,
        source: &str,
        contents: &str,
        options: &Options,
    ) -> Result<Import, ImportError> {
        parse_tasks(OPEN_SPEC, source, contents, options)
    }
}

/// One item as the document wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    /// The source's own id, verbatim: `T012`, `1.1`.
    source_id: String,
    title: String,
    checked: bool,
    /// The `##` heading it sat under.
    section: Option<String>,
    /// `[P]`, `[US1]`, in the order they appeared.
    markers: Vec<String>,
    /// The prose under this item's heading, before the first item: spec-kit's
    /// `**Purpose**:` and `**Goal**:` lines, `openspec`'s opening note. Kept
    /// because it is the only description of *why* a group of items exists.
    prose: String,
    /// Indent depth: 0 for a top-level item, more for a nested one. The model
    /// has no nesting, so a nested item becomes a task of its own and the
    /// flattening is reported, exactly as `checklist` reports it.
    depth: usize,
    line: usize,
}

fn parse_tasks(
    dialect: Dialect,
    source_name: &str,
    contents: &str,
    options: &Options,
) -> Result<Import, ImportError> {
    if contents.trim().is_empty() {
        return Err(ImportError::Empty {
            source_name: source_name.to_string(),
            detail: Some(": the file is empty".to_string()),
        });
    }
    let (items, ordering_prose, dropped_lines) = scan(dialect, contents);
    if items.is_empty() {
        return Err(ImportError::Empty {
            source_name: source_name.to_string(),
            detail: Some(declined(dialect)),
        });
    }

    let mut report = Report::new();
    let prefix = options.id_prefix.as_str();
    report.push(Notice::new(
        source_name,
        NoticeKind::Provenance,
        if prefix.is_empty() {
            format!(
                "{} item(s) kept the ids the document gave them, unqualified. A `{}` repository \
                 writes the same ids in every feature's `tasks.md`, so importing a second \
                 feature will collide with these and be refused: pass `--id-prefix` to qualify \
                 them",
                items.len(),
                dialect.flavor
            )
        } else {
            format!(
                "{} item(s) kept the ids the document gave them, qualified with `{prefix}-`: a \
                 `{}` repository writes the same ids in every feature's `tasks.md`, so \
                 unqualified they would collide on the second import",
                items.len(),
                dialect.flavor
            )
        },
    ));
    report_ordering(dialect, source_name, ordering_prose, &mut report);
    if dropped_lines > 0 {
        report.push(Notice::new(
            source_name,
            NoticeKind::DroppedField,
            format!(
                "{dropped_lines} line(s) of prose *between* items became no task and no body: \
                 the checkpoint and purpose notes a group carries after its first item, and the \
                 template's HTML comments. What sits under a heading before its first item is \
                 kept in the bodies"
            ),
        ));
    }
    report_markers(&items, source_name, &mut report);
    report_nesting(&items, source_name, &mut report);

    let (tasks, bodies) = build(dialect, &items, source_name, options, &mut report);

    Ok(Import {
        tasks,
        report,
        bodies,
    })
}

/// One task and one body per item, and the report of what that cost.
///
/// Split from `parse_tasks` so each half is readable: the scan decides what the
/// document *said*, this decides what the model can hold of it.
fn build(
    dialect: Dialect,
    items: &[Item],
    source_name: &str,
    options: &Options,
    report: &mut Report,
) -> (Vec<Task>, Vec<String>) {
    let prefix = options.id_prefix.as_str();
    let mut tasks = Vec::with_capacity(items.len());
    let mut bodies = Vec::with_capacity(items.len());
    for item in items {
        let id = if prefix.is_empty() {
            item.source_id.clone()
        } else {
            format!("{prefix}-{}", item.source_id)
        };
        // A ticked box is a claim, and importing it as `done` would be this tool
        // asserting what it cannot resolve (ADR-0012 clause 4). The shared
        // helper keeps that answer identical to the checklist flavor's, which is
        // the point of sharing it.
        let (status, evidence) = if item.checked {
            checklist::resolve_checked(&id, source_name, options, report)
        } else {
            (Status::Todo, Vec::new())
        };
        let mut tags: Vec<String> = Vec::new();
        if let Some(section) = &item.section {
            let tag = checklist::tag_for(section);
            if !tag.is_empty() {
                tags.push(tag);
            }
        }
        for marker in &item.markers {
            tags.push(marker_tag(marker));
        }
        if item.checked {
            tags.push("checklist:checked".to_string());
        }
        // An item that writes `[P] [US1] [P]` is one claim, not two, and a tag
        // repeated in a task file is noise a reader has to check (`A1-039`,
        // found by its audit). Order is kept, so the first mention wins.
        let mut seen = std::collections::BTreeSet::new();
        tags.retain(|tag| seen.insert(tag.clone()));
        let title = checklist::title_for(&item.title).unwrap_or(&item.title);
        bodies.push(body_for(dialect, source_name, item, title));
        tasks.push(Task {
            id,
            title: title.to_string(),
            phase: options.phase,
            status,
            owner: None,
            effort_days: None,
            blocked_by: Vec::new(),
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
    }

    (tasks, bodies)
}

/// Split the document into items, and count the lines of ordering prose.
///
/// Line-based, for `checklist`'s reason: the shape is a convention rather than a
/// language, and a markdown parser would accept documents this convention does
/// not cover while still not understanding the English in them.
fn scan(dialect: Dialect, contents: &str) -> (Vec<Item>, usize, usize) {
    let mut items: Vec<Item> = Vec::new();
    let mut section: Option<String> = None;
    // Prose under the current heading, before its first item, and the count of
    // everything else this cannot hold. ADR-0012 forbids a silent omission, and
    // the first version of this dropped 85 lines of the upstream template --
    // `**Purpose**`, `**Checkpoint**`, the HTML comments -- without a word.
    let mut prose = String::new();
    let mut prose_open = true;
    let mut dropped_lines = 0;
    // The item a continuation line would belong to, while one is open.
    let mut open_item: Option<usize> = None;
    // The depth of the ordering heading, while inside it. Depth matters: the
    // ordering section has sub-headings (`### Within Each User Story`), and
    // treating one of those as leaving the section made the prose invisible and
    // the notice silent -- which is the notice this flavor most needs to emit.
    let mut ordering_depth: Option<usize> = None;
    let mut ordering_lines = 0;

    for (n, raw) in contents.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with("##") {
            open_item = None;
            let depth = line.chars().take_while(|c| *c == '#').count();
            let heading = line.trim_start_matches('#').trim();
            ordering_depth = match ordering_depth {
                Some(open) if depth > open => Some(open),
                _ => is_ordering_heading(heading).then_some(depth),
            };
            section = (!heading.is_empty()).then(|| heading.to_string());
            prose.clear();
            prose_open = true;
            continue;
        }
        let Some(rest) = checkbox(line) else {
            if line.is_empty() {
                open_item = None;
                continue;
            }
            // A wrapped item: the continuation belongs to the item above, and
            // dropping it truncated the description mid-sentence (`A1-039`,
            // found by its audit).
            if let Some(at) = open_item
                && raw.starts_with(char::is_whitespace)
                && ordering_depth.is_none()
            {
                let item: &mut Item = &mut items[at];
                item.title.push(' ');
                item.title.push_str(line);
                continue;
            }
            if ordering_depth.is_some() {
                ordering_lines += 1;
            } else if prose_open && !line.starts_with("<!--") && !line.starts_with("---") {
                if !prose.is_empty() {
                    prose.push('\n');
                }
                prose.push_str(line);
            } else {
                dropped_lines += 1;
            }
            continue;
        };
        let (checked, body) = rest;
        let Some((source_id, remainder)) = split_id(dialect, body) else {
            continue; // A checkbox with no id is not this format's item.
        };
        let (markers, title) = split_markers(dialect, remainder);
        // Runs of spaces in a source line are layout, not content, and a title
        // is display text.
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            continue; // An id with no description is a task nobody can find.
        }
        prose_open = false;
        open_item = Some(items.len());
        items.push(Item {
            source_id,
            title,
            checked,
            section: section.clone(),
            markers,
            prose: prose.clone(),
            depth: raw.len() - raw.trim_start().len(),
            line: n + 1,
        });
    }
    (items, ordering_lines, dropped_lines)
}

/// `- [ ] rest` / `- [x] rest`, at any indent.
fn checkbox(line: &str) -> Option<(bool, &str)> {
    let rest = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))?;
    let checked = if rest.starts_with("[x]") || rest.starts_with("[X]") {
        true
    } else if rest.starts_with("[ ]") {
        false
    } else {
        return None;
    };
    Some((checked, rest[3..].trim()))
}

/// The id the document wrote, and what follows it.
fn split_id(dialect: Dialect, body: &str) -> Option<(String, &str)> {
    let (head, rest) = match body.split_once(char::is_whitespace) {
        Some((head, rest)) => (head, rest.trim_start()),
        None => (body, ""),
    };
    let head = head.trim_end_matches(['.', ':']);
    let looks_right = match dialect.id {
        IdShape::Prefixed => {
            let mut chars = head.chars();
            chars.next().is_some_and(|c| c.is_ascii_uppercase())
                && head.len() > 1
                && chars.all(|c| c.is_ascii_digit())
        }
        IdShape::Dotted => {
            !head.is_empty()
                && head.chars().all(|c| c.is_ascii_digit() || c == '.')
                && head.chars().any(|c| c.is_ascii_digit())
        }
    };
    looks_right.then(|| (head.to_string(), rest))
}

/// Peel `[P]` and `[US1]` off the front of a description.
fn split_markers(dialect: Dialect, mut text: &str) -> (Vec<String>, String) {
    let mut markers = Vec::new();
    if dialect.markers {
        while let Some(rest) = text.strip_prefix('[') {
            let Some((marker, tail)) = rest.split_once(']') else {
                break;
            };
            // Only the two the format documents. A description that opens with
            // `[some prose]` is a description, and eating it would lose the
            // title.
            let known = marker.eq_ignore_ascii_case("p")
                || (marker.len() > 2
                    && marker[..2].eq_ignore_ascii_case("us")
                    && marker[2..].chars().all(|c| c.is_ascii_digit()));
            if !known {
                break;
            }
            markers.push(marker.to_string());
            text = tail.trim_start();
        }
    }
    (markers, text.trim().to_string())
}

/// `P` becomes `spec-kit:parallel`, `US1` becomes `spec-kit:us1`.
fn marker_tag(marker: &str) -> String {
    if marker.eq_ignore_ascii_case("p") {
        "spec-kit:parallel".to_string()
    } else {
        format!("spec-kit:{}", marker.to_ascii_lowercase())
    }
}

fn is_ordering_heading(heading: &str) -> bool {
    let lower = heading.to_ascii_lowercase();
    lower.starts_with("dependencies") || lower.contains("execution order")
}

/// The ordering this cannot represent, reported rather than invented.
fn report_ordering(dialect: Dialect, source_name: &str, lines: usize, report: &mut Report) {
    if lines == 0 {
        return;
    }
    report.push(Notice::new(
        source_name,
        NoticeKind::DroppedField,
        format!(
            "{lines} line(s) of ordering prose under the `{}` document's dependencies heading \
             became no `blocked_by` edge: it is English about phases and stories (\"models \
             before services\"), and inferring edges from it would be a guess dressed as a \
             dependency. Declare the ones that matter with `aplan dep add <id> --after <id>`",
            dialect.flavor
        ),
    ));
}

/// Nesting the model does not hold, reported rather than silently flattened.
fn report_nesting(items: &[Item], source_name: &str, report: &mut Report) {
    let nested = items.iter().filter(|i| i.depth > 0).count();
    if nested == 0 {
        return;
    }
    report.push(Notice::new(
        source_name,
        NoticeKind::Placement,
        format!(
            "{nested} item(s) were indented under another and became tasks of their own: this \
             model has no nesting, and the sub-item relationship is not a dependency. Where it \
             was one, say so with `aplan dep add <id> --after <id>`"
        ),
    ));
}

fn report_markers(items: &[Item], source_name: &str, report: &mut Report) {
    let parallel = items
        .iter()
        .filter(|i| i.markers.iter().any(|m| m.eq_ignore_ascii_case("p")))
        .count();
    if parallel > 0 {
        report.push(Notice::new(
            source_name,
            NoticeKind::DroppedField,
            format!(
                "{parallel} item(s) were marked `[P]`, which claims they share no files with \
                 their neighbours. That is an assertion about the code rather than about the \
                 plan and nothing here can check it, so it became the tag `spec-kit:parallel`"
            ),
        ));
    }
    let stories: std::collections::BTreeSet<&str> = items
        .iter()
        .flat_map(|i| i.markers.iter())
        .filter(|m| !m.eq_ignore_ascii_case("p"))
        .map(String::as_str)
        .collect();
    // ADR-0014: one modelled grouping level, and a second lives in tags.
    if !stories.is_empty() {
        report.push(Notice::new(
            source_name,
            NoticeKind::Placement,
            format!(
                // The decision number stays in the code comment above: a notice
                // is read in the adopter's terminal, where this project's
                // `docs/adr/` does not resolve (`A1-044`).
                "{} user story grouping(s) became tags ({}); `aplan` models one grouping level \
                 and it is the cut, so if these are your cuts, move the tasks into milestones \
                 with `aplan mv --cut`",
                stories.len(),
                stories
                    .iter()
                    .map(|s| format!("`spec-kit:{}`", s.to_ascii_lowercase()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
}

/// The task body: where it came from, and what the source said.
fn body_for(dialect: Dialect, source_name: &str, item: &Item, title: &str) -> String {
    let mut s = format!(
        "Imported from `{source_name}` (`{}`), item `{}` on line {}.\n",
        dialect.flavor, item.source_id, item.line
    );
    if let Some(section) = &item.section {
        let _ = write!(s, "\nUnder the heading: {section}\n");
    }
    // An item in these documents is often a paragraph of instruction -- one real
    // OpenSpec item runs to 300 characters -- and a title that long is unusable
    // in `ready`. The title is the first sentence, and the rest lands here so
    // nothing the source said is lost (`A1-039`).
    if title != item.title {
        let _ = write!(s, "\nWhat the source wrote:\n\n{}\n", item.title);
    }
    if !item.prose.is_empty() {
        let _ = write!(s, "\nContext under that heading:\n\n{}\n", item.prose);
    }
    if !item.markers.is_empty() {
        let _ = write!(
            s,
            "\nThe source marked it `[{}]`.\n",
            item.markers.join("] [")
        );
    }
    s
}

fn declined(dialect: Dialect) -> String {
    match dialect.id {
        IdShape::Prefixed => ": no `- [ ] T001 ...` items were found, which is what \
                             `--format spec-kit` reads. A checklist whose items carry no ids is \
                             `--format checklist`"
            .to_string(),
        IdShape::Dotted => ": no `- [ ] 1.1 ...` items were found, which is what \
                           `--format openspec` reads. A checklist whose items carry no ids is \
                           `--format checklist`"
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real upstream template, trimmed to the shapes that matter
    /// (github/spec-kit, `templates/tasks-template.md`, fetched 2026-08-17).
    const SPEC_KIT_DOC: &str = "# Tasks: Feature\n\
        \n\
        ## Phase 1: Setup (Shared Infrastructure)\n\
        \n\
        - [ ] T001 Create project structure per implementation plan\n\
        - [x] T003 [P] Configure linting and formatting tools\n\
        \n\
        ## Phase 3: User Story 1 - Title (Priority: P1)\n\
        \n\
        - [ ] T012 [P] [US1] Create Entity1 model in src/models/entity1.py\n\
        - [ ] T014 [US1] Implement Service in src/services/service.py\n\
        \n\
        ## Dependencies & Execution Order\n\
        \n\
        ### Within Each User Story\n\
        \n\
        - Models before services\n\
        - Services before endpoints\n";

    /// A real `openspec` change (`Fission-AI/OpenSpec`,
    /// `openspec/changes/archive/2025-08-13-add-diff-command/tasks.md`).
    const OPEN_SPEC_DOC: &str = "# Implementation Tasks\n\
        \n\
        ## 1. Core Implementation\n\
        - [x] 1.1 Create `src/core/diff.ts` with diff logic\n\
        - [x] 1.2 Implement change directory scanning\n\
        \n\
        ## 2. CLI Integration\n\
        - [ ] 2.1 Add diff command to `src/cli/index.ts`\n";

    fn opts() -> Options {
        Options {
            id_prefix: "SK".to_string(),
            ..Options::default()
        }
    }

    #[test]
    fn a_spec_kit_document_keeps_its_own_ids() {
        let import = SpecKit
            .parse("tasks.md", SPEC_KIT_DOC, &opts())
            .expect("parses");
        let ids: Vec<&str> = import.tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["SK-T001", "SK-T003", "SK-T012", "SK-T014"]);
        assert_eq!(
            import.tasks[0].title,
            "Create project structure per implementation plan"
        );
        // The heading becomes a tag, and the story and parallel markers become
        // tags of their own rather than disappearing.
        assert!(
            import.tasks[2]
                .tags
                .contains(&"spec-kit:parallel".to_string())
        );
        assert!(import.tasks[2].tags.contains(&"spec-kit:us1".to_string()));
        assert!(import.tasks[3].tags.contains(&"spec-kit:us1".to_string()));
        assert!(
            !import.tasks[3]
                .tags
                .contains(&"spec-kit:parallel".to_string())
        );
    }

    /// The ordering is the thing this format cannot give and must not invent.
    #[test]
    fn ordering_prose_becomes_a_notice_and_never_an_edge() {
        let import = SpecKit
            .parse("tasks.md", SPEC_KIT_DOC, &opts())
            .expect("parses");
        assert!(
            import.tasks.iter().all(|t| t.blocked_by.is_empty()),
            "no edge may be invented from English"
        );
        let text = import.report.to_string();
        assert!(text.contains("became no `blocked_by` edge"), "{text}");
        assert!(
            text.contains("aplan dep add"),
            "the answer is named: {text}"
        );
    }

    /// A ticked box is a claim (ADR-0012 clause 4), here as in `checklist`.
    #[test]
    fn a_ticked_box_imports_as_todo_and_says_so() {
        let import = SpecKit
            .parse("tasks.md", SPEC_KIT_DOC, &opts())
            .expect("parses");
        let ticked = &import.tasks[1];
        assert_eq!(ticked.status, Status::Todo);
        assert!(ticked.tags.contains(&"checklist:checked".to_string()));
        assert!(
            import
                .report
                .to_string()
                .contains("was ticked in the source")
        );
    }

    #[test]
    fn an_openspec_document_reads_through_the_same_parser() {
        let import = OpenSpec
            .parse("tasks.md", OPEN_SPEC_DOC, &opts())
            .expect("parses");
        let ids: Vec<&str> = import.tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["SK-1.1", "SK-1.2", "SK-2.1"]);
        assert_eq!(
            import.tasks[0].title,
            "Create `src/core/diff.ts` with diff logic"
        );
        // No markers in this dialect, so nothing is peeled off a description
        // that happens to start with a bracket.
        assert!(
            import
                .tasks
                .iter()
                .all(|t| !t.tags.iter().any(|tag| tag.starts_with("spec-kit:"))),
            "openspec has no `[P]`/`[US]` markers to report"
        );
    }

    /// Each dialect reads its own document and declines the other's, which is
    /// what makes two names honest rather than decorative.
    #[test]
    fn a_dialect_declines_the_other_dialects_document() {
        let err = OpenSpec
            .parse("tasks.md", SPEC_KIT_DOC, &opts())
            .expect_err("T001 is not a dotted id");
        assert!(err.to_string().contains("--format openspec"), "{err}");
        let err = SpecKit
            .parse("tasks.md", OPEN_SPEC_DOC, &opts())
            .expect_err("1.1 is not a prefixed id");
        assert!(err.to_string().contains("--format spec-kit"), "{err}");
    }

    /// ADR-0012 forbids a silent omission, and these documents are full of prose
    /// (`**Purpose**`, `**Checkpoint**`, HTML comments). What sits under a
    /// heading before its first item is kept in the body; the rest is counted
    /// and named. The first version of this dropped 85 lines of the upstream
    /// template without a word.
    #[test]
    fn prose_is_either_carried_into_a_body_or_counted() {
        let doc = "## Phase 1: Setup\n\
            \n\
            **Purpose**: Project initialization and basic structure\n\
            \n\
            - [ ] T001 Create the project structure\n\
            \n\
            **Checkpoint**: Foundation ready\n";
        let import = SpecKit.parse("tasks.md", doc, &opts()).expect("parses");
        assert_eq!(import.tasks.len(), 1);
        assert!(
            import.bodies[0].contains("**Purpose**: Project initialization"),
            "context before the first item is kept: {}",
            import.bodies[0]
        );
        let text = import.report.to_string();
        assert!(
            text.contains("1 line(s) of prose *between* items"),
            "and what is not kept is counted: {text}"
        );
    }

    /// An item that runs to a paragraph gets a title of its first sentence, and
    /// the rest of what the source wrote lands in the body rather than in a
    /// title nobody can read in `ready` (`A1-039`, measured on a real `openspec`
    /// item of 300 characters).
    #[test]
    fn a_paragraph_item_keeps_its_whole_text_in_the_body() {
        let doc = "## 1. Work\n- [ ] 1.1 Create the thing. Then wire it up, mirroring the \
                   other thing, and register it in the manifest.\n";
        let import = OpenSpec.parse("tasks.md", doc, &opts()).expect("parses");
        assert_eq!(import.tasks[0].title, "Create the thing.");
        assert!(
            import.bodies[0].contains("register it in the manifest"),
            "nothing the source said is lost: {}",
            import.bodies[0]
        );
    }

    /// The four shapes this task's audit found, each of which lost or invented
    /// something (`A1-039`).
    #[test]
    fn a_wrapped_item_keeps_its_whole_sentence() {
        let doc = "## 1. Work\n\
            - [ ] 1.1 A first line that wraps\n\
            \x20     onto a second line with the rest of it.\n\
            - [ ] 1.2 Short one\n";
        let import = OpenSpec.parse("tasks.md", doc, &opts()).expect("parses");
        assert_eq!(import.tasks.len(), 2, "the continuation is not a task");
        assert!(
            import.tasks[0]
                .title
                .contains("wraps onto a second line with the rest of it."),
            "the sentence was truncated: {}",
            import.tasks[0].title
        );
    }

    #[test]
    fn a_repeated_marker_is_one_tag_and_layout_is_not_content() {
        let doc = "## Phase 1: Setup\n- [ ] T002 [P] [US1] [P] Repeated   markers\n";
        let import = SpecKit.parse("tasks.md", doc, &opts()).expect("parses");
        let tags = &import.tasks[0].tags;
        assert_eq!(
            tags.iter().filter(|t| *t == "spec-kit:parallel").count(),
            1,
            "a tag repeated in a task file is noise: {tags:?}"
        );
        assert_eq!(
            import.tasks[0].title, "Repeated markers",
            "runs of spaces are layout"
        );
    }

    #[test]
    fn a_nested_item_is_flattened_and_the_flattening_is_reported() {
        let doc = "## Phase 1: Setup\n\
            - [ ] T003 A parent\n\
            \x20 - [ ] T004 A nested item\n";
        let import = SpecKit.parse("tasks.md", doc, &opts()).expect("parses");
        assert_eq!(import.tasks.len(), 2, "a nested item is a task of its own");
        let text = import.report.to_string();
        assert!(text.contains("indented under another"), "{text}");
        assert!(
            import.tasks.iter().all(|t| t.blocked_by.is_empty()),
            "nesting is not a dependency"
        );
    }

    /// A description opening with a bracket keeps its title.
    #[test]
    fn only_the_documented_markers_are_peeled() {
        let doc = "## Phase 1: Setup\n- [ ] T001 [refactor] rename the thing\n";
        let import = SpecKit.parse("tasks.md", doc, &opts()).expect("parses");
        assert_eq!(import.tasks[0].title, "[refactor] rename the thing");
        assert!(
            import.tasks[0]
                .tags
                .iter()
                .all(|t| !t.starts_with("spec-kit:"))
        );
    }
}
