//! The path from an empty repository to a first valid plan (`A1-292`).
//!
//! Found in the `p00023` pilot, on the first command of the adoption. Under
//! `deny_warnings: true` with `require_cut_membership: true` -- the policy this
//! repository runs on itself -- neither half of a new plan can be added on its
//! own: a first task belongs to no cut (`APLAN022`) and a first cut has no
//! members (`APLAN021`), and each refusal used to point at the other.
//!
//! What these pin is that the way through is *written down and works*. The recipe
//! is not retyped here: it is read out of `docs/quickstart.md` and run, so a documented
//! path that stops working fails a test rather than an adopter.

use std::path::Path;

use crate::common;
use common::{aplan, run};

/// The policy this repository runs on itself, which is the one that traps.
const STRICT: &str = "root: aplan\ndeny_warnings: true\nrequire_cut_membership: true\n";

fn quickstart() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join("docs/quickstart.md")).expect("docs/quickstart.md")
}

/// The body of the first fenced block of `kind` under a `## ` heading.
fn fenced_under(doc: &str, heading: &str, kind: &str) -> String {
    let after = doc
        .split_once(&format!("\n## {heading}\n"))
        .unwrap_or_else(|| panic!("docs/quickstart.md has a `## {heading}` section"))
        .1;
    let end = after.find("\n## ").unwrap_or(after.len());
    let section = &after[..end];
    let open = format!("```{kind}\n");
    let start = section
        .find(&open)
        .unwrap_or_else(|| panic!("`## {heading}` opens no ```{kind} block"))
        + open.len();
    let close = section[start..]
        .find("```")
        .expect("an unclosed fence in docs/quickstart.md");
    section[start..start + close].to_string()
}

fn greenfield(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), STRICT).expect("config");
}

/// **The recipe in `docs/quickstart.md` is run, not described.**
///
/// The YAML is the bytes the quickstart shows, so the documented path and the tested
/// one cannot drift apart: an edit that breaks the recipe fails here.
#[test]
fn the_documented_recipe_reaches_a_valid_plan_under_the_strict_policy() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    greenfield(dir.path());
    let recipe = fenced_under(&quickstart(), "Starting a plan", "yaml");
    assert!(
        recipe.contains("tags: [milestone]") && recipe.contains("cut: M-v0.1"),
        "the quickstart's recipe is not a cut and its work: {recipe}"
    );
    std::fs::write(dir.path().join("plan.yml"), &recipe).expect("plan.yml");

    let (out, code) = aplan(dir.path(), &["add", "--from", "plan.yml"]);
    assert_eq!(code, 0, "the documented recipe was refused: {out}");

    let (out, code) = aplan(dir.path(), &["check"]);
    assert_eq!(
        code, 0,
        "the plan the recipe produces does not check: {out}"
    );
    assert!(out.contains("ok: 3 tasks, 0 warning(s)"), "{out}");
}

/// What the quickstart says the recipe prints is what it prints.
#[test]
fn the_documented_output_is_the_output() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let doc = quickstart();
    std::fs::write(
        dir.path().join("plan.yml"),
        fenced_under(&doc, "Starting a plan", "yaml"),
    )
    .expect("plan.yml");

    // The three commands the block shows, in the order it shows them, against
    // the config `aplan init` writes and nothing else. **Not the strict policy**:
    // the block shows a reader typing `aplan init` and then the recipe, so what
    // it quotes has to be what that produces. The strict policy is the harder
    // case and is the test above.
    let init = run(dir.path(), &["init"]).text();
    let console = fenced_under(&doc, "Starting a plan", "console");
    let real = format!(
        "{init}{}{}",
        run(dir.path(), &["add", "--from", "plan.yml"]).text(),
        run(dir.path(), &["check"]).text()
    );
    // Line by line rather than whole-block: the console shows `aplan init`'s
    // output too, and the `-> path` line under each task names a temporary
    // directory that no document can carry.
    let quoted: Vec<&str> = console
        .lines()
        .filter(|l| {
            l.starts_with("wrote ")
                || l.starts_with("ok: ")
                || l.starts_with("  `")
                || l.starts_with("edit it")
        })
        .collect();
    // **Counted, because a `contains` loop over an empty list passes.** A quickstart
    // edit that reworded every quoted line would otherwise leave this asserting
    // nothing at all.
    assert_eq!(
        quoted.len(),
        7,
        "the console block no longer quotes the output it used to: {quoted:#?}"
    );
    for line in quoted {
        assert!(
            real.contains(line),
            "docs/quickstart.md shows a line the run does not print: {line:?}\n{real}"
        );
    }
}

/// **Both halves of the deadlock name the way out.**
///
/// The regression that matters most: the `APLAN022` refusal used to say
/// `aplan add "the first cut" --id <ID> --tag milestone` makes a cut, which is
/// the command the `APLAN021` refusal declines under this policy. An adopter
/// following either message arrived at the other one.
#[test]
fn neither_refusal_sends_the_reader_at_the_one_that_refuses_them() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    greenfield(dir.path());

    let cut_first = run(
        dir.path(),
        &["add", "The first cut", "--id", "M-s0", "--tag", "milestone"],
    );
    assert_ne!(cut_first.code, 0, "{}", cut_first.text());
    assert!(
        cut_first.text().contains("aplan add --from plan.yml"),
        "the empty-cut refusal does not name a way through: {}",
        cut_first.text()
    );

    let task_first = run(
        dir.path(),
        &["add", "The first task", "--id", "T-001", "--effort", "1"],
    );
    assert_ne!(task_first.code, 0, "{}", task_first.text());
    let text = task_first.text();
    assert!(
        text.contains("aplan add --from plan.yml"),
        "the no-cut refusal does not name a way through: {text}"
    );
    assert!(
        !text.contains("--tag milestone"),
        "the no-cut refusal still sends the reader at the command that refuses them: {text}"
    );
}

/// A cut is created with its members, and where the members already exist and
/// are in no cut, one command does it. The other half of what the quickstart's table
/// promises.
#[test]
fn a_cut_can_be_created_around_work_the_plan_already_has() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    greenfield(dir.path());
    // A `cross-cutting` task satisfies `APLAN022` while belonging to no cut,
    // which is the only way a plan under this policy holds unplaced work.
    let unplaced = |tags: &str| {
        format!(
            "---\nid: T-003\ntitle: Unplaced work\nphase: 1\nstatus: todo\n\
             effort_days: 1.0\n{tags}---\n\n## Why\n\nBecause.\n"
        )
    };
    std::fs::write(
        dir.path().join("aplan/T-003.md"),
        unplaced("tags:\n- cross-cutting\n"),
    )
    .expect("task");

    // **And placing it while it still carries the tag is refused** (`A1-212`).
    // `APLAN052` says the tag cannot mean "belongs to no cut" on a task that is
    // in one, and `add` runs the rules against the plan it would produce -- so
    // the contradiction is caught at the moment it would be created rather than
    // sitting in the plan afterwards.
    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "Second cut",
            "--id",
            "M-v0.2",
            "--tag",
            "milestone",
            "--blocked-by",
            "T-003",
        ],
    );
    assert_ne!(
        code, 0,
        "placing a `cross-cutting` task in a cut was allowed: {out}"
    );
    assert!(out.contains("APLAN052"), "{out}");

    // The tag goes when the work is placed, which is what it means.
    std::fs::write(dir.path().join("aplan/T-003.md"), unplaced("")).expect("task");

    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "Second cut",
            "--id",
            "M-v0.2",
            "--tag",
            "milestone",
            "--blocked-by",
            "T-003",
        ],
    );
    assert_eq!(code, 0, "a cut named with its member was refused: {out}");
    let (out, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "{out}");
}

/// **The advice depends on the policy, because the deadlock does.**
///
/// `APLAN021` is a warning, so a first cut can be made empty wherever warnings do
/// not block, and the two-step is the simpler answer there. Collapsing both into
/// one sentence would either send a strict repository at a command it refuses, or
/// send everybody else to write a file they do not need.
#[test]
fn a_repository_that_allows_an_empty_cut_is_told_to_make_one() {
    let lenient = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(lenient.path().join("aplan")).expect("mkdir");
    std::fs::write(
        lenient.path().join(".aplan.yml"),
        "root: aplan\nrequire_cut_membership: true\n",
    )
    .expect("config");
    let text = run(
        lenient.path(),
        &["add", "a first task", "--id", "A-001", "--effort", "1"],
    )
    .text();
    assert!(
        text.contains("--tag milestone"),
        "a policy that allows an empty cut is sent the long way round: {text}"
    );

    let strict = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    greenfield(strict.path());
    let text = run(
        strict.path(),
        &["add", "a first task", "--id", "A-001", "--effort", "1"],
    )
    .text();
    assert!(
        text.contains("aplan add --from plan.yml") && !text.contains("--tag milestone"),
        "a policy that refuses an empty cut is sent at the command that refuses it: {text}"
    );
}

/// **A cut this change emptied is not a cut this change created.**
///
/// `mv` moving the last member out raises the same `APLAN021` that a new empty
/// cut does, and the first version of this advice fired there too -- telling the
/// author of a *move* to write a file and run `aplan add --from`. The rule's own
/// remedy is complete for that case, so nothing is added to it.
#[test]
fn emptying_a_cut_is_not_told_how_to_create_one() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    greenfield(dir.path());
    std::fs::write(
        dir.path().join("plan.yml"),
        "- id: M-a\n  title: Alpha\n  tags: [milestone]\n\
         - id: T-001\n  title: The only member of Alpha\n  cut: M-a\n  effort: 1\n\
         - id: M-b\n  title: Beta\n  tags: [milestone]\n\
         - id: T-002\n  title: A member of Beta\n  cut: M-b\n  effort: 1\n",
    )
    .expect("plan.yml");
    let (out, code) = aplan(dir.path(), &["add", "--from", "plan.yml"]);
    assert_eq!(code, 0, "{out}");

    let out = run(dir.path(), &["mv", "T-001", "--cut", "M-b"]);
    assert_ne!(
        out.code,
        0,
        "emptying a cut is refused under this policy: {}",
        out.text()
    );
    let text = out.text();
    assert!(text.contains("APLAN021"), "{text}");
    assert!(
        !text.contains("A cut is created with its work"),
        "a move was told how to create a cut: {text}"
    );
    assert!(
        !text.contains("aplan add --from"),
        "a move was sent to the bulk add seam: {text}"
    );
}
