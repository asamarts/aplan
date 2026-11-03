//! The release gate has the changelog and history it needs before publishing
//! (`A1-321`, `A1-402`).

use std::path::{Path, PathBuf};
use std::process::Command;

use yaml_compat::Value;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn check(version: &str, changelog: &Path) -> std::process::Output {
    Command::new("bash")
        .arg(workspace_root().join("ci/scripts/check-release-changelog.sh"))
        .arg(version)
        .arg(changelog)
        .output()
        .expect("run the release changelog gate")
}

#[test]
fn a_release_without_its_changelog_section_is_refused() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let changelog = dir.path().join("CHANGELOG.md");
    std::fs::write(
        &changelog,
        "# Changelog\n\n## [Unreleased]\n\n```md\n## [0.4.0]\n```\n",
    )
    .expect("write changelog");

    let out = check("0.4.0", &changelog);
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "a fenced example is not a section");
    assert!(
        error.contains("0.4.0"),
        "the version is actionable: {error}"
    );
    assert!(
        error.contains(&changelog.display().to_string()),
        "the file is actionable: {error}"
    );
}

#[test]
fn an_empty_unreleased_section_above_the_release_passes() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let changelog = dir.path().join("CHANGELOG.md");
    std::fs::write(
        &changelog,
        "# Changelog\n\n## [Unreleased]\n\n## [0.4.0] - 2026-10-06\n\n### Added\n\n- one thing\n",
    )
    .expect("write changelog");

    let out = check("0.4.0", &changelog);
    assert!(
        out.status.success(),
        "a closed changelog was refused: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn only_a_bare_or_iso_dated_version_heading_is_a_section() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let changelog = dir.path().join("CHANGELOG.md");

    std::fs::write(&changelog, "# Changelog\n\n## [0.4.0]\n").expect("write bare heading");
    assert!(
        check("0.4.0", &changelog).status.success(),
        "a changelog may omit the release date"
    );

    std::fs::write(&changelog, "# Changelog\n\n## [0.4.0] - someday\n")
        .expect("write lookalike heading");
    assert!(
        !check("0.4.0", &changelog).status.success(),
        "arbitrary text after the version is not a release heading"
    );
}

#[test]
fn the_changelog_gate_runs_before_the_release_build() {
    let release = std::fs::read_to_string(workspace_root().join("ci/scripts/release.sh"))
        .expect("release script");
    let check = release
        .find("ci/scripts/check-release-changelog.sh")
        .expect("release script calls the changelog gate");
    let build = release
        .find("cargo build --release")
        .expect("release script builds the artifact");
    assert!(
        check < build,
        "a missing changelog must fail before the release build starts"
    );
}

#[test]
fn the_runbook_checks_the_changelog_before_tagging() {
    let runbook =
        std::fs::read_to_string(workspace_root().join("ci/README.md")).expect("release runbook");
    let changelog = runbook
        .find("close `CHANGELOG.md`")
        .expect("runbook closes the changelog");
    let gate = runbook
        .find("TAG=vX.Y.Z ci/scripts/release.sh")
        .expect("runbook runs the release gate");
    let tag = runbook
        .find("create the annotated `vX.Y.Z` tag")
        .expect("runbook creates the tag");
    assert!(
        changelog < gate && gate < tag,
        "the runbook must close, check, and only then tag the release"
    );
}

#[test]
fn the_release_checkout_carries_the_history_the_version_gate_reads() {
    let workflow = release_workflow();
    let steps = release_steps(&workflow);
    let (checkout_index, checkout) = steps
        .iter()
        .enumerate()
        .find(|(_, step)| {
            step["uses"]
                .as_str()
                .is_some_and(|action| action.starts_with("actions/checkout@"))
        })
        .expect("release workflow checks out its source");
    let build_index = steps
        .iter()
        .position(|step| step["name"] == "build the release artifact")
        .expect("release workflow builds the artifact");
    assert!(
        checkout_index < build_index,
        "checkout must precede the build"
    );
    assert_eq!(
        checkout["with"]["fetch-depth"], 0,
        "the release checkout must fetch the tags and history used by `aplan version`"
    );
}

fn release_workflow() -> Value {
    let text = std::fs::read_to_string(workspace_root().join(".github/workflows/release.yml"))
        .expect("read release workflow");
    yaml_compat::from_str(&text).expect("release workflow is YAML")
}

fn release_steps(document: &Value) -> &[Value] {
    document
        .get("jobs")
        .and_then(|jobs| jobs.get("release"))
        .and_then(|job| job.get("steps"))
        .and_then(Value::as_sequence)
        .expect("release job has steps")
}

fn release_steps_mut(document: &mut Value) -> &mut Vec<Value> {
    document
        .get_mut("jobs")
        .and_then(|jobs| jobs.get_mut("release"))
        .and_then(|job| job.get_mut("steps"))
        .and_then(Value::as_sequence_mut)
        .expect("release job has mutable steps")
}

fn step_run(step: &Value) -> Option<&str> {
    step.get("run").and_then(Value::as_str)
}

fn release_workflow_problem(document: &Value) -> Option<String> {
    let Some(job) = document.get("jobs").and_then(|jobs| jobs.get("release")) else {
        return Some("release job is missing".to_string());
    };
    let timeout = job.get("timeout-minutes").and_then(Value::as_u64);
    if timeout.is_none_or(|minutes| minutes < 60) {
        return Some("release timeout does not cover full validation plus packaging".to_string());
    }
    let expected_source = document
        .get("env")
        .and_then(|env| env.get("APLAN_RELEASE_SHA"))
        .and_then(Value::as_str);
    if expected_source != Some("${{ github.sha }}") {
        return Some("release steps are not bound to github.sha".to_string());
    }
    if job
        .get("env")
        .and_then(|env| env.get("APLAN_RELEASE_SHA"))
        .is_some()
    {
        return Some("release job overrides the workflow source binding".to_string());
    }

    let Some(steps) = job.get("steps").and_then(Value::as_sequence) else {
        return Some("release job has no steps".to_string());
    };
    let checkout_steps = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| {
            step.get("uses")
                .and_then(Value::as_str)
                .is_some_and(|uses| uses.starts_with("actions/checkout@"))
        })
        .collect::<Vec<_>>();
    if checkout_steps.len() != 1 {
        return Some(format!(
            "release must have exactly one checkout, found {}",
            checkout_steps.len()
        ));
    }
    let (checkout_index, checkout) = checkout_steps[0];
    if checkout
        .get("with")
        .and_then(|with| with.get("fetch-depth"))
        .and_then(Value::as_u64)
        != Some(0)
    {
        return Some("release checkout does not fetch full history".to_string());
    }

    let validation_index = steps
        .iter()
        .position(|step| step_run(step) == Some("ci/scripts/validate.sh --profile release"));
    let build_index = steps
        .iter()
        .position(|step| step_run(step) == Some("ci/scripts/release.sh"));
    let publish_index = steps
        .iter()
        .position(|step| step.get("name").and_then(Value::as_str) == Some("publish the release"));
    let (Some(validation_index), Some(build_index), Some(publish_index)) =
        (validation_index, build_index, publish_index)
    else {
        return Some("release validation, build, or publication step is missing".to_string());
    };
    if !(checkout_index < validation_index
        && validation_index < build_index
        && build_index < publish_index)
    {
        return Some(
            "release must check out, validate, build, and publish in that order".to_string(),
        );
    }

    let validation = &steps[validation_index];
    if validation.get("if").is_some() || validation.get("continue-on-error").is_some() {
        return Some("release validation may not be skipped or ignored".to_string());
    }
    if validation
        .get("env")
        .and_then(|env| env.get("APLAN_RELEASE_SHA"))
        .is_some()
    {
        return Some("release validation overrides the workflow source binding".to_string());
    }
    let build = &steps[build_index];
    if build.get("if").is_some() || build.get("continue-on-error").is_some() {
        return Some("release artifact construction may not be skipped or ignored".to_string());
    }
    if build
        .get("env")
        .and_then(|env| env.get("APLAN_RELEASE_SHA"))
        .is_some()
    {
        return Some("release build overrides the workflow source binding".to_string());
    }
    let publish = &steps[publish_index];
    if publish.get("if").and_then(Value::as_str) != Some("github.ref_type == 'tag'") {
        return Some("only a tag may publish a GitHub release".to_string());
    }
    if publish.get("continue-on-error").is_some() {
        return Some("release publication may not ignore failure".to_string());
    }
    None
}

#[test]
fn release_workflow_cannot_publish_without_same_source_full_validation() {
    let canonical = release_workflow();
    assert_eq!(release_workflow_problem(&canonical), None);

    let mut without_job = canonical.clone();
    without_job
        .get_mut("jobs")
        .and_then(Value::as_mapping_mut)
        .expect("workflow jobs are a mapping")
        .remove(Value::String("release".to_string()));
    assert!(release_workflow_problem(&without_job).is_some());

    let mut without_validation = canonical.clone();
    release_steps_mut(&mut without_validation)
        .retain(|step| step_run(step) != Some("ci/scripts/validate.sh --profile release"));
    assert!(release_workflow_problem(&without_validation).is_some());

    let mut second_checkout = canonical.clone();
    let checkout = release_steps(&second_checkout)
        .iter()
        .find(|step| step.get("uses").is_some())
        .expect("checkout step")
        .clone();
    let build = release_steps(&second_checkout)
        .iter()
        .position(|step| step_run(step) == Some("ci/scripts/release.sh"))
        .expect("build step");
    release_steps_mut(&mut second_checkout).insert(build, checkout);
    assert!(release_workflow_problem(&second_checkout).is_some());

    let mut skipped_validation = canonical.clone();
    let validation = release_steps_mut(&mut skipped_validation)
        .iter_mut()
        .find(|step| step_run(step) == Some("ci/scripts/validate.sh --profile release"))
        .expect("validation step");
    validation
        .as_mapping_mut()
        .expect("validation step is a mapping")
        .insert(Value::String("if".to_string()), Value::Bool(false));
    assert!(release_workflow_problem(&skipped_validation).is_some());

    let mut ignored_validation = canonical.clone();
    let validation = release_steps_mut(&mut ignored_validation)
        .iter_mut()
        .find(|step| step_run(step) == Some("ci/scripts/validate.sh --profile release"))
        .expect("validation step");
    validation
        .as_mapping_mut()
        .expect("validation step is a mapping")
        .insert(
            Value::String("continue-on-error".to_string()),
            Value::Bool(true),
        );
    assert!(release_workflow_problem(&ignored_validation).is_some());

    let mut rebound_validation = canonical.clone();
    let validation = release_steps_mut(&mut rebound_validation)
        .iter_mut()
        .find(|step| step_run(step) == Some("ci/scripts/validate.sh --profile release"))
        .expect("validation step");
    validation
        .get_mut("env")
        .and_then(Value::as_mapping_mut)
        .expect("validation environment is a mapping")
        .insert(
            Value::String("APLAN_RELEASE_SHA".to_string()),
            Value::String("${{ github.ref }}".to_string()),
        );
    assert!(release_workflow_problem(&rebound_validation).is_some());

    let mut early_publish = canonical.clone();
    let steps = release_steps_mut(&mut early_publish);
    let validation = steps
        .iter()
        .position(|step| step_run(step) == Some("ci/scripts/validate.sh --profile release"))
        .expect("validation step");
    let publish = steps
        .iter()
        .position(|step| step.get("name").and_then(Value::as_str) == Some("publish the release"))
        .expect("publish step");
    steps.swap(validation, publish);
    assert!(release_workflow_problem(&early_publish).is_some());
}

fn artifact_source_problem(script: &str) -> Option<&'static str> {
    let checks = script
        .match_indices("ci/scripts/release-source.sh")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if checks.len() != 2 {
        return Some("artifact construction must verify its source before and after building");
    }
    let Some(toolchain) = script.find("ci/scripts/toolchain.sh verify") else {
        return Some("artifact construction does not verify the exact toolchain");
    };
    let Some(first_build) = script.find("cargo build --release") else {
        return Some("artifact construction does not build an artifact");
    };
    let Some(package) = script.find("rm -rf \"$OUT_DIR\"") else {
        return Some("artifact construction does not reach packaging");
    };
    if !(checks[0] < toolchain
        && toolchain < first_build
        && first_build < checks[1]
        && checks[1] < package)
    {
        return Some("source checks do not surround artifact construction");
    }
    if !script.contains("export APLAN_RELEASE_SHA=\"$release_source\"") {
        return Some("the second source check is not bound to the first one");
    }
    None
}

#[test]
fn release_artifact_construction_rechecks_the_validated_source() {
    let canonical = std::fs::read_to_string(workspace_root().join("ci/scripts/release.sh"))
        .expect("read release script");
    assert_eq!(artifact_source_problem(&canonical), None);

    let without_second = canonical.replacen(
        "source_after=\"$(ci/scripts/release-source.sh \"$REPO_ROOT\")\"",
        "source_after=\"$release_source\"",
        1,
    );
    assert!(artifact_source_problem(&without_second).is_some());

    let unbound = canonical.replace(
        "export APLAN_RELEASE_SHA=\"$release_source\"",
        "unset APLAN_RELEASE_SHA",
    );
    assert!(artifact_source_problem(&unbound).is_some());

    let without_build = canonical.replace("cargo build --release", "true # removed build");
    assert!(artifact_source_problem(&without_build).is_some());
}

#[test]
fn the_release_proves_headless_isolated_then_builds_with_tui_explicitly() {
    let release = std::fs::read_to_string(workspace_root().join("ci/scripts/release.sh"))
        .expect("release script");
    let headless = release
        .find("cargo build --release --locked -p aplan --no-default-features")
        .expect("release script builds the headless shape");
    let graph = release
        .find("cargo tree --locked -p aplan --no-default-features")
        .expect("release script inspects the headless dependency graph");
    let absent = release
        .find("facts --require build.tui >/dev/null 2>&1")
        .expect("headless artifact proves the capability is absent");
    let release_build = release
        .find("cargo build --release --locked -p aplan --features tui")
        .expect("release artifact enables tui explicitly");
    let present = release[release_build..]
        .find("facts --require build.tui")
        .expect("release artifact proves the capability is present");

    assert!(headless < graph && graph < absent && absent < release_build);
    assert!(
        present > 0,
        "the capability probe must follow the release build"
    );
    assert!(
        release.contains("headless_require_status\" -ne 1"),
        "the absent-capability probe must distinguish a clean refusal from a usage or internal error"
    );
    assert!(
        release.contains("headless_tui_status\" -ne 2"),
        "the headless stub must prove its documented usage-error exit code"
    );
}
