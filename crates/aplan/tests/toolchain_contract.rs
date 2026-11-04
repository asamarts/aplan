//! The development/release compiler has one exact source (`A1-619`).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn read(relative: &str) -> String {
    std::fs::read_to_string(root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

fn toolchain(command: &str, path: &Path) -> Output {
    Command::new(root().join("ci/scripts/toolchain.sh"))
        .args([command, path.to_str().expect("UTF-8 fixture path")])
        .current_dir(root())
        .output()
        .expect("run toolchain contract reader")
}

fn contract(path: &Path) -> Output {
    toolchain("contract", path)
}

fn successful_field(command: &str, path: &Path) -> String {
    let output = toolchain(command, path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("toolchain field is UTF-8")
        .trim()
        .to_string()
}

fn exact_semver(word: &str) -> bool {
    let word = word.trim_matches(|ch: char| !ch.is_ascii_digit() && ch != '.');
    let parts = word.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
}

fn independent_toolchain_versions(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| {
            line.contains("RUST_VERSION")
                || line.contains("toolchain install")
                || line.contains("default-toolchain")
                || line.contains("rustup default")
                || line.contains("--toolchain")
                || line.contains("channel=")
                || line.contains("cache-key=rust-")
        })
        .flat_map(|line| line.split(|ch: char| !ch.is_ascii_digit() && ch != '.'))
        .filter(|word| exact_semver(word))
        .map(str::to_string)
        .collect()
}

fn assert_current_contract() {
    let canonical = root().join("rust-toolchain.toml");
    let channel = successful_field("channel", &canonical);
    let components = successful_field("components", &canonical);
    let cache_key = successful_field("cache-key", &canonical);
    assert!(exact_semver(&channel), "{channel}");
    let component_set = components
        .split(',')
        .collect::<std::collections::BTreeSet<_>>();
    assert!(component_set.contains("rustfmt"), "{components}");
    assert!(component_set.contains("clippy"), "{components}");
    assert_eq!(
        cache_key,
        format!("rust-{channel}-{}", components.replace(',', "-"))
    );

    let current = contract(&canonical);
    assert!(
        current.status.success(),
        "{}",
        String::from_utf8_lossy(&current.stderr)
    );
    let current = String::from_utf8(current.stdout).expect("contract is UTF-8");
    assert_eq!(
        current.trim(),
        format!("channel={channel} components={components} cache-key={cache_key}")
    );
}

fn fixture_version() -> String {
    [9, 98, 7]
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

fn manifest(channel: &str, components: &str, profile: &str) -> String {
    format!(
        "[toolchain]\nchannel = \"{channel}\"\ncomponents = [{components}]\nprofile = \"{profile}\"\n"
    )
}

fn assert_rejected_contracts(fixture: &Path) {
    let version = fixture_version();
    let second_version = format!("{version}.1");
    for (name, source, expected) in [
        (
            "moving.toml",
            manifest("stable", "\"rustfmt\", \"clippy\"", "minimal"),
            "exact Rust release",
        ),
        (
            "missing-component.toml",
            manifest(&version, "\"rustfmt\"", "minimal"),
            "required component 'clippy' is missing",
        ),
        (
            "second-channel.toml",
            format!(
                "[toolchain]\nchannel = \"{version}\"\nchannel = \"{second_version}\"\ncomponents = [\"rustfmt\", \"clippy\"]\nprofile = \"minimal\"\n"
            ),
            "exactly one channel",
        ),
        (
            "broad-profile.toml",
            manifest(&version, "\"rustfmt\", \"clippy\"", "default"),
            "exactly one minimal profile",
        ),
    ] {
        let path = fixture.join(name);
        std::fs::write(&path, source).expect("write rejected contract");
        let output = contract(&path);
        assert_eq!(output.status.code(), Some(1), "{name}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn assert_simulated_bump(fixture: &Path) {
    let canonical = read("rust-toolchain.toml");
    let current = successful_field("channel", &root().join("rust-toolchain.toml"));
    let bumped_version = fixture_version();
    assert_ne!(
        current, bumped_version,
        "the simulated bump must move the pin"
    );
    let bumped = fixture.join("bumped.toml");
    let bumped_source = canonical.replacen(
        &format!("channel = \"{current}\""),
        &format!("channel = \"{bumped_version}\""),
        1,
    );
    assert_ne!(
        canonical, bumped_source,
        "the canonical channel was not found"
    );
    std::fs::write(&bumped, bumped_source).expect("write simulated bump");
    let output = contract(&bumped);
    assert!(output.status.success());
    let output = String::from_utf8(output.stdout).expect("bump output");
    assert_eq!(
        output.trim(),
        format!(
            "channel={bumped_version} components=rustfmt,clippy cache-key=rust-{bumped_version}-rustfmt-clippy"
        )
    );
}

fn assert_every_consumer_derives_the_contract() {
    let consumers = [
        "ci/release.Containerfile",
        "ci/scripts/prepare-hosted.sh",
        "ci/scripts/validate.sh",
        "ci/scripts/release.sh",
        ".github/workflows/ci.yml",
        ".github/workflows/release.yml",
        "crates/aplan/tests/toolchain_contract.rs",
    ];
    for relative in consumers {
        let text = read(relative);
        assert!(
            independent_toolchain_versions(&text).is_empty(),
            "{relative} carries a second exact development/release version: {:?}",
            independent_toolchain_versions(&text)
        );
    }
    let seeded_version = [1, 98, 0]
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".");
    let seeded_duplicate = format!("assert!(output.contains(\"channel={seeded_version}\"))");
    assert_eq!(
        independent_toolchain_versions(&seeded_duplicate),
        [seeded_version],
        "the duplicate-version audit's negative control stopped detecting a second pin"
    );

    let builder = read("ci/release.Containerfile");
    assert!(builder.contains("COPY rust-toolchain.toml"));
    assert!(builder.contains("aplan-toolchain.sh channel"));
    assert!(!builder.contains("ARG RUST_VERSION="));
    let hosted = read("ci/scripts/prepare-hosted.sh");
    assert!(hosted.contains("toolchain.sh\" channel"), "{hosted}");
    assert!(hosted.contains("toolchain.sh\" components"), "{hosted}");
    let context = read(".containerignore");
    assert!(context.lines().any(|line| line == "**"));
    for required in [
        "!rust-toolchain.toml",
        "!ci/scripts/toolchain.sh",
        "!ci/release.Containerfile",
    ] {
        assert!(
            context.lines().any(|line| line == required),
            "container context omits {required}"
        );
    }
}

#[test]
fn toolchain_contract_rejects_drift_and_derives_every_consumer() {
    assert_current_contract();
    let fixture = tempfile::Builder::new()
        .prefix("aplan-toolchain-contract-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("toolchain fixtures");
    assert_rejected_contracts(fixture.path());
    assert_simulated_bump(fixture.path());
    assert_every_consumer_derives_the_contract();
}

fn msrv_is_explicit(script: &str) -> bool {
    script.contains("MSRV=\"$(sed -n")
        && script.contains("cargo \"+${MSRV}\" check --locked --workspace")
        && !script.lines().any(|line| {
            line.trim_start().starts_with("cargo check")
                || line.trim_start().starts_with("cargo +stable check")
        })
}

fn release_is_pinned(script: &str, workflow: &str) -> bool {
    let verify = script.find("ci/scripts/toolchain.sh verify");
    let first_cargo = script.find("cargo build");
    verify.is_some_and(|verify| first_cargo.is_some_and(|cargo| verify < cargo))
        && workflow.contains("run: ci/scripts/release.sh")
        && !workflow.contains("ci/scripts/latest-stable.sh")
}

#[test]
fn msrv_latest_stable_and_release_keep_distinct_authorities() {
    let msrv = read("ci/scripts/msrv-build.sh");
    assert!(msrv_is_explicit(&msrv), "{msrv}");
    assert!(
        !msrv_is_explicit(&msrv.replace("cargo \"+${MSRV}\"", "cargo")),
        "the MSRV negative control no longer detects use of the development compiler"
    );

    let release = read("ci/scripts/release.sh");
    let release_workflow = read(".github/workflows/release.yml");
    assert!(release_is_pinned(&release, &release_workflow));
    assert!(
        !release_is_pinned(
            &release.replace("ci/scripts/toolchain.sh verify", "true"),
            &release_workflow
        ),
        "the release negative control no longer detects a pin bypass"
    );

    let latest_script = read("ci/scripts/latest-stable.sh");
    let latest_workflow = read(".github/workflows/latest-stable.yml");
    for expected in [
        "rustup toolchain install stable",
        "export RUSTUP_TOOLCHAIN=stable",
        "this is not a full or release receipt",
    ] {
        assert!(latest_script.contains(expected), "missing {expected}");
    }
    assert!(latest_workflow.contains("schedule:"));
    assert!(latest_workflow.contains("workflow_dispatch:"));
    assert!(latest_workflow.contains("run: ci/scripts/latest-stable.sh"));

    let validate = read("ci/scripts/validate.sh");
    assert!(validate.contains("\"$SCRIPT_DIR/toolchain.sh\" verify"));
    assert!(validate.contains("[[ \"$gate\" != \"msrv-build\" ]]"));

    // A1-616 owns introducing external caches. When one appears, its key must
    // consume the identity this task exposes rather than inventing a compiler
    // spelling beside it.
    for workflow in [
        read(".github/workflows/ci.yml"),
        release_workflow,
        latest_workflow,
    ] {
        for line in workflow
            .lines()
            .filter(|line| line.trim_start().starts_with("key:"))
        {
            assert!(
                line.contains("APLAN_TOOLCHAIN_CACHE_KEY"),
                "external cache key omits the canonical toolchain identity: {line}"
            );
        }
    }
}
