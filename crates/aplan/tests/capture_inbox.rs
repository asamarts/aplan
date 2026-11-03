//! The CLI gate/graph boundary for versioned capture intake (`A1-514`).

use crate::common;

const ID: &str = "I-01K8Z3A4H00000000000000000";

fn repository(config: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("capture-inbox-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    git2::Repository::init(dir.path()).expect("git repository");
    std::fs::create_dir_all(dir.path().join("aplan/.inbox")).expect("inbox");
    std::fs::write(dir.path().join(".aplan.yml"), config).expect("config");
    std::fs::write(
        dir.path().join("aplan/work.md"),
        "---\nid: A-1\ntitle: work\nphase: 1\nstatus: todo\neffort_days: 1\n---\n",
    )
    .expect("task");
    dir
}

fn valid_capture(kind: &str) -> String {
    format!(
        "schema: aplan/capture/v1\nid: {ID}\ntitle: captured\nkind: {kind}\nstate: open\ncaptured_at: '2026-01-01T00:00:00Z'\norigin: cli\n"
    )
}

/// `check` and `verify` are structural gates over captures, while a graph read
/// remains task-only even when the inbox cannot be interpreted.
#[test]
fn capture_validation_blocks_gates_without_entering_task_graph_reads() {
    let dir = repository("root: aplan\n");
    std::fs::write(
        dir.path().join(format!("aplan/.inbox/{ID}.yml")),
        "schema: aplan/capture/v1\nid: [unterminated\n",
    )
    .expect("malformed capture");

    for command in [["check"].as_slice(), ["verify"].as_slice()] {
        let out = common::run(dir.path(), command);
        assert_eq!(out.code, 1, "{}: {}", command[0], out.text());
        assert!(out.text().contains("APLAN057"), "{}", out.text());
        assert!(
            out.text().contains("invalid capture YAML"),
            "{}",
            out.text()
        );
    }

    let verified = common::run(dir.path(), &["verify", "--format", "json"]);
    assert_eq!(verified.code, 1, "{}", verified.text());
    let document: serde_json::Value =
        serde_json::from_str(&verified.stdout).expect("verify JSON report");
    assert_eq!(document["schema"], "aplan/verify-report/v1");

    let ready = common::run(dir.path(), &["ready"]);
    assert_eq!(ready.code, 0, "{}", ready.text());
    assert!(ready.text().contains("A-1"), "{}", ready.text());
    assert!(!ready.text().contains("APLAN057"), "{}", ready.text());

    let add = common::run(
        dir.path(),
        &["add", "must not publish", "--id", "A-2", "--effort", "1"],
    );
    assert_eq!(add.code, 2, "{}", add.text());
    assert!(add.text().contains("APLAN057"), "{}", add.text());
    assert!(
        aplan_core::load_plan(&dir.path().join("aplan"))
            .tasks
            .iter()
            .all(|task| task.task.id != "A-2"),
        "a guarded plan mutation must publish nothing while the inbox is malformed"
    );
}

/// The selected repository config extends kinds for the selected plan root;
/// absent config leaves exactly the built-ins.
#[test]
fn selected_root_and_capture_config_control_inbox_validation() {
    let dir = repository("root: aplan\ncapture:\n  kinds: [question]\n");
    std::fs::write(
        dir.path().join(format!("aplan/.inbox/{ID}.yml")),
        valid_capture("question"),
    )
    .expect("capture");
    let configured = common::run(dir.path(), &["check"]);
    assert_eq!(configured.code, 0, "{}", configured.text());

    let builtins = common::run(dir.path(), &["--no-config", "--root", "aplan", "check"]);
    assert_eq!(builtins.code, 1, "{}", builtins.text());
    assert!(
        builtins.text().contains("capture.kinds"),
        "{}",
        builtins.text()
    );
}

fn capture_json(dir: &std::path::Path, args: &[&str]) -> serde_json::Value {
    let out = common::run(dir, args);
    assert_eq!(out.code, 0, "{}", out.text());
    serde_json::from_str(&out.stdout)
        .unwrap_or_else(|error| panic!("capture output is not JSON: {error}\n{}", out.text()))
}

/// The complete CLI lifecycle keeps intake outside the graph, carries full ids
/// in machine output, hides only the opaque source key from collections, and
/// never turns a preview into a write.
#[test]
#[allow(clippy::too_many_lines)] // One ordered journey proves identity and every lifecycle transition together.
fn capture_cli_create_list_show_amend_discard_restore_and_dry_run() {
    const BEHAVIOUR: &str = "capture.inbox_lifecycle_commands";
    assert_eq!(BEHAVIOUR, "capture.inbox_lifecycle_commands");
    let dir = repository("root: aplan\ncapture:\n  kinds: [question]\n");
    let created = capture_json(
        dir.path(),
        &[
            "capture",
            "Could this be faster?",
            "--kind",
            "question",
            "--label",
            "ux",
            "--label",
            "review",
            "--body",
            "Initial detail",
            "--source",
            "pilot:opaque-secret",
            "--format",
            "json",
        ],
    );
    assert_eq!(created["outcome"], "applied");
    let id = created["capture"]["id"].as_str().expect("full id");
    assert_eq!(id.len(), 28);
    let path = dir.path().join(format!("aplan/.inbox/{id}.yml"));
    let original = std::fs::read_to_string(&path).expect("created capture");

    let retried = capture_json(
        dir.path(),
        &[
            "capture",
            "Could this be faster?",
            "--kind",
            "question",
            "--label",
            "ux",
            "--label",
            "review",
            "--body",
            "Initial detail",
            "--source",
            "pilot:opaque-secret",
            "--format",
            "json",
        ],
    );
    assert_eq!(retried["outcome"], "unchanged");
    assert_eq!(retried["capture"]["id"], id);
    assert_eq!(
        std::fs::read_dir(dir.path().join("aplan/.inbox"))
            .unwrap()
            .count(),
        1
    );

    let retried_preview = capture_json(
        dir.path(),
        &[
            "capture",
            "Could this be faster?",
            "--kind",
            "question",
            "--label",
            "ux",
            "--label",
            "review",
            "--body",
            "Initial detail",
            "--source",
            "pilot:opaque-secret",
            "--dry-run",
            "--format",
            "json",
        ],
    );
    assert_eq!(retried_preview["outcome"], "unchanged");
    assert_eq!(retried_preview["capture"]["id"], id);
    assert_eq!(retried_preview["files"], serde_json::json!([]));

    let list = capture_json(dir.path(), &["inbox", "--format", "json"]);
    assert_eq!(list["complete"], true);
    assert_eq!(list["deferred"], 0);
    assert_eq!(list["captures"][0]["id"], id);
    assert_eq!(list["captures"][0]["source_namespace"], "pilot");
    let list_text = serde_json::to_string(&list).unwrap();
    assert!(!list_text.contains("opaque-secret"), "{list_text}");

    let prefix = &id[..8];
    let focused = capture_json(dir.path(), &["inbox", "show", prefix, "--format", "json"]);
    assert_eq!(focused["capture"]["id"], id);
    assert_eq!(focused["capture"]["source"]["key"], "opaque-secret");

    let predicted = capture_json(
        dir.path(),
        &[
            "inbox",
            "amend",
            prefix,
            "--title",
            "Predicted only",
            "--dry-run",
            "--format",
            "json",
        ],
    );
    assert_eq!(predicted["outcome"], "predicted");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);

    let human_preview = common::run(
        dir.path(),
        &[
            "inbox",
            "amend",
            prefix,
            "--title",
            "Human preview only",
            "--dry-run",
        ],
    );
    assert_eq!(human_preview.code, 0, "{}", human_preview.text());
    for field in [
        "input snapshot:",
        "plan snapshot:",
        "configuration snapshot:",
        "inbox snapshot:",
        "publishable: yes",
        "file:",
    ] {
        assert!(
            human_preview.stdout.contains(field),
            "human action omitted `{field}`: {}",
            human_preview.stdout
        );
    }
    assert!(human_preview.stdout.contains(&format!(".inbox/{id}.yml")));

    let amended = capture_json(
        dir.path(),
        &[
            "inbox",
            "amend",
            prefix,
            "--title",
            "Review later",
            "--revisit-at",
            "2099-01-01T00:00:00Z",
            "--clear-labels",
            "--format",
            "json",
        ],
    );
    assert_eq!(amended["capture"]["id"], id);
    assert_eq!(amended["capture"]["origin"], "cli");
    assert_eq!(amended["capture"]["source"]["key"], "opaque-secret");
    assert!(amended["capture"].get("labels").is_none());

    let deferred = capture_json(dir.path(), &["inbox", "--format", "json"]);
    assert_eq!(deferred["captures"], serde_json::json!([]));
    assert_eq!(deferred["deferred"], 1);
    let explicit = capture_json(
        dir.path(),
        &["inbox", "--revisit", "deferred", "--format", "json"],
    );
    assert_eq!(explicit["captures"][0]["id"], id);

    let discarded = capture_json(
        dir.path(),
        &[
            "inbox", "discard", prefix, "--reason", "not work", "--format", "json",
        ],
    );
    assert_eq!(discarded["capture"]["state"], "discarded");
    assert!(discarded["capture"]["disposed_at"].is_string());
    let refused = common::run(
        dir.path(),
        &["inbox", "amend", prefix, "--title", "too late"],
    );
    assert_eq!(refused.code, 2, "{}", refused.text());
    assert!(
        refused.text().contains("only open captures"),
        "{}",
        refused.text()
    );

    let restored = capture_json(
        dir.path(),
        &["inbox", "restore", prefix, "--format", "json"],
    );
    assert_eq!(restored["capture"]["state"], "open");
    assert!(restored["capture"].get("disposed_at").is_none());
    assert!(restored["capture"].get("reason").is_none());
    assert_eq!(restored["capture"]["source"]["key"], "opaque-secret");

    let check = common::run(dir.path(), &["check"]);
    assert_eq!(check.code, 0, "{}", check.text());
    let ready = common::run(dir.path(), &["ready"]);
    assert_eq!(ready.code, 0, "{}", ready.text());
    assert!(
        !ready.text().contains(id),
        "a capture entered the task graph"
    );
}

fn promotion_repository() -> tempfile::TempDir {
    let dir = repository(
        r"root: aplan
deny_warnings: true
require_cut_membership: true
tag_families:
  intake:
    members: [from-profile, explicit]
    require: at-most-one
capture:
  profiles:
    triage:
      promotion:
        phase: 1
        cut: M-next
        effort_days: 2
        owner: intake
        paths: [src/**]
        tags: [from-profile]
",
    );
    std::fs::write(
        dir.path().join("aplan/work.md"),
        "---\nid: A-1\ntitle: existing work\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/**]\n---\n",
    )
    .expect("existing task");
    std::fs::write(
        dir.path().join("aplan/cut.md"),
        "---\nid: M-next\ntitle: next cut\nphase: 1\nstatus: todo\nblocked_by: [A-1]\ntags: [milestone]\n---\n",
    )
    .expect("cut");
    dir
}

/// Proof for behaviour `capture.promotion_is_atomic`: promotion transfers the
/// origin snapshot through ordinary task validation, publishes both sides, and
/// a lost-response retry cannot create a second task.
#[test]
#[allow(clippy::too_many_lines)]
fn capture_promotion_is_validated_atomic_and_idempotent() {
    const BEHAVIOUR: &str = "capture.promotion_is_atomic";
    assert_eq!(BEHAVIOUR, "capture.promotion_is_atomic");
    let dir = promotion_repository();
    let created = capture_json(
        dir.path(),
        &[
            "capture",
            "Promoted work",
            "--profile",
            "triage",
            "--body",
            "## Why\n\nRetained intake detail.\n",
            "--format",
            "json",
        ],
    );
    let capture_id = created["capture"]["id"].as_str().expect("capture id");
    let capture_path = dir.path().join(format!("aplan/.inbox/{capture_id}.yml"));
    let open_bytes = std::fs::read_to_string(&capture_path).expect("open capture");

    let preview = capture_json(
        dir.path(),
        &[
            "inbox",
            "promote",
            capture_id,
            "--task-id",
            "A-2",
            "--phase",
            "1",
            "--cut",
            "M-next",
            "--effort",
            "3",
            "--owner",
            "delivery",
            "--path",
            "crates/**",
            "--tag",
            "explicit",
            "--reason",
            "triaged",
            "--dry-run",
            "--format",
            "agent",
        ],
    );
    assert_eq!(preview["schema"], "aplan/agent-capture-action/v1");
    assert!(
        preview["agent_instruction"]
            .as_str()
            .unwrap()
            .contains("task id")
    );
    assert_eq!(preview["outcome"], "predicted");
    assert_eq!(preview["task_id"], "A-2");
    assert_eq!(preview["capture"]["promoted_to"], "A-2");
    assert_eq!(preview["files"].as_array().unwrap().len(), 3);
    assert_eq!(std::fs::read_to_string(&capture_path).unwrap(), open_bytes);
    assert!(!dir.path().join("aplan/A-2.md").exists());

    let promoted = capture_json(
        dir.path(),
        &[
            "inbox",
            "promote",
            capture_id,
            "--task-id",
            "A-2",
            "--phase",
            "1",
            "--cut",
            "M-next",
            "--effort",
            "3",
            "--owner",
            "delivery",
            "--path",
            "crates/**",
            "--tag",
            "explicit",
            "--reason",
            "triaged",
            "--format",
            "json",
        ],
    );
    assert_eq!(promoted["outcome"], "applied");
    assert_eq!(promoted["capture"]["state"], "promoted");
    assert_eq!(promoted["capture"]["title"], "Promoted work");
    assert_eq!(
        promoted["capture"]["body"],
        "## Why\n\nRetained intake detail.\n"
    );
    assert!(promoted["capture"]["disposed_at"].is_string());
    assert_eq!(promoted["capture"]["reason"], "triaged");
    assert_eq!(
        promoted["capture"]["promotion_fingerprint"]
            .as_str()
            .unwrap()
            .len(),
        64
    );

    let task_path = dir.path().join("aplan/A-2.md");
    let task = std::fs::read_to_string(&task_path).expect("promoted task");
    for field in [
        "id: A-2",
        "title: Promoted work",
        "effort_days: 3.0",
        "owner: delivery",
        "crates/**",
        "explicit",
        "Retained intake detail.",
    ] {
        assert!(task.contains(field), "task omitted `{field}`:\n{task}");
    }
    assert!(
        !task.contains("src/**"),
        "explicit paths did not win:\n{task}"
    );
    assert!(
        !task.contains("from-profile"),
        "explicit tags did not win:\n{task}"
    );
    let cut = std::fs::read_to_string(dir.path().join("aplan/cut.md")).unwrap();
    assert!(
        cut.contains("A-2"),
        "promoted task did not join its cut: {cut}"
    );

    let promoted_before_amend = std::fs::read_to_string(&capture_path).unwrap();
    let immutable = common::run(
        dir.path(),
        &["inbox", "amend", capture_id, "--title", "Changed origin"],
    );
    assert_eq!(immutable.code, 2, "{}", immutable.text());
    assert!(immutable.text().contains("only open captures"));
    assert_eq!(
        std::fs::read_to_string(&capture_path).unwrap(),
        promoted_before_amend
    );

    std::fs::write(&task_path, format!("{task}\nLater task-only edit.\n"))
        .expect("later task edit");
    let retry_preview = capture_json(
        dir.path(),
        &[
            "inbox",
            "promote",
            capture_id,
            "--task-id",
            "A-2",
            "--phase",
            "1",
            "--cut",
            "M-next",
            "--effort",
            "3",
            "--owner",
            "delivery",
            "--path",
            "crates/**",
            "--tag",
            "explicit",
            "--reason",
            "triaged",
            "--dry-run",
            "--format",
            "json",
        ],
    );
    assert_eq!(retry_preview["outcome"], "unchanged");
    assert_eq!(retry_preview["task_id"], "A-2");
    assert_eq!(retry_preview["files"], serde_json::json!([]));
    let retry = capture_json(
        dir.path(),
        &[
            "inbox",
            "promote",
            capture_id,
            "--task-id",
            "A-2",
            "--phase",
            "1",
            "--cut",
            "M-next",
            "--effort",
            "3",
            "--owner",
            "delivery",
            "--path",
            "crates/**",
            "--tag",
            "explicit",
            "--reason",
            "triaged",
            "--format",
            "json",
        ],
    );
    assert_eq!(retry["outcome"], "unchanged");
    assert_eq!(retry["task_id"], "A-2");
    assert_eq!(retry["files"], serde_json::json!([]));
    assert_eq!(
        aplan_core::load_plan(&dir.path().join("aplan"))
            .tasks
            .iter()
            .filter(|loaded| loaded.task.id == "A-2")
            .count(),
        1
    );

    let before_conflict = std::fs::read_to_string(&capture_path).unwrap();
    let conflicting = common::run(
        dir.path(),
        &[
            "inbox",
            "promote",
            capture_id,
            "--task-id",
            "A-2",
            "--effort",
            "4",
        ],
    );
    assert_eq!(conflicting.code, 2, "{}", conflicting.text());
    assert!(conflicting.text().contains("different request"));
    assert_eq!(
        std::fs::read_to_string(capture_path).unwrap(),
        before_conflict
    );

    let check = common::run(dir.path(), &["check"]);
    assert_eq!(check.code, 0, "{}", check.text());
}

#[test]
#[allow(clippy::too_many_lines)] // One refusal matrix proves every terminal and invalid state leaves all three files unchanged.
fn capture_promotion_refusals_leave_task_and_capture_unchanged() {
    let dir = promotion_repository();
    let cut_path = dir.path().join("aplan/cut.md");
    let cut_before = std::fs::read_to_string(&cut_path).unwrap();

    let collision_capture = capture_json(
        dir.path(),
        &[
            "capture",
            "Collision",
            "--profile",
            "triage",
            "--format",
            "json",
        ],
    );
    let collision_id = collision_capture["capture"]["id"].as_str().unwrap();
    let collision_path = dir.path().join(format!("aplan/.inbox/{collision_id}.yml"));
    let collision_before = std::fs::read_to_string(&collision_path).unwrap();
    let collision = common::run(
        dir.path(),
        &["inbox", "promote", collision_id, "--task-id", "A-1"],
    );
    assert_eq!(collision.code, 2, "{}", collision.text());
    assert!(collision.text().contains("already exists"));
    assert_eq!(
        std::fs::read_to_string(&collision_path).unwrap(),
        collision_before
    );
    assert_eq!(std::fs::read_to_string(&cut_path).unwrap(), cut_before);

    let invalid_capture = capture_json(
        dir.path(),
        &["capture", "Missing task fields", "--format", "json"],
    );
    let invalid_id = invalid_capture["capture"]["id"].as_str().unwrap();
    let invalid_path = dir.path().join(format!("aplan/.inbox/{invalid_id}.yml"));
    let invalid_before = std::fs::read_to_string(&invalid_path).unwrap();
    let denied_preview = common::run(
        dir.path(),
        &[
            "inbox",
            "promote",
            invalid_id,
            "--task-id",
            "A-3",
            "--clear-paths",
            "--clear-tags",
            "--dry-run",
            "--format",
            "json",
        ],
    );
    assert_eq!(denied_preview.code, 1, "{}", denied_preview.text());
    let denied_preview_json: serde_json::Value =
        serde_json::from_str(&denied_preview.stdout).unwrap();
    assert_eq!(denied_preview_json["outcome"], "refused");
    assert_eq!(denied_preview_json["publishable"], false);
    assert_eq!(
        std::fs::read_to_string(&invalid_path).unwrap(),
        invalid_before
    );
    assert!(!dir.path().join("aplan/A-3.md").exists());
    assert_eq!(std::fs::read_to_string(&cut_path).unwrap(), cut_before);
    let denied = common::run(
        dir.path(),
        &[
            "inbox",
            "promote",
            invalid_id,
            "--task-id",
            "A-3",
            "--clear-paths",
            "--clear-tags",
            "--format",
            "json",
        ],
    );
    assert_eq!(denied.code, 1, "{}", denied.text());
    let denied_json: serde_json::Value = serde_json::from_str(&denied.stdout).unwrap();
    assert_eq!(denied_json["outcome"], "refused");
    assert_eq!(denied_json["publishable"], false);
    assert!(
        denied_json["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| { line.as_str().is_some_and(|line| line.contains("APLAN022")) })
    );
    assert_eq!(
        std::fs::read_to_string(&invalid_path).unwrap(),
        invalid_before
    );
    assert!(!dir.path().join("aplan/A-3.md").exists());
    assert_eq!(std::fs::read_to_string(&cut_path).unwrap(), cut_before);

    let discarded = capture_json(
        dir.path(),
        &["inbox", "discard", invalid_id, "--format", "json"],
    );
    assert_eq!(discarded["capture"]["state"], "discarded");
    let discarded_before = std::fs::read_to_string(&invalid_path).unwrap();
    let terminal = common::run(
        dir.path(),
        &["inbox", "promote", invalid_id, "--task-id", "A-3"],
    );
    assert_eq!(terminal.code, 2, "{}", terminal.text());
    assert!(terminal.text().contains("only open captures"));
    assert_eq!(
        std::fs::read_to_string(invalid_path).unwrap(),
        discarded_before
    );
    assert!(!dir.path().join("aplan/A-3.md").exists());

    let malformed = dir.path().join("aplan/.inbox/broken.yml");
    std::fs::write(&malformed, "schema: [broken\n").expect("malformed inbox member");
    let collision_still_before = std::fs::read_to_string(&collision_path).unwrap();
    let refused_partial = common::run(
        dir.path(),
        &["inbox", "promote", collision_id, "--task-id", "A-4"],
    );
    assert_eq!(refused_partial.code, 2, "{}", refused_partial.text());
    assert!(refused_partial.text().contains("APLAN057"));
    assert_eq!(
        std::fs::read_to_string(collision_path).unwrap(),
        collision_still_before
    );
    assert!(!dir.path().join("aplan/A-4.md").exists());
}

#[test]
fn capture_promotion_refuses_invalid_selected_configuration_without_writes() {
    let dir = promotion_repository();
    let created = capture_json(
        dir.path(),
        &[
            "capture",
            "Configuration guarded",
            "--profile",
            "triage",
            "--format",
            "json",
        ],
    );
    let capture_id = created["capture"]["id"].as_str().unwrap();
    let capture_path = dir.path().join(format!("aplan/.inbox/{capture_id}.yml"));
    let capture_before = std::fs::read_to_string(&capture_path).unwrap();
    let cut_path = dir.path().join("aplan/cut.md");
    let cut_before = std::fs::read_to_string(&cut_path).unwrap();
    std::fs::write(dir.path().join(".aplan.yml"), "root: [unterminated\n")
        .expect("malformed selected config");

    let invalid_config = common::run(
        dir.path(),
        &["inbox", "promote", capture_id, "--task-id", "A-5"],
    );

    assert_eq!(invalid_config.code, 2, "{}", invalid_config.text());
    assert!(
        invalid_config
            .text()
            .contains("could not parse `.aplan.yml`"),
        "{}",
        invalid_config.text()
    );
    assert_eq!(
        std::fs::read_to_string(capture_path).unwrap(),
        capture_before
    );
    assert_eq!(std::fs::read_to_string(cut_path).unwrap(), cut_before);
    assert!(!dir.path().join("aplan/A-5.md").exists());
}

#[test]
fn generated_task_id_is_excluded_from_promotion_retry_identity() {
    let dir = promotion_repository();
    let created = capture_json(
        dir.path(),
        &[
            "capture",
            "Mint on promotion",
            "--profile",
            "triage",
            "--format",
            "json",
        ],
    );
    let capture_id = created["capture"]["id"].as_str().unwrap();
    let promoted = capture_json(
        dir.path(),
        &["inbox", "promote", capture_id, "--format", "json"],
    );
    let task_id = promoted["task_id"].as_str().expect("generated task id");
    assert_eq!(promoted["capture"]["promoted_to"], task_id);
    let task = std::fs::read_to_string(dir.path().join(format!("aplan/{task_id}.md")))
        .expect("task created from snapshotted defaults");
    for default in [
        "phase: 1",
        "effort_days: 2.0",
        "owner: intake",
        "src/**",
        "from-profile",
    ] {
        assert!(
            task.contains(default),
            "promoted task omitted profile default `{default}`:\n{task}"
        );
    }
    assert!(
        std::fs::read_to_string(dir.path().join("aplan/cut.md"))
            .unwrap()
            .contains(task_id),
        "profile-default cut did not gain the generated task"
    );

    let retried = capture_json(
        dir.path(),
        &["inbox", "promote", capture_id, "--format", "json"],
    );
    assert_eq!(retried["outcome"], "unchanged");
    assert_eq!(retried["task_id"], task_id);
    assert_eq!(retried["files"], serde_json::json!([]));
}

/// An amendment that changes no typed field is a no-op even when a human used
/// comments or non-canonical YAML presentation in the valid source record.
#[test]
fn no_op_amend_preserves_human_capture_source_bytes() {
    let dir = repository("root: aplan\n");
    let path = dir.path().join(format!("aplan/.inbox/{ID}.yml"));
    let source = format!(
        "# retained human context\nschema: aplan/capture/v1\nid: {ID}\ntitle: 'captured' # keep this spelling\nkind: task\nstate: open\ncaptured_at: '2026-01-01T00:00:00Z'\norigin: cli\n"
    );
    std::fs::write(&path, &source).expect("capture");

    let result = capture_json(dir.path(), &["inbox", "amend", ID, "--format", "json"]);
    assert_eq!(result["outcome"], "unchanged");
    assert_eq!(result["files"], serde_json::json!([]));
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

/// Body input has one bounded validation contract across argv, files and stdin.
#[test]
fn capture_body_sources_share_one_bounded_contract() {
    use std::io::Write as _;

    let dir = repository("root: aplan\n");
    let blank_file = dir.path().join("blank.txt");
    std::fs::write(&blank_file, " \n\t").expect("blank body");
    for args in [
        vec!["capture", "\u{200b}", "--format", "json"],
        vec!["capture", "title", "--body", "   "],
        vec![
            "capture",
            "title",
            "--body-file",
            blank_file.to_str().unwrap(),
        ],
    ] {
        let out = common::run(dir.path(), &args);
        assert_eq!(out.code, 2, "{args:?}: {}", out.text());
        assert!(
            out.text().contains("blank or invisible"),
            "{args:?}: {}",
            out.text()
        );
    }

    let valid_file = dir.path().join("body.txt");
    std::fs::write(&valid_file, "body from file\n").expect("file body");
    let from_file = capture_json(
        dir.path(),
        &[
            "capture",
            "file body",
            "--body-file",
            valid_file.to_str().unwrap(),
            "--format",
            "json",
        ],
    );
    assert_eq!(from_file["capture"]["body"], "body from file\n");

    let mut stdin_child =
        common::command(dir.path(), &["capture", "stdin blank", "--body-file", "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn stdin capture");
    stdin_child
        .stdin
        .take()
        .unwrap()
        .write_all(b" \n")
        .expect("write stdin");
    let stdin_out = stdin_child.wait_with_output().expect("stdin result");
    assert_eq!(stdin_out.status.code(), Some(2));

    let mut valid_stdin = common::command(
        dir.path(),
        &[
            "capture",
            "stdin body",
            "--body-file",
            "-",
            "--format",
            "json",
        ],
    )
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .spawn()
    .expect("spawn valid stdin capture");
    valid_stdin
        .stdin
        .take()
        .unwrap()
        .write_all(b"body from stdin")
        .expect("write valid stdin");
    let stdin_out = valid_stdin.wait_with_output().expect("valid stdin result");
    assert!(
        stdin_out.status.success(),
        "{}",
        String::from_utf8_lossy(&stdin_out.stderr)
    );
    let stdin_json: serde_json::Value = serde_json::from_slice(&stdin_out.stdout).unwrap();
    assert_eq!(stdin_json["capture"]["body"], "body from stdin");

    let dashed = common::run(
        dir.path(),
        &["capture", "--format", "json", "--", "-leading dash"],
    );
    assert_eq!(dashed.code, 0, "{}", dashed.text());
}

#[test]
fn capture_command_refusals_cover_parser_editor_and_stream_boundaries() {
    use std::io::Write as _;

    let dir = repository("root: aplan\n");
    let created = capture_json(dir.path(), &["capture", "seed", "--format", "json"]);
    let id = created["capture"]["id"].as_str().expect("capture id");

    for args in [
        vec!["capture", "bad source", "--source", "missing-separator"],
        vec!["inbox", "--state", "unknown"],
        vec!["inbox", "--revisit", "unknown"],
        vec![
            "inbox",
            "amend",
            id,
            "--clear-promotion",
            "--promotion-phase",
            "2",
        ],
    ] {
        let refused = common::run(dir.path(), &args);
        assert_eq!(refused.code, 2, "{args:?}: {}", refused.text());
    }

    for command in ["show", "amend", "discard", "restore"] {
        let refused = common::run(dir.path(), &["inbox", "--limit", "1", command, id]);
        assert_eq!(refused.code, 2, "{command}: {}", refused.text());
        assert!(
            refused.text().contains("list filters cannot be combined"),
            "{command}: {}",
            refused.text()
        );
    }

    let no_editor = common::command(dir.path(), &["capture", "no editor", "--edit"])
        .env_remove("VISUAL")
        .env_remove("EDITOR")
        .output()
        .expect("no-editor command");
    assert_eq!(no_editor.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&no_editor.stderr).contains("VISUAL"));

    let malformed_editor = common::command(dir.path(), &["capture", "bad editor", "--edit"])
        .env("VISUAL", "'unterminated")
        .env_remove("EDITOR")
        .output()
        .expect("malformed-editor command");
    assert_eq!(malformed_editor.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&malformed_editor.stderr).contains("parsing editor"));

    for (title, body, message) in [
        ("invalid UTF-8", vec![0xff, 0xfe], "not UTF-8"),
        (
            "oversized stdin",
            vec![b'x'; aplan_core::capture::MAX_CAPTURE_BYTES + 1],
            "exceeds",
        ),
    ] {
        let mut child = common::command(dir.path(), &["capture", title, "--body-file", "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("stdin refusal command");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(&body)
            .expect("write body");
        let output = child.wait_with_output().expect("stdin refusal");
        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Editor settings are argv, never shell source, and a failure or unchanged
/// edit cannot mutate the record.
#[test]
fn capture_editor_is_direct_and_atomic() {
    let dir = repository("root: aplan\n");

    let editor = dir.path().join("editor.sh");
    std::fs::write(
        &editor,
        "#!/bin/sh\n[ \"$1\" = --replace ] || exit 9\nprintf 'edited body' > \"$2\"\n",
    )
    .expect("editor");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let editor_setting = format!("{} --replace", editor.display());
    let edited = common::command(
        dir.path(),
        &["capture", "editor", "--edit", "--format", "json"],
    )
    .env_remove("VISUAL")
    .env("EDITOR", &editor_setting)
    .output()
    .expect("editor capture");
    assert!(
        edited.status.success(),
        "{}",
        String::from_utf8_lossy(&edited.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&edited.stdout).unwrap();
    assert_eq!(document["capture"]["body"], "edited body");
    let id = document["capture"]["id"].as_str().unwrap();
    let path = dir.path().join(format!("aplan/.inbox/{id}.yml"));
    let before = std::fs::read_to_string(&path).unwrap();

    let unchanged = dir.path().join("unchanged.sh");
    std::fs::write(&unchanged, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&unchanged, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let no_change = common::command(
        dir.path(),
        &["inbox", "amend", id, "--edit", "--format", "json"],
    )
    .env_remove("VISUAL")
    .env("EDITOR", &unchanged)
    .output()
    .unwrap();
    assert!(no_change.status.success());
    let no_change_json: serde_json::Value = serde_json::from_slice(&no_change.stdout).unwrap();
    assert_eq!(no_change_json["outcome"], "unchanged");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

    let failing = dir.path().join("failing.sh");
    std::fs::write(&failing, "#!/bin/sh\nprintf changed > \"$1\"\nexit 7\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&failing, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let failed = common::command(dir.path(), &["inbox", "amend", id, "--edit"])
        .env_remove("VISUAL")
        .env("EDITOR", &failing)
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(2));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

    let discarded = common::run(dir.path(), &["inbox", "discard", id]);
    assert_eq!(discarded.code, 0, "{}", discarded.text());
    let marker = dir.path().join("editor-ran");
    let marker_editor = dir.path().join("marker-editor.sh");
    std::fs::write(&marker_editor, "#!/bin/sh\nprintf ran > \"$1\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&marker_editor, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let marker_setting = format!("{} {}", marker_editor.display(), marker.display());
    let closed = common::command(dir.path(), &["inbox", "amend", id, "--edit"])
        .env_remove("VISUAL")
        .env("EDITOR", &marker_setting)
        .output()
        .unwrap();
    assert_eq!(closed.status.code(), Some(2));
    assert!(
        !marker.exists(),
        "a refused closed-state amend launched the editor"
    );

    let restored = common::run(dir.path(), &["inbox", "restore", id]);
    assert_eq!(restored.code, 0, "{}", restored.text());
    let invalid = common::command(
        dir.path(),
        &["inbox", "amend", id, "--title", "\u{200b}", "--edit"],
    )
    .env_remove("VISUAL")
    .env("EDITOR", &marker_setting)
    .output()
    .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(
        !marker.exists(),
        "an amendment with invalid deterministic input launched the editor"
    );
}

#[test]
fn capture_profiles_and_promotion_defaults_are_snapshotted_and_amendable() {
    let dir = repository(
        "root: aplan\ncapture:\n  kinds: [question]\n  profiles:\n    triage:\n      kind: question\n      labels: [inbox]\n      body: scaffold\n      promotion:\n        phase: 2\n        cut: M-next\n        paths: [src/**]\n        tags: [feature]\n",
    );
    let created = capture_json(
        dir.path(),
        &[
            "capture",
            "profiled",
            "--profile",
            "triage",
            "--promotion-owner",
            "team",
            "--format",
            "json",
        ],
    );
    let capture = &created["capture"];
    assert_eq!(capture["kind"], "question");
    assert_eq!(capture["body"], "scaffold");
    assert_eq!(capture["labels"], serde_json::json!(["inbox"]));
    assert_eq!(capture["profile"], "triage");
    assert_eq!(capture["promotion_defaults"]["phase"], 2);
    assert_eq!(capture["promotion_defaults"]["cut"], "M-next");
    assert_eq!(capture["promotion_defaults"]["owner"], "team");
    assert_eq!(
        capture["promotion_defaults"]["paths"],
        serde_json::json!(["src/**"])
    );
    let id = capture["id"].as_str().unwrap();

    let amended = capture_json(
        dir.path(),
        &[
            "inbox",
            "amend",
            id,
            "--promotion-effort",
            "3",
            "--format",
            "json",
        ],
    );
    let defaults = &amended["capture"]["promotion_defaults"];
    assert_eq!(defaults["phase"], 2);
    assert_eq!(defaults["cut"], "M-next");
    assert_eq!(defaults["owner"], "team");
    assert_eq!(defaults["effort_days"], 3.0);
    assert_eq!(defaults["paths"], serde_json::json!(["src/**"]));

    let focused = common::run(dir.path(), &["inbox", "show", id]);
    assert_eq!(focused.code, 0, "{}", focused.text());
    for field in [
        "schema: aplan/capture/v1",
        "profile: triage",
        "promotion phase: 2",
        "promotion cut: M-next",
        "promotion effort: 3",
        "promotion owner: team",
        "promotion paths: src/**",
        "promotion tags: feature",
    ] {
        assert!(
            focused.stdout.contains(field),
            "focused human output omitted `{field}`: {}",
            focused.stdout
        );
    }

    let cleared = capture_json(
        dir.path(),
        &[
            "inbox",
            "amend",
            id,
            "--clear-promotion",
            "--format",
            "json",
        ],
    );
    assert!(cleared["capture"].get("promotion_defaults").is_none());
}

/// One source identity is one retained record under contention. This is the
/// directory-wide guard case: every writer has a different ULID path, so a
/// file-only compare-and-swap would let all of them publish.
#[test]
fn concurrent_same_source_capture_has_one_idempotent_result() {
    let dir = repository("root: aplan\n");
    let mut children = Vec::new();
    for _ in 0..8 {
        children.push(
            common::command(
                dir.path(),
                &[
                    "capture",
                    "same intake",
                    "--source",
                    "race:one-key",
                    "--format",
                    "json",
                ],
            )
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn capture"),
        );
    }
    let mut ids = std::collections::BTreeSet::new();
    for child in children {
        let output = child.wait_with_output().expect("capture result");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        ids.insert(value["capture"]["id"].as_str().unwrap().to_string());
    }
    assert_eq!(ids.len(), 1, "same source produced {ids:?}");
    assert_eq!(
        std::fs::read_dir(dir.path().join("aplan/.inbox"))
            .unwrap()
            .count(),
        1
    );

    let mut unique = Vec::new();
    for index in 0..8 {
        unique.push(
            common::command(
                dir.path(),
                &[
                    "capture",
                    &format!("independent {index}"),
                    "--format",
                    "json",
                ],
            )
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn independent capture"),
        );
    }
    let mut unique_ids = std::collections::BTreeSet::new();
    for child in unique {
        let output = child
            .wait_with_output()
            .expect("independent capture result");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        unique_ids.insert(value["capture"]["id"].as_str().unwrap().to_string());
    }
    assert_eq!(unique_ids.len(), 8);
    assert_eq!(
        std::fs::read_dir(dir.path().join("aplan/.inbox"))
            .unwrap()
            .count(),
        9
    );
}

/// Human output makes hostile direction controls visible, while machine output
/// is structurally identical under every color policy except for its injected
/// operation instant.
#[test]
fn capture_output_is_sanitized_and_machine_color_invariant() {
    let dir = repository("root: aplan\n");
    let body = format!(
        "schema: aplan/capture/v1\nid: {ID}\ntitle: \"safe\\u202etail\"\nkind: task\nstate: open\ncaptured_at: '2026-01-01T00:00:00Z'\norigin: cli\n"
    );
    std::fs::write(dir.path().join(format!("aplan/.inbox/{ID}.yml")), body).unwrap();

    let human = common::run(dir.path(), &["inbox"]);
    assert_eq!(human.code, 0, "{}", human.text());
    assert!(!human.stdout.contains('\u{202e}'), "{}", human.stdout);
    assert!(human.stdout.contains('\u{fffd}'), "{}", human.stdout);

    let always = common::run(dir.path(), &["--color=always", "inbox", "--format", "json"]);
    let never = common::run(dir.path(), &["--color=never", "inbox", "--format", "json"]);
    assert_eq!(always.code, 0, "{}", always.text());
    assert_eq!(never.code, 0, "{}", never.text());
    let mut always_json: serde_json::Value =
        serde_json::from_str(&always.stdout).expect("always-colour JSON");
    let mut never_json: serde_json::Value =
        serde_json::from_str(&never.stdout).expect("never-colour JSON");
    for document in [&mut always_json, &mut never_json] {
        let evaluated_at = document["evaluated_at"]
            .as_str()
            .expect("list evaluation instant");
        assert!(evaluated_at.ends_with('Z'), "{evaluated_at}");
        document["evaluated_at"] = serde_json::Value::String("<clock>".into());
    }
    assert_eq!(always_json, never_json);
    assert!(!always.stdout.contains("\u{1b}["));
}
