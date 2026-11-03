//! Executable privacy boundary for the fresh public repository (`A1-727`).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("aplan crate is two levels below the workspace")
        .to_path_buf()
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct CandidateFixture {
    temp: tempfile::TempDir,
    source: PathBuf,
    candidate: PathBuf,
    manifest: PathBuf,
    report: PathBuf,
}

impl CandidateFixture {
    fn build() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("publication-boundary-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("temporary publication fixture");
        let source = temp.path().join("source");
        let candidate = temp.path().join("candidate");
        let manifest = temp.path().join("paths.yml");
        let report = temp.path().join("report.json");
        std::fs::create_dir(&source).expect("source directory");
        std::fs::write(source.join("allowed.txt"), "reviewed public bytes\n")
            .expect("public fixture");
        std::fs::write(
            source.join("identity.rs"),
            "// PrivateAdopter supplied this fixture\nconst PRIVATEADOPTER: &str = \"private-adopter\";\n",
        )
        .expect("identity fixture");
        std::fs::write(
            source.join("config.yml"),
            "# privateadopter remains private even in configuration\n",
        )
        .expect("configuration fixture");
        std::fs::create_dir(source.join(".github")).expect("GitHub fixture directory");
        std::fs::write(
            source.join(".github/dependabot.yml"),
            "version: 2\nupdates:\n  - package-ecosystem: cargo\n    directory: \"/\"\n    schedule:\n      interval: weekly\n  - package-ecosystem: github-actions\n    directory: \"/\"\n    schedule:\n      interval: weekly\n",
        )
        .expect("Dependabot fixture");
        std::fs::write(
            &manifest,
            r#"{
  "schema": 1,
  "releases": [
    {"ref": "v0.0.1", "tag": "v0.0.1", "assets": []}
  ],
  "paths": {
    ".github/dependabot.yml": "public",
    "allowed.txt": "public",
    "config.yml": "public",
    "identity.rs": "public"
  },
  "overlays": [],
  "transforms": [
    {
      "kind": "public_dependabot",
      "output": ".github/dependabot.yml",
      "snapshots": ["current"],
      "source": ".github/dependabot.yml"
    }
  ],
  "private_output_prefixes": ["docs/pilots/"],
  "forbidden_content": [
    {"id": "absolute-home", "literal": "/home/"}
  ],
  "private_identities": [
    {"literal": "private-adopter", "replacement": "public_adopter1"},
    {"literal": "privateadopter", "replacement": "publicadopter1"}
  ],
  "required_output_paths": ["allowed.txt"]
}
"#,
        )
        .expect("fixture manifest");
        git(&source, &["init", "-q", "-b", "main"]);
        git(&source, &["config", "user.name", "Fixture"]);
        git(
            &source,
            &["config", "user.email", "fixture@example.invalid"],
        );
        git(
            &source,
            &[
                "add",
                ".github/dependabot.yml",
                "allowed.txt",
                "config.yml",
                "identity.rs",
            ],
        );
        git(&source, &["commit", "-q", "-m", "fixture"]);
        git(&source, &["tag", "v0.0.1"]);

        let output = Command::new(workspace().join("ci/scripts/build-public-snapshot.sh"))
            .args(["--source", source.to_str().expect("source path")])
            .args(["--revision", "HEAD"])
            .args(["--manifest", manifest.to_str().expect("manifest path")])
            .args(["--output", candidate.to_str().expect("candidate path")])
            .args(["--report", report.to_str().expect("report path")])
            .output()
            .expect("publication builder runs");
        assert!(
            output.status.success(),
            "publication fixture build failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        Self {
            temp,
            source,
            candidate,
            manifest,
            report,
        }
    }

    fn audit(&self) -> Output {
        self.audit_candidate(&self.candidate)
    }

    fn audit_candidate(&self, candidate: &Path) -> Output {
        Command::new(workspace().join("ci/scripts/audit-public-snapshot.sh"))
            .args(["--source", self.source.to_str().expect("source path")])
            .args(["--candidate", candidate.to_str().expect("candidate path")])
            .args(["--manifest", self.manifest.to_str().expect("manifest path")])
            .args(["--report", self.report.to_str().expect("report path")])
            .output()
            .expect("publication auditor runs")
    }
}

#[test]
fn clean_candidate_passes_the_executable_privacy_audit() {
    let fixture = CandidateFixture::build();
    let output = fixture.audit();
    assert!(
        output.status.success(),
        "clean candidate failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn private_identities_are_sanitized_in_every_text_shape_and_snapshot() {
    let fixture = CandidateFixture::build();
    let current = std::fs::read_to_string(fixture.candidate.join("identity.rs"))
        .expect("current identity fixture");
    assert_eq!(
        current,
        "// Publicadopter1 supplied this fixture\nconst PUBLICADOPTER1: &str = \"public_adopter1\";\n"
    );
    let config = std::fs::read_to_string(fixture.candidate.join("config.yml"))
        .expect("sanitized configuration fixture");
    assert_eq!(
        config,
        "# publicadopter1 remains private even in configuration\n"
    );

    let historical = Command::new("git")
        .args(["show", "v0.0.1:identity.rs"])
        .current_dir(&fixture.candidate)
        .output()
        .expect("read reconstructed release snapshot");
    assert!(historical.status.success());
    assert_eq!(String::from_utf8_lossy(&historical.stdout), current);

    let report = std::fs::read_to_string(&fixture.report).expect("publication report");
    assert!(report.contains("identity_redactions"));
    for private in ["PrivateAdopter", "PRIVATEADOPTER", "private-adopter"] {
        assert!(
            !current.contains(private),
            "private identity survived: {private}"
        );
        assert!(
            !config.contains(private),
            "private identity survived: {private}"
        );
        assert!(
            !String::from_utf8_lossy(&historical.stdout).contains(private),
            "private identity survived in the release snapshot: {private}"
        );
    }
}

#[test]
fn public_candidate_pauses_every_dependabot_version_update_ecosystem() {
    let fixture = CandidateFixture::build();
    let policy = std::fs::read_to_string(fixture.candidate.join(".github/dependabot.yml"))
        .expect("public Dependabot policy");
    assert_eq!(policy.matches("  - package-ecosystem:").count(), 2);
    assert_eq!(policy.matches("    open-pull-requests-limit: 0").count(), 2);
    assert!(policy.contains(
        "package-ecosystem: cargo\n    directory: \"/\"\n    open-pull-requests-limit: 0\n"
    ));
    assert!(policy.contains(
        "package-ecosystem: github-actions\n    directory: \"/\"\n    open-pull-requests-limit: 0\n"
    ));
}

#[test]
fn relocated_candidate_is_bound_by_content_not_builder_path() {
    let fixture = CandidateFixture::build();
    let relocated = fixture.temp.path().join("remote-clone");
    let candidate = fixture.candidate.to_str().expect("candidate path");
    let destination = relocated.to_str().expect("relocated path");
    git(
        fixture.temp.path(),
        &["clone", "-q", "--no-local", candidate, destination],
    );
    git(&relocated, &["remote", "remove", "origin"]);

    let output = fixture.audit_candidate(&relocated);
    assert!(
        output.status.success(),
        "relocated candidate failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn new_private_path_is_rejected_by_the_audit() {
    let fixture = CandidateFixture::build();
    let private = fixture.candidate.join("docs/pilots/new-adopter.md");
    std::fs::create_dir_all(private.parent().expect("private parent"))
        .expect("private fixture directory");
    std::fs::write(private, "confidential observation\n").expect("private fixture");

    let output = fixture.audit();
    assert!(
        !output.status.success(),
        "the private path passed the audit"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("private output path"),
        "the refusal did not identify the private path:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn forbidden_content_in_an_admitted_file_is_rejected_by_the_audit() {
    let fixture = CandidateFixture::build();
    std::fs::write(
        fixture.candidate.join("allowed.txt"),
        "review artifact at /home/alice/private/session.json\n",
    )
    .expect("hostile admitted file");

    let output = fixture.audit();
    assert!(
        !output.status.success(),
        "the forbidden string passed the audit"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("absolute-home"),
        "the refusal did not identify the content rule:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn private_identity_added_after_construction_is_rejected_by_the_audit() {
    let fixture = CandidateFixture::build();
    std::fs::write(
        fixture.candidate.join("allowed.txt"),
        "the undisclosed PrivateAdopter repository\n",
    )
    .expect("private identity fixture");

    let output = fixture.audit();
    assert!(
        !output.status.success(),
        "the private identity passed the audit"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("private identity 'privateadopter'"),
        "the refusal did not identify the private identity:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn release_archive_payloads_with_private_identities_are_rejected() {
    let temp = tempfile::Builder::new()
        .prefix("publication-release-identity-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("release identity fixture");
    let payload = temp.path().join("payload");
    let release = temp.path().join("assets/v0.0.1");
    std::fs::create_dir_all(&payload).expect("payload directory");
    std::fs::create_dir_all(&release).expect("release directory");
    let mut hostile_payload = vec![b'x'; 1024 * 1024 - 8];
    hostile_payload.extend_from_slice(b" PrivateAdopter\n");
    std::fs::write(payload.join("aplan"), hostile_payload)
        .expect("hostile release payload crossing the audit chunk boundary");
    let archive = release.join("aplan-v0.0.1.tar.gz");
    let tar = Command::new("tar")
        .args(["-czf"])
        .arg(&archive)
        .args(["-C"])
        .arg(&payload)
        .arg("aplan")
        .output()
        .expect("construct release archive");
    assert!(
        tar.status.success(),
        "tar failed:\n{}",
        String::from_utf8_lossy(&tar.stderr)
    );
    let checksum = Command::new("sha256sum")
        .arg(&archive)
        .output()
        .expect("hash release archive");
    assert!(checksum.status.success());
    let checksum = String::from_utf8(checksum.stdout)
        .expect("ASCII checksum")
        .split_whitespace()
        .next()
        .expect("checksum value")
        .to_string();
    let size = std::fs::metadata(&archive).expect("archive metadata").len();
    let manifest = temp.path().join("manifest.json");
    std::fs::write(
        &manifest,
        format!(
            r#"{{
  "releases": [{{
    "tag": "v0.0.1",
    "assets": [{{
      "name": "aplan-v0.0.1.tar.gz",
      "sha256": "{checksum}",
      "size": {size}
    }}]
  }}],
  "private_identities": [{{
    "literal": "privateadopter",
    "replacement": "publicadopter1"
  }}]
}}
"#
        ),
    )
    .expect("release manifest");

    let probe = r#"
import importlib.util
import json
from pathlib import Path
import sys

spec = importlib.util.spec_from_file_location("publication", sys.argv[1])
publication = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publication)
try:
    publication.audit_release_assets(
        json.loads(Path(sys.argv[2]).read_text()), Path(sys.argv[3])
    )
except publication.PublicationError as error:
    print(error, file=sys.stderr)
    raise SystemExit(0 if "private identity 'privateadopter'" in str(error) else 2)
raise SystemExit(1)
"#;
    let output = Command::new("python3")
        .args(["-B", "-c", probe])
        .arg(workspace().join("ci/publication/publication.py"))
        .arg(&manifest)
        .arg(temp.path().join("assets"))
        .output()
        .expect("release identity auditor runs");
    assert!(
        output.status.success(),
        "release identity audit did not refuse the payload:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_private_pilot_log_requires_an_identity_registry_entry() {
    let temp = tempfile::Builder::new()
        .prefix("publication-identity-census-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("identity census fixture");
    let manifest = temp.path().join("manifest.json");
    std::fs::write(
        &manifest,
        r#"{
  "schema": 1,
  "paths": {"docs/pilots/undisclosed-repository.md": "private"},
  "private_identities": []
}
"#,
    )
    .expect("identity census manifest");
    let probe = r#"
import importlib.util
import sys

spec = importlib.util.spec_from_file_location("publication", sys.argv[1])
publication = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publication)
try:
    publication.load_manifest(publication.Path(sys.argv[2]))
except publication.PublicationError as error:
    print(error, file=sys.stderr)
    raise SystemExit(0 if "no private identity registry entry" in str(error) else 2)
raise SystemExit(1)
"#;
    let output = Command::new("python3")
        .args(["-B", "-c", probe])
        .arg(workspace().join("ci/publication/publication.py"))
        .arg(&manifest)
        .output()
        .expect("identity census probe runs");
    assert!(
        output.status.success(),
        "unregistered private pilot identity was accepted:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn private_identity_aliases_must_preserve_source_layout_width() {
    let temp = tempfile::Builder::new()
        .prefix("publication-identity-width-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("identity width fixture");
    let manifest = temp.path().join("manifest.json");
    std::fs::write(
        &manifest,
        r#"{
  "schema": 1,
  "paths": {"allowed.txt": "public"},
  "private_identities": [
    {"literal": "privateadopter", "replacement": "short_alias"}
  ]
}
"#,
    )
    .expect("identity width manifest");
    let probe = r#"
import importlib.util
import sys

spec = importlib.util.spec_from_file_location("publication", sys.argv[1])
publication = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publication)
try:
    publication.load_manifest(publication.Path(sys.argv[2]))
except publication.PublicationError as error:
    print(error, file=sys.stderr)
    raise SystemExit(0 if "same byte length" in str(error) else 2)
raise SystemExit(1)
"#;
    let output = Command::new("python3")
        .args(["-B", "-c", probe])
        .arg(workspace().join("ci/publication/publication.py"))
        .arg(&manifest)
        .output()
        .expect("identity width probe runs");
    assert!(
        output.status.success(),
        "source-layout-changing identity alias was accepted:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn pinned_public_release_refs_refuse_transform_drift() {
    let fixture = CandidateFixture::build();
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&fixture.report).expect("publication report bytes"))
            .expect("publication report JSON");
    let release = &report["snapshots"][0];
    let mut manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&fixture.manifest).expect("publication manifest bytes"),
    )
    .expect("publication manifest JSON");
    manifest["releases"][0]["public_commit"] = release["public_commit"].clone();
    manifest["releases"][0]["public_tag_object"] = release["public_tag_object"].clone();
    manifest["private_identities"][0]["replacement"] =
        serde_json::Value::String("public_adopter2".into());
    std::fs::write(
        &fixture.manifest,
        serde_json::to_vec_pretty(&manifest).expect("serialized manifest"),
    )
    .expect("drifted publication manifest");

    let candidate = fixture.temp.path().join("drifted-candidate");
    let report = fixture.temp.path().join("drifted-report.json");
    let output = Command::new(workspace().join("ci/scripts/build-public-snapshot.sh"))
        .args(["--source", fixture.source.to_str().expect("source path")])
        .args(["--revision", "HEAD"])
        .args([
            "--manifest",
            fixture.manifest.to_str().expect("manifest path"),
        ])
        .args(["--output", candidate.to_str().expect("candidate path")])
        .args(["--report", report.to_str().expect("report path")])
        .output()
        .expect("drifted publication builder runs");
    assert!(!output.status.success(), "transform drift was accepted");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("reconstructed public commit differs for v0.0.1"),
        "the refusal did not name the drifted public ref:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
