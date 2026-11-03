//! Creating a new cut around work already owned by another cut (`A1-704`).

use crate::common::aplan;
use std::os::unix::fs::PermissionsExt as _;

fn write(path: &std::path::Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("fixture directory");
    }
    std::fs::write(path, text).expect("fixture file");
}

fn amorph_plan() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    write(
        &dir.path().join(".aplan.yml"),
        "root: aplan\ndeny_warnings: true\nrequire_cut_membership: true\n",
    );
    let tasks = dir.path().join("aplan/tasks");
    for (id, title) in [
        ("AM-074", "Retained release work"),
        ("AM-075", "Run hosted CI for fork pull requests"),
    ] {
        write(
            &tasks.join(format!("{id}.md")),
            &format!(
                "---\nid: {id}\ntitle: {title}\nphase: 6\nstatus: todo\n\
                 effort_days: 1\npaths: [src/**]\n---\n\n## Why\n\nBecause.\n"
            ),
        );
    }
    write(
        &tasks.join("M-s6.md"),
        "---\nid: M-s6\ntitle: Release six\nphase: 6\nstatus: todo\n\
         blocked_by: [AM-074, AM-075]\ntags: [milestone, s6]\n---\n\n## Why\n\nBecause.\n",
    );
    dir
}

fn run_new_cut(dir: &std::path::Path, extra: &[&str]) -> (String, i32) {
    let mut args = vec![
        "add",
        "Hosted CI after publication",
        "--id",
        "M-sG",
        "--phase",
        "7",
        "--tag",
        "milestone",
        "--tag",
        "sG",
    ];
    args.extend_from_slice(extra);
    aplan(dir, &args)
}

/// Proves `add.creates_cut_around_existing_members` with the exact P00002
/// transition: the source loses its member only when the new cut is created.
#[test]
fn a_new_cut_moves_existing_amorph_work_atomically() {
    let dir = amorph_plan();
    let source = dir.path().join("aplan/tasks/M-s6.md");
    let destination = dir.path().join("aplan/tasks/M-sG.md");
    let before = std::fs::read_to_string(&source).expect("source cut");

    let (out, code) = run_new_cut(dir.path(), &["--blocked-by", "M-s6", "--member", "AM-075"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(destination.to_string_lossy().as_ref()),
        "{out}"
    );
    assert!(out.contains(source.to_string_lossy().as_ref()), "{out}");
    assert!(out.contains("added AM-075 to `M-sG`"), "{out}");
    assert!(out.contains("removed AM-075 from `M-s6`"), "{out}");

    let old = std::fs::read_to_string(&source).expect("source after");
    assert!(old.contains("blocked_by: [AM-074]"), "{old}");
    let new = std::fs::read_to_string(&destination).expect("new cut");
    assert!(new.contains("blocked_by:\n- M-s6\n- AM-075"), "{new}");

    let (checked, check_code) = aplan(dir.path(), &["check"]);
    assert_eq!(check_code, 0, "{checked}");
    assert_ne!(old, before, "the source membership did not move");
}

#[test]
fn dry_run_and_multi_source_review_name_every_transition_without_writing() {
    let dir = amorph_plan();
    let tasks = dir.path().join("aplan/tasks");
    for id in ["AM-072", "AM-073"] {
        write(
            &tasks.join(format!("{id}.md")),
            &format!(
                "---\nid: {id}\ntitle: {id}\nphase: 5\nstatus: todo\neffort_days: 1\n\
                 paths: [src/**]\n---\n\n## Why\n\nBecause.\n"
            ),
        );
    }
    write(
        &tasks.join("M-s5.md"),
        "---\nid: M-s5\ntitle: Release five\nphase: 5\nstatus: todo\n\
         blocked_by: [AM-072, AM-073]\ntags: [milestone, s5]\n---\n\n## Why\n\nBecause.\n",
    );
    let source_five = tasks.join("M-s5.md");
    let source_six = tasks.join("M-s6.md");
    let before_five = std::fs::read_to_string(&source_five).expect("s5");
    let before_six = std::fs::read_to_string(&source_six).expect("s6");

    let (preview, preview_code) = run_new_cut(
        dir.path(),
        &["--member", "AM-073", "--member", "AM-075", "--dry-run"],
    );
    assert_eq!(preview_code, 0, "{preview}");
    for expected in [
        "would add AM-073 to `M-sG`",
        "would add AM-075 to `M-sG`",
        "would remove AM-073 from `M-s5`",
        "would remove AM-075 from `M-s6`",
        "M-s5.md",
        "M-s6.md",
        "M-sG.md",
    ] {
        assert!(
            preview.contains(expected),
            "missing {expected:?}: {preview}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(&source_five).expect("s5 unchanged"),
        before_five
    );
    assert_eq!(
        std::fs::read_to_string(&source_six).expect("s6 unchanged"),
        before_six
    );
    assert!(!tasks.join("M-sG.md").exists());

    let (out, code) = run_new_cut(dir.path(), &["--member", "AM-073", "--member", "AM-075"]);
    assert_eq!(code, 0, "{out}");
    let (checked, check_code) = aplan(dir.path(), &["check"]);
    assert_eq!(check_code, 0, "{checked}");
}

#[test]
fn every_ambiguous_or_unsupported_member_shape_is_refused_before_writing() {
    struct Case {
        name: &'static str,
        prepare: fn(&std::path::Path),
        args: &'static [&'static str],
        says: &'static str,
    }
    fn unchanged(_: &std::path::Path) {}
    fn terminal(root: &std::path::Path) {
        let path = root.join("aplan/tasks/AM-075.md");
        let text = std::fs::read_to_string(&path).expect("member");
        write(&path, &text.replace("status: todo", "status: done"));
    }
    fn ambiguous_id(root: &std::path::Path) {
        write(
            &root.join("aplan/tasks/AM-075-copy.md"),
            "---\nid: AM-075\ntitle: Duplicate declaration\nphase: 6\nstatus: todo\n\
             effort_days: 1\npaths: [src/**]\n---\n",
        );
    }
    fn cross_cutting(root: &std::path::Path) {
        let path = root.join("aplan/tasks/AM-075.md");
        let text = std::fs::read_to_string(&path).expect("member");
        write(
            &path,
            &text.replace("paths: [src/**]", "paths: [src/**]\ntags: [cross-cutting]"),
        );
    }
    fn unowned(root: &std::path::Path) {
        write(
            &root.join("aplan/tasks/AM-076.md"),
            "---\nid: AM-076\ntitle: Unowned\nphase: 6\nstatus: todo\neffort_days: 1\n\
             paths: [src/**]\n---\n",
        );
    }
    fn multiply_owned(root: &std::path::Path) {
        write(
            &root.join("aplan/tasks/M-other.md"),
            "---\nid: M-other\ntitle: Other\nphase: 6\nstatus: todo\n\
             blocked_by: [AM-074, AM-075]\ntags: [milestone, other]\n---\n",
        );
    }
    fn empty_source(root: &std::path::Path) {
        let path = root.join("aplan/tasks/M-s6.md");
        let text = std::fs::read_to_string(&path).expect("source");
        write(&path, &text.replace("[AM-074, AM-075]", "[AM-075]"));
    }
    fn invalid_retained_cross_cutting(root: &std::path::Path) {
        let path = root.join("aplan/tasks/AM-074.md");
        let text = std::fs::read_to_string(&path).expect("retained member");
        write(
            &path,
            &text.replace("paths: [src/**]", "paths: [src/**]\ntags: [cross-cutting]"),
        );
    }
    fn id_collision(root: &std::path::Path) {
        write(
            &root.join("aplan/tasks/M-sG.md"),
            "---\nid: M-sG\ntitle: Existing\nphase: 7\nstatus: todo\n\
             blocked_by: [AM-074]\ntags: [milestone, existing]\n---\n",
        );
    }

    let empty = amorph_plan();
    let plan = aplan_core::load_plan(&empty.path().join("aplan"));
    let no_members = aplan_interop::cut::plan_new_cut(&plan, &[])
        .expect_err("the shared planner must not prepare an empty move");
    assert!(
        no_members.to_string().contains("at least one `member`"),
        "{no_members}"
    );

    let cases = [
        Case {
            name: "unknown",
            prepare: unchanged,
            args: &["--member", "MISSING"],
            says: "no task `MISSING`",
        },
        Case {
            name: "duplicate",
            prepare: unchanged,
            args: &["--member", "AM-075", "--member", "AM-075"],
            says: "named more than once",
        },
        Case {
            name: "ambiguous duplicate id",
            prepare: ambiguous_id,
            args: &["--member", "AM-075"],
            says: "appears in more than one file",
        },
        Case {
            name: "terminal",
            prepare: terminal,
            args: &["--member", "AM-075"],
            says: "already `done`",
        },
        Case {
            name: "milestone",
            prepare: unchanged,
            args: &["--member", "M-s6"],
            says: "is a milestone, not work",
        },
        Case {
            name: "cross-cutting",
            prepare: cross_cutting,
            args: &["--member", "AM-075"],
            says: "tagged `cross-cutting`",
        },
        Case {
            name: "unowned",
            prepare: unowned,
            args: &["--member", "AM-076"],
            says: "belongs to no cut",
        },
        Case {
            name: "multiply owned",
            prepare: multiply_owned,
            args: &["--member", "AM-075"],
            says: "belongs to multiple cuts",
        },
        Case {
            name: "emptied source",
            prepare: empty_source,
            args: &["--member", "AM-075"],
            says: "leave source cut `M-s6` with no live members",
        },
        Case {
            name: "cross-cutting is not retained cut work",
            prepare: invalid_retained_cross_cutting,
            args: &["--member", "AM-075"],
            says: "leave source cut `M-s6` with no live members",
        },
        Case {
            name: "id collision",
            prepare: id_collision,
            args: &["--member", "AM-075"],
            says: "already exists",
        },
        Case {
            name: "work in sequencing field",
            prepare: unchanged,
            args: &["--blocked-by", "AM-074", "--member", "AM-075"],
            says: "reserve `--blocked-by` for milestone sequencing",
        },
        Case {
            name: "invalid milestone metadata",
            prepare: unchanged,
            args: &["--effort", "1", "--member", "AM-075"],
            says: "APLAN021",
        },
    ];

    for case in cases {
        let dir = amorph_plan();
        (case.prepare)(dir.path());
        let source = dir.path().join("aplan/tasks/M-s6.md");
        let before = std::fs::read_to_string(&source).expect("source before");
        let (out, code) = run_new_cut(dir.path(), case.args);
        assert_ne!(code, 0, "{} unexpectedly succeeded: {out}", case.name);
        assert!(
            out.contains(case.says),
            "{} did not say {:?}: {out}",
            case.name,
            case.says
        );
        assert_eq!(
            std::fs::read_to_string(&source).expect("source after"),
            before,
            "{} changed its source despite refusal",
            case.name
        );
        if case.name != "id collision" {
            assert!(
                !dir.path().join("aplan/tasks/M-sG.md").exists(),
                "{} created the destination despite refusal",
                case.name
            );
        }
    }

    let dir = amorph_plan();
    let (without_tag, code) = aplan(
        dir.path(),
        &[
            "add",
            "Not declared a cut",
            "--id",
            "M-sG",
            "--phase",
            "7",
            "--member",
            "AM-075",
        ],
    );
    assert_ne!(code, 0, "{without_tag}");
    assert!(
        without_tag.contains("must carry `--tag milestone`"),
        "{without_tag}"
    );

    let (inferred_identity, code) = aplan(
        dir.path(),
        &[
            "add",
            "Invented cut identity",
            "--tag",
            "milestone",
            "--member",
            "AM-075",
        ],
    );
    assert_ne!(code, 0, "{inferred_identity}");
    assert!(
        inferred_identity.contains("supply both `--id` and `--phase`"),
        "{inferred_identity}"
    );

    let (nested, code) = run_new_cut(dir.path(), &["--cut", "M-s6", "--member", "AM-075"]);
    assert_ne!(code, 0, "{nested}");
    assert!(nested.contains("cannot be used with"), "{nested}");
}

#[test]
fn a_late_invalid_source_and_a_locked_batch_leave_every_candidate_unchanged() {
    let dir = amorph_plan();
    let tasks = dir.path().join("aplan/tasks");
    for id in ["AM-072", "AM-073"] {
        write(
            &tasks.join(format!("{id}.md")),
            &format!(
                "---\nid: {id}\ntitle: {id}\nphase: 5\nstatus: todo\neffort_days: 1\n\
                 paths: [src/**]\n---\n"
            ),
        );
    }
    let source_five = tasks.join("M-s5.md");
    write(
        &source_five,
        "---\nid: M-s5\ntitle: Release five\nphase: 5\nstatus: todo\n\
         blocked_by: [AM-072, AM-073]\ntags: [milestone, s5]\n---\n",
    );
    let source_six = tasks.join("M-s6.md");
    let normal_six = std::fs::read_to_string(&source_six).expect("s6 normal");
    write(
        &source_six,
        &normal_six.replace(
            "blocked_by: [AM-074, AM-075]",
            "blocked_by: [\n  AM-074,\n  AM-075\n]",
        ),
    );
    let before_five = std::fs::read_to_string(&source_five).expect("s5 before");
    let before_six = std::fs::read_to_string(&source_six).expect("s6 before");
    let (invalid, code) = run_new_cut(dir.path(), &["--member", "AM-075", "--member", "AM-073"]);
    assert_ne!(code, 0, "{invalid}");
    assert!(invalid.contains("could not edit `blocked_by`"), "{invalid}");
    assert_eq!(std::fs::read_to_string(&source_five).unwrap(), before_five);
    assert_eq!(std::fs::read_to_string(&source_six).unwrap(), before_six);
    assert!(!tasks.join("M-sG.md").exists());

    write(&source_six, &normal_six);
    let before_five = std::fs::read_to_string(&source_five).expect("s5 lock baseline");
    let before_six = std::fs::read_to_string(&source_six).expect("s6 lock baseline");
    let lock = tasks.join(".M-s5.md.lock");
    write(&lock, "held");
    let (locked, code) = run_new_cut(dir.path(), &["--member", "AM-075", "--member", "AM-073"]);
    assert_ne!(code, 0, "{locked}");
    assert!(locked.contains("nothing was written"), "{locked}");
    assert_eq!(std::fs::read_to_string(&source_five).unwrap(), before_five);
    assert_eq!(std::fs::read_to_string(&source_six).unwrap(), before_six);
    assert!(!tasks.join("M-sG.md").exists());

    std::fs::remove_file(&lock).expect("unlock");
    let original_mode = std::fs::metadata(&source_five)
        .expect("source metadata")
        .permissions()
        .mode();
    std::fs::set_permissions(&source_five, std::fs::Permissions::from_mode(0o400))
        .expect("make source read-only");
    let (unwritable, code) = run_new_cut(dir.path(), &["--member", "AM-075", "--member", "AM-073"]);
    std::fs::set_permissions(&source_five, std::fs::Permissions::from_mode(original_mode))
        .expect("restore source mode");
    assert_ne!(code, 0, "{unwritable}");
    assert!(unwritable.contains("Permission denied"), "{unwritable}");
    assert_eq!(std::fs::read_to_string(&source_five).unwrap(), before_five);
    assert_eq!(std::fs::read_to_string(&source_six).unwrap(), before_six);
    assert!(!tasks.join("M-sG.md").exists());
}
