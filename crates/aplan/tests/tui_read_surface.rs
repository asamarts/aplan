//! Closed-world audit of the read-only TUI boundary (`A1-435`).
//!
//! A command is not covered because its name appears in a design table. Every
//! native read entry below drives the real binary against a repository where
//! it can complete, and the test proves that the plan stayed byte-identical.
//! Native mutations are joined through the typed action union; batch,
//! transport and nested non-native modes state the product boundary
//! explicitly.

#![cfg(feature = "tui")]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use aplan_tui::discovery::{COMMANDS, CommandDisposition, NativeSurface};

use crate::common::census::{RUNS, Start, a_plan_to_act_on, plan_bytes};

use crate::common;

fn git(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn complete_read_fixture(dir: &Path) {
    a_plan_to_act_on(dir);
    std::fs::create_dir_all(dir.join("src")).expect("source directory");
    std::fs::write(dir.join("src/lib.rs"), "pub fn proof() {}\n").expect("source");
    std::fs::write(
        dir.join(".aplan.yml"),
        concat!(
            "root: aplan\n",
            "evidence:\n",
            "  observation_inputs: [src/**]\n",
            "test:\n",
            "  list: [printf, 'suite::proof\\n']\n",
            "  run: [test, '{test}', '=', 'suite::proof']\n",
            "docs:\n",
            "  paths: [AUDIT.md]\n",
            "version:\n",
            "  current:\n",
            "    command: [printf, '0.1.0\\n']\n",
        ),
    )
    .expect("config");
    std::fs::write(
        dir.join("AUDIT.md"),
        "# Configured audit input\n\nThe missing task W-404 must be reported.\n",
    )
    .expect("configured document");
    let dependent = dir.join("aplan/tasks/W-3.md");
    let body = std::fs::read_to_string(&dependent).expect("dependent task");
    std::fs::write(
        dependent,
        body.replace("effort_days: 1\n", "effort_days: 1\nblocked_by: [W-1]\n"),
    )
    .expect("dependency");
    let done = dir.join("aplan/tasks/W-2.md");
    let body = std::fs::read_to_string(&done).expect("done task");
    std::fs::write(
        done,
        body.replace("effort_days: 1\n", "effort_days: 1\npaths: [src/**]\n")
            .replace(
                "kind: note\n    ref: reviewed",
                "kind: test\n    ref: suite::proof",
            )
            .replace("    captured: 2026-08-14\n", ""),
    )
    .expect("test evidence");
    git(dir, &["init", "-q", "."]);
    git(dir, &["config", "user.name", "surface audit"]);
    git(dir, &["config", "user.email", "surface@example.invalid"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "fixture"]);
}

struct ReadFixture {
    context: aplan_app::PlanContext,
    verification: aplan_evidence::VerifyOptions,
}

fn read_context(dir: &Path) -> ReadFixture {
    let config = aplan_config::load_config(dir).expect("read fixture config");
    let verification = aplan_evidence::VerifyOptions {
        test_runner: config.test.clone(),
        max_unrelated_files: config
            .evidence
            .as_ref()
            .and_then(|evidence| evidence.max_unrelated_files),
        url_check: config.url.clone(),
        config_file: Some(dir.join(".aplan.yml")),
        version: config.version.clone(),
        ..aplan_evidence::VerifyOptions::default()
    };
    let context = aplan_app::PlanContext::new(
        aplan_core::load_plan_identified(&dir.join("aplan")),
        aplan_app::ContextInputs {
            repository_root: dir.to_path_buf(),
            configuration_file: Some(dir.join(".aplan.yml")),
            trust: aplan_app::Trust::RepositoryRead,
            options: aplan_core::Options::default(),
            verdict: aplan_core::Verdict::default(),
        },
    )
    .with_report_config(config)
    .with_documents(aplan_app::DocumentInputs {
        state: aplan_app::DocumentInputState::Configured,
        documents: vec![aplan_app::DocumentInput {
            label: "AUDIT.md".to_string(),
            path: dir.join("AUDIT.md"),
            content: Ok(std::fs::read_to_string(dir.join("AUDIT.md")).expect("fixture document")),
            membership: None,
        }],
    });
    ReadFixture {
        context,
        verification,
    }
}

fn drive_diagnostics(
    context: &aplan_app::PlanContext,
    dir: &Path,
    command: &str,
    scope: aplan_app::DiagnosticScope,
) {
    let diagnostics = aplan_app::task_view(context).diagnostics;
    let run = match scope {
        aplan_app::DiagnosticScope::Check => &diagnostics.check,
        aplan_app::DiagnosticScope::CheckDocs => &diagnostics.documents,
        aplan_app::DiagnosticScope::Verify => {
            panic!("verify has a dedicated interactive driver")
        }
    };
    assert_eq!(
        run.scope, scope,
        "{command} drove the wrong diagnostic scope"
    );
    assert!(
        !run.input_label.is_empty(),
        "{command} drove no diagnostic input"
    );
    if scope == aplan_app::DiagnosticScope::CheckDocs {
        assert_eq!(
            run.coverage,
            aplan_app::DiagnosticCoverage::Complete { subjects: 1 },
            "check-docs did not inspect the configured document"
        );
        assert_eq!(run.input_label, "configured documents (1)");
        assert!(
            run.findings.iter().any(|finding| {
                finding.code == "DOC-REFERENCE"
                    && finding.subject == "reference W-404"
                    && finding.file.as_deref() == Some(dir.join("AUDIT.md").as_path())
            }),
            "check-docs did not inspect the configured document bytes"
        );
    }
}

fn drive_verification(
    context: &aplan_app::PlanContext,
    dir: &Path,
    verification: &aplan_evidence::VerifyOptions,
) {
    let sheet = aplan_app::verification_run_sheet(context, verification);
    assert_eq!(sheet.input_sha256.len(), 64);
    assert_eq!(sheet.repository, dir);
    assert_eq!(sheet.providers.test_runners, 1);
    assert!(sheet.providers.version_current.is_available());
    assert!(sheet.execute.is_available());
    assert_eq!(sheet.trust, aplan_app::Trust::ConfiguredProcesses);
    let result = aplan_app::verify_context(context, verification)
        .expect("configured verification projection");
    assert!(
        result.citations.iter().any(|citation| {
            citation.task == "W-2"
                && citation.kind == "test"
                && citation.reference == "suite::proof"
                && citation.resolver.starts_with("test runner `")
                && citation.strength == aplan_app::CitationStrength::Resolved
                && citation.outcome == aplan_app::CitationOutcome::Resolved
        }),
        "verify did not resolve the configured test citation through its runner"
    );
}

fn drive_native(surface: NativeSurface, dir: &Path, command: &str) {
    let ReadFixture {
        context,
        verification,
    } = read_context(dir);
    let tasks = aplan_app::task_view(&context);
    match surface {
        NativeSurface::Diagnostics(scope) => drive_diagnostics(&context, dir, command, scope),
        NativeSurface::Verification => drive_verification(&context, dir, &verification),
        NativeSurface::TestCatalogue => {
            let view = aplan_tui::evidence::search_tests(&context, Some("proof".to_string()))
                .expect("configured TUI test catalogue");
            assert_eq!(view.catalogue.summary.matched_pairs, 1);
            assert_eq!(view.choices.len(), 1);
            assert_eq!(view.choices[0].reference, "suite::proof");
            assert_eq!(view.choices[0].citation, "test:suite::proof");
            assert!(view.choices[0].selectable);
        }
        NativeSurface::Work => {
            let workspace = aplan_app::WorkDiscoveryWorkspace::from_context(&context);
            let first = context.plan().tasks.first().expect("work task");
            let projection = workspace
                .project(
                    &first.task.id,
                    aplan_app::WorkDiscoveryRequest {
                        traversal: aplan_app::WhyTraversal::CutBounded,
                        as_of_epoch_seconds: 0,
                    },
                )
                .expect("work projection");
            assert_eq!(projection.task_id, first.task.id);
            assert_eq!(projection.input_sha256, context.identity().sha256);
        }
        NativeSurface::Report(kind) => {
            let projection = aplan_app::report_workspace(&context, tasks.catalogue)
                .project(kind, &aplan_app::ReportRequest::default());
            assert_eq!(projection.kind, kind, "{command} drove the wrong report");
            assert_eq!(projection.input.sha256, context.identity().sha256);
        }
        NativeSurface::Graph => {
            let projection = aplan_app::graph_workspace(&context, tasks.catalogue)
                .project(&aplan_app::GraphRequest::default());
            assert!(
                !projection.view.nodes.is_empty(),
                "{command} drove an empty graph"
            );
            assert_eq!(projection.input.sha256, context.identity().sha256);
        }
        NativeSurface::Inspector => {
            assert!(
                tasks.inspectors.iter().any(|show| show.id == "W-1"),
                "{command} drove no task inspector"
            );
        }
        NativeSurface::Explanation => {
            let explanation = aplan_app::explain_rule("APLAN001").expect("known rule");
            assert_eq!(explanation.code, "APLAN001");
        }
        NativeSurface::Tasks => {
            assert_eq!(tasks.catalogue.len(), context.plan().tasks.len());
            assert!(tasks.catalogue.iter().any(|row| row.id == "W-1"));
        }
        NativeSurface::Inbox => {
            let workspace = aplan_app::capture_workspace(
                &context,
                &aplan_core::capture::FixedClock(1_800_000_000),
            )
            .expect("native inbox workspace");
            assert_eq!(workspace.input.sha256, context.identity().sha256);
            assert_eq!(workspace.total(), 0);
            assert!(workspace.kinds.iter().any(|kind| kind == "task"));
        }
    }
}

/// Proof for `tui.action_discovery_is_closed_and_contextual`: the disposition
/// is closed over clap and facts, every promised read command actually
/// completes, and none of those runs writes the plan.
#[test]
#[allow(clippy::too_many_lines)] // One pass binds every disposition to the same facts/clap census and typed driver.
fn command_disposition_is_closed_and_every_read_member_runs() {
    let declared: BTreeSet<String> = aplan::cli_command()
        .get_subcommands()
        .map(|command| command.get_name().to_string())
        .collect();
    let answered: BTreeSet<String> = COMMANDS
        .iter()
        .map(|command| command.name.to_string())
        .collect();
    assert_eq!(
        answered, declared,
        "every advertised command needs one disposition"
    );
    assert_eq!(
        answered.len(),
        COMMANDS.len(),
        "duplicate command disposition"
    );
    let mut nested_boundaries = BTreeSet::new();
    for command in COMMANDS {
        for boundary in command.boundaries {
            assert!(
                nested_boundaries.insert(boundary.name),
                "duplicate nested boundary `{}`",
                boundary.name
            );
            assert!(
                !answered.contains(boundary.name),
                "nested boundary `{}` collides with a top-level command",
                boundary.name
            );
        }
    }
    let facts: serde_json::Value = serde_json::from_str(&aplan::facts()).expect("facts JSON");
    let facts_commands = facts["commands"]
        .as_array()
        .expect("facts commands")
        .iter()
        .map(|command| {
            command["name"]
                .as_str()
                .expect("facts command name")
                .to_string()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        answered, facts_commands,
        "the TUI contract must follow the command inventory users receive"
    );

    let run_by_name: BTreeMap<_, _> = RUNS
        .iter()
        .map(|(name, argv, start)| (*name, (*argv, *start)))
        .collect();
    let mut drove = BTreeSet::new();
    let mut mutation_actions = BTreeSet::new();
    for command in COMMANDS {
        match command.disposition {
            CommandDisposition::Native => {
                mutation_actions.extend(command.mutations.iter().copied());
                if command.surfaces.is_empty() {
                    continue;
                }
                let name = command.name;
                let (argv, start) = run_by_name.get(name).unwrap_or_else(|| {
                    panic!("native read command `{name}` has no executable driver")
                });
                assert!(
                    *start == Start::Populated,
                    "read command starts without a plan"
                );
                let dir = tempfile::Builder::new()
                    .prefix("tui-read-surface-")
                    .tempdir_in(aplan_core::fixture::fixture_root())
                    .expect("fixture");
                complete_read_fixture(dir.path());
                let before = plan_bytes(dir.path());
                let output = common::output(dir.path(), argv);
                assert!(
                    output.status.success(),
                    "`aplan {}` did not complete its native journey ({}): stdout={} stderr={}",
                    argv.join(" "),
                    output.status,
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(
                    !output.stdout.is_empty() || !output.stderr.is_empty(),
                    "`aplan {}` exercised no observable answer",
                    argv.join(" ")
                );
                assert_eq!(
                    plan_bytes(dir.path()),
                    before,
                    "read disposition `{name}` wrote the plan"
                );
                for surface in command.surfaces.iter().copied() {
                    drive_native(surface, dir.path(), name);
                }
                drove.insert(name);
            }
            CommandDisposition::Guided { reason, example }
            | CommandDisposition::CliOnly { reason, example }
            | CommandDisposition::EntryPoint { reason, example } => {
                assert!(
                    reason.len() > 30,
                    "{} has no reviewable boundary: {reason}",
                    command.name
                );
                assert!(
                    example.starts_with(&format!("aplan {}", command.name)),
                    "{} has no exact command example: {example}",
                    command.name
                );
            }
        }
        for boundary in command.boundaries {
            assert!(
                boundary.reason.len() > 30,
                "{} has no reason",
                boundary.name
            );
            assert!(
                boundary
                    .example
                    .starts_with(&format!("aplan {}", boundary.name)),
                "{} has no exact command example: {}",
                boundary.name,
                boundary.example
            );
        }
    }
    assert_eq!(
        mutation_actions,
        aplan_tui::mutations::MutationAction::ALL
            .into_iter()
            .collect(),
        "every native mutation must be reachable from the product command contract"
    );
    let expected_reads = COMMANDS
        .iter()
        .filter(|command| !command.surfaces.is_empty())
        .map(|command| command.name)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        drove, expected_reads,
        "a promised read member received no executable journey"
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum OptionDisposition {
    LaunchSession,
    SharedSelector,
    NativeControl,
    OutputOnly,
    CliOnly,
    HeldMutation,
}

fn option_disposition(command: &str, id: &str) -> OptionDisposition {
    match id {
        "root" | "config" | "no_config" | "cache" => OptionDisposition::LaunchSession,
        "color" | "progress" | "format" | "help" | "timings" | "layout" | "width" | "ascii" => {
            OptionDisposition::OutputOnly
        }
        "cut" | "phase" | "tag" | "owner" | "unclaimed" | "status" => {
            OptionDisposition::SharedSelector
        }
        _ => match (command, id) {
            ("check", "no_require_effort" | "no_traces" | "deny_warnings" | "no_deny_warnings")
            | ("verify", "execute")
            | ("ready", "explain")
            | ("due", "as_of")
            | ("why", "id" | "all")
            | ("version", "id" | "repo")
            | (
                "graph",
                "reach" | "membership" | "focus" | "critical" | "level" | "orientation" | "order"
                | "detail",
            )
            | (
                "show",
                "id"
                | "body"
                | "repo"
                | "no_resolve"
                | "refresh_evidence"
                | "require_current_evidence",
            )
            | ("explain", "vocabulary" | "name")
            | ("tree", "id" | "depth" | "live")
            | ("paths", "sample" | "repo")
            | ("stale", "older_than" | "as_of" | "repo")
            | ("check_docs", "paths")
            | ("summary", "at")
            | ("critical_path", "to")
            | ("roadmap", "regenerate_with") => OptionDisposition::NativeControl,
            ("inbox", "kind" | "state" | "label" | "revisit" | "search" | "limit") => {
                OptionDisposition::NativeControl
            }
            // A receipt import set is a caller-owned, path-bearing trust
            // boundary assembled by CI orchestration. The interactive
            // workbench renders receipt provenance in retained typed results,
            // but does not ask a terminal user to select or authenticate that
            // external manifest.
            ("check", "traces_root")
            | (
                "verify",
                "repo"
                | "no_require_effort"
                | "traces_root"
                | "no_traces"
                | "deny_warnings"
                | "no_deny_warnings"
                | "test_receipts"
                | "refresh_test_receipts"
                | "refresh_base"
                | "refresh_head"
                | "refresh_working_tree"
                | "refresh_shadow",
            ) => OptionDisposition::CliOnly,
            ("verify", "fix") => OptionDisposition::HeldMutation,
            _ => panic!("`{command}` option `{id}` has no interactive disposition"),
        },
    }
}

fn cli_only_boundary(command: &str, id: &str) -> Option<&'static str> {
    match (command, id) {
        ("check", "traces_root") => Some("check --traces-root"),
        ("verify", "repo") => Some("verify --repo"),
        ("verify", "no_require_effort") => Some("verify --no-require-effort"),
        ("verify", "traces_root") => Some("verify --traces-root"),
        ("verify", "no_traces") => Some("verify --no-traces"),
        ("verify", "deny_warnings") => Some("verify --deny-warnings"),
        ("verify", "no_deny_warnings") => Some("verify --no-deny-warnings"),
        ("verify", "test_receipts") => Some("verify --test-receipts"),
        ("verify", "refresh_test_receipts" | "refresh_base" | "refresh_head") => {
            Some("verify --refresh-test-receipts")
        }
        ("verify", "refresh_working_tree") => Some("verify --refresh-working-tree"),
        ("verify", "refresh_shadow") => Some("verify --refresh-shadow"),
        _ => None,
    }
}

#[derive(Debug)]
struct OptionCase {
    argv: &'static [&'static str],
    covers: &'static [(&'static str, &'static str)],
}

fn run_option_case(dir: &Path, case: &OptionCase) {
    let before = plan_bytes(dir);
    let output = common::output(dir, case.argv);
    assert!(
        output.status.success(),
        "option journey `aplan {}` failed: {}",
        case.argv.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(plan_bytes(dir), before, "option journey wrote the plan");
}

const OPTION_CASES: &[OptionCase] = &[
    OptionCase {
        argv: &[
            "tasks",
            "--no-config",
            "--cache",
            "off",
            "--root",
            "aplan",
            "--status",
            "todo",
        ],
        covers: &[
            ("global", "no_config"),
            ("global", "cache"),
            ("global", "root"),
            ("selection", "status"),
        ],
    },
    OptionCase {
        argv: &[
            "show",
            "W-2",
            "--refresh-evidence",
            "--require-current-evidence",
            "--format",
            "json",
        ],
        covers: &[
            ("show", "refresh_evidence"),
            ("show", "require_current_evidence"),
        ],
    },
    OptionCase {
        argv: &[
            "tasks",
            "--config",
            ".aplan.yml",
            "--phase",
            "1",
            "--unclaimed",
        ],
        covers: &[
            ("global", "config"),
            ("selection", "phase"),
            ("selection", "unclaimed"),
        ],
    },
    OptionCase {
        argv: &[
            "tasks", "--cut", "M-a", "--tag", "alpha", "--owner", "nobody",
        ],
        covers: &[
            ("selection", "cut"),
            ("selection", "tag"),
            ("selection", "owner"),
        ],
    },
    OptionCase {
        argv: &[
            "check",
            "--no-require-effort",
            "--no-traces",
            "--deny-warnings",
            "--format",
            "json",
        ],
        covers: &[
            ("check", "no_require_effort"),
            ("check", "no_traces"),
            ("check", "deny_warnings"),
        ],
    },
    OptionCase {
        argv: &[
            "check",
            "--traces-root",
            ".",
            "--no-deny-warnings",
            "--format",
            "json",
        ],
        covers: &[("check", "no_deny_warnings")],
    },
    OptionCase {
        argv: &[
            "verify",
            "--repo",
            ".",
            "--no-require-effort",
            "--no-traces",
            "--execute",
            "--deny-warnings",
            "--format",
            "json",
        ],
        covers: &[("verify", "execute")],
    },
    OptionCase {
        argv: &[
            "verify",
            "--traces-root",
            ".",
            "--no-deny-warnings",
            "--format",
            "json",
        ],
        covers: &[],
    },
    OptionCase {
        argv: &["ready", "--explain"],
        covers: &[("ready", "explain")],
    },
    OptionCase {
        argv: &["due", "--as-of", "2030-01-01T00:00:00Z", "--format", "json"],
        covers: &[("due", "as_of")],
    },
    OptionCase {
        argv: &["why", "W-1", "--all", "--format", "json"],
        covers: &[("why", "id"), ("why", "all")],
    },
    OptionCase {
        argv: &["version", "M-a", "--format", "json"],
        covers: &[("version", "id")],
    },
    OptionCase {
        argv: &["version", "--repo", ".", "--format", "json"],
        covers: &[("version", "repo")],
    },
    OptionCase {
        argv: &[
            "graph",
            "--reach",
            "downstream:W-1",
            "--membership",
            "--format",
            "json",
        ],
        covers: &[("graph", "reach"), ("graph", "membership")],
    },
    OptionCase {
        argv: &[
            "graph",
            "--focus",
            "W-1",
            "--critical",
            "--format",
            "text",
            "--width",
            "240",
        ],
        covers: &[("graph", "focus"), ("graph", "critical")],
    },
    OptionCase {
        argv: &[
            "graph",
            "--format",
            "text",
            "--layout",
            "layered",
            "--level",
            "task:W-1",
            "--orientation",
            "top-to-bottom",
            "--order",
            "id",
            "--detail",
            "compact",
            "--width",
            "240",
            "--ascii",
        ],
        covers: &[
            ("graph", "level"),
            ("graph", "orientation"),
            ("graph", "order"),
            ("graph", "detail"),
        ],
    },
    OptionCase {
        argv: &[
            "show",
            "W-2",
            "--body",
            "--repo",
            ".",
            "--no-resolve",
            "--format",
            "json",
        ],
        covers: &[
            ("show", "id"),
            ("show", "body"),
            ("show", "repo"),
            ("show", "no_resolve"),
        ],
    },
    OptionCase {
        argv: &["explain", "rule", "APLAN001"],
        covers: &[("explain", "vocabulary"), ("explain", "name")],
    },
    OptionCase {
        argv: &["tree", "W-1", "--depth", "2", "--live"],
        covers: &[("tree", "id"), ("tree", "depth"), ("tree", "live")],
    },
    OptionCase {
        argv: &["paths", "--sample", "1", "--repo", ".", "--format", "json"],
        covers: &[("paths", "sample"), ("paths", "repo")],
    },
    OptionCase {
        argv: &[
            "stale",
            "--older-than",
            "1",
            "--as-of",
            "2030-01-01",
            "--repo",
            ".",
            "--format",
            "json",
        ],
        covers: &[
            ("stale", "older_than"),
            ("stale", "as_of"),
            ("stale", "repo"),
        ],
    },
    OptionCase {
        argv: &["check-docs", "NOTES.md", "--format", "json"],
        covers: &[("check_docs", "paths")],
    },
    OptionCase {
        argv: &["summary", "--at", "HEAD", "--format", "json"],
        covers: &[("summary", "at")],
    },
    OptionCase {
        argv: &["critical-path", "W-1", "--format", "json"],
        covers: &[("critical_path", "to")],
    },
    OptionCase {
        argv: &[
            "roadmap",
            "--format",
            "markdown",
            "--regenerate-with",
            "make roadmap",
        ],
        covers: &[("roadmap", "regenerate_with")],
    },
    OptionCase {
        argv: &[
            "inbox",
            "--kind",
            "task",
            "--state",
            "all",
            "--label",
            "missing",
            "--revisit",
            "all",
            "--search",
            "missing",
            "--limit",
            "7",
            "--format",
            "json",
        ],
        covers: &[
            ("inbox", "kind"),
            ("inbox", "state"),
            ("inbox", "label"),
            ("inbox", "revisit"),
            ("inbox", "search"),
            ("inbox", "limit"),
        ],
    },
];

/// Proof for `tui.action_discovery_is_closed_and_contextual`: clap supplies
/// the membership, and the matcher supplies the answer. A new
/// behavior-bearing option therefore fails here until it has an interactive
/// meaning or an explicit boundary. Every behavior-bearing member must be
/// named exactly once by a successful command case; presentation and held
/// mutation members remain dispositions, not fake executions.
#[test]
#[allow(clippy::too_many_lines)] // One pass binds clap membership, visible boundaries and running option cases.
fn every_read_option_has_a_disposition_and_behavioral_dimensions_run() {
    let cli = aplan::cli_command();
    let mut counts = BTreeMap::<OptionDisposition, usize>::new();
    let mut expected = BTreeSet::<(String, String)>::new();
    let mut register = |command: &str, id: &str, disposition| {
        *counts.entry(disposition).or_default() += 1;
        let owner = match disposition {
            OptionDisposition::LaunchSession => Some("global"),
            OptionDisposition::SharedSelector => Some("selection"),
            OptionDisposition::NativeControl => Some(command),
            OptionDisposition::OutputOnly
            | OptionDisposition::CliOnly
            | OptionDisposition::HeldMutation => None,
        };
        if let Some(owner) = owner {
            expected.insert((owner.to_string(), id.to_string()));
        }
    };
    for argument in cli.get_arguments() {
        let disposition = option_disposition("global", argument.get_id().as_str());
        register("global", argument.get_id().as_str(), disposition);
    }
    let read_commands = COMMANDS
        .iter()
        .filter(|command| !command.surfaces.is_empty());
    for contract in read_commands {
        let name = contract.name;
        let command = cli
            .get_subcommands()
            .find(|candidate| candidate.get_name() == name)
            .unwrap_or_else(|| panic!("missing read command {name}"));
        for argument in command.get_arguments() {
            let command = name.replace('-', "_");
            let disposition = option_disposition(&command, argument.get_id().as_str());
            if disposition == OptionDisposition::CliOnly {
                let boundary_name = cli_only_boundary(&command, argument.get_id().as_str())
                    .unwrap_or_else(|| {
                        panic!(
                            "CLI-only option `{name} --{}` has no user-visible boundary mapping",
                            argument.get_id()
                        )
                    });
                let boundary = contract
                    .boundaries
                    .iter()
                    .find(|boundary| boundary.name == boundary_name)
                    .unwrap_or_else(|| {
                        panic!(
                            "CLI-only option `{name} --{}` maps to absent boundary `{boundary_name}`",
                            argument.get_id()
                        )
                    });
                let flag = format!("--{}", argument.get_id().as_str().replace('_', "-"));
                assert!(
                    boundary.aliases.contains(&flag.as_str())
                        || boundary.example.split_whitespace().any(|word| word == flag),
                    "boundary `{boundary_name}` never names `{flag}`"
                );
            }
            register(&command, argument.get_id().as_str(), disposition);
        }
    }
    for disposition in [
        OptionDisposition::LaunchSession,
        OptionDisposition::SharedSelector,
        OptionDisposition::NativeControl,
        OptionDisposition::OutputOnly,
        OptionDisposition::CliOnly,
        OptionDisposition::HeldMutation,
    ] {
        assert!(
            counts.get(&disposition).copied().unwrap_or_default() > 0,
            "no option exercised the {disposition:?} disposition"
        );
    }
    let dir = tempfile::Builder::new()
        .prefix("tui-read-options-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    complete_read_fixture(dir.path());
    let mut covered = BTreeSet::new();
    for case in OPTION_CASES {
        let matches = cli
            .clone()
            .try_get_matches_from(std::iter::once("aplan").chain(case.argv.iter().copied()))
            .unwrap_or_else(|error| panic!("invalid option case {:?}: {error}", case.argv));
        let (_, command_matches) = matches.subcommand().expect("option case command");
        run_option_case(dir.path(), case);
        for &(owner, id) in case.covers {
            let key = (owner.to_string(), id.to_string());
            assert!(
                expected.contains(&key),
                "option case {:?} claims stale or non-behavioral {owner}.{id}",
                case.argv
            );
            assert_eq!(
                command_matches.value_source(id),
                Some(clap::parser::ValueSource::CommandLine),
                "option case {:?} claims {owner}.{id} without passing it",
                case.argv
            );
            assert!(
                covered.insert(key),
                "behavioral option {owner}.{id} is claimed by more than one case"
            );
        }
    }
    assert_eq!(
        covered, expected,
        "every behavioral option needs one running case"
    );
}

fn leaf_commands<'a>(
    command: &'a clap::Command,
    prefix: &str,
    leaves: &mut Vec<(String, &'a clap::Command)>,
) {
    let children = command
        .get_subcommands()
        .filter(|child| child.get_name() != "help")
        .collect::<Vec<_>>();
    if children.is_empty() {
        if !prefix.is_empty() {
            leaves.push((prefix.to_string(), command));
        }
        return;
    }
    for child in children {
        let path = if prefix.is_empty() {
            child.get_name().to_string()
        } else {
            format!("{prefix} {}", child.get_name())
        };
        leaf_commands(child, &path, leaves);
    }
}

/// Proof for `tui.action_discovery_is_closed_and_contextual`: command-level
/// native coverage cannot hide a newly added semantic mode. The detailed read
/// and mutation audits own native leaves; this pass closes the nested,
/// guided, CLI-only and entry-point neighbours those catalogues do not share.
#[test]
#[allow(clippy::too_many_lines)] // One recursive audit keeps leaf ownership and each exact exception together.
fn every_command_leaf_and_option_has_one_interactive_disposition() {
    let cli = aplan::cli_command();
    let globals = cli
        .get_arguments()
        .map(|argument| argument.get_id().as_str().to_string())
        .chain(["help".to_string(), "version".to_string()])
        .collect::<BTreeSet<_>>();
    let mut leaves = Vec::new();
    leaf_commands(&cli, "", &mut leaves);
    let mut seen_top = BTreeSet::new();
    let mut nested_native = BTreeSet::new();

    let parse_discovery_example = |owner: &str, example: &str| match cli
        .clone()
        .try_get_matches_from(example.split_whitespace())
    {
        Ok(_) => {}
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => {}
        Err(error) => panic!("invalid discovery example `{example}` for {owner}: {error}"),
    };
    for command in COMMANDS {
        if let Some((_, example)) = command.disposition.boundary() {
            parse_discovery_example(command.name, example);
        }
        for boundary in command.boundaries {
            parse_discovery_example(boundary.name, boundary.example);
        }
    }

    for (path, leaf) in leaves {
        let top = path.split_whitespace().next().expect("nonempty leaf path");
        let contract = aplan_tui::discovery::command(top)
            .unwrap_or_else(|| panic!("CLI leaf `{path}` has no product disposition"));
        seen_top.insert(top.to_string());
        let own = leaf
            .get_arguments()
            .map(|argument| argument.get_id().as_str())
            .filter(|id| !globals.contains(*id))
            .collect::<BTreeSet<_>>();

        match contract.disposition {
            CommandDisposition::CliOnly { .. } => {
                // The entire command is the explicit batch/transport
                // boundary. New modes inherit that boundary rather than
                // becoming falsely native.
                assert!(
                    own.iter().all(|id| !id.trim().is_empty()),
                    "`{path}` exposed a vacuous CLI-only mode"
                );
            }
            CommandDisposition::EntryPoint { .. } => {
                let expected = match path.as_str() {
                    "tui" => BTreeSet::from(["ascii", "reduced_motion", "theme"]),
                    _ => BTreeSet::new(),
                };
                assert_eq!(
                    own, expected,
                    "entry point `{path}` gained a mode that needs a launch or boundary disposition"
                );
                if path == "tui" {
                    for example in [
                        ["aplan", "tui", "--theme", "light"].as_slice(),
                        ["aplan", "tui", "--theme", "low-color"].as_slice(),
                        ["aplan", "tui", "--ascii", "--reduced-motion"].as_slice(),
                    ] {
                        cli.clone()
                            .try_get_matches_from(example)
                            .unwrap_or_else(|error| {
                                panic!("invalid TUI appearance option {example:?}: {error}")
                            });
                    }
                }
            }
            CommandDisposition::Guided { .. } => {
                let expected = match path.as_str() {
                    "init" => BTreeSet::from(["dir"]),
                    "facts" => BTreeSet::from(["identity", "require"]),
                    _ => panic!("unknown guided command `{path}`"),
                };
                assert_eq!(
                    own, expected,
                    "guided command `{path}` gained an undisposed mode"
                );
                if path == "facts" {
                    let boundary = contract
                        .boundaries
                        .iter()
                        .find(|boundary| boundary.name == "facts --require")
                        .expect("facts --require needs a visible automation boundary");
                    assert!(boundary.aliases.contains(&"facts --require"));
                    nested_native.insert(boundary.name.to_string());
                }
            }
            CommandDisposition::Native => {
                if !contract.mutations.is_empty() && contract.surfaces.is_empty() {
                    // `tui_mutation_surface` derives these exact leaf modes
                    // from clap and refuses every missing disposition.
                    continue;
                }
                match path.as_str() {
                    "inbox show" => {
                        assert_eq!(own, BTreeSet::from(["id"]));
                        nested_native.insert(path);
                    }
                    "inbox amend" | "inbox discard" | "inbox restore" | "inbox promote" => {
                        assert!(
                            contract.mutations.iter().any(|action| action.cli() == path),
                            "native inbox leaf `{path}` has no mutation action"
                        );
                        nested_native.insert(path);
                    }
                    "test list" => {
                        assert_eq!(own, BTreeSet::from(["query"]));
                        nested_native.insert(path);
                    }
                    "test receipt" => {
                        assert_eq!(
                            own,
                            BTreeSet::from([
                                "output",
                                "producer_event",
                                "producer_privilege",
                                "producer_repository",
                                "producer_run_id",
                                "producer_workflow",
                                "runner",
                            ]),
                            "receipt production is an explicit CLI-only mode inside native test discovery"
                        );
                        let boundary = contract
                            .boundaries
                            .iter()
                            .find(|boundary| boundary.name == path)
                            .expect("test receipt needs a user-visible CLI-only boundary");
                        assert!(boundary.aliases.contains(&"receipt"));
                        nested_native.insert(path);
                    }
                    _ if !path.contains(' ') => {
                        let command = path.replace('-', "_");
                        for id in own {
                            let _ = option_disposition(&command, id);
                        }
                    }
                    _ => panic!("native leaf `{path}` has no interactive option audit"),
                }
            }
        }
    }

    assert_eq!(
        seen_top,
        COMMANDS
            .iter()
            .map(|command| command.name.to_string())
            .collect(),
        "the leaf audit did not reach every facts command"
    );
    assert_eq!(
        nested_native,
        BTreeSet::from([
            "facts --require".to_string(),
            "inbox amend".to_string(),
            "inbox discard".to_string(),
            "inbox promote".to_string(),
            "inbox restore".to_string(),
            "inbox show".to_string(),
            "test list".to_string(),
            "test receipt".to_string(),
        ]),
        "nested native modes were silently added or removed"
    );
}
