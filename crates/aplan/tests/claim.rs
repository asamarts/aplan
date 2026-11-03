//! Claiming, releasing, starting and stopping (`A1-359`, `A1-531`).
//!
//! Three concepts, deliberately not one. `owner` is durable responsibility,
//! the Git-common claim record answers "who holds this, now", and
//! `in_progress` is the author's claim that work has begun,
//! which is stored and then checked against the repository rather than taken on
//! trust -- the same pattern `done` already gets from `verify`, applied to
//! another status.
//!
//! **Why these commands exist at all**, given that `A1-077` refused a
//! `status <id> <status>` setter as "write surface bought with no invariant to
//! maintain". Two answers, and they are different answers. For `claim` the
//! invariant is *mutual exclusion*: two agents both read "unassigned", both
//! write their name, last writer wins, and `check` sees a perfectly valid plan
//! while two actors do the same work. Hand-editing cannot maintain that, and
//! the compare-and-swap writer can, which is exactly the bar `A1-077` set. For
//! `start`/`stop` the answer is that `A1-077`'s test assumed an actor with an
//! editor: an agent reaching aplan over MCP has tools and no way to edit a file,
//! so "hand-editing is fine as it is" is false for it.

use std::path::Path;

use crate::common;

/// A plan with one unheld live task, one held live task, and one closed task.
fn fixture(dir: &Path) {
    git2::Repository::init(dir).expect("git init");
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    for (id, status, extra) in [
        ("T-free", "todo", ""),
        ("T-held", "todo", "owner: ada\n"),
        (
            "T-done",
            "done",
            "owner: ada\nevidence:\n  - kind: note\n    ref: shipped\n    captured: 2026-10-01\n",
        ),
    ] {
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: task {id}\nphase: 0\nstatus: {status}\n\
                 effort_days: 1\n{extra}---\n\n## Why\n\nBecause.\n"
            ),
        )
        .expect("task");
    }
    let claimed = common::run(dir, &["claim", "T-held", "--as", "ada"]);
    assert_eq!(claimed.code, 0, "fixture claim failed: {}", claimed.text());
}

/// The `owner:` line of a task, or `None`.
fn owner_of(dir: &Path, id: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(format!("aplan/tasks/{id}.md"))).expect("read");
    text.lines()
        .find(|l| l.starts_with("owner:"))
        .map(|l| l.trim_start_matches("owner:").trim().to_string())
}

/// The ephemeral holder in the Git-common claim store, or `None`.
fn claim_holder(dir: &Path, id: &str) -> Option<String> {
    let state = dir.join(".git/aplan/claims-v1.json");
    let text = match std::fs::read_to_string(state) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => panic!("read claim state: {error}"),
    };
    let value: serde_json::Value = serde_json::from_str(&text).expect("claim JSON");
    value["claims"][id]["holder"].as_str().map(str::to_string)
}

/// The `status:` line of a task.
fn status_of(dir: &Path, id: &str) -> String {
    let text = std::fs::read_to_string(dir.join(format!("aplan/tasks/{id}.md"))).expect("read");
    text.lines()
        .find(|l| l.starts_with("status:"))
        .map(|l| l.trim_start_matches("status:").trim().to_string())
        .expect("a task has a status")
}

fn tmp(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir")
}

fn git(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// **Exactly one of eight simultaneous claimants wins** (`A1-359`).
///
/// The invariant the whole write surface is justified by, and the one
/// hand-editing cannot maintain. Each claimant reads the task, sees it
/// unassigned, and writes its own name; without a compare-and-swap all eight
/// would report success and the file would hold whichever wrote last, with
/// `check` reporting a perfectly valid plan over seven actors who each believe
/// they hold the task.
///
/// Asserted as "exactly one", not "the first one": which claimant wins is a
/// race and is not the question. What must hold is that the number of callers
/// told they hold it equals the number of callers who do.
#[test]
fn exactly_one_of_eight_simultaneous_claimants_is_told_it_holds_the_task() {
    let dir = tmp("claim-race");
    let dir = dir.path();
    fixture(dir);

    let handles: Vec<_> = (0..8)
        .map(|i| {
            let dir = dir.to_path_buf();
            std::thread::spawn(move || {
                let name = format!("agent{i}");
                let out = common::run(&dir, &["claim", "T-free", "--as", &name]);
                (out.code == 0).then_some(name)
            })
        })
        .collect();
    let winners: Vec<String> = handles
        .into_iter()
        .filter_map(|h| h.join().expect("claimant thread"))
        .collect();

    assert_eq!(
        winners.len(),
        1,
        "{} claimants were told they hold `T-free`: {winners:?}",
        winners.len()
    );
    assert_eq!(
        claim_holder(dir, "T-free").as_deref(),
        Some(winners[0].as_str()),
        "the task is not held by the caller that was told it holds it"
    );
    let (said, code) = common::aplan(dir, &["check"]);
    assert_eq!(code, 0, "the plan does not load after the race:\n{said}");
}

/// Proof for `claim.uses_worktree_shared_ephemeral_state` and
/// `claim.warns_on_cross_origin_identity_collision` (`A1-531`).
#[test]
fn exactly_one_cross_worktree_claimant_is_told_it_won() {
    let dir = tmp("claim-linked");
    fixture(dir.path());
    git(dir.path(), &["config", "user.name", "Claim Test"]);
    git(
        dir.path(),
        &["config", "user.email", "claim@example.invalid"],
    );
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-m", "fixture"]);

    let peers = tmp("claim-linked-peers");
    let peer = peers.path().join("peer");
    git(
        dir.path(),
        &[
            "worktree",
            "add",
            "-b",
            "claim-peer",
            peer.to_str().expect("UTF-8 path"),
        ],
    );
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = [
        (dir.path().to_path_buf(), "primary"),
        (peer.clone(), "peer"),
    ]
    .into_iter()
    .map(|(root, holder)| {
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            let out = common::run(&root, &["claim", "T-free", "--as", holder]);
            (out.code == 0).then_some(holder)
        })
    })
    .collect();
    let winners: Vec<_> = handles
        .into_iter()
        .filter_map(|handle| handle.join().expect("claim thread"))
        .collect();
    assert_eq!(winners.len(), 1, "cross-worktree winners: {winners:?}");
    assert_eq!(
        claim_holder(dir.path(), "T-free").as_deref(),
        Some(winners[0])
    );

    let released = common::run(dir.path(), &["release", "T-free"]);
    assert_eq!(released.code, 0, "{}", released.text());
    let first = common::run(dir.path(), &["claim", "T-free", "--as", "shared"]);
    assert_eq!(first.code, 0, "{}", first.text());
    let collision = common::run(
        &peer,
        &["claim", "T-free", "--as", "shared", "--from", "shared"],
    );
    assert_eq!(collision.code, 0, "{}", collision.text());
    assert!(
        collision.text().contains("already holds claim(s)")
            && collision.text().contains("distinct"),
        "the same label moving between worktree origins was not explained: {}",
        collision.text()
    );
}

/// Proof for `ready.exposes_ephemeral_claims`, `show.exposes_ephemeral_claims`,
/// `stale.exposes_ephemeral_claims`, `tasks.exposes_ephemeral_claims`,
/// `selection.unclaimed_uses_ephemeral_claims` and
/// `owner.remains_stable_responsibility` (`A1-531`).
#[test]
fn read_surfaces_share_ephemeral_claims_and_keep_owner_stable() {
    let dir = tmp("claim-reads");
    fixture(dir.path());
    let before = std::fs::read(dir.path().join("aplan/tasks/T-free.md")).expect("task bytes");
    let claimed = common::run(dir.path(), &["claim", "T-free", "--as", "grace"]);
    assert_eq!(claimed.code, 0, "{}", claimed.text());
    assert_eq!(
        std::fs::read(dir.path().join("aplan/tasks/T-free.md")).expect("task bytes"),
        before,
        "claim changed durable task bytes"
    );

    let json = |args: &[&str]| {
        let output = common::output(dir.path(), args);
        assert!(
            output.status.success(),
            "{}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).expect("JSON output")
    };
    let ready = json(&["ready", "--format", "json"]);
    assert_eq!(ready["schema"], "aplan/ready/v2");
    assert_eq!(ready["coordination"]["kind"], "git_common_directory");
    assert!(ready["claim_observed_at_epoch_seconds"].as_i64().is_some());
    let ready_row = ready["ready"]
        .as_array()
        .expect("ready rows")
        .iter()
        .find(|row| row["id"] == "T-free")
        .expect("T-free ready");
    assert_eq!(ready_row["claim"]["holder"], "grace");
    assert_eq!(ready_row["held_by"], "grace");
    assert!(ready_row["owner"].is_null());

    let shown = json(&["show", "T-free", "--no-resolve", "--format", "json"]);
    assert_eq!(shown["schema"], "aplan/show/v3");
    assert_eq!(shown["owner"], serde_json::Value::Null);
    assert_eq!(shown["claim"]["holder"], "grace");
    assert!(shown["claim_age_seconds"].as_i64().is_some());

    let tasks = json(&["tasks", "--format", "json"]);
    assert_eq!(tasks["schema"], "aplan/tasks/v2");
    let task_row = tasks["tasks"]
        .as_array()
        .expect("task rows")
        .iter()
        .find(|row| row["id"] == "T-free")
        .expect("T-free task row");
    assert_eq!(task_row["claim"]["holder"], "grace");

    let released = common::run(dir.path(), &["release", "T-held"]);
    assert_eq!(released.code, 0, "{}", released.text());
    let unclaimed = json(&["ready", "--unclaimed", "--format", "json"]);
    let stable_owner = unclaimed["ready"]
        .as_array()
        .expect("ready rows")
        .iter()
        .find(|row| row["id"] == "T-held")
        .expect("a stable owner does not imply an active claim");
    assert!(stable_owner["claim"].is_null());
    assert_eq!(stable_owner["owner"], "ada");

    git(dir.path(), &["config", "user.name", "Claim Test"]);
    git(
        dir.path(),
        &["config", "user.email", "claim@example.invalid"],
    );
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-m", "fixture"]);
    let stale = json(&[
        "stale",
        "--older-than",
        "0",
        "--as-of",
        "2099-01-01",
        "--format",
        "json",
    ]);
    assert_eq!(stale["schema"], "aplan/stale/v2");
    let stale_row = stale["tasks"]
        .as_array()
        .expect("stale rows")
        .iter()
        .find(|row| row["id"] == "T-free")
        .expect("T-free stale row");
    assert_eq!(stale_row["claim"]["holder"], "grace");
    let stable_owner = stale["tasks"]
        .as_array()
        .expect("stale rows")
        .iter()
        .find(|row| row["id"] == "T-held")
        .expect("stable owner stale row");
    assert_eq!(stable_owner["owner"], "ada");
    assert!(stable_owner["claim"].is_null());
}

/// Proof for `rekey.refuses_claimed_task` (`A1-531`).
#[test]
fn rekey_refuses_a_claim_that_would_become_an_orphan() {
    let dir = tmp("claim-rekey");
    fixture(dir.path());
    let claimed = common::run(dir.path(), &["claim", "T-free", "--as", "ada"]);
    assert_eq!(claimed.code, 0, "{}", claimed.text());
    let refused = common::run(dir.path(), &["rekey", "T-free", "T-moved"]);
    assert_ne!(refused.code, 0, "a claimed id was rekeyed");
    assert!(
        refused.text().contains("release T-free"),
        "{}",
        refused.text()
    );
    assert!(dir.path().join("aplan/tasks/T-free.md").exists());
    assert!(!dir.path().join("aplan/tasks/T-moved.md").exists());
}

/// Second proof for `release.recovers_orphaned_local_claims`: branch-local work
/// never looks free from a worktree that cannot see its claimed task.
#[test]
fn a_branch_that_cannot_see_a_claim_fails_closed_until_explicit_release() {
    let dir = tmp("claim-branch");
    let root = dir.path();
    let output = std::process::Command::new("git")
        .args(["init", "--quiet", "--initial-branch", "main"])
        .arg(root)
        .output()
        .expect("git init");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    git(root, &["config", "user.name", "Claim Test"]);
    git(root, &["config", "user.email", "claim@example.invalid"]);
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("tasks");
    std::fs::write(root.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        root.join("aplan/tasks/T-base.md"),
        "---\nid: T-base\ntitle: base\nphase: 0\nstatus: todo\neffort_days: 1\n---\n",
    )
    .expect("base task");
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "base"]);
    git(root, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(
        root.join("aplan/tasks/T-feature.md"),
        "---\nid: T-feature\ntitle: feature\nphase: 0\nstatus: todo\neffort_days: 1\n---\n",
    )
    .expect("feature task");
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "feature task"]);

    let claimed = common::run(root, &["claim", "T-feature", "--as", "ada"]);
    assert_eq!(claimed.code, 0, "{}", claimed.text());
    git(root, &["checkout", "-q", "main"]);

    let refused = common::run(root, &["ready"]);
    assert_ne!(refused.code, 0, "an invisible claim was treated as free");
    assert!(refused.text().contains("cannot see"), "{}", refused.text());
    assert!(refused.text().contains("T-feature"), "{}", refused.text());

    let released = common::run(root, &["release", "T-feature"]);
    assert_eq!(released.code, 0, "{}", released.text());
    let healthy = common::run(root, &["ready"]);
    assert_eq!(healthy.code, 0, "{}", healthy.text());
}

/// Proof for `claim.warns_on_placeholder_identity` (`A1-531`).
#[test]
fn a_placeholder_identity_is_actionably_warned() {
    let dir = tmp("claim-placeholder");
    fixture(dir.path());
    let output = common::run(dir.path(), &["claim", "T-free", "--as", "root"]);
    assert_eq!(output.code, 0, "{}", output.text());
    assert!(output.text().contains("placeholder"), "{}", output.text());
    assert!(output.text().contains("APLAN_OWNER"), "{}", output.text());
}

/// **A takeover names both sides, and a wrong name is refused** (`A1-359`).
///
/// `amend --replace-evidence`'s idiom: naming only the taker leaves a diff
/// nobody can read back, and accepting an unchecked `--from` would let a
/// caller take a task from somebody who is no longer there without noticing
/// that the holder changed under them.
#[test]
fn taking_a_held_task_names_who_it_is_taken_from() {
    let dir = tmp("claim-takeover");
    let dir = dir.path();
    fixture(dir);

    let (said, code) = common::aplan(dir, &["claim", "T-held", "--as", "grace"]);
    assert_ne!(code, 0, "a held task was taken silently:\n{said}");
    assert!(said.contains("ada") && said.contains("--from"), "{said}");
    assert_eq!(
        claim_holder(dir, "T-held").as_deref(),
        Some("ada"),
        "it moved"
    );

    let (said, code) = common::aplan(dir, &["claim", "T-held", "--as", "grace", "--from", "bob"]);
    assert_ne!(code, 0, "a wrong `--from` was accepted:\n{said}");
    assert!(said.contains("ada") && said.contains("bob"), "{said}");
    assert_eq!(
        claim_holder(dir, "T-held").as_deref(),
        Some("ada"),
        "it moved"
    );

    let (said, code) = common::aplan(dir, &["claim", "T-held", "--as", "grace", "--from", "ada"]);
    assert_eq!(code, 0, "a named takeover was refused:\n{said}");
    assert!(
        said.contains("taken from"),
        "the takeover is not recorded: {said}"
    );
    assert_eq!(claim_holder(dir, "T-held").as_deref(), Some("grace"));
    assert_eq!(owner_of(dir, "T-held").as_deref(), Some("ada"));

    // And `--from` against a task nobody holds is a mistake worth naming
    // rather than ignoring: the caller believes somebody holds it.
    let (said, code) = common::aplan(dir, &["claim", "T-free", "--as", "grace", "--from", "ada"]);
    assert_ne!(code, 0, "a `--from` naming nobody was accepted:\n{said}");
    assert!(said.contains("held by nobody"), "{said}");
}

/// **Anybody may release, and it says whose claim it dropped** (`A1-359`).
///
/// A claim has no expiry, so there has to be a way out that does not depend on
/// the holder being around; the alternative turns an abandoned claim into
/// something no command can clear, which is the hand-editing this removes. The
/// bypass is therefore open by design, and the price is that it leaves a
/// record.
#[test]
fn anybody_can_release_a_claim_and_is_told_whose_it_was() {
    let dir = tmp("claim-release");
    let dir = dir.path();
    fixture(dir);

    // `release` takes no identity: who releases is not recorded, only who held
    // it. So the flag is rejected by clap rather than silently ignored.
    let (_, code) = common::aplan(dir, &["release", "T-held", "--as", "somebody-else"]);
    assert_ne!(code, 0, "`release` accepted an identity it does not record");

    let (said, code) = common::aplan(dir, &["release", "T-held"]);
    assert_eq!(code, 0, "{said}");
    assert!(
        said.contains("ada"),
        "the released claim is not named: {said}"
    );
    assert_eq!(claim_holder(dir, "T-held"), None, "the key was left behind");
    assert_eq!(owner_of(dir, "T-held").as_deref(), Some("ada"));

    // Releasing what nobody holds is the outcome the caller asked for, said
    // plainly rather than as a failure a script has to parse.
    let (said, code) = common::aplan(dir, &["release", "T-held"]);
    assert_eq!(code, 0, "{said}");
    assert!(said.contains("nobody"), "{said}");
}

/// Proof for `start.reports_ephemeral_holder`: `start` and `stop` move only the status, and refuse a terminal task
/// (`A1-359`).
///
/// Surgical, to `close`'s standard: the rest of the file is byte-identical, so
/// a claim that work has begun does not arrive as a reformatted document.
#[test]
fn starting_and_stopping_move_the_status_and_nothing_else() {
    let dir = tmp("claim-start");
    let dir = dir.path();
    fixture(dir);
    let before = std::fs::read_to_string(dir.join("aplan/tasks/T-free.md")).expect("read");
    let claim = common::run(dir, &["claim", "T-free", "--as", "grace"]);
    assert_eq!(claim.code, 0, "{}", claim.text());

    let (said, code) = common::aplan(dir, &["start", "T-free"]);
    assert_eq!(code, 0, "{said}");
    assert_eq!(status_of(dir, "T-free"), "in_progress");
    let after = std::fs::read_to_string(dir.join("aplan/tasks/T-free.md")).expect("read");
    assert_eq!(
        after.replace("status: in_progress", "status: todo"),
        before,
        "`start` changed something other than the status"
    );
    // The coordination half is a different act, and the status surface reports
    // its ephemeral record rather than consulting durable owner.
    assert!(said.contains("held by `grace`"), "{said}");

    let (said, code) = common::aplan(dir, &["stop", "T-free"]);
    assert_eq!(code, 0, "{said}");
    assert_eq!(status_of(dir, "T-free"), "todo");
    assert_eq!(
        std::fs::read_to_string(dir.join("aplan/tasks/T-free.md")).expect("read"),
        before,
        "a start followed by a stop did not return the file to what it was"
    );

    // Terminal: nothing reopens a closed task, and the remedy named has to be
    // one that exists for *that* state.
    let (said, code) = common::aplan(dir, &["start", "T-done"]);
    assert_ne!(code, 0, "a closed task was started:\n{said}");
    assert!(
        said.contains("terminal") && said.contains("supersede"),
        "the refusal does not name a remedy that works: {said}"
    );

    // Status and claim are independent. Corrupt local coordination cannot be
    // reported as free work, but it must not prevent a durable start or stop.
    let state = dir.join(".git/aplan/claims-v1.json");
    std::fs::write(&state, "{ not JSON\n").expect("corrupt claim state");
    let (said, code) = common::aplan(dir, &["start", "T-held"]);
    assert_eq!(code, 0, "{said}");
    assert!(said.contains("coordination unavailable"), "{said}");
    assert_eq!(status_of(dir, "T-held"), "in_progress");
    let (said, code) = common::aplan(dir, &["stop", "T-held"]);
    assert_eq!(code, 0, "{said}");
    assert_eq!(status_of(dir, "T-held"), "todo");

    // And stopping something that is not under way says so rather than writing.
    let (said, code) = common::aplan(dir, &["stop", "T-held"]);
    assert_eq!(code, 0, "{said}");
    assert!(
        said.contains("already") || said.contains("nothing to stop"),
        "{said}"
    );
}

/// Second proof for `selection.unclaimed_uses_ephemeral_claims` (`A1-531`).
#[test]
fn unclaimed_refuses_when_local_coordination_cannot_be_observed() {
    let dir = tmp("claim-non-git");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: one\nphase: 0\nstatus: todo\neffort_days: 1\n---\n",
    )
    .expect("task");

    let ordinary = common::run(dir.path(), &["ready", "--format", "json"]);
    assert_eq!(ordinary.code, 0, "{}", ordinary.text());
    let value: serde_json::Value = serde_json::from_str(&ordinary.stdout).expect("ready JSON");
    assert_eq!(value["coordination"]["available"], false);

    let refused = common::run(dir.path(), &["ready", "--unclaimed", "--format", "json"]);
    assert_ne!(refused.code, 0, "unobserved claims were treated as free");
    assert!(
        refused.text().contains("coordination"),
        "{}",
        refused.text()
    );
    assert!(
        refused.text().contains("not evidence"),
        "{}",
        refused.text()
    );
}

/// **Identity resolves explicitly first, and a refusal names every source**
/// (`A1-359`).
///
/// Measured on one machine the three derived sources disagree, which is why the
/// explicit forms come first. The one source never consulted is the
/// repository's own config: several actors share one repository, so a name
/// recorded there would be the wrong one by construction.
#[test]
fn identity_comes_from_the_caller_or_the_environment_and_never_from_the_plan() {
    let dir = tmp("claim-identity");
    let dir = dir.path();
    fixture(dir);
    // A repository-level owner must not be consulted, so putting one there
    // changes nothing. It is also not a key the schema has, which is the
    // stronger statement: `--no-config` is not needed to prove this.
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\nowner: from-config\n").expect("config");
    let (said, code) = common::aplan(dir, &["claim", "T-free", "--as", "explicit"]);
    assert_ne!(
        code, 0,
        "`.aplan.yml` accepted an `owner` key, which is the one place this must not read:\n{said}"
    );

    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    let (said, code) = common::aplan(dir, &["claim", "T-free", "--as", "explicit"]);
    assert_eq!(code, 0, "{said}");
    assert_eq!(claim_holder(dir, "T-free").as_deref(), Some("explicit"));

    // `APLAN_OWNER` when no flag is given, and the flag over it when both are.
    let out = common::command(dir, &["claim", "T-held", "--from", "ada"])
        .env("APLAN_OWNER", "from-env")
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(claim_holder(dir, "T-held").as_deref(), Some("from-env"));

    let out = common::command(
        dir,
        &["claim", "T-held", "--from", "from-env", "--as", "flag-wins"],
    )
    .env("APLAN_OWNER", "from-env")
    .output()
    .expect("run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(claim_holder(dir, "T-held").as_deref(), Some("flag-wins"));
}

/// **Exactly one of eight releases is told it dropped the claim** (`A1-359`,
/// audit).
///
/// The sibling of the claim race, and the same defect one surface over: the
/// holder was read from the plan snapshot, which is taken *before* the file is
/// read. Eight releases all load a plan where `ada` holds the task; the first
/// removes the line; the other seven then read a file that is already unheld,
/// produce content identical to what they read, pass the compare-and-swap
/// because nothing changed, and report that they dropped `ada`'s claim. Seven
/// callers told they displaced somebody who was already gone.
///
/// Counting is what makes this unflakeable: however the threads interleave, the
/// number of callers told they dropped a claim must equal the number of claims
/// there were to drop.
#[test]
fn exactly_one_of_eight_releases_is_told_it_dropped_the_claim() {
    let dir = tmp("release-race");
    let dir = dir.path();
    fixture(dir);

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let dir = dir.to_path_buf();
            std::thread::spawn(move || {
                let out = common::run(&dir, &["release", "T-held"]);
                // `released; it was held by X` versus `held by nobody`.
                out.code == 0 && out.text().contains("it was held by")
            })
        })
        .collect();
    let dropped = handles
        .into_iter()
        .filter_map(|h| h.join().ok())
        .filter(|reported| *reported)
        .count();

    assert_eq!(
        dropped, 1,
        "{dropped} callers were told they dropped `ada`'s claim, and there was one claim"
    );
    assert_eq!(claim_holder(dir, "T-held"), None);
}

/// **Exactly one of eight starts is told it changed the status** (`A1-359`,
/// audit).
///
/// The same shape again. The status was read from the plan snapshot, so eight
/// concurrent `start`s all saw `todo`, none took the "already there" path, and
/// each wrote `in_progress` over a file that mostly already said it -- content
/// identical to what was read, so the compare-and-swap had nothing to object
/// to, and eight callers were told they started the work.
///
/// It matters beyond the count: a caller that is told it moved the task is a
/// caller that believes it now owns the next step.
#[test]
fn exactly_one_of_eight_starts_is_told_it_changed_the_status() {
    let dir = tmp("start-race");
    let dir = dir.path();
    fixture(dir);

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let dir = dir.to_path_buf();
            std::thread::spawn(move || {
                let out = common::run(&dir, &["start", "T-free"]);
                out.code == 0 && out.text().contains("is now `in_progress`")
            })
        })
        .collect();
    let changed = handles
        .into_iter()
        .filter_map(|h| h.join().ok())
        .filter(|reported| *reported)
        .count();

    assert_eq!(
        changed, 1,
        "{changed} callers were told they started `T-free`, and one start happened"
    );
    assert_eq!(status_of(dir, "T-free"), "in_progress");
}

/// **A blank name is a mistake, not an absence** (`A1-359`, `A1-531`, audit).
/// Proof for `owner.refuses_vacuous_values`.
///
/// Two shapes, one stance, and it is the stance `refuse_blank_tags` already
/// takes on `--tag ""`. `--as ""` is almost always `--as "$SOMEVAR"` against an
/// unset variable, and accepting it attributed the claim to whatever the
/// machine's git config said. `--owner ""` was worse: it wrote `owner: ''`, a
/// third state the model does not have: no durable responsibility a reader can
/// see, but not unassigned either. Absent is how the model spells unassigned,
/// and the only way to write that is to leave the key out. Ephemeral claims are
/// separate (`A1-531`).
#[test]
fn a_blank_name_is_refused_rather_than_written_or_ignored() {
    let dir = tmp("claim-blank");
    let dir = dir.path();
    fixture(dir);

    for blank in ["", "   ", "\u{200b}"] {
        let (said, code) = common::aplan(dir, &["claim", "T-free", "--as", blank]);
        assert_ne!(code, 0, "a blank `--as` was accepted:\n{said}");
        assert!(said.contains("blank"), "{said}");
        assert_eq!(claim_holder(dir, "T-free"), None, "it was claimed anyway");

        let (said, code) = common::aplan(
            dir,
            &[
                "add", "new", "--id", "T-new", "--owner", blank, "--effort", "1",
            ],
        );
        assert_ne!(code, 0, "a blank `--owner` was accepted:\n{said}");
        assert!(said.contains("not the same as unassigned"), "{said}");
    }

    // And a task filed with no `--owner` at all is unassigned, which is the
    // state the blank was being mistaken for.
    let (said, code) = common::aplan(dir, &["add", "unheld", "--id", "T-unheld", "--effort", "1"]);
    assert_eq!(code, 0, "{said}");
    let filed = said
        .lines()
        .find_map(|l| l.strip_prefix("wrote "))
        .and_then(|l| l.split_whitespace().next())
        .expect("an id");
    assert_eq!(
        owner_of(dir, filed),
        None,
        "`add` without `--owner` wrote one"
    );
}

/// **The per-user file is read, and loses to the environment** (`A1-364`).
///
/// End to end, because the unit tests cover `resolve` and this covers the half
/// that finds the file at all: `$XDG_CONFIG_HOME` first, then `$HOME/.config`,
/// which is what the XDG base-directory specification says and what every
/// other tool on the machine already does.
///
/// The actor this serves is the one `A1-359` was built for: an agent reaching
/// aplan through the MCP server, started by a desktop client that sources no
/// shell profile. It has no `APLAN_OWNER` to inherit, and the next guess down
/// is `git config user.name`, which on the machine this was measured on said
/// `t`, left by a test setup.
#[test]
fn the_per_user_file_names_the_claimant_and_the_environment_overrides_it() {
    let dir = tmp("user-config");
    let dir = dir.path();
    fixture(dir);
    let home = tmp("user-home");
    let home = home.path();
    std::fs::create_dir_all(home.join(".config/aplan")).expect("mkdir");
    std::fs::write(home.join(".config/aplan/config.yml"), "me: from-file\n").expect("write");

    let out = common::command(dir, &["claim", "T-free"])
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("APLAN_OWNER")
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(claim_holder(dir, "T-free").as_deref(), Some("from-file"));

    // The variable is set for this run, so it wins.
    let out = common::command(dir, &["claim", "T-free", "--from", "from-file"])
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env("APLAN_OWNER", "from-env")
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(claim_holder(dir, "T-free").as_deref(), Some("from-env"));

    // `XDG_CONFIG_HOME` is honoured ahead of `$HOME/.config`.
    std::fs::create_dir_all(home.join("xdg/aplan")).expect("mkdir");
    std::fs::write(home.join("xdg/aplan/config.yml"), "me: from-xdg\n").expect("write");
    let out = common::command(dir, &["claim", "T-free", "--from", "from-env"])
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("xdg"))
        .env_remove("APLAN_OWNER")
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(claim_holder(dir, "T-free").as_deref(), Some("from-xdg"));
}

/// **A per-user file holding a repository's key is refused by name, and the
/// command carries on** (`A1-364`).
///
/// Two halves. The refusal has to say *why* -- a reader who put
/// `deny_warnings` here is a reader who believes it is in effect, and the one
/// thing this file must never be able to do is relax a repository's policy. And
/// it must not be fatal: `resolve` has three more sources, and the caller is
/// trying to record a claim rather than to fix their configuration.
#[test]
fn a_repository_key_in_the_per_user_file_is_named_and_not_fatal() {
    let dir = tmp("user-config-wrong");
    let dir = dir.path();
    fixture(dir);
    let home = tmp("user-home-wrong");
    let home = home.path();
    std::fs::create_dir_all(home.join(".config/aplan")).expect("mkdir");
    std::fs::write(
        home.join(".config/aplan/config.yml"),
        "deny_warnings: false\n",
    )
    .expect("write");

    let out = common::command(dir, &["claim", "T-free", "--as", "ada"])
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("APLAN_OWNER")
        .output()
        .expect("run");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("never a repository's policy"),
        "the refusal does not say why the key is not accepted: {said}"
    );
    assert!(
        out.status.success(),
        "a bad per-user file stopped a claim it has nothing to do with: {said}"
    );
    assert_eq!(
        claim_holder(dir, "T-free").as_deref(),
        Some("ada"),
        "the claim did not go through"
    );
}

/// **A repository cannot supply the per-user identity** (`A1-364`, audit).
///
/// The end-to-end half of the trust boundary. `HOME=""` is set-but-empty,
/// which happens in containers, systemd units and some CI runners; joined, it
/// yielded the relative path `.config/aplan/config.yml`, read from the working
/// directory. So a repository that shipped that file named whoever ran
/// `aplan claim` in it -- the repository-supplied identity this design rejected
/// when it refused to read a name from `.aplan.yml`, reached by another route.
///
/// **The claim has to succeed for this to test anything.** The first version of
/// this test passed `--from`, which made every run refuse before the identity
/// was consulted, so it passed with the guard removed as happily as with it.
/// One free task per case, no `--from`, and the assertion is on the name that
/// was recorded.
#[test]
fn a_file_in_the_repository_is_never_read_as_the_per_user_identity() {
    let dir = tmp("user-config-injection");
    let dir = dir.path();
    fixture(dir);
    std::fs::create_dir_all(dir.join(".config/aplan")).expect("mkdir");
    std::fs::write(
        dir.join(".config/aplan/config.yml"),
        "me: injected-by-the-repo\n",
    )
    .expect("write");

    for (n, (xdg, home)) in [
        (None, Some("")),
        (Some(".config"), Some("")),
        (Some(""), Some("")),
    ]
    .into_iter()
    .enumerate()
    {
        // A fresh unheld task per case, so each claim can succeed and the
        // recorded name is what is being measured.
        let id = format!("T-inject{n}");
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: free\nphase: 0\nstatus: todo\neffort_days: 1\n\
                 ---\n\n## Why\n\nx\n"
            ),
        )
        .expect("task");

        let mut cmd = common::command(dir, &["claim", &id]);
        cmd.env_remove("APLAN_OWNER")
            .env("HOME", home.unwrap_or(""));
        match xdg {
            Some(v) => cmd.env("XDG_CONFIG_HOME", v),
            None => cmd.env_remove("XDG_CONFIG_HOME"),
        };
        let out = cmd.output().expect("run");
        // **Success is not required, and requiring it was wrong.** With `HOME`
        // emptied there may be no identity left on the machine at all -- a
        // container has no `USER` and no `git config user.name` either -- and
        // refusing is then the correct outcome. The first version of this
        // assertion demanded a successful claim and so passed locally and failed
        // in CI, which is the test encoding an assumption about the machine
        // rather than about the code (`A1-364`, found by CI).
        //
        // What must hold either way is the only thing this test is about: the
        // name in the repository's own file is never the one recorded. With the
        // guard removed the claim *succeeds* using it, so the assertion still
        // bites.
        assert_ne!(
            claim_holder(dir, &id).as_deref(),
            Some("injected-by-the-repo"),
            "xdg={xdg:?} home={home:?}: the repository's own file named the claimant; \
             the command said: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// **`--no-config` does not suppress the per-user file, deliberately**
/// (`A1-364`, audit).
///
/// That flag exists so a tree under review cannot say what a run may do: it
/// refuses the repository's `.aplan.yml`, its runners and its policy. The
/// per-user file is not the tree's, and by construction it holds nothing a
/// repository can require, so there is nothing in it for the flag to protect a
/// reviewer from -- and suppressing it would cost `claim` the operator's
/// identity for no safety gained.
///
/// Pinned because the flag's own contract used to read "no configuration at
/// all", which stopped being true when this file started being read. A trust
/// boundary described imprecisely is the kind of documentation defect that
/// matters, so the wording and the behaviour are now checked together.
#[test]
fn no_config_refuses_the_repositorys_policy_and_still_knows_who_you_are() {
    let dir = tmp("user-config-no-config");
    let dir = dir.path();
    fixture(dir);
    let home = tmp("user-home-no-config");
    let home = home.path();
    std::fs::create_dir_all(home.join(".config/aplan")).expect("mkdir");
    std::fs::write(home.join(".config/aplan/config.yml"), "me: from-the-file\n").expect("write");

    let out = common::command(dir, &["--no-config", "claim", "T-free"])
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("APLAN_OWNER")
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        claim_holder(dir, "T-free").as_deref(),
        Some("from-the-file"),
        "`--no-config` cost the operator their own identity"
    );

    // And the note it prints is true: what it refused was this repository's.
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("no configuration was read from this repository"),
        "the note overstates what was refused: {said}"
    );
}
