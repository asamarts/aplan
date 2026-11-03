//! Non-blocking evidence observations (`A1-590`).

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use crate::common;

fn executable(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write runner");
    let mut permissions = path.metadata().expect("runner metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("runner executable");
}

fn fixture() -> (tempfile::TempDir, tempfile::TempDir) {
    let directory = tempfile::Builder::new()
        .prefix("evidence-observation-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("repository");
    let cache = tempfile::Builder::new()
        .prefix("evidence-observation-cache-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("cache");
    let root = directory.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(root.join("src")).expect("sources");
    std::fs::create_dir_all(root.join("runners")).expect("runners");
    std::fs::write(root.join("src/lib.rs"), "pub fn observed() {}\n").expect("source");
    executable(
        &root.join("runners/one.sh"),
        "#!/bin/sh\nprintf 'one\\n' >> provider-runs.txt\nprintf 'suite::one: test\\n'\n",
    );
    executable(
        &root.join("runners/two.sh"),
        "#!/bin/sh\nprintf 'two\\n' >> provider-runs.txt\nprintf 'suite::two: test\\n'\n",
    );
    std::fs::write(
        root.join(".aplan.yml"),
        "root: aplan\nevidence:\n  observation_inputs: [src/**, runners/**]\ntest:\n\
         - name: one\n  list: [sh, runners/one.sh]\n  run: ['true', '{test}']\n\
         - name: two\n  list: [sh, runners/two.sh]\n  run: ['true', '{test}']\n",
    )
    .expect("config");
    std::fs::write(
        root.join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: observed task\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths: [src/**]\nevidence:\n- kind: test\n  ref: suite::one\ntags: [cross-cutting]\n---\n",
    )
    .expect("task");
    std::fs::write(
        root.join("aplan/tasks/T-2.md"),
        "---\nid: T-2\ntitle: local evidence only\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths: [src/**]\nevidence:\n- kind: note\n  ref: recorded locally\ntags: [cross-cutting]\n---\n",
    )
    .expect("task");
    common::git(root, &["init", "-q", "."]);
    common::git(root, &["config", "user.email", "test@example.com"]);
    common::git(root, &["config", "user.name", "test"]);
    common::git(root, &["add", "-A"]);
    common::git(root, &["commit", "-qm", "fixture"]);
    (directory, cache)
}

fn run(root: &Path, cache: &Path, args: &[&str]) -> common::Run {
    let output = common::command(root, args)
        .env("APLAN_CACHE_DIR", cache)
        .output()
        .expect("run aplan");
    common::Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(-1),
    }
}

fn json(run: &common::Run) -> serde_json::Value {
    serde_json::from_str(&run.stdout)
        .unwrap_or_else(|error| panic!("JSON output ({error}): {}", run.text()))
}

/// Proof for `show.evidence_observations_are_nonblocking`.
#[test]
fn show_default_never_starts_configured_providers_and_requires_current_explicitly() {
    let (directory, cache) = fixture();
    let root = directory.path();
    let shown = run(root, cache.path(), &["show", "T-1", "--format", "json"]);
    assert_eq!(shown.code, 0, "{}", shown.text());
    assert!(!root.join("provider-runs.txt").exists(), "provider ran");
    let document = json(&shown);
    assert_eq!(document["schema"], "aplan/show/v3");
    assert_eq!(document["resolved"], false);
    assert_eq!(document["evidence_observation"]["state"], "not_observed");
    assert_eq!(
        document["evidence"][0]["observation"]["state"],
        "not_observed"
    );

    let required = run(
        root,
        cache.path(),
        &[
            "show",
            "T-1",
            "--format",
            "json",
            "--require-current-evidence",
        ],
    );
    assert_eq!(required.code, 1, "{}", required.text());
    assert!(serde_json::from_str::<serde_json::Value>(&required.stdout).is_ok());
    assert!(!root.join("provider-runs.txt").exists(), "provider ran");

    let local_refresh = run(
        root,
        cache.path(),
        &["show", "T-2", "--format", "json", "--refresh-evidence"],
    );
    assert_eq!(local_refresh.code, 0, "{}", local_refresh.text());
    assert_eq!(
        json(&local_refresh)["evidence_observation"]["state"],
        "current"
    );
    assert!(local_refresh.stderr.is_empty(), "{}", local_refresh.text());
    assert!(
        !root.join("provider-runs.txt").exists(),
        "a task with no test citation enumerated configured runners"
    );
}

/// Proof for `show.evidence_observations_are_nonblocking`.
#[test]
fn refresh_reuses_raw_catalogues_rederives_plan_changes_and_marks_input_changes_stale() {
    let (directory, cache) = fixture();
    let root = directory.path();
    let refreshed = run(
        root,
        cache.path(),
        &["show", "T-1", "--format", "json", "--refresh-evidence"],
    );
    assert_eq!(refreshed.code, 0, "{}", refreshed.text());
    let document = json(&refreshed);
    assert_eq!(document["resolved"], true, "{document}");
    assert_eq!(document["evidence_observation"]["state"], "current");
    assert_eq!(
        document["evidence"][0]["observation"]["outcome"],
        "resolved"
    );
    assert!(
        refreshed.stderr.is_empty(),
        "machine output must not opt into progress implicitly: {}",
        refreshed.text()
    );
    assert!(!refreshed.stdout.contains("evidence refresh:"));
    let runs = std::fs::read_to_string(root.join("provider-runs.txt")).expect("runner log");
    assert_eq!(runs.lines().count(), 2, "{runs}");

    let task = root.join("aplan/tasks/T-1.md");
    let before = std::fs::read_to_string(&task).expect("task");
    std::fs::write(&task, before.replace("observed task", "renamed task")).expect("plan edit");
    let replanned = run(root, cache.path(), &["show", "T-1", "--format", "json"]);
    assert_eq!(replanned.code, 0, "{}", replanned.text());
    let replanned_json = json(&replanned);
    assert_eq!(replanned_json["evidence_observation"]["state"], "current");
    assert_eq!(replanned_json["title"], "renamed task");
    let after_runs = std::fs::read_to_string(root.join("provider-runs.txt")).expect("runner log");
    assert_eq!(after_runs, runs, "plan-only read reran a provider");
    assert_ne!(
        replanned_json["evidence_observation"]["resolution_identity"],
        document["evidence_observation"]["resolution_identity"],
        "plan-only change reused the old derived verdict identity"
    );

    std::fs::write(root.join("src/lib.rs"), "pub fn changed() {}\n").expect("input edit");
    let stale = run(root, cache.path(), &["show", "T-1", "--format", "json"]);
    assert_eq!(stale.code, 0, "{}", stale.text());
    let stale_json = json(&stale);
    assert_eq!(stale_json["resolved"], false);
    assert_eq!(stale_json["evidence_observation"]["state"], "stale");
    assert_eq!(stale_json["evidence"][0]["observation"]["state"], "stale");
    let final_runs = std::fs::read_to_string(root.join("provider-runs.txt")).expect("runner log");
    assert_eq!(final_runs, runs, "stale read reran a provider");
}

#[test]
fn explicit_refresh_exposes_unsupported_and_failed_provider_states() {
    let (unsupported_directory, unsupported_cache) = fixture();
    let unsupported_root = unsupported_directory.path();
    let config = unsupported_root.join(".aplan.yml");
    let before = std::fs::read_to_string(&config).expect("config");
    std::fs::write(
        &config,
        before.replace(
            "evidence:\n  observation_inputs: [src/**, runners/**]\n",
            "",
        ),
    )
    .expect("remove exhaustive claim");
    let unsupported = run(
        unsupported_root,
        unsupported_cache.path(),
        &["show", "T-1", "--format", "json", "--refresh-evidence"],
    );
    assert_eq!(unsupported.code, 0, "{}", unsupported.text());
    let unsupported = json(&unsupported);
    assert_eq!(unsupported["evidence_observation"]["state"], "unsupported");
    assert_eq!(
        unsupported["evidence"][0]["observation"]["state"],
        "unsupported"
    );

    let (failed_directory, failed_cache) = fixture();
    let failed_root = failed_directory.path();
    executable(
        &failed_root.join("runners/one.sh"),
        "#!/bin/sh\nprintf 'failed\\n' >> provider-runs.txt\nexit 9\n",
    );
    let failed = run(
        failed_root,
        failed_cache.path(),
        &["show", "T-1", "--format", "json", "--refresh-evidence"],
    );
    assert_eq!(failed.code, 0, "{}", failed.text());
    let failed = json(&failed);
    assert_eq!(failed["evidence_observation"]["state"], "refresh_failed");
    assert_eq!(
        failed["evidence"][0]["observation"]["state"],
        "refresh_failed"
    );
    assert_eq!(
        failed["evidence"][0]["observation"]["outcome"],
        "unavailable"
    );
}
