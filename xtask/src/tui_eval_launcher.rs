//! Launcher for the opt-in, separately built synthetic TUI evaluation tool.
//!
//! Provider integration is deliberately outside the workspace: normal CI and
//! coverage are hermetic, while `cargo xtask tui-eval` remains the stable local
//! entry point.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub(crate) fn command(args: &[String], root: &Path) -> Result<()> {
    let manifest = manifest_path(root);
    if !manifest.is_file() {
        bail!("synthetic TUI evaluation manifest is missing");
    }
    let status = cargo_command(root, &manifest, args)
        .status()
        .context("starting the synthetic TUI evaluation tool")?;
    if !status.success() {
        bail!("synthetic TUI evaluation tool failed");
    }
    Ok(())
}

fn manifest_path(root: &Path) -> PathBuf {
    root.join("tools/tui-eval/Cargo.toml")
}

fn cargo_command(root: &Path, manifest: &Path, args: &[String]) -> Command {
    let mut command = Command::new("cargo");
    command
        .args(["run", "--quiet", "--locked", "--manifest-path"])
        .arg(manifest)
        .arg("--")
        .args(args)
        .current_dir(root);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launcher_points_at_the_standalone_harness() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let manifest = manifest_path(&root);
        assert!(manifest.is_file(), "missing {}", manifest.display());
        let source = manifest.parent().unwrap_or(&root).join("src/harness.rs");
        assert!(source.is_file(), "missing {}", source.display());
        let paper = manifest.parent().unwrap_or(&root).join("src/paper.rs");
        assert!(paper.is_file(), "missing {}", paper.display());
    }

    #[test]
    fn the_hardened_harness_contract_is_auditable() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let source_root = root.join("tools/tui-eval/src");
        let main = std::fs::read_to_string(source_root.join("main.rs")).expect("read main source");
        let harness =
            std::fs::read_to_string(source_root.join("harness.rs")).expect("read harness source");
        let paper =
            std::fs::read_to_string(source_root.join("paper.rs")).expect("read paper source");

        for command in ["review-self-test", "review-export", "review-audit"] {
            assert!(main.contains(command), "missing review command {command}");
        }
        for boundary in [
            "--clearenv",
            "run_isolation_probe",
            "project_sqlite_database",
            "validate_sqlite_schema",
            "expected_schema_sha256",
            "os_isolation_passed",
        ] {
            assert!(
                harness.contains(boundary),
                "missing adapter-isolation boundary {boundary}"
            );
        }
        for audit in [
            "ADVERSARIAL_CANARY_SCREEN",
            "critic_grounding_errors",
            "invocation_ids",
            "cohort_sha256",
            "disposition",
            "STABILITY_CANARY_REPETITIONS",
            "ATTEMPT_DIRECTORY",
            "ensure_no_prior_attempt",
            "cohort_session_schedule",
            "rejected_turn",
            "ensure_tracked_clean_file",
            "canonical cohort does not exactly match its tracked attempt",
        ] {
            assert!(paper.contains(audit), "missing replay audit field {audit}");
        }
        assert!(
            !paper.contains("Protocol repair:"),
            "actor retries must not repair an invalid evaluation turn"
        );
    }

    #[test]
    fn the_launcher_preserves_each_argument_and_pins_the_lockfile() {
        let root = Path::new("/workspace");
        let manifest = manifest_path(root);
        let args = vec!["canary".to_string(), "claude".to_string()];
        let command = cargo_command(root, &manifest, &args);
        assert_eq!(command.get_program(), "cargo");
        assert_eq!(command.get_current_dir(), Some(root));
        let actual = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let manifest = manifest.to_string_lossy().into_owned();
        assert_eq!(
            actual,
            [
                "run".to_string(),
                "--quiet".to_string(),
                "--locked".to_string(),
                "--manifest-path".to_string(),
                manifest,
                "--".to_string(),
                "canary".to_string(),
                "claude".to_string()
            ]
        );
    }

    #[test]
    fn a_missing_standalone_harness_is_refused_before_cargo_runs() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let error = command(&[], root.path()).expect_err("missing harness must fail");
        assert!(error.to_string().contains("manifest is missing"));
    }
}
