//! **A public workflow can name no persistent runner that exists** (`A1-382`).
//!
//! A pull request supplies the workflow that runs, so a router inside that
//! workflow is not a security boundary: the author can bypass it and write the
//! self-hosted labels directly. The boundary chosen in the pre-public repository-boundary decision is outside the
//! workflow. The persistent runner exists only while the repository is private;
//! it is deregistered before the repository becomes public, and every committed
//! job selects a standard hosted runner in that state. The one portability
//! workflow widens the public standard-hosted answer to macOS and Windows,
//! behind a pre-allocation public-only guard (ADR-0059).
//!
//! This is an executable census rather than a line scan. Every workflow job is
//! accounted for, every job has the one visibility expression whose branches
//! were reviewed, and every job that compiles the repository names the hosted
//! preparation profile it needs. A new job is a failure until it joins the
//! table, so the audit cannot quietly drift behind the surface it claims to
//! cover.

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

/// One reviewed expression is deliberately repeated in workflow YAML. A YAML
/// anchor would hide the placement decision from GitHub's workflow reader, and
/// a router job can be rewritten by the same pull request it is meant to judge.
const VISIBILITY_RUNNER: &str = "${{ (github.event.repository.private && \
    fromJSON('[\"self-hosted\",\"linux\",\"aplan\"]')) || fromJSON('[\"ubuntu-latest\"]') }}";
const PUBLIC_PORTABILITY_RUNNER: &str = "${{ matrix.runner }}";
const PUBLIC_PORTABILITY_GUARD: &str = "github.event.repository.private == false";

/// The one workflow reachable from a pull request while private must retain an
/// event allow-list. An exclusion such as `event != pull_request` silently
/// admits any future trigger somebody adds without reviewing its trust model.
const ACTION_SELFTEST_GUARD: &str = "github.event_name == 'push' || \
    github.event_name == 'workflow_dispatch' || (github.event_name == 'pull_request' && \
    github.event.repository.private == false)";

/// Hosted preparation profiles, one per job that invokes repository scripts.
///
/// The entry drives an assertion against the actual YAML. This is not a list of
/// names that passes merely by containing every current member.
const HOSTED_PREPARATION: &[(&str, &str, &str)] = &[
    ("ci.yml", "development", "\"$APLAN_ORDINARY_PROFILE\""),
    ("ci.yml", "full", "coverage"),
    ("evidence-rollout.yml", "produce", "base"),
    ("latest-stable.yml", "latest-stable", "latest-stable"),
    ("release.yml", "release", "release"),
    ("scale.yml", "scale", "base"),
    ("tui-portability.yml", "linux", "portability"),
    ("tui-portability.yml", "public-platforms", "portability"),
];

/// Jobs that need no toolchain preparation, with the reason held beside the
/// census rather than inferred from an absent table row.
const NO_HOSTED_PREPARATION: &[(&str, &str, &str)] = &[
    (
        "action-selftest.yml",
        "selftest",
        "the action downloads the released static binary and GitHub-hosted runners already provide gh, curl and tar",
    ),
    (
        "evidence-rollout.yml",
        "consume",
        "the consumer executes the same-workflow binary and Python audit harness without compiling; both private and standard hosted images provide Python and Git",
    ),
];

/// Immutable external action identities admitted by this repository. Local
/// actions are sourced from the checked-out revision and have no independent
/// ref to pin.
const PINNED_ACTIONS: &[(&str, &str)] = &[
    (
        "actions/checkout",
        "3d3c42e5aac5ba805825da76410c181273ba90b1",
    ),
    (
        "actions/download-artifact",
        "9000827ccba6bdab643e8b6fd33ac0654aef8333",
    ),
    (
        "actions/upload-artifact",
        "cf430e030ddbb5b0abf93d22962f4752f3646cd9",
    ),
    (
        "rust-lang/crates-io-auth-action",
        "3268eb413133e2960fa4ae0e963ca76380c54cd9",
    ),
];

#[derive(Clone, Copy)]
enum WarningPolicy {
    Cargo,
    DelegatesWithCargo,
    IsolatedMsrv,
    NoWorkspaceCompilation(&'static str),
}

/// Every shell entry point, not just the ones already known to compile. A new
/// script fails until its warning-policy disposition is explicit, and each
/// disposition drives a check against the script rather than merely naming it.
const SCRIPT_WARNING_POLICY: &[(&str, WarningPolicy)] = &[
    (
        "audit-public-snapshot.sh",
        WarningPolicy::NoWorkspaceCompilation("audits a reconstructed Git repository"),
    ),
    (
        "build-public-snapshot.sh",
        WarningPolicy::NoWorkspaceCompilation("reconstructs a reviewed Git repository"),
    ),
    (
        "check-public-runner-boundary.sh",
        WarningPolicy::NoWorkspaceCompilation("checks GitHub repository metadata"),
    ),
    (
        "check-release-changelog.sh",
        WarningPolicy::NoWorkspaceCompilation("checks changelog text"),
    ),
    (
        "check-version-pins.sh",
        WarningPolicy::NoWorkspaceCompilation("checks version pins"),
    ),
    ("clippy.sh", WarningPolicy::Cargo),
    ("coverage.sh", WarningPolicy::Cargo),
    (
        "current-version.sh",
        WarningPolicy::NoWorkspaceCompilation("reads Cargo metadata without compiling"),
    ),
    ("doc.sh", WarningPolicy::Cargo),
    ("evidence.sh", WarningPolicy::Cargo),
    (
        "fmt.sh",
        WarningPolicy::NoWorkspaceCompilation("runs rustfmt without compiling"),
    ),
    (
        "install-cargo-deny.sh",
        WarningPolicy::NoWorkspaceCompilation(
            "installs and verifies the pinned supply-chain auditor",
        ),
    ),
    (
        "install-nextest.sh",
        WarningPolicy::NoWorkspaceCompilation("installs and verifies the pinned test runner"),
    ),
    (
        "install-cargo-ci-config.sh",
        WarningPolicy::NoWorkspaceCompilation("installs the shared hosted Cargo profile"),
    ),
    ("latest-stable.sh", WarningPolicy::Cargo),
    ("msrv-build.sh", WarningPolicy::IsolatedMsrv),
    ("preflight.sh", WarningPolicy::DelegatesWithCargo),
    (
        "prepare-hosted.sh",
        WarningPolicy::NoWorkspaceCompilation("prepares tools and queries cargo-llvm-cov"),
    ),
    (
        "publish-crates.sh",
        WarningPolicy::NoWorkspaceCompilation("packages and publishes workspace crates"),
    ),
    (
        "publish-github-release.sh",
        WarningPolicy::NoWorkspaceCompilation("publishes and verifies GitHub release assets"),
    ),
    (
        "pushed.sh",
        WarningPolicy::NoWorkspaceCompilation("observes a pushed workflow run"),
    ),
    (
        "release-source.sh",
        WarningPolicy::NoWorkspaceCompilation("validates an immutable Git source"),
    ),
    ("release.sh", WarningPolicy::Cargo),
    ("scale.sh", WarningPolicy::Cargo),
    ("spec.sh", WarningPolicy::Cargo),
    ("style.sh", WarningPolicy::Cargo),
    ("structural.sh", WarningPolicy::Cargo),
    ("supply-chain.sh", WarningPolicy::Cargo),
    ("test.sh", WarningPolicy::Cargo),
    ("tui-portability.sh", WarningPolicy::Cargo),
    (
        "toolchain.sh",
        WarningPolicy::NoWorkspaceCompilation("reads and verifies the toolchain contract"),
    ),
    ("validate.sh", WarningPolicy::DelegatesWithCargo),
    (
        "validation-generated.sh",
        WarningPolicy::NoWorkspaceCompilation("is generated validation data"),
    ),
];

fn shell_runs_workspace_compiler(line: &str) -> bool {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') || line.starts_with("echo ") {
        return false;
    }
    let words: Vec<_> = line
        .split(|character: char| character.is_ascii_whitespace() || "$(;\\\"".contains(character))
        .filter(|word| !word.is_empty())
        .collect();
    words.iter().enumerate().any(|(index, word)| {
        matches!(*word, "cargo" | "cargo_command")
            && words[index + 1..].iter().any(|candidate| {
                matches!(
                    *candidate,
                    "build" | "check" | "clippy" | "doc" | "run" | "test"
                )
            })
    })
}

fn first_shell_workspace_compilation(text: &str) -> Option<usize> {
    let mut logical = String::new();
    let mut start = 0;
    for (index, physical) in text.lines().enumerate() {
        let line = physical.trim();
        if logical.is_empty()
            && (line.is_empty() || line.starts_with('#') || line.starts_with("echo "))
        {
            continue;
        }
        if logical.is_empty() {
            start = index;
        }
        logical.push(' ');
        logical.push_str(line.strip_suffix('\\').unwrap_or(line));
        if line.ends_with('\\') {
            continue;
        }
        if shell_runs_workspace_compiler(&logical) {
            return Some(start);
        }
        logical.clear();
    }
    (!logical.is_empty() && shell_runs_workspace_compiler(&logical)).then_some(start)
}

fn shell_assignments<'a>(text: &'a str, variable: &str) -> Vec<(usize, &'a str)> {
    let needle = format!("{variable}=");
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.trim();
            (!line.starts_with('#')
                && line
                    .split_ascii_whitespace()
                    .any(|word| word.starts_with(&needle)))
            .then_some((index, line))
        })
        .collect()
}

fn yaml_values_for_key<'a>(
    value: &'a yaml_compat::Value,
    key: &str,
    found: &mut Vec<&'a yaml_compat::Value>,
) {
    match value {
        yaml_compat::Value::Mapping(mapping) => {
            for (candidate, value) in mapping {
                if candidate.as_str() == Some(key) {
                    found.push(value);
                }
                yaml_values_for_key(value, key, found);
            }
        }
        yaml_compat::Value::Sequence(sequence) => {
            for value in sequence {
                yaml_values_for_key(value, key, found);
            }
        }
        _ => {}
    }
}

fn audit_script_warning_disposition(
    name: &str,
    disposition: WarningPolicy,
    text: &str,
) -> Vec<String> {
    let cargo_policy = shell_assignments(text, "CARGO_BUILD_WARNINGS");
    let script_rustflags = shell_assignments(text, "RUSTFLAGS");
    let encoded_rustflags = shell_assignments(text, "CARGO_ENCODED_RUSTFLAGS");
    let first_compilation = first_shell_workspace_compilation(text);

    match disposition {
        WarningPolicy::Cargo | WarningPolicy::DelegatesWithCargo => {
            assert_eq!(
                cargo_policy,
                [(
                    text.lines()
                        .position(|line| line.trim() == "export CARGO_BUILD_WARNINGS=deny")
                        .unwrap_or(usize::MAX),
                    "export CARGO_BUILD_WARNINGS=deny"
                )],
                "ci/scripts/{name} must assign Cargo's warning policy exactly once and may not override it"
            );
            if matches!(disposition, WarningPolicy::Cargo) {
                let compilation = first_compilation.unwrap_or_else(|| {
                    panic!("ci/scripts/{name} is classified as compiling but drives no compiler")
                });
                assert!(
                    cargo_policy[0].0 < compilation,
                    "ci/scripts/{name} declares warning policy only after its first Cargo command"
                );
            } else {
                assert!(
                    first_compilation.is_none(),
                    "ci/scripts/{name} now compiles directly and needs the ordinary Cargo disposition"
                );
            }
        }
        WarningPolicy::IsolatedMsrv => {
            assert!(
                cargo_policy.is_empty(),
                "the old MSRV silently ignores Cargo's warning policy"
            );
            assert_eq!(
                script_rustflags,
                [(
                    text.lines()
                        .position(|line| line.trim() == "export RUSTFLAGS=\"-D warnings\"")
                        .unwrap_or(usize::MAX),
                    "export RUSTFLAGS=\"-D warnings\""
                )],
                "the isolated MSRV gate must have exactly one fail-closed rustflag assignment"
            );
            let target = text
                .lines()
                .position(|line| line.trim_start().starts_with("export CARGO_TARGET_DIR="))
                .expect("the MSRV gate must isolate its build target");
            let compilation = first_compilation.expect("the MSRV gate must compile the workspace");
            assert!(
                target < script_rustflags[0].0 && script_rustflags[0].0 < compilation,
                "the MSRV target isolation and warning policy must precede compilation"
            );
        }
        WarningPolicy::NoWorkspaceCompilation(reason) => {
            assert!(
                !reason.is_empty(),
                "the no-compilation disposition needs a reason"
            );
            assert!(
                first_compilation.is_none(),
                "ci/scripts/{name} now compiles the workspace; replace its no-compilation disposition ({reason})"
            );
            assert!(
                cargo_policy.is_empty()
                    && script_rustflags.is_empty()
                    && encoded_rustflags.is_empty(),
                "ci/scripts/{name} does not compile and must not mutate compiler warning flags"
            );
        }
    }

    script_rustflags
        .iter()
        .chain(&encoded_rustflags)
        .map(|(_, line)| format!("{name}:{line}"))
        .collect()
}

fn audit_script_warning_policy(scripts: &std::path::Path) {
    let mut actual_scripts: Vec<_> = std::fs::read_dir(scripts)
        .expect("read CI scripts")
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension().is_some_and(|extension| extension == "sh")).then(|| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
        })
        .collect();
    actual_scripts.sort();
    let mut classified_scripts: Vec<_> = SCRIPT_WARNING_POLICY
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect();
    classified_scripts.sort();
    assert_eq!(
        classified_scripts, actual_scripts,
        "every CI shell entry point must have one executable warning-policy disposition"
    );

    let mut rustflags = Vec::new();
    for (name, disposition) in SCRIPT_WARNING_POLICY {
        let text = std::fs::read_to_string(scripts.join(name))
            .unwrap_or_else(|error| panic!("read {name}: {error}"));
        rustflags.extend(audit_script_warning_disposition(name, *disposition, &text));
    }

    rustflags.sort();
    assert_eq!(
        rustflags,
        [
            "msrv-build.sh:export RUSTFLAGS=\"-D warnings\"",
            "release.sh:export CARGO_ENCODED_RUSTFLAGS=\"--remap-path-prefix=${REPO_ROOT}=/aplan/source${rustflags_separator}--remap-path-prefix=${HOME:?}=/aplan/home\"",
        ],
        "only the isolated MSRV policy and audited release path remapping may change rustc's artifact fingerprint"
    );
}

#[test]
fn release_artifact_paths_are_host_neutral() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let release = std::fs::read_to_string(root.join("ci/scripts/release.sh"))
        .expect("read the release builder");
    let remap = release
        .find("export CARGO_ENCODED_RUSTFLAGS=\"--remap-path-prefix=${REPO_ROOT}=/aplan/source")
        .expect("release builds remap their source and home paths");
    let first_build = release
        .find("cargo build")
        .expect("release builds with Cargo");
    let retained_prefix_check = release
        .find("grep -aFq \"$private_build_prefix\" \"$binary\"")
        .expect("release builds inspect the produced binary for retained paths");
    let packaging = release
        .find("echo \"==> Packaging ${archive}\"")
        .expect("release packaging boundary");
    assert!(
        remap < first_build,
        "path remapping starts after compilation"
    );
    assert!(
        retained_prefix_check < packaging,
        "a binary can be packaged before its private paths are refused"
    );
    for required in [
        "if [[ -n \"${RUSTFLAGS:-}\" || -n \"${CARGO_ENCODED_RUSTFLAGS:-}\" ]]",
        "--remap-path-prefix=${HOME:?}=/aplan/home",
        "for private_build_prefix in \"${REPO_ROOT}/\" \"${HOME}/\"",
    ] {
        assert!(
            release.contains(required),
            "release path contract lost {required}"
        );
    }
}

/// `A2-012`: a release artifact has one native build environment, not one per
/// caller. v0.5.2 proved why the Rust pin alone is insufficient: Arch GCC 16
/// and hosted Ubuntu's compiler produced different libgit2/zlib objects from
/// the same tagged Rust source. The platform digest freezes the upstream image,
/// the Debian snapshot freezes every added native package, and both local and
/// hosted callers enter that environment through the same script.
#[test]
fn release_artifact_builder_pins_native_inputs_for_every_caller() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let builder = std::fs::read_to_string(root.join("ci/release.Containerfile"))
        .expect("read release builder");
    let release = std::fs::read_to_string(root.join("ci/scripts/release.sh"))
        .expect("read release entry point");
    let hosted = std::fs::read_to_string(root.join("ci/scripts/prepare-hosted.sh"))
        .expect("read hosted preparation");
    let workflow = std::fs::read_to_string(root.join(".github/workflows/release.yml"))
        .expect("read release workflow");
    let dockerignore =
        std::fs::read_to_string(root.join(".dockerignore")).expect("read Docker context policy");
    let containerignore =
        std::fs::read_to_string(root.join(".containerignore")).expect("read Podman context policy");
    let toolchain = std::fs::read_to_string(root.join("rust-toolchain.toml"))
        .expect("read Rust toolchain contract");

    let from = builder
        .lines()
        .find(|line| line.starts_with("FROM "))
        .expect("release builder has a base image");
    let (image, digest) = from
        .strip_prefix("FROM ghcr.io/rust-lang/rust:")
        .and_then(|value| value.split_once("@sha256:"))
        .expect("release builder pins the official Rust image by platform digest");
    assert_eq!(digest.len(), 64, "release builder digest is not SHA-256");
    assert!(
        digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "release builder digest is not hexadecimal"
    );
    let channel = toolchain
        .lines()
        .find_map(|line| line.trim().strip_prefix("channel = \"")?.strip_suffix('"'))
        .expect("toolchain channel");
    assert!(
        image.starts_with(&format!("{channel}-")),
        "release builder tag {image} does not name toolchain {channel}"
    );

    for required in [
        "ARG DEBIAN_SNAPSHOT=20261101T000000Z",
        "snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}/",
        "snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}/",
        "git \\",
        "musl-tools \\",
        "pkg-config \\",
        "zlib1g-dev \\",
        "actual=\"$(rustc --version | awk '{print $2}')\"",
        "test \"$actual\" = \"$required\"",
        "rustup target add x86_64-unknown-linux-musl --toolchain \"$required\"",
    ] {
        assert!(
            builder.contains(required),
            "release builder lost {required}"
        );
    }

    for required in [
        "if [[ \"${APLAN_RELEASE_IN_BUILDER:-}\" != 1 ]]",
        "--file ci/release.Containerfile",
        "--platform linux/amd64",
        "--volume \"$REPO_ROOT:/aplan/source:ro\"",
        "git rev-parse --path-format=absolute --git-common-dir",
        "git_mount=(--volume \"$git_common:$git_common:ro\")",
        "--env APLAN_RELEASE_IN_BUILDER=1",
        "--env CARGO_TARGET_DIR=/aplan/build/target",
        "${APLAN_RELEASE_SCRATCH_ROOT:-${HOME:?}/.tmp}",
    ] {
        assert!(
            release.contains(required),
            "release entry point lost {required}"
        );
    }
    assert!(
        workflow.contains("run: ci/scripts/release.sh"),
        "hosted publication bypasses the shared release entry point"
    );
    assert!(
        hosted.contains("for command in docker gh jq; do")
            && hosted.contains("docker info >/dev/null")
            && hosted.contains("docker build --help | grep -q -- '--platform'")
            && !hosted.contains("musl-tools")
            && !hosted.contains("rustup target add x86_64-unknown-linux-musl"),
        "hosted preparation must provide the engine, not a second native artifact toolchain"
    );
    for allowed in [
        "**",
        "!rust-toolchain.toml",
        "!ci/release.Containerfile",
        "!ci/scripts/toolchain.sh",
    ] {
        assert!(
            dockerignore.lines().any(|line| line == allowed),
            "release builder context lost {allowed}"
        );
    }
    let docker_policy = dockerignore
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    let podman_policy = containerignore
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    assert_eq!(
        docker_policy, podman_policy,
        "Docker and Podman must build from the same bounded context"
    );
}

fn audit_workflow_warning_policy() {
    let files = workflows();
    let mut workflow_rustflags = Vec::new();
    let mut workflow_warning_policy = Vec::new();
    for (file, document) in &files {
        for key in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"] {
            let mut values = Vec::new();
            yaml_values_for_key(document, key, &mut values);
            workflow_rustflags.extend(
                values
                    .into_iter()
                    .map(|value| format!("{file}: {key}={value:?}")),
            );
        }
        let mut values = Vec::new();
        yaml_values_for_key(document, "CARGO_BUILD_WARNINGS", &mut values);
        workflow_warning_policy.extend(
            values
                .into_iter()
                .map(|value| (file.as_str(), value.as_str())),
        );
    }
    assert!(
        workflow_rustflags.is_empty(),
        "workflow-level RUSTFLAGS fork pinned artifacts:\n  - {}",
        workflow_rustflags.join("\n  - ")
    );
    assert!(
        workflow_warning_policy
            .iter()
            .all(|(_, value)| *value == Some("deny")),
        "a nested workflow environment must not weaken or invalidate CARGO_BUILD_WARNINGS: {workflow_warning_policy:?}"
    );
    for name in ["ci.yml", "release.yml", "scale.yml", "tui-portability.yml"] {
        let document = files
            .iter()
            .find_map(|(file, document)| (file == name).then_some(document))
            .unwrap_or_else(|| panic!("{name} is absent from the workflow census"));
        assert_eq!(
            document
                .get("env")
                .and_then(|env| env.get("CARGO_BUILD_WARNINGS"))
                .and_then(yaml_compat::Value::as_str),
            Some("deny"),
            "{name} must deny warnings before hosted preparation or any workspace build"
        );
    }
}

/// `A1-631`: warning denial is a Cargo policy on every ordinary compiler. The
/// MSRV's isolated target and the release's host-path remapping are the only
/// deliberate rustflag exceptions.
#[test]
fn every_compiling_gate_denies_warnings_without_forking_pinned_artifacts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    audit_script_warning_policy(&root.join("ci/scripts"));
    audit_workflow_warning_policy();
}

#[test]
fn warning_policy_census_sees_multiline_commands_assignments_and_nested_overrides() {
    assert_eq!(
        first_shell_workspace_compilation("cargo \\\n            test --workspace"),
        Some(0)
    );
    assert_eq!(
        first_shell_workspace_compilation("echo cargo test\n# cargo build\ncargo fmt --all"),
        None
    );
    assert_eq!(
        first_shell_workspace_compilation("\"$cargo_command\" run -p xtask -- dependency-audit"),
        Some(0)
    );
    assert_eq!(
        shell_assignments(
            "# RUSTFLAGS=-D warnings\nenv RUSTFLAGS=-Dwarnings cargo test",
            "RUSTFLAGS"
        ),
        [(1, "env RUSTFLAGS=-Dwarnings cargo test")]
    );
    assert!(
        shell_assignments(
            "export CARGO_ENCODED_RUSTFLAGS=--remap-path-prefix=/private=/public",
            "RUSTFLAGS"
        )
        .is_empty()
    );

    let workflow: yaml_compat::Value = yaml_compat::from_str(
        "env:\n  CARGO_BUILD_WARNINGS: deny\njobs:\n  test:\n    env:\n      CARGO_BUILD_WARNINGS: allow\n",
    )
    .expect("parse nested warning-policy fixture");
    let mut values = Vec::new();
    yaml_values_for_key(&workflow, "CARGO_BUILD_WARNINGS", &mut values);
    assert_eq!(
        values
            .into_iter()
            .map(yaml_compat::Value::as_str)
            .collect::<Vec<_>>(),
        [Some("deny"), Some("allow")]
    );
}

fn audit_consolidated_sources(
    name: &str,
    manifest: &std::path::Path,
    integration_targets: &[&serde_json::Value],
) {
    let tests = manifest.parent().expect("package directory").join("tests");
    let Ok(entries) = std::fs::read_dir(&tests) else {
        return;
    };
    let sources: std::collections::BTreeSet<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .filter_map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect();
    if sources.len() <= 1 {
        return;
    }

    assert_eq!(
        integration_targets.len(),
        1,
        "workspace crate {name} has multiple integration sources but no single harness"
    );
    let harness = std::path::Path::new(
        integration_targets[0]["src_path"]
            .as_str()
            .expect("integration target source path"),
    );
    let harness = std::fs::read_to_string(harness)
        .unwrap_or_else(|error| panic!("read {}: {error}", harness.display()));
    let reached: std::collections::BTreeSet<_> = harness
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("#[path = \"../")
                .and_then(|line| line.strip_suffix("\"]"))
        })
        .filter(|path| !path.contains('/'))
        .map(str::to_owned)
        .collect();
    for (index, line) in harness.lines().enumerate() {
        let Some(path) = line
            .trim()
            .strip_prefix("#[path = \"../")
            .and_then(|line| line.strip_suffix("\"]"))
            .filter(|path| !path.contains('/'))
        else {
            continue;
        };
        let module = path.strip_suffix(".rs").expect("top-level Rust source");
        let declaration = format!("mod {module};");
        assert_eq!(
            harness.lines().nth(index + 1).map(str::trim),
            Some(declaration.as_str()),
            "workspace crate {name}'s harness must use source stem `{module}` as its module prefix"
        );
    }
    assert_eq!(
        reached, sources,
        "workspace crate {name}'s single integration harness must reach every top-level test source exactly once"
    );
}

/// `A1-630`: Cargo's resolved target graph is the authority for how many test
/// executables exist. The source-to-harness comparison closes the other side of
/// `autotests = false`: a newly added test file must not silently disappear.
#[test]
fn every_workspace_crate_has_at_most_one_integration_target_and_all_sources_are_reached() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--no-deps", "--format-version=1"])
        .current_dir(&root)
        .output()
        .expect("run cargo metadata for the integration-target census");
    assert!(
        output.status.success(),
        "cargo metadata failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata JSON");
    let members: std::collections::BTreeSet<_> = metadata["workspace_members"]
        .as_array()
        .expect("workspace_members")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();

    for package in metadata["packages"].as_array().expect("packages") {
        let id = package["id"].as_str().expect("package id");
        if !members.contains(id) {
            continue;
        }
        let name = package["name"].as_str().expect("package name");
        let integration_targets: Vec<_> = package["targets"]
            .as_array()
            .expect("targets")
            .iter()
            .filter(|target| {
                target["kind"]
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind == "test"))
            })
            .collect();
        assert!(
            integration_targets.len() <= 1,
            "workspace crate {name} has {} integration-test binaries: {:?}",
            integration_targets.len(),
            integration_targets
                .iter()
                .filter_map(|target| target["name"].as_str())
                .collect::<Vec<_>>()
        );
        let manifest =
            std::path::Path::new(package["manifest_path"].as_str().expect("manifest path"));
        audit_consolidated_sources(name, manifest, &integration_targets);
    }
}

fn assert_once(text: &str, needle: &str, owner: &str) {
    assert_eq!(
        text.matches(needle).count(),
        1,
        "{owner} must contain `{needle}` exactly once"
    );
}

/// `A1-632`: the image, container and job hook form one resource contract. A
/// setting in only one layer is a comment, not a bound, so this census requires
/// each declaration to drive its corresponding runtime verification.
#[test]
fn private_runner_resource_limits_and_cleanup_hook_are_persisted_and_verified() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let containerfile = std::fs::read_to_string(root.join("ci/Containerfile"))
        .expect("read private-runner Containerfile");
    let setup = std::fs::read_to_string(root.join("ci/runner/setup.sh"))
        .expect("read private-runner setup");
    let hook = std::fs::read_to_string(root.join("ci/runner/job-started.sh"))
        .expect("read private-runner job-started hook");
    let reclaimer = std::fs::read_to_string(root.join("ci/runner/reclaim-coverage.py"))
        .expect("read private-runner coverage reclaimer");
    let cargo_config = std::fs::read_to_string(root.join("ci/cargo-ci-config.toml"))
        .expect("read shared CI Cargo config");

    for setting in [
        "COPY --chown=runner:runner ci/cargo-ci-config.toml /usr/local/cargo/config.toml",
        "COPY ci/runner/job-started.sh /home/runner/job-started.sh",
        "COPY ci/runner/reclaim-coverage.py /home/runner/reclaim-coverage.py",
    ] {
        assert!(
            containerfile.contains(setting),
            "runner image lost build-cost setting `{setting}`"
        );
    }
    for setting in [
        "[build]",
        "incremental = false",
        "[profile.dev]",
        "[profile.dev.package.\"*\"]",
        "debug = false",
        "[profile.test]",
    ] {
        assert!(
            cargo_config.contains(setting),
            "shared CI Cargo config lost `{setting}`"
        );
    }
    assert_eq!(
        cargo_config.matches("debug = \"line-tables-only\"").count(),
        2,
        "dev and test profiles must both retain line tables"
    );

    for setting in [
        "RUNNER_CPUS=\"${RUNNER_CPUS:-8}\"",
        "RUNNER_CPU_SHARES=\"${RUNNER_CPU_SHARES:-64}\"",
        "RUNNER_MEMORY_HIGH_BYTES=\"${RUNNER_MEMORY_HIGH_BYTES:-25769803776}\"",
        "RUNNER_TARGET_CAP_GIB=\"${RUNNER_TARGET_CAP_GIB:-100}\"",
        "--cpus \"${RUNNER_CPUS}\"",
        "--cpu-shares \"${RUNNER_CPU_SHARES}\"",
        "--cgroup-conf \"memory.high=${RUNNER_MEMORY_HIGH_BYTES}\"",
        "-e ACTIONS_RUNNER_HOOK_JOB_STARTED=/home/runner/job-started.sh",
        "verify_equal NanoCpus",
        "verify_equal CpuShares",
        "verify_equal memory.high-config",
        "verify_equal memory.high",
        "verify_equal cpu.max",
    ] {
        assert!(
            setup.contains(setting),
            "runner setup lost resource declaration or verification `{setting}`"
        );
    }
    assert_once(&setup, "podman create \\", "runner setup");
    assert_once(&setup, "podman start \"${CONTAINER_NAME}\"", "runner setup");

    for boundary in [
        "/usr/bin/python3 \"$HOOK_DIR/reclaim-coverage.py\"",
        "TARGET_CAP_GIB=\"${TARGET_CAP_GIB:-100}\"",
    ] {
        assert!(
            hook.contains(boundary),
            "runner cleanup hook lost deletion boundary `{boundary}`"
        );
    }
    for boundary in [
        "^\\.aplan-coverage-([1-9][0-9]*)-(0|[1-9][0-9]*)$",
        ".aplan-coverage-owner-v1.json",
        "candidate.lstat()",
        "os.O_NOFOLLOW",
        "process_state(owner, current_boot)",
        "wanted_group == group",
        "APLAN_COVERAGE_LEASE=",
        "shutil.rmtree.avoids_symlink_attacks",
        "shutil.rmtree(candidate)",
        "--legacy-quiescent",
    ] {
        assert!(
            reclaimer.contains(boundary),
            "coverage reclaimer lost ownership or deletion boundary `{boundary}`"
        );
    }
}

fn cargo_fixture(
    directory: &std::path::Path,
    target: &std::path::Path,
    deny_warnings: bool,
) -> std::process::Output {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .args([
            "test",
            "--offline",
            "--no-run",
            "--verbose",
            "--color",
            "never",
        ])
        .current_dir(directory)
        .env("CARGO_TARGET_DIR", target)
        // Make the fixture prove its command-line presentation boundary rather
        // than inheriting a convenient local default (`A1-641`).
        .env("CARGO_TERM_COLOR", "always")
        .env_remove("CARGO_BUILD_WARNINGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS");
    if deny_warnings {
        command.env("CARGO_BUILD_WARNINGS", "deny");
    }
    command.output().expect("run Cargo warning-policy fixture")
}

fn assert_warning_fixture_fresh(output: &std::process::Output, context: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{context} failed:\n{stderr}");
    assert!(
        !stderr.as_bytes().contains(&0x1b),
        "{context} retained terminal escapes:\n{stderr}"
    );
    assert!(
        stderr.contains("Fresh warning-policy-fixture")
            && !stderr.contains("Compiling warning-policy-fixture")
            && !stderr.contains("Dirty warning-policy-fixture"),
        "{context} forked or rebuilt the artifact:\n{stderr}"
    );
}

/// `A1-631`: Cargo's policy rejects a real compiler warning but does not make a
/// warning-free package stale after an ordinary build.
#[test]
fn cargo_warning_policy_is_fail_closed_and_fingerprint_neutral() {
    let fixture = tempfile::Builder::new()
        .prefix("cargo-warning-policy-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create Cargo warning-policy fixture");
    std::fs::create_dir(fixture.path().join("src")).expect("create fixture source directory");
    std::fs::write(
        fixture.path().join("Cargo.toml"),
        "[package]\nname = \"warning-policy-fixture\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .expect("write fixture manifest");
    let source = fixture.path().join("src/lib.rs");
    std::fs::write(
        &source,
        "pub fn answer() -> usize { let unused = 41; 42 }\n",
    )
    .expect("write warning-bearing source");
    let target = fixture.path().join("target");

    let allowed = cargo_fixture(fixture.path(), &target, false);
    assert!(
        allowed.status.success() && String::from_utf8_lossy(&allowed.stderr).contains("unused"),
        "negative control did not compile and report the seeded warning:\n{}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    let denied = cargo_fixture(fixture.path(), &target, true);
    assert!(
        !denied.status.success()
            && String::from_utf8_lossy(&denied.stderr).contains("unused variable"),
        "CARGO_BUILD_WARNINGS=deny did not reject the seeded warning:\n{}",
        String::from_utf8_lossy(&denied.stderr)
    );

    std::fs::write(&source, "pub fn answer() -> usize { 42 }\n").expect("remove fixture warning");
    let ordinary = cargo_fixture(fixture.path(), &target, false);
    assert!(
        ordinary.status.success(),
        "warning-free ordinary build failed:\n{}",
        String::from_utf8_lossy(&ordinary.stderr)
    );
    let gated = cargo_fixture(fixture.path(), &target, true);
    assert_warning_fixture_fresh(&gated, "changing only Cargo's warning policy");
    let ordinary_again = cargo_fixture(fixture.path(), &target, false);
    assert_warning_fixture_fresh(&ordinary_again, "removing only Cargo's warning policy");
}

fn workflows() -> Vec<(String, yaml_compat::Value)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .expect("the workflows directory")
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|x| x == "yml" || x == "yaml") {
            let text = std::fs::read_to_string(&path).expect("read a workflow");
            let doc: yaml_compat::Value = yaml_compat::from_str(&text)
                .unwrap_or_else(|e| panic!("{} does not parse: {e}", path.display()));
            out.push((
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                doc,
            ));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A1-647: the private runner deliberately puts build output in a persistent
/// external target directory. The rollout producer must consume Cargo's actual
/// output location rather than a workspace-relative path that happens to work
/// on a fresh GitHub-hosted runner.
#[test]
fn evidence_rollout_producer_honours_cargo_target_dir() {
    let files = workflows();
    let produce = job(&files, "evidence-rollout.yml", "produce");
    let command = produce["steps"]
        .as_sequence()
        .expect("evidence rollout producer has steps")
        .iter()
        .find(|step| step["name"].as_str() == Some("record direct oracle and receipt artifact"))
        .and_then(|step| step["run"].as_str())
        .expect("evidence rollout producer command");
    assert!(
        command.contains("--binary \"${CARGO_TARGET_DIR:-target}/release/aplan\""),
        "the producer must find the release binary in Cargo's configured target directory: {command}"
    );
    assert!(
        !command.contains("--binary target/release/aplan"),
        "a workspace-relative binary path breaks the private runner: {command}"
    );
}

/// `A1-616`: an external action is executable supply-chain input, not a
/// cosmetic workflow label. Enumerate every real step from parsed YAML and
/// require the reviewed immutable object identity.
#[test]
fn every_external_workflow_action_is_pinned_and_censused() {
    let files = workflows();
    let mut observed = std::collections::BTreeMap::<String, usize>::new();

    for (file, document) in &files {
        let jobs = document["jobs"]
            .as_mapping()
            .unwrap_or_else(|| panic!("{file} has no jobs mapping"));
        for (job_name, job) in jobs {
            let Some(steps) = job["steps"].as_sequence() else {
                continue;
            };
            for (index, step) in steps.iter().enumerate() {
                let Some(reference) = step.get("uses").and_then(yaml_compat::Value::as_str) else {
                    continue;
                };
                if reference.starts_with("./") {
                    continue;
                }
                let (action, revision) = reference.split_once('@').unwrap_or_else(|| {
                    panic!(
                        "{file}:{job_name:?}:step {} action `{reference}` has no ref",
                        index + 1
                    )
                });
                let expected = PINNED_ACTIONS
                    .iter()
                    .find_map(|(candidate, sha)| (*candidate == action).then_some(*sha))
                    .unwrap_or_else(|| {
                        panic!(
                            "{file}:{job_name:?}:step {} action `{action}` is absent from PINNED_ACTIONS",
                            index + 1
                        )
                    });
                assert_eq!(
                    revision,
                    expected,
                    "{file}:{job_name:?}:step {} must pin `{action}` to its reviewed commit",
                    index + 1
                );
                *observed.entry(action.to_string()).or_default() += 1;
            }
        }
    }

    assert_eq!(
        observed.keys().map(String::as_str).collect::<Vec<_>>(),
        PINNED_ACTIONS
            .iter()
            .map(|(action, _)| *action)
            .collect::<Vec<_>>(),
        "the pinned-action census contains a held or unreachable entry"
    );
}

/// `A1-616`: hosted measurements found no net win from dependency caches,
/// executable target caches or nextest archive fanout. Keep the ordinary CI
/// topology direct until a later same-gate measurement reverses that result.
#[test]
fn hosted_ci_reuse_stays_direct_after_the_measured_rejection() {
    let files = workflows();
    let workflow = files
        .iter()
        .find_map(|(file, document)| (file == "ci.yml").then_some(document))
        .expect("ci workflow");
    let jobs = workflow["jobs"].as_mapping().expect("ci jobs mapping");

    let mut uploads = Vec::new();
    let mut downloads = Vec::new();
    for (name, job) in jobs {
        for action in job["steps"]
            .as_sequence()
            .into_iter()
            .flatten()
            .filter_map(|step| step["uses"].as_str())
        {
            assert!(
                !action.starts_with("actions/cache"),
                "ci.yml:{name:?} reintroduced rejected cache action `{action}` without a new measurement"
            );
            if action.starts_with("actions/upload-artifact") {
                uploads.push(name);
            }
            if action.starts_with("actions/download-artifact") {
                downloads.push(name);
            }
        }
    }
    assert!(
        uploads.is_empty() || downloads.is_empty(),
        "ci.yml reintroduced rejected archive fanout from {uploads:?} to {downloads:?} without a new measurement"
    );
}

const DEVELOPMENT_EVENTS: &str =
    "github.event_name == 'push' || github.event_name == 'pull_request'";
const FULL_EVENTS: &str =
    "github.event_name == 'schedule' || github.event_name == 'workflow_dispatch'";
const VALIDATION_CONCURRENCY_GROUP: &str = "${{ (github.event_name == 'pull_request' && format('{0}-development-pr-{1}', github.workflow, github.event.pull_request.number)) || (github.event_name == 'push' && format('{0}-development-ref-{1}', github.workflow, github.ref)) || format('{0}-full-run-{1}', github.workflow, github.run_id) }}";
const DEVELOPMENT_CANCELLATION: &str =
    "${{ github.event_name == 'push' || github.event_name == 'pull_request' }}";

fn events_in_condition(condition: &str) -> std::collections::BTreeSet<&str> {
    condition
        .split(" || ")
        .map(|clause| {
            clause
                .strip_prefix("github.event_name == '")
                .and_then(|clause| clause.strip_suffix('\''))
                .unwrap_or_else(|| panic!("unsupported event condition clause `{clause}`"))
        })
        .collect()
}

fn validation_group(event: &str, identity: &str, run_id: u64) -> (String, bool) {
    match event {
        "pull_request" => (format!("CI-development-pr-{identity}"), true),
        "push" => (format!("CI-development-ref-{identity}"), true),
        "schedule" | "workflow_dispatch" => (format!("CI-full-run-{run_id}"), false),
        _ => panic!("event `{event}` is outside the validation event matrix"),
    }
}

fn assert_validation_trigger_contract(workflow: &yaml_compat::Value) {
    let triggers = workflow["on"].as_mapping().expect("CI trigger mapping");
    let trigger_names = triggers
        .keys()
        .map(|key| key.as_str().expect("string trigger"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        trigger_names,
        ["pull_request", "push", "schedule", "workflow_dispatch"]
            .into_iter()
            .collect(),
        "CI trigger surface changed"
    );
    for event in ["push", "pull_request"] {
        let policy = triggers[event]
            .as_mapping()
            .unwrap_or_else(|| panic!("{event} trigger policy"));
        assert_eq!(
            policy["branches"].as_sequence().map(|branches| {
                branches
                    .iter()
                    .filter_map(yaml_compat::Value::as_str)
                    .collect::<Vec<_>>()
            }),
            Some(vec!["main"]),
            "{event} must cover the complete main integration surface"
        );
        for filter in ["paths", "paths-ignore"] {
            assert!(
                policy.get(filter).is_none(),
                "{event} uses `{filter}`, which can leave a required check Pending and omit a no-op verdict"
            );
        }
    }
    let schedules = triggers["schedule"]
        .as_sequence()
        .expect("schedule trigger list");
    assert_eq!(schedules.len(), 1, "CI must have one authoritative nightly");
    assert_eq!(schedules[0]["cron"].as_str(), Some("37 7 * * *"));

    let concurrency = workflow["concurrency"]
        .as_mapping()
        .expect("CI concurrency policy");
    assert_eq!(
        concurrency["group"].as_str(),
        Some(VALIDATION_CONCURRENCY_GROUP)
    );
    assert_eq!(
        concurrency["cancel-in-progress"].as_str(),
        Some(DEVELOPMENT_CANCELLATION)
    );
}

fn validation_event_routes(
    workflow: &yaml_compat::Value,
) -> std::collections::BTreeMap<&'static str, Vec<&'static str>> {
    let jobs = workflow["jobs"].as_mapping().expect("CI jobs mapping");
    let job_names = jobs
        .keys()
        .map(|key| key.as_str().expect("string job name"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        job_names,
        ["development", "full"].into_iter().collect(),
        "CI must route through exactly the two authoritative validation profiles"
    );
    let expected_jobs = [
        (
            "development",
            "development validation",
            DEVELOPMENT_EVENTS,
            "ci/scripts/preflight.sh --profile \"$APLAN_ORDINARY_PROFILE\" --base \"$APLAN_VALIDATION_BASE\" --head \"$APLAN_VALIDATION_HEAD\"",
        ),
        (
            "full",
            "full validation",
            FULL_EVENTS,
            "ci/scripts/validate.sh --profile full",
        ),
    ];
    let mut routed = std::collections::BTreeMap::<&str, Vec<&str>>::new();
    for (job_name, status_name, condition, command) in expected_jobs {
        let job = jobs
            .get(yaml_compat::Value::String(job_name.to_owned()))
            .unwrap_or_else(|| panic!("missing {job_name} job"));
        assert_eq!(job["name"].as_str(), Some(status_name));
        assert_eq!(job["if"].as_str(), Some(condition));
        assert!(job.get("continue-on-error").is_none());
        assert!(job.get("needs").is_none());
        for event in events_in_condition(condition) {
            routed.entry(event).or_default().push(job_name);
        }
        let steps = job["steps"].as_sequence().expect("job steps");
        assert_eq!(
            steps
                .iter()
                .filter(|step| step["run"].as_str() == Some(command))
                .count(),
            1,
            "{job_name} must invoke its authoritative profile exactly once"
        );
        let checkout = steps
            .iter()
            .find(|step| {
                step["uses"]
                    .as_str()
                    .is_some_and(|uses| uses.starts_with("actions/checkout@"))
            })
            .expect("checkout step");
        assert_eq!(checkout["with"]["fetch-depth"].as_u64(), Some(0));
    }
    routed
}

fn assert_development_source_contract(workflow: &yaml_compat::Value) {
    let jobs = workflow["jobs"].as_mapping().expect("CI jobs mapping");
    let development = &jobs["development"];
    assert_eq!(
        development["env"]["APLAN_ORDINARY_PROFILE"].as_str(),
        Some("development"),
        "ordinary CI must use the fast profile until the explicit rollback switch is changed"
    );
    assert_eq!(
        development["timeout-minutes"].as_u64(),
        Some(60),
        "the ordinary job must retain enough time for its one-change full rollback"
    );
    let invocation = development["steps"]
        .as_sequence()
        .expect("development steps")
        .iter()
        .find(|step| step["name"].as_str() == Some("change-aware development validation"))
        .expect("development invocation");
    assert_eq!(
        invocation["env"]["APLAN_VALIDATION_BASE"].as_str(),
        Some(
            "${{ github.event_name == 'pull_request' && github.event.pull_request.base.sha || github.event.before }}"
        )
    );
    assert_eq!(
        invocation["env"]["APLAN_VALIDATION_HEAD"].as_str(),
        Some("${{ github.sha }}")
    );
}

/// `A1-618`: one reviewed value controls both environment preparation and
/// validation execution. The two executable aliases prove that changing that
/// value to `full` cannot leave a hosted rollback without coverage tooling.
#[test]
fn ordinary_profile_has_one_complete_full_rollback_switch() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let preparation = root.join("ci/scripts/prepare-hosted.sh");
    for (profile, expected) in [("development", "test"), ("full", "coverage")] {
        let output = Command::new(&preparation)
            .args(["--resolve-profile", profile])
            .output()
            .expect("resolve hosted validation preparation profile");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout)
                .expect("profile output is UTF-8")
                .trim(),
            expected
        );
    }

    let timing = tempfile::Builder::new()
        .prefix("validation-setup-timing-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create setup timing fixture");
    for (script, arguments, file) in [
        (
            root.join("ci/scripts/prepare-hosted.sh"),
            vec!["--resolve-profile", "development"],
            "hosted.env",
        ),
        (
            root.join("ci/scripts/install-nextest.sh"),
            vec!["unknown-mode"],
            "self-hosted.env",
        ),
    ] {
        let environment = timing.path().join(file);
        let _ = Command::new(script)
            .args(arguments)
            .env("GITHUB_ENV", &environment)
            .env_remove("APLAN_VALIDATION_SETUP_STARTED_EPOCH")
            .output()
            .expect("exercise setup timing boundary");
        let marker = std::fs::read_to_string(environment).expect("read setup timing marker");
        assert!(
            marker
                .strip_prefix("APLAN_VALIDATION_SETUP_STARTED_EPOCH=")
                .and_then(|value| value.trim().parse::<u64>().ok())
                .is_some(),
            "{file} did not publish a valid setup start: {marker:?}"
        );
    }

    let workflow = std::fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .expect("read validation workflow");
    assert_eq!(
        workflow
            .matches("APLAN_ORDINARY_PROFILE: development")
            .count(),
        1
    );
    assert_eq!(
        workflow
            .matches("prepare-hosted.sh \"$APLAN_ORDINARY_PROFILE\"")
            .count(),
        1
    );
    assert_eq!(
        workflow
            .matches("preflight.sh --profile \"$APLAN_ORDINARY_PROFILE\"")
            .count(),
        1
    );
    let rolled_back = workflow.replacen(
        "APLAN_ORDINARY_PROFILE: development",
        "APLAN_ORDINARY_PROFILE: full",
        1,
    );
    assert_eq!(
        workflow
            .lines()
            .zip(rolled_back.lines())
            .filter(|(before, after)| before != after)
            .count(),
        1,
        "full-on-every-push rollback must be one routing-line change"
    );
}

/// `A2-011`: the release profile executes coverage before publication, so its
/// hosted preparation must compose the coverage toolchain with release-only
/// artifact and publication tools. This invokes the same capability decision
/// used by the installer instead of maintaining a second expectation in YAML.
#[test]
fn hosted_release_prepares_every_tool_required_by_release_validation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let preparation = root.join("ci/scripts/prepare-hosted.sh");

    for (profile, expected) in [
        ("development", Vec::<&str>::new()),
        ("full", vec!["coverage"]),
        ("coverage", vec!["coverage"]),
        ("release", vec!["coverage", "release"]),
    ] {
        let output = Command::new(&preparation)
            .args(["--resolve-capabilities", profile])
            .output()
            .expect("resolve hosted preparation capabilities");
        assert!(
            output.status.success(),
            "{profile}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout)
                .expect("capability output is UTF-8")
                .lines()
                .collect::<Vec<_>>(),
            expected,
            "hosted preparation capabilities drifted for {profile}"
        );
    }
}

fn assert_validation_cancellation_contract() {
    let (push_a, cancel_push) = validation_group("push", "refs/heads/main", 1);
    let (push_b, _) = validation_group("push", "refs/heads/main", 2);
    let (pull_request, cancel_pr) = validation_group("pull_request", "42", 3);
    let (scheduled_a, cancel_scheduled) = validation_group("schedule", "", 4);
    let (scheduled_b, _) = validation_group("schedule", "", 5);
    let (manual, cancel_manual) = validation_group("workflow_dispatch", "", 6);
    assert_eq!(
        push_a, push_b,
        "newer pushes on one ref must supersede older ones"
    );
    assert_ne!(push_a, pull_request);
    assert_ne!(
        scheduled_a, scheduled_b,
        "full runs must never replace each other"
    );
    assert_ne!(scheduled_a, manual, "full runs need unique identities");
    assert!(cancel_push && cancel_pr);
    assert!(!cancel_scheduled && !cancel_manual);
}

/// `A1-617`: the workflow is a cadence router, not another copy of the gate
/// inventory. Derive the event/job matrix from parsed YAML, then drive every
/// admitted event through it. This catches a new trigger, a missing verdict,
/// a path-filtered no-op change, or a full run that shares a cancellable group
/// with development validation.
#[test]
fn validation_workflow_routes_every_event_to_one_non_divergent_profile() {
    let files = workflows();
    let workflow = files
        .iter()
        .find_map(|(file, document)| (file == "ci.yml").then_some(document))
        .expect("ci workflow");
    assert_validation_trigger_contract(workflow);
    let expected_events = ["pull_request", "push", "schedule", "workflow_dispatch"]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let routed = validation_event_routes(workflow);
    for event in &expected_events {
        assert_eq!(
            routed.get(event).map(Vec::as_slice),
            Some(
                [if matches!(*event, "push" | "pull_request") {
                    "development"
                } else {
                    "full"
                }]
                .as_slice()
            ),
            "{event} must produce one stable verdict"
        );
    }
    assert_eq!(routed.len(), expected_events.len());
    assert_development_source_contract(workflow);
    assert_validation_cancellation_contract();

    let release = files
        .iter()
        .find_map(|(file, document)| (file == "release.yml").then_some(document))
        .expect("release workflow");
    assert!(
        release.get("concurrency").is_none(),
        "release publication must not be cancelled or replaced by another validation run"
    );
}

/// `A1-616`: runner placement cannot silently select a different Cargo build
/// profile. The committed file is copied into the private image and installed
/// before any hosted repository compilation.
#[test]
fn private_and_hosted_runners_share_one_ci_cargo_profile() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let container =
        std::fs::read_to_string(root.join("ci/Containerfile")).expect("read private runner image");
    let hosted = std::fs::read_to_string(root.join("ci/scripts/prepare-hosted.sh"))
        .expect("read hosted preparation");
    let installer = std::fs::read_to_string(root.join("ci/scripts/install-cargo-ci-config.sh"))
        .expect("read CI Cargo profile installer");
    let containerignore = std::fs::read_to_string(root.join(".containerignore"))
        .expect("read container context policy");
    let profile = std::fs::read_to_string(root.join("ci/cargo-ci-config.toml"))
        .expect("read shared CI Cargo profile");

    let repository_legacy_config = root.join(".cargo/config");
    assert!(
        !repository_legacy_config.exists() && !repository_legacy_config.is_symlink(),
        "{} has higher precedence than the audited immutable-CI profile",
        repository_legacy_config.display()
    );
    assert_eq!(
        std::fs::read_to_string(root.join(".cargo/config.toml"))
            .expect("read repository Cargo config"),
        "[alias]\nxtask = \"run --package xtask --\"\n\n",
        "the higher-precedence repository Cargo config may retain the xtask alias but must not override the audited CI profile"
    );

    assert_eq!(
        profile,
        "# One immutable-checkout build profile for both private and GitHub-hosted CI.\n\
# Keep this out of `.cargo/`: local debugging retains the workspace defaults.\n\
[build]\n\
incremental = false\n\
\n\
[profile.dev]\n\
debug = \"line-tables-only\"\n\
\n\
[profile.dev.package.\"*\"]\n\
debug = false\n\
\n\
[profile.test]\n\
debug = \"line-tables-only\"\n",
        "the audited CI profile changed; review its build and disk effects"
    );

    assert_once(
        &container,
        "COPY --chown=runner:runner ci/cargo-ci-config.toml /usr/local/cargo/config.toml",
        "private runner image",
    );
    assert_once(
        &hosted,
        "\"$REPO_ROOT/ci/scripts/install-cargo-ci-config.sh\"",
        "hosted preparation",
    );
    for boundary in [
        "committed_config=\"$REPO_ROOT/ci/cargo-ci-config.toml\"",
        "legacy=\"${destination%.toml}\"",
        "if [[ \"$legacy\" != \"$destination\" && ( -e \"$legacy\" || -L \"$legacy\" ) ]]",
        "legacy Cargo config shadows ci/cargo-ci-config.toml",
        "if [[ -e \"$destination\" || -L \"$destination\" ]]",
        "! cmp -s \"$committed_config\" \"$destination\"",
        "install -m 0644 \"$committed_config\" \"$destination\"",
    ] {
        assert!(
            installer.contains(boundary),
            "CI Cargo profile installer lost boundary `{boundary}`"
        );
    }
    assert!(
        containerignore
            .lines()
            .any(|line| line == "!ci/cargo-ci-config.toml"),
        "the private runner build context omits the shared CI Cargo config"
    );

    for (file, workflow) in workflows() {
        let rendered =
            yaml_compat::to_string(&workflow).expect("render workflow for profile audit");
        for override_name in ["CARGO_INCREMENTAL", "CARGO_PROFILE_"] {
            assert!(
                !rendered.contains(override_name),
                "{file} overrides the shared CI Cargo profile through `{override_name}`"
            );
        }
    }
}

#[cfg(target_os = "linux")]
fn assert_legacy_cargo_configs_are_refused(
    root: &std::path::Path,
    installer: &std::path::Path,
    destination: &std::path::Path,
    outside: &std::path::Path,
) {
    use std::os::unix::fs::symlink;

    let legacy = destination.with_file_name("config");
    std::fs::write(&legacy, "[build]\nincremental = true\n")
        .expect("write conflicting legacy profile");
    let legacy_conflict = Command::new(installer)
        .arg(destination)
        .current_dir(root)
        .output()
        .expect("refuse conflicting legacy hosted profile");
    assert!(!legacy_conflict.status.success());
    assert!(
        String::from_utf8_lossy(&legacy_conflict.stderr)
            .contains("legacy Cargo config shadows ci/cargo-ci-config.toml")
    );
    assert!(
        !destination.exists(),
        "a hidden higher-precedence legacy config must be refused before installation"
    );

    std::fs::remove_file(&legacy).expect("remove conflicting legacy profile");
    symlink(outside, &legacy).expect("create hostile legacy profile symlink");
    let legacy_hostile = Command::new(installer)
        .arg(destination)
        .current_dir(root)
        .output()
        .expect("refuse symlinked legacy hosted profile");
    assert!(!legacy_hostile.status.success());
    assert_eq!(
        std::fs::read_to_string(outside).expect("legacy symlink target remains"),
        "untouched\n"
    );

    std::fs::remove_file(&legacy).expect("remove hostile legacy profile symlink");
    std::fs::write(
        &legacy,
        std::fs::read(root.join("ci/cargo-ci-config.toml")).expect("committed profile"),
    )
    .expect("write identical legacy profile");
    let identical_legacy = Command::new(installer)
        .arg(destination)
        .current_dir(root)
        .output()
        .expect("refuse identical legacy hosted profile");
    assert!(
        !identical_legacy.status.success(),
        "even identical legacy/config.toml pairs make Cargo warn and silently prefer the legacy spelling"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn hosted_cargo_profile_install_refuses_conflicts_and_symlinks() {
    use std::os::unix::fs::symlink;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let installer = root.join("ci/scripts/install-cargo-ci-config.sh");
    let fixture = tempfile::Builder::new()
        .prefix("hosted-cargo-profile-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create hosted profile fixture");
    let destination = fixture.path().join("cargo/config.toml");

    let first = Command::new(&installer)
        .arg(&destination)
        .current_dir(&root)
        .output()
        .expect("install hosted profile");
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(
        std::fs::read(&destination).expect("installed profile"),
        std::fs::read(root.join("ci/cargo-ci-config.toml")).expect("committed profile")
    );
    assert!(
        Command::new(&installer)
            .arg(&destination)
            .current_dir(&root)
            .status()
            .expect("repeat hosted profile install")
            .success(),
        "an identical existing profile must be idempotent"
    );

    std::fs::write(&destination, "[build]\nincremental = true\n")
        .expect("write conflicting profile");
    let conflict = Command::new(&installer)
        .arg(&destination)
        .current_dir(&root)
        .output()
        .expect("refuse conflicting hosted profile");
    assert!(!conflict.status.success());
    assert!(
        String::from_utf8_lossy(&conflict.stderr)
            .contains("hosted Cargo config differs from ci/cargo-ci-config.toml")
    );
    assert_eq!(
        std::fs::read_to_string(&destination).expect("conflict remains"),
        "[build]\nincremental = true\n",
        "a refusal must not overwrite the host's policy"
    );

    std::fs::remove_file(&destination).expect("remove conflicting profile");
    let outside = fixture.path().join("outside");
    std::fs::write(&outside, "untouched\n").expect("write symlink target");
    symlink(&outside, &destination).expect("create hostile destination symlink");
    let hostile = Command::new(&installer)
        .arg(&destination)
        .current_dir(&root)
        .output()
        .expect("refuse symlinked hosted profile");
    assert!(!hostile.status.success());
    assert_eq!(
        std::fs::read_to_string(&outside).expect("symlink target remains"),
        "untouched\n"
    );

    std::fs::remove_file(&destination).expect("remove hostile destination symlink");
    assert_legacy_cargo_configs_are_refused(&root, &installer, &destination, &outside);
}

fn runs_on(job: &yaml_compat::Value) -> String {
    match job.get("runs-on") {
        Some(yaml_compat::Value::String(s)) => s.clone(),
        Some(yaml_compat::Value::Sequence(xs)) => xs
            .iter()
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join(","),
        other => format!("{other:?}"),
    }
}

fn job<'a>(
    files: &'a [(String, yaml_compat::Value)],
    file: &str,
    name: &str,
) -> &'a yaml_compat::Value {
    files
        .iter()
        .find(|(candidate, _)| candidate == file)
        .and_then(|(_, doc)| doc.get("jobs"))
        .and_then(yaml_compat::Value::as_mapping)
        .and_then(|jobs| jobs.get(yaml_compat::Value::String(name.to_string())))
        .unwrap_or_else(|| panic!("{file}:{name} is in the CI census but not in the workflow"))
}

#[test]
fn the_required_action_selftest_context_is_not_path_conditional() {
    let files = workflows();
    let workflow = files
        .iter()
        .find(|(file, _)| file == "action-selftest.yml")
        .map(|(_, workflow)| workflow)
        .expect("action self-test workflow is in the census");
    let triggers = workflow["on"]
        .as_mapping()
        .expect("action self-test declares mapped triggers");
    for event in ["push", "pull_request"] {
        let trigger = triggers
            .get(yaml_compat::Value::String(event.to_owned()))
            .and_then(yaml_compat::Value::as_mapping)
            .unwrap_or_else(|| panic!("action self-test has no mapped {event} trigger"));
        assert!(
            !trigger.contains_key(yaml_compat::Value::String("paths".to_owned())),
            "the required action self-test context must be emitted for every {event}; a path filter blocks every unrelated protected-branch change"
        );
    }
}

/// The preparation step is usable only after checkout, and useful only before
/// anything else can consume the checkout. Keep the two adjacent so a future
/// command or action cannot quietly arrive in the unprepared interval.
fn preparation_order_problem(steps: &[yaml_compat::Value], command: &str) -> Option<String> {
    let checkout = steps.iter().position(|step| {
        step.get("uses")
            .and_then(yaml_compat::Value::as_str)
            .is_some_and(|uses| uses.starts_with("actions/checkout@"))
    });
    let preparation = steps
        .iter()
        .position(|step| step.get("run").and_then(yaml_compat::Value::as_str) == Some(command));

    match (checkout, preparation) {
        (None, _) => Some("has no checkout step".to_string()),
        (_, None) => Some(format!("does not invoke `{command}`")),
        (Some(checkout), Some(preparation)) if preparation != checkout + 1 => Some(format!(
            "runs hosted preparation at step {}, but checkout is step {}; preparation must be \
             immediately after checkout and before every consumer of the repository",
            preparation + 1,
            checkout + 1
        )),
        _ => None,
    }
}

/// The performance harness is executable test code, so its interpreter belongs
/// to both environment contracts. This reads the declarations that actually
/// build and prepare those environments rather than maintaining a third list.
#[test]
fn every_test_environment_declares_the_performance_harness_runtime() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let containerfile = std::fs::read_to_string(root.join("ci/Containerfile"))
        .expect("read the private runner image");
    let packages = containerfile
        .split_once("apt-get install -y --no-install-recommends")
        .expect("private image has one package-install block")
        .1
        .split_once("&& rm -rf /var/lib/apt/lists/*")
        .expect("private image bounds its package-install block")
        .0;
    assert!(
        packages
            .split_ascii_whitespace()
            .any(|word| word.trim_end_matches('\\') == "python3"),
        "ci/Containerfile must install `python3` for the read-performance tests"
    );

    let preparation = std::fs::read_to_string(root.join("ci/scripts/prepare-hosted.sh"))
        .expect("read hosted preparation");
    let commands = preparation
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("for command in ")
                .and_then(|line| line.strip_suffix("; do"))
        })
        .expect("hosted preparation has a command census");
    assert!(
        commands
            .split_ascii_whitespace()
            .any(|word| word == "python3"),
        "hosted preparation must require `python3` before repository work"
    );
}

/// A1-650: private and hosted execution must consume one artifact identity.
/// Copying a version or checksum into either environment would let one runner
/// stay green on a different nextest build.
fn assert_workflows_install_nextest_on_every_runner(files: &[(String, yaml_compat::Value)]) {
    for (file, name) in [
        ("ci.yml", "development"),
        ("ci.yml", "full"),
        ("release.yml", "release"),
    ] {
        let job = job(files, file, name);
        let steps = job
            .get("steps")
            .and_then(yaml_compat::Value::as_sequence)
            .unwrap_or_else(|| panic!("{file}:{name} has no steps"));
        let install = steps
            .iter()
            .enumerate()
            .filter(|(_, step)| {
                step.get("run").and_then(yaml_compat::Value::as_str)
                    == Some("ci/scripts/install-nextest.sh install")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            install.len(),
            1,
            "{file}:{name} must invoke the shared nextest installer exactly once"
        );
        let (install_index, install_step) = install[0];
        assert!(
            install_step.get("if").is_none(),
            "{file}:{name} must install or verify nextest on both private and hosted runners"
        );
        let preparation_index = steps
            .iter()
            .position(|step| {
                step.get("run")
                    .and_then(yaml_compat::Value::as_str)
                    .is_some_and(|command| command.starts_with("ci/scripts/prepare-hosted.sh "))
            })
            .expect("hosted preparation is present by the executable census");
        assert_eq!(
            install_index,
            preparation_index + 1,
            "{file}:{name} must verify nextest immediately after environment preparation and before repository work"
        );
    }
}

/// A1-450: `command -v` returns a path, not a shell word. Hosted tool caches
/// and developer homes may contain spaces, so the supply-chain gate must
/// execute the resolved cargo-deny path as one quoted argument.
#[test]
fn cargo_deny_check_handles_executable_paths_with_spaces() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let version = std::fs::read_to_string(root.join("ci/deny.env"))
        .expect("read cargo-deny artifact manifest")
        .lines()
        .find_map(|line| line.strip_prefix("CARGO_DENY_VERSION="))
        .expect("ci/deny.env defines CARGO_DENY_VERSION")
        .to_string();
    let fixture = tempfile::Builder::new()
        .prefix("cargo deny path ")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create spaced cargo-deny fixture");
    let bin = fixture.path().join("bin with spaces");
    std::fs::create_dir(&bin).expect("create fake binary directory");
    let fake = bin.join("cargo-deny");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\nprintf '%s\\n' 'cargo-deny {version}'\n"),
    )
    .expect("write fake cargo-deny");
    let mut permissions = std::fs::metadata(&fake)
        .expect("fake cargo-deny metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fake, permissions).expect("make fake cargo-deny executable");

    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(bin.clone()).chain(std::env::split_paths(&inherited_path)),
    )
    .expect("compose fixture PATH");
    let output = Command::new(root.join("ci/scripts/install-cargo-deny.sh"))
        .arg("check")
        .env("PATH", path)
        .output()
        .expect("run cargo-deny checker");
    assert!(
        output.status.success(),
        "a valid cargo-deny at a spaced path was refused: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("checker output is UTF-8");
    assert!(
        stdout.starts_with(&format!("cargo-deny: version={version} target="))
            && stdout.lines().count() == 1,
        "unexpected checker output: {stdout:?}"
    );
}

#[test]
fn private_and_hosted_test_environments_share_one_pinned_nextest_artifact() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = std::fs::read_to_string(root.join("ci/nextest.env"))
        .expect("read nextest artifact manifest");
    let installer = std::fs::read_to_string(root.join("ci/scripts/install-nextest.sh"))
        .expect("read nextest installer");
    let container =
        std::fs::read_to_string(root.join("ci/Containerfile")).expect("read private runner image");
    let containerignore = std::fs::read_to_string(root.join(".containerignore"))
        .expect("read container context policy");
    let validation: yaml_compat::Value = yaml_compat::from_str(
        &std::fs::read_to_string(root.join("ci/validation.yml"))
            .expect("read validation selection inventory"),
    )
    .expect("parse validation selection inventory");

    for key in [
        "NEXTEST_VERSION",
        "NEXTEST_COMMIT",
        "NEXTEST_X86_64_UNKNOWN_LINUX_GNU_SHA256",
        "NEXTEST_AARCH64_UNKNOWN_LINUX_GNU_SHA256",
        "NEXTEST_UNIVERSAL_APPLE_DARWIN_SHA256",
    ] {
        assert_eq!(
            manifest
                .lines()
                .filter(|line| line.starts_with(&format!("{key}=")))
                .count(),
            1,
            "ci/nextest.env must define {key} exactly once"
        );
        assert!(
            installer.contains(&format!("${key}")),
            "the installer does not consume {key}"
        );
    }
    assert!(container.contains("ci/scripts/install-nextest.sh"));
    assert!(
        !container.contains("0.9.143"),
        "the private image copied the version pin"
    );
    assert!(
        !container.contains("66786b9abe23920d022a182d1416b1bbc8130dd4872a9553d76985a1708dcd1e"),
        "the private image copied an artifact checksum"
    );
    let hosted = std::fs::read_to_string(root.join("ci/scripts/prepare-hosted.sh"))
        .expect("read hosted preparation");
    assert!(
        !hosted.contains("install-nextest.sh"),
        "hosted-only preparation must not duplicate the common private/hosted installer step"
    );
    assert_workflows_install_nextest_on_every_runner(&workflows());
    for included in ["!ci/nextest.env", "!ci/scripts/install-nextest.sh"] {
        assert!(
            containerignore.lines().any(|line| line == included),
            "the bounded container context omits {included}"
        );
    }
    let path_rules = validation["selection"]["paths"]
        .as_sequence()
        .expect("selection.paths is a sequence");
    assert_eq!(
        path_rules
            .iter()
            .filter(|rule| {
                rule["disposition"].as_str() == Some("global")
                    && rule["exact"].as_sequence().is_some_and(|exact| {
                        exact
                            .iter()
                            .any(|path| path.as_str() == Some(".config/nextest.toml"))
                    })
            })
            .count(),
        1,
        "nextest configuration changes must widen to the complete Rust validation set"
    );
}

#[test]
fn a_public_repository_cannot_name_a_persistent_self_hosted_runner() {
    let files = workflows();
    let mut seen = Vec::new();
    let mut wrong = Vec::new();

    for (file, doc) in &files {
        let Some(jobs) = doc.get("jobs").and_then(yaml_compat::Value::as_mapping) else {
            panic!("{file} declares no jobs");
        };
        for (name, job) in jobs {
            let name = name.as_str().unwrap_or_default();
            seen.push((file.as_str(), name));
            let actual = runs_on(job);
            let expected = if file == "tui-portability.yml" && name == "public-platforms" {
                PUBLIC_PORTABILITY_RUNNER
            } else {
                VISIBILITY_RUNNER
            };
            if actual != expected {
                wrong.push(format!("{file}:{name} uses `{actual}`"));
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "{} workflow job(s) do not use the reviewed private/self-hosted, public/hosted expression \
         from ADR-0045/ADR-0059. A pull request can replace its workflow and name the old labels directly, \
         so the external flip gate must also prove the public repository has zero registered \
         runners; this census proves the committed workflows do not regress by accident:\n  - {}",
        wrong.len(),
        wrong.join("\n  - ")
    );

    let accounted = HOSTED_PREPARATION
        .iter()
        .map(|(file, name, _)| (*file, *name))
        .chain(
            NO_HOSTED_PREPARATION
                .iter()
                .map(|(file, name, _)| (*file, *name)),
        )
        .collect::<Vec<_>>();
    let missing = seen
        .iter()
        .filter(|entry| !accounted.contains(entry))
        .map(|(file, name)| format!("{file}:{name}"))
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "{} workflow job(s) have no hosted-environment decision in the executable census:\n  - {}",
        missing.len(),
        missing.join("\n  - ")
    );
    assert_eq!(
        seen.len(),
        accounted.len(),
        "the hosted-environment census contains a stale or duplicate entry"
    );

    let selftest = job(&files, "action-selftest.yml", "selftest");
    assert_eq!(
        selftest.get("if").and_then(yaml_compat::Value::as_str),
        Some(ACTION_SELFTEST_GUARD),
        "the action self-test must fail closed to its three reviewed events and refuse a private pull request"
    );

    let portability = job(&files, "tui-portability.yml", "public-platforms");
    assert_eq!(
        portability.get("if").and_then(yaml_compat::Value::as_str),
        Some(PUBLIC_PORTABILITY_GUARD),
        "macOS and Windows must be skipped before runner allocation while the repository is private"
    );
    let matrix = portability["strategy"]["matrix"]["include"]
        .as_sequence()
        .expect("the public portability job has an explicit matrix")
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().unwrap_or_default(),
                entry["runner"].as_str().unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        matrix,
        vec![("macOS", "macos-latest"), ("Windows", "windows-latest")],
        "the public portability matrix must contain only reviewed standard hosted labels"
    );
}

/// The macOS hosted image invokes `/bin/bash` 3.2 when a workflow asks for
/// `shell: bash`. Keep the shared reader free of builtins introduced later,
/// because portability preparation calls it before any test can run (`A2-007`).
#[test]
fn portability_toolchain_reader_accepts_the_macos_system_bash_surface() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let reader = std::fs::read_to_string(root.join("ci/scripts/toolchain.sh"))
        .expect("read toolchain contract reader");

    for unsupported in ["mapfile", "readarray", "declare -A"] {
        assert!(
            !reader.contains(unsupported),
            "ci/scripts/toolchain.sh uses `{unsupported}`, which macOS /bin/bash 3.2 does not provide"
        );
    }
    assert!(
        !reader.contains("seen_components=()"),
        "an empty indexed array is unbound under Bash 3.2 nounset; use a scalar set"
    );
}

#[test]
fn every_compiling_job_prepares_its_hosted_environment() {
    let files = workflows();
    for (file, name, profile) in HOSTED_PREPARATION {
        let job = job(&files, file, name);
        let steps = job
            .get("steps")
            .and_then(yaml_compat::Value::as_sequence)
            .unwrap_or_else(|| panic!("{file}:{name} has no steps"));
        let command = format!("ci/scripts/prepare-hosted.sh {profile}");
        let preparation = steps.iter().find(|step| {
            step.get("run").and_then(yaml_compat::Value::as_str) == Some(command.as_str())
        });
        let Some(preparation) = preparation else {
            panic!("{file}:{name} does not invoke its `{profile}` hosted preparation profile");
        };
        assert_eq!(
            preparation.get("if").and_then(yaml_compat::Value::as_str),
            Some("runner.environment == 'github-hosted'"),
            "{file}:{name} must not alter the persistent runner while preparing the hosted one"
        );
        assert_eq!(
            preparation_order_problem(steps, &command),
            None,
            "{file}:{name} does work before its hosted environment is ready"
        );
    }

    for (file, name, why) in NO_HOSTED_PREPARATION {
        assert!(
            why.len() > 80,
            "{file}:{name} has no preparation and no reason worth reviewing: {why:?}"
        );
        let _ = job(&files, file, name);
    }
}

/// The regression shape `A1-403` found: the old census found the guarded step
/// and passed, although a public runner would execute repository code first.
#[test]
fn hosted_preparation_must_precede_repository_work() {
    let job: yaml_compat::Value = yaml_compat::from_str(
        r"
steps:
  - uses: actions/checkout@v7
  - run: ci/scripts/test.sh
  - name: prepare a GitHub-hosted runner
    if: runner.environment == 'github-hosted'
    run: ci/scripts/prepare-hosted.sh base
",
    )
    .expect("synthetic workflow job");
    let steps = job
        .get("steps")
        .and_then(yaml_compat::Value::as_sequence)
        .expect("steps");
    let command = "ci/scripts/prepare-hosted.sh base";

    assert!(
        steps.iter().any(|step| {
            step.get("run").and_then(yaml_compat::Value::as_str) == Some(command)
                && step.get("if").and_then(yaml_compat::Value::as_str)
                    == Some("runner.environment == 'github-hosted'")
        }),
        "the old presence-and-guard check must pass this fixture or it does not reproduce the gap"
    );
    let problem = preparation_order_problem(steps, command)
        .expect("work before preparation must be rejected");
    assert!(problem.contains("immediately after checkout"), "{problem}");
}

#[test]
fn the_public_boundary_gate_requires_public_zero_runners_and_all_external_approval() {
    let temp = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("temporary fake gh directory");
    let fake = temp.path().join("gh");
    std::fs::write(
        &fake,
        r#"#!/usr/bin/env bash
set -euo pipefail
case "$2" in
  */actions/runners) printf '%s\n' "${FAKE_RUNNERS}" ;;
  */actions/permissions/fork-pr-contributor-approval) printf '%s\n' "${FAKE_APPROVAL}" ;;
  repos/*) printf '%s\n' "${FAKE_VISIBILITY}" ;;
  *) echo "unexpected fake gh call: $*" >&2; exit 3 ;;
esac
"#,
    )
    .expect("write fake gh");
    let mut permissions = std::fs::metadata(&fake)
        .expect("fake gh metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fake, permissions).expect("make fake gh executable");

    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ci/scripts/check-public-runner-boundary.sh");
    let run = |visibility: &str, runners: &str, approval: &str| {
        Command::new("bash")
            .arg(&script)
            .arg("owner/repository")
            .env("GH_BIN", &fake)
            .env("FAKE_VISIBILITY", visibility)
            .env("FAKE_RUNNERS", runners)
            .env("FAKE_APPROVAL", approval)
            .output()
            .expect("run the public boundary gate")
    };

    let good = run("public", "0", "all_external_contributors");
    assert!(
        good.status.success(),
        "the complete boundary was refused: {}",
        String::from_utf8_lossy(&good.stderr)
    );

    for (visibility, runners, approval, expected) in [
        ("private", "0", "all_external_contributors", "visibility"),
        ("public", "1", "all_external_contributors", "registered"),
        ("public", "0", "first_time_contributors", "approval policy"),
    ] {
        let output = run(visibility, runners, approval);
        assert!(!output.status.success(), "an incomplete boundary passed");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "the refusal did not name {expected}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
