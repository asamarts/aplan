//! A1-618's rollout corpus drives the real selector and dispatcher in pairs.
//!
//! Each row changes one repository surface, injects one failing full-profile
//! gate, and compares development with full. A selected or widened gate must
//! fail both. A development pass is permitted only when the injected gate is
//! named as deferred and full still fails it.

use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::fixture_executable::install_executable as write_executable;

struct ShadowCase {
    name: &'static str,
    path: &'static str,
    negative_gate: &'static str,
    untracked: bool,
}

const CASES: &[ShadowCase] = &[
    ShadowCase {
        name: "plan-structure",
        path: "aplan/tasks/A1-618.md",
        negative_gate: "structural",
        untracked: false,
    },
    ShadowCase {
        name: "plan-evidence",
        path: "aplan/tasks/A1-617.md",
        negative_gate: "evidence",
        untracked: false,
    },
    ShadowCase {
        name: "root-document",
        path: "README.md",
        negative_gate: "fmt",
        untracked: false,
    },
    ShadowCase {
        name: "documentation-prefix",
        path: "docs/design/development-validation.md",
        negative_gate: "style",
        untracked: false,
    },
    ShadowCase {
        name: "package-aplan",
        path: "crates/aplan/src/lib.rs",
        negative_gate: "clippy",
        untracked: false,
    },
    ShadowCase {
        name: "package-app",
        path: "crates/aplan-app/src/lib.rs",
        negative_gate: "doc",
        untracked: false,
    },
    ShadowCase {
        name: "package-attest",
        path: "crates/aplan-attest/src/lib.rs",
        negative_gate: "test",
        untracked: false,
    },
    ShadowCase {
        name: "package-config",
        path: "crates/aplan-config/src/lib.rs",
        negative_gate: "coverage",
        untracked: false,
    },
    ShadowCase {
        name: "package-core",
        path: "crates/aplan-core/src/lib.rs",
        negative_gate: "test",
        untracked: false,
    },
    ShadowCase {
        name: "package-e2e",
        path: "crates/aplan-e2e/src/lib.rs",
        negative_gate: "clippy",
        untracked: false,
    },
    ShadowCase {
        name: "package-evidence",
        path: "crates/aplan-evidence/src/lib.rs",
        negative_gate: "doc",
        untracked: false,
    },
    ShadowCase {
        name: "package-interop",
        path: "crates/aplan-interop/src/lib.rs",
        negative_gate: "test",
        untracked: false,
    },
    ShadowCase {
        name: "package-mcp",
        path: "crates/aplan-mcp/src/lib.rs",
        negative_gate: "clippy",
        untracked: false,
    },
    ShadowCase {
        name: "package-output",
        path: "crates/aplan-output/src/lib.rs",
        negative_gate: "doc",
        untracked: false,
    },
    ShadowCase {
        name: "package-tui",
        path: "crates/aplan-tui/src/lib.rs",
        negative_gate: "test",
        untracked: false,
    },
    ShadowCase {
        name: "package-xtask",
        path: "xtask/src/main.rs",
        negative_gate: "coverage",
        untracked: false,
    },
    ShadowCase {
        name: "schema",
        path: "schemas/v1/task.json",
        negative_gate: "spec",
        untracked: false,
    },
    ShadowCase {
        name: "generated-facts",
        path: "facts.json",
        negative_gate: "spec",
        untracked: false,
    },
    ShadowCase {
        name: "root-manifest",
        path: "Cargo.toml",
        negative_gate: "msrv-build",
        untracked: false,
    },
    ShadowCase {
        name: "dependency-policy",
        path: "deny.toml",
        negative_gate: "supply-chain",
        untracked: false,
    },
    ShadowCase {
        name: "package-manifest",
        path: "crates/aplan-core/Cargo.toml",
        negative_gate: "msrv-build",
        untracked: false,
    },
    ShadowCase {
        name: "validation-manifest",
        path: "ci/validation.yml",
        negative_gate: "test",
        untracked: false,
    },
    ShadowCase {
        name: "workflow",
        path: ".github/workflows/ci.yml",
        negative_gate: "clippy",
        untracked: false,
    },
    ShadowCase {
        name: "unknown-untracked",
        path: "a1-618-unknown.probe",
        negative_gate: "test",
        untracked: true,
    },
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run rollout git command");
    assert!(
        output.status.success(),
        "git {}: {}",
        args.join(" "),
        stderr(&output)
    );
    String::from_utf8(output.stdout)
        .expect("git output is UTF-8")
        .trim()
        .to_owned()
}

fn git_commit(root: &Path, message: &str) {
    git(
        root,
        &[
            "-c",
            "user.name=aplan validation fixture",
            "-c",
            "user.email=validation-fixture@example.invalid",
            "commit",
            "-q",
            "-m",
            message,
        ],
    );
}

fn append_probe(path: &Path) {
    let marker = match path.extension().and_then(|extension| extension.to_str()) {
        Some("rs") => "\n// A1-618 rollout shadow\n",
        Some("toml" | "yml" | "yaml") => "\n# A1-618 rollout shadow\n",
        Some("md") => "\n<!-- A1-618 rollout shadow -->\n",
        Some("json") => "\n",
        _ => "\nA1-618 rollout shadow\n",
    };
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap_or_else(|error| panic!("open {}: {error}", path.display()));
    file.write_all(marker.as_bytes())
        .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}

fn refresh_selector_bindings(source: &Path) {
    use sha2::{Digest as _, Sha256};
    use std::fmt::Write as _;

    let manifest =
        std::fs::read(source.join("ci/validation.yml")).expect("read changed validation manifest");
    let digest =
        Sha256::digest(manifest)
            .iter()
            .fold(String::with_capacity(64), |mut output, byte| {
                write!(output, "{byte:02x}").expect("write selector digest");
                output
            });
    let path = source.join("ci/validation-selection.json");
    let mut document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read selector policy"))
            .expect("parse selector policy");
    document["source"]["sha256"] = digest.into();
    for input in document["workspace"]["inputs"]
        .as_array_mut()
        .expect("selector workspace inputs")
    {
        let relative = input["path"].as_str().expect("selector input path");
        let digest = Sha256::digest(
            std::fs::read(source.join(relative)).expect("read selector workspace input"),
        )
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            write!(output, "{byte:02x}").expect("write workspace input digest");
            output
        });
        input["sha256"] = digest.into();
    }
    let mut output = serde_json::to_string_pretty(&document).expect("render selector policy");
    output.push('\n');
    std::fs::write(path, output).expect("write selector policy");
}

fn selector(source: &Path) -> Command {
    let mut command = Command::new("python3");
    command.arg(source.join("ci/scripts/validation-select.py"));
    command
}

fn selection(source: &Path, case: &ShadowCase) -> serde_json::Value {
    let output = if case.untracked {
        std::fs::write(source.join(case.path), "A1-618 rollout shadow\n")
            .expect("write unknown rollout probe");
        selector(source)
            .arg("--root")
            .arg(source)
            .args(["--working-tree", "--format", "json"])
            .current_dir(source)
            .output()
            .expect("classify unknown rollout probe")
    } else {
        let base = git(source, &["rev-parse", "HEAD"]);
        append_probe(&source.join(case.path));
        if case.path == "ci/validation.yml" || case.path.ends_with("Cargo.toml") {
            refresh_selector_bindings(source);
            git(source, &["add", case.path, "ci/validation-selection.json"]);
        } else {
            git(source, &["add", case.path]);
        }
        git(source, &["commit", "-q", "-m", case.name]);
        let head = git(source, &["rev-parse", "HEAD"]);
        selector(source)
            .args(["--root", source.to_str().expect("UTF-8 rollout root")])
            .args(["--base", &base, "--head", &head, "--format", "json"])
            .current_dir(source)
            .output()
            .expect("classify rollout revision")
    };
    assert!(
        output.status.success(),
        "{}: {}",
        case.name,
        stderr(&output)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{} emitted invalid selection JSON: {error}", case.name))
}

fn shadow_dispatcher() -> tempfile::TempDir {
    let fixture = tempfile::Builder::new()
        .prefix("aplan-validation-rollout-dispatch-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create rollout dispatcher fixture");
    let scripts = fixture.path().join("ci/scripts");
    std::fs::create_dir_all(&scripts).expect("create rollout script directory");
    write_executable(
        &scripts.join("validate.sh"),
        include_str!("../../ci/scripts/validate.sh"),
    );
    write_executable(
        &scripts.join("validation-receipt.py"),
        include_str!("../../ci/scripts/validation-receipt.py"),
    );
    std::fs::write(
        scripts.join("validation-generated.sh"),
        include_str!("../../ci/scripts/validation-generated.sh"),
    )
    .expect("write generated rollout inventory");
    write_executable(
        &scripts.join("toolchain.sh"),
        "#!/usr/bin/env bash\nset -euo pipefail\necho 'toolchain: channel=1.99.1 host=x86_64-unknown-linux-gnu commit=1111111111111111111111111111111111111111 components=rustfmt,clippy cache-key=rust-1.99.1-rustfmt-clippy'\n",
    );
    std::fs::write(
        fixture.path().join("ci/validation.yml"),
        include_str!("../../ci/validation.yml"),
    )
    .expect("write rollout inventory");
    std::fs::write(
        fixture.path().join("Cargo.toml"),
        "[workspace.package]\nversion = \"0.5.0-dev\"\n",
    )
    .expect("write rollout workspace manifest");
    for gate in [
        "fmt",
        "clippy",
        "doc",
        "test",
        "spec",
        "msrv-build",
        "style",
        "supply-chain",
        "coverage",
        "structural",
        "evidence",
    ] {
        write_executable(
            &scripts.join(format!("{gate}.sh")),
            &format!(
                "#!/usr/bin/env bash\nset -euo pipefail\nif [[ \"${{APLAN_SHADOW_FAIL_GATE:-}}\" == \"{gate}\" ]]; then exit 41; fi\n"
            ),
        );
    }
    git(fixture.path(), &["init", "-q"]);
    git(
        fixture.path(),
        &["config", "user.email", "rollout@example.invalid"],
    );
    git(fixture.path(), &["config", "user.name", "Rollout Fixture"]);
    git(fixture.path(), &["add", "."]);
    git(fixture.path(), &["commit", "-q", "-m", "rollout fixture"]);
    fixture
}

fn strings(document: &serde_json::Value, key: &str) -> Vec<String> {
    document[key]
        .as_array()
        .unwrap_or_else(|| panic!("selection field `{key}` is not an array"))
        .iter()
        .map(|value| {
            value
                .as_str()
                .unwrap_or_else(|| panic!("selection field `{key}` has a non-string"))
                .to_owned()
        })
        .collect()
}

fn run_profile(
    fixture: &Path,
    artifacts: &Path,
    case: &ShadowCase,
    profile: &str,
    report: &serde_json::Value,
) -> (Output, serde_json::Value) {
    let mut command = Command::new(fixture.join("ci/scripts/validate.sh"));
    command.args(["--profile", profile]);
    if profile == "development" {
        for gate in strings(report, "selected_gates") {
            command.args(["--select", &gate]);
        }
        for package in strings(report, "selected_packages") {
            command.args(["--package", &package]);
        }
        for reason in strings(report, "widening_reasons") {
            command.args(["--widen", &reason]);
        }
    }
    let receipt = artifacts.join(format!("{}-{profile}.json", case.name));
    let output = command
        .env_remove("PREFLIGHT_SKIP")
        .env("APLAN_SHADOW_FAIL_GATE", case.negative_gate)
        .env("APLAN_VALIDATION_RECEIPT", &receipt)
        .env("RUNNER_ENVIRONMENT", "local-shadow")
        .env("RUNNER_OS", "Linux")
        .env("RUNNER_ARCH", "X64")
        .current_dir(fixture)
        .output()
        .expect("run rollout profile");
    let document = serde_json::from_slice(
        &std::fs::read(&receipt)
            .unwrap_or_else(|error| panic!("read {} receipt: {error}", case.name)),
    )
    .unwrap_or_else(|error| panic!("parse {} receipt: {error}", case.name));
    (output, document)
}

fn audit_case(
    source: &Path,
    dispatcher: &Path,
    artifacts: &Path,
    case: &ShadowCase,
) -> (BTreeSet<String>, bool) {
    let report = selection(source, case);
    let changes = report["changes"].as_array().expect("selection changes");
    assert!(
        changes.iter().any(|change| change["path"] == case.path),
        "{} did not report its changed path: {report}",
        case.name
    );
    let packages = changes
        .iter()
        .filter_map(|change| {
            change["classification"]
                .as_str()
                .and_then(|classification| classification.strip_prefix("package:"))
                .map(str::to_owned)
        })
        .collect();

    let mut development = strings(&report, "always_gates");
    development.extend(strings(&report, "selected_gates"));
    development.extend(strings(&report, "widened_gates"));
    let deferred = strings(&report, "deferred_gates");
    let (development_output, development_receipt) =
        run_profile(dispatcher, artifacts, case, "development", &report);
    let was_deferred = if development.contains(&case.negative_gate.to_owned()) {
        assert_eq!(development_output.status.code(), Some(41), "{}", case.name);
        assert_eq!(development_receipt["outcome"], "failed");
        false
    } else {
        assert!(
            deferred.contains(&case.negative_gate.to_owned()),
            "{} silently omitted `{}`: {report}",
            case.name,
            case.negative_gate
        );
        assert!(
            development_output.status.success(),
            "{}",
            stderr(&development_output)
        );
        assert_eq!(development_receipt["outcome"], "passed");
        true
    };
    assert_eq!(development_receipt["profile"], "development");

    let (full_output, full_receipt) = run_profile(dispatcher, artifacts, case, "full", &report);
    assert_eq!(full_output.status.code(), Some(41), "{}", case.name);
    assert_eq!(full_receipt["profile"], "full");
    assert_eq!(full_receipt["outcome"], "failed");
    assert!(
        full_receipt["gates"]
            .as_array()
            .expect("full gate results")
            .iter()
            .any(|gate| gate["name"] == case.negative_gate && gate["outcome"] == "failed"),
        "full did not report {}'s injected `{}` failure",
        case.name,
        case.negative_gate
    );
    (packages, was_deferred)
}

fn install_current_selector(source: &Path) {
    git(source, &["config", "user.email", "rollout@example.invalid"]);
    git(source, &["config", "user.name", "Rollout Fixture"]);
    let workspace = workspace_root();
    std::fs::copy(
        workspace.join("ci/scripts/validation-select.py"),
        source.join("ci/scripts/validation-select.py"),
    )
    .expect("install current production selector");
    std::fs::copy(
        workspace.join("ci/validation-selection.json"),
        source.join("ci/validation-selection.json"),
    )
    .expect("install current selector policy");
    std::fs::copy(
        workspace.join("ci/validation.yml"),
        source.join("ci/validation.yml"),
    )
    .expect("install current validation inventory");
    std::fs::copy(
        workspace.join("ci/scripts/supply-chain.sh"),
        source.join("ci/scripts/supply-chain.sh"),
    )
    .expect("install current supply-chain gate command");
    std::fs::copy(workspace.join("deny.toml"), source.join("deny.toml"))
        .expect("install current dependency policy root");
    let policy: serde_json::Value = serde_json::from_slice(
        &std::fs::read(workspace.join("ci/validation-selection.json"))
            .expect("read current selector policy"),
    )
    .expect("parse current selector policy");
    let mut installed = vec![
        "ci/scripts/validation-select.py".to_owned(),
        "ci/validation-selection.json".to_owned(),
        "ci/validation.yml".to_owned(),
        "ci/scripts/supply-chain.sh".to_owned(),
        "deny.toml".to_owned(),
    ];
    for input in policy["workspace"]["inputs"]
        .as_array()
        .expect("current selector workspace inputs")
    {
        let relative = input["path"]
            .as_str()
            .expect("current selector workspace input path");
        std::fs::copy(workspace.join(relative), source.join(relative))
            .unwrap_or_else(|error| panic!("install current selector input {relative}: {error}"));
        installed.push(relative.to_owned());
    }
    let staged = Command::new("git")
        .arg("add")
        .args(&installed)
        .current_dir(source)
        .output()
        .expect("stage current production selector and inputs");
    assert!(staged.status.success(), "{}", stderr(&staged));
    let staged = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(source)
        .status()
        .expect("inspect current production selector");
    match staged.code() {
        Some(0) => {}
        Some(1) => {
            git_commit(source, "current production selector");
        }
        status => panic!("git diff --cached --quiet exited {status:?}"),
    }
}

fn install_historical_probe_paths(source: &Path) {
    let missing = ["aplan/tasks/A1-617.md", "aplan/tasks/A1-618.md"]
        .into_iter()
        .filter(|path| !source.join(path).is_file())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return;
    }
    for path in &missing {
        std::fs::write(source.join(path), "A1 rollout probe source\n")
            .expect("write historical rollout probe source");
    }
    let mut add = Command::new("git");
    add.arg("add").args(&missing).current_dir(source);
    let output = add.output().expect("stage historical rollout probe paths");
    assert!(output.status.success(), "{}", stderr(&output));
    git_commit(source, "install rollout probe paths");
}

/// The rollout is an executable comparison, not a table that merely names
/// surfaces. It covers every workspace package and every validation input
/// class with more than the promised twenty development/full pairs.
#[test]
fn twenty_four_shadow_pairs_keep_every_omission_explicit_and_full_catches_every_seed() {
    let outer = tempfile::Builder::new()
        .prefix("aplan-validation-rollout-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create rollout fixture root");
    let source = outer.path().join("source");
    let clone = Command::new("git")
        .args(["clone", "-q", "--shared"])
        .arg(workspace_root())
        .arg(&source)
        .output()
        .expect("clone rollout source");
    assert!(clone.status.success(), "{}", stderr(&clone));
    let audited_source = git(&workspace_root(), &["rev-parse", "HEAD"]);
    git(&source, &["checkout", "-q", "--detach", &audited_source]);
    install_historical_probe_paths(&source);
    install_current_selector(&source);

    let dispatcher = shadow_dispatcher();
    let artifacts = outer.path().join("receipts");
    std::fs::create_dir(&artifacts).expect("create rollout receipt directory");
    let mut observed_packages = BTreeSet::new();
    let mut observed_gates = BTreeSet::new();
    let mut deferred_controls = 0;

    for case in CASES {
        let (packages, was_deferred) = audit_case(&source, dispatcher.path(), &artifacts, case);
        observed_packages.extend(packages);
        observed_gates.insert(case.negative_gate);
        if was_deferred {
            deferred_controls += 1;
        }
    }

    assert!(CASES.len() >= 20);
    assert_eq!(
        observed_packages,
        [
            "aplan",
            "aplan-app",
            "aplan-attest",
            "aplan-config",
            "aplan-core",
            "aplan-e2e",
            "aplan-evidence",
            "aplan-interop",
            "aplan-mcp",
            "aplan-output",
            "aplan-tui",
            "xtask",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );
    assert_eq!(
        observed_gates,
        [
            "clippy",
            "coverage",
            "doc",
            "evidence",
            "fmt",
            "msrv-build",
            "spec",
            "structural",
            "style",
            "supply-chain",
            "test",
        ]
        .into_iter()
        .collect()
    );
    assert!(
        deferred_controls >= 3,
        "coverage and evidence controls must exercise the full-only boundary"
    );
}
