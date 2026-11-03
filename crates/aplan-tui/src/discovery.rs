//! Closed product-level discovery contract for the interactive interface.
//!
//! `aplan facts` is the live command inventory. This module records the
//! interactive disposition of every member without depending on clap or the
//! binary crate, and the integration audit compares the two. The shell uses
//! the same values for search aliases and for user-visible boundaries, so a
//! command cannot be "covered" only inside a test table.

use crate::mutations::MutationAction;

/// One typed application surface that proves a native command is represented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeSurface {
    Diagnostics(aplan_app::DiagnosticScope),
    Verification,
    TestCatalogue,
    Work,
    Report(aplan_app::ReportKind),
    Graph,
    Inspector,
    Explanation,
    Tasks,
    Inbox,
}

/// How one top-level command is exposed by the interactive product.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandDisposition {
    /// One or more independently driven in-process surfaces answer it.
    Native,
    /// The TUI explains and routes the workflow without silently performing it.
    Guided {
        reason: &'static str,
        example: &'static str,
    },
    /// The workflow deliberately remains non-interactive and is copyable.
    CliOnly {
        reason: &'static str,
        example: &'static str,
    },
    /// This command enters the interface and cannot be an action within itself.
    EntryPoint {
        reason: &'static str,
        example: &'static str,
    },
}

impl CommandDisposition {
    #[must_use]
    pub const fn boundary(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Native => None,
            Self::Guided { reason, example }
            | Self::CliOnly { reason, example }
            | Self::EntryPoint { reason, example } => Some((reason, example)),
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Guided { .. } => "guided",
            Self::CliOnly { .. } => "CLI-only",
            Self::EntryPoint { .. } => "entry point",
        }
    }
}

/// One top-level member of the command inventory published by `aplan facts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandDiscovery {
    pub name: &'static str,
    pub title: &'static str,
    pub disposition: CommandDisposition,
    pub surfaces: &'static [NativeSurface],
    pub mutations: &'static [MutationAction],
    /// Deliberately non-native leaves beneath an otherwise native command.
    pub boundaries: &'static [CommandBoundary],
}

/// One semantic CLI leaf which stays visible but inert in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandBoundary {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub reason: &'static str,
    pub example: &'static str,
}

const CLAIM: &[MutationAction] = &[MutationAction::Claim];
const RELEASE: &[MutationAction] = &[MutationAction::Release];
const START: &[MutationAction] = &[MutationAction::Start];
const STOP: &[MutationAction] = &[MutationAction::Stop];
const DEP: &[MutationAction] = &[
    MutationAction::DependencyAdd,
    MutationAction::DependencyRemove,
    MutationAction::PreferenceAdd,
    MutationAction::PreferenceRemove,
];
const TAG: &[MutationAction] = &[
    MutationAction::TagAdd,
    MutationAction::TagRemove,
    MutationAction::TagRename,
];
const MOVE: &[MutationAction] = &[MutationAction::Move];
const DROP: &[MutationAction] = &[MutationAction::DropTasks, MutationAction::DropCuts];
const ADD: &[MutationAction] = &[MutationAction::Add, MutationAction::Manifest];
const IMPORT: &[MutationAction] = &[MutationAction::Import];
const SUPERSEDE: &[MutationAction] = &[MutationAction::Supersede];
const REKEY: &[MutationAction] = &[MutationAction::Rekey];
const CLOSE: &[MutationAction] = &[MutationAction::Close, MutationAction::Restamp];
const AMEND: &[MutationAction] = &[
    MutationAction::AmendAdd,
    MutationAction::AmendReplace,
    MutationAction::AmendRemove,
];
const VERIFY_FIX: &[MutationAction] = &[MutationAction::VerifyFix];
const CAPTURE: &[MutationAction] = &[MutationAction::CaptureCreate];
const INBOX: &[MutationAction] = &[
    MutationAction::CaptureAmend,
    MutationAction::CaptureDiscard,
    MutationAction::CaptureRestore,
    MutationAction::CapturePromote,
];
const CHECK_BOUNDARIES: &[CommandBoundary] = &[CommandBoundary {
    name: "check --traces-root",
    aliases: &["check --traces-root", "alternate trace root"],
    reason: "the interactive advanced check can use or disable the loaded trace root, but selecting a different filesystem root remains an explicit CLI session boundary",
    example: "aplan check --traces-root .",
}];
const VERIFY_BOUNDARIES: &[CommandBoundary] = &[
    CommandBoundary {
        name: "verify --repo",
        aliases: &["verify --repo", "alternate evidence repository"],
        reason: "the interactive verifier is bound to the loaded repository identity; selecting another evidence repository requires an explicit CLI invocation",
        example: "aplan verify --repo .",
    },
    CommandBoundary {
        name: "verify --no-require-effort",
        aliases: &["verify --no-require-effort", "verification effort override"],
        reason: "the interactive verifier reports the loaded effort policy but does not weaken it for one run; that override remains explicit at the CLI boundary",
        example: "aplan verify --no-require-effort",
    },
    CommandBoundary {
        name: "verify --traces-root",
        aliases: &["verify --traces-root", "verification alternate trace root"],
        reason: "the interactive verifier is bound to the loaded trace policy and root; selecting another trace root remains an explicit CLI session boundary",
        example: "aplan verify --traces-root .",
    },
    CommandBoundary {
        name: "verify --no-traces",
        aliases: &["verify --no-traces", "verification without traces"],
        reason: "the interactive verifier reports the loaded trace policy but does not disable trace validation for one run; that override remains explicit at the CLI boundary",
        example: "aplan verify --no-traces",
    },
    CommandBoundary {
        name: "verify --deny-warnings",
        aliases: &["verify --deny-warnings", "verification deny warnings"],
        reason: "the interactive verifier uses the loaded warning verdict and does not strengthen it for one run; that override remains explicit at the CLI boundary",
        example: "aplan verify --deny-warnings",
    },
    CommandBoundary {
        name: "verify --no-deny-warnings",
        aliases: &["verify --no-deny-warnings", "verification allow warnings"],
        reason: "the interactive verifier uses the loaded warning verdict and does not weaken it for one run; that override remains explicit at the CLI boundary",
        example: "aplan verify --no-deny-warnings",
    },
    CommandBoundary {
        name: "verify --test-receipts",
        aliases: &["verify --test-receipts", "receipt-backed verification"],
        reason: "receipt import is a caller-owned path and trust boundary assembled by automation, so the interactive verifier displays retained provenance but does not select a manifest",
        example: "aplan verify --test-receipts receipts.json",
    },
    CommandBoundary {
        name: "verify --refresh-test-receipts",
        aliases: &[
            "verify --refresh-test-receipts",
            "refresh-base",
            "refresh-head",
            "refresh receipts",
        ],
        reason: "receipt refresh publishes an automation artifact against explicit Git revisions, so its set, base and head remain one deliberate CLI workflow",
        example: "aplan verify --refresh-test-receipts receipts.json --refresh-base HEAD~1 --refresh-head HEAD",
    },
    CommandBoundary {
        name: "verify --refresh-working-tree",
        aliases: &[
            "verify --refresh-working-tree",
            "refresh dirty worktree receipts",
        ],
        reason: "refreshing receipts from index and working-tree bytes is an explicit automation boundary rather than an action inferred from the loaded interactive snapshot",
        example: "aplan verify --refresh-working-tree --refresh-test-receipts receipts.json --refresh-base HEAD",
    },
    CommandBoundary {
        name: "verify --refresh-shadow",
        aliases: &["verify --refresh-shadow", "shadow receipt verification"],
        reason: "the complete direct shadow oracle can execute every configured provider and remains an explicit trust-bearing automation invocation",
        example: "aplan verify --refresh-shadow --refresh-test-receipts receipts.json --refresh-base HEAD~1 --refresh-head HEAD",
    },
];
const TEST_BOUNDARIES: &[CommandBoundary] = &[CommandBoundary {
    name: "test receipt",
    aliases: &["test receipt", "receipt", "portable test evidence"],
    reason: "receipt production executes one configured provider and binds external CI provenance, so it remains an explicit automation boundary rather than an interactive catalogue search",
    example: "aplan test receipt --help",
}];
const FACTS_BOUNDARIES: &[CommandBoundary] = &[CommandBoundary {
    name: "facts --require",
    aliases: &[
        "facts --require",
        "require capability",
        "capability assertion",
    ],
    reason: "About displays the typed capability inventory, while a process-exit assertion for automation remains an explicit machine-oriented CLI boundary",
    example: "aplan facts --require build.tui",
}];

/// The accepted interactive product contract, in the order published by facts.
///
/// The integration audit compares membership rather than trusting this order.
pub const COMMANDS: &[CommandDiscovery] = &[
    native_with_boundaries(
        "check",
        "Validate the whole plan",
        &[NativeSurface::Diagnostics(
            aplan_app::DiagnosticScope::Check,
        )],
        CHECK_BOUNDARIES,
    ),
    native_with_mutations_and_boundaries(
        "verify",
        "Resolve evidence and review repairs",
        &[NativeSurface::Verification],
        VERIFY_FIX,
        VERIFY_BOUNDARIES,
    ),
    native_with_boundaries(
        "test",
        "Search configured test evidence",
        &[NativeSurface::TestCatalogue],
        TEST_BOUNDARIES,
    ),
    native("ready", "Find startable work", &[NativeSurface::Work]),
    native("due", "Review scheduled work", &[NativeSurface::Work]),
    native("why", "Explain blocked work", &[NativeSurface::Work]),
    native(
        "version",
        "Review the next-version projection",
        &[NativeSurface::Report(aplan_app::ReportKind::Version)],
    ),
    native(
        "graph",
        "Explore the dependency graph",
        &[NativeSurface::Graph],
    ),
    CommandDiscovery {
        name: "tui",
        title: "Open the interactive terminal",
        disposition: CommandDisposition::EntryPoint {
            reason: "this command starts the current interface and cannot recursively appear as an invokable action inside itself",
            example: "aplan tui",
        },
        surfaces: &[],
        mutations: &[],
        boundaries: &[],
    },
    native("show", "Inspect one task", &[NativeSurface::Inspector]),
    native(
        "explain",
        "Explain a rule or vocabulary term",
        &[NativeSurface::Explanation],
    ),
    native(
        "tree",
        "Explore dependency ancestry",
        &[NativeSurface::Graph],
    ),
    native(
        "paths",
        "Review path specificity",
        &[NativeSurface::Report(aplan_app::ReportKind::Paths)],
    ),
    native(
        "vacuity",
        "Review validation coverage",
        &[NativeSurface::Report(aplan_app::ReportKind::Vacuity)],
    ),
    native(
        "stale",
        "Review stale work",
        &[NativeSurface::Report(aplan_app::ReportKind::Stale)],
    ),
    native(
        "deprecations",
        "Review deprecation runway",
        &[NativeSurface::Report(aplan_app::ReportKind::Deprecations)],
    ),
    CommandDiscovery {
        name: "attest",
        title: "Create a commit-bound attestation",
        disposition: CommandDisposition::CliOnly {
            reason: "attestation is a clean-commit batch artifact, and optional git-note attachment belongs to the explicit command boundary",
            example: "aplan attest --no-config",
        },
        surfaces: &[],
        mutations: &[],
        boundaries: &[],
    },
    CommandDiscovery {
        name: "export-agents-md",
        title: "Generate the agent contract",
        disposition: CommandDisposition::CliOnly {
            reason: "generated-document stdout, write and check modes are deterministic batch contracts rather than interactive plan actions",
            example: "aplan export-agents-md --check AGENTS.md",
        },
        surfaces: &[],
        mutations: &[],
        boundaries: &[],
    },
    CommandDiscovery {
        name: "export-roadmap-json",
        title: "Regenerate a roadmap JSON contract",
        disposition: CommandDisposition::CliOnly {
            reason: "the foreign roadmap file is both template and output, so its explicit path and check/write contract remain a batch workflow",
            example: "aplan export-roadmap-json --check roadmap.json",
        },
        surfaces: &[],
        mutations: &[],
        boundaries: &[],
    },
    CommandDiscovery {
        name: "export",
        title: "Export the plan",
        disposition: CommandDisposition::CliOnly {
            reason: "serialized stdout and file output are intended for pipes and generated artifacts rather than the full-screen renderer",
            example: "aplan export --format checklist",
        },
        surfaces: &[],
        mutations: &[],
        boundaries: &[],
    },
    native(
        "check-docs",
        "Check configured documents",
        &[NativeSurface::Diagnostics(
            aplan_app::DiagnosticScope::CheckDocs,
        )],
    ),
    native(
        "summary",
        "Review plan history",
        &[NativeSurface::Report(aplan_app::ReportKind::Summary)],
    ),
    mutation("capture", "Capture intake for later review", CAPTURE),
    native_with_mutations(
        "inbox",
        "Review and disposition captured intake",
        &[NativeSurface::Inbox],
        INBOX,
    ),
    mutation("drop", "Drop tasks or cuts", DROP),
    CommandDiscovery {
        name: "park",
        title: "Park portfolio work",
        disposition: CommandDisposition::CliOnly {
            reason: "portfolio parking is an all-or-none claim-aware batch whose shared reason, optional schedule and tags require one explicit command until the TUI has a complete batch review form",
            example: "aplan park T-123 --reason \"deferred until the next cycle\"",
        },
        surfaces: &[],
        mutations: &[],
        boundaries: &[],
    },
    CommandDiscovery {
        name: "unpark",
        title: "Return parked portfolio work to todo",
        disposition: CommandDisposition::CliOnly {
            reason: "portfolio unparking is an all-or-none claim-aware batch that clears scheduling metadata, so it remains an explicit command until the TUI can review the complete target set",
            example: "aplan unpark T-123",
        },
        surfaces: &[],
        mutations: &[],
        boundaries: &[],
    },
    mutation("claim", "Claim a task", CLAIM),
    mutation("release", "Release a task claim", RELEASE),
    mutation("start", "Start a task", START),
    mutation("stop", "Stop a task", STOP),
    mutation("mv", "Move tasks", MOVE),
    mutation("rekey", "Rekey a task", REKEY),
    native("tasks", "Browse tasks", &[NativeSurface::Tasks]),
    native(
        "critical-path",
        "Inspect the critical route",
        &[NativeSurface::Graph],
    ),
    native(
        "roadmap",
        "Review the roadmap",
        &[NativeSurface::Report(aplan_app::ReportKind::Roadmap)],
    ),
    mutation("close", "Close a task with evidence", CLOSE),
    mutation("amend", "Amend task evidence", AMEND),
    mutation("supersede", "Supersede an outcome", SUPERSEDE),
    mutation("dep", "Edit task dependencies", DEP),
    mutation("tag", "Edit task tags", TAG),
    mutation("import", "Import proposal tasks", IMPORT),
    CommandDiscovery {
        name: "blueprint",
        title: "Apply an explicit blueprint file",
        disposition: CommandDisposition::CliOnly {
            reason: "blueprint application captures a caller-selected local file and renders its complete predicted graph before an explicit guarded batch write, so it remains a copyable command until the TUI has a file picker and full multi-task review form",
            example: "aplan blueprint apply --file feature.yml --cut M-next --dry-run",
        },
        surfaces: &[],
        mutations: &[],
        boundaries: &[],
    },
    mutation("add", "Add tasks", ADD),
    CommandDiscovery {
        name: "mcp",
        title: "Serve the MCP transport",
        disposition: CommandDisposition::CliOnly {
            reason: "the MCP server owns standard input and output as a long-running machine transport rather than an interactive plan action",
            example: "aplan mcp",
        },
        surfaces: &[],
        mutations: &[],
        boundaries: &[],
    },
    CommandDiscovery {
        name: "init",
        title: "Initialize a plan",
        disposition: CommandDisposition::Guided {
            reason: "setup renders the selected target and a non-overwriting command, then retries discovery after the user performs it",
            example: "aplan init .",
        },
        surfaces: &[],
        mutations: &[],
        boundaries: &[],
    },
    CommandDiscovery {
        name: "facts",
        title: "Inspect build capabilities",
        disposition: CommandDisposition::Guided {
            reason: "About renders the typed capability document and build identity without duplicating the machine-oriented assertion mode",
            example: "aplan facts --identity",
        },
        surfaces: &[],
        mutations: &[],
        boundaries: FACTS_BOUNDARIES,
    },
    CommandDiscovery {
        name: "schema",
        title: "Print the JSON schema",
        disposition: CommandDisposition::CliOnly {
            reason: "schema output is a machine-consumed stdout artifact whose exact serialization is the purpose of the command",
            example: "aplan schema",
        },
        surfaces: &[],
        mutations: &[],
        boundaries: &[],
    },
];

const fn native(
    name: &'static str,
    title: &'static str,
    surfaces: &'static [NativeSurface],
) -> CommandDiscovery {
    CommandDiscovery {
        name,
        title,
        disposition: CommandDisposition::Native,
        surfaces,
        mutations: &[],
        boundaries: &[],
    }
}

const fn native_with_boundaries(
    name: &'static str,
    title: &'static str,
    surfaces: &'static [NativeSurface],
    boundaries: &'static [CommandBoundary],
) -> CommandDiscovery {
    CommandDiscovery {
        name,
        title,
        disposition: CommandDisposition::Native,
        surfaces,
        mutations: &[],
        boundaries,
    }
}

const fn native_with_mutations(
    name: &'static str,
    title: &'static str,
    surfaces: &'static [NativeSurface],
    mutations: &'static [MutationAction],
) -> CommandDiscovery {
    CommandDiscovery {
        name,
        title,
        disposition: CommandDisposition::Native,
        surfaces,
        mutations,
        boundaries: &[],
    }
}

const fn native_with_mutations_and_boundaries(
    name: &'static str,
    title: &'static str,
    surfaces: &'static [NativeSurface],
    mutations: &'static [MutationAction],
    boundaries: &'static [CommandBoundary],
) -> CommandDiscovery {
    CommandDiscovery {
        name,
        title,
        disposition: CommandDisposition::Native,
        surfaces,
        mutations,
        boundaries,
    }
}

const fn mutation(
    name: &'static str,
    title: &'static str,
    actions: &'static [MutationAction],
) -> CommandDiscovery {
    native_with_mutations(name, title, &[], actions)
}

#[must_use]
pub fn command(name: &str) -> Option<&'static CommandDiscovery> {
    COMMANDS.iter().find(|command| command.name == name)
}

/// Resolve a top-level or nested non-native boundary by its unique name.
#[must_use]
pub fn boundary(name: &str) -> Option<(&'static str, &'static str)> {
    command(name)
        .and_then(|command| command.disposition.boundary())
        .or_else(|| {
            COMMANDS
                .iter()
                .flat_map(|command| command.boundaries)
                .find(|boundary| boundary.name == name)
                .map(|boundary| (boundary.reason, boundary.example))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_distinguishes_commands_nested_boundaries_and_unknown_names() {
        assert_eq!(command("check").map(|command| command.name), Some("check"));
        assert!(command("test receipt").is_none());
        assert!(command("not-a-command").is_none());

        assert_eq!(
            boundary("schema").map(|(_, example)| example),
            Some("aplan schema")
        );
        assert_eq!(
            boundary("test receipt").map(|(_, example)| example),
            Some("aplan test receipt --help")
        );
        assert!(boundary("check").is_none());
        assert!(boundary("not-a-boundary").is_none());
        assert_eq!(CommandDisposition::Native.label(), "native");
    }
}
