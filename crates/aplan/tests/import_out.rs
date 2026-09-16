//! Where `aplan import --out` may point, and what the id check means there
//! (`A1-163`).
//!
//! Identity is the `id` field (ADR-0010), so an import refuses to write a task
//! whose id the plan already holds. That question belongs to the plan the files
//! will join: outside the plan root they join none, and the unscoped check
//! refused the ordinary comparison workflow -- importing a source somewhere else
//! to diff against the plan already in the tree. Found while auditing `A1-161`
//! by trying to do exactly that.

use std::path::Path;

mod common;
use common::aplan;

const SOURCE: &str = r#"{"format_version":1,
  "phases":[{"id":"1","title":"A phase","status":"not_started"}],
  "workstreams":[]}"#;

#[test]
fn a_second_import_into_the_same_plan_is_still_refused() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (first, code) = import(dir.path(), &[]);
    assert_eq!(code, 0, "{first}");
    let (second, code) = import(dir.path(), &[]);
    assert_ne!(code, 0, "{second}");
    assert!(second.contains("already in this plan"), "{second}");
    // And the message names the way to do what the caller probably wanted.
    assert!(second.contains("--out"), "{second}");
}

/// The comparison workflow: import a copy somewhere else and diff it. There is
/// no plan there to collide with, so there is nothing to refuse.
#[test]
fn importing_a_copy_outside_the_plan_is_allowed_and_says_what_it_means() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let (first, code) = import(dir.path(), &[]);
    assert_eq!(code, 0, "{first}");

    let copy = dir.path().join("compare");
    let (out, code) = import(dir.path(), &["--out", copy.to_str().expect("utf-8")]);
    assert_eq!(code, 0, "the same ids, outside the plan: {out}");
    assert!(
        copy.join("PHASE-01.md").exists(),
        "the copy was written: {out}"
    );
    // Silence here would be worse than the refusal: nothing checks these files
    // until something points a root at them.
    assert!(out.contains("join no plan"), "{out}");
    assert!(out.contains("--root"), "and how to read them: {out}");

    // The plan itself is untouched, and still loads.
    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "{check}");
    assert!(check.contains("1 tasks"), "one task, not two: {check}");
}

/// `--cut` edits a milestone in the plan to name these tasks, and the plan
/// cannot see them: the cut would be left naming ids that do not resolve. The
/// one combination that leaves the plan worse than it started, so it is refused
/// rather than reported.
#[test]
fn a_cut_with_a_destination_outside_the_plan_is_refused() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let copy = dir.path().join("compare");

    let (out, code) = import(
        dir.path(),
        &["--out", copy.to_str().expect("utf-8"), "--cut", "M-s1"],
    );
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("the plan cannot see"), "{out}");
    assert!(
        !copy.exists(),
        "and nothing was written before the refusal: {out}"
    );
}

/// A relative `--out` is judged where it lands, not by how it is spelled: a
/// destination inside the plan still collides, however many `..` it takes to
/// name it.
#[test]
fn a_relative_destination_is_judged_by_where_it_lands() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let (first, code) = import(dir.path(), &[]);
    assert_eq!(code, 0, "{first}");

    // `./aplan/../aplan/tasks` is the plan's own directory by another name.
    let (out, code) = import(dir.path(), &["--out", "aplan/../aplan/tasks"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("already in this plan"), "{out}");
}

/// A symlink pointing back into the plan is inside it, whatever the path looks
/// like. Judging containment lexically alone let this through, and the import
/// wrote a second `PHASE-01` into a plan that already had one -- the duplicate
/// id the check exists to prevent (`A1-163`, found by its own audit).
///
/// The path-level refusal in `emit` does not cover it: identity is the `id`
/// field, not the file name (ADR-0010), so a task whose id lives in a
/// differently named file leaves the destination path free.
#[test]
fn a_symlink_back_into_the_plan_is_inside_it() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/whatever.md"),
        "---\nid: PHASE-01\ntitle: Already here\nphase: 1\nstatus: todo\neffort_days: 1\n\
         ---\n\nbody\n",
    )
    .expect("task");
    std::os::unix::fs::symlink(dir.path().join("aplan/tasks"), dir.path().join("sneaky"))
        .expect("symlink");

    let (out, code) = import(dir.path(), &["--out", "sneaky"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("already in this plan"), "{out}");

    // Nothing was written, and the plan still holds one task rather than two
    // with one id.
    assert!(
        !dir.path().join("aplan/tasks/PHASE-01.md").exists(),
        "{out}"
    );
    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "{check}");
    assert!(check.contains("1 tasks"), "{check}");
}

/// A destination several directories deep in nothing still lands wherever its
/// existing ancestor really is, so "does not exist yet" does not mean "outside".
#[test]
fn a_destination_that_does_not_exist_yet_is_judged_by_its_ancestor() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let (first, code) = import(dir.path(), &[]);
    assert_eq!(code, 0, "{first}");

    // Inside the plan, two levels below anything that exists.
    let (out, code) = import(dir.path(), &["--out", "aplan/tasks/nested/deep"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("already in this plan"), "{out}");

    // Outside it, the same shape, allowed.
    let (out, code) = import(dir.path(), &["--out", "elsewhere/nested/deep"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("join no plan"), "{out}");
    assert!(
        dir.path()
            .join("elsewhere/nested/deep/PHASE-01.md")
            .exists(),
        "{out}"
    );
}

fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.join("roadmap.json"), SOURCE).expect("source");
}

fn import(dir: &Path, extra: &[&str]) -> (String, i32) {
    let mut args = vec!["import", "roadmap.json", "--format", "roadmap-json"];
    args.extend_from_slice(extra);
    aplan(dir, &args)
}
