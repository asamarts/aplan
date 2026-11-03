//! The generators say when the working tree holds work a clean checkout does not
//! (`A1-144`).
//!
//! Driven through the built binary, which is the half a unit test cannot reach:
//! that `gen-roadmap` and `check-docs` actually *call* the note. The binary
//! resolves its root from `CARGO_MANIFEST_DIR` at compile time, so it can only
//! read this repository's plan -- and an untracked file under `aplan/tasks/`
//! would race every test that reads the live plan. An untracked markdown file
//! beside the tasks directory is invisible to the loader (which reads
//! `aplan/tasks/*.md`) and visible to `git ls-files --others`, which is exactly
//! the difference being tested.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Removes the probe file however the test ends, including on a panic.
struct Probe(PathBuf);

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn xtask(root: &Path, args: &[&str]) -> String {
    let out = Command::new(crate::common::xtask())
        .args(args)
        .current_dir(root)
        .output()
        .expect("running xtask");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}

/// Whether this copy of the workspace is a git checkout.
///
/// It is not always one: the CI container this project verifies against extracts
/// a tarball, and a released source archive has no `.git` either. The note is
/// documented as silent where git cannot answer, so both states are asserted
/// rather than one of them being a reason to fail (`A1-144`, found by running
/// the suite in the container).
fn is_a_checkout(root: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(root)
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn the_generators_note_an_untracked_plan_file() {
    let root = repo_root();
    let checkout = is_a_checkout(&root);
    if checkout {
        // Nothing untracked: neither gate says anything about the working tree.
        let before = xtask(&root, &["gen-roadmap", "--check"]);
        assert!(
            !before.contains("does not track"),
            "this working tree already holds untracked plan files, so the test cannot \
             tell the two states apart:\n{before}"
        );
    }

    let probe = root.join("aplan/a1-144-probe.md");
    std::fs::write(&probe, "A file git does not track.\n").expect("probe");
    let _cleanup = Probe(probe);

    for (args, what) in [
        (vec!["gen-roadmap", "--check"], "this rollup"),
        (vec!["check-docs"], "this comparison"),
    ] {
        let out = xtask(&root, &args);
        if checkout {
            assert!(
                out.contains("does not track") && out.contains("a1-144-probe.md"),
                "`{}` did not note the untracked file:\n{out}",
                args.join(" ")
            );
            assert!(
                out.contains(what),
                "the note must say what reads it:\n{out}"
            );
        } else {
            // Silent where git cannot answer, which is the documented
            // degradation: a generator that failed because there is no
            // repository would be a worse hazard than the one being reported.
            assert!(
                !out.contains("does not track"),
                "there is no repository here, so nothing could have been asked:\n{out}"
            );
        }
        // A note, not a refusal: the gate's own verdict is unchanged by it, in
        // either environment.
        assert!(
            out.contains("up to date") || out.contains("agree with the plan"),
            "the note turned a passing gate into a failure:\n{out}"
        );
    }
}
