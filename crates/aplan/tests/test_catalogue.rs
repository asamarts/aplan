//! Configured test-evidence discovery (`A1-478`).

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use crate::common;

fn write_executable(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write runner");
    let mut permissions = path.metadata().expect("runner metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("make runner executable");
}

fn basic_plan(dir: &Path, config: &str) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(dir.join("src")).expect("source");
    std::fs::write(dir.join("src/lib.rs"), "pub fn covered() {}\n").expect("source");
    std::fs::write(dir.join(".aplan.yml"), config).expect("config");
    std::fs::write(
        dir.join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: exact test evidence\nphase: 1\nstatus: in_progress\n\
         effort_days: 1\npaths: [src/**]\ntags: [cross-cutting]\n---\n\n## Acceptance criteria\n\n\
         - [x] covered\n",
    )
    .expect("task");
}

fn catalogue_fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("python")).expect("python runner directory");
    std::fs::create_dir_all(dir.join("rust")).expect("rust runner directory");
    write_executable(
        &dir.join("python/list.sh"),
        "#!/bin/sh\npwd >> ../python-seen.txt\nprintf '%s\\n' 'shared::case: test' 'package.tests.WidgetTests.test_handles_case' 'package.tests.WidgetTests.test_handles_case'\nprintf '\\033[31mhostile\\tfield\\n'\n",
    );
    write_executable(
        &dir.join("rust/list.sh"),
        "#!/bin/sh\npwd >> ../rust-seen.txt\nprintf '%s\\n' 'zeta: test' 'shared::case: test'\n",
    );
    basic_plan(
        dir,
        "root: aplan\ntest:\n\
         - name: python-unittest\n  dir: python\n  list: [sh, list.sh]\n  run: ['true', '{test}']\n\
         - name: rust-unit\n  dir: rust\n  list: [sh, list.sh]\n  run: ['true', '{test}']\n",
    );
}

fn json(dir: &Path, args: &[&str]) -> (common::Run, serde_json::Value) {
    let run = common::run(dir, args);
    let value = serde_json::from_str(&run.stdout)
        .unwrap_or_else(|error| panic!("JSON output ({error}): {}", run.text()));
    (run, value)
}

#[test]
fn configured_catalogue_searches_exact_names_and_exposes_ambiguity_deterministically() {
    let dir = tempfile::Builder::new()
        .prefix("test-catalogue-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    catalogue_fixture(dir.path());

    let (found, document) = json(
        dir.path(),
        &["test", "list", "HANDLES_case", "--format", "json"],
    );
    assert_eq!(found.code, 0, "{}", found.text());
    assert_eq!(document["schema"], "aplan/test-catalogue/v1");
    let catalogue = &document["catalogue"];
    assert_eq!(catalogue["state"], "complete");
    assert_eq!(catalogue["summary"]["runners"], 2);
    assert_eq!(catalogue["summary"]["matched_names"], 1);
    assert_eq!(catalogue["summary"]["matched_pairs"], 1);
    assert_eq!(catalogue["runners"][0]["runner"]["index"], 1);
    assert_eq!(
        catalogue["runners"][0]["runner"]["label"],
        "python-unittest"
    );
    assert_eq!(
        catalogue["runners"][0]["tests"][0]["evidence"],
        "test:package.tests.WidgetTests.test_handles_case"
    );

    let format_before_subcommand = common::run(
        dir.path(),
        &["test", "--format", "json", "list", "HANDLES_case"],
    );
    assert_eq!(
        format_before_subcommand.code,
        0,
        "{}",
        format_before_subcommand.text()
    );
    serde_json::from_str::<serde_json::Value>(&format_before_subcommand.stdout)
        .expect("the pre-existing global option order remains accepted");

    let first = common::run(dir.path(), &["test", "list", "shared", "--format", "agent"]);
    let second = common::run(dir.path(), &["test", "list", "shared", "--format", "agent"]);
    assert_eq!(first.code, 0, "{}", first.text());
    assert_eq!(
        first.stdout, second.stdout,
        "machine output is not deterministic"
    );
    let agent: serde_json::Value = serde_json::from_str(&first.stdout).expect("agent JSON");
    assert_eq!(agent["schema"], "aplan/test-catalogue-agent/v1");
    assert!(agent["agent_instruction"].as_str().is_some());
    assert_eq!(agent["catalogue"]["summary"]["ambiguous_names"], 1);
    for runner in agent["catalogue"]["runners"].as_array().expect("runners") {
        let test = &runner["tests"][0];
        assert_eq!(test["name"], "shared::case");
        assert_eq!(test["ambiguous"], true);
        assert_eq!(test["listed_by"].as_array().expect("owners").len(), 2);
    }

    let human = common::run(dir.path(), &["test", "list"]);
    assert_eq!(human.code, 0, "{}", human.text());
    assert!(
        !human.stdout.contains('\u{1b}'),
        "raw terminal control: {}",
        human.stdout
    );
    assert!(
        !human.stdout.contains('\t'),
        "raw terminal tab: {}",
        human.stdout
    );
    assert!(
        human.stdout.contains("\u{fffd}[31mhostile field"),
        "{}",
        human.stdout
    );
    assert!(
        human.stdout.contains("not valid evidence"),
        "{}",
        human.stdout
    );

    let python_dir = dir
        .path()
        .join("python")
        .canonicalize()
        .expect("python path");
    let rust_dir = dir.path().join("rust").canonicalize().expect("rust path");
    let python_seen = std::fs::read_to_string(dir.path().join("python-seen.txt")).expect("seen");
    let rust_seen = std::fs::read_to_string(dir.path().join("rust-seen.txt")).expect("seen");
    assert!(
        python_seen
            .lines()
            .all(|line| line == python_dir.to_string_lossy())
    );
    assert!(
        rust_seen
            .lines()
            .all(|line| line == rust_dir.to_string_lossy())
    );
}

#[test]
fn catalogue_distinguishes_unconfigured_from_an_empty_runner() {
    let unconfigured = tempfile::Builder::new()
        .prefix("test-catalogue-none-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    basic_plan(unconfigured.path(), "root: aplan\n");
    let (run, value) = json(unconfigured.path(), &["test", "list", "--format", "json"]);
    assert_eq!(run.code, 0, "{}", run.text());
    assert_eq!(value["catalogue"]["state"], "unconfigured");
    let human = common::run(unconfigured.path(), &["test", "list"]);
    assert_eq!(human.code, 0, "{}", human.text());
    assert!(human.stdout.contains("no test runners are configured"));
    let (agent, value) = json(unconfigured.path(), &["test", "list", "--format", "agent"]);
    assert_eq!(agent.code, 0, "{}", agent.text());
    assert!(
        value["agent_instruction"]
            .as_str()
            .is_some_and(|text| text.contains("Configure a trusted test runner"))
    );

    let empty = tempfile::Builder::new()
        .prefix("test-catalogue-empty-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    basic_plan(
        empty.path(),
        "root: aplan\ntest:\n  name: empty-suite\n  list: ['true']\n  run: ['true', '{test}']\n",
    );
    let (run, value) = json(empty.path(), &["test", "list", "--format", "json"]);
    assert_eq!(run.code, 0, "{}", run.text());
    assert_eq!(value["catalogue"]["state"], "complete");
    assert_eq!(value["catalogue"]["runners"][0]["state"], "empty");
    assert_eq!(value["catalogue"]["findings"][0]["code"], "APLAN036");
    let human = common::run(empty.path(), &["test", "list"]);
    assert_eq!(human.code, 0, "{}", human.text());
    assert!(human.stdout.contains("runner #1 `empty-suite`: empty"));
    assert!(human.stdout.contains("warning[APLAN036]"));
    std::fs::write(
        empty.path().join(".aplan.yml"),
        "root: aplan\ndeny_warnings: true\ntest:\n  name: empty-suite\n  list: ['true']\n  run: ['true', '{test}']\n",
    )
    .expect("strict config");
    let (run, value) = json(empty.path(), &["test", "list", "--format", "json"]);
    assert_eq!(run.code, 1, "a denied warning must fail: {}", run.text());
    assert_eq!(value["catalogue"]["state"], "complete");
    assert_eq!(value["catalogue"]["denied"], true);
}

#[test]
fn catalogue_distinguishes_failed_partial_answers_from_complete_no_matches() {
    let failed = tempfile::Builder::new()
        .prefix("test-catalogue-failed-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    basic_plan(
        failed.path(),
        "root: aplan\ntest:\n  name: broken-suite\n  list: ['false']\n  run: ['true', '{test}']\n",
    );
    let (run, value) = json(
        failed.path(),
        &["test", "list", "missing", "--format", "json"],
    );
    assert_eq!(run.code, 1, "{}", run.text());
    assert_eq!(value["catalogue"]["state"], "partial");
    assert_eq!(value["catalogue"]["runners"][0]["state"], "failed");
    assert_eq!(value["catalogue"]["findings"][0]["code"], "APLAN013");
    let finding = value["catalogue"]["findings"][0]["message"]
        .as_str()
        .expect("finding message");
    assert!(finding.contains("broken-suite") && finding.contains("`false` exited 1"));
    assert!(
        value["catalogue"]["findings"][0]["file"]
            .as_str()
            .is_some_and(|file| file.ends_with(".aplan.yml"))
    );
    let human = common::run(failed.path(), &["test", "list", "missing"]);
    assert_eq!(human.code, 1, "{}", human.text());
    assert!(human.stdout.contains("partial - 0 runners answered"));
    assert!(
        human
            .stdout
            .contains("runner #1 `broken-suite`: failed (see finding below)")
    );
    assert!(human.stdout.contains("error[APLAN013]"));
    assert!(human.stdout.contains("at:"));
    assert!(human.stdout.contains("fix:"));
    let (agent, value) = json(
        failed.path(),
        &["test", "list", "missing", "--format", "agent"],
    );
    assert_eq!(agent.code, 1, "{}", agent.text());
    assert!(
        value["agent_instruction"]
            .as_str()
            .is_some_and(|text| text.contains("Fix every runner finding"))
    );
    std::fs::write(
        failed.path().join(".aplan.yml"),
        "root: aplan\nadvisory: [APLAN013]\ntest:\n  name: broken-suite\n  list: ['false']\n  run: ['true', '{test}']\n",
    )
    .expect("advisory config");
    let (run, value) = json(
        failed.path(),
        &["test", "list", "missing", "--format", "json"],
    );
    assert_eq!(
        run.code,
        0,
        "an explicit advisory is non-blocking: {}",
        run.text()
    );
    assert_eq!(value["catalogue"]["state"], "partial");
    assert_eq!(value["catalogue"]["denied"], false);
    assert_eq!(value["catalogue"]["findings"][0]["advisory"], true);

    let complete = tempfile::Builder::new()
        .prefix("test-catalogue-miss-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    catalogue_fixture(complete.path());
    let (run, value) = json(
        complete.path(),
        &["test", "list", "does-not-exist", "--format", "json"],
    );
    assert_eq!(run.code, 0, "{}", run.text());
    assert_eq!(value["catalogue"]["state"], "no_match");
    let human = common::run(complete.path(), &["test", "list", "does-not-exist"]);
    assert_eq!(human.code, 0, "{}", human.text());
    assert!(human.stdout.contains("no names match `does-not-exist`"));
    let (agent, value) = json(
        complete.path(),
        &["test", "list", "does-not-exist", "--format", "agent"],
    );
    assert_eq!(agent.code, 0, "{}", agent.text());
    assert!(
        value["agent_instruction"]
            .as_str()
            .is_some_and(|text| text.contains("Broaden the search"))
    );

    let blank = common::run(complete.path(), &["test", "list", " \t"]);
    assert_eq!(blank.code, 2, "{}", blank.text());
    assert!(blank.text().contains("must contain a visible character"));
}

#[test]
fn close_points_at_configured_test_discovery() {
    // Proof for `evidence.test_unresolved_points_to_catalogue`: a source-level
    // Python function is not the class-qualified name unittest reports, and
    // the refusal must tell the caller how to discover that exact namespace.
    const BEHAVIOUR: &str = "evidence.test_unresolved_points_to_catalogue";
    let dir = tempfile::Builder::new()
        .prefix("test-catalogue-close-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    catalogue_fixture(dir.path());
    common::git(dir.path(), &["init", "-q", "."]);
    common::git(dir.path(), &["config", "user.email", "t@example.invalid"]);
    common::git(dir.path(), &["config", "user.name", "t"]);
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-q", "-m", "fixture"]);

    let refused = common::run(
        dir.path(),
        &["close", "T-1", "--evidence", "test:test_handles_case"],
    );
    assert_ne!(refused.code, 0, "{BEHAVIOUR}: {}", refused.text());
    assert!(
        refused.text().contains("APLAN013"),
        "{BEHAVIOUR}: {}",
        refused.text()
    );
    assert!(
        refused.text().contains("aplan test list"),
        "{BEHAVIOUR}: {}",
        refused.text()
    );

    let ambiguous = common::run(
        dir.path(),
        &["close", "T-1", "--evidence", "test:shared::case"],
    );
    assert_ne!(ambiguous.code, 0, "{BEHAVIOUR}: {}", ambiguous.text());
    assert!(
        ambiguous.text().contains("APLAN013"),
        "{BEHAVIOUR}: {}",
        ambiguous.text()
    );
    assert!(
        ambiguous.text().contains("aplan test list"),
        "{BEHAVIOUR}: {}",
        ambiguous.text()
    );
}

#[test]
fn an_external_policy_still_runs_catalogue_runners_in_the_reviewed_repository() {
    let repository = tempfile::Builder::new()
        .prefix("test-catalogue-repository-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("repository");
    let policy = tempfile::Builder::new()
        .prefix("test-catalogue-policy-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("policy");
    catalogue_fixture(repository.path());
    let selected = policy.path().join("trusted.yml");
    std::fs::write(
        &selected,
        "root: nowhere-near-the-plan\ntest:\n  name: reviewed-python\n  dir: python\n  \
         list: [sh, list.sh]\n  run: ['true', '{test}']\n",
    )
    .expect("trusted policy");

    let selected = selected.display().to_string();
    let run = common::run(
        repository.path(),
        &[
            "--root",
            "aplan",
            "--config",
            &selected,
            "test",
            "list",
            "handles_case",
            "--format",
            "json",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.text());
    let value: serde_json::Value = serde_json::from_str(&run.stdout).expect("catalogue JSON");
    assert_eq!(
        value["catalogue"]["runners"][0]["tests"][0]["evidence"],
        "test:package.tests.WidgetTests.test_handles_case"
    );
    assert!(
        !policy.path().join("python-seen.txt").exists(),
        "runner dir was resolved against the trusted policy"
    );
    assert!(repository.path().join("python-seen.txt").is_file());
}

fn workspace_root() -> std::path::PathBuf {
    let inherited = std::env::var_os("PWD").map(std::path::PathBuf::from);
    inherited
        .into_iter()
        .chain(std::env::current_dir())
        .find(|candidate| candidate.join("crates/aplan/src/lib.rs").is_file())
        .expect("test process retains the workspace path")
}

fn production_hit_count(relative: &str, needle: &str) -> usize {
    let path = workspace_root().join(relative);
    let source = std::fs::read_to_string(&path).expect("read production source");
    common::sources::production_lines(&source)
        .into_iter()
        .filter(|(_, text)| text.contains(needle))
        .count()
}

#[test]
fn configured_test_catalogue_has_one_resolver_owner_and_every_client_is_dispositioned() {
    assert_eq!(
        production_hit_count(
            "crates/aplan-app/src/test_catalogue.rs",
            "aplan_evidence::configured_test_catalogue_bounded("
        ),
        1,
        "the application catalogue query must have one adapter over evidence discovery"
    );
    assert_eq!(
        production_hit_count("crates/aplan/src/lib.rs", "aplan_app::test_catalogue("),
        1,
        "the CLI quiet path must call the typed application query exactly once"
    );
    assert_eq!(
        production_hit_count(
            "crates/aplan/src/lib.rs",
            "aplan_app::test_catalogue_with_progress("
        ),
        1,
        "the CLI observed path must call the progress-aware application query exactly once"
    );
    for constructor in [
        "TestResolver::new(",
        "TestResolver::new_scheduled(",
        "TestResolver::from_catalogue(",
    ] {
        assert_eq!(
            production_hit_count("crates/aplan-evidence/src/lib.rs", constructor),
            1,
            "only one evidence-crate entry point may use `{constructor}`"
        );
    }

    let root = workspace_root();
    let tui = root.join("crates/aplan-tui/src");
    let forbidden = [
        "TestResolver",
        "configured_test_catalogue",
        "run_configured_command",
    ];
    for source in common::sources::all_sources(&root)
        .into_iter()
        .filter(|source| source.starts_with(&tui))
    {
        let text = std::fs::read_to_string(&source).expect("read TUI source");
        for token in forbidden {
            assert!(
                !common::sources::production_lines(&text)
                    .into_iter()
                    .any(|(_, line)| line.contains(token)),
                "{} bypasses the application query through `{token}`",
                source.display()
            );
        }
    }

    assert_eq!(
        production_hit_count(
            "crates/aplan-tui/src/evidence.rs",
            "aplan_app::test_catalogue("
        ),
        1,
        "the delivered TUI client must use the shared application query exactly once"
    );
}

fn receipt_fixture(dir: &Path, mutate_source: bool) {
    let mutation = if mutate_source {
        "printf 'changed\\n' > src/lib.rs; "
    } else {
        ""
    };
    basic_plan(
        dir,
        "root: aplan\nevidence:\n  observation_inputs: [.aplan.yml, runner.sh, 'src/**']\n\
         test:\n  name: unit\n  list: [sh, runner.sh, list]\n  run: [sh, runner.sh, run, '{test}']\n  run_selects: true\n",
    );
    write_executable(
        &dir.join("runner.sh"),
        &format!(
            "#!/bin/sh\ncase \"$1\" in\n  list) printf '%s\\n' alpha beta ;;\n  run) {mutation}test \"$2\" = alpha -o \"$2\" = beta ;;\n  *) exit 2 ;;\nesac\n"
        ),
    );
    common::git(dir, &["init", "-q", "."]);
    common::git(dir, &["config", "user.email", "t@example.invalid"]);
    common::git(dir, &["config", "user.name", "t"]);
    common::git(dir, &["add", "-A"]);
    common::git(dir, &["commit", "-q", "-m", "receipt fixture"]);
}

fn receipt_skip_fixture(dir: &Path, skip_query_succeeds: bool) {
    basic_plan(
        dir,
        "root: aplan\nevidence:\n  observation_inputs: [.aplan.yml, runner.sh, 'src/**']\n\
         test:\n  name: unit\n  list: [sh, runner.sh, list]\n  skipped: [sh, runner.sh, skipped]\n  run: [sh, runner.sh, run, '{test}']\n  run_selects: true\n",
    );
    let skipped = if skip_query_succeeds {
        "printf '%s\\n' beta"
    } else {
        "exit 23"
    };
    write_executable(
        &dir.join("runner.sh"),
        &format!(
            "#!/bin/sh\ncase \"$1\" in\n  list) printf '%s\\n' alpha beta ;;\n  skipped) {skipped} ;;\n  run) test \"$2\" = alpha -o \"$2\" = beta ;;\n  *) exit 2 ;;\nesac\n"
        ),
    );
    common::git(dir, &["init", "-q", "."]);
    common::git(dir, &["config", "user.email", "t@example.invalid"]);
    common::git(dir, &["config", "user.name", "t"]);
    common::git(dir, &["add", "-A"]);
    common::git(dir, &["commit", "-q", "-m", "receipt skip fixture"]);
}

fn receipt_args(output: &str) -> Vec<&str> {
    vec![
        "test",
        "receipt",
        "unit",
        "--output",
        output,
        "--producer-repository",
        "owner/repository",
        "--producer-workflow",
        "checks.yml:unit",
        "--producer-run-id",
        "17",
        "--producer-event",
        "push",
        "--producer-privilege",
        "trusted",
    ]
}

/// Proof for `evidence.portable_provider_receipts`.
#[test]
fn receipt_command_exports_a_complete_plan_independent_provider_statement() {
    let dir = tempfile::Builder::new()
        .prefix("test-receipt-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    receipt_fixture(dir.path(), false);
    let run = common::run(dir.path(), &receipt_args("-"));
    assert_eq!(run.code, 0, "{}", run.text());
    assert!(run.stderr.is_empty(), "{}", run.text());
    let value: serde_json::Value = serde_json::from_str(&run.stdout).expect("receipt JSON");
    assert_eq!(value["_type"], "https://in-toto.io/Statement/v1");
    assert_eq!(
        value["predicateType"],
        "https://aplan.dev/attestation/test-evidence/v1"
    );
    assert!(value["predicate"].get("plan").is_none());
    assert_eq!(value["predicate"]["provider"]["index"], 1);
    assert_eq!(value["predicate"]["provider"]["label"], "unit");
    assert_eq!(
        value["predicate"]["catalogue"]["names"],
        serde_json::json!(["alpha", "beta"])
    );
    assert_eq!(
        value["predicate"]["requested"],
        serde_json::json!(["alpha", "beta"])
    );
    assert_eq!(value["predicate"]["outcomes"][0]["outcome"], "passed");
    assert_eq!(value["predicate"]["outcomes"][1]["outcome"], "passed");
    assert_eq!(value["predicate"]["complete"], true);
}

#[test]
fn receipt_command_publishes_atomically_and_refuses_material_mutation() {
    let good = tempfile::Builder::new()
        .prefix("test-receipt-file-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    receipt_fixture(good.path(), false);
    let output = good.path().join("receipt.json");
    std::fs::write(&output, "old").expect("old destination");
    let output_text = output.to_string_lossy().to_string();
    let run = common::run(good.path(), &receipt_args(&output_text));
    assert_eq!(run.code, 0, "{}", run.text());
    let written = std::fs::read_to_string(&output).expect("receipt");
    assert!(written.ends_with('\n'));
    serde_json::from_str::<serde_json::Value>(&written).expect("complete JSON");

    let changing = tempfile::Builder::new()
        .prefix("test-receipt-changing-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    receipt_fixture(changing.path(), true);
    let output = changing.path().join("receipt.json");
    std::fs::write(&output, "old").expect("old destination");
    let output_text = output.to_string_lossy().to_string();
    let run = common::run(changing.path(), &receipt_args(&output_text));
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(
        run.text().contains("material changed during execution"),
        "{}",
        run.text()
    );
    assert_eq!(
        std::fs::read_to_string(output).expect("old survives"),
        "old"
    );
}

#[test]
fn receipt_command_preserves_skips_and_refuses_a_failed_skip_query() {
    let skipped = tempfile::Builder::new()
        .prefix("test-receipt-skipped-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    receipt_skip_fixture(skipped.path(), true);
    let run = common::run(skipped.path(), &receipt_args("-"));
    assert_eq!(run.code, 0, "{}", run.text());
    let value: serde_json::Value = serde_json::from_str(&run.stdout).expect("receipt JSON");
    assert_eq!(value["predicate"]["outcomes"][0]["test"], "alpha");
    assert_eq!(value["predicate"]["outcomes"][0]["outcome"], "passed");
    assert_eq!(value["predicate"]["outcomes"][1]["test"], "beta");
    assert_eq!(value["predicate"]["outcomes"][1]["outcome"], "skipped");

    let failed = tempfile::Builder::new()
        .prefix("test-receipt-skip-failed-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    receipt_skip_fixture(failed.path(), false);
    let output = failed.path().join("receipt.json");
    std::fs::write(&output, "old").expect("old destination");
    let output_text = output.to_string_lossy().to_string();
    let run = common::run(failed.path(), &receipt_args(&output_text));
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(run.text().contains("incomplete"), "{}", run.text());
    assert_eq!(
        std::fs::read_to_string(output).expect("old survives"),
        "old"
    );
}

#[test]
fn receipt_command_refuses_unknown_providers_and_missing_material_claims_before_execution() {
    let dir = tempfile::Builder::new()
        .prefix("test-receipt-refusal-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    receipt_fixture(dir.path(), false);
    let mut unknown = receipt_args("-");
    unknown[2] = "absent";
    let run = common::run(dir.path(), &unknown);
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(
        run.text().contains("matches no configured runner"),
        "{}",
        run.text()
    );

    let mut invalid_producer = receipt_args("-");
    invalid_producer[10] = " ";
    std::fs::remove_file(dir.path().join("runner.sh")).expect("make execution impossible");
    let run = common::run(dir.path(), &invalid_producer);
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(
        run.text().contains("producer run id is absent or blank"),
        "{}",
        run.text()
    );
    common::git(dir.path(), &["checkout", "--", "runner.sh"]);

    let mut meaningless_format = receipt_args("-");
    meaningless_format.insert(1, "--format");
    meaningless_format.insert(2, "json");
    let run = common::run(dir.path(), &meaningless_format);
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(
        run.text().contains("receipts always use canonical JSON"),
        "{}",
        run.text()
    );

    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ntest:\n  name: unit\n  list: [sh, runner.sh, list]\n  run: [sh, runner.sh, run, '{test}']\n  run_selects: true\n",
    )
    .expect("configuration without material claim");
    let run = common::run(dir.path(), &receipt_args("-"));
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(
        run.text()
            .contains("requires exhaustive `evidence.observation_inputs`"),
        "{}",
        run.text()
    );
}

/// Proof for `evidence.receipt_backed_whole_plan_resolution` and
/// `evidence.verification_reports_provider_provenance`.
#[test]
fn verify_imports_current_receipts_without_starting_the_provider() {
    let dir = tempfile::Builder::new()
        .prefix("test-receipt-verify-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    receipt_fixture(dir.path(), false);
    std::fs::write(dir.path().join(".gitignore"), "provider.log\n").expect("ignore log");
    write_executable(
        &dir.path().join("runner.sh"),
        "#!/bin/sh\nprintf '%s:%s\n' \"$1\" \"${2:-}\" >> provider.log\ncase \"$1\" in\n  list) printf '%s\n' alpha beta ;;\n  run) test \"$2\" = alpha -o \"$2\" = beta ;;\n  *) exit 2 ;;\nesac\n",
    );
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: exact test evidence\nphase: 1\nstatus: done\n\
         effort_days: 1\npaths: [src/**]\ntags: [cross-cutting]\nevidence:\n\
         - kind: test\n  ref: alpha\n---\n\n## Acceptance criteria\n\n- [x] covered\n",
    )
    .expect("closed task");
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-q", "-m", "receipt consumer"]);

    let receipt = dir.path().join("receipt.json");
    let receipt_text = receipt.to_string_lossy().to_string();
    let produced = common::run(dir.path(), &receipt_args(&receipt_text));
    assert_eq!(produced.code, 0, "{}", produced.text());
    let before = std::fs::read_to_string(dir.path().join("provider.log")).expect("provider log");
    let import_set = dir.path().join("receipts.json");
    std::fs::write(
        &import_set,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "aplan/test-evidence-import-set/v1",
            "receipts": [{
                "provider": "unit",
                "path": "receipt.json",
                "trust": {"mode": "local_file"}
            }]
        }))
        .expect("import set"),
    )
    .expect("write import set");

    let verified = common::run(
        dir.path(),
        &[
            "--timings",
            "verify",
            "--test-receipts",
            import_set.to_str().expect("UTF-8 fixture path"),
            "--format",
            "json",
        ],
    );
    assert_eq!(verified.code, 0, "{}", verified.text());
    let document: serde_json::Value = serde_json::from_str(&verified.stdout).expect("verify JSON");
    assert_eq!(document["evidence_observation"]["scope"], "whole_plan");
    assert_eq!(
        document["evidence_observation"]["provider_source"],
        "receipt"
    );
    assert_eq!(document["evidence_observation"]["state"], "current");
    assert_eq!(document["evidence_observation"]["completeness"], "complete");
    assert!(
        verified.stderr.contains("0 test-provider spawns"),
        "{}",
        verified.text()
    );
    let timings = verified
        .stderr
        .lines()
        .find(|line| line.starts_with("aplan: timings "))
        .expect("receipt phase timings");
    for field in [
        "receipt_manifest_ms=",
        "receipt_material_ms=",
        "receipt_parse_ms=",
        "receipt_identity_ms=",
        "receipt_authentication_ms=",
        "receipt_catalogue_ms=",
        "receipt_accepted=1",
        "receipt_rejected=0",
        "receipt_test_provider_spawns=0",
    ] {
        assert!(timings.contains(field), "missing `{field}` in {timings}");
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("provider.log")).expect("provider log"),
        before,
        "receipt-backed verify started a configured provider"
    );

    let human = common::run(
        dir.path(),
        &[
            "verify",
            "--test-receipts",
            import_set.to_str().expect("UTF-8 fixture path"),
        ],
    );
    assert_eq!(human.code, 0, "{}", human.text());
    assert!(
        human.stdout.contains(
            "evidence: scope=whole_plan, provider_source=receipt, freshness=current, completeness=complete"
        ),
        "{}",
        human.text()
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("provider.log")).expect("provider log"),
        before,
        "human receipt-backed verify started a configured provider"
    );

    std::fs::write(
        &import_set,
        serde_json::to_vec(&serde_json::json!({
            "schema": "aplan/test-evidence-import-set/v1",
            "receipts": []
        }))
        .expect("missing import set"),
    )
    .expect("write missing import set");
    let missing = common::run(
        dir.path(),
        &[
            "verify",
            "--test-receipts",
            import_set.to_str().expect("UTF-8 fixture path"),
            "--format",
            "json",
        ],
    );
    assert_eq!(missing.code, 1, "{}", missing.text());
    let missing_document: serde_json::Value =
        serde_json::from_str(&missing.stdout).expect("missing receipt JSON");
    assert_eq!(
        missing_document["evidence_observation"]["state"],
        "not_observed"
    );
    assert_eq!(
        missing_document["evidence_observation"]["provider_source"],
        "receipt"
    );
    assert_eq!(
        missing_document["evidence_observation"]["completeness"],
        "partial"
    );
    assert!(missing.stderr.contains("0 test-provider spawns"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("provider.log")).expect("provider log"),
        before,
        "missing receipt triggered provider fallback"
    );

    let conflicting = common::run(
        dir.path(),
        &[
            "verify",
            "--test-receipts",
            import_set.to_str().expect("UTF-8 fixture path"),
            "--execute",
        ],
    );
    assert_eq!(conflicting.code, 2, "{}", conflicting.text());
    assert!(conflicting.text().contains("cannot be used with"));
}

/// Proof for `evidence.incremental_receipt_refresh`.
#[test]
fn incremental_refresh_executes_only_invalidated_tests_and_rederives_the_whole_plan() {
    let dir = tempfile::Builder::new()
        .prefix("test-receipt-incremental-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    basic_plan(
        dir.path(),
        "root: aplan\nevidence:\n  observation_inputs: [.aplan.yml, runner.sh, 'src/**']\n\
         test:\n  name: unit\n  list: [sh, runner.sh, list]\n  run: [sh, runner.sh, run, '{test}']\n  run_selects: true\n  inputs_by_test:\n    - tests: [alpha]\n      paths: ['src/alpha/**']\n    - tests: [beta]\n      paths: ['src/beta/**']\n",
    );
    std::fs::create_dir_all(dir.path().join("src/alpha")).expect("alpha dir");
    std::fs::create_dir_all(dir.path().join("src/beta")).expect("beta dir");
    std::fs::write(dir.path().join("src/alpha/input"), "one\n").expect("alpha input");
    std::fs::write(dir.path().join("src/beta/input"), "one\n").expect("beta input");
    std::fs::write(dir.path().join(".gitignore"), "provider.log\nartifacts/\n")
        .expect("ignore log and local refresh artifacts");
    write_executable(
        &dir.path().join("runner.sh"),
        "#!/bin/sh\nprintf '%s:%s\n' \"$1\" \"${2:-}\" >> provider.log\ncase \"$1\" in\n  list) printf '%s\n' alpha beta ;;\n  run) test \"$2\" = alpha -o \"$2\" = beta ;;\n  *) exit 2 ;;\nesac\n",
    );
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: both exact tests\nphase: 1\nstatus: done\neffort_days: 1\n\
         paths: [src/**]\ntags: [cross-cutting]\nevidence:\n  - kind: test\n    ref: alpha\n  - kind: test\n    ref: beta\n---\n\n## Acceptance criteria\n\n- [x] covered\n",
    )
    .expect("task");
    common::git(dir.path(), &["init", "-q", "."]);
    common::git(dir.path(), &["config", "user.email", "t@example.invalid"]);
    common::git(dir.path(), &["config", "user.name", "t"]);
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-q", "-m", "base"]);
    let base = String::from_utf8(common::git(dir.path(), &["rev-parse", "HEAD"]).stdout)
        .expect("base UTF-8")
        .trim()
        .to_string();

    let artifacts = dir.path().join("artifacts");
    std::fs::create_dir(&artifacts).expect("artifacts");
    let produced = common::run(dir.path(), &receipt_args("artifacts/receipt.json"));
    assert_eq!(produced.code, 0, "{}", produced.text());
    let import_set = artifacts.join("receipts.json");
    std::fs::write(
        &import_set,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "aplan/test-evidence-import-set/v1",
            "receipts": [{
                "provider": "unit",
                "path": "receipt.json",
                "trust": {"mode": "local_file"}
            }]
        }))
        .expect("manifest"),
    )
    .expect("manifest write");

    std::fs::write(dir.path().join("src/alpha/input"), "two\n").expect("change alpha");
    common::git(dir.path(), &["add", "src/alpha/input"]);
    common::git(dir.path(), &["commit", "-q", "-m", "change alpha"]);
    let head = String::from_utf8(common::git(dir.path(), &["rev-parse", "HEAD"]).stdout)
        .expect("head UTF-8")
        .trim()
        .to_string();
    let before = std::fs::read_to_string(dir.path().join("provider.log")).expect("initial log");

    let refreshed = common::run(
        dir.path(),
        &[
            "verify",
            "--refresh-test-receipts",
            "artifacts/receipts.json",
            "--refresh-base",
            &base,
            "--refresh-head",
            &head,
            "--format",
            "json",
        ],
    );
    assert_eq!(refreshed.code, 0, "{}", refreshed.text());
    let document: serde_json::Value =
        serde_json::from_str(&refreshed.stdout).expect("refresh JSON");
    let observation = &document["evidence_observation"];
    assert_eq!(observation["scope"], "whole_plan");
    assert_eq!(observation["provider_source"], "incremental_refresh");
    assert_eq!(observation["state"], "current");
    assert_eq!(observation["completeness"], "complete");
    assert_eq!(observation["refresh"]["mode"], "base_head");
    assert_eq!(observation["refresh"]["changed_paths"], 1);
    assert_eq!(
        observation["refresh"]["refreshed"],
        serde_json::json!(["unit"])
    );
    assert_eq!(observation["refresh"]["selected_tests"], 1);
    assert_eq!(observation["refresh"]["widened_providers"], 0);
    let after = std::fs::read_to_string(dir.path().join("provider.log")).expect("refresh log");
    let delta = after.strip_prefix(&before).expect("log append only");
    assert!(delta.contains("list:"), "{delta}");
    assert!(delta.contains("run:alpha"), "{delta}");
    assert!(!delta.contains("run:beta"), "{delta}");

    let task_path = dir.path().join("aplan/tasks/T-1.md");
    let task = std::fs::read_to_string(&task_path).expect("task before plan-only edit");
    std::fs::write(
        &task_path,
        task.replace(
            "title: both exact tests",
            "title: both exact tests, reworded",
        ),
    )
    .expect("plan-only edit");
    common::git(dir.path(), &["add", "aplan/tasks/T-1.md"]);
    common::git(dir.path(), &["commit", "-q", "-m", "change plan prose"]);
    let plan_head = String::from_utf8(common::git(dir.path(), &["rev-parse", "HEAD"]).stdout)
        .expect("plan head UTF-8")
        .trim()
        .to_string();
    let plan_only = common::run(
        dir.path(),
        &[
            "verify",
            "--refresh-test-receipts",
            "artifacts/receipts.json",
            "--refresh-base",
            &head,
            "--refresh-head",
            &plan_head,
            "--format",
            "json",
        ],
    );
    assert_eq!(plan_only.code, 0, "{}", plan_only.text());
    let plan_document: serde_json::Value =
        serde_json::from_str(&plan_only.stdout).expect("plan-only JSON");
    assert_eq!(
        plan_document["evidence_observation"]["refresh"]["selected_tests"],
        0
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("provider.log")).expect("plan-only log"),
        after,
        "a plan-only edit must re-derive the verdict without rerunning provider outcomes"
    );

    let before_reuse = after.clone();
    let reused = common::run(
        dir.path(),
        &[
            "verify",
            "--refresh-test-receipts",
            "artifacts/receipts.json",
            "--refresh-base",
            &plan_head,
            "--refresh-head",
            &plan_head,
            "--format",
            "json",
        ],
    );
    assert_eq!(reused.code, 0, "{}", reused.text());
    let reused_document: serde_json::Value =
        serde_json::from_str(&reused.stdout).expect("reuse JSON");
    assert_eq!(
        reused_document["evidence_observation"]["refresh"]["reused"],
        serde_json::json!(["unit"])
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("provider.log")).expect("reuse log"),
        before_reuse,
        "a current provider must not be started"
    );

    std::fs::write(dir.path().join("src/beta/input"), "dirty\n").expect("dirty beta input");
    let before_dirty =
        std::fs::read_to_string(dir.path().join("provider.log")).expect("before dirty log");
    let dirty = common::run(
        dir.path(),
        &[
            "verify",
            "--refresh-test-receipts",
            "artifacts/receipts.json",
            "--refresh-base",
            &plan_head,
            "--refresh-working-tree",
            "--format",
            "json",
        ],
    );
    assert_eq!(dirty.code, 0, "{}", dirty.text());
    let dirty_document: serde_json::Value =
        serde_json::from_str(&dirty.stdout).expect("dirty JSON");
    assert_eq!(
        dirty_document["evidence_observation"]["refresh"]["mode"],
        "working_tree"
    );
    assert_eq!(
        dirty_document["evidence_observation"]["refresh"]["selected_tests"],
        1
    );
    let after_dirty =
        std::fs::read_to_string(dir.path().join("provider.log")).expect("after dirty log");
    let dirty_delta = after_dirty
        .strip_prefix(&before_dirty)
        .expect("dirty log append only");
    assert!(dirty_delta.contains("run:beta"), "{dirty_delta}");
    assert!(!dirty_delta.contains("run:alpha"), "{dirty_delta}");

    common::git(dir.path(), &["checkout", "--", "src/beta/input"]);
    let mut config = std::fs::read_to_string(dir.path().join(".aplan.yml")).expect("config");
    config.push_str("# policy changed\n");
    std::fs::write(dir.path().join(".aplan.yml"), config).expect("change config");
    let config_changed = common::run(
        dir.path(),
        &[
            "verify",
            "--refresh-test-receipts",
            "artifacts/receipts.json",
            "--refresh-base",
            &plan_head,
            "--refresh-working-tree",
            "--format",
            "json",
        ],
    );
    assert_eq!(config_changed.code, 0, "{}", config_changed.text());
    let config_document: serde_json::Value =
        serde_json::from_str(&config_changed.stdout).expect("config-change JSON");
    let config_refresh = &config_document["evidence_observation"]["refresh"];
    assert_eq!(config_refresh["widened"], serde_json::json!(["unit"]));
    assert_eq!(config_refresh["selected_tests"], 2);

    common::git(dir.path(), &["checkout", "--", ".aplan.yml"]);
    let restored = common::run(
        dir.path(),
        &[
            "verify",
            "--refresh-test-receipts",
            "artifacts/receipts.json",
            "--refresh-base",
            &plan_head,
            "--refresh-working-tree",
            "--format",
            "json",
        ],
    );
    assert_eq!(restored.code, 0, "{}", restored.text());

    std::fs::write(dir.path().join("new-component.input"), "unknown\n").expect("new unknown input");
    let widened = common::run(
        dir.path(),
        &[
            "verify",
            "--refresh-test-receipts",
            "artifacts/receipts.json",
            "--refresh-base",
            &plan_head,
            "--refresh-working-tree",
            "--refresh-shadow",
            "--format",
            "json",
        ],
    );
    assert_eq!(widened.code, 0, "{}", widened.text());
    let widened_document: serde_json::Value =
        serde_json::from_str(&widened.stdout).expect("widened JSON");
    let refresh = &widened_document["evidence_observation"]["refresh"];
    assert_eq!(refresh["widened"], serde_json::json!(["unit"]));
    assert_eq!(refresh["selected_tests"], 2);
    assert_eq!(refresh["shadow_compared"], true);

    let manifest_before = std::fs::read(&import_set).expect("manifest before refusal");
    let manifest: serde_json::Value =
        serde_json::from_slice(&manifest_before).expect("published manifest JSON");
    let receipt_path = manifest["receipts"][0]["path"]
        .as_str()
        .expect("published receipt path");
    std::fs::write(artifacts.join(receipt_path), "not a receipt\n").expect("corrupt prior receipt");
    let corrupt_prior = common::run(
        dir.path(),
        &[
            "verify",
            "--refresh-test-receipts",
            "artifacts/receipts.json",
            "--refresh-base",
            &plan_head,
            "--refresh-working-tree",
        ],
    );
    assert_eq!(corrupt_prior.code, 2, "{}", corrupt_prior.text());
    assert!(
        corrupt_prior
            .text()
            .contains("prior receipt cannot be trusted"),
        "{}",
        corrupt_prior.text()
    );
    assert_eq!(
        std::fs::read(&import_set).expect("manifest after refusal"),
        manifest_before,
        "a refused refresh must not publish a new manifest"
    );
}

/// Production shadow audit for `A1-647`.
#[test]
fn twenty_five_incremental_shadow_changes_match_the_direct_oracle() {
    enum Mutation {
        Write(&'static str, &'static str),
        Append(&'static str, &'static str),
        Remove(&'static str),
        Rename(&'static str, &'static str),
    }

    let dir = tempfile::Builder::new()
        .prefix("test-receipt-shadow-matrix-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    basic_plan(
        dir.path(),
        r"root: aplan
evidence:
  observation_inputs: [.aplan.yml, runner.sh, catalogue.txt, 'src/**', 'tests/**', 'fixtures/**', 'generated/**', rust-toolchain.toml, Cargo.lock, '.github/workflows/**', 'ci/**', 'inputs/known/**']
test:
  name: unit
  list: [sh, runner.sh, list]
  run: [sh, runner.sh, run, '{test}']
  run_selects: true
  inputs_by_test:
    - tests: ['*']
      paths: [runner.sh, catalogue.txt, 'src/**', 'tests/**', 'fixtures/**', 'generated/**', rust-toolchain.toml, Cargo.lock, '.github/workflows/**', 'ci/**', 'inputs/known/**']
",
    );
    std::fs::write(dir.path().join("catalogue.txt"), "alpha\nbeta\n").expect("catalogue");
    std::fs::write(dir.path().join(".gitignore"), "provider.log\nartifacts/\n")
        .expect("ignore local observations");
    write_executable(
        &dir.path().join("runner.sh"),
        r#"#!/bin/sh
printf '%s:%s\n' "$1" "${2:-}" >> provider.log
case "$1" in
  list) cat catalogue.txt ;;
  run) grep -Fqx -- "$2" catalogue.txt ;;
  *) exit 2 ;;
esac
"#,
    );
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        r"---
id: T-1
title: shadow matrix evidence
phase: 1
status: done
effort_days: 1
paths: [src/**]
tags: [cross-cutting]
evidence:
  - kind: test
    ref: alpha
  - kind: test
    ref: beta
---

## Acceptance criteria

- [x] covered
",
    )
    .expect("audited task");
    common::git(dir.path(), &["init", "-q", "."]);
    common::git(dir.path(), &["config", "user.email", "t@example.invalid"]);
    common::git(dir.path(), &["config", "user.name", "t"]);
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-q", "-m", "base"]);
    let mut previous = String::from_utf8(common::git(dir.path(), &["rev-parse", "HEAD"]).stdout)
        .expect("base UTF-8")
        .trim()
        .to_string();

    std::fs::create_dir(dir.path().join("artifacts")).expect("artifacts");
    let produced = common::run(dir.path(), &receipt_args("artifacts/receipt.json"));
    assert_eq!(produced.code, 0, "{}", produced.text());
    std::fs::write(
        dir.path().join("artifacts/receipts.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "aplan/test-evidence-import-set/v1",
            "receipts": [{
                "provider": "unit",
                "path": "receipt.json",
                "trust": {"mode": "local_file"}
            }]
        }))
        .expect("manifest JSON"),
    )
    .expect("manifest");

    let cases = [
        (
            "task-add-one",
            Mutation::Write(
                "aplan/tasks/T-2.md",
                "---\nid: T-2\ntitle: First plan-only change\nphase: 1\nstatus: todo\n\
                 effort_days: 1\npaths: [src/**]\ntags: [cross-cutting]\n---\n\n\
                 ## Acceptance criteria\n\n- [ ] pending\n",
            ),
        ),
        (
            "task-add-two",
            Mutation::Write(
                "aplan/tasks/T-3.md",
                "---\nid: T-3\ntitle: Second plan-only change\nphase: 1\nstatus: todo\n\
                 effort_days: 1\npaths: [src/**]\ntags: [cross-cutting]\n---\n\n\
                 ## Acceptance criteria\n\n- [ ] pending\n",
            ),
        ),
        (
            "task-add-three",
            Mutation::Write(
                "aplan/tasks/T-4.md",
                "---\nid: T-4\ntitle: Third plan-only change\nphase: 1\nstatus: todo\n\
                 effort_days: 1\npaths: [src/**]\ntags: [cross-cutting]\n---\n\n\
                 ## Acceptance criteria\n\n- [ ] pending\n",
            ),
        ),
        ("code-alpha", Mutation::Write("src/alpha/new.rs", "one\n")),
        ("code-beta", Mutation::Write("src/beta/new.rs", "two\n")),
        ("code-shared", Mutation::Write("src/shared.rs", "three\n")),
        ("test-alpha", Mutation::Write("tests/alpha.rs", "one\n")),
        ("test-beta", Mutation::Write("tests/beta.rs", "two\n")),
        ("test-shared", Mutation::Write("tests/shared.rs", "three\n")),
        (
            "fixture-alpha",
            Mutation::Write("fixtures/alpha/input", "one\n"),
        ),
        (
            "fixture-beta",
            Mutation::Write("fixtures/beta/input", "two\n"),
        ),
        (
            "fixture-shared",
            Mutation::Write("fixtures/shared/input", "three\n"),
        ),
        (
            "generated-alpha",
            Mutation::Write("generated/alpha", "one\n"),
        ),
        ("generated-beta", Mutation::Write("generated/beta", "two\n")),
        (
            "config",
            Mutation::Append(".aplan.yml", "# shadow policy mutation\n"),
        ),
        (
            "runner",
            Mutation::Append("runner.sh", "# shadow runner mutation\n"),
        ),
        ("catalogue", Mutation::Append("catalogue.txt", "gamma\n")),
        (
            "toolchain",
            Mutation::Write("rust-toolchain.toml", "[toolchain]\nchannel = 'stable'\n"),
        ),
        (
            "lockfile",
            Mutation::Write("Cargo.lock", "# shadow lock input\n"),
        ),
        (
            "ci-workflow",
            Mutation::Write(".github/workflows/check.yml", "name: check\n"),
        ),
        (
            "ci-policy",
            Mutation::Write("ci/policy.yml", "version: 1\n"),
        ),
        (
            "known-input",
            Mutation::Write("inputs/known/alpha", "one\n"),
        ),
        (
            "unknown-input",
            Mutation::Write("inputs/unknown/value", "one\n"),
        ),
        ("generated-delete", Mutation::Remove("generated/beta")),
        (
            "fixture-rename",
            Mutation::Rename("fixtures/shared/input", "fixtures/shared/renamed"),
        ),
    ];

    for (label, mutation) in cases {
        match mutation {
            Mutation::Write(relative, contents) => {
                let path = dir.path().join(relative);
                std::fs::create_dir_all(path.parent().expect("mutation parent"))
                    .expect("mutation directory");
                std::fs::write(path, contents).expect("write mutation");
            }
            Mutation::Append(relative, contents) => {
                use std::io::Write as _;
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(dir.path().join(relative))
                    .expect("open append mutation");
                file.write_all(contents.as_bytes())
                    .expect("append mutation");
            }
            Mutation::Remove(relative) => {
                std::fs::remove_file(dir.path().join(relative)).expect("remove mutation");
            }
            Mutation::Rename(from, to) => {
                let destination = dir.path().join(to);
                std::fs::create_dir_all(destination.parent().expect("rename parent"))
                    .expect("rename directory");
                std::fs::rename(dir.path().join(from), destination).expect("rename mutation");
            }
        }
        common::git(dir.path(), &["add", "-A"]);
        common::git(dir.path(), &["commit", "-q", "-m", label]);
        let head = String::from_utf8(common::git(dir.path(), &["rev-parse", "HEAD"]).stdout)
            .expect("head UTF-8")
            .trim()
            .to_string();
        let refreshed = common::run(
            dir.path(),
            &[
                "verify",
                "--refresh-test-receipts",
                "artifacts/receipts.json",
                "--refresh-base",
                &previous,
                "--refresh-head",
                &head,
                "--refresh-shadow",
                "--format",
                "json",
            ],
        );
        assert_eq!(refreshed.code, 0, "{label}: {}", refreshed.text());
        let document: serde_json::Value =
            serde_json::from_str(&refreshed.stdout).unwrap_or_else(|error| {
                panic!("{label}: invalid JSON ({error}): {}", refreshed.text())
            });
        let observation = &document["evidence_observation"];
        assert_eq!(observation["scope"], "whole_plan", "{label}: {observation}");
        assert_eq!(
            observation["provider_source"], "incremental_refresh",
            "{label}: {observation}"
        );
        assert_eq!(observation["state"], "current", "{label}: {observation}");
        assert_eq!(
            observation["completeness"], "complete",
            "{label}: {observation}"
        );
        assert_eq!(
            observation["refresh"]["shadow_compared"], true,
            "{label}: the production direct oracle was not compared"
        );
        previous = head;
    }
}

fn assert_trusted_incremental_refresh_refuses(
    repo: &Path,
    import_set: &Path,
    base: &str,
    head: &str,
) {
    let result = common::run(
        repo,
        &[
            "verify",
            "--refresh-test-receipts",
            import_set.to_str().expect("UTF-8 manifest"),
            "--refresh-base",
            base,
            "--refresh-head",
            head,
        ],
    );
    assert_eq!(result.code, 2, "{}", result.text());
    assert!(
        result
            .text()
            .contains("authenticated artifact or signed receipt cannot be replaced"),
        "{}",
        result.text()
    );
}

#[test]
fn incremental_refresh_never_downgrades_authenticated_receipt_trust() {
    let dir = tempfile::Builder::new()
        .prefix("test-receipt-refresh-trust-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    receipt_fixture(dir.path(), false);
    let base = String::from_utf8(common::git(dir.path(), &["rev-parse", "HEAD"]).stdout)
        .expect("base UTF-8")
        .trim()
        .to_string();
    let artifacts = tempfile::Builder::new()
        .prefix("test-receipt-refresh-trust-artifacts-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("artifacts");
    let receipt = artifacts.path().join("receipt.json");
    let produced = common::run(
        dir.path(),
        &receipt_args(receipt.to_str().expect("UTF-8 receipt")),
    );
    assert_eq!(produced.code, 0, "{}", produced.text());
    let receipt_bytes = std::fs::read(&receipt).expect("receipt bytes");
    let import_set = artifacts.path().join("receipts.json");
    std::fs::write(
        &import_set,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "aplan/test-evidence-import-set/v1",
            "receipts": [{
                "provider": "unit",
                "path": "receipt.json",
                "trust": {
                    "mode": "same_workflow",
                    "artifact_sha256": aplan_attest::sha256_hex(&receipt_bytes),
                    "producer": {
                        "repository": "owner/repository",
                        "workflow": "checks.yml:unit",
                        "run_id": "17",
                        "event": "push",
                        "privilege": "trusted"
                    }
                }
            }]
        }))
        .expect("manifest JSON"),
    )
    .expect("manifest");
    let before = std::fs::read(&import_set).expect("manifest before refresh");

    std::fs::write(dir.path().join("README.md"), "unrelated plan prose\n")
        .expect("unrelated change");
    common::git(dir.path(), &["add", "README.md"]);
    common::git(
        dir.path(),
        &["commit", "-q", "-m", "change unrelated prose"],
    );
    let unrelated_head = String::from_utf8(common::git(dir.path(), &["rev-parse", "HEAD"]).stdout)
        .expect("unrelated head UTF-8")
        .trim()
        .to_string();
    assert_trusted_incremental_refresh_refuses(dir.path(), &import_set, &base, &unrelated_head);
    assert_eq!(
        std::fs::read(&import_set).expect("manifest after unrelated refusal"),
        before,
        "revision-only refresh must not downgrade trusted provenance"
    );

    std::fs::write(dir.path().join("src/lib.rs"), "changed\n").expect("material change");
    common::git(dir.path(), &["add", "src/lib.rs"]);
    common::git(dir.path(), &["commit", "-q", "-m", "change material"]);
    let head = String::from_utf8(common::git(dir.path(), &["rev-parse", "HEAD"]).stdout)
        .expect("head UTF-8")
        .trim()
        .to_string();
    assert_trusted_incremental_refresh_refuses(dir.path(), &import_set, &base, &head);
    assert_eq!(
        std::fs::read(&import_set).expect("manifest after refusal"),
        before,
        "trust refusal must not rewrite the caller-owned manifest"
    );
}
