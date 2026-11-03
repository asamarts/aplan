//! The interactive report hub consumes the same typed facts the CLI publishes.

use std::path::Path;

use crate::common;

fn fixture(root: &Path) {
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("task directory");
    std::fs::create_dir_all(root.join("src")).expect("source directory");
    std::fs::write(root.join("src/lib.rs"), "pub fn shipped() {}\n").expect("source");
    std::fs::write(
        root.join(".aplan.yml"),
        concat!(
            "root: aplan\n",
            "effort_days_per_week: 5\n",
            "stale_after_days: 2\n",
            "tag_families:\n",
            "  kind: [kind-build, kind-doc]\n",
            "roadmap:\n",
            "  group_by: kind\n",
            "summary:\n",
            "  group_by: kind\n",
            "version:\n",
            "  current:\n",
            "    command: [printf, 1.2.3]\n",
        ),
    )
    .expect("config");
    let tasks = [
        (
            "R-1",
            "---\nid: R-1\ntitle: measured path\nphase: 1\nstatus: done\n",
            "effort_days: 1\npaths: [src/**]\n",
            "",
        ),
        (
            "R-2",
            "---\nid: R-2\ntitle: zero-hit path\nphase: 1\nstatus: todo\n",
            "effort_days: 2\npaths: [docs/**]\nversion_impact: breaking\n",
            "blocked_by: [R-1]\n",
        ),
        (
            "R-3",
            "---\nid: R-3\ntitle: no path\nphase: 2\nstatus: todo\n",
            "effort_days: 1\n",
            "",
        ),
        (
            "R-4",
            "---\nid: R-4\ntitle: invalid path\nphase: 2\nstatus: todo\n",
            "effort_days: 1\npaths: ['[']\n",
            "",
        ),
        (
            "R-D",
            "---\nid: R-D\ntitle: announce removal\nphase: 1\nstatus: todo\n",
            "effort_days: 1\ntags: [deprecation]\n",
            "",
        ),
        (
            "R-X",
            "---\nid: R-X\ntitle: remove API\nphase: 1\nstatus: todo\n",
            "effort_days: 1\ntags: [removal]\n",
            "blocked_by: [R-D]\n",
        ),
        (
            "R-U",
            "---\nid: R-U\ntitle: unannounced removal\nphase: 1\nstatus: todo\n",
            "effort_days: 1\ntags: [removal]\n",
            "",
        ),
        (
            "M-A",
            "---\nid: M-A\ntitle: announcement cut\nphase: 1\nstatus: todo\n",
            "tags: [milestone, v0.1]\n",
            "blocked_by: [R-D]\n",
        ),
        (
            "M-R",
            "---\nid: M-R\ntitle: report cut\nphase: 2\nstatus: todo\n",
            "tags: [milestone, v0.2]\n",
            "blocked_by: [R-1, R-2, R-3, R-4, R-X, R-U, M-A]\n",
        ),
    ];
    for (id, head, fields, tail) in tasks {
        std::fs::write(
            root.join(format!("aplan/tasks/{id}.md")),
            format!("{head}{fields}{tail}---\n\nReport fixture.\n"),
        )
        .expect("task");
    }

    let repository = git2::Repository::init(root).expect("git init");
    let mut index = repository.index().expect("index");
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .expect("git add");
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("write tree");
    let tree = repository.find_tree(tree_id).expect("tree");
    let when = git2::Time::new(1_767_225_600, 0); // 2026-01-01T00:00:00Z
    let signature = git2::Signature::new("test", "test@example.invalid", &when).expect("signature");
    repository
        .commit(Some("HEAD"), &signature, &signature, "fixture", &tree, &[])
        .expect("commit");
}

fn cli_json(root: &Path, command: &[&str]) -> serde_json::Value {
    let output = common::output(root, command);
    assert!(
        output.status.success(),
        "aplan {} failed: {}",
        command.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("CLI JSON")
}

fn workspace(root: &Path) -> aplan_app::ReportWorkspace {
    let config = aplan_config::load_config(root).expect("config");
    let context = aplan_app::PlanContext::new(
        aplan_core::load_plan_identified(&root.join("aplan")),
        aplan_app::ContextInputs {
            repository_root: root.to_path_buf(),
            configuration_file: Some(root.join(".aplan.yml")),
            trust: aplan_app::Trust::RepositoryRead,
            options: aplan_core::Options::default(),
            verdict: aplan_core::Verdict::default(),
        },
    )
    .with_report_config(config);
    let catalogue = aplan_app::task_catalogue(&context);
    aplan_app::report_workspace(&context, catalogue)
}

fn assert_rollup_parity(root: &Path, workspace: &aplan_app::ReportWorkspace) {
    let request = aplan_app::ReportRequest::default();
    let roadmap = workspace.project(aplan_app::ReportKind::Roadmap, &request);
    let aplan_app::ReportBody::Roadmap(roadmap) = roadmap.body.expect("roadmap body") else {
        panic!("wrong roadmap body")
    };
    let cli = cli_json(root, &["roadmap", "--format", "json"]);
    assert_eq!(cli["cuts"].as_array().unwrap().len(), roadmap.cuts.len());
    assert_eq!(cli["total_effort_days"], roadmap.total_effort_days());
    assert_eq!(
        cli["cuts"][0]["critical_path_days"],
        roadmap.cuts[0].critical_path.total_effort_days
    );

    let summary = workspace.project(aplan_app::ReportKind::Summary, &request);
    let aplan_app::ReportBody::Summary(aplan_app::SummaryReport::Groups(summary)) =
        summary.body.expect("summary body")
    else {
        panic!("wrong summary body")
    };
    let cli = cli_json(root, &["summary", "--format", "json"]);
    assert_eq!(cli["group_by"], "kind");
    assert_eq!(cli["total_tasks"], summary.total_tasks());
    assert_eq!(cli["total_done"], summary.total_done());
    assert_eq!(cli["total_effort_days"], summary.total_effort_days());
    assert_eq!(
        cli["total_weeks"],
        serde_json::to_value(summary.total_weeks()).unwrap()
    );
}

fn assert_version_parity(root: &Path, workspace: &aplan_app::ReportWorkspace) {
    let version = workspace.project(
        aplan_app::ReportKind::Version,
        &aplan_app::ReportRequest::default(),
    );
    let aplan_app::ReportBody::Version(version) = version.body.expect("version body") else {
        panic!("wrong version body")
    };
    let node = version
        .nodes
        .iter()
        .find(|node| node.id == "M-R")
        .expect("milestone version row");
    let cli = cli_json(root, &["version", "M-R", "--format", "json"]);
    assert_eq!(cli["owes"], node.owes.to_string());
    assert_eq!(
        cli["raised_by"],
        serde_json::to_value(&node.raised_by).unwrap()
    );
    assert_eq!(
        cli["recorded"],
        serde_json::to_value(&node.recorded).unwrap()
    );
}

fn assert_analysis_parity(root: &Path, workspace: &aplan_app::ReportWorkspace) {
    let request = aplan_app::ReportRequest::default();
    let paths = workspace.project(aplan_app::ReportKind::Paths, &request);
    let aplan_app::ReportBody::Paths(paths) = paths.body.expect("paths body") else {
        panic!("wrong paths body")
    };
    let cli = cli_json(root, &["paths", "--sample", "100", "--format", "json"]);
    assert_eq!(cli["sampled"], paths.sampled);
    assert_eq!(cli["without_paths"], paths.without_paths);
    assert_eq!(cli["unmeasurable"], paths.unmeasurable);
    assert_eq!(cli["tasks"].as_array().unwrap().len(), paths.tasks.len());
    for row in &paths.tasks {
        let cli_row = cli["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["id"] == row.id)
            .expect("CLI path row");
        assert_eq!(cli_row["hits"], row.hits);
        assert_eq!(cli_row["percent"], row.percent);
    }

    let vacuity = workspace.project(aplan_app::ReportKind::Vacuity, &request);
    let aplan_app::ReportBody::Vacuity(vacuity) = vacuity.body.expect("vacuity body") else {
        panic!("wrong vacuity body")
    };
    let cli = cli_json(root, &["vacuity", "--format", "json"]);
    for row in &vacuity {
        let cli_row = cli["rules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["rule"] == row.rule.code())
            .expect("CLI vacuity row");
        assert_eq!(cli_row["subjects"], row.subjects);
        assert_eq!(cli_row["checked"], row.checked());
        assert_eq!(cli_row["blind"], row.unbound);
    }

    let stale_request = aplan_app::ReportRequest {
        stale_as_of: Some("2026-01-10".to_string()),
        ..request.clone()
    };
    let stale = workspace.project(aplan_app::ReportKind::Stale, &stale_request);
    let aplan_app::ReportBody::Stale(stale) = stale.body.expect("stale body") else {
        panic!("wrong stale body")
    };
    let cli = cli_json(
        root,
        &["stale", "--as-of", "2026-01-10", "--format", "json"],
    );
    assert_eq!(cli["window_days"], stale.window_days);
    assert_eq!(cli["fresh"], stale.fresh);
    assert_eq!(cli["untracked"], stale.untracked);
    assert_eq!(cli["tasks"].as_array().unwrap().len(), stale.tasks.len());
}

fn assert_deprecation_parity(root: &Path, workspace: &aplan_app::ReportWorkspace) {
    let deprecations = workspace.project(
        aplan_app::ReportKind::Deprecations,
        &aplan_app::ReportRequest::default(),
    );
    let aplan_app::ReportBody::Deprecations(deprecations) =
        deprecations.body.expect("deprecations body")
    else {
        panic!("wrong deprecations body")
    };
    let cli = cli_json(root, &["deprecations", "--format", "json"]);
    assert_eq!(
        cli["runways"].as_array().unwrap().len(),
        deprecations.runways.len()
    );
    assert!(deprecations.runways[0].faults.is_empty());
    assert_eq!(cli["runways"][0]["faults"], serde_json::json!([]));
    assert_eq!(deprecations.unannounced[0].id, "R-U");
    let rows = workspace.project(
        aplan_app::ReportKind::Deprecations,
        &aplan_app::ReportRequest::default(),
    );
    assert!(
        rows.rows.iter().any(|row| {
            row.key == "R-D" && row.summary == "1 removal task(s); runway is complete"
        })
    );
    assert!(
        rows.rows
            .iter()
            .any(|row| { row.key == "R-U" && row.summary == "removal has no announcement task" })
    );
}

/// Proof for `tui.report_hub_preserves_cli_report_semantics`.
#[test]
fn report_hub_typed_values_match_cli_json() {
    let root = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(root.path());
    let workspace = workspace(root.path());
    assert_rollup_parity(root.path(), &workspace);
    assert_version_parity(root.path(), &workspace);
    assert_analysis_parity(root.path(), &workspace);
    assert_deprecation_parity(root.path(), &workspace);
}

/// Proof for `tui.report_hub_preserves_cli_report_semantics`: invalid inputs,
/// unavailable providers and semantically different zero states remain
/// distinguishable at the typed report boundary.
#[test]
fn report_hub_distinguishes_unavailable_invalid_and_zero_measurements() {
    let root = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(root.path());
    let reports = workspace(root.path());
    let invalid = reports.project(
        aplan_app::ReportKind::Stale,
        &aplan_app::ReportRequest {
            stale_as_of: Some("not-a-date".to_string()),
            ..aplan_app::ReportRequest::default()
        },
    );
    assert!(matches!(invalid.state, aplan_app::ReportState::Invalid(_)));

    let paths = reports.project(
        aplan_app::ReportKind::Paths,
        &aplan_app::ReportRequest::default(),
    );
    assert!(
        paths
            .rows
            .iter()
            .any(|row| row.summary.contains("measured zero"))
    );
    assert!(paths.rows.iter().any(|row| row.key == "without-paths"));
    assert!(paths.rows.iter().any(|row| row.key == "unmeasurable"));
    let vacuity = reports.project(
        aplan_app::ReportKind::Vacuity,
        &aplan_app::ReportRequest::default(),
    );
    let zero_subject = vacuity
        .rows
        .iter()
        .find(|row| row.summary.contains("0 subject(s)"))
        .expect("zero-subject reach remains visible");
    assert_eq!(
        zero_subject.provenance,
        aplan_app::ReportRowProvenance::NoSubjects
    );

    let context = aplan_app::PlanContext::new(
        aplan_core::load_plan_identified(&root.path().join("aplan")),
        aplan_app::ContextInputs {
            repository_root: root.path().to_path_buf(),
            configuration_file: Some(root.path().join(".aplan.yml")),
            trust: aplan_app::Trust::Hermetic,
            options: aplan_core::Options::default(),
            verdict: aplan_core::Verdict::default(),
        },
    );
    let hermetic = aplan_app::report_workspace(&context, aplan_app::task_catalogue(&context));
    assert!(matches!(
        hermetic
            .project(
                aplan_app::ReportKind::Paths,
                &aplan_app::ReportRequest::default()
            )
            .state,
        aplan_app::ReportState::Unavailable(_)
    ));
    let version = hermetic.project(
        aplan_app::ReportKind::Version,
        &aplan_app::ReportRequest::default(),
    );
    assert_eq!(version.rows[0].key, "current-source");
    assert!(version.rows[0].task_ids.is_empty());

    let empty = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    std::fs::create_dir_all(empty.path().join("aplan/tasks")).expect("task directory");
    std::fs::write(empty.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        empty.path().join("aplan/tasks/N-1.md"),
        "---\nid: N-1\ntitle: no history\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/**]\n---\n",
    )
    .expect("task");
    git2::Repository::init(empty.path()).expect("git init");
    let empty_paths = workspace(empty.path()).project(
        aplan_app::ReportKind::Paths,
        &aplan_app::ReportRequest::default(),
    );
    let Some(aplan_app::ReportBody::Paths(report)) = &empty_paths.body else {
        panic!("paths body")
    };
    assert_eq!(report.sampled, 0);
    assert!(empty_paths.rows.iter().any(|row| row.key == "no-sample"));

    let vacant = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    std::fs::create_dir_all(vacant.path().join("aplan/tasks")).expect("task directory");
    std::fs::write(vacant.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    git2::Repository::init(vacant.path()).expect("git init");
    let stale = workspace(vacant.path()).project(
        aplan_app::ReportKind::Stale,
        &aplan_app::ReportRequest::default(),
    );
    let Some(aplan_app::ReportBody::Stale(report)) = &stale.body else {
        panic!("stale body")
    };
    assert!(report.tasks.is_empty());
    assert!(report.coordination.available);
    assert_eq!(
        report.coordination.kind.as_deref(),
        Some("git_common_directory")
    );

    let non_git = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    std::fs::create_dir_all(non_git.path().join("aplan/tasks")).expect("task directory");
    std::fs::write(non_git.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    let stale = workspace(non_git.path()).project(
        aplan_app::ReportKind::Stale,
        &aplan_app::ReportRequest::default(),
    );
    assert!(matches!(
        stale.state,
        aplan_app::ReportState::Unavailable(reason)
            if reason.contains("repository") || reason.contains("Git")
    ));
}

#[test]
fn stale_reports_keep_owner_claim_age_and_tracking_classes_distinct() {
    let root = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("tempdir");
    fixture(root.path());
    let r2 = root.path().join("aplan/tasks/R-2.md");
    let task = std::fs::read_to_string(&r2).expect("R-2");
    std::fs::write(
        &r2,
        task.replace("status: todo\n", "status: todo\nowner: team\n"),
    )
    .expect("stable owner");
    std::fs::write(
        root.path().join("aplan/tasks/R-N.md"),
        "---\nid: R-N\ntitle: untracked task\nphase: 2\nstatus: todo\neffort_days: 1\n---\n",
    )
    .expect("untracked task");
    let claimed = common::output(root.path(), &["claim", "R-2", "--as", "ada"]);
    assert!(
        claimed.status.success(),
        "claim failed: {}",
        String::from_utf8_lossy(&claimed.stderr)
    );

    let reports = workspace(root.path());
    let before_head = reports.project(
        aplan_app::ReportKind::Stale,
        &aplan_app::ReportRequest {
            stale_as_of: Some("2025-12-31".to_string()),
            ..aplan_app::ReportRequest::default()
        },
    );
    assert!(matches!(
        before_head.state,
        aplan_app::ReportState::Invalid(_)
    ));

    let fresh = reports.project(
        aplan_app::ReportKind::Stale,
        &aplan_app::ReportRequest {
            stale_as_of: Some("2026-01-10".to_string()),
            stale_window_days: Some(20),
            ..aplan_app::ReportRequest::default()
        },
    );
    let Some(aplan_app::ReportBody::Stale(fresh)) = fresh.body else {
        panic!("fresh stale-report body")
    };
    assert!(fresh.fresh > 0);
    assert_eq!(fresh.untracked, 1);
    assert!(fresh.tasks.is_empty());

    let stale = reports.project(
        aplan_app::ReportKind::Stale,
        &aplan_app::ReportRequest {
            stale_as_of: Some("2026-01-10".to_string()),
            stale_window_days: Some(2),
            ..aplan_app::ReportRequest::default()
        },
    );
    let Some(aplan_app::ReportBody::Stale(stale)) = stale.body else {
        panic!("stale-report body")
    };
    let r2 = stale
        .tasks
        .iter()
        .find(|task| task.id == "R-2")
        .expect("R-2 stale row");
    assert_eq!(r2.owner.as_deref(), Some("team"));
    assert_eq!(r2.held_by.as_deref(), Some("ada"));
    assert!(r2.claim.is_some());
    assert!(r2.claim_age_seconds.is_some());
    assert!(stale.untracked > 0);
}
