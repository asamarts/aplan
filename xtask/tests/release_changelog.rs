//! The release gate has the changelog and history it needs before publishing
//! (`A1-321`, `A1-402`).

use std::path::{Path, PathBuf};
use std::process::Command;

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
    let workflow = std::fs::read_to_string(workspace_root().join(".github/workflows/release.yml"))
        .expect("release workflow");
    let checkout = workflow
        .find("- uses: actions/checkout@v7")
        .expect("release workflow checks out its source");
    let build = workflow
        .find("- name: build the release artifact")
        .expect("release workflow builds the artifact");
    let checkout_step = &workflow[checkout..build];
    assert!(
        checkout_step.contains("with:\n          fetch-depth: 0"),
        "the release checkout must fetch the tags and history used by `aplan version`"
    );
}
