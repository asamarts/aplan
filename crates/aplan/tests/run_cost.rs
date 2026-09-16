//! What a run costs, and when it is worth saying (`A1-309`).
//!
//! The measurement lives in `aplan-evidence` and is unit-tested there. These two
//! pin the part an adopter meets: the line appears when a runner reached
//! something worth knowing, and does not otherwise. A report nobody needs is a
//! report nobody reads, which is the failure mode that matters for a note.

mod common;

use common::run;
use std::path::Path;

/// A runner that forks enough to be worth mentioning is mentioned.
#[test]
fn a_runner_that_forks_is_reported() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(
        dir.path(),
        // Forty children, alive long enough for the sampler to see them.
        "for i in $(seq 1 40); do sleep 1 & done; wait; printf 'known\\n'",
    );

    let out = run(dir.path(), &["verify"]);
    assert_eq!(out.code, 0, "{}", out.text());
    assert!(
        out.stderr.contains("peaked at"),
        "a forty-process runner is worth a line: {}",
        out.text()
    );
    assert!(
        out.stderr.contains("summed RSS"),
        "the line names which figure it is, because summed and proportional \
         differ six-fold: {}",
        out.text()
    );
}

/// A runner that forks nothing says nothing.
///
/// The threshold is the whole point: this project's own `cargo` runners sit
/// under it, so an ordinary run gains no line at all.
#[test]
fn a_quiet_runner_is_not_reported() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), "printf 'known\\n'");

    let out = run(dir.path(), &["verify"]);
    assert_eq!(out.code, 0, "{}", out.text());
    assert!(
        !out.stderr.contains("peaked at"),
        "a runner that forks nothing must not be mentioned: {}",
        out.text()
    );
    assert!(
        !out.stderr.contains("could not be measured"),
        "and must not be reported as unmeasurable: `/proc` is readable here, \
         the command merely finished quickly: {}",
        out.text()
    );
}

/// One `done` task citing one test, and a runner whose `list` is the argument.
fn fixture(dir: &Path, list: &str) {
    std::process::Command::new("git")
        .args(["init", "-q", "."])
        .current_dir(dir)
        .status()
        .expect("git init");
    std::fs::write(
        dir.join(".aplan.yml"),
        format!(
            "root: aplan\nrequire_cut_membership: false\ntest:\n  name: probe\n  \
             list: [sh, -c, \"{list}\"]\n  run: [sh, -c, 'exit 0', '{{test}}']\n"
        ),
    )
    .expect("config");
    std::fs::create_dir_all(dir.join("aplan")).expect("plan dir");
    std::fs::write(
        dir.join("aplan/T-1.md"),
        "---\nid: T-1\ntitle: cites a test\nphase: 1\nstatus: done\n\
         effort_days: 1\ntags: [cross-cutting]\nevidence:\n- kind: test\n  \
         ref: known\n---\n\n## Why\n\nx\n\n## Acceptance criteria\n\n- [x] cited\n",
    )
    .expect("task");
}
