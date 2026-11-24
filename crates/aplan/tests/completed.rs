//! End-to-end CLI tests for `aplan completed` (`A2-111`, ADR-0074).

use crate::common;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str], commit_ts: Option<i64>) {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "tester")
        .env("GIT_AUTHOR_EMAIL", "tester@example.com")
        .env("GIT_COMMITTER_NAME", "tester")
        .env("GIT_COMMITTER_EMAIL", "tester@example.com");
    if let Some(ts) = commit_ts {
        cmd.env("GIT_AUTHOR_DATE", format!("{ts} +0000"));
        cmd.env("GIT_COMMITTER_DATE", format!("{ts} +0000"));
    }
    let out = cmd.output().expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn completed_cli_supports_ordering_filters_bounds_history_at_paging_and_json() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("mkdir");
    git(root, &["init", "-q", "-b", "main", "."], None);

    // Commit 1: C-LEG starts todo; C-REC already done with recorded closed_at;
    // M-1 is a done milestone (must stay out of completed work view);
    // C-DROP is dropped (must stay out of completed work view).
    std::fs::write(
        root.join("aplan/tasks/C-LEG.md"),
        "---\nid: C-LEG\ntitle: Legacy task\nphase: 1\nstatus: todo\neffort_days: 1\n\
         owner: alice\ntags: [cli]\n---\n\nBody.\n",
    )
    .expect("C-LEG");
    std::fs::write(
        root.join("aplan/tasks/C-REC.md"),
        "---\nid: C-REC\ntitle: Recorded task\nphase: 2\nstatus: done\neffort_days: 1\n\
         owner: bob\ntags: [engine]\nclosed_at: 2026-10-10T12:00:00Z\n\
         evidence:\n  - kind: note\n    ref: done\n    captured: 2026-10-10\n---\n\nBody.\n",
    )
    .expect("C-REC");
    std::fs::write(
        root.join("aplan/tasks/M-1.md"),
        "---\nid: M-1\ntitle: Cut 1\nphase: 1\nstatus: done\ntags: [milestone, v1]\n\
         closed_at: 2026-10-15T00:00:00Z\nblocked_by: [C-REC]\n\
         evidence:\n  - kind: note\n    ref: cut\n    captured: 2026-10-15\n---\n\nCut.\n",
    )
    .expect("M-1");
    std::fs::write(
        root.join("aplan/tasks/C-DROP.md"),
        "---\nid: C-DROP\ntitle: Dropped task\nphase: 1\nstatus: dropped\neffort_days: 1\n---\n\nDrop.\n",
    )
    .expect("C-DROP");
    git(root, &["add", "-A"], None);
    git(root, &["commit", "-q", "-m", "c1"], Some(1_791_500_000));

    // Commit 2 (2026-10-12T00:00:00Z = 1_791_763_200): C-LEG transitions to done without closed_at.
    std::fs::write(
        root.join("aplan/tasks/C-LEG.md"),
        "---\nid: C-LEG\ntitle: Legacy task\nphase: 1\nstatus: done\neffort_days: 1\n\
         owner: alice\ntags: [cli]\n\
         evidence:\n  - kind: note\n    ref: legacy\n    captured: 2026-10-12\n---\n\nBody.\n",
    )
    .expect("C-LEG done");
    git(root, &["add", "-A"], None);
    git(root, &["commit", "-q", "-m", "c2"], Some(1_791_763_200));

    // 1. Default newest-first JSON: C-LEG (2026-10-12, history_first_seen) then C-REC (2026-10-10, recorded).
    let out = common::run(root, &["completed", "--format", "json"]);
    assert_eq!(
        out.code, 0,
        "completed json failed:\n{}{}",
        out.stdout, out.stderr
    );
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect("valid json");
    assert_eq!(doc["schema"], "aplan/completed/v1");
    assert_eq!(doc["completeness"], "complete");
    let items = doc["items"].as_array().expect("items array");
    let ids: Vec<&str> = items
        .iter()
        .map(|i| i["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, vec!["C-LEG", "C-REC"]);
    assert_eq!(items[0]["source"], "history_first_seen");
    assert_eq!(items[0]["instant"], "2026-10-12T00:00:00Z");
    assert!(items[0]["commit"].is_string());
    assert_eq!(items[1]["source"], "recorded");
    assert_eq!(items[1]["recorded_origin"], "local");

    // 2. `--history never` leaves C-LEG as `unknown` after known C-REC, even under `--oldest`.
    let never = common::run(
        root,
        &[
            "completed",
            "--oldest",
            "--history",
            "never",
            "--format",
            "json",
        ],
    );
    assert_eq!(
        never.code, 0,
        "never failed:\n{}{}",
        never.stdout, never.stderr
    );
    let never_doc: serde_json::Value = serde_json::from_str(&never.stdout).expect("valid json");
    assert_eq!(never_doc["completeness"], "history_skipped");
    let never_ids: Vec<&str> = never_doc["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["id"].as_str().expect("id"))
        .collect();
    assert_eq!(never_ids, vec!["C-REC", "C-LEG"]);
    assert_eq!(never_doc["items"][1]["source"], "unknown");
    assert!(never_doc["items"][1]["instant"].is_null());

    // 3. `--at HEAD~1` reads committed plan AND history from commit 1 (only C-REC is done).
    let at_prev = common::run(root, &["completed", "--at", "HEAD~1", "--format", "json"]);
    assert_eq!(
        at_prev.code, 0,
        "--at HEAD~1 failed:\n{}{}",
        at_prev.stdout, at_prev.stderr
    );
    let at_doc: serde_json::Value = serde_json::from_str(&at_prev.stdout).expect("valid json");
    let at_ids: Vec<&str> = at_doc["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["id"].as_str().expect("id"))
        .collect();
    assert_eq!(at_ids, vec!["C-REC"]);

    // 4. Filters (`--cut`, `--phase`, `--owner`, `--tag`, `--status`, `--unclaimed`,
    //    `--since`/`--until`) apply before `--limit`.
    let filtered = common::run(
        root,
        &[
            "completed",
            "--cut",
            "M-1",
            "--phase",
            "2",
            "--owner",
            "bob",
            "--tag",
            "engine",
            "--status",
            "done",
            "--since",
            "2026-10-10",
            "--until",
            "2026-10-10",
            "--limit",
            "1",
            "--format",
            "json",
        ],
    );
    assert_eq!(
        filtered.code, 0,
        "filtered failed:\n{}{}",
        filtered.stdout, filtered.stderr
    );
    let f_doc: serde_json::Value = serde_json::from_str(&filtered.stdout).expect("valid json");
    assert_eq!(f_doc["total_matched"], 1);
    assert_eq!(f_doc["items"][0]["id"], "C-REC");
    let unclaimed = common::run(root, &["completed", "--unclaimed", "--format", "json"]);
    assert_eq!(
        unclaimed.code, 0,
        "unclaimed failed:\n{}{}",
        unclaimed.stdout, unclaimed.stderr
    );

    // 5. Pagination via `--limit 1` + `--cursor` returns both pages cleanly.
    let p1 = common::run(root, &["completed", "--limit", "1", "--format", "json"]);
    assert_eq!(p1.code, 0);
    let p1_doc: serde_json::Value = serde_json::from_str(&p1.stdout).expect("p1 json");
    assert_eq!(p1_doc["truncated"], true);
    let cursor = p1_doc["next_cursor"].as_str().expect("next_cursor");
    let p2 = common::run(
        root,
        &[
            "completed",
            "--limit",
            "1",
            "--cursor",
            cursor,
            "--format",
            "json",
        ],
    );
    assert_eq!(p2.code, 0, "page 2 failed:\n{}{}", p2.stdout, p2.stderr);
    let p2_doc: serde_json::Value = serde_json::from_str(&p2.stdout).expect("p2 json");
    assert_eq!(p2_doc["truncated"], false);
    assert_eq!(p2_doc["items"][0]["id"], "C-REC");

    // 6. Invalid inputs (`--limit 0`, reversed bounds) are refused clearly.
    assert_ne!(common::run(root, &["completed", "--limit", "0"]).code, 0);
    assert_ne!(
        common::run(
            root,
            &[
                "completed",
                "--since",
                "2026-10-12",
                "--until",
                "2026-10-10"
            ]
        )
        .code,
        0
    );

    // 7. Human output shows absolute time, provenance cues, and non-causal note.
    let human = common::run(root, &["completed"]);
    assert_eq!(
        human.code, 0,
        "human failed:\n{}{}",
        human.stdout, human.stderr
    );
    assert!(
        human.stdout.contains("2026-10-12T00:00:00Z"),
        "{}",
        human.stdout
    );
    assert!(human.stdout.contains("history:"), "{}", human.stdout);
    assert!(human.stdout.contains("recorded"), "{}", human.stdout);
    assert!(
        human.stdout.contains("not a causal sequence"),
        "{}",
        human.stdout
    );
}
