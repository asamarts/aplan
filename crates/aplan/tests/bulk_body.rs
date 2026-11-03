//! `add --from` carries the task body (`A1-260`).
//!
//! `p0000014` authored 134 tasks through this seam from a research document, and
//! every one of them landed with the scaffold `aplan add` writes. That is not a
//! tidying pass deferred: leave the scaffold's line unticked and `APLAN038`
//! reports every closure, tick it and `APLAN039` does. Both rules are right, and
//! between them a plan authored through the supported bulk surface could not be
//! closed without editing every file through the format the surface exists to
//! hide (pre-public adoption evidence item 2).
//!
//! So the property under test is not "a string reaches a file". It is that a
//! plan authored *entirely* through `--from` closes clean, and that the same
//! plan without bodies does not -- because a fix that satisfies one of the two
//! rules leaves the adopter exactly where they started.

use std::path::Path;

use crate::common;
use common::aplan;

fn repo() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    dir
}

fn manifest(dir: &Path, yaml: &str) -> String {
    std::fs::write(dir.join("plan.yml"), yaml).expect("manifest");
    "plan.yml".to_string()
}

fn add_from(dir: &Path, yaml: &str) -> (String, i32) {
    let file = manifest(dir, yaml);
    aplan(dir, &["add", "--from", &file])
}

fn body_of(dir: &Path, id: &str) -> String {
    let text = std::fs::read_to_string(dir.join(format!("aplan/tasks/{id}.md"))).expect("task");
    let (_, body) = aplan_core::load::split_frontmatter(&text).expect("frontmatter");
    body.trim().to_string()
}

/// The body arrives as written: headings, checkboxes, fences and all.
#[test]
fn a_supplied_body_is_written_verbatim() {
    let dir = repo();
    let (out, code) = add_from(
        dir.path(),
        "- title: A task authored with its body\n  id: T-1\n  effort: 1\n  body: |\n\
         \x20   ## Why\n\n\x20   The research document already says this.\n\n\
         \x20   ## Acceptance criteria\n\n\x20   - [ ] the schema carries a version\n\
         \x20   - [ ] the view reads only through it\n",
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        body_of(dir.path(), "T-1"),
        "## Why\n\nThe research document already says this.\n\n\
         ## Acceptance criteria\n\n- [ ] the schema carries a version\n\
         - [ ] the view reads only through it"
    );
}

/// Omitted keeps the scaffold, which is what an author who has not written the
/// body yet wants. This is the behaviour every existing manifest relies on.
#[test]
fn an_omitted_body_keeps_the_scaffold() {
    let dir = repo();
    let (out, code) = add_from(dir.path(), "- title: A task\n  id: T-1\n  effort: 1\n");
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        body_of(dir.path(), "T-1"),
        aplan_core::scaffold::ADD_BODY.trim()
    );
}

/// `null` says the same thing as leaving the key out, because that is what it
/// says in YAML.
#[test]
fn a_null_body_is_the_same_as_omitting_it() {
    let dir = repo();
    let (out, code) = add_from(
        dir.path(),
        "- title: A task\n  id: T-1\n  effort: 1\n  body: null\n",
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        body_of(dir.path(), "T-1"),
        aplan_core::scaffold::ADD_BODY.trim()
    );
}

/// An empty body is refused rather than written.
///
/// A manifest is generated, so `body: ""` is a lookup that came back empty far
/// more often than it is an author asking for a task with no prose -- and the
/// way to ask for that is to leave the key out, which says the same thing and
/// keeps a scaffold to write into.
#[test]
fn an_empty_body_is_refused_and_names_the_entry() {
    let dir = repo();
    for value in ["\"\"", "\"   \""] {
        let (out, code) = add_from(
            dir.path(),
            &format!("- title: A task\n  id: T-1\n  effort: 1\n  body: {value}\n"),
        );
        assert_eq!(code, 2, "{out}");
        assert!(
            out.contains("entry 1 (`A task`)"),
            "names which entry: {out}"
        );
        assert!(out.contains("Leave the key out"), "says what to do: {out}");
        assert!(
            !dir.path().join("aplan/tasks/T-1.md").exists(),
            "wrote a task it refused"
        );
    }
}

/// A body of `---` lines does not split the file into two tasks.
///
/// The generator this seam replaces emitted frontmatter directly, so a manifest
/// written from a document that *shows* frontmatter is the ordinary case rather
/// than a contrived one.
#[test]
fn a_body_full_of_delimiters_round_trips_as_one_task() {
    let dir = repo();
    let (out, code) = add_from(
        dir.path(),
        "- title: A body carrying delimiters\n  id: T-1\n  effort: 1\n  body: |\n\
         \x20   ## Why\n\n\x20   ```yaml\n\x20   ---\n\x20   id: X-1\n\x20   ---\n\x20   ```\n\n\
         \x20   ---\n\n\x20   Prose after a horizontal rule.\n\n\
         \x20   ## Acceptance criteria\n\n\x20   - [ ] one task, not two\n",
    );
    assert_eq!(code, 0, "{out}");

    let (list, _) = aplan(dir.path(), &["tasks", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&list).expect("json");
    let ids: Vec<&str> = v["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .map(|t| t["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, vec!["T-1"], "the body's `---` split the file");

    // And the checkbox inside the fence is not a criterion, while the real one
    // is: the loader's fence reader is what makes a body safe to write.
    let (out, _) = aplan(dir.path(), &["check"]);
    assert!(out.contains("ok: 1 tasks"), "{out}");
}

/// One bad entry writes nothing, bodies included: the batch is one change.
#[test]
fn a_failing_entry_writes_no_bodies() {
    let dir = repo();
    let (out, code) = add_from(
        dir.path(),
        "- title: One\n  id: T-1\n  effort: 1\n  body: \"## Why\\n\\nreal prose\\n\"\n\
         - title: Two\n  id: T-2\n  effort: 1\n  body: \"\"\n",
    );
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("entry 2"), "{out}");
    assert!(
        !dir.path().join("aplan/tasks/T-1.md").exists(),
        "the entry before the bad one was written"
    );
}

/// A dry run reports what it would write and writes nothing.
#[test]
fn a_dry_run_carries_bodies_and_writes_nothing() {
    let dir = repo();
    let file = manifest(
        dir.path(),
        "- title: One\n  id: T-1\n  effort: 1\n  body: \"## Why\\n\\nreal prose\\n\"\n",
    );
    let (out, code) = aplan(dir.path(), &["add", "--from", &file, "--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("would write T-1"), "{out}");
    assert!(!dir.path().join("aplan/tasks/T-1.md").exists());
}

/// The run says how many tasks got a body.
///
/// A manifest is generated, so a template that emitted `body` for some entries
/// and not others produces a run that reads exactly like one that worked. The
/// difference surfaces at closure, when `APLAN038` reports every task that kept
/// the scaffold -- which is `p0000014`'s own finding arriving one level up
/// (`A1-260`, audit).
#[test]
fn the_run_says_how_many_tasks_carry_a_body() {
    let dir = repo();
    let (out, _) = add_from(
        dir.path(),
        "- title: One\n  id: T-1\n  effort: 1\n  body: \"## Why\\n\\na\\n\"\n\
         - title: Two\n  id: T-2\n  effort: 1\n  body: \"## Why\\n\\nb\\n\"\n",
    );
    assert!(out.contains("all 2 carry a body"), "{out}");

    let mixed = repo();
    let (out, _) = add_from(
        mixed.path(),
        "- title: One\n  id: T-1\n  effort: 1\n  body: \"## Why\\n\\na\\n\"\n\
         - title: Two\n  id: T-2\n  effort: 1\n",
    );
    assert!(
        out.contains("1 of 2 carry a body; the rest keep the scaffold"),
        "the half that silently kept the scaffold is what needs saying: {out}"
    );

    // And a manifest carrying none is every manifest written before this, so it
    // says nothing new.
    let plain = repo();
    let (out, _) = add_from(
        plain.path(),
        "- title: One\n  id: T-1\n  effort: 1\n- title: Two\n  id: T-2\n  effort: 1\n",
    );
    assert!(
        !out.contains("carry a body"),
        "noise on an unchanged run: {out}"
    );
}

/// **The property the task exists for.** A plan authored entirely through
/// `--from`, with bodies, closes clean under both `APLAN038` and `APLAN039`; the
/// same plan without them does not.
///
/// Both halves matter. Satisfying only the first would leave an adopter ticking
/// the scaffold and meeting the second, which is where `p0000014` already is.
#[test]
fn a_plan_authored_with_bodies_closes_clean_under_both_rules() {
    let dir = repo();
    let (out, code) = add_from(
        dir.path(),
        "- title: Authored with a real criterion\n  id: T-1\n  effort: 1\n  body: |\n\
         \x20   ## Why\n\n\x20   Because the document says so.\n\n\
         \x20   ## Acceptance criteria\n\n\x20   - [x] the schema carries a version\n\
         - title: Left with the scaffold\n  id: T-2\n  effort: 1\n",
    );
    assert_eq!(code, 0, "{out}");

    let scaffolded = dir.path().join("aplan/tasks/T-2.md");

    // A `file` citation resolving against the task's own file: no git needed,
    // and `--no-resolve` keeps it from being opened at all. What is under test
    // is what `close` *reports*, not what it proves.
    let close = |id: &str| {
        let (out, _) = aplan(
            dir.path(),
            &[
                "close",
                id,
                "--evidence",
                &format!("file:aplan/tasks/{id}.md"),
                "--no-resolve",
            ],
        );
        out
    };

    let authored = close("T-1");
    assert!(
        !authored.contains("APLAN038") && !authored.contains("APLAN039"),
        "a task authored with its own criteria was reported:\n{authored}"
    );

    let untouched = close("T-2");
    assert!(
        untouched.contains("APLAN038"),
        "the scaffold left unticked must still be reported:\n{untouched}"
    );

    // A tick on the scaffold is the shortcut the adopter reaches for, and it is
    // applied to the file *as `close` left it*: reading before the close and
    // writing after would put the task back to `todo`, where neither rule
    // applies and this assertion would pass for that reason.
    let ticked = std::fs::read_to_string(&scaffolded)
        .expect("read")
        .replace("- [ ]", "- [x]");
    std::fs::write(&scaffolded, ticked).expect("write");
    let (after, _) = aplan(dir.path(), &["check"]);
    assert!(
        after.contains("APLAN039"),
        "the scaffold ticked must still be reported, or this fix changed nothing:\n{after}"
    );
}
