//! What an automatic gate does to the warning tier (`A1-272`).
//!
//! `README.md`'s *In CI* section, `action.yml`'s `args` input and the sentence
//! `export-agents-md` writes all rest on one fact: **a warning does not fail
//! `check`.** A pilot switched on three warning-tier rules deliberately, wired
//! the gate into CI, and measured two `APLAN018` findings under a green build --
//! nothing in the tool or its documentation had said that would happen.
//!
//! The documentation is the fix, and documentation rots when the behaviour it
//! describes moves under it. So the behaviour is pinned here, in both
//! directions: a warning is green by default, and `deny_warnings` is what makes
//! it a gate. If either assertion ever fails, three documents are wrong and this
//! is where that is said.

use std::path::Path;

mod common;
use common::aplan;

/// A plan with one warning and nothing else, under exactly the config given.
///
/// `APLAN009` is the warning, chosen because it needs one task and no
/// repository: this file is about the *tier*, not about any particular rule in
/// it. `require_effort` defaults on, and is written out so the control below can
/// turn it off against an otherwise identical config.
fn plan_with_one_warning(config: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan dir");
    std::fs::write(
        root.join(".aplan.yml"),
        format!("require_effort: true\n{config}"),
    )
    .expect("config");
    std::fs::write(
        root.join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: A task carrying no estimate\nphase: 1\n\
         status: todo\nowner: kaminsod\npaths:\n- src/**\n---\n\n\
         ## Why\n\nSomething to check.\n",
    )
    .expect("task");
    dir
}

fn check(dir: &Path) -> (String, i32) {
    aplan(dir, &["check", "--root", "aplan"])
}

/// The measured case: the finding is reported, and the build is green.
///
/// The `ok:` is as important as the `0`. A gate's log is read by whoever asks
/// for it, and "ok" is the word a passing build prints -- so a warning here is
/// heard by a person watching and by nothing else.
#[test]
fn a_warning_is_reported_and_the_gate_stays_green() {
    let dir = plan_with_one_warning("");
    let (out, code) = check(dir.path());
    assert!(out.contains("warning[APLAN009]"), "reported: {out}");
    assert_eq!(code, 0, "a warning must not fail the default gate:\n{out}");
    assert!(out.contains("ok: 1 tasks, 1 warning(s)"), "{out}");
}

/// And the key that ends the tier, which is what the documentation sends the
/// reader to. Without this assertion the one above is only half a fact: it would
/// hold just as well if warnings could never block at all.
#[test]
fn deny_warnings_is_what_makes_the_same_finding_block() {
    let dir = plan_with_one_warning("deny_warnings: true\n");
    let (out, code) = check(dir.path());
    assert_eq!(code, 1, "`deny_warnings` must block:\n{out}");
    assert!(
        out.contains("denied: 1 warning(s)"),
        "and say the policy is why:\n{out}"
    );
}

/// The control: with the switch off there is no warning to be green about, so
/// the first test is measuring the tier rather than a plan that was clean.
#[test]
fn without_the_switch_the_same_plan_says_nothing() {
    let dir = plan_with_one_warning("");
    std::fs::write(dir.path().join(".aplan.yml"), "require_effort: false\n").expect("config");
    let (out, code) = check(dir.path());
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("APLAN009"), "nothing was switched on:\n{out}");
    assert!(out.contains("0 warning(s)"), "{out}");
}
