//! A table is not a checklist, and the refusal has to say so (`A1-129`).
//!
//! A large table-only programme plan passed to the Markdown importer returns a
//! short refusal and an exit code:
//!
//! ```text
//! aplan: PLAN.md: no importable proposal found
//! ```
//!
//! The refusal is right. A table row has no single meaning the way a checkbox
//! does: a table's rows can describe streams, gates, risks, deliverables, metrics,
//! budgets, role assignments, or comparisons. What was missing is that the
//! message said nothing about what it read, what it wanted, or what would work
//! instead (ADR-0019).

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use crate::common;
use common::aplan;

fn fixture() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// A large deterministic shape: 45 tables, 451 data rows, no checkbox rows.
/// A silent empty import cannot come back without this failing.
#[test]
fn the_refusal_measures_the_document_it_declined() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let mut doc = String::from("# Programme plan\n\n");
    // 45 tables whose data rows total 451: forty-four of ten rows, one of eleven.
    for table in 0..45 {
        let rows = if table == 44 { 11 } else { 10 };
        let _ = write!(doc, "## Section {table}\n\n| ID | Scope |\n|---|---|\n");
        for row in 0..rows {
            let _ = writeln!(doc, "| T-{table}-{row} | some scope |");
        }
        doc.push('\n');
    }
    let path = dir.path().join("PLAN.md");
    std::fs::write(&path, &doc).expect("fixture");

    let (out, code) = aplan(
        dir.path(),
        &["import", "PLAN.md", "--format", "checklist", "--dry-run"],
    );
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("45 table(s) with 451 data row(s)"),
        "the refusal must name what it read:\n{out}"
    );
    assert!(
        out.contains("this reads `- [ ]` and `- [x]` rows, and the document has none"),
        "and what it wanted:\n{out}"
    );
    assert!(
        out.contains("A table row has no single meaning"),
        "and why it will not guess:\n{out}"
    );
    assert!(
        out.contains("List the items you mean as `- [ ]` rows"),
        "and the supported next step:\n{out}"
    );
    assert!(
        out.contains("docs/importing-proposals.md"),
        "which is documented, and the message says where:\n{out}"
    );
}

/// An empty file is a different mistake and says so, rather than reporting the
/// tables it does not have.
#[test]
fn an_empty_file_is_not_reported_as_a_document_full_of_tables() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::write(dir.path().join("PLAN.md"), "   \n").expect("fixture");
    let (out, code) = aplan(
        dir.path(),
        &["import", "PLAN.md", "--format", "checklist", "--dry-run"],
    );
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("the file is empty"), "{out}");
    assert!(!out.contains("table(s)"), "{out}");
}

/// The documented recipe, run verbatim.
///
/// The doc's first draft named the wrong column and swept three tables into one
/// import, which is what a plausible-looking command does. Running it here makes
/// the documentation executable: if the pipeline in `docs/importing-proposals.md`
/// stops working, this fails.
#[test]
fn the_documented_extraction_recipe_produces_importable_tasks() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let source = fixture().join("tests/data/programme-section.md");
    std::fs::copy(&source, dir.path().join("PLAN.md")).expect("fixture");

    // Verbatim from `docs/importing-proposals.md`, under "A table is not a
    // checklist": the section, then the first table in it, then the columns.
    let pipeline = concat!(
        r#"awk '/^## 6\. Stream registry/,/^## 7\./' PLAN.md"#,
        r#" | awk '/^\|/{t=1} t&&!/^\|/{exit} t'"#,
        r#" | awk -F'|' 'NR>2 { gsub(/^ +| +$/,"",$2); gsub(/^ +| +$/,"",$5);"#,
        r#" print "- [ ] " $2 ": " $5 }' > streams.md"#,
    );
    let status = Command::new("sh")
        .arg("-c")
        .arg(pipeline)
        .current_dir(dir.path())
        .status()
        .expect("running the documented pipeline");
    assert!(status.success(), "the documented pipeline failed");

    let extracted = std::fs::read_to_string(dir.path().join("streams.md")).expect("streams.md");
    // The middle pass is the one that matters: without it the range takes the
    // metrics table under the same heading as well.
    assert_eq!(
        extracted.lines().count(),
        3,
        "the recipe took rows from more than the one table:\n{extracted}"
    );
    assert!(
        extracted.starts_with("- [ ] `V0`: Fixed scope"),
        "{extracted}"
    );
    assert!(!extracted.contains("Rebuilders"), "{extracted}");

    let (out, code) = aplan(
        dir.path(),
        &[
            "import",
            "streams.md",
            "--format",
            "checklist",
            "--dry-run",
            "--id-prefix",
            "STR",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("would import 3 task(s)"), "{out}");
    // And the import is still gated: generated ids are reported, as they are for
    // any checklist.
    assert!(out.contains("a checklist has no task ids"), "{out}");
}

/// The counts an import is judged on at programme scale, where reading every
/// task is not an option.
#[test]
fn the_report_counts_titles_that_want_a_human() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let long = "x".repeat(200);
    std::fs::write(
        dir.path().join("list.md"),
        format!("- [ ] Short one\n- [ ] Build it. And then explain the write path at length.\n- [ ] {long}\n"),
    )
    .expect("fixture");

    let (out, code) = aplan(
        dir.path(),
        &["import", "list.md", "--format", "checklist", "--dry-run"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("1 title(s) were shortened"),
        "the derived ones:\n{out}"
    );
    assert!(
        out.contains("2 title(s) are the item as written, 1 of them longer than 120 characters"),
        "and the ones a person has to rewrite:\n{out}"
    );
}

/// A ticked box is a claim, and stays one. Nothing in this task changes that, and
/// the criterion asked for it to be proved rather than assumed.
#[test]
fn a_ticked_row_is_still_imported_as_a_claim() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::write(
        dir.path().join("list.md"),
        "- [x] Ship the already-implemented widget\n",
    )
    .expect("fixture");
    let (out, code) = aplan(
        dir.path(),
        &["import", "list.md", "--format", "checklist", "--dry-run"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("todo"), "{out}");
    assert!(!out.contains("  done  "), "{out}");
    assert!(
        out.contains("a checkbox is a claim, not proof"),
        "and the report says why:\n{out}"
    );
}

/// A prose document has no tables, and must not be answered about them. The
/// message said "a table row has no single meaning" to a file containing one
/// heading and three sentences.
#[test]
fn a_document_with_no_tables_is_not_told_about_tables() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::write(
        dir.path().join("prose.md"),
        "# Just prose\n\nNothing here is a list.\n",
    )
    .expect("fixture");
    let (out, code) = aplan(
        dir.path(),
        &["import", "prose.md", "--format", "checklist", "--dry-run"],
    );
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("It has 1 heading(s)"), "{out}");
    // The clause about tables, not the mere word: the message ends with a
    // pointer to a section whose title contains it.
    assert!(!out.contains("A table row has no single meaning"), "{out}");
    assert!(!out.contains("table(s) with"), "{out}");
    // The next step is the same, and still stated.
    assert!(
        out.contains("List the items you mean as `- [ ]` rows"),
        "{out}"
    );
}

/// A document that *shows* a table in a fenced example has no tables. The
/// scanner already knew this, because a roadmap documenting its own syntax was
/// once imported as a task; the counter explaining what the scanner found did
/// not, so it reported examples as content.
#[test]
fn a_table_inside_a_fenced_example_is_not_a_table() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::write(
        dir.path().join("doc.md"),
        "# How to write a table\n\nThis document has no tables. It only shows one:\n\n\
         ```markdown\n| ID | Scope |\n|---|---|\n| A | one |\n| B | two |\n```\n\n\
         ## A real heading\n\n```markdown\n## Not a real heading\n```\n",
    )
    .expect("fixture");

    let (out, code) = aplan(
        dir.path(),
        &["import", "doc.md", "--format", "checklist", "--dry-run"],
    );
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("It has 2 heading(s)"),
        "the two real headings, not the one inside the fence:\n{out}"
    );
    assert!(
        !out.contains("table(s) with"),
        "an example was counted as content:\n{out}"
    );
}

/// The report says what happened, so a run with nothing to rewrite does not
/// report a count of zero.
#[test]
fn an_import_with_no_long_titles_says_nothing_about_rewriting() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::write(
        dir.path().join("list.md"),
        "- [ ] Short one\n- [ ] Short two\n",
    )
    .expect("fixture");
    let (out, code) = aplan(
        dir.path(),
        &["import", "list.md", "--format", "checklist", "--dry-run"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("2 title(s) are the item as written"), "{out}");
    assert!(!out.contains("wanting a rewrite"), "{out}");
    assert!(!out.contains("0 of them"), "{out}");
}

/// `--phase`, which no test invoked (`A1-208`).
///
/// A proposal repository has no equivalent field, so every imported task takes
/// the one the flag names. Left unexercised, an import that silently filed
/// everything at the default phase would look exactly like a working one, and
/// `phase` is the destination reporting coordinate and `summary`'s default row
/// key; roadmap independently groups by cut.
#[test]
fn imported_tasks_take_the_phase_they_are_given() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("PLAN.md"),
        "# Plan\n\n- [ ] Wire the sequencer\n- [x] Land the rail\n",
    )
    .expect("fixture");

    let (out, code) = aplan(
        dir.path(),
        &["import", "PLAN.md", "--format", "checklist", "--phase", "3"],
    );
    assert_eq!(code, 0, "{out}");

    let written: Vec<String> = std::fs::read_dir(dir.path().join("aplan/tasks"))
        .expect("tasks")
        .flatten()
        .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
        .collect();
    assert!(!written.is_empty(), "the import wrote nothing: {out}");
    let wrong: Vec<&String> = written.iter().filter(|b| !b.contains("phase: 3")).collect();
    assert!(
        wrong.is_empty(),
        "{} imported task(s) did not take `--phase 3`:\n{}",
        wrong.len(),
        wrong
            .iter()
            .map(|b| b.lines().take(6).collect::<Vec<_>>().join("\n"))
            .collect::<Vec<_>>()
            .join("\n--\n")
    );
}
