//! Docs-vs-facts gate: the rules and commands the tool actually has (from
//! `aplan::facts()`, the generated source of truth) must be documented, so a new
//! rule or command cannot ship undocumented, the "six invariants over a table of
//! ten" class of drift this project was burned by (HLD Section 16).
//!
//! **What changed, and why it is a check rather than a generator (`A1-092`).**
//! The first version of this file asked whether `HLD.md` *contained* each rule
//! code, anywhere in the file, and never consulted `README.md` for a rule at all.
//! A rule named once in a paragraph satisfied it forever, so everything about
//! *how* a rule was documented went unchecked. Two defects were sitting in the
//! tree that it could not see: `HLD.md` carried the `APLAN027` row twice, and
//! `README.md` claimed three warnings above a table marking seven.
//!
//! The obvious fix was to generate the tables, the way `schemas/v1/*.json` and
//! `ROADMAP.status.md` are generated. Reading them first showed that to be wrong.
//! None of these tables is purely derived: the HLD catalog documents `APLAN016`
//! which is planned and absent from `facts`, and cites the task
//! that shipped each rule; the HLD CLI table carries planned commands with a
//! status column; the README command block is curated examples with arguments;
//! and the README rule table is the *always-on* set on purpose, with the three
//! opt-in rules covered in their own paragraph below it. Generating any of them
//! would delete real content in order to gate a fraction of it.
//!
//! So the tables stay hand-written and this gate reads their *structure*: one row
//! per rule, severity agreeing with the code, a rule absent from `facts` allowed
//! only where the row says `planned`, and every command documented in both
//! directions.
//!
//! **What the adversarial pass added.** The first version strengthened the *rule*
//! half and left the *command* half on the same anywhere-in-the-file `contains`
//! test it had removed as useless: it passed while `drop`, `mv` and `tasks` were
//! absent from the README's command list, satisfied by incidental mentions in
//! prose. Commands are now held to a row and to a place in that list. Two further
//! properties came out of the same pass: tables inside code fences are not read as
//! tables (a doc *showing* this format would otherwise break the gate), and every
//! invocation the README documents is handed to the real clap parser, which is how
//! `aplan mv --to`, a flag that does not exist, was caught minutes after being
//! written into the file this gate protects.

use crate::common;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

fn repo_file(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn repository_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn assert_public_plan_epoch() {
    let boundary = repository_root().join("aplan/tasks/A2-001.md");
    let text = std::fs::read_to_string(&boundary)
        .unwrap_or_else(|error| panic!("read {}: {error}", boundary.display()));
    assert!(
        text.contains("explicit epoch boundary")
            && text.contains("Earlier implementation records remain in the private"),
        "private repository audits may be absent only at the documented A2 import boundary"
    );
}

#[test]
fn roadmap_narrative_declares_every_current_cut_member() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let run = common::run(&root, &["check-docs", "ROADMAP.md"]);
    assert_eq!(run.code, 0, "{}", run.text());
    assert!(run.stdout.contains("ok: 1 document(s)"), "{}", run.text());
}

#[test]
fn validation_assurance_vocabulary_has_one_documented_authority() {
    let ci = repo_file("ci/README.md");
    let contributing = repo_file("CONTRIBUTING.md");

    for term in ["selected", "full", "release"] {
        let definition = format!("- **{term}**");
        assert_eq!(
            ci.matches(&definition).count(),
            1,
            "ci/README.md must define `{term}` exactly once"
        );
        assert!(
            !contributing.contains(&definition),
            "CONTRIBUTING.md must link to the vocabulary, not redefine `{term}`"
        );
    }
    assert!(
        contributing.contains("ci/README.md#run-the-gates-locally"),
        "CONTRIBUTING.md must point to the canonical assurance vocabulary"
    );
    assert!(
        contributing.contains("selected feedback,\nnot a full or verified claim"),
        "the contribution guide must not overstate selected development feedback"
    );
}

/// End-to-end proof for `check_docs.complete_multi_hyphen_ids`: the CLI learns
/// the P00024-shaped family solely from the loaded plan, resolves exact ids and
/// reports an unknown sibling under its complete spelling.
#[test]
fn check_docs_resolves_complete_multi_hyphen_ids_end_to_end() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let tasks = dir.path().join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("plan directory");
    for id in ["M-p0-design", "M-p0-build", "M-p1-read"] {
        std::fs::write(
            tasks.join(format!("{id}.md")),
            format!("---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: todo\ntags: [milestone]\n---\n"),
        )
        .expect("task");
    }
    std::fs::write(
        dir.path().join("NOTES.md"),
        "Known: `M-p0-design`, M-p0-build; M-p1-read.\nUnknown: `M-p0-review`.\n",
    )
    .expect("document");

    let run = common::run(dir.path(), &["check-docs", "NOTES.md"]);
    assert_eq!(run.code, 1, "{}", run.text());
    assert!(run.stdout.contains("`M-p0-review`"), "{}", run.text());
    assert!(!run.stdout.contains("`M-p0` "), "{}", run.text());
    assert!(
        run.stdout.contains("1 reference(s) naming no task"),
        "known complete ids were misreported: {}",
        run.text()
    );
}

/// End-to-end proof for `check_docs.identifier_boundaries_are_symmetric`: the
/// CLI must not accept or report a shorter task id cut from a larger word.
#[test]
fn check_docs_does_not_truncate_ids_before_an_underscore() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let tasks = dir.path().join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("plan directory");
    for id in ["A1-557", "M-v0.8", "M-p0-design"] {
        std::fs::write(
            tasks.join(format!("{id}.md")),
            format!("---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: todo\ntags: [milestone]\n---\n"),
        )
        .expect("task");
    }
    std::fs::write(
        dir.path().join("NOTES.md"),
        "Joined: A1-557_suffix A1-999_suffix M-v0.8_suffix M-p0-design_suffix.\n\
         Real: A1-999.\n",
    )
    .expect("document");

    let run = common::run(dir.path(), &["check-docs", "NOTES.md"]);
    assert_eq!(run.code, 1, "{}", run.text());
    assert!(run.stdout.contains("`A1-999`"), "{}", run.text());
    assert!(
        run.stdout.contains("1 reference(s) naming no task"),
        "joined tokens were truncated: {}",
        run.text()
    );
}

fn facts() -> Value {
    serde_json::from_str(&aplan::facts()).expect("facts() is JSON")
}

/// The declared severity of every rule, by code.
fn rule_severities() -> BTreeMap<String, String> {
    facts()["rules"]
        .as_array()
        .expect("rules array")
        .iter()
        .map(|r| {
            (
                r["code"].as_str().expect("code").to_string(),
                r["severity"].as_str().expect("severity").to_string(),
            )
        })
        .collect()
}

fn command_names() -> BTreeSet<String> {
    facts()["commands"]
        .as_array()
        .expect("commands array")
        .iter()
        .map(|c| c["name"].as_str().expect("name").to_string())
        .collect()
}

fn format_names() -> BTreeSet<String> {
    facts()["formats"]
        .as_array()
        .expect("formats array")
        .iter()
        .map(|f| f.as_str().expect("format name").to_string())
        .collect()
}

/// One second-level Markdown section, through the next one or EOF.
fn hld_section<'a>(hld: &'a str, heading: &str) -> &'a str {
    let marker = format!("\n## {heading}\n");
    let rest = hld
        .split_once(&marker)
        .unwrap_or_else(|| panic!("HLD.md has a `{heading}` section"))
        .1;
    rest.split_once("\n## ")
        .map_or(rest, |(section, _)| section)
}

fn first_code_cell(cells: &[String]) -> Option<String> {
    let first = cells.first()?;
    (first.starts_with('`') && first.ends_with('`')).then(|| first.trim_matches('`').to_string())
}

fn workspace_crates() -> BTreeSet<String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut manifests = vec![root.join("xtask/Cargo.toml")];
    manifests.extend(
        std::fs::read_dir(root.join("crates"))
            .expect("read workspace crates")
            .flatten()
            .map(|entry| entry.path().join("Cargo.toml"))
            .filter(|path| path.is_file()),
    );
    manifests
        .into_iter()
        .map(|path| {
            let manifest = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            manifest
                .lines()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix("name = \"")
                        .and_then(|name| name.strip_suffix('"'))
                })
                .unwrap_or_else(|| panic!("{} has no package name", path.display()))
                .to_string()
        })
        .collect()
}

/// The cells of a markdown pipe-table row, trimmed, with the empty strings that
/// the leading and trailing `|` produce dropped.
///
/// Returns `None` for anything that is not a table row, including the `|---|`
/// separator, so a caller can filter a whole file through it.
fn cells(line: &str) -> Option<Vec<String>> {
    let line = line.trim();
    if !line.starts_with('|') {
        return None;
    }
    let cells: Vec<String> = line
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim().to_string())
        .collect();
    if cells
        .iter()
        .all(|c| c.chars().all(|ch| ch == '-' || ch == ':') && !c.is_empty())
    {
        return None; // the separator row
    }
    Some(cells)
}

/// The rule code a table row is about, if its first cell is one.
///
/// Matches `` | `APLANnnn` | `` and nothing else, so a row merely *mentioning* a
/// code in its prose is not mistaken for that rule's own row. That distinction is
/// the whole point: mentioning was the old gate.
fn row_code(cells: &[String]) -> Option<String> {
    let first = cells.first()?.trim_matches('`');
    let rest = first.strip_prefix("APLAN")?;
    (rest.len() == 3 && rest.chars().all(|c| c.is_ascii_digit())).then(|| first.to_string())
}

/// A severity cell as the docs write it (`error`, `warning`, `**warning**`).
fn severity_of(cell: &str) -> String {
    cell.trim_matches('*').trim().to_lowercase()
}

/// The lines of a document that are prose, with fenced code blocks removed.
///
/// A gate that reads tables must not read a table that is being *shown*. The
/// README's `jq` recipes already contain lines beginning with `|`, and they parse
/// as rows today only because no cell of theirs happens to look like a rule code
/// or a command. Sample `aplan check` output, or a doc illustrating this very
/// table format, would break the gate for reasons that have nothing to do with
/// the docs being wrong.
fn prose_lines(doc: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            out.push(line);
        }
    }
    out
}

/// Every table row in a document that is not inside a code fence.
fn table_rows(doc: &str) -> Vec<Vec<String>> {
    prose_lines(doc).into_iter().filter_map(cells).collect()
}

/// Every rule row in a document, as `(code, cells)`, in file order. Duplicates
/// are preserved, because finding them is one of this gate's jobs.
fn rule_rows(doc: &str) -> Vec<(String, Vec<String>)> {
    table_rows(doc)
        .into_iter()
        .filter_map(|c| row_code(&c).map(|code| (code, c)))
        .collect()
}

/// Whether a row describes something that does not exist yet.
///
/// Read from the **status column** (the last cell) alone. Scanning every cell
/// would let a rule whose description merely used the word "planned" pass as
/// unbuilt, which is the escape hatch pointing the wrong way: a row for a removed
/// rule would then read as legitimately unshipped.
fn is_planned(cells: &[String]) -> bool {
    cells.last().is_some_and(|c| {
        c.trim_matches('*')
            .trim()
            .to_lowercase()
            .starts_with("planned")
    })
}

#[test]
fn every_rule_appears_exactly_once_in_the_hld_catalog() {
    // The old gate asked `hld.contains(code)`, which is true for one row and for
    // two. `APLAN027` was duplicated in the catalog and nothing noticed.
    let rows = rule_rows(&repo_file("HLD.md"));
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (code, _) in &rows {
        *seen.entry(code.clone()).or_default() += 1;
    }

    let mut problems = Vec::new();
    for (code, _) in rule_severities() {
        match seen.get(&code).copied().unwrap_or(0) {
            1 => {}
            0 => problems.push(format!("{code}: no row in the HLD.md rule catalog")),
            n => problems.push(format!("{code}: {n} rows in HLD.md, expected 1")),
        }
    }

    assert!(
        problems.is_empty(),
        "HLD.md's rule catalog disagrees with `aplan facts`:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn the_hld_records_each_rules_severity_as_the_code_declares_it() {
    // Severity is declared once, on the `define_rules!` row. A table restating it
    // is a second copy, and this is what binds the two.
    let declared = rule_severities();
    let mut problems = Vec::new();

    for (code, cells) in rule_rows(&repo_file("HLD.md")) {
        let Some(want) = declared.get(&code) else {
            continue; // a planned rule; the next test governs those
        };
        // Column 3 in both catalog tables: code, invariant, severity, ...
        let Some(got) = cells.get(2).map(|c| severity_of(c)) else {
            problems.push(format!("{code}: row has no severity column"));
            continue;
        };
        if &got != want {
            problems.push(format!(
                "{code}: HLD.md says `{got}`, the code says `{want}`"
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "HLD.md records a severity the code does not declare:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn a_rule_row_for_a_rule_that_does_not_exist_says_planned() {
    // `APLAN016` (url reachability) is documented
    // and unbuilt, which is legitimate and must stay possible. What must not be
    // possible is a row for a rule that was removed or renamed reading exactly
    // like a rule that ships.
    let declared = rule_severities();
    let mut problems = Vec::new();

    for (code, cells) in rule_rows(&repo_file("HLD.md")) {
        if !declared.contains_key(&code) && !is_planned(&cells) {
            problems.push(format!(
                "{code}: documented in HLD.md, absent from `aplan facts`, and not marked `planned`"
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "HLD.md documents a rule that does not exist:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn every_rule_is_mentioned_in_the_readme() {
    // Deliberately weaker than the HLD check, and deliberately present at all.
    // The README's rule table is the *always-on* set: the three opt-in rules
    // (`APLAN017`, `APLAN018`, `APLAN022`) are covered in their own paragraph
    // below it, and the evidence-ladder rules in the section after that. That is
    // editorial structure worth keeping, so this asks only that a reader can find
    // every rule somewhere. Before `A1-092` the README was not consulted for a
    // rule at all.
    let readme = repo_file("README.md");
    let missing: Vec<String> = rule_severities()
        .into_keys()
        .filter(|code| !readme.contains(code.as_str()))
        .collect();

    assert!(
        missing.is_empty(),
        "rules `aplan facts` knows that README.md never mentions:\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn the_readme_table_marks_exactly_the_warnings_the_code_declares() {
    // The README table annotates warnings inline, e.g. *(warning, `verify`)*.
    // That is a hand-maintained copy of a fact the code owns: the prose above the
    // table named three warnings while the table itself marked seven.
    let declared = rule_severities();
    let mut problems = Vec::new();

    for (code, cells) in rule_rows(&repo_file("README.md")) {
        let Some(want) = declared.get(&code) else {
            problems.push(format!(
                "{code}: in the README table, absent from `aplan facts`"
            ));
            continue;
        };
        let text = cells.get(1).map(String::as_str).unwrap_or_default();
        let marked = if text.to_lowercase().contains("warning") {
            "warning"
        } else {
            "error"
        };
        if marked != want {
            problems.push(format!(
                "{code}: the README table reads as `{marked}`, the code declares `{want}`"
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "README.md's rule table disagrees with `aplan facts`:\n  {}",
        problems.join("\n  ")
    );
}

/// Every `N of ... M rules are warnings` claim in a document's prose, as `(N, M)`.
///
/// Hand-parsed rather than matched with a pattern, because this crate has no
/// regex dependency and one sentence shape does not earn one. The shape is read
/// from its two anchors rather than from a character window: `M` is the number
/// immediately before the phrase, and `N` the number immediately before the
/// ` of ` that introduces it. That reads every wording used -- *17 of this
/// tool's 40 rules are warnings*, *17 of aplan's 40*, *17 of the 40* -- and
/// survives the line break prose wrapping puts between the two numbers, which a
/// sentence-window version did not.
fn warning_counts(text: &str) -> Vec<(usize, usize)> {
    const PHRASE: &str = "rules are warnings";
    /// How far back ` of ` may be. Long enough for a possessive, short enough
    /// that an unrelated `of` in the sentence before is out of reach.
    const LOOKBACK: usize = 40;

    /// The digits ending at `end`, and where they start.
    fn number_before(text: &str, end: usize) -> Option<(usize, usize)> {
        let head = text[..end].trim_end();
        let start = head.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        if start == head.len() {
            return None;
        }
        head[start..].parse().ok().map(|n| (n, start))
    }

    // Prose only, like every other reader in this file: a document *showing* this
    // sentence in a fenced example is quoting it, not claiming it.
    let text = &prose_lines(text).join("\n");
    let mut out = Vec::new();
    for (at, _) in text.match_indices(PHRASE) {
        let Some((total, m_start)) = number_before(text, at) else {
            continue;
        };
        let from = m_start.saturating_sub(LOOKBACK);
        let Some(rel) = text[from..m_start].rfind(" of ") else {
            continue;
        };
        if let Some((warnings, _)) = number_before(text, from + rel) {
            out.push((warnings, total));
        }
    }
    out
}

/// Every document that counts the warning tier counts it correctly (`A1-272`).
///
/// The CI guidance persuades an adopter with a number -- *17 of this tool's 40
/// rules are warnings* -- and a number written into prose is a copy of a fact the
/// code owns. This file exists because one of those copies had already rotted
/// once: the README's prose said three warnings above a table marking seven.
/// `APLAN040` moved the total from 39 to 40 the day before this was written.
///
/// Two assertions, because either alone is half a gate. Every claim found must be
/// right, and the four documents that make the argument must each still make it:
/// a gate that only checks claims it finds passes a document that dropped the
/// sentence, which is the failure mode that lets guidance quietly disappear.
#[test]
fn every_document_counting_the_warning_tier_counts_it_correctly() {
    let declared = rule_severities();
    let total = declared.len();
    let warnings = declared.values().filter(|s| *s == "warning").count();

    // Where the argument is made, and so where it has to keep being made. Not
    // `CHANGELOG.md`, deliberately: it records what a release said, so a living
    // number in it would have to either rot or be rewritten after the fact, and
    // this gate would be demanding the second (`A1-272`'s second audit).
    let documents = ["README.md", "action.yml", "ci/README.md"];
    let mut problems = Vec::new();

    for name in documents {
        let found = warning_counts(&repo_file(name));
        if found.is_empty() {
            problems.push(format!(
                "{name}: states no warning-tier count, and the CI guidance rests on one"
            ));
            continue;
        }
        for (said_w, said_t) in found {
            if (said_w, said_t) != (warnings, total) {
                problems.push(format!(
                    "{name}: says {said_w} of {said_t}, the code declares {warnings} of {total}"
                ));
            }
        }
    }

    assert!(
        problems.is_empty(),
        "a documented count disagrees with `aplan facts`:\n  {}",
        problems.join("\n  ")
    );
}

/// A document *showing* the sentence is quoting it, not claiming it.
///
/// The same property `a_fenced_heading_is_not_a_heading` pins for headings: this
/// file's readers all work on prose, and a gate that read fences would make
/// documenting its own format impossible.
#[test]
fn a_fenced_count_is_not_a_claim() {
    let fenced = "before\n\n```text\n3 of 9 rules are warnings\n```\n\nafter\n";
    assert!(
        warning_counts(fenced).is_empty(),
        "read a quoted example as a claim"
    );
    // And the same words outside a fence are read, so the exclusion above is a
    // fence rule rather than a gate that sees nothing.
    assert_eq!(
        warning_counts("before\n\n3 of 9 rules are warnings\n\nafter\n"),
        vec![(3, 9)]
    );
}

/// Whether a status cell claims the command ships today.
///
/// The cell must *begin* with `exists` or `partial`, not merely contain the word.
/// The row `` | `aplan migrate` | schema version upgrades | when `v2` exists | ``
/// is a condition, not a claim, and a `contains` test reads it as one: that is
/// exactly the "computed X, asserted Y" shape `A1-091` is about, caught here by
/// its own gate on the first run.
fn claims_to_exist(status: &str) -> bool {
    let s = status.trim_matches('*').trim().to_lowercase();
    s.starts_with("exists") || s.starts_with("partial")
}

/// The command an HLD CLI row is about: the first word after `aplan` in its first
/// cell, so `` `aplan why <id>` `` is the `why` row.
fn cli_row_command(cells: &[String]) -> Option<String> {
    let first = cells.first()?.trim_matches('`');
    let rest = first.strip_prefix("aplan ")?;
    let name = rest.split_whitespace().next()?;
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c == '-')
        .then(|| name.to_string())
}

/// The body of the README's `## Commands` block: the curated list a reader scans
/// to find out what the tool can do.
fn readme_command_list(readme: &str) -> String {
    let after = readme
        .split_once("\n## Commands\n")
        .expect("README.md has a `## Commands` section")
        .1;
    let mut parts = after.split("```");
    parts.next();
    parts
        .next()
        .expect("the `## Commands` section opens a fenced block")
        .to_string()
}

/// Whether the command list names `aplan <name>` as a command, rather than as
/// the prefix of a longer one.
///
/// A plain `contains` reads `aplan export-roadmap-json` as satisfying `export`,
/// so the day `export` was added it was already documented as far as this gate
/// could tell (`A1-189`). The next character has to end the word.
fn lists_command(list: &str, name: &str) -> bool {
    let needle = format!("aplan {name}");
    list.match_indices(&needle).any(|(at, _)| {
        list[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '-')
    })
}

#[test]
fn every_command_is_in_the_readme_command_list() {
    // This asked `readme.contains("aplan <name>")` until the audit of `A1-092`,
    // which is the same anywhere-in-the-file test that made the *rule* half of
    // this gate useless. It passed for every command, and three of them
    // (`drop`, `mv`, `tasks`) were absent from the command list entirely,
    // satisfying it through incidental mentions in prose.
    let list = readme_command_list(&repo_file("README.md"));
    let missing: Vec<String> = command_names()
        .into_iter()
        .filter(|name| !lists_command(&list, name))
        .collect();

    assert!(
        missing.is_empty(),
        "commands missing from README.md's `## Commands` list:\n  {}",
        missing.join("\n  ")
    );
}

/// The blueprint guidance is one product boundary on five reader surfaces
/// (`A1-393`), not five opportunities to widen it independently.
///
/// This checks stable facts and names rather than whole sentences: the
/// generated agent block must tell an author how to preview, the adopter guide
/// must retain the parameter/reference and trust contracts, the HLD must retain
/// the evidence limit and staged exclusions, and the changelog must publish the
/// require-able behavior. The README's separate command census above proves the
/// command remains discoverable and exhaustive.
#[test]
fn blueprint_guidance_agrees_on_scope_trust_and_evidence_limits() {
    let readme = repo_file("README.md");
    let upgrading = repo_file("docs/upgrading.md");
    let hld = repo_file("HLD.md");
    let changelog = repo_file("CHANGELOG.md");
    let agents = repo_file("AGENTS.md");
    let flatten = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let upgrading_flat = flatten(&upgrading);
    let hld_flat = flatten(&hld);
    let agents_flat = flatten(&agents);

    for (name, text, command) in [
        ("README.md", readme.as_str(), "aplan blueprint apply"),
        (
            "docs/upgrading.md",
            upgrading.as_str(),
            "aplan blueprint apply",
        ),
        ("HLD.md", hld.as_str(), "`aplan blueprint`"),
        ("CHANGELOG.md", changelog.as_str(), "aplan blueprint apply"),
        ("AGENTS.md", agents.as_str(), "aplan blueprint apply"),
    ] {
        assert!(
            text.contains(command),
            "{name} no longer names the shipped command"
        );
    }

    for fact in [
        "--set NAME=VALUE",
        "local:<key>",
        "task:<existing-id>",
        "non-executable plan-authoring data",
        "SHA-256",
        "no discovery, catalogue, composition, replay, built-ins, installation or fetch path",
    ] {
        assert!(
            upgrading_flat.contains(fact),
            "the adopter guide lost blueprint fact `{fact}`"
        );
    }

    for fact in [
        "no adjacent open-source request",
        "first-party history establishes recurring one-task conventions",
        "neither proves one universal template or market pull",
        "discovery, catalogues, composition, replay, built-ins, installation and fetching",
    ] {
        assert!(
            hld_flat.contains(fact),
            "the HLD lost evidence limit `{fact}`"
        );
    }

    let behavior = "agent_contract.guides_explicit_blueprint_application";
    assert!(
        changelog.contains(behavior) && upgrading.contains(behavior),
        "the shipped generated-contract change is not stageable by `{behavior}`"
    );
    for fact in [
        "task fields or sibling topology recur",
        "--file FILE_PATH --cut CUT_ID",
        "source path and digest",
        "without `--dry-run`",
    ] {
        assert!(
            agents_flat.contains(fact),
            "the generated agent contract lost `{fact}`"
        );
    }
}

/// Every `aplan ...` an adopter is told to run, in the document they read while
/// staging a binary.
///
/// `every_documented_invocation_actually_parses` covers the README's curated
/// command list, and the sweep over the tool's own strings covers its messages.
/// `docs/upgrading.md` carries 58 invocations and was covered by neither, which
/// this author found by adding a 59th and having to run it by hand to know it
/// worked (`A1-160`'s audit). It is the worst file to leave ungated: a reader is
/// there precisely because they are about to change something.
/// Every numbered finding in a pilot log names the work it became (`A1-166`).
///
/// pre-public adoption evidence states the one rule those logs have -- a friction
/// item that becomes work names the task id that owns it -- and nothing checked
/// it. `check-docs` asks whether an id a document names resolves to a task; it
/// cannot ask whether a finding named one at all, which is the direction that
/// rots. Two had not: `p00020` item 24, labelled "blocker for this consumer",
/// which `A1-138` fixed and which said nothing about it, and `p0000004` item 4,
/// waiting on `A1-117`.
///
/// A finding is a heading beginning with a number, which is the shape all seven
/// logs use. A narrative section ("What the fourth pass confirmed working") is
/// not one and is not asked for a task. The window runs to the next *heading of
/// any level*, because a shared `**Disposition.**` paragraph covering several
/// consecutive findings is house style (`p00020` items 15-19) and the last of
/// them has to be able to reach it.
#[test]
fn every_pilot_finding_names_the_work_it_became() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !root.join("docs/pilots").is_dir() {
        assert_public_plan_epoch();
        return;
    }
    let mut problems = Vec::new();
    let mut checked = 0usize;

    let mut logs: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("docs/pilots"))
        .expect("docs/pilots")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter(|p| p.file_name().is_some_and(|n| n != "README.md"))
        .collect();
    logs.sort();
    assert!(logs.len() >= 5, "only {} log(s) found", logs.len());

    for log in &logs {
        let text = std::fs::read_to_string(log).expect("read");
        let lines: Vec<&str> = text.lines().collect();
        // A `#` inside a fenced block is not a heading, and these logs quote
        // markdown constantly -- `p0000014` item 2 shows the scaffold `add`
        // writes, whose first line is `## Why`. Scanning raw lines closed that
        // finding's window twelve lines in and made every disposition after it
        // invisible, so the item could not be answered while it quoted the thing
        // it was about (`A1-268`). `unfenced_lines` is the one fence reader in
        // this repository and exists for exactly this (`A1-165`, `A1-224`).
        let headings = heading_lines(&text);
        let unfenced: std::collections::HashSet<usize> = aplan_core::anchor::unfenced_lines(&text)
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        let heading_at = |i: usize| headings.contains(&i);
        let findings: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| is_finding(l))
            .map(|(i, _)| i)
            .collect();
        // A heading that opens with a digit and does not parse as a finding is a
        // style this gate does not recognise, and a whole log written that way
        // would be swept by nothing while the total stayed comfortably above the
        // floor below. That is the partial-vacuity shape (`A1-114`), here in the
        // gate itself.
        for (i, line) in lines.iter().enumerate() {
            if !unfenced.contains(&i) {
                continue;
            }
            let opens_with_digit = line
                .strip_prefix("### ")
                .or_else(|| line.strip_prefix("#### "))
                .and_then(|r| r.chars().next())
                .is_some_and(|c| c.is_ascii_digit());
            if opens_with_digit && !is_finding(line) {
                problems.push(format!(
                    "{}: {} opens with a number but is not `### N. ...`, so this gate does not \
                     see it as a finding",
                    log.file_name().unwrap_or_default().to_string_lossy(),
                    line.trim()
                ));
            }
        }
        for (n, &start) in findings.iter().enumerate() {
            let number = finding_number(lines[start]).unwrap_or_default();
            let section_end = lines
                .iter()
                .enumerate()
                .position(|(i, l)| i > start && heading_at(i) && l.starts_with("## "))
                .unwrap_or(lines.len());
            // A finding's own text ends at the *next heading of any level*, not
            // at the next numbered finding: these logs put narrative
            // sub-headings between findings ("What the fourth pass confirmed
            // working"), and a window that ran past one swallowed a hundred
            // lines of prose about something else. Removing item 24's real
            // disposition still left this gate green until that was fixed
            // (`A1-166`'s own audit).
            let next = lines
                .iter()
                .enumerate()
                .position(|(i, _)| i > start && heading_at(i))
                .unwrap_or(lines.len());
            let _ = n;
            let own = lines[start..next].join("\n");
            checked += 1;
            if dispositioned(&own) {
                continue;
            }
            // A shared `**Disposition.**` paragraph covering several
            // consecutive findings is house style (`p00020` items 15-19), and it
            // names the items it answers: "Item 15 is tracked as `A1-099`, item
            // 16 as `A1-098`". So a finding may be answered further down its own
            // section -- but only where that paragraph says its number, which is
            // what stops one finding borrowing its neighbour's disposition.
            if section_end > next && answers_item(&lines[next..section_end].join("\n"), &number) {
                continue;
            }
            problems.push(format!(
                "{}: {}",
                log.file_name().unwrap_or_default().to_string_lossy(),
                lines[start].trim()
            ));
        }
    }

    assert!(
        checked >= 40,
        "the sweep found {checked} finding(s), so it has stopped matching and this passes \
         vacuously -- which is the shape `A1-114` is about, in this repository's own prose"
    );
    assert!(
        problems.is_empty(),
        "a pilot finding names neither a task nor a disposition, so a reader cannot tell whether \
         it is open:\n  {}",
        problems.join("\n  ")
    );
}

/// The number a finding's heading opens with.
/// The line numbers of `text` that are headings, ignoring fenced code.
///
/// A `#` inside a fence is a quoted sample, not a heading, and these logs quote
/// markdown constantly. Reading it as a heading closes a finding's window early
/// and hides every disposition written after it (`A1-268`). One fence reader,
/// shared, is the rule `anchor::unfenced_lines` exists to keep (`A1-165`).
fn heading_lines(text: &str) -> std::collections::HashSet<usize> {
    aplan_core::anchor::unfenced_lines(text)
        .into_iter()
        .filter(|(_, l)| l.starts_with('#'))
        .map(|(i, _)| i)
        .collect()
}

/// **A heading inside a fence does not end a finding.**
///
/// The bug `A1-268` fixed, pinned in both directions: `p0000014` item 2 quotes
/// the scaffold `aplan add` writes, whose first line is `## Why`, and the gate
/// closed its window there and could never see the disposition below.
#[test]
fn a_fenced_heading_is_not_a_heading() {
    let quoted = "### 1. A finding\n\n```markdown\n## Why\n```\n\n**Disposition.** Filed.\n";
    let headings = heading_lines(quoted);
    assert_eq!(
        headings,
        std::collections::HashSet::from([0]),
        "only the finding's own heading, not the one it quotes: {quoted}"
    );

    // The same heading unfenced does end it, or the fix would have blinded the
    // gate rather than corrected it (`A1-166`'s audit is what that would break).
    let bare = "### 1. A finding\n\n## Why\n\n**Disposition.** Filed.\n";
    assert_eq!(
        heading_lines(bare),
        std::collections::HashSet::from([0, 2]),
        "an unfenced sub-heading still closes the window: {bare}"
    );
}

fn finding_number(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("### ")
        .or_else(|| line.strip_prefix("#### "))?;
    let (n, _) = rest.split_once('.')?;
    (!n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).then(|| n.to_string())
}

/// Whether a later paragraph answers *this* finding: it names the item number
/// and carries a task id.
fn answers_item(text: &str, number: &str) -> bool {
    if number.is_empty() {
        return false;
    }
    let needle = format!("item {number}");
    let lower = text.to_lowercase();
    lower.match_indices(&needle).any(|(i, _)| {
        // The number itself, not the start of a longer one: "item 1" must not
        // answer a paragraph about item 15.
        let after = lower[i + needle.len()..].chars().next();
        // The marker, not the bare-id path: a later section may discuss a
        // finding at length -- `p00020` has a "Correction to item 24" -- without
        // saying what became of it, and an id mentioned in passing there is not
        // a disposition (`A1-166`'s own audit).
        after.is_none_or(|c| !c.is_ascii_digit()) && text.contains("**Disposition")
    })
}

/// A heading that opens a numbered finding: `### 12. ...`.
fn is_finding(line: &str) -> bool {
    let Some(rest) = line
        .strip_prefix("### ")
        .or_else(|| line.strip_prefix("#### "))
    else {
        return false;
    };
    // `### N. ...`, the numbered form.
    if rest
        .split_once('.')
        .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    {
        return true;
    }
    // `### 2026-09-27 -- ...`, the dated form. `p000000000018` writes nearly
    // every entry this way and `p00002` writes some, and this gate used to
    // report each of them as "opens with a number but is not `### N. ...`" --
    // the partial-vacuity guard firing on a convention the gate did not know
    // rather than on a real gap (`A1-284`). Recognising it *adds* coverage:
    // these entries now have to carry a disposition like any other.
    dated(rest)
}

/// Whether a heading opens with an ISO date, which is the second entry form
/// these logs use.
fn dated(rest: &str) -> bool {
    let bytes = rest.as_bytes();
    bytes.len() >= 10
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

/// Whether a finding says what became of it.
///
/// Two spellings, because two are what the seven logs actually use:
/// `**Disposition.**` or `**Disposition: ...**` (`p00020`, `p003`, `p00013`,
/// `p0000019`, `p0000000000005`), and the task id named inside the finding's own
/// prose (`p0000004`). The gate describes the convention rather than imposing a
/// third on seven append-only files.
///
/// A heading tagged `(no action)` is *not* a further spelling. `p00013` item 4 is
/// the only one, and it carries a marked disposition too -- so accepting the tag
/// would be a branch no finding needs, and a phrase a body could fall into by
/// accident (`A1-166`'s audit).
fn dispositioned(window: &str) -> bool {
    window.contains("**Disposition")
        // The same paragraph without the bold markers, which is how
        // `p000000000018` writes every one of them: `Disposition: not a defect
        // -- ...`. Matched at a line start, so the word used in prose is not a
        // decision.
        || window
            .lines()
            .any(|l| l.trim_start().starts_with("Disposition:"))
        || task_id_in(window)
        // The other two forms the README documents, written as it documents
        // them rather than wrapped in a `**Disposition` paragraph. Every use of
        // "Fixed in" until `p00002` happened to sit inside one, so the gate
        // implemented one of three spellings and nobody could tell -- until a
        // ninth log wrote the third exactly as it is specified (`A1-234`,
        // audit).
        //
        // Kept tight on purpose: the backtick after "Fixed in" is what stops
        // prose like "fixed in the next release" from reading as a decision, and
        // "not worth fixing" is the README's own phrase rather than a paraphrase
        // a body could fall into.
        || window.contains("Fixed in `")
        || window.contains("not worth fixing")
        // Two more the corpus writes, found when `p0000021` and `p00023` landed
        // and this gate failed on ten entries that were all decided (`A1-284`,
        // which is about exactly this: a vocabulary narrower than the logs).
        //
        // `**Became work:` is how a log records that the *pilot repository*
        // took the work, naming its own task (`SX-063`, `MN-142`), which
        // `task_id_in` cannot see because it looks for this repository's ids.
        // It is the "Fixed in `<project>`" disposition under the spelling five
        // logs actually use.
        //
        // `Not a defect` is the README's second disposition without the
        // `**Disposition` wrapper. Kept as the bold lead-in the corpus uses, so
        // prose that merely argues a thing is not a defect does not read as a
        // decision.
        || window.contains("**Became work:")
        || window.contains("**Not a defect")
}

/// The predicate itself, over the three forms the README documents and the
/// prose they must not be confused with.
///
/// The logs cannot test this: none of them contains a body that mentions fixing
/// without also carrying a disposition, so dropping the backtick the phrase
/// requires passes the sweep unchanged. That is the tightness claim going
/// untested, which is the shape this repository keeps finding (`A1-234`, audit).
#[test]
fn a_disposition_is_one_of_the_three_forms_and_not_prose_about_them() {
    for accepted in [
        "**Disposition: tracked as `A1-099`.**",
        "The adjudication: aplan's defect, filed as `A1-159`.",
        "**Fixed in `p00002`:** `prototype/checks.mjs` lists five names.",
        "This is not worth fixing, because the cost lands on one adopter.",
    ] {
        assert!(
            dispositioned(accepted),
            "rejected a documented form: {accepted}"
        );
    }
    for rejected in [
        "The bug is fixed in the next release, so this log records it only.",
        // Capitalised and unbacktick: the project name is what makes the phrase
        // a decision rather than a sentence about one.
        "Fixed in the next release of the prototype, which this log does not track.",
        "Fixing this would mean a second parser, which is why the item is here.",
        "It became worth fixing once the fourth pilot hit it as well.",
        "A citation naming AM-008 is p00002's own id, not a task in this plan.",
    ] {
        assert!(
            !dispositioned(rejected),
            "prose read as a decision: {rejected}"
        );
    }
}

/// An id in this plan's own dialect, which `check-docs` then proves resolves.
///
/// Two logs name the task inside the finding's own prose rather than in a
/// marked paragraph (`p0000004`, `p0000000000005`), and both shapes have to
/// pass. Safe only because the window above is the finding's own text: over a
/// whole section, "an id appears somewhere below" is satisfied by prose about
/// something else.
fn task_id_in(text: &str) -> bool {
    text.as_bytes().windows(6).any(|w| {
        w[0] == b'A'
            && w[1].is_ascii_digit()
            && w[2] == b'-'
            && w[3..].iter().all(u8::is_ascii_digit)
    })
}

#[test]
fn every_invocation_the_upgrade_guide_names_actually_parses() {
    let text = repo_file("docs/upgrading.md");
    let mut problems = Vec::new();
    let mut checked = 0usize;

    for span in commands_named_in(&text) {
        if !span.starts_with("aplan ") {
            continue;
        }
        // A shell *line* is not an invocation to parse: `aplan --version | grep`
        // documents a pipeline, and its second half is not this CLI's business.
        //
        // Tokens, not `contains`. The first version skipped any span holding a
        // `>` and so skipped every `<placeholder>` in the file -- which is most
        // of them, and exactly the spans this was written to check. Its own
        // vacuity guard passed, because 48 spans were still being read
        // (`A1-160`'s audit of this test).
        if span
            .split_whitespace()
            .any(|w| matches!(w, "|" | ">" | ">>" | "&&" | "||" | ";"))
        {
            continue;
        }
        checked += 1;
        // An invocation that names an evidence citation has to be one a reader
        // can paste: everything but the value is filled in, so it reads as a
        // complete command. `aplan tree <id>` is a *form* -- the reader is
        // choosing the subject -- and keeps its brackets, which is the line
        // `A1-156` drew and the same one its code-side sweep draws.
        let cites = ["commit:", "test:", "file:", "url:", "note:"]
            .iter()
            .any(|k| span.contains(k));
        let shell = std::process::Command::new("bash")
            .args(["-n", "-c", &span])
            .output()
            .expect("bash runs");
        if cites && !shell.status.success() {
            problems.push(format!(
                "`{span}`\n      not shell: {}",
                String::from_utf8_lossy(&shell.stderr)
                    .lines()
                    .next_back()
                    .unwrap_or_default()
                    .trim()
            ));
            continue;
        }
        let argv = common::argv_of(&span);
        if let Err(e) = aplan::cli_command().try_get_matches_from(&argv) {
            // The same reading the message sweep takes: whether the command and
            // its flags *exist*, not whether the line is runnable as written. An
            // upgrade note naming `aplan close --restamp` is naming a form, and
            // demanding a complete invocation would force every row to invent an
            // id.
            use clap::error::ErrorKind::{InvalidSubcommand, UnknownArgument, ValueValidation};
            if matches!(
                e.kind(),
                UnknownArgument | InvalidSubcommand | ValueValidation
            ) {
                let first = e.to_string().lines().next().unwrap_or_default().to_string();
                problems.push(format!("`{span}`\n      {first}"));
            }
        }
    }

    assert!(
        checked >= 50,
        "the sweep found {checked} invocations, so it has stopped matching and this passes \
         vacuously"
    );
    assert!(
        problems.is_empty(),
        "docs/upgrading.md names an invocation the CLI rejects:\n  {}",
        problems.join("\n  ")
    );
}

/// Every command a markdown document names: the inline backticked spans, plus
/// the lines inside its fenced blocks.
///
/// Fences first, and this is the whole subtlety. The scanner that reads the
/// tool's own Rust strings pairs backticks left to right, which is right there
/// and wrong here: a markdown fence is three of them, so pairing across one shifts
/// every span after it and the sweep silently finds almost nothing. Its own
/// vacuity guard caught that on the first run (`A1-160`'s audit), which is the
/// argument for having one.
fn commands_named_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut prose = String::new();
    let mut inside = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            inside = !inside;
            continue;
        }
        if inside {
            // A fenced line is a command as written, not a span inside prose.
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("$ ") {
                out.push(rest.to_string());
            } else {
                out.push(line.to_string());
            }
        } else {
            prose.push_str(line);
            prose.push('\n');
        }
    }
    let mut rest = prose.as_str();
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        match after.find('`') {
            Some(close) => {
                // Collapsed, because markdown collapses it: a backticked span
                // may wrap across a line in the source and still render as one
                // command, and the reader copies what they see.
                out.push(
                    after[..close]
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                rest = &after[close + 1..];
            }
            None => break,
        }
    }
    out
}

#[test]
fn every_documented_invocation_actually_parses() {
    // The README's command list reads as copy-pasteable, so it has to be. Written
    // during the audit of `A1-092`, after two of the three lines added moments
    // earlier were wrong: `aplan mv P1-003 --to M-v0.7` names a flag that does not
    // exist (it is `--cut`), and `drop --cut` is a boolean rather than a flag
    // taking a cut id. A reader who copies advice that fails at the first attempt
    // has no way to tell which half of the documentation to trust after that.
    //
    // Parsed, never executed: this proves the argv is well-formed, not that the
    // ids in the examples exist.
    let mut problems = Vec::new();

    for line in readme_command_list(&repo_file("README.md")).lines() {
        let line = line.trim();
        if !line.starts_with("aplan ") {
            continue;
        }
        let argv = common::argv_of(line);
        if let Err(e) = aplan::cli_command().try_get_matches_from(&argv) {
            let first = e.to_string().lines().next().unwrap_or_default().to_string();
            problems.push(format!("{line}\n      {first}"));
        }
    }

    assert!(
        problems.is_empty(),
        "README.md documents an invocation the CLI rejects:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn every_command_has_exactly_one_row_in_the_hld_cli_table() {
    // A set was the wrong audit structure: it made the shipped and planned rows
    // for both `attest` and `export-agents-md` look exactly like one correct row.
    // Keep the multiplicity beside the name, as the rule census above does.
    let mut documented: BTreeMap<String, usize> = BTreeMap::new();
    for cells in table_rows(hld_section(&repo_file("HLD.md"), "8. CLI surface")) {
        if let Some(name) = cli_row_command(&cells) {
            *documented.entry(name).or_default() += 1;
        }
    }
    let mut problems = Vec::new();
    for name in command_names() {
        match documented.get(&name).copied().unwrap_or(0) {
            1 => {}
            0 => problems.push(format!("`aplan {name}`: no row")),
            n => problems.push(format!("`aplan {name}`: {n} rows, expected 1")),
        }
    }

    assert!(
        problems.is_empty(),
        "HLD.md's CLI surface table is not one row per command:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn every_workspace_crate_has_exactly_one_existing_hld_row() {
    let hld = repo_file("HLD.md");
    let rows = table_rows(hld_section(&hld, "7. Crate architecture"));
    let mut documented: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
    for cells in rows {
        if let Some(name) = first_code_cell(&cells)
            && (name == "xtask" || name == "aplan" || name.starts_with("aplan-"))
        {
            documented.entry(name).or_default().push(cells);
        }
    }

    let actual = workspace_crates();
    let mut problems = Vec::new();
    for name in &actual {
        match documented.get(name).map(Vec::as_slice).unwrap_or_default() {
            [row] if row.last().is_some_and(|status| claims_to_exist(status)) => {}
            [row] => problems.push(format!(
                "`{name}` exists in the workspace but HLD.md says `{}`",
                row.last().map_or("no status", String::as_str)
            )),
            [] => problems.push(format!("`{name}`: no HLD crate row")),
            rows => problems.push(format!("`{name}`: {} HLD crate rows", rows.len())),
        }
    }
    for (name, rows) in &documented {
        if !actual.contains(name)
            && rows
                .iter()
                .any(|row| row.last().is_some_and(|status| claims_to_exist(status)))
        {
            problems.push(format!(
                "`{name}` is absent from the workspace but its HLD row claims it exists"
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "HLD.md's crate table disagrees with the workspace:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn every_output_format_has_exactly_one_existing_hld_row() {
    let hld = repo_file("HLD.md");
    let rows = table_rows(hld_section(&hld, "10. Output"));
    let mut documented: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
    for cells in rows {
        if let Some(name) = first_code_cell(&cells) {
            documented.entry(name).or_default().push(cells);
        }
    }

    let actual = format_names();
    let mut problems = Vec::new();
    for name in &actual {
        match documented.get(name).map(Vec::as_slice).unwrap_or_default() {
            [row] if row.last().is_some_and(|status| claims_to_exist(status)) => {}
            [row] => problems.push(format!(
                "`{name}` exists in `aplan facts` but HLD.md says `{}`",
                row.last().map_or("no status", String::as_str)
            )),
            [] => problems.push(format!("`{name}`: no HLD output row")),
            rows => problems.push(format!("`{name}`: {} HLD output rows", rows.len())),
        }
    }
    for (name, rows) in &documented {
        if !actual.contains(name)
            && rows
                .iter()
                .any(|row| row.last().is_some_and(|status| claims_to_exist(status)))
        {
            problems.push(format!(
                "`{name}` is absent from `aplan facts` but its HLD row claims it exists"
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "HLD.md's output table disagrees with `aplan facts`:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn an_hld_cli_row_marked_exists_names_a_command_that_exists() {
    // The other direction, which nothing checked. A command removed or renamed
    // left its row behind claiming to exist, and the old gate only ever asked
    // whether real commands were documented, never whether documented ones were
    // real.
    let known = command_names();
    let mut problems = Vec::new();

    for cells in table_rows(&repo_file("HLD.md")) {
        let Some(name) = cli_row_command(&cells) else {
            continue;
        };
        let status = cells.last().map(String::as_str).unwrap_or_default();
        if claims_to_exist(status) && !known.contains(&name) {
            problems.push(format!(
                "`aplan {name}`: the HLD CLI table says it exists, `aplan facts` does not know it"
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "HLD.md's CLI surface table claims a command that does not exist:\n  {}",
        problems.join("\n  ")
    );
}

#[cfg(test)]
mod parsing {
    use super::*;

    #[test]
    fn a_longer_command_does_not_document_a_shorter_one() {
        // The defect this helper replaced: `export` counted as listed because
        // `export-roadmap-json` is in the block.
        let list = "aplan export-roadmap-json roadmap.json  # a foreign contract\n";
        assert!(!lists_command(list, "export"));
        assert!(lists_command(list, "export-roadmap-json"));
        assert!(lists_command("aplan facts\n", "facts"));
        assert!(lists_command("aplan check --format json\n", "check"));
    }

    #[test]
    fn a_separator_row_is_not_a_row() {
        assert!(cells("|------|-----------|").is_none());
        assert!(cells("| --- | :---: |").is_none());
        assert!(cells("not a table").is_none());
    }

    #[test]
    fn cells_are_trimmed_and_the_edges_dropped() {
        let c = cells("| `APLAN001` | Frontmatter | error | n/a |").expect("a row");
        assert_eq!(c, vec!["`APLAN001`", "Frontmatter", "error", "n/a"]);
    }

    #[test]
    fn only_the_first_cell_names_the_rule() {
        // A row whose prose mentions another code is not that code's row. This is
        // the distinction the old `contains` gate could not draw.
        let c = cells("| `APLAN024` | extends `APLAN012` | warning | L2 |").expect("a row");
        assert_eq!(row_code(&c).as_deref(), Some("APLAN024"));
    }

    #[test]
    fn a_prose_line_naming_a_code_is_not_a_row() {
        assert!(cells("`APLAN022` requires every task to be a member").is_none());
    }

    #[test]
    fn severity_is_read_through_the_bolding() {
        assert_eq!(severity_of("**warning**"), "warning");
        assert_eq!(severity_of(" error "), "error");
    }

    #[test]
    fn a_cli_row_is_named_by_its_command_not_its_arguments() {
        let c = cells("| `aplan why <id>` | explanation | exists |").expect("a row");
        assert_eq!(cli_row_command(&c).as_deref(), Some("why"));
        let c = cells("| `aplan-lsp` | LSP server | planned |").expect("a row");
        assert_eq!(cli_row_command(&c), None, "a crate row is not a CLI row");
    }

    #[test]
    fn a_status_claims_existence_only_when_it_starts_with_it() {
        assert!(claims_to_exist("exists"));
        assert!(claims_to_exist("exists (`A1-004`)"));
        assert!(claims_to_exist("partial (`A1-019`, `A1-030`)"));
        assert!(!claims_to_exist("planned"));
        // The trap: a condition that names the word. `aplan migrate` ships when
        // `v2` does, and reading this as a claim was this gate's first false
        // positive.
        assert!(!claims_to_exist("when `v2` exists"));
    }

    #[test]
    fn a_table_inside_a_code_fence_is_not_a_table() {
        // A doc that *shows* the catalog format must not be read as the catalog.
        let doc = "| `APLAN001` | real | error | n/a |\n\
                   ```markdown\n\
                   | `APLAN001` | an example | error | n/a |\n\
                   ```\n";
        assert_eq!(rule_rows(doc).len(), 1, "the fenced copy is not a row");
    }

    #[test]
    fn the_readme_command_list_is_the_fenced_block_under_its_heading() {
        let doc = "intro\n\n## Commands\n\n```sh\naplan check\n```\n\nafter `aplan nonesuch`\n";
        let list = readme_command_list(doc);
        assert!(list.contains("aplan check"));
        assert!(
            !list.contains("aplan nonesuch"),
            "prose after the block is not the command list, which is the whole \
             point: mentions elsewhere used to satisfy this gate"
        );
    }

    #[test]
    fn an_invocation_splits_on_quotes_and_drops_the_comment() {
        assert_eq!(
            common::argv_of("aplan check  # validate"),
            vec!["aplan", "check"]
        );
        assert_eq!(
            common::argv_of("aplan add \"Ship it\" --paths 'src/**'"),
            vec!["aplan", "add", "Ship it", "--paths", "src/**"]
        );
    }

    #[test]
    fn planned_is_read_from_the_status_column_only() {
        let row = |desc: &str, status: &str| {
            vec![
                "`APLAN016`".to_string(),
                desc.to_string(),
                "warning".to_string(),
                "L1".to_string(),
                status.to_string(),
            ]
        };
        assert!(is_planned(&row("url evidence is reachable", "planned")));
        assert!(!is_planned(&row("commit evidence resolves", "done")));
        // A description merely using the word must not read as a status, or a row
        // for a rule that was *removed* passes as one that has not shipped yet.
        assert!(!is_planned(&row("replaces the planned glob check", "done")));
    }
}

#[test]
fn the_require_help_names_every_source_it_accepts() {
    // `A1-093`. `--require` validates against five parts of `aplan facts`, and
    // its help is a doc comment, so nothing but this makes the two agree. It once
    // accepted output formats and evidence kinds while naming only commands: an
    // adopter asserting `--require test` was doing something supported and
    // undocumented, and one reading the help would not have known to try.
    //
    // Doc comments are compile-time, so the help cannot literally be *derived*
    // from the list. This is the achievable half: adding a source without saying
    // so fails here.
    let facts_cmd = aplan::cli_command()
        .get_subcommands()
        .find(|c| c.get_name() == "facts")
        .expect("`aplan facts` exists")
        .clone();
    let arg = facts_cmd
        .get_arguments()
        .find(|a| a.get_id() == "require")
        .expect("`--require` exists")
        .clone();
    let help = format!(
        "{} {}",
        arg.get_help().map(ToString::to_string).unwrap_or_default(),
        arg.get_long_help()
            .map(ToString::to_string)
            .unwrap_or_default()
    );

    let missing: Vec<&str> = aplan::capability_help_words()
        .into_iter()
        .filter(|w| !help.contains(w))
        .collect();

    assert!(
        missing.is_empty(),
        "`--require` accepts these and its help does not say so, which is how a \
         supported assertion becomes an undocumented one:\n  {}\nhelp was: {help}",
        missing.join("\n  ")
    );
}

#[test]
fn the_mcp_help_names_every_tool_the_server_exposes() {
    // `A1-119`. The help said "a deliberately small tool set (`ready`, `why`,
    // `check`, `close`)" while the server exposed six: `aplan_verify` (`A1-102`)
    // and `aplan_add` (`A1-103`) both landed without the sentence being touched.
    // An operator reading it concluded an agent could not resolve evidence or
    // create work, so the help *retracted* two shipped features.
    //
    // Gated rather than derived, on `A1-093`'s precedent. Interpolating the names
    // into the doc comment cannot drift, but clap help is consumer-facing prose
    // and a generated list reads worse than a written sentence. Doc comments are
    // compile-time anyway, so this is the achievable half: adding a tool without
    // saying so fails here.
    let help = mcp_help();
    let missing = tools_missing_from(&aplan::mcp_tool_help_words(), &help);
    assert!(
        missing.is_empty(),
        "`aplan mcp` exposes these and its help does not name them, which is how a \
         shipped tool becomes an invisible one:\n  {}\nhelp was: {help}",
        missing.join("\n  ")
    );
}

/// The tools in `tools` that `help` does not name, as inline code.
fn tools_missing_from(tools: &[&'static str], help: &str) -> Vec<String> {
    tools
        .iter()
        .filter(|w| !help.contains(&format!("`{w}`")))
        .map(|w| (*w).to_string())
        .collect()
}

#[test]
fn the_mcp_help_does_not_name_a_tool_that_does_not_exist() {
    // The other direction, and the worse one: a help that over-promises sends an
    // agent operator to wire up a tool that will never answer. The check is scoped
    // to the parenthesised enumeration rather than to every backticked word in the
    // help, so ordinary prose (`` `close` goes through the same evidence gate ``)
    // does not have to be an allowlist. The convention that buys that: the tool
    // list is the one parenthesised group of comma-separated backticked words.
    let help = mcp_help();
    let listed = enumerated_tools(&help);
    let real: Vec<String> = aplan::mcp_tool_help_words()
        .into_iter()
        .map(str::to_string)
        .collect();
    assert_eq!(
        listed, real,
        "the tool list in `aplan mcp`'s help must be exactly what the server \
         exposes, in order; help was: {help}"
    );
}

/// `aplan mcp`'s help, short and long, as one string.
fn mcp_help() -> String {
    let cmd = aplan::cli_command()
        .get_subcommands()
        .find(|c| c.get_name() == "mcp")
        .expect("`aplan mcp` exists")
        .clone();
    // `long_about` when present, else `about`, which is what clap itself shows and
    // therefore what an operator reads. Concatenating both (the `--require` gate
    // does, harmlessly, because it only asks `contains`) duplicates the
    // enumeration the moment the doc comment gains a blank line, and this gate
    // compares the enumeration exactly. Measured: adding one paragraph break made
    // it fail with "found 2" on a help that was entirely correct.
    cmd.get_long_about()
        .or_else(|| cmd.get_about())
        .map(ToString::to_string)
        .unwrap_or_default()
}

/// The one parenthesised list of comma-separated backticked words in `help`.
///
/// Panics if there is not exactly one, because the ambiguity is the bug: a second
/// such list means the help enumerates something else the same way, and this gate
/// would then be pinning the wrong group.
fn enumerated_tools(help: &str) -> Vec<String> {
    let mut found: Vec<Vec<String>> = Vec::new();
    for chunk in help.split('(').skip(1) {
        let Some((inside, _)) = chunk.split_once(')') else {
            continue;
        };
        let items: Vec<String> = inside
            .split(',')
            .map(|w| w.trim().trim_matches('`').to_string())
            .collect();
        let is_list = items.len() > 1
            && inside.matches('`').count() == items.len() * 2
            && items.iter().all(|w| !w.is_empty());
        if is_list {
            found.push(items);
        }
    }
    assert_eq!(
        found.len(),
        1,
        "expected exactly one backticked enumeration in `aplan mcp`'s help, found \
         {}; this gate pins that group and cannot tell two apart",
        found.len()
    );
    found.remove(0)
}

// The two gates above run against the real help, so on a clean tree they pass
// whatever they do. These prove the comparison itself, on synthetic input, which
// is what `A1-119`'s second criterion asked for: adding a tool *in the test*
// rather than trusting a mutation someone ran once and reverted.

#[test]
fn the_upgrade_guide_names_every_capability_source() {
    // Found auditing `A1-119`: `docs/upgrading.md` told adopters `--require`
    // accepts "a config key, a rule code, a command, an output format, or an
    // evidence kind" -- five of six. `A1-099` added `behaviours` and did not
    // update this sentence, so the guide that exists to tell an adopter what a
    // build can assert was a release behind on the mechanism for asserting it.
    //
    // Same list, same gate as `--require`'s own help. That help was correct while
    // this was wrong, which is the argument for holding both to the source rather
    // than one of them.
    // Scoped to the sentence, not the file. The first version of this test asked
    // whether the *document* contained each word and passed on the stale list,
    // because table rows further down happened to say "behaviour" and "MCP tool".
    // A gate over a whole document is nearly always vacuous: something somewhere
    // says the word.
    let guide = repo_file("docs/upgrading.md");
    let lead = "Anything the build advertises:";
    let start = guide
        .find(lead)
        .expect("`docs/upgrading.md` introduces the `--require` sources with a known phrase");
    let sentence = &guide[start..][..guide[start..]
        .find('.')
        .expect("that sentence ends with a full stop")];

    let missing: Vec<&str> = aplan::capability_help_words()
        .into_iter()
        .filter(|w| !sentence.contains(w))
        .collect();
    assert!(
        missing.is_empty(),
        "`--require` accepts these and `docs/upgrading.md`'s list of what it accepts \
         does not name them, so an adopter cannot learn they are assertable:\n  \
         {}\nthe sentence was: {sentence}",
        missing.join("\n  ")
    );
}

#[test]
fn a_tool_the_help_omits_is_reported() {
    let help = "a deliberately small tool set (`ready`, `why`) to an agent.";
    assert_eq!(
        tools_missing_from(&["ready", "why", "verify"], help),
        vec!["verify".to_string()],
        "a tool absent from the help must be named"
    );
    assert!(
        tools_missing_from(&["ready", "why"], help).is_empty(),
        "a help naming every tool must be silent"
    );
}

#[test]
fn a_tool_the_help_invents_is_reported() {
    // The over-promising direction: the enumeration is read whole, so an extra
    // name is visible even though every real tool is present.
    assert_eq!(
        enumerated_tools("a tool set (`ready`, `why`, `deploy`) to an agent."),
        vec!["ready".to_string(), "why".to_string(), "deploy".to_string()]
    );
}

#[test]
fn prose_parentheses_are_not_mistaken_for_the_tool_list() {
    // Why the enumeration must hold more than one backticked item: ordinary help
    // prose says things like "(`close`)", and treating that as a second list
    // would make the gate fail on correct text.
    assert_eq!(
        enumerated_tools("set (`ready`, `why`) to an agent (`close` is gated)."),
        vec!["ready".to_string(), "why".to_string()]
    );
}

#[test]
fn a_build_predating_a_behaviour_does_not_advertise_it() {
    // `A1-099` claimed this and did not check it: `--require traces.table_rows`
    // must *fail* on a build that predates row anchors, or the assertion is
    // decoration. The fixture is this repository's own `facts.json` from the
    // commit before `behaviours` existed, so it is what an older binary really
    // printed rather than a doctored document.
    let old: Value = serde_json::from_str(&repo_file(
        "crates/aplan/tests/fixtures/facts-before-behaviours.json",
    ))
    .expect("the archived facts are JSON");

    assert!(
        old.get("behaviours").is_none(),
        "the fixture must predate the key, or this test proves nothing"
    );
    let advertised = aplan::advertised_names(&old);
    assert!(
        !advertised.contains("traces.table_rows"),
        "an older build must not advertise a behaviour it does not have"
    );
    // And it still advertises everything it did have, so the skip is scoped to
    // the missing key rather than emptying the set.
    assert!(advertised.contains("APLAN008"), "rules still resolve");
    assert!(advertised.contains("check"), "commands still resolve");
    assert!(
        advertised.contains("detect_drift"),
        "config keys still resolve"
    );

    // This build does advertise it, which is the other half of the contract.
    let now: Value = serde_json::from_str(&aplan::facts()).expect("facts is JSON");
    assert!(aplan::advertised_names(&now).contains("traces.table_rows"));
}

/// The build this behaviour is about, rather than one predating behaviours
/// entirely.
///
/// `facts-before-behaviours.json` proves the mechanism and not this claim: it has
/// no `behaviours` key at all, so it fails to advertise *everything*. This
/// fixture is `facts.json` from the commit before `close` resolved evidence, so
/// it advertises `close`, `aplan_close` and `traces.table_rows`, and is exactly
/// the build an adopter could have pinned by name while getting the silent
/// failure `A1-109` repaired (`A1-132`).
#[test]
fn the_build_before_close_resolved_evidence_advertises_the_names_but_not_the_widening() {
    let old: Value = serde_json::from_str(&repo_file(
        "crates/aplan/tests/fixtures/facts-before-close-resolved.json",
    ))
    .expect("the archived facts are JSON");
    let advertised = aplan::advertised_names(&old);

    // The names an adopter would have required, all present on the build that
    // could not keep the promise.
    assert!(
        advertised.contains("close"),
        "the command was already named"
    );
    assert!(
        advertised.contains("aplan_close"),
        "the tool was already named"
    );
    assert!(
        advertised.contains("traces.table_rows"),
        "it had behaviours, so this is not the mechanism failing"
    );

    // And not the one that says what the widening did.
    assert!(
        !advertised.contains("close.evidence_resolves_before_write"),
        "an older build advertised a guarantee it did not have"
    );

    // This build does, which is the other half of the contract.
    let now: Value = serde_json::from_str(&aplan::facts()).expect("facts is JSON");
    assert!(aplan::advertised_names(&now).contains("close.evidence_resolves_before_write"));
}

#[test]
fn a_behaviour_name_cannot_be_mistaken_for_something_else() {
    // `advertised` is one flat set, so a behaviour sharing a name with a rule,
    // command, config key, format or evidence kind would be indistinguishable:
    // `--require X` would pass for the wrong reason, which is worse than failing.
    let facts: Value = serde_json::from_str(&aplan::facts()).expect("facts is JSON");
    let behaviours: BTreeSet<String> = facts["behaviours"]
        .as_array()
        .expect("behaviours array")
        .iter()
        .map(|b| b["name"].as_str().expect("name").to_string())
        .collect();
    assert!(!behaviours.is_empty(), "the namespace exists to be used");

    let mut others = rule_severities().into_keys().collect::<BTreeSet<String>>();
    others.extend(command_names());
    for key in ["config_keys", "formats", "evidence_kinds"] {
        others.extend(
            facts[key]
                .as_array()
                .expect("array")
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string),
        );
    }

    let clashes: Vec<&String> = behaviours.iter().filter(|b| others.contains(*b)).collect();
    assert!(
        clashes.is_empty(),
        "a behaviour name is taken already: {clashes:?}"
    );

    // And never an `APLANxxx` code: a rule code is a stable consumer contract, so
    // `APLAN008.table_rows` would read as a rule (AGENTS.md).
    let rule_shaped: Vec<&String> = behaviours
        .iter()
        .filter(|b| b.starts_with("APLAN"))
        .collect();
    assert!(
        rule_shaped.is_empty(),
        "a behaviour must not be spelled as a rule code: {rule_shaped:?}"
    );
}

/// The decision `A1-142` reached, kept where it was put.
///
/// The measurement said no gate: a catalogue-diff would have caught three of
/// nine cases at 25% precision, a corpus-diff four at 19%, and a gate wrong
/// three times in four teaches its own escape hatch. What was built instead is a
/// question in the audit step and a reminder at each point where a behaviour
/// moves -- which is worth exactly as much as it is still there, so this is what
/// says it is.
///
/// Five places, not four. `A1-142`'s own audit found two unnamed behaviours it
/// had missed (`ready` at a cut boundary, and every mutating command refusing
/// what `check` would reject), both of them *commands*, because the reminders
/// covered rules, tools and the corpus and nothing sat where a command's
/// behaviour is edited. A list that omits a surface is a blind spot with a test
/// around it.
#[test]
fn the_behaviour_reminder_is_at_every_point_a_behaviour_moves() {
    let places = [
        (
            "AGENTS.md",
            "Did this change alter what an existing rule, command or tool *does*?",
        ),
        (
            "crates/aplan-core/src/diagnostic.rs",
            "Changing what an existing rule *does* earns a `Behaviour`",
        ),
        (
            "crates/aplan-mcp/src/tools.rs",
            "Changing what an existing tool *does* earns a `Behaviour`",
        ),
        (
            "crates/aplan-e2e/tests/coverage.rs",
            "If you changed *when* a rule fires, name the behaviour",
        ),
        (
            "crates/aplan/src/lib.rs",
            "Changing what an existing command *does* earns a `Behaviour`",
        ),
    ];
    let missing: Vec<&str> = places
        .iter()
        .filter(|(path, phrase)| !repo_file(path).contains(*phrase))
        .map(|(path, _)| *path)
        .collect();
    assert!(
        missing.is_empty(),
        "the reminder `A1-142` put at each point of change is gone from: {missing:?}. \
         Deleting it is a decision; make it deliberately, and say so in the commit."
    );
}

/// Every optional feature says who it is *not* for (`A1-149`).
///
/// The third pilot read three shipped features in full and declined all three,
/// each correctly, because nothing in the guide said which plan shape a feature
/// is for (pre-public adoption evidence item 25). The fix is a table, and a table is
/// prose: what keeps it true is this test. A new top-level config key cannot
/// ship without a row saying when to skip it.
///
/// Top-level keys only. `evidence.exempt_tags` is a knob inside a feature whose
/// row already exists, and demanding a row per knob would produce the noise this
/// table exists to remove.
#[test]
fn every_optional_feature_says_who_it_is_not_for() {
    /// Does this cell name this key, as a word?
    ///
    /// A key may be followed by a dot -- the `evidence` row is titled
    /// `evidence.require_min_strength`, because that is the knob a reader is
    /// deciding about -- but never joined to letters, digits or an underscore,
    /// which is how `root` hid inside `traces_root`.
    fn names_key(cell: &str, key: &str) -> bool {
        cell.match_indices(key).any(|(at, _)| {
            let before = cell[..at].chars().next_back();
            let after = cell[at + key.len()..].chars().next();
            let open = |c: Option<char>| !c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            open(before) && open(after)
        })
    }

    let upgrading = repo_file("docs/upgrading.md");
    let table = upgrading
        .split("## Which of these are for you")
        .nth(1)
        .expect("the section exists")
        .split("\n## ")
        .next()
        .expect("its rows");

    // The *first cell* of each row, which is what names the feature. Searching
    // the whole table instead let a missing row pass on a coincidence: delete
    // the `root` row and `traces_root` two rows down still contains the word
    // (this task's audit).
    let named: Vec<&str> = table
        .lines()
        .filter(|l| l.starts_with('|') && !l.starts_with("|---") && !l.starts_with("| Feature"))
        .filter_map(|l| l.trim_matches('|').split(" | ").next())
        .collect();

    let facts: Value = serde_json::from_str(&aplan::facts()).expect("facts is JSON");
    let missing: Vec<String> = facts["config_keys"]
        .as_array()
        .expect("config_keys array")
        .iter()
        .map(|k| k.as_str().expect("key").to_string())
        .filter(|k| !k.contains('.'))
        .filter(|k| !named.iter().any(|cell| names_key(cell, k)))
        .collect();
    assert!(
        missing.is_empty(),
        "config keys with no row in `Which of these are for you`, so an adopter has to read the \
         feature to find out it is not for them:\n  {}",
        missing.join("\n  ")
    );

    // One row per feature. A second row for the same one is how the `attest`
    // entry arrived: the upgrade table's row was pasted into this table, so it
    // carried a feature *description* where the applicability answer belongs and
    // the real row sat under it (`A1-153`, from the fifth pilot). Which column a
    // sentence answers is a review's judgement; that a feature is named twice is
    // not, and this is the half a gate can hold.
    let mut seen: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for cell in &named {
        *seen.entry(cell.trim()).or_default() += 1;
    }
    let repeated: Vec<&str> = seen
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(cell, _)| *cell)
        .collect();
    assert!(
        repeated.is_empty(),
        "a feature has more than one row, so one of them is answering a question this table does \
         not ask:\n  {}",
        repeated.join("\n  ")
    );

    // A row naming a command names one that exists. Four rows describe features
    // that are not config keys -- a command, a task field, a tag, a plan shape --
    // so `facts` cannot gate them the way it gates the keys; this catches the
    // half of that which is checkable, which is a row left behind by a rename.
    let commands: Vec<&str> = facts["commands"]
        .as_array()
        .expect("commands array")
        .iter()
        .map(|c| c["name"].as_str().expect("name"))
        .collect();
    for cell in &named {
        if let Some(rest) = cell.split("`aplan ").nth(1)
            && let Some(name) = rest.split(['`', ' ']).next()
        {
            assert!(
                commands.contains(&name),
                "the `{name}` row names a command this build does not have: {cell}"
            );
        }
    }

    // Every row answers both halves. A row whose right-hand cell is empty is the
    // omission this test exists for, arriving as a formatting slip.
    for row in table
        .lines()
        .filter(|l| l.starts_with('|') && !l.starts_with("|---") && !l.starts_with("| Feature"))
    {
        let cells: Vec<&str> = row.trim_matches('|').split(" | ").collect();
        assert_eq!(cells.len(), 3, "a row must have three cells: {row}");
        assert!(
            cells.iter().all(|c| c.trim().len() > 3),
            "a row must answer both halves: {row}"
        );
    }
}

#[test]
fn every_behaviour_is_documented_where_an_adopter_stages_a_binary() {
    // The same class the rule and command checks cover: a name a consumer can
    // assert, shipping with nothing telling them it exists.
    let upgrading = repo_file("docs/upgrading.md");
    let facts: Value = serde_json::from_str(&aplan::facts()).expect("facts is JSON");
    let missing: Vec<String> = facts["behaviours"]
        .as_array()
        .expect("behaviours array")
        .iter()
        .map(|b| b["name"].as_str().expect("name").to_string())
        .filter(|n| !upgrading.contains(n.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "behaviours absent from docs/upgrading.md, where a consumer staging a binary looks:\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn tui_docs_separate_read_only_views_from_guarded_mutations() {
    // The read-only A1-431 workbench row remained literally true about its own
    // result view after A1-441 shipped, but its unqualified statement that
    // repairs were unavailable made the final operations cut contradict the
    // very next row. Pin both halves of the final product contract.
    let upgrading = repo_file("docs/upgrading.md");
    let row = upgrading
        .lines()
        .find(|line| line.starts_with("| TUI evidence-verification workbench"))
        .expect("the verification workbench upgrade row");
    assert!(
        row.contains("result view itself remains read-only"),
        "the workbench must not imply that merely viewing a result writes the plan"
    );
    assert!(
        row.contains("tui.evidence_mutation_previews_bind_identified_inputs")
            && row.contains("Apply verification repairs"),
        "the final guide must route exact repairs to the separately guarded action"
    );
    assert!(
        !row.contains("Applying proposed repairs is unavailable"),
        "the pre-A1-441 limitation returned to the current upgrade guide"
    );

    let overview = upgrading
        .lines()
        .find(|line| line.starts_with("| TUI plan-health overview"))
        .expect("the overview upgrade row");
    assert!(overview.contains("overview projection itself is read-only"));
    assert!(
        !overview.contains("The TUI remains read-only"),
        "a read-only view was broadened into a false product-wide claim"
    );

    let design = repo_file("docs/design/tui.md");
    assert!(design.contains("The loaded snapshot itself is immutable"));
    assert!(!design.contains("The current TUI is read-only"));
    assert!(
        !design.contains(
            "remaining Reports, onboarding and mutation operations stay in the later feature milestones"
        ),
        "the current design still describes delivered surfaces as future work"
    );
}

/// **The MSRV gate derives its toolchain instead of naming one** (`A1-330`).
///
/// The gate compiles this workspace with the declared `rust-version`, and the
/// obvious way to write it is `cargo +1.88 check`. That would be a third copy
/// of the number, free to drift from `Cargo.toml` the moment the floor moves,
/// and a gate asserting a compiler nobody promised. `A1-327` exists because a
/// contract nothing checked went false; its follow-up must not add one.
///
/// So: the script reads the number, and this refuses a literal version in it.
#[test]
fn the_msrv_gate_derives_the_toolchain_rather_than_naming_it() {
    let script = repo_file("ci/scripts/msrv-build.sh");
    assert!(
        script.contains("Cargo.toml"),
        "the MSRV gate no longer reads `Cargo.toml`, so the compiler it asks for is not \
         derived from the promise it checks"
    );
    // A version literal in an executable line. Comments may name one -- the
    // measured cost and the rationale both do, and forbidding that would make
    // the script undocumentable.
    let literal = regex_like_version_in_code(&script);
    assert!(
        literal.is_none(),
        "`ci/scripts/msrv-build.sh` names a Rust version in code: {}. That is a second copy \
         of `rust-version`, which is the defect `A1-327` fixed",
        literal.unwrap_or_default()
    );
}

/// A `1.NN` literal outside a comment, if there is one. Deliberately crude:
/// the point is to refuse a hardcoded toolchain, not to parse shell.
fn regex_like_version_in_code(script: &str) -> Option<String> {
    for line in script.lines() {
        let code = line.split('#').next().unwrap_or_default();
        // `sed -n 's/^rust-version = "\([0-9.]*\)"...` is the line that reads
        // the number, and its own pattern contains digits and dots. What is
        // forbidden is a *concrete* version: `1.` followed by a digit.
        if code.contains("rust-version") {
            continue;
        }
        let bytes: Vec<char> = code.chars().collect();
        for i in 0..bytes.len().saturating_sub(2) {
            if bytes[i] == '1' && bytes[i + 1] == '.' && bytes[i + 2].is_ascii_digit() {
                return Some(code.trim().to_string());
            }
        }
    }
    None
}

/// **The MSRV an adopter reads is the one the workspace declares** (`A1-327`).
///
/// `A1-327` fixed a declared `rust-version` that nothing checked and that had
/// been false for as long as the lockfile existed. Writing the corrected number
/// into prose without a gate would recreate exactly that defect one document
/// over: two more copies of a contract, drifting from `Cargo.toml` the next
/// time the floor moves, in the two places a source adopter actually looks.
///
/// `cargo xtask msrv` holds the *declaration* to the dependency graph. This
/// holds the *documents* to the declaration. Neither is the other's job.
#[test]
fn the_documented_msrv_is_the_one_the_workspace_declares() {
    let manifest = repo_file("Cargo.toml");
    let declared = manifest
        .lines()
        .find_map(|l| l.strip_prefix("rust-version = "))
        .map(|v| v.trim().trim_matches('"').to_string())
        .expect("the workspace declares a rust-version");
    // `1.88`, not `1.88.0`: what a person writes in prose. A patch-level floor
    // would need the documents to say one too, and nothing here has ever
    // needed one.
    assert_eq!(
        declared.split('.').count(),
        2,
        "the declared rust-version gained a patch level, so the prose below needs \
         revisiting rather than this assertion loosening: {declared}"
    );
    for doc in ["README.md", "docs/upgrading.md"] {
        let text = repo_file(doc);
        assert!(
            text.contains(&format!("Rust {declared}")),
            "{doc} does not say `Rust {declared}`, so the compiler a source adopter is told \
             to use has drifted from the one `Cargo.toml` declares. That is the defect \
             `A1-327` fixed, moved into the documentation"
        );
    }
}

/// The timeout defaults an adopter reads are the ones the code uses (`A1-121`).
///
/// `README.md` and `docs/upgrading.md` state 600 seconds for `test.list` and 30
/// for `url.check`, and an adopter plans a slow runner around those numbers. A
/// unit test pins the constants; this pins the documents to them, so changing
/// one without the other is a failure here rather than a surprise in somebody
/// else's CI.
#[test]
fn the_documented_timeout_defaults_match_the_ones_that_ship() {
    for (doc, needles) in [
        ("README.md", ["600", "30"]),
        ("docs/upgrading.md", ["10 minutes", "30 seconds"]),
    ] {
        let text = repo_file(doc);
        for needle in needles {
            assert!(
                text.contains(needle),
                "{doc} no longer states `{needle}`, so the documented default has drifted \
                 from the one `crates/aplan-evidence/src/resolve.rs` ships"
            );
        }
    }
}

/// The sequential-id recovery runbook keeps the part only a reader can decide
/// explicit (`A1-372`).
///
/// Git can expose both files while the add/add conflict is unresolved, but no
/// final-tree check can recover a task somebody chose not to keep. The guide
/// therefore has to name that inspection point, every structured reference a
/// renumber moves, and the prose it must not guess at. Merely saying "renumber
/// it" is the unsafe procedure this task replaced.
#[test]
fn the_branch_collision_runbook_names_the_irreducible_steps() {
    let guide = repo_file("docs/upgrading.md");
    let section = guide
        .split_once("\n## Resolving a sequential-id add/add conflict\n")
        .expect("the upgrading guide has the branch-collision runbook")
        .1;
    let section = section
        .split_once("\n## ")
        .map_or(section, |(section, _)| section);

    for required in [
        "git show :2:",
        "git show :3:",
        "title and body",
        "frontmatter `id`",
        "identity is the field rather than the path",
        "`blocked_by`",
        "`prefer_after`",
        "`ratified_by`",
        "`contains`",
        "`policy[].when.cut`",
        "git grep -n -F",
        "repository-wide substitution is therefore unsafe",
        "cargo xtask gen-roadmap",
        "cargo xtask check-docs",
        "aplan check",
        "aplan verify",
        "`A1-409`",
    ] {
        assert!(
            section.contains(required),
            "the collision runbook does not name `{required}`"
        );
    }
}

/// The eleven collided names became eleven accounted-for tasks, not whichever
/// side happened to win an add/add conflict (`A1-372`).
///
/// This is intentionally an audit of the repository-specific recovery. The
/// two consolidations are where a count of new files alone would lie, and the
/// two pilot sections are where a global string replacement actually produced
/// a false citation while the work was landed.
const BRANCH_COLLISION_RECOVERY: [(u16, u16, &str); 11] = [
    (
        351,
        358,
        "close duplicates a live task's existing trace fingerprints",
    ),
    (
        352,
        373,
        "p00011 adoption pilot (the repository that had its own plan linter)",
    ),
    (
        353,
        374,
        "A stale installed binary cannot read a newer plan, and the adopter must notice",
    ),
    (
        354,
        375,
        "No way to ask a document which traces anchors it offers",
    ),
    (
        355,
        376,
        "export-agents-md --check reads as a mode and is an option with a required value",
    ),
    (356, 377, "An adopter with CI has nothing to install"),
    (
        357,
        378,
        "The generated agent contract omits revisit_at, so an authoring agent never parks a task with one",
    ),
    (
        358,
        379,
        "A YAML front-matter delimiter mints a heading anchor the rendered document does not have",
    ),
    (
        359,
        380,
        "require_min_strength is a set of accepted kinds, and its name says floor",
    ),
    (
        360,
        381,
        "An exempt tag no task carries is silent, where a declared tag family is not",
    ),
    (
        361,
        371,
        "A parked task's revisit instant passes in silence, because due is wired nowhere",
    ),
];

#[test]
fn the_branch_collision_recovery_keeps_both_sides_and_every_disposition() {
    if !repository_root().join("aplan/tasks/A1-372.md").is_file() {
        assert_public_plan_epoch();
        return;
    }
    let audit = repo_file("aplan/tasks/A1-372.md");
    for (old, current, title) in BRANCH_COLLISION_RECOVERY {
        let id = format!("A1-{current:03}");
        let task = repo_file(&format!("aplan/tasks/{id}.md"));
        assert!(task.lines().any(|line| line == format!("id: {id}")));
        assert!(
            task.lines().any(|line| line == format!("title: {title}")),
            "{id} is no longer the recovered `{title}` task"
        );
        assert!(
            audit.contains(&format!("| `A1-{old:03}` | `{id}` |")),
            "A1-{old:03} -> {id} is absent from the collision audit"
        );
    }

    let p00011 = repo_file("docs/pilots/p00011.md");
    for old in 352..=361 {
        assert!(
            !p00011.contains(&format!("A1-{old:03}")),
            "p00011 still cites the collided A1-{old:03} name"
        );
    }
    for current in 373..=381 {
        assert!(
            p00011.contains(&format!("`A1-{current:03}`")),
            "p00011 lost its disposition to A1-{current:03}"
        );
    }
    let baseid_fifteen = p00011
        .split_once("\n### 15.")
        .expect("p00011 has finding 15")
        .1;
    let baseid_fifteen = baseid_fifteen
        .split_once("\n### ")
        .map_or(baseid_fifteen, |(section, _)| section);
    assert!(baseid_fifteen.contains("`A1-371`"));
    assert!(!baseid_fifteen.contains("`A1-361`"));

    let heroes = repo_file("docs/pilots/p0000000015.md");
    let heroes_twenty_nine = heroes
        .split_once("\n### 29.")
        .expect("p0000000015 has finding 29")
        .1;
    let heroes_twenty_nine = heroes_twenty_nine
        .split_once("\n### ")
        .map_or(heroes_twenty_nine, |(section, _)| section);
    assert!(heroes_twenty_nine.contains("`A1-358`"));
    assert!(!heroes_twenty_nine.contains("`A1-379`"));

    let consolidated = repo_file("aplan/tasks/A1-371.md");
    assert!(
        consolidated.contains(
            "two runs at one commit produce identical output whatever the wall clock says"
        )
    );
}

/// Every relative link this repository's own documents make resolves: the file
/// exists, and where a `#fragment` is given, something in that file mints it.
///
/// `check-docs` gates the ids a document names and `APLAN008` gates the anchors a
/// *plan* traces; nothing gated the links the prose itself makes, so a heading
/// rename silently broke a pointer from one file to another. `A1-056` added the
/// first cross-file anchor link in the README, which is what made the hole worth
/// closing rather than noting.
///
/// Anchors are resolved with the shipped rule (`aplan_core::anchor`), headings
/// and table-row first cells alike (ADR-0015), so this gate and `APLAN008` cannot
/// disagree about what an anchor is.
///
/// **What it does not read**, measured rather than assumed: reference-style links
/// (`[text][ref]`). There is one in the tree, in `CODE_OF_CONDUCT.md`, and it
/// points at a url, so no relative link is currently written that way. A gate
/// that cannot see a shape should say so rather than let its silence read as
/// coverage.
#[test]
fn every_relative_link_in_the_repositorys_docs_resolves() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Every markdown file the repository tracks, rather than a list of the ones
    // worth gating: a list is a second copy of "which documents matter" and would
    // silently stop covering the next one added.
    let mut docs: Vec<String> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if path.is_dir() {
                if !matches!(name.as_str(), "target" | ".git" | "node_modules") {
                    stack.push(path);
                }
            } else if std::path::Path::new(&name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
            {
                docs.push(
                    path.strip_prefix(&root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .to_string(),
                );
            }
        }
    }
    assert!(docs.len() > 100, "only {} documents were found", docs.len());
    let docs: Vec<&str> = docs.iter().map(String::as_str).collect();
    let mut problems = Vec::new();
    let mut checked = 0usize;
    for doc in docs {
        let path = root.join(doc);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue; // A document this repository does not have is not a link.
        };
        let base = path
            .parent()
            .expect("a document has a directory")
            .to_path_buf();
        for target in relative_links(&text) {
            let (file, fragment) = match target.split_once('#') {
                Some((f, a)) => (f, Some(a)),
                None => (target.as_str(), None),
            };
            // A bare `#fragment` points inside this document.
            let target_path = if file.is_empty() {
                path.clone()
            } else {
                base.join(file)
            };
            if !target_path.exists() {
                problems.push(format!(
                    "{doc} links to `{target}`, which is not in the tree"
                ));
                continue;
            }
            checked += 1;
            let Some(fragment) = fragment else { continue };
            if !target_path.is_file() {
                continue; // A directory has no anchors.
            }
            let body = std::fs::read_to_string(&target_path).expect("read link target");
            if !aplan_core::anchor::resolvable_anchors(&body).contains(&fragment.to_lowercase()) {
                problems.push(format!(
                    "{doc} links to `{target}`, and nothing in that file mints `#{fragment}`"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    // Measured at 54 when this was written, over every markdown file in the tree.
    // The floor is well under that: it exists to catch a scan that stopped
    // reading, not to pin a count that moves whenever a link is added or dropped.
    assert!(
        checked > 40,
        "only {checked} links were read, so this gate is not looking"
    );
}

/// Every `](target)` in `text` whose target is a path in this repository, rather
/// than a `http(s)` url or a `mailto:`.
///
/// Fenced blocks and inline code spans are skipped, because a document's examples
/// are not its content: this gate's first run reported `[v0.8](#x)`, which is
/// prose in `docs/upgrading.md` *about* what a decorated heading looks like. It is
/// the same rule `check-docs` learned the hard way, so it is read with the same
/// function.
fn relative_links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (_, line) in aplan_core::anchor::unfenced_lines(text) {
        // Odd-indexed pieces sit between backticks, so they are code spans.
        for prose in line.split('`').step_by(2) {
            let chars: Vec<char> = prose.chars().collect();
            for i in 0..chars.len() {
                if chars[i] != '(' || i == 0 || chars[i - 1] != ']' {
                    continue;
                }
                let Some(end) = chars[i + 1..].iter().position(|c| *c == ')') else {
                    continue;
                };
                let target: String = chars[i + 1..i + 1 + end].iter().collect();
                if target.is_empty()
                    || target.starts_with("http")
                    || target.starts_with("mailto:")
                    || target.contains(' ')
                {
                    continue;
                }
                out.push(target);
            }
        }
    }
    out
}

/// Every friction log is in the pilots index, and the index names only logs that
/// exist.
///
/// `A1-071` asked for its log to be reachable from the documentation index and
/// the answer was a link to the *directory*, which is reachable and is not an
/// index. A hand-kept list of seven files is the unchecked table `AGENTS.md`
/// warns about, so the list is gated instead: enumerate the surface (the files),
/// require an entry, and let the test drive it.
#[test]
fn every_pilot_log_is_indexed() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/pilots");
    if !dir.is_dir() {
        assert_public_plan_epoch();
        return;
    }
    let mut logs: Vec<String> = std::fs::read_dir(&dir)
        .expect("docs/pilots")
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            let is_md = std::path::Path::new(&n)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"));
            (is_md && n != "README.md").then_some(n)
        })
        .collect();
    logs.sort();
    assert!(
        logs.len() >= 5,
        "only {} logs found, so this is not looking",
        logs.len()
    );

    let index = std::fs::read_to_string(dir.join("README.md")).expect("the pilots README");
    let listed: std::collections::BTreeSet<String> = index
        .lines()
        .filter(|l| l.starts_with("| ["))
        .filter_map(|l| l.split('`').nth(1).map(str::to_string))
        .collect();

    let missing: Vec<&String> = logs.iter().filter(|l| !listed.contains(*l)).collect();
    assert!(missing.is_empty(), "not in the pilots index: {missing:?}");
    let extra: Vec<&String> = listed.iter().filter(|l| !logs.contains(l)).collect();
    assert!(
        extra.is_empty(),
        "the pilots index names files that do not exist: {extra:?}"
    );
}
