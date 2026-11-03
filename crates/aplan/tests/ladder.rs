//! `verify` is a superset of `check`, not a sibling of it (`A1-058`).
//!
//! The bug this pins: `verify` resolved evidence, skipped every hermetic rule,
//! and printed `ok: N tasks, 0 warning(s)` on a plan `check` rejected. That is a
//! claim about the subset of rules it ran, phrased as a claim about the plan, and
//! an adopter wiring one command into CI reaches for this one because it sounds
//! like the deeper check. The `p00025` pilot hit it while probing `APLAN023`,
//! which looked unimplemented because the probe ran under `verify`.
//!
//! Driven through the binary rather than the library, because the defect lived in
//! the command's wiring: every layer underneath was correct.

use std::path::Path;
use std::process::Command;

use crate::common;
use common::aplan;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("running git")
        .status
        .success();
    assert!(ok, "git {args:?} failed");
}

/// A repository with one commit, and a plan directory.
fn repo() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(dir.path().join("f.txt"), "x").expect("file");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "t@t"]);
    git(dir.path(), &["config", "user.name", "t"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "init"]);
    dir
}

fn head(dir: &Path) -> String {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .expect("rev-parse");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Exercise the shared mutation post-state resolver separately from `close`'s
/// citation preflight (`A1-572`).
fn assert_amend_refuses_unreachable(dir: &Path, unreachable: &str) {
    let reachable_task = dir.join("aplan/tasks/T-301.md");
    let before = std::fs::read_to_string(&reachable_task).expect("reachable task");
    let (amended, code) = aplan(
        dir,
        &[
            "amend",
            "T-301",
            "--add-evidence",
            &format!("commit:{unreachable}"),
        ],
    );
    assert_eq!(code, 1, "amend wrote locally-only evidence:\n{amended}");
    assert!(
        amended.contains("APLAN011") && amended.contains("T-301"),
        "{amended}"
    );
    assert_eq!(
        std::fs::read_to_string(reachable_task).expect("unchanged reachable task"),
        before,
        "the refused post-state mutation wrote anyway"
    );
}

/// A `done` task citing a real commit but declaring no `paths`: `APLAN023`
/// warns, and every evidence rule is satisfied, so the finding exists only on
/// the hermetic side. This is the exact shape the pilot reported.
fn hermetic_only_finding(dir: &Path) {
    let sha = head(dir);
    std::fs::write(
        dir.join("aplan/tasks/T-001.md"),
        format!(
            "---\nid: T-001\ntitle: cites a commit with no paths\nphase: 1\nstatus: done\n\
             evidence:\n  - kind: commit\n    ref: \"{sha}\"\n---\n"
        ),
    )
    .expect("write task");
}

#[test]
fn verify_reports_a_finding_that_only_check_produces() {
    let dir = repo();
    hermetic_only_finding(dir.path());

    let (check, _) = aplan(dir.path(), &["check"]);
    assert!(
        check.contains("APLAN023"),
        "the setup must be hermetic-only: {check}"
    );

    let (verify, code) = aplan(dir.path(), &["verify"]);
    assert!(
        verify.contains("APLAN023"),
        "verify hid a finding check reports: {verify}"
    );
    assert!(
        !verify.contains("0 warning(s)"),
        "verify counted warnings from the subset of rules it ran: {verify}"
    );
    // A warning is still advisory: the exit code is unchanged by this task.
    assert_eq!(code, 0, "{verify}");
}

#[test]
fn the_two_commands_agree_on_whether_a_plan_passes() {
    // The failure mode was a permanently green CI on a plan the other command
    // rejects, so the property worth pinning is agreement, not wording.
    let dir = repo();
    hermetic_only_finding(dir.path());
    std::fs::write(dir.path().join(".aplan.yml"), "deny_warnings: true\n").expect("config");

    let (_, check_code) = aplan(dir.path(), &["check"]);
    let (verify_out, verify_code) = aplan(dir.path(), &["verify"]);
    assert_eq!(check_code, 1, "the plan must fail under check");
    assert_eq!(
        verify_code, check_code,
        "verify disagreed with check about the same plan: {verify_out}"
    );
}

#[test]
fn the_hermetic_flags_move_both_commands_together() {
    // Containment has to survive the flags, or it is only true of the default
    // invocation. `check --no-traces` passing while `verify` failed on the trace
    // it was told to ignore is the same false-reassurance defect wearing a flag,
    // and it is what this looked like before `verify` carried these.
    let dir = repo();
    std::fs::write(
        dir.path().join("aplan/tasks/T-004.md"),
        "---\nid: T-004\ntitle: traces a doc that is not there\nphase: 1\nstatus: todo\n\
         effort_days: 1\ntraces:\n  - MISSING.md#x\n---\n",
    )
    .expect("write task");

    for flags in [
        vec![],
        vec!["--no-traces"],
        vec!["--no-require-effort"],
        vec!["--no-traces", "--no-require-effort"],
    ] {
        let mut check_args = vec!["check"];
        check_args.extend(flags.iter());
        let mut verify_args = vec!["verify"];
        verify_args.extend(flags.iter());

        let (check_out, check_code) = aplan(dir.path(), &check_args);
        let (verify_out, verify_code) = aplan(dir.path(), &verify_args);
        assert_eq!(
            verify_code, check_code,
            "with {flags:?} the commands disagreed\ncheck: {check_out}\nverify: {verify_out}"
        );
    }
}

#[test]
fn an_error_only_check_produces_is_not_hidden_by_verify() {
    // `APLAN005`, the rule this whole project is named for: a `done` task with no
    // evidence at all. Evidence resolution has nothing to resolve, so before this
    // `verify` called such a plan clean.
    let dir = repo();
    std::fs::write(
        dir.path().join("aplan/tasks/T-002.md"),
        "---\nid: T-002\ntitle: claims done with nothing\nphase: 1\nstatus: done\n---\n",
    )
    .expect("write task");

    let (out, code) = aplan(dir.path(), &["verify"]);
    assert!(out.contains("APLAN005"), "{out}");
    assert_eq!(code, 1, "an error must fail verify: {out}");
}

#[test]
fn evidence_rules_still_run_and_are_reported_together() {
    // The superset must be a superset: adding the hermetic rules must not have
    // displaced the resolution this command exists for.
    let dir = repo();
    std::fs::write(
        dir.path().join("aplan/tasks/T-003.md"),
        "---\nid: T-003\ntitle: cites a commit that does not exist\nphase: 1\nstatus: done\n\
         paths: [f.txt]\nevidence:\n  - kind: commit\n    ref: 0123456789abcdef0123456789abcdef01234567\n---\n",
    )
    .expect("write task");

    let (out, code) = aplan(dir.path(), &["verify"]);
    assert!(
        out.contains("APLAN011"),
        "the evidence rule must still fire: {out}"
    );
    assert_eq!(code, 1, "{out}");

    // And `check` alone cannot see it, which is what keeps both commands useful.
    let (check, _) = aplan(dir.path(), &["check"]);
    assert!(!check.contains("APLAN011"), "check is hermetic: {check}");
}

/// Proves `evidence.commit_requires_head_reachability` end to end (`A1-572`):
/// an ordinary ancestor is reproducible from the verified tree, while neither
/// an unmerged branch nor a dangling object left by a removed linked worktree
/// is. `close` goes through the same post-state resolver and must not write the
/// latter citation.
#[test]
fn only_commits_reachable_from_head_can_be_commit_evidence() {
    let dir = repo();
    let reachable = head(dir.path());
    let branch = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(dir.path())
        .output()
        .expect("current branch");
    let branch = String::from_utf8_lossy(&branch.stdout).trim().to_string();

    git(dir.path(), &["checkout", "-qb", "unmerged"]);
    std::fs::write(dir.path().join("f.txt"), "unmerged\n").expect("side work");
    git(dir.path(), &["add", "f.txt"]);
    git(dir.path(), &["commit", "-qm", "unmerged work"]);
    let unmerged = head(dir.path());
    git(dir.path(), &["checkout", "-q", &branch]);

    let linked_parent = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("linked worktree parent");
    let linked = linked_parent.path().join("worktree");
    let linked_arg = linked.to_str().expect("UTF-8 fixture path");
    git(
        dir.path(),
        &["worktree", "add", "-qb", "abandoned", linked_arg, "HEAD"],
    );
    std::fs::write(linked.join("f.txt"), "abandoned\n").expect("worktree work");
    git(&linked, &["add", "f.txt"]);
    git(&linked, &["commit", "-qm", "abandoned worktree work"]);
    let abandoned = head(&linked);
    git(dir.path(), &["worktree", "remove", "--force", linked_arg]);
    git(dir.path(), &["branch", "-D", "abandoned"]);

    for (id, reference) in [
        ("T-301", &reachable),
        ("T-302", &unmerged),
        ("T-303", &abandoned),
    ] {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: reachability fixture\nphase: 1\nstatus: done\n\
                 paths: [f.txt]\nevidence:\n  - kind: commit\n    ref: '{reference}'\n---\n"
            ),
        )
        .expect("task");
    }
    std::fs::write(
        dir.path().join("aplan/tasks/T-304.md"),
        "---\nid: T-304\ntitle: mutation fixture\nphase: 1\nstatus: todo\n\
         effort_days: 1\npaths: [f.txt]\n---\n",
    )
    .expect("todo task");
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &["commit", "-qm", "record reachability fixtures"],
    );

    let (out, code) = aplan(dir.path(), &["verify"]);
    assert_eq!(code, 1, "locally-only citations must fail:\n{out}");
    assert_eq!(
        out.matches("APLAN011").count(),
        2,
        "only the unmerged and abandoned citations fail:\n{out}"
    );
    assert!(out.contains("T-302") && out.contains("T-303"), "{out}");
    assert!(
        !out.lines()
            .any(|line| line.contains("APLAN011") && line.contains("T-301")),
        "the reachable ancestor was rejected:\n{out}"
    );
    assert!(
        out.contains("resolves locally") && out.contains("not reachable"),
        "the diagnostic conflated local reachability with absence:\n{out}"
    );

    let (closed, code) = aplan(
        dir.path(),
        &[
            "close",
            "T-304",
            "--evidence",
            &format!("commit:{abandoned}"),
        ],
    );
    assert_eq!(code, 1, "close wrote locally-only evidence:\n{closed}");
    assert!(
        closed.contains("APLAN011") && closed.contains("T-304"),
        "{closed}"
    );
    let task =
        std::fs::read_to_string(dir.path().join("aplan/tasks/T-304.md")).expect("unchanged task");
    assert!(
        task.contains("status: todo"),
        "the refused close wrote status: {task}"
    );
    assert!(
        !task.contains(&abandoned),
        "the refused close wrote evidence: {task}"
    );

    assert_amend_refuses_unreachable(dir.path(), &abandoned);
}

// --- one task's defect must not silence the plan (`A1-066`) ------------------

/// The exact plan from `A1-066`: one task with a `paths` glob that will not
/// compile, and one `done` task with no evidence at all.
fn a_bad_glob_beside_a_real_finding(dir: &Path) {
    std::fs::write(
        dir.join("aplan/tasks/T-001.md"),
        "---\nid: T-001\ntitle: has an unusable glob\nphase: 1\nstatus: done\n\
         paths:\n  - 'src/['\nevidence:\n  - kind: note\n    ref: x\n    captured: 2026-01-01\n---\n",
    )
    .expect("write task");
    std::fs::write(
        dir.join("aplan/tasks/T-002.md"),
        "---\nid: T-002\ntitle: done with nothing to show\nphase: 1\nstatus: done\n---\n",
    )
    .expect("write task");
}

/// The reported defect, end to end: `verify` used to abort on `T-001`'s typo and
/// report nothing about `T-002`, exiting `2` while `check` exited `1` with a
/// finding. That is `A1-058`'s containment broken by a task file.
/// Proves the named behaviour `verify.one_tasks_defect_does_not_stop_the_run` (`A1-211`).
#[test]
fn a_tasks_bad_glob_does_not_hide_another_tasks_finding() {
    let dir = repo();
    a_bad_glob_beside_a_real_finding(dir.path());

    let (check, check_code) = aplan(dir.path(), &["check"]);
    let (verify, verify_code) = aplan(dir.path(), &["verify"]);

    for (what, out) in [("check", &check), ("verify", &verify)] {
        assert!(
            out.contains("APLAN005") && out.contains("T-002"),
            "{what} lost the other task's finding:\n{out}"
        );
        assert!(
            out.contains("APLAN032") && out.contains("src/["),
            "{what} does not name the unusable glob:\n{out}"
        );
    }
    // ADR-0007: a finding is `1`. `2` is the invocation or the config, and this
    // defect is in a task file.
    assert_eq!(check_code, 1, "{check}");
    assert_eq!(
        verify_code, check_code,
        "the two commands disagree about the same plan:\n{verify}"
    );
}

/// A runner that cannot run is the same shape one layer out: it used to abort
/// the run and take every unrelated finding with it.
#[test]
fn a_runner_that_cannot_run_is_reported_and_the_plan_is_still_judged() {
    let dir = repo();
    std::fs::write(
        dir.path().join("aplan/tasks/T-002.md"),
        "---\nid: T-002\ntitle: done with nothing to show\nphase: 1\nstatus: done\n---\n",
    )
    .expect("write task");
    std::fs::write(
        dir.path().join("aplan/tasks/T-003.md"),
        "---\nid: T-003\ntitle: cites a test\nphase: 1\nstatus: done\npaths:\n  - 'f.txt'\n\
         evidence:\n  - kind: test\n    ref: a_test\n---\n",
    )
    .expect("write task");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "test:\n  dir: not-here\n  list: [\"printf\", \"%s\\n\", \"a_test\"]\n  \
         run: [\"true\", \"{test}\"]\n",
    )
    .expect("config");

    let (out, code) = aplan(dir.path(), &["verify"]);
    assert_eq!(code, 1, "a finding, not a configuration abort:\n{out}");
    assert!(
        out.contains("APLAN013") && out.contains("could not be asked"),
        "the broken runner is not reported:\n{out}"
    );
    assert!(
        out.contains("APLAN005") && out.contains("T-002"),
        "the unrelated finding was lost:\n{out}"
    );
    // The citation that depended on the runner is not blamed for it, and the
    // message stays on one line however the command is spelled.
    assert!(
        !out.contains("is not one the configured runner lists"),
        "a citation was reported as a typo because its runner is broken:\n{out}"
    );
    let runner_line = out
        .lines()
        .find(|l| l.contains("could not be asked"))
        .expect("the runner finding");
    assert!(
        runner_line.contains("\\n"),
        "a newline inside the command must be escaped, not printed: {runner_line}"
    );
}

/// A merge citation blocks a close under `deny_warnings` (`A1-145`, audit).
///
/// `close` refuses exactly what `verify` would fail, so a repository whose
/// policy is that every warning blocks cannot close a task on a merge without
/// changing the citation or tolerating the code. That is a consequence adopters
/// meet rather than read, so it is pinned here.
#[test]
fn a_merge_citation_blocks_a_close_where_every_warning_blocks() {
    let dir = repo();
    let root = dir.path();
    std::fs::write(root.join(".aplan.yml"), "deny_warnings: true\n").expect("config");
    std::fs::create_dir_all(root.join("src")).expect("src");

    // A branch with the work, a main-line commit, and a merge joining them.
    git(root, &["checkout", "-q", "-b", "side"]);
    std::fs::write(root.join("src/a.rs"), "work").expect("work");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "the work"]);
    git(root, &["checkout", "-q", "-"]);
    std::fs::write(root.join("other.txt"), "x").expect("other");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "other"]);
    git(root, &["merge", "--no-ff", "-q", "side", "-m", "merge"]);
    let merge = head(root);

    std::fs::write(
        root.join("aplan/tasks/T-009.md"),
        "---\nid: T-009\ntitle: Owns src\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths:\n- src/**\n---\n\n## Why\n\nx\n",
    )
    .expect("task");

    let (out, code) = aplan(
        root,
        &["close", "T-009", "--evidence", &format!("commit:{merge}")],
    );
    assert_ne!(code, 0, "the close was accepted on a merge:\n{out}");
    assert!(out.contains("APLAN035"), "{out}");
    assert!(
        std::fs::read_to_string(root.join("aplan/tasks/T-009.md"))
            .expect("read")
            .contains("status: todo"),
        "the refusal wrote the closure anyway"
    );
}

/// A shallow clone is reported once, against the checkout, and no citation is
/// blamed for it (`A1-177`).
///
/// `actions/checkout` defaults to `fetch-depth: 1`. Every `commit` citation then
/// fails to resolve and the pre-`A1-177` message told the reader to "cite a
/// commit SHA that exists, or remove the evidence" -- advice that deletes correct
/// proof. Measured against this repository's own plan in a depth-1 clone: 164
/// findings across 187 tasks, every one false.
#[test]
fn a_shallow_clone_is_one_finding_about_the_checkout_not_one_per_task() {
    let dir = repo();
    let sha = head(dir.path());
    for id in ["T-101", "T-102", "T-103"] {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: proved by a commit\nphase: 1\nstatus: done\n\
                 paths:\n  - 'f.txt'\nevidence:\n  - kind: commit\n    ref: \"{sha}\"\n---\n"
            ),
        )
        .expect("write task");
    }
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "plan"]);

    // A full clone resolves all three.
    let (out, code) = aplan(dir.path(), &["verify"]);
    assert_eq!(code, 0, "{out}");

    // The same plan, cloned to depth 1.
    let shallow = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let target = shallow.path().join("copy");
    let ok = Command::new("git")
        .args(["clone", "-q", "--depth", "1"])
        .arg(format!("file://{}", dir.path().display()))
        .arg(&target)
        .output()
        .expect("git clone")
        .status
        .success();
    assert!(ok, "the shallow clone failed");

    let (out, code) = aplan(&target, &["verify"]);
    assert_eq!(code, 1, "{out}");
    assert_eq!(
        out.matches("APLAN011").count(),
        1,
        "one finding, not one per task:\n{out}"
    );
    assert!(out.contains("shallow clone"), "{out}");
    assert!(
        !out.contains("remove the evidence"),
        "the fix must not tell a reader to delete correct citations:\n{out}"
    );
    for id in ["T-101", "T-102", "T-103"] {
        assert!(
            !out.contains(id),
            "`{id}` was blamed for the checkout:\n{out}"
        );
    }
}

/// A citation that resolves to the clone's boundary is left unjudged, not passed
/// (`A1-177`, found by its audit).
///
/// The sharper half of the shallow-clone problem, and the one the first fix
/// missed. A boundary commit's parent is absent, so `commit_paths` diffed it
/// against the empty tree -- the rule for a *root* commit -- which made it touch
/// every file in the repository. `APLAN012` therefore passed for any `paths` at
/// all: the same citation errored in a full clone and reported `ok` in a shallow
/// one.
#[test]
fn a_citation_on_the_shallow_boundary_is_not_judged_against_the_whole_tree() {
    let dir = repo();
    std::fs::write(dir.path().join("other.txt"), "y").expect("file");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "second"]);
    // A task whose `paths` the *tip* does not touch: the tip adds the task file.
    std::fs::write(
        dir.path().join("aplan/tasks/T-201.md"),
        "---\nid: T-201\ntitle: cites the tip\nphase: 1\nstatus: done\n\
         paths:\n  - 'f.txt'\nevidence:\n  - kind: commit\n    ref: \"PLACEHOLDER\"\n---\n",
    )
    .expect("write task");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "plan"]);
    let tip = head(dir.path());
    let task = dir.path().join("aplan/tasks/T-201.md");
    let body = std::fs::read_to_string(&task).expect("read");
    std::fs::write(&task, body.replace("PLACEHOLDER", &tip)).expect("write");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "cite"]);

    // In a full clone the citation is judged and rejected: the tip touched the
    // task file, not `f.txt`.
    let (out, code) = aplan(dir.path(), &["verify"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("APLAN012"), "{out}");

    // Cloned to depth 2, the cited commit is present and its parent is not.
    let shallow = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let target = shallow.path().join("copy");
    let ok = Command::new("git")
        .args(["clone", "-q", "--depth", "2"])
        .arg(format!("file://{}", dir.path().display()))
        .arg(&target)
        .output()
        .expect("git clone")
        .status
        .success();
    assert!(ok, "the shallow clone failed");

    let (out, code) = aplan(&target, &["verify"]);
    assert_eq!(code, 1, "a boundary citation must not report ok:\n{out}");
    assert!(
        out.contains("boundary"),
        "the finding must name the checkout, not the task:\n{out}"
    );
    assert!(
        !out.contains("APLAN012"),
        "`APLAN012` cannot be judged here and must not be claimed either way:\n{out}"
    );
}
