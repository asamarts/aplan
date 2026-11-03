//! Historical plan reporting (A1-194).
//!
//! A revision selects the committed plan and, under ordinary discovery, the
//! repository configuration beside it. It does not select a clock: --as-of
//! remains the explicit evaluation-time override for age reports.

use std::path::Path;

use crate::common;

fn init(root: &Path) {
    common::git(root, &["init", "-q"]);
    common::git(root, &["config", "user.email", "fixture@example.com"]);
    common::git(root, &["config", "user.name", "Fixture"]);
}

fn commit(root: &Path, message: &str) -> String {
    common::git(root, &["add", "."]);
    common::git(root, &["commit", "-q", "-m", message]);
    let out = common::git(root, &["rev-parse", "HEAD"]);
    String::from_utf8(out.stdout)
        .expect("sha is utf-8")
        .trim()
        .to_string()
}

fn write_task(root: &Path, phase: u32, status: &str, effort: u32, tags: &str) {
    write_task_in(&root.join("aplan"), phase, status, effort, tags);
}

fn write_task_in(plan: &Path, phase: u32, status: &str, effort: u32, tags: &str) {
    std::fs::create_dir_all(plan.join("tasks")).expect("plan directory");
    std::fs::write(
        plan.join("tasks/T-1.md"),
        format!(
            "---\nid: T-1\ntitle: Historical task\nphase: {phase}\nstatus: {status}\n\
             effort_days: {effort}\ntags: [{tags}]\n---\n\n## Why\n\nFixture.\n"
        ),
    )
    .expect("task");
}

/// Neither the current checkout nor the current config is an input to a
/// revision-qualified answer. This also covers a plan root that existed at the
/// selected commit but has since been removed, so implementation by reading the
/// old config and then walking the current filesystem cannot pass.
#[test]
fn a_historical_root_and_config_do_not_need_to_exist_in_the_worktree() {
    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: old-plan\n").expect("old config");
    write_task_in(&dir.path().join("old-plan"), 4, "todo", 3, "");
    let old = commit(dir.path(), "old rooted plan");

    std::fs::remove_dir_all(dir.path().join("old-plan")).expect("remove old plan");
    std::fs::write(dir.path().join(".aplan.yml"), "root: [not valid\n")
        .expect("broken current config");
    commit(dir.path(), "current checkout cannot load");

    let run = common::run(dir.path(), &["summary", "--format", "json", "--at", &old]);
    assert_eq!(run.code, 0, "{}", run.text());
    let doc: serde_json::Value = serde_json::from_str(&run.stdout).expect("json");
    assert_eq!(doc["phases"][0]["phase"], 4);
    assert_eq!(doc["total_effort_days"], 3.0);
}

/// A committed root name is interpreted in the selected tree. Following the
/// current checkout's symlink first would make an old answer depend on today's
/// unrelated target and can select a directory that did not exist then.
#[cfg(unix)]
#[test]
fn a_historical_plan_root_does_not_follow_todays_symlink() {
    use std::os::unix::fs::symlink;

    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: plan-link\n").expect("config");
    write_task_in(&dir.path().join("old-plan"), 2, "todo", 2, "");
    symlink("old-plan", dir.path().join("plan-link")).expect("old link");
    let old = commit(dir.path(), "old linked plan");

    std::fs::remove_file(dir.path().join("plan-link")).expect("remove link");
    write_task_in(&dir.path().join("new-plan"), 9, "todo", 9, "");
    symlink("new-plan", dir.path().join("plan-link")).expect("new link");
    commit(dir.path(), "move linked plan");

    let run = common::run(dir.path(), &["summary", "--format", "json", "--at", &old]);
    assert_eq!(run.code, 0, "{}", run.text());
    let doc: serde_json::Value = serde_json::from_str(&run.stdout).expect("json");
    assert_eq!(doc["phases"][0]["phase"], 2, "{doc}");
}

/// A repository-root target normalises to the empty repository-relative path.
/// That is the top tree, not an absent path (`A1-498`).
/// Proves `summary.revision_uses_historical_repository_boundary`.
#[cfg(unix)]
#[test]
fn a_committed_plan_root_symlink_can_resolve_to_the_repository_root() {
    use std::os::unix::fs::symlink;

    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: plan-link\n").expect("config");
    write_task_in(dir.path(), 6, "todo", 4, "");
    symlink(".", dir.path().join("plan-link")).expect("root link");
    commit(dir.path(), "repository-root plan link");

    let run = common::run(dir.path(), &["summary", "--format", "json", "--at", "HEAD"]);
    assert_eq!(run.code, 0, "{}", run.text());
    let doc: serde_json::Value = serde_json::from_str(&run.stdout).expect("json");
    assert_eq!(doc["phases"][0]["phase"], 6, "{doc}");
    assert_eq!(doc["total_effort_days"], 4.0, "{doc}");
}

/// A link is present historical input even when its target is not. Calling it
/// an absent plan root hides the invalid committed input that was actually
/// selected (`A1-498`).
/// Proves `summary.revision_uses_historical_repository_boundary`.
#[cfg(unix)]
#[test]
fn a_broken_committed_plan_root_symlink_is_named_as_invalid_input() {
    use std::os::unix::fs::symlink;

    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: plan-link\n").expect("config");
    symlink("missing-plan", dir.path().join("plan-link")).expect("broken root link");
    commit(dir.path(), "broken plan-root link");

    let run = common::run(dir.path(), &["summary", "--at", "HEAD"]);
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains("symbolic link"), "{}", run.stderr);
    assert!(run.stderr.contains("plan-link"), "{}", run.stderr);
    assert!(!run.stderr.contains("does not contain"), "{}", run.stderr);
}

/// The explicit root chooses a path inside the current repository. Its current
/// symlink target must not choose an entirely different repository before the
/// selected commit gets a chance to resolve the old link.
#[cfg(unix)]
#[test]
fn an_explicit_historical_root_does_not_follow_todays_repository_symlink() {
    use std::os::unix::fs::symlink;

    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: ignored\n").expect("config");
    write_task_in(&dir.path().join("old-plan"), 2, "todo", 2, "");
    symlink("old-plan", dir.path().join("plan-link")).expect("old link");
    let old = commit(dir.path(), "old linked plan");

    let other = fixture();
    write_task_in(&other.path().join("plan"), 9, "todo", 9, "");
    commit(other.path(), "other repository");
    std::fs::remove_file(dir.path().join("plan-link")).expect("remove link");
    symlink(other.path(), dir.path().join("plan-link")).expect("cross-repository link");

    let run = common::run(
        dir.path(),
        &[
            "--root",
            "plan-link",
            "summary",
            "--format",
            "json",
            "--at",
            &old,
        ],
    );
    assert_eq!(run.code, 0, "{}", run.text());
    let doc: serde_json::Value = serde_json::from_str(&run.stdout).expect("json");
    assert_eq!(doc["phases"][0]["phase"], 2);
}

/// A directory can become a nested repository after the selected outer commit.
/// Repository ownership for an explicit historical root is established from
/// the invocation before today's nested marker is interpreted, just as today's
/// symlink target is not allowed to choose the history (`A1-498`).
/// Proves `summary.revision_uses_historical_repository_boundary`.
#[test]
fn todays_nested_repository_cannot_capture_an_outer_historical_root() {
    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: historical-plan\n").expect("config");
    let root = dir.path().join("historical-plan");
    write_task_in(&root, 2, "todo", 2, "");
    commit(dir.path(), "outer historical plan");
    common::git(dir.path(), &["tag", "historical"]);

    std::fs::remove_dir_all(&root).expect("replace historical root");
    std::fs::create_dir(&root).expect("nested repository root");
    init(&root);
    write_task_in(&root, 9, "todo", 9, "");
    commit(&root, "nested current plan");
    common::git(&root, &["tag", "historical"]);

    let run = common::run(
        dir.path(),
        &[
            "--root",
            "historical-plan",
            "summary",
            "--format",
            "json",
            "--at",
            "historical",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.text());
    let doc: serde_json::Value = serde_json::from_str(&run.stdout).expect("json");
    assert_eq!(doc["phases"][0]["phase"], 2, "{doc}");
    assert_eq!(doc["total_effort_days"], 2.0, "{doc}");
}

/// An explicit root outside the invocation repository deliberately selects the
/// repository that owns that path. The nested-root correction must not turn
/// the current directory into an implicit global `--repo`, and ADR-0047's
/// explicit policy boundary still applies (`A1-498`).
/// Proves `summary.revision_uses_historical_repository_boundary`.
#[test]
fn an_external_historical_root_selects_its_repository_after_policy_is_explicit() {
    let caller = fixture();
    std::fs::write(caller.path().join("README.md"), "caller\n").expect("caller marker");
    commit(caller.path(), "caller repository");

    let other = fixture();
    let root = other.path().join("plan");
    write_task_in(&root, 7, "todo", 3, "");
    commit(other.path(), "external plan");
    common::git(other.path(), &["tag", "historical"]);

    let refused = common::run(
        caller.path(),
        &[
            "--root",
            root.to_str().expect("utf-8 fixture path"),
            "summary",
            "--at",
            "historical",
        ],
    );
    assert_eq!(refused.code, 2, "{}", refused.text());
    assert!(refused.stdout.is_empty(), "{}", refused.stdout);
    assert!(
        refused.stderr.contains("outside repository"),
        "{}",
        refused.stderr
    );
    assert!(refused.stderr.contains("--config"), "{}", refused.stderr);
    assert!(refused.stderr.contains("--no-config"), "{}", refused.stderr);

    let run = common::run(
        caller.path(),
        &[
            "--root",
            root.to_str().expect("utf-8 fixture path"),
            "--no-config",
            "summary",
            "--format",
            "json",
            "--at",
            "historical",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.text());
    let doc: serde_json::Value = serde_json::from_str(&run.stdout).expect("json");
    assert_eq!(doc["phases"][0]["phase"], 7, "{doc}");
    assert_eq!(doc["total_effort_days"], 3.0, "{doc}");
    assert!(run.stderr.contains("--no-config"), "{}", run.stderr);
}

/// Repository discovery deliberately accepts an external repository reached
/// through a symlink alias. Converting the selected root back to a repository
/// path must not canonicalise only the worktree side and then reject the alias
/// it just accepted (`A1-498`).
/// Proves `summary.revision_uses_historical_repository_boundary`.
#[cfg(unix)]
#[test]
fn an_external_repository_symlink_alias_remains_addressable() {
    use std::os::unix::fs::symlink;

    let caller = fixture();
    std::fs::write(caller.path().join("README.md"), "caller\n").expect("caller marker");
    commit(caller.path(), "caller repository");

    let other = fixture();
    write_task_in(&other.path().join("plan"), 8, "todo", 5, "");
    commit(other.path(), "aliased external plan");

    let aliases = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("alias parent");
    let alias = aliases.path().join("repository");
    symlink(other.path(), &alias).expect("repository alias");
    let root = alias.join("plan");

    let run = common::run(
        caller.path(),
        &[
            "--root",
            root.to_str().expect("utf-8 fixture path"),
            "--no-config",
            "summary",
            "--format",
            "json",
            "--at",
            "HEAD",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.text());
    let doc: serde_json::Value = serde_json::from_str(&run.stdout).expect("json");
    assert_eq!(doc["phases"][0]["phase"], 8, "{doc}");
    assert_eq!(doc["total_effort_days"], 5.0, "{doc}");
}

/// A caller policy is current state and cannot be attributed to the selected
/// repository's commit. The refusal must name both safe explicit choices, and
/// each choice must actually work (`A1-498`).
/// Proves `summary.revision_uses_historical_repository_boundary`.
#[test]
fn an_external_root_refuses_a_discovered_caller_policy_unless_selected_or_declined() {
    let caller = fixture();
    std::fs::write(
        caller.path().join(".aplan.yml"),
        "tag_families:\n  kind: [external]\nsummary:\n  group_by: kind\n",
    )
    .expect("caller config");
    commit(caller.path(), "caller policy");

    let other = fixture();
    let root = other.path().join("plan");
    write_task_in(&root, 7, "todo", 3, "external");
    commit(other.path(), "external plan");

    let run = common::run(
        caller.path(),
        &[
            "--root",
            root.to_str().expect("utf-8 fixture path"),
            "summary",
            "--at",
            "HEAD",
        ],
    );
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains(".aplan.yml"), "{}", run.stderr);
    assert!(run.stderr.contains("outside repository"), "{}", run.stderr);
    assert!(run.stderr.contains("--config"), "{}", run.stderr);
    assert!(run.stderr.contains("--no-config"), "{}", run.stderr);

    let declined = common::run(
        caller.path(),
        &[
            "--root",
            root.to_str().expect("utf-8 fixture path"),
            "--no-config",
            "summary",
            "--format",
            "json",
            "--at",
            "HEAD",
        ],
    );
    assert_eq!(declined.code, 0, "{}", declined.text());
    let declined_doc: serde_json::Value =
        serde_json::from_str(&declined.stdout).expect("declined json");
    assert_eq!(declined_doc["schema"], "aplan/summary/v2");
    assert!(
        declined.stderr.contains("--no-config"),
        "{}",
        declined.stderr
    );

    let selected = common::run(
        caller.path(),
        &[
            "--root",
            root.to_str().expect("utf-8 fixture path"),
            "--config",
            ".aplan.yml",
            "summary",
            "--format",
            "json",
            "--at",
            "HEAD",
        ],
    );
    assert_eq!(selected.code, 0, "{}", selected.text());
    let selected_doc: serde_json::Value =
        serde_json::from_str(&selected.stdout).expect("selected json");
    assert_eq!(selected_doc["schema"], "aplan/summary-grouped/v1");
    assert_eq!(selected_doc["groups"][0]["group"], "external");
}

/// The filesystem loader follows a file symlink when the plan entry itself is
/// inside the root. A committed snapshot must follow the selected tree's link
/// to the selected tree's blob too, including a target elsewhere in the repo.
#[cfg(unix)]
#[test]
fn a_committed_task_symlink_matches_the_clean_worktree_loader() {
    use std::os::unix::fs::symlink;

    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    write_task_in(&dir.path().join("shared"), 6, "todo", 4, "");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan directory");
    symlink(
        "../../shared/tasks/T-1.md",
        dir.path().join("aplan/tasks/T-1.md"),
    )
    .expect("task link");
    commit(dir.path(), "linked task");

    let ordinary = common::run(dir.path(), &["summary", "--format", "json"]);
    let historical = common::run(dir.path(), &["summary", "--format", "json", "--at", "HEAD"]);
    assert_eq!(ordinary.code, 0, "{}", ordinary.text());
    assert_eq!(historical.code, 0, "{}", historical.text());
    assert_eq!(historical.stdout, ordinary.stdout);
    let doc: serde_json::Value = serde_json::from_str(&historical.stdout).expect("json");
    assert_eq!(doc["phases"][0]["phase"], 6);
}

/// Config discovery sees a `.aplan.yml` path, and the filesystem reader follows
/// it. The immutable reader must resolve the link inside the selected tree
/// rather than trying to parse the link target's pathname as YAML.
#[cfg(unix)]
#[test]
fn a_committed_config_symlink_matches_the_clean_worktree_loader() {
    use std::os::unix::fs::symlink;

    let dir = fixture();
    std::fs::create_dir_all(dir.path().join("config")).expect("config directory");
    std::fs::write(
        dir.path().join("config/plan.yml"),
        "root: aplan\ntag_families:\n  kind: [linked]\nsummary:\n  group_by: kind\n",
    )
    .expect("config target");
    symlink("config/plan.yml", dir.path().join(".aplan.yml")).expect("config link");
    write_task(dir.path(), 1, "todo", 1, "linked");
    commit(dir.path(), "linked config");

    let ordinary = common::run(dir.path(), &["summary", "--format", "json"]);
    let historical = common::run(dir.path(), &["summary", "--format", "json", "--at", "HEAD"]);
    assert_eq!(ordinary.code, 0, "{}", ordinary.text());
    assert_eq!(historical.code, 0, "{}", historical.text());
    assert_eq!(historical.stdout, ordinary.stdout);
}

/// Git permits non-UTF-8 filenames on Unix, while task contents and ids remain
/// UTF-8. The snapshot loader takes `PathBuf`, so the repository seam must not
/// silently discard a task solely because its filename is not text.
#[cfg(unix)]
#[test]
fn a_non_utf8_task_filename_is_not_lost_from_a_committed_plan() {
    use std::os::unix::ffi::OsStringExt;

    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    let tasks = dir.path().join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("tasks");
    let name = std::ffi::OsString::from_vec(vec![0xff, b'.', b'm', b'd']);
    std::fs::write(
        tasks.join(name),
        "---\nid: T-BYTES\ntitle: Byte path\nphase: 8\nstatus: todo\neffort_days: 1\n---\n",
    )
    .expect("task");
    let byte_dir = tasks.join(std::ffi::OsString::from_vec(vec![b'd', 0xfe]));
    std::fs::create_dir(&byte_dir).expect("byte directory");
    std::fs::write(
        byte_dir.join("T-NESTED.md"),
        "---\nid: T-NESTED\ntitle: Nested byte path\nphase: 8\nstatus: todo\neffort_days: 1\n---\n",
    )
    .expect("nested task");
    commit(dir.path(), "byte path");

    let ordinary = common::run(dir.path(), &["summary", "--format", "json"]);
    let historical = common::run(dir.path(), &["summary", "--format", "json", "--at", "HEAD"]);
    assert_eq!(ordinary.code, 0, "{}", ordinary.text());
    assert_eq!(historical.code, 0, "{}", historical.text());
    assert_eq!(historical.stdout, ordinary.stdout);
}

/// A config in the selected tree is input, not decoration: malformed bytes are
/// refused by name rather than falling back to today's config or to defaults.
#[test]
fn invalid_historical_config_is_refused_as_that_revision() {
    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), [0xff, b'\n']).expect("invalid config");
    write_task(dir.path(), 1, "todo", 1, "");
    let broken = commit(dir.path(), "invalid historical config");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("fixed config");
    commit(dir.path(), "fixed config");

    let run = common::run(dir.path(), &["summary", "--at", &broken]);
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains(".aplan.yml"), "{}", run.stderr);
    assert!(
        run.stderr.to_ascii_lowercase().contains("utf-8"),
        "{}",
        run.stderr
    );
}

#[test]
fn a_non_file_historical_config_is_refused_not_treated_as_absent() {
    let dir = fixture();
    std::fs::create_dir(dir.path().join(".aplan.yml")).expect("config directory");
    std::fs::write(dir.path().join(".aplan.yml/inside"), "not configuration\n")
        .expect("directory member");
    write_task(dir.path(), 1, "todo", 1, "");
    let broken = commit(dir.path(), "config is a directory");
    std::fs::remove_dir_all(dir.path().join(".aplan.yml")).expect("remove directory");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("fixed config");
    commit(dir.path(), "config is a file");

    let run = common::run(dir.path(), &["summary", "--at", &broken]);
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains(".aplan.yml"), "{}", run.stderr);
    assert!(run.stderr.contains("not a file"), "{}", run.stderr);
}

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    init(dir.path());
    dir
}

/// The data answer at HEAD is byte-identical to the ordinary worktree answer
/// on a clean checkout. The diagnostic stream names the resolved commit without
/// changing either human or JSON schemas.
#[test]
fn summary_at_head_is_identical_to_the_clean_worktree_answer() {
    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    write_task(dir.path(), 3, "todo", 2, "");
    let head = commit(dir.path(), "plan");

    let ordinary = common::run(dir.path(), &["summary", "--format", "json"]);
    let historical = common::run(dir.path(), &["summary", "--format", "json", "--at", "HEAD"]);
    assert_eq!(ordinary.code, 0, "{}", ordinary.text());
    assert_eq!(historical.code, 0, "{}", historical.text());
    assert_eq!(historical.stdout, ordinary.stdout);
    assert!(
        historical.stderr.contains(&head),
        "the revision-qualified answer names its resolved commit: {}",
        historical.stderr
    );
}

/// The revision owns both inputs that affect a discovered summary: task files
/// and the grouping/throughput configuration committed beside them.
/// Proves `summary.revision_selects_committed_inputs`.
#[test]
fn summary_at_a_revision_reads_its_plan_and_discovered_view_config() {
    let dir = fixture();
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\neffort_days_per_week: 2\ntag_families:\n  kind: [kind-old]\n\
         summary:\n  group_by: kind\n",
    )
    .expect("old config");
    write_task(dir.path(), 1, "done", 2, "kind-old, untestable");
    let old = commit(dir.path(), "old plan");

    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\neffort_days_per_week: 1\n",
    )
    .expect("current config");
    write_task(dir.path(), 9, "todo", 5, "");
    commit(dir.path(), "current plan");

    let current = common::run(dir.path(), &["summary", "--format", "json"]);
    assert_eq!(current.code, 0, "{}", current.text());
    let current_doc: serde_json::Value = serde_json::from_str(&current.stdout).expect("json");
    assert_eq!(current_doc["schema"], "aplan/summary/v2");
    assert_eq!(current_doc["phases"][0]["phase"], 9);
    assert_eq!(current_doc["total_effort_days"], 5.0);

    let historical = common::run(dir.path(), &["summary", "--format", "json", "--at", &old]);
    assert_eq!(historical.code, 0, "{}", historical.text());
    let old_doc: serde_json::Value = serde_json::from_str(&historical.stdout).expect("json");
    assert_eq!(old_doc["schema"], "aplan/summary-grouped/v1");
    assert_eq!(old_doc["group_by"], "kind");
    assert_eq!(old_doc["groups"][0]["group"], "kind-old");
    assert_eq!(old_doc["total_done"], 1);
    assert_eq!(old_doc["total_effort_days"], 2.0);
    assert_eq!(old_doc["total_weeks"], 1.0);
}

#[test]
fn a_revision_whose_plan_does_not_load_is_refused_with_the_reason() {
    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan directory");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: [not closed\n---\n",
    )
    .expect("broken task");
    let broken = commit(dir.path(), "broken plan");
    write_task(dir.path(), 1, "todo", 1, "");
    commit(dir.path(), "fixed plan");

    let run = common::run(dir.path(), &["summary", "--at", &broken]);
    assert_eq!(run.code, 1, "{}", run.text());
    assert!(run.text().contains("T-1.md"), "{}", run.text());
    assert!(
        run.text().contains("invalid YAML frontmatter"),
        "{}",
        run.text()
    );
}

#[test]
fn an_unknown_revision_and_a_missing_plan_root_are_refused_not_reported_empty() {
    let dir = fixture();
    std::fs::write(dir.path().join("README.md"), "before the plan\n").expect("readme");
    let before_plan = commit(dir.path(), "before plan");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    write_task(dir.path(), 1, "todo", 1, "");
    commit(dir.path(), "add plan");

    let unknown = common::run(dir.path(), &["summary", "--at", "no-such-revision"]);
    assert_eq!(unknown.code, 2, "{}", unknown.text());
    assert!(
        unknown.text().contains("no-such-revision"),
        "{}",
        unknown.text()
    );
    assert!(unknown.stdout.is_empty(), "{}", unknown.stdout);

    let absent = common::run(dir.path(), &["summary", "--at", &before_plan]);
    assert_eq!(absent.code, 2, "{}", absent.text());
    assert!(absent.text().contains("aplan"), "{}", absent.text());
    assert!(
        absent.text().contains("does not contain"),
        "{}",
        absent.text()
    );
    assert!(absent.stdout.is_empty(), "{}", absent.stdout);
}

#[test]
fn an_existing_root_with_no_task_candidates_is_an_empty_plan() {
    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::create_dir(dir.path().join("aplan")).expect("plan root");
    std::fs::write(dir.path().join("aplan/README.txt"), "no tasks yet\n").expect("marker");
    commit(dir.path(), "empty plan root");

    let run = common::run(dir.path(), &["summary", "--format", "json", "--at", "HEAD"]);
    assert_eq!(run.code, 0, "{}", run.text());
    let doc: serde_json::Value = serde_json::from_str(&run.stdout).expect("json");
    assert_eq!(doc["total_tasks"], 0);
    assert_eq!(doc["phases"], serde_json::json!([]));
}

/// `load_plan` prunes a root-level build directory before inspecting anything
/// below it. A Git tree has to make the same decision before a gitlink is
/// interpreted as an unavailable part of the plan.
#[test]
fn an_ignored_build_directory_cannot_make_the_revision_loader_stricter() {
    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    write_task(dir.path(), 1, "todo", 1, "");
    let head = commit(dir.path(), "plan");
    common::git(
        dir.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{head},aplan/target"),
        ],
    );
    common::git(dir.path(), &["commit", "-q", "-m", "ignored build gitlink"]);

    let run = common::run(dir.path(), &["summary", "--format", "json", "--at", "HEAD"]);
    assert_eq!(run.code, 0, "{}", run.text());
    let doc: serde_json::Value = serde_json::from_str(&run.stdout).expect("json");
    assert_eq!(doc["total_tasks"], 1);
}

#[test]
fn a_submodule_inside_the_plan_is_refused_as_incomplete_input() {
    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    write_task(dir.path(), 1, "todo", 1, "");
    let head = commit(dir.path(), "plan");
    common::git(
        dir.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{head},aplan/tasks/external"),
        ],
    );
    common::git(dir.path(), &["commit", "-q", "-m", "plan submodule"]);

    let run = common::run(dir.path(), &["summary", "--at", "HEAD"]);
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains("submodule"), "{}", run.stderr);
    assert!(run.stderr.contains("tasks/external"), "{}", run.stderr);
}

/// Proves `summary.revision_uses_historical_repository_boundary`.
#[test]
fn a_submodule_as_the_plan_root_is_refused_as_incomplete_input() {
    let dir = fixture();
    std::fs::write(dir.path().join("README.md"), "repository\n").expect("marker");
    let head = commit(dir.path(), "repository");
    common::git(
        dir.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{head},aplan"),
        ],
    );
    common::git(dir.path(), &["commit", "-q", "-m", "plan root gitlink"]);

    let run = common::run(dir.path(), &["summary", "--at", "HEAD"]);
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains("submodule"), "{}", run.stderr);
    assert!(run.stderr.contains("aplan"), "{}", run.stderr);
    assert!(!run.stderr.contains("does not contain"), "{}", run.stderr);
}

#[cfg(unix)]
#[test]
fn a_symlinked_plan_root_resolving_to_a_submodule_is_refused() {
    use std::os::unix::fs::symlink;

    let dir = fixture();
    std::fs::write(dir.path().join("README.md"), "repository\n").expect("marker");
    let head = commit(dir.path(), "repository");
    std::fs::write(dir.path().join(".aplan.yml"), "root: plan-link\n").expect("config");
    symlink("vendor-plan", dir.path().join("plan-link")).expect("plan link");
    common::git(dir.path(), &["add", ".aplan.yml", "plan-link"]);
    common::git(
        dir.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{head},vendor-plan"),
        ],
    );
    common::git(
        dir.path(),
        &["commit", "-q", "-m", "linked plan root gitlink"],
    );

    let run = common::run(dir.path(), &["summary", "--at", "HEAD"]);
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains("submodule"), "{}", run.stderr);
    assert!(run.stderr.contains("vendor-plan"), "{}", run.stderr);
}

/// An explicitly selected config is a trust boundary, not repository content.
/// Revision discovery must not replace it with an older in-tree policy.
/// Proves `summary.revision_selects_committed_inputs`.
#[test]
fn an_explicit_config_remains_the_selected_policy_for_a_historical_plan() {
    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("repo config");
    write_task(dir.path(), 1, "todo", 1, "trusted-kind");
    let old = commit(dir.path(), "plan");
    std::fs::write(
        dir.path().join("trusted.yml"),
        "root: aplan\ntag_families:\n  kind: [trusted-kind]\nsummary:\n  group_by: kind\n",
    )
    .expect("trusted config");

    let run = common::run(
        dir.path(),
        &[
            "--config",
            "trusted.yml",
            "summary",
            "--format",
            "json",
            "--at",
            &old,
        ],
    );
    assert_eq!(run.code, 0, "{}", run.text());
    let doc: serde_json::Value = serde_json::from_str(&run.stdout).expect("json");
    assert_eq!(doc["schema"], "aplan/summary-grouped/v1");
    assert_eq!(doc["groups"][0]["group"], "trusted-kind");

    let unknown = common::run(
        dir.path(),
        &[
            "--config",
            "trusted.yml",
            "summary",
            "--at",
            "unknown-revision",
        ],
    );
    assert_eq!(unknown.code, 2, "{}", unknown.text());
    assert!(unknown.stdout.is_empty(), "{}", unknown.stdout);
    assert!(
        unknown.stderr.contains("unknown-revision"),
        "{}",
        unknown.stderr
    );
}

#[test]
fn no_config_remains_a_refusal_for_a_historical_plan() {
    let dir = fixture();
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ntag_families:\n  kind: [configured]\nsummary:\n  group_by: kind\n",
    )
    .expect("repo config");
    write_task(dir.path(), 1, "todo", 1, "configured");
    let commit = commit(dir.path(), "configured plan");

    let run = common::run(
        dir.path(),
        &[
            "--no-config",
            "summary",
            "--format",
            "json",
            "--at",
            &commit,
        ],
    );
    assert_eq!(run.code, 0, "{}", run.text());
    let doc: serde_json::Value = serde_json::from_str(&run.stdout).expect("json");
    assert_eq!(doc["schema"], "aplan/summary/v2");
    assert!(run.stderr.contains("--no-config"), "{}", run.stderr);
}

#[test]
fn revision_discovery_with_no_config_uses_the_default_root() {
    let dir = fixture();
    write_task(dir.path(), 5, "todo", 2, "");
    commit(dir.path(), "plan without config");

    let run = common::run(dir.path(), &["summary", "--format", "json", "--at", "HEAD"]);
    assert_eq!(run.code, 0, "{}", run.text());
    let doc: serde_json::Value = serde_json::from_str(&run.stdout).expect("json");
    assert_eq!(doc["phases"][0]["phase"], 5);
}

/// Historical discovery begins where ordinary discovery begins, so the nearest
/// config wins even when the repository root carries a different one.
#[test]
fn historical_config_discovery_keeps_the_nearest_ancestor() {
    let dir = fixture();
    std::fs::write(dir.path().join(".aplan.yml"), "root: wrong-plan\n").expect("outer config");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    std::fs::write(workspace.join(".aplan.yml"), "root: plan\n").expect("near config");
    write_task_in(&workspace.join("plan"), 7, "todo", 2, "");
    commit(dir.path(), "nested plan");

    let ordinary = common::run(&workspace, &["summary", "--format", "json"]);
    let historical = common::run(&workspace, &["summary", "--format", "json", "--at", "HEAD"]);
    assert_eq!(ordinary.code, 0, "{}", ordinary.text());
    assert_eq!(historical.code, 0, "{}", historical.text());
    assert_eq!(historical.stdout, ordinary.stdout);
}

/// A config above the Git worktree has no version at the named commit. Falling
/// back to it implicitly would create a hybrid answer, while silently ignoring
/// it would disagree with ordinary discovery. The caller has to select the
/// external policy (or none) explicitly.
#[test]
fn revision_discovery_refuses_an_implicit_config_outside_the_repository() {
    let outer = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("outer tempdir");
    std::fs::write(outer.path().join(".aplan.yml"), "root: repo/aplan\n").expect("outer config");
    let repo = outer.path().join("repo");
    std::fs::create_dir(&repo).expect("repository directory");
    init(&repo);
    write_task(&repo, 1, "todo", 1, "");
    commit(&repo, "plan without in-repo config");

    let run = common::run(&repo, &["summary", "--at", "HEAD"]);
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains("outside repository"), "{}", run.stderr);
    assert!(run.stderr.contains("--config"), "{}", run.stderr);
    assert!(run.stderr.contains("--no-config"), "{}", run.stderr);
}
