//! A finding names the line it is about, on every surface (`A1-202`).
//!
//! The design document asked for this "from day one" and it did not happen, so
//! one tool answered one run two ways: `check-docs` printed `DOC.md:3:` for a
//! reference in a markdown file, while `check` printed the bare path for a field
//! in a task file. GitHub put the annotation on the file header, and SARIF
//! carried `"startLine": 1` -- a placeholder a code-scanning dashboard renders
//! as fact.
//!
//! These tests do not compare against a line number somebody typed here. They
//! read the file back and assert that the line the tool named really is the
//! field it claims, so the fixture can be edited without silently turning the
//! assertion into a check that 7 equals 7.

use std::path::Path;
use std::process::Command;

mod common;

/// A plan with one task whose `blocked_by` names a task that is not there, and
/// one `done` task citing nothing. The first has a field to point at; the
/// second is *about* an absent field and so has none.
fn fixture(dir: &Path) -> std::path::PathBuf {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    let path = dir.join("aplan/tasks/W-001.md");
    std::fs::write(
        &path,
        "---\nid: W-001\ntitle: points at a task that is not there\nphase: 0\nstatus: todo\n\
         owner: t\neffort_days: 1\nblocked_by:\n  - W-404\n---\n\n## Why\n\nx\n",
    )
    .expect("task");
    std::fs::write(
        dir.join("aplan/tasks/W-002.md"),
        "---\nid: W-002\ntitle: done citing nothing\nphase: 0\nstatus: done\nowner: t\n\
         effort_days: 1\n---\n\n## Why\n\nx\n",
    )
    .expect("task");
    path
}

/// A JSON number as an index, refusing anything that is not one.
fn as_usize(value: &serde_json::Value, what: &str) -> usize {
    let n = value
        .as_u64()
        .unwrap_or_else(|| panic!("{what}, but it is {value}"));
    usize::try_from(n).expect("a line number fits a usize")
}

fn run(dir: &Path, args: &[&str]) -> String {
    let out = common::output(dir, args);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let text = run(dir, args);
    serde_json::from_str(text.trim()).unwrap_or_else(|e| panic!("not JSON: {e}\n{text}"))
}

/// The diagnostic for `code` in a `--format json` document.
fn finding<'a>(doc: &'a serde_json::Value, code: &str) -> &'a serde_json::Value {
    doc["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .find(|d| d["code"] == code)
        .unwrap_or_else(|| panic!("no {code} finding in {doc}"))
}

#[test]
fn a_finding_about_a_field_names_the_line_that_field_is_on() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let path = fixture(dir.path());

    let doc = json(dir.path(), &["check", "--format", "json"]);
    let d = finding(&doc, "APLAN003");
    let line = as_usize(&d["line"], "APLAN003 carries a line");
    let column = d["column"].as_u64().expect("APLAN003 carries a column");

    // Read it back: the assertion is about the file, not about a number.
    let source = std::fs::read_to_string(&path).expect("read back");
    let named = source.lines().nth(line - 1).expect("the line exists");
    assert!(
        named.starts_with("blocked_by:"),
        "line {line} was named for a `blocked_by` finding, but it is {named:?}"
    );
    assert_eq!(column, 1, "a top-level field starts at column one");
    assert_ne!(
        line, 1,
        "line 1 is the placeholder this task exists to remove"
    );
}

/// One run must not answer two ways. Every surface reports the same place.
#[test]
fn every_format_places_the_finding_at_the_same_line() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let doc = json(dir.path(), &["check", "--format", "json"]);
    let line = as_usize(&finding(&doc, "APLAN003")["line"], "a line");

    let human = run(dir.path(), &["check"]);
    assert!(
        human.contains(&format!("W-001.md:{line}:1")),
        "the human `-->` does not carry `path:{line}:1`:\n{human}"
    );

    let github = run(dir.path(), &["check", "--format", "github"]);
    let annotation = github
        .lines()
        .find(|l| l.contains("APLAN003"))
        .expect("an APLAN003 annotation");
    assert!(
        annotation.contains(&format!("line={line},col=1,")),
        "the annotation is still on the file header: {annotation}"
    );

    let sarif = json(dir.path(), &["check", "--format", "sarif"]);
    let result = sarif["runs"][0]["results"]
        .as_array()
        .expect("results")
        .iter()
        .find(|r| r["ruleId"] == "APLAN003")
        .expect("an APLAN003 result");
    let region = &result["locations"][0]["physicalLocation"]["region"];
    assert_eq!(region["startLine"].as_u64(), Some(line as u64), "{region}");
    assert_eq!(region["startColumn"].as_u64(), Some(1));

    let agent = json(dir.path(), &["check", "--format", "agent"]);
    assert_eq!(
        finding(&agent, "APLAN003")["line"].as_u64(),
        Some(line as u64)
    );
}

/// The other half of the specification, and the harder one to get right: a
/// finding with nowhere to point must say so. `APLAN005` is *about* a missing
/// `evidence` field, so there is no `evidence:` line in the file to name, and
/// naming line 1 would be a guess a dashboard would render as fact.
#[test]
fn a_finding_with_no_single_location_says_so_rather_than_guessing() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let doc = json(dir.path(), &["check", "--format", "json"]);
    let d = finding(&doc, "APLAN005");
    assert!(
        d["line"].is_null() && d["column"].is_null(),
        "a finding with no field was given a line: {d}"
    );

    let github = run(dir.path(), &["check", "--format", "github"]);
    let annotation = github
        .lines()
        .find(|l| l.contains("APLAN005"))
        .expect("an APLAN005 annotation");
    assert!(
        !annotation.contains("line="),
        "an unplaceable finding was placed anyway: {annotation}"
    );

    let sarif = json(dir.path(), &["check", "--format", "sarif"]);
    let result = sarif["runs"][0]["results"]
        .as_array()
        .expect("results")
        .iter()
        .find(|r| r["ruleId"] == "APLAN005")
        .expect("an APLAN005 result");
    assert!(
        result["locations"][0]["physicalLocation"]["region"].is_null(),
        "SARIF carried a region for a finding with no location: {result}"
    );
    assert!(
        !result["locations"][0]["physicalLocation"]["artifactLocation"]["uri"].is_null(),
        "the file itself should still be named: {result}"
    );

    let human = run(dir.path(), &["check"]);
    assert!(
        human.lines().any(|l| l.trim_end().ends_with("W-002.md")),
        "the human `-->` should be the bare path when there is no line:\n{human}"
    );
}

/// The one case that can be placed exactly. A file that did not parse has no
/// fields to look up, but the parser knows the line it gave up on -- which is
/// better than any field would have been.
#[test]
fn a_file_that_does_not_parse_is_placed_where_the_parser_stopped() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let broken = dir.path().join("aplan/tasks/W-003.md");
    std::fs::write(
        &broken,
        "---\nid: W-003\ntitle: an unquoted colon: right here\nphase: 0\nstatus: todo\n---\n\n\
         ## Why\n\nx\n",
    )
    .expect("broken task");

    let doc = json(dir.path(), &["check", "--format", "json"]);
    let d = finding(&doc, "APLAN001");
    let line = as_usize(&d["line"], "a parse failure carries a line");
    let column = as_usize(&d["column"], "and a column");

    let source = std::fs::read_to_string(&broken).expect("read back");
    let named = source.lines().nth(line - 1).expect("the line exists");
    assert!(
        named.starts_with("title:"),
        "the parse failure was placed on line {line}, which is {named:?}"
    );
    // The column is the second colon: the one that started a nested mapping.
    assert_eq!(
        named.as_bytes().get(column - 1).copied(),
        Some(b':'),
        "column {column} of {named:?} is not the colon the parser stopped at"
    );
}

/// A line must be a line in the file the finding *names*.
///
/// Locations were first keyed by task id, which is unique in every plan except
/// the one that raises `APLAN002`. With three files claiming one id, collecting
/// by id keeps the last, and the finding about the second file was given the
/// third file's line -- a number that is not even in the file the finding
/// names. Keyed by path now, which is what a location is a place in.
#[test]
fn a_duplicate_id_does_not_borrow_another_file_s_line() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");

    // The same id in three files, with `id:` on a different line in each, so a
    // location taken from the wrong file is visible rather than a coincidence.
    let files = [
        (
            "D-a.md",
            "---\nid: D-001\ntitle: first\nphase: 0\nstatus: todo\nowner: t\n\
                    effort_days: 1\n---\n\n## Why\n\nx\n",
        ),
        (
            "D-b.md",
            "---\ntitle: second\nphase: 0\nstatus: todo\nowner: t\neffort_days: 1\n\
                    tags: [alpha]\nid: D-001\n---\n\n## Why\n\nx\n",
        ),
        (
            "D-c.md",
            "---\nphase: 0\nstatus: todo\nowner: t\neffort_days: 1\ntags: [alpha]\n\
                    traces: []\npaths: []\nprefer_after: []\nid: D-001\ntitle: third\n---\n\
                    \n## Why\n\nx\n",
        ),
    ];
    for (name, body) in files {
        std::fs::write(dir.path().join("aplan/tasks").join(name), body).expect("task");
    }

    let doc = json(dir.path(), &["check", "--format", "json"]);
    let duplicates: Vec<&serde_json::Value> = doc["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .filter(|d| d["code"] == "APLAN002")
        .collect();
    assert_eq!(
        duplicates.len(),
        2,
        "expected one finding per colliding file: {doc}"
    );

    for d in duplicates {
        let named = d["file"].as_str().expect("a file");
        let line = as_usize(&d["line"], "a duplicate-id finding carries a line");
        let source = std::fs::read_to_string(named).expect("read back the file it names");
        let text = source
            .lines()
            .nth(line - 1)
            .unwrap_or_else(|| panic!("{named} has no line {line}: the line is another file's"));
        assert!(
            text.starts_with("id:"),
            "{named}:{line} is {text:?}, not the `id` the finding is about"
        );
    }
}

/// `verify` must place a finding the same way `check` does.
///
/// The evidence half builds its own diagnostics from its own resolvers and did
/// not go through the same placement, so one `verify` run answered two ways
/// about one file: `APLAN023`, raised by the structural half, named the
/// `evidence:` line, while `APLAN011` -- about that same field, in that same
/// file, in that same run -- named nothing. "One tool, one run, two answers" is
/// the complaint this task was filed about, and it survived inside the fix.
#[test]
fn verify_places_an_evidence_finding_where_check_would() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.path().join("f.txt"), "x").expect("file");
    for args in [
        &["init", "-q"][..],
        &["config", "user.email", "t@t"],
        &["config", "user.name", "t"],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .expect("running git")
                .status
                .success(),
            "git {args:?} failed"
        );
    }

    let path = dir.path().join("aplan/tasks/V-001.md");
    std::fs::write(
        &path,
        "---\nid: V-001\ntitle: cites a commit that is not there\nphase: 0\nstatus: done\n\
         owner: t\neffort_days: 1\nevidence:\n  - kind: commit\n    \
         ref: 0123456789abcdef0123456789abcdef01234567\n---\n\n## Why\n\nx\n",
    )
    .expect("task");
    for args in [&["add", "-A"][..], &["commit", "-qm", "seed"]] {
        Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("running git");
    }

    let doc = json(dir.path(), &["verify", "--format", "json"]);
    let unresolved = finding(&doc, "APLAN011");
    let line = as_usize(&unresolved["line"], "a verify finding carries a line");

    let source = std::fs::read_to_string(&path).expect("read back");
    let named = source.lines().nth(line - 1).expect("the line exists");
    assert!(
        named.starts_with("evidence:"),
        "APLAN011 was placed on line {line}, which is {named:?}"
    );

    // Both halves of the same run must agree about the same field.
    let structural = finding(&doc, "APLAN023");
    assert_eq!(
        structural["line"], unresolved["line"],
        "the two halves of one run placed the same field differently: {doc}"
    );
}
