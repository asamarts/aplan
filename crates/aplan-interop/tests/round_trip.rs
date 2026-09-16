//! End to end: a real `kep.yaml` becomes task files that the actual loader
//! reads back and the actual validator accepts.
//!
//! This is A1-019's round-trip criterion, and the only test that can prove the
//! claim ADR-0012 makes about import output being valid: unit tests assert what
//! the mapping *intends*, but only running the emitted files through
//! [`load_plan`] and [`validate`] shows the plan `aplan check` would actually
//! see.

use aplan_core::{Status, load_plan, validate};
use aplan_interop::{ImplementedAs, Options, ProposalSource, emit, kep::Kep};

const KEP: &str = r"
title: Graduate the widget API to stable
kep-number: 3063
authors:
  - '@alice'
owning-sig: sig-node
status: implementable
creation-date: 2024-01-15
approvers:
  - '@dave'
stage: beta
milestone:
  beta: v1.31
";

const IMPLEMENTED_KEP: &str = r"
title: A shipped enhancement
kep-number: 1287
authors:
  - '@bob'
status: implemented
stage: stable
";

/// Import `sources` into a fresh plan directory and return the loaded,
/// validated result exactly as `aplan check` would see it.
fn import_and_check(
    sources: &[(&str, &str)],
    options: &Options,
    validate_opts: &validate::Options,
) -> (tempfile::TempDir, aplan_core::Report) {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let tasks = dir.path().join("tasks");

    for (name, contents) in sources {
        let out = Kep.parse(name, contents, options).expect("parses");
        for task in &out.tasks {
            emit::write_task(&tasks, task, &emit::provenance_body(name, Kep.flavor()))
                .expect("writes");
        }
    }

    let plan = load_plan(dir.path());
    let report = validate(&plan, validate_opts);
    (dir, report)
}

#[test]
fn an_imported_kep_loads_back_and_check_reports_no_errors() {
    let (_dir, report) = import_and_check(
        &[("kep.yaml", KEP)],
        &Options::default(),
        &validate::Options::default(),
    );

    assert_eq!(
        report.errors(),
        0,
        "a fresh import must not produce a plan that fails check: {:#?}",
        report.diagnostics
    );
    assert_eq!(report.tasks_checked, 1);
}

#[test]
fn the_round_tripped_task_keeps_everything_the_model_holds() {
    let (dir, _report) = import_and_check(
        &[("kep.yaml", KEP)],
        &Options::default(),
        &validate::Options::default(),
    );

    // Re-read through the loader, so this asserts what `aplan` parses, not what
    // the importer believed it wrote.
    let plan = load_plan(dir.path());
    let loaded = &plan.tasks[0].task;

    assert_eq!(loaded.id, "KEP-3063");
    assert_eq!(loaded.title, "Graduate the widget API to stable");
    assert_eq!(loaded.status, Status::Todo);
    assert_eq!(loaded.owner.as_deref(), Some("alice"));
    assert!(loaded.tags.contains(&"kep:implementable".to_string()));
    assert!(loaded.tags.contains(&"kep:stage:beta".to_string()));
    assert!(loaded.tags.contains(&"sig:node".to_string()));
    assert!(
        plan.load_diagnostics.is_empty(),
        "the emitted file must parse cleanly: {:#?}",
        plan.load_diagnostics
    );
}

#[test]
fn the_emitted_file_is_where_it_says_it_is() {
    let (dir, _r) = import_and_check(
        &[("kep.yaml", KEP)],
        &Options::default(),
        &validate::Options::default(),
    );
    assert!(dir.path().join("tasks/KEP-3063.md").is_file());
}

#[test]
fn a_strict_evidence_policy_still_accepts_a_default_import() {
    // ADR-0012's central claim, tested rather than asserted: because
    // "implemented" does not become `done`, the output clears APLAN017 even
    // under the strictest policy (this repository's own excludes `note`).
    let strict = validate::Options {
        require_min_strength: Some(vec![
            aplan_core::EvidenceKind::Commit,
            aplan_core::EvidenceKind::Test,
            aplan_core::EvidenceKind::File,
        ]),
        ..validate::Options::default()
    };
    let (_dir, report) = import_and_check(
        &[("kep.yaml", KEP), ("shipped.yaml", IMPLEMENTED_KEP)],
        &Options::default(),
        &strict,
    );

    assert_eq!(
        report.errors(),
        0,
        "the default mapping must satisfy any evidence policy: {:#?}",
        report.diagnostics
    );
}

#[test]
fn opting_in_to_done_is_the_documented_asymmetry() {
    // The other half of the claim: the opt-in path is weaker ON PURPOSE. It is
    // a valid plan by default, and fails only where the policy excludes `note`.
    let opted_in = Options {
        implemented_as: ImplementedAs::Done {
            as_of: "2026-07-30".to_string(),
        },
        ..Options::default()
    };

    let (_dir, lenient) = import_and_check(
        &[("shipped.yaml", IMPLEMENTED_KEP)],
        &opted_in,
        &validate::Options::default(),
    );
    assert_eq!(
        lenient.errors(),
        0,
        "a dated note satisfies APLAN005 and APLAN010: {:#?}",
        lenient.diagnostics
    );

    let strict = validate::Options {
        require_min_strength: Some(vec![aplan_core::EvidenceKind::Commit]),
        ..validate::Options::default()
    };
    let (_dir, strict_report) =
        import_and_check(&[("shipped.yaml", IMPLEMENTED_KEP)], &opted_in, &strict);
    assert!(
        strict_report
            .diagnostics
            .iter()
            .any(|d| d.code == aplan_core::code::WEAK_EVIDENCE),
        "an imported claim must not survive a strict gate: {:#?}",
        strict_report.diagnostics
    );
}

#[test]
fn importing_the_same_source_twice_refuses_rather_than_clobbers() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .unwrap();
    let tasks = dir.path().join("tasks");
    let out = Kep.parse("kep.yaml", KEP, &Options::default()).unwrap();
    let task = &out.tasks[0];

    emit::write_task(&tasks, task, "## Why\n\nfirst\n").expect("first write");
    let err = emit::write_task(&tasks, task, "## Why\n\nsecond\n").expect_err("must refuse");
    assert!(matches!(err, emit::EmitError::Exists { .. }), "{err:?}");

    let on_disk = std::fs::read_to_string(tasks.join("KEP-3063.md")).unwrap();
    assert!(on_disk.contains("first"), "the original body survived");
    assert!(!on_disk.contains("second"));
}

#[test]
fn yaml_hostile_titles_survive_the_round_trip_exactly() {
    // Imported text is someone else's, so it will eventually contain every
    // character that means something to YAML. Each of these would corrupt the
    // frontmatter if it were pasted in unquoted; serialising from `Task`
    // through serde is what makes them safe, and this pins that.
    for title in [
        "Graduate the API: the sequel",
        "Fix #123 the thing",
        r#"The "widget" API"#,
        "Support {templated} names",
        "- leading dash",
        // Bare `yes` is a boolean in YAML 1.1; it must come back as a string.
        "yes",
        "a: [b, c]",
        "trailing spaces   ",
        "* asterisk & ampersand",
    ] {
        let escaped = title.replace('\'', "''");
        let src = format!("title: '{escaped}'\nkep-number: 7\nstatus: provisional\n");
        let out = Kep
            .parse("k.yaml", &src, &Options::default())
            .expect("parse");

        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
        let tasks = dir.path().join("tasks");
        emit::write_task(&tasks, &out.tasks[0], "## Why\n\nx\n").expect("write");

        let plan = load_plan(dir.path());
        assert!(
            plan.load_diagnostics.is_empty(),
            "title {title:?} broke the emitted frontmatter: {:#?}",
            plan.load_diagnostics
        );
        assert_eq!(
            plan.tasks[0].task.title,
            title.trim_end(),
            "title {title:?} did not survive the round trip"
        );
        let report = validate(&plan, &validate::Options::default());
        assert_eq!(report.errors(), 0, "{:#?}", report.diagnostics);
    }
}

#[test]
fn a_fresh_import_warns_about_the_estimate_it_cannot_invent() {
    // Documented consequence rather than a defect: import has no `effort_days`
    // to draw on and will not fabricate one, so `APLAN009` warns. It is a
    // warning, so `check` still exits 0, and the command layer says so once.
    let (_dir, report) = import_and_check(
        &[("kep.yaml", KEP)],
        &Options::default(),
        &validate::Options::default(),
    );

    assert_eq!(report.errors(), 0);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == aplan_core::code::MISSING_EFFORT),
        "expected the missing-effort warning: {:#?}",
        report.diagnostics
    );
}
