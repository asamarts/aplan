//! `aplan explain`: a human answer for one name in an enumerable vocabulary
//! (`A1-198`).
//!
//! The rule catalogue is already the reviewable record of what every rule
//! means. This surface must expose that meaning, not grow a parallel body of
//! prose that agrees only until one of them changes.

use std::path::Path;

use crate::common;

fn run(dir: &Path, args: &[&str]) -> common::Run {
    common::run(dir, args)
}

fn catalogue() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/rule-catalogue.md");
    std::fs::read_to_string(path).expect("rule catalogue")
}

fn catalogue_heading(rule: aplan_core::Rule) -> String {
    format!(
        "{} ({})\n\n{}\n",
        rule.code(),
        rule.severity(),
        rule.title()
    )
}

#[test]
fn every_rule_is_explained_with_the_catalogues_own_heading() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let catalogue = catalogue();

    for &rule in aplan_core::Rule::ALL {
        let run = run(dir.path(), &["explain", "rule", rule.code()]);
        let expected = catalogue_heading(rule);
        assert!(run.ok(), "{} was not explained: {run:?}", rule.code());
        assert_eq!(run.stdout, expected, "{} changed meaning", rule.code());
        assert!(
            catalogue.contains(&format!("## {expected}")),
            "{}'s answer is not the heading already carried by the catalogue",
            rule.code()
        );
        assert!(run.stderr.is_empty(), "unexpected stderr: {run:?}");
    }
}

#[test]
fn the_vocabulary_is_explicit_and_show_still_means_a_task() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(
        dir.path().join("aplan/tasks/APLAN012.md"),
        "---\nid: APLAN012\ntitle: A task whose id is also a rule code\nphase: 1\n\
         status: todo\neffort_days: 1\ntags: [cross-cutting]\n---\n",
    )
    .expect("task");

    let shown = run(dir.path(), &["show", "APLAN012", "--no-resolve"]);
    assert!(shown.ok(), "show stopped accepting the task: {shown:?}");
    assert!(shown.stdout.contains("A task whose id is also a rule code"));
    assert!(
        !shown
            .stdout
            .contains(aplan_core::Rule::CommitIrrelevant.title())
    );

    let explained = run(dir.path(), &["explain", "rule", "APLAN012"]);
    assert!(explained.ok(), "the rule was not explained: {explained:?}");
    assert_eq!(
        explained.stdout,
        catalogue_heading(aplan_core::Rule::CommitIrrelevant)
    );
    assert!(
        !explained
            .stdout
            .contains("A task whose id is also a rule code")
    );

    let untyped = run(dir.path(), &["explain", "APLAN012"]);
    assert_eq!(untyped.code, 2, "an untyped name was guessed: {untyped:?}");
    assert!(untyped.stderr.contains("invalid value 'APLAN012'"));
    assert!(untyped.stderr.contains("rule"));
}

#[test]
fn configuration_selectors_are_ignored_and_the_help_says_so() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::write(dir.path().join(".aplan.yml"), "not: [valid").expect("broken config");

    let absent_config = dir.path().join("absent.yml");
    let absent_root = dir.path().join("absent-plan");
    let absent_config = absent_config.to_str().expect("utf-8 fixture path");
    let absent_root = absent_root.to_str().expect("utf-8 fixture path");
    for args in [
        vec!["explain", "rule", "APLAN012"],
        vec!["--config", absent_config, "explain", "rule", "APLAN012"],
        vec!["--no-config", "explain", "rule", "APLAN012"],
        vec!["--root", absent_root, "explain", "rule", "APLAN012"],
    ] {
        let run = run(dir.path(), &args);
        assert!(
            run.ok(),
            "explanation consulted its surroundings for {args:?}: {run:?}"
        );
        assert_eq!(
            run.stdout,
            catalogue_heading(aplan_core::Rule::CommitIrrelevant),
            "different answer under {args:?}"
        );
        assert!(
            run.stderr.is_empty(),
            "a config-free explanation warned about configuration under {args:?}: {run:?}"
        );
    }

    let help = run(dir.path(), &["explain", "--help"]);
    assert!(help.ok(), "explain help failed: {help:?}");
    assert!(
        help.stdout.contains("For a command that reads")
            && help.stdout.contains("Commands that need no configuration"),
        "the inherited selector still promises every missing file is an error:\n{}",
        help.stdout
    );
    assert!(help.stdout.contains("`explain`"), "{}", help.stdout);
}

#[test]
fn an_unknown_rule_is_refused_with_the_copyable_vocabulary() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");

    let run = run(dir.path(), &["explain", "rule", "APLAN999"]);
    assert_eq!(run.code, 2, "an unknown rule was accepted: {run:?}");
    assert!(run.stderr.contains("unknown rule `APLAN999`"), "{run:?}");
    assert!(run.stderr.contains("APLAN001"), "{run:?}");
    assert!(run.stderr.contains("APLAN055"), "{run:?}");
    assert!(
        run.stdout.is_empty(),
        "a refusal printed an answer: {run:?}"
    );
}
