//! Importing spec-driven task lists, end to end (`A1-039`).
//!
//! The unit tests in `aplan-interop` prove the parse. These prove the thing that
//! matters to an adopter and that a parser test cannot: the tasks it writes load
//! back through the real loader and pass the real validator, which is the round
//! trip `A1-019` established for the KEP flavor.
//!
//! The documents are the real ones, trimmed rather than invented: the item
//! shapes come from `github/spec-kit`'s `templates/tasks-template.md` and
//! `Fission-AI/OpenSpec`'s `openspec/changes/.../tasks.md`, read on 2026-08-17.

use std::path::Path;

use crate::common;
use common::aplan;

/// The shapes that matter from spec-kit's template, including the ordering prose
/// this must refuse to turn into edges.
const SPEC_KIT: &str = "# Tasks: Feature\n\
    \n\
    ## Phase 1: Setup (Shared Infrastructure)\n\
    \n\
    - [ ] T001 Create project structure per implementation plan\n\
    - [x] T003 [P] Configure linting and formatting tools\n\
    \n\
    ## Phase 3: User Story 1 - Title (Priority: P1)\n\
    \n\
    - [ ] T012 [P] [US1] Create Entity1 model in src/models/entity1.py\n\
    \n\
    ## Dependencies & Execution Order\n\
    \n\
    ### Within Each User Story\n\
    \n\
    - Models before services\n\
    - Services before endpoints\n";

const OPEN_SPEC: &str = "# Implementation Tasks\n\
    \n\
    ## 1. Core Implementation\n\
    - [x] 1.1 Create `src/core/diff.ts` with diff logic\n\
    - [ ] 1.2 Implement change directory scanning\n\
    \n\
    ## 2. CLI Integration\n\
    - [ ] 2.1 Add diff command to `src/cli/index.ts`\n";

#[test]
fn a_spec_kit_document_imports_and_the_plan_it_writes_validates() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), "tasks.md", SPEC_KIT);

    let (out, code) = aplan(
        dir.path(),
        &[
            "import",
            "tasks.md",
            "--format",
            "spec-kit",
            "--id-prefix",
            "SK",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("SK-T001"), "ids are the document's: {out}");
    assert!(
        out.contains("became no `blocked_by` edge"),
        "the ordering prose is reported, not imported: {out}"
    );

    // The round trip: what it wrote is a plan this build can read and judge.
    let (check, code) = aplan(dir.path(), &["check"]);
    assert!(code <= 1, "the written plan must load: {check}");
    assert!(check.contains("3 tasks"), "{check}");
    assert!(
        !check.contains("APLAN003"),
        "no edge was invented, so no id can dangle: {check}"
    );

    // And the ordering the source described in English can be declared, which is
    // the answer the notice names.
    let (dep, code) = aplan(dir.path(), &["dep", "add", "SK-T012", "--after", "SK-T001"]);
    assert_eq!(code, 0, "{dep}");
    let (check, _) = aplan(dir.path(), &["check"]);
    assert!(check.contains("DAG acyclic"), "{check}");
}

#[test]
fn an_openspec_document_imports_through_the_same_command() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), "tasks.md", OPEN_SPEC);

    let (out, code) = aplan(
        dir.path(),
        &[
            "import",
            "tasks.md",
            "--format",
            "openspec",
            "--id-prefix",
            "OS",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("OS-1.1"), "{out}");
    // A ticked box is a claim: it arrives `todo`, and the report says why.
    assert!(out.contains("was ticked in the source"), "{out}");
    assert!(!out.contains("OS-1.1  done"), "{out}");

    let (check, code) = aplan(dir.path(), &["check"]);
    assert!(code <= 1, "{check}");
    assert!(check.contains("3 tasks"), "{check}");
}

/// The two flavors are two names for one parser, and each still refuses the
/// other's document rather than reading it wrong (`A1-039`).
#[test]
fn each_flavor_declines_the_other_document_with_a_message_that_helps() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), "tasks.md", SPEC_KIT);
    let (out, code) = aplan(dir.path(), &["import", "tasks.md", "--format", "openspec"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("--format openspec"), "{out}");
    assert!(out.contains("--format checklist"), "the fallback: {out}");

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), "tasks.md", OPEN_SPEC);
    let (out, code) = aplan(dir.path(), &["import", "tasks.md", "--format", "spec-kit"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("--format spec-kit"), "{out}");
}

/// A document that carries ids keeps them when nothing is asked for, and
/// `--id-prefix` is how two features coexist (`A1-039`, and its own test wart:
/// the default prefix produced `T-T001`).
#[test]
fn an_unqualified_import_keeps_the_documents_own_ids() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), "tasks.md", SPEC_KIT);
    let (out, code) = aplan(dir.path(), &["import", "tasks.md", "--format", "spec-kit"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(" T001 "), "the document's own id: {out}");
    assert!(
        !out.contains("T-T001"),
        "and not the checklist default: {out}"
    );
    assert!(
        out.contains("will collide with these and be refused"),
        "the report says what unqualified costs: {out}"
    );
}

/// Two features in one repository write the same ids, and an import that did not
/// qualify them would collide on the second one.
#[test]
fn the_same_ids_from_two_features_do_not_collide() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), "one.md", SPEC_KIT);
    std::fs::write(dir.path().join("two.md"), SPEC_KIT).expect("write");

    let (_, code) = aplan(
        dir.path(),
        &[
            "import",
            "one.md",
            "--format",
            "spec-kit",
            "--id-prefix",
            "F1",
        ],
    );
    assert_eq!(code, 0);
    let (out, code) = aplan(
        dir.path(),
        &[
            "import",
            "two.md",
            "--format",
            "spec-kit",
            "--id-prefix",
            "F2",
        ],
    );
    assert_eq!(code, 0, "{out}");

    let (check, _) = aplan(dir.path(), &["check"]);
    assert!(check.contains("6 tasks"), "both features landed: {check}");
    // Re-importing under a prefix already used meets the ids it wrote last time,
    // and the command refuses rather than overwriting.
    let (out, code) = aplan(
        dir.path(),
        &[
            "import",
            "two.md",
            "--format",
            "spec-kit",
            "--id-prefix",
            "F1",
        ],
    );
    assert_ne!(code, 0, "a colliding import must be refused: {out}");
    assert!(out.contains("already in this plan"), "{out}");
}

fn fixture(dir: &Path, name: &str, contents: &str) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.join(name), contents).expect("source");
}
