//! The release gate has the changelog and history it needs before publishing
//! (`A1-321`, `A1-402`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, os::unix::fs::PermissionsExt};

use yaml_compat::Value;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn write_executable(path: &Path, text: &str) {
    fs::write(path, text).expect("write executable fixture");
    let mut permissions = fs::metadata(path)
        .expect("read executable fixture metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make fixture executable");
}

fn commit_fixture(root: &Path) -> String {
    for arguments in [
        vec!["init", "--quiet"],
        vec!["config", "user.name", "Release Test"],
        vec!["config", "user.email", "release-test@example.invalid"],
        vec!["add", "."],
        vec!["commit", "--quiet", "-m", "fixture"],
    ] {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(root)
            .output()
            .expect("run Git for release fixture");
        assert!(
            output.status.success(),
            "Git fixture setup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD^{commit}"])
        .current_dir(root)
        .output()
        .expect("resolve release fixture source");
    assert!(
        output.status.success(),
        "release fixture source lookup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("release fixture source is UTF-8")
        .trim()
        .to_string()
}

fn copy_release_script(root: &Path, name: &str) {
    let destination = root.join("ci/scripts").join(name);
    fs::copy(workspace_root().join("ci/scripts").join(name), &destination)
        .expect("copy release fixture script");
    let mut permissions = fs::metadata(&destination)
        .expect("read copied script metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(destination, permissions).expect("make copied script executable");
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

#[test]
fn release_workflow_publishes_crates_before_github_assets() {
    let workflow = release_workflow();
    let workflow_text =
        std::fs::read_to_string(workspace_root().join(".github/workflows/release.yml"))
            .expect("read release workflow");
    assert_eq!(
        workflow["permissions"]["id-token"], "write",
        "trusted publishing needs only the workflow's short-lived OIDC identity"
    );
    assert_eq!(
        workflow["jobs"]["release"]["environment"], "release",
        "the crates.io publisher must be bound to the protected release environment"
    );

    let steps = release_steps(&workflow);
    let validation = steps
        .iter()
        .position(|step| step_run(step) == Some("ci/scripts/validate.sh --profile release"))
        .expect("full release validation");
    let artifact = steps
        .iter()
        .position(|step| step_run(step) == Some("ci/scripts/release.sh"))
        .expect("release artifact build");
    let auth = steps
        .iter()
        .position(|step| {
            step["uses"]
                .as_str()
                .is_some_and(|uses| uses.starts_with("rust-lang/crates-io-auth-action@"))
        })
        .expect("crates.io OIDC authentication");
    let crates = steps
        .iter()
        .position(|step| step_run(step) == Some("ci/scripts/publish-crates.sh"))
        .expect("crates.io publication");
    let github = steps
        .iter()
        .position(|step| step_run(step) == Some("ci/scripts/publish-github-release.sh"))
        .expect("GitHub release publication");

    assert!(validation < artifact && artifact < auth && auth < crates && crates < github);
    assert_eq!(steps[auth]["if"], "github.ref_type == 'tag'");
    assert_eq!(steps[crates]["if"], "github.ref_type == 'tag'");
    assert_eq!(steps[github]["if"], "github.ref_type == 'tag'");
    assert_eq!(
        steps[crates]["env"]["CARGO_REGISTRY_TOKEN"], "${{ steps.crates_io_auth.outputs.token }}",
        "cargo must receive the temporary token through its environment"
    );
    for index in [auth, crates, github] {
        assert!(
            steps[index].get("continue-on-error").is_none(),
            "no publication stage may turn a failure into success"
        );
    }
    let auth_action = steps[auth]["uses"].as_str().expect("authentication action");
    let (_, revision) = auth_action
        .rsplit_once('@')
        .expect("authentication action is pinned");
    assert!(
        revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "the OIDC action must be pinned to a full commit, found {revision}"
    );
    assert!(
        !workflow_text.contains("secrets.CARGO"),
        "release automation must not retain a long-lived crates.io secret"
    );
    let hosted = std::fs::read_to_string(workspace_root().join("ci/scripts/prepare-hosted.sh"))
        .expect("read hosted release preparation");
    assert!(
        hosted.contains("for command in gh jq; do"),
        "hosted release setup must fail before validation when publication tools are absent"
    );
}

fn publish_order(script: &str) -> Vec<&str> {
    let body = script
        .split_once("PUBLISH_ORDER=(")
        .expect("publish order starts")
        .1
        .split_once(')')
        .expect("publish order ends")
        .0;
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

const EXPECTED_PUBLISH_ORDER: [&str; 10] = [
    "aplan-core",
    "aplan-output",
    "aplan-config",
    "aplan-evidence",
    "aplan-interop",
    "aplan-attest",
    "aplan-app",
    "aplan-tui",
    "aplan-mcp",
    "aplan",
];

fn assert_publish_order_matches_workspace(order: &[&str]) {
    let positions = order
        .iter()
        .enumerate()
        .map(|(index, package)| (*package, index))
        .collect::<std::collections::BTreeMap<_, _>>();
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let metadata = std::process::Command::new(cargo)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(workspace_root())
        .output()
        .expect("run cargo metadata");
    assert!(
        metadata.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&metadata.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&metadata.stdout).expect("parse cargo metadata");
    let packages = metadata["packages"].as_array().expect("metadata packages");
    let published = packages
        .iter()
        .filter(|package| package["publish"] != serde_json::json!([]))
        .map(|package| package["name"].as_str().expect("package name"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        published,
        positions.keys().copied().collect(),
        "the script must enumerate every and only publishable workspace package"
    );
    for package in packages {
        let name = package["name"].as_str().expect("package name");
        let Some(&package_index) = positions.get(name) else {
            continue;
        };
        assert_eq!(package["publish"], serde_json::json!(["crates-io"]));
        assert!(
            package["homepage"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert!(
            package["readme"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        for dependency in package["dependencies"]
            .as_array()
            .expect("package dependencies")
        {
            if dependency["kind"] == "dev" {
                continue;
            }
            let dependency = dependency["name"].as_str().expect("dependency name");
            let Some(&dependency_index) = positions.get(dependency) else {
                continue;
            };
            assert!(
                dependency_index < package_index,
                "{name} is published before its internal dependency {dependency}"
            );
        }
    }
}

#[test]
fn publish_script_uses_dependency_order_and_registry_confirmation() {
    let script = std::fs::read_to_string(workspace_root().join("ci/scripts/publish-crates.sh"))
        .expect("read crates.io publication script");
    let order = publish_order(&script);
    assert_eq!(order, EXPECTED_PUBLISH_ORDER);
    assert_publish_order_matches_workspace(&order);
    for required in [
        "ci/scripts/release-source.sh",
        "cargo package --locked --no-verify --registry crates-io -p",
        "cargo publish --locked --registry crates-io -p",
        "cargo info \"${crate}@${crate_version}\"",
        "https://crates.io/api/v1/crates/",
        "sha256sum",
        "checksum",
        "wait_for_registry",
    ] {
        assert!(
            script.contains(required),
            "publication script omitted `{required}`"
        );
    }
    assert!(
        script.find("cargo package --locked --no-verify --registry crates-io -p")
            < script.find("cargo publish --locked --registry crates-io -p")
            && script.find("cargo publish --locked --registry crates-io -p")
                < script.rfind("wait_for_registry"),
        "package identity must precede upload, which must precede registry confirmation"
    );

    let github =
        std::fs::read_to_string(workspace_root().join("ci/scripts/publish-github-release.sh"))
            .expect("read GitHub publication script");
    assert!(github.contains("cmp --silent"));
    assert!(
        !github.contains("--clobber"),
        "a rerun must refuse asset drift rather than replace different bytes"
    );
}

fn prepare_crates_publication_fixture(root: &Path) -> String {
    fs::create_dir_all(root.join("ci/scripts")).expect("create scripts directory");
    fs::create_dir_all(root.join("bin")).expect("create command directory");
    for name in [
        "publish-crates.sh",
        "release-source.sh",
        "current-version.sh",
        "check-release-changelog.sh",
    ] {
        copy_release_script(root, name);
    }
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver = \"3\"\n\n[workspace.package]\nversion = \"0.5.0\"\n",
    )
    .expect("write fixture manifest");
    fs::write(
        root.join("CHANGELOG.md"),
        "# Changelog\n\n## [Unreleased]\n\n## [0.5.0] - 2026-11-03\n",
    )
    .expect("write fixture changelog");
    fs::write(root.join(".gitignore"), "/state\n/target\n").expect("write fixture ignores");
    write_executable(
        &root.join("bin/cargo"),
        r#"#!/usr/bin/env bash
set -euo pipefail
root="${RELEASE_FIXTURE_ROOT:?}"
command="$1"
shift
case "$command" in
    package|publish)
        crate=""
        while [[ "$#" -gt 0 ]]; do
            if [[ "$1" == "-p" ]]; then crate="$2"; shift 2; else shift; fi
        done
        [[ -n "$crate" ]]
        printf '%s %s\n' "$command" "$crate" >> "$root/state/cargo.log"
        if [[ "$command" == package ]]; then
            mkdir -p "$root/target/package"
            printf 'archive for %s\n' "$crate" > "$root/target/package/${crate}-0.5.0.crate"
        else
            touch "$root/state/published-${crate}"
        fi
        ;;
    info)
        specification="$1"
        crate="${specification%@*}"
        [[ -f "$root/state/published-${crate}" ]]
        printf 'info %s\n' "$specification" >> "$root/state/cargo.log"
        ;;
    *) echo "unexpected cargo command: $command" >&2; exit 2 ;;
esac
"#,
    );
    write_executable(
        &root.join("bin/curl"),
        r#"#!/usr/bin/env bash
set -euo pipefail
root="${RELEASE_FIXTURE_ROOT:?}"
url="${!#}"
version="${url##*/}"
crate_path="${url%/*}"
crate="${crate_path##*/}"
if [[ -f "$root/state/published-${crate}" ]]; then
    checksum="$(sha256sum "$root/target/package/${crate}-${version}.crate" | awk '{print $1}')"
    printf '{"version":{"checksum":"%s"}}\n200' "$checksum"
else
    printf '{}\n404'
fi
"#,
    );
    write_executable(&root.join("bin/sleep"), "#!/usr/bin/env bash\nexit 0\n");
    let source = commit_fixture(root);
    fs::create_dir(root.join("state")).expect("create fixture state");
    source
}

fn expected_crates_publication_log() -> String {
    EXPECTED_PUBLISH_ORDER
        .iter()
        .flat_map(|name| {
            [
                format!("package {name}"),
                format!("publish {name}"),
                format!("info {name}@0.5.0"),
            ]
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn crates_publication_runs_the_registry_transaction_in_dependency_order() {
    let fixture = tempfile::Builder::new()
        .prefix("publish-crates-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create crates publication fixture");
    let root = fixture.path();
    let fixture_source = prepare_crates_publication_fixture(root);

    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").expect("PATH")
    );
    let output = Command::new(root.join("ci/scripts/publish-crates.sh"))
        .current_dir(root)
        .env("PATH", path)
        .env("TAG", "v0.5.0")
        // A release-profile parent exports its own source. This isolated
        // checkout must bind the copied scripts to the fixture commit.
        .env("APLAN_RELEASE_SHA", fixture_source)
        .env("RELEASE_FIXTURE_ROOT", root)
        .env("APLAN_REGISTRY_ATTEMPTS", "2")
        .env("APLAN_REGISTRY_INTERVAL_SECONDS", "0")
        .output()
        .expect("run crates publication fixture");
    assert!(
        output.status.success(),
        "publication fixture failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let observed =
        fs::read_to_string(root.join("state/cargo.log")).expect("read publication transaction log");
    assert_eq!(observed.trim(), expected_crates_publication_log());
}

fn prepare_github_publication_fixture(root: &Path) -> (&'static str, String, String) {
    fs::create_dir_all(root.join("ci/scripts")).expect("create scripts directory");
    fs::create_dir_all(root.join("bin")).expect("create command directory");
    for name in [
        "publish-github-release.sh",
        "release-source.sh",
        "current-version.sh",
    ] {
        copy_release_script(root, name);
    }
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver = \"3\"\n\n[workspace.package]\nversion = \"0.5.0\"\n",
    )
    .expect("write fixture manifest");
    fs::write(root.join(".gitignore"), "/dist\n/state\n").expect("write fixture ignores");
    write_executable(
        &root.join("bin/gh"),
        r#"#!/usr/bin/env bash
set -euo pipefail
root="${RELEASE_FIXTURE_ROOT:?}"
[[ "$1 $2" == "release view" || "$1 $2" == "release download" || "$1 $2" == "release upload" ]]
operation="$2"
shift 2
case "$operation" in
    view)
        if [[ " $* " == *" --json "* ]]; then
            find "$root/state/remote" -maxdepth 1 -type f -printf '%f\n' | sort
        fi
        ;;
    download)
        directory=""
        pattern=""
        while [[ "$#" -gt 0 ]]; do
            case "$1" in
                --dir) directory="$2"; shift 2 ;;
                --pattern) pattern="$2"; shift 2 ;;
                *) shift ;;
            esac
        done
        cp "$root/state/remote/$pattern" "$directory/$pattern"
        ;;
    upload)
        printf 'upload\n' >> "$root/state/gh.log"
        for argument in "$@"; do
            [[ -f "$argument" ]] && cp "$argument" "$root/state/remote/"
        done
        ;;
esac
"#,
    );
    let source = commit_fixture(root);
    fs::create_dir_all(root.join("state/remote")).expect("create remote fixture state");
    fs::create_dir(root.join("dist")).expect("create local release assets");
    let archive = "aplan-v0.5.0-x86_64-unknown-linux-musl.tar.gz";
    let checksum = format!("{archive}.sha256");
    fs::write(root.join("dist").join(archive), "validated archive\n").expect("write local archive");
    fs::write(root.join("dist").join(&checksum), "validated checksum\n")
        .expect("write local checksum");
    fs::write(
        root.join("state/remote").join(archive),
        "different archive\n",
    )
    .expect("write mismatched remote archive");
    fs::write(
        root.join("state/remote").join(&checksum),
        "validated checksum\n",
    )
    .expect("write matching remote checksum");
    let command_path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").expect("PATH")
    );
    (archive, command_path, source)
}

fn run_github_publication(
    root: &Path,
    command_path: &str,
    fixture_source: &str,
) -> std::process::Output {
    Command::new(root.join("ci/scripts/publish-github-release.sh"))
        .current_dir(root)
        .env("PATH", command_path)
        .env("TAG", "v0.5.0")
        .env("APLAN_RELEASE_SHA", fixture_source)
        .env("GITHUB_REPOSITORY", "asamarts/aplan")
        .env("GH_TOKEN", "fixture-token")
        .env("RELEASE_FIXTURE_ROOT", root)
        .output()
        .expect("run GitHub publication fixture")
}

#[test]
fn github_publication_refuses_drift_and_only_fills_missing_assets() {
    let fixture = tempfile::Builder::new()
        .prefix("publish-github-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create GitHub publication fixture");
    let root = fixture.path();
    let (archive, command_path, fixture_source) = prepare_github_publication_fixture(root);
    let checksum = format!("{archive}.sha256");

    let drift = run_github_publication(root, &command_path, &fixture_source);
    assert!(
        !drift.status.success(),
        "different remote bytes were accepted"
    );
    assert!(
        String::from_utf8_lossy(&drift.stderr).contains("differs from the validated artifact"),
        "drift refusal was not actionable: {}",
        String::from_utf8_lossy(&drift.stderr)
    );
    assert!(!root.join("state/gh.log").exists(), "drift reached upload");

    fs::copy(
        root.join("dist").join(archive),
        root.join("state/remote").join(archive),
    )
    .expect("repair remote archive");
    fs::remove_file(root.join("state/remote").join(&checksum)).expect("remove remote checksum");
    let missing = run_github_publication(root, &command_path, &fixture_source);
    assert!(
        missing.status.success(),
        "missing asset was not filled: {}",
        String::from_utf8_lossy(&missing.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.join("state/remote").join(checksum))
            .expect("read uploaded checksum"),
        "validated checksum\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("state/gh.log")).expect("read upload log"),
        "upload\n"
    );
}
