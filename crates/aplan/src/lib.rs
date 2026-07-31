//! `aplan`, spec-driven plan validator, as a library so its CLI is introspectable
//! (`facts()` reads the clap command tree) and testable; `src/main.rs` is a thin
//! shim over [`run_cli`].
//!
//! Exit codes are a contract (ADR-0007), four codes so a consumer's CI can tell
//! *your* fault from *aplan's*:
//!
//! | code | meaning |
//! |------|---------|
//! | 0    | clean (warnings allowed) |
//! | 1    | at least one error |
//! | 2    | usage or configuration error (your input) |
//! | 3    | internal error (a bug in aplan) |

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use aplan_core::{Graph, Options, load_plan, validate};
use aplan_output::{
    Format, mermaid, render, render_critical_path, render_roadmap, render_roadmap_markdown,
    render_summary, render_verified,
};
use clap::{CommandFactory, Parser, Subcommand};

const EXIT_OK: u8 = 0;
const EXIT_VIOLATIONS: u8 = 1;
/// A usage or configuration error: the invocation, the input, or `.aplan.yml` is
/// wrong. The default for any error the CLI surfaces, since most are the user's.
const EXIT_USAGE: u8 = 2;
/// An internal error: aplan itself misbehaved (e.g. produced output that fails its
/// own self-check). Reserved for errors tagged [`Bug`]; a consumer can treat this
/// as "file a bug", distinct from `2` ("fix your input").
const EXIT_INTERNAL: u8 = 3;

/// The default plan directory when neither `--root` nor `.aplan.yml`'s `root` key
/// sets one. Namespaced and visible (ADR-0010): the plan is durable, hand-edited,
/// reviewed-in-the-diff content, so it lives in a named directory, not a hidden or
/// generic one that could collide with an existing `plan/`.
const DEFAULT_ROOT: &str = "aplan";

/// A marker attached (via `anyhow` context) to an error that is a bug in aplan,
/// not a usage/config problem, so [`run_cli`] maps it to [`EXIT_INTERNAL`]. Most
/// CLI errors are the user's, so the default is [`EXIT_USAGE`]; only aplan's own
/// invariant failures are tagged.
#[derive(Debug)]
struct Bug;

impl std::fmt::Display for Bug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "this is a bug in aplan, please report it")
    }
}

impl std::error::Error for Bug {}

#[derive(Parser, Debug)]
#[command(name = "aplan", version, about, long_about = None)]
struct Cli {
    /// Directory containing the plan's task files (default: `aplan`, or the `root`
    /// key in `.aplan.yml`).
    #[arg(long, global = true)]
    root: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Validate the plan. Fails on any error. Reads `.aplan.yml` if present;
    /// flags below override it.
    Check {
        /// Output format: human, json, agent, github.
        #[arg(long, default_value = "human")]
        format: String,
        /// Do not warn about tasks lacking an `effort_days` estimate.
        #[arg(long)]
        no_require_effort: bool,
        /// Resolve `traces` anchors against this root (default: the current
        /// directory, i.e. the repo root, unless `.aplan.yml` sets it).
        #[arg(long)]
        traces_root: Option<PathBuf>,
        /// Skip `APLAN008` entirely, do not resolve `traces` anchors.
        #[arg(long, conflicts_with = "traces_root")]
        no_traces: bool,
    },
    /// Resolve evidence against the git repository (L1/L2, and L3 with
    /// `--execute`). Unlike `check`, this opens the repo: a `commit` must exist
    /// and touch the task's `paths`, a `file` must exist at the cited rev, a
    /// `test` must be one the configured runner lists.
    Verify {
        /// Output format: human, json, agent, github.
        #[arg(long, default_value = "human")]
        format: String,
        /// Repository to resolve evidence against (default: the current directory).
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Run `test` evidence (rung L3), not just resolve it. Off by default;
        /// CI must not enable it for pull requests from forks (see SECURITY.md).
        #[arg(long)]
        execute: bool,
    },
    /// List tasks whose blockers are all `done`.
    Ready,
    /// Explain what is blocking a task, transitively.
    Why {
        /// Task id.
        id: String,
    },
    /// Print the dependency graph as Mermaid.
    Graph,
    /// Roll the plan up by phase: tasks, effort, and how many are ready.
    Summary {
        /// Output format: human (a table) or json.
        #[arg(long, default_value = "human")]
        format: String,
    },
    /// Show the critical path: the longest `effort_days`-weighted chain of
    /// dependent tasks. Advisory, never a gate.
    CriticalPath {
        /// Output format: human (a table) or json.
        #[arg(long, default_value = "human")]
        format: String,
    },
    /// Render the plan grouped by cut (the milestones): per-cut rollups, the
    /// critical path, and cross-cutting tasks. Advisory (ADR-0009).
    Roadmap {
        /// Output format: human, json, or markdown (the committed companion).
        #[arg(long, default_value = "human")]
        format: String,
    },
    /// Close a task: set it `done` and attach evidence, in place. With no
    /// `--evidence`, records the current `HEAD` commit.
    Close {
        /// Task id to close.
        id: String,
        /// Evidence as `kind:ref`, e.g. `commit:d213c9a`, `test:mod::name`,
        /// `file:path`. Repeatable. Omit to derive the `HEAD` commit.
        #[arg(long)]
        evidence: Vec<String>,
        /// Repository to read `HEAD` from (default: the current directory).
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Re-stamp a `done` task's `traces` fingerprints (`APLAN018`), after
        /// re-reviewing a section that drifted. Does not change status or evidence.
        #[arg(long, conflicts_with = "evidence")]
        restamp: bool,
    },
    /// Import proposals from another tool's format into `aplan` tasks. Lossy by
    /// design, and says so: every field the task model cannot hold is reported
    /// (ADR-0012).
    Import(ImportArgs),
    /// Scaffold a starter `.aplan.yml`, refusing to overwrite an existing one.
    Init {
        /// Directory to write `.aplan.yml` into (default: the current directory).
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Print this tool's facts as JSON: its rules, output formats, commands, and
    /// evidence kinds, generated from the code (the source for `facts.json`).
    Facts,
    /// Print the JSON Schema derived from the Rust types.
    Schema,
}

/// The CLI entry point. `src/main.rs` is a thin shim over this.
pub fn run_cli() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("aplan: {e:#}");
            ExitCode::from(exit_code_for(&e))
        }
    }
}

/// The exit code for a surfaced error: [`EXIT_INTERNAL`] (3) if the chain carries
/// the [`Bug`] marker, else [`EXIT_USAGE`] (2). A usage/config problem is the
/// user's to fix; an internal bug is aplan's to fix, and a consumer's CI can tell
/// them apart.
fn exit_code_for(err: &anyhow::Error) -> u8 {
    // `anyhow::Error::downcast_ref` searches the whole context chain, so it finds
    // the `Bug` tag wherever it was attached, even under a later `.context(...)`.
    if err.downcast_ref::<Bug>().is_some() {
        EXIT_INTERNAL
    } else {
        EXIT_USAGE
    }
}

/// A `CloseError` as a CLI error. `Produced` means `close`/`restamp` failed its
/// own self-check (it built frontmatter that would not load), which is aplan's
/// bug, so it is tagged [`Bug`] (exit 3); every other variant is a usage problem
/// with the task's state (exit 2).
fn close_err(action: &str, id: &str, e: &aplan_core::CloseError) -> anyhow::Error {
    let err = anyhow::anyhow!("cannot {action} `{id}`: {e}");
    if matches!(e, aplan_core::CloseError::Produced(_)) {
        err.context(Bug)
    } else {
        err
    }
}

/// Write `s` to stdout, treating a closed downstream pipe (e.g. `aplan ready |
/// head`) as a clean exit rather than the panic `println!` would raise.
fn write_stdout(s: &str) -> Result<()> {
    match std::io::stdout().write_all(s.as_bytes()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Load the plan at `root`, refusing a root that is not a directory.
///
/// A missing root used to load as an *empty, clean* plan: `aplan check --root
/// typo` printed `ok: 0 tasks` and exited 0, so a renamed plan directory left CI
/// green forever while validating nothing. A gate that silently disables itself
/// is worse than no gate, so this is a usage error (exit 2).
fn load_checked(root: &std::path::Path) -> Result<aplan_core::load::Plan> {
    if !root.is_dir() {
        anyhow::bail!(
            "plan root `{}` is not a directory, nothing would be checked. \
             Pass `--root <dir>` to point at your plan.",
            root.display()
        );
    }
    Ok(load_plan(root))
}

/// Resolve the plan root. Precedence: the `--root` flag, then the `root` key in
/// `.aplan.yml` (discovered by walking up from the current directory; a relative
/// value is resolved against the config's directory, so a repo pins its plan
/// location once), then [`DEFAULT_ROOT`]. `schema`/`facts`/`init` never call this,
/// so they never read `.aplan.yml` and cannot fail on a missing or invalid one.
fn resolve_root(cli_root: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(r) = cli_root {
        return Ok(r);
    }
    let (config, config_dir) =
        aplan_config::load_config_located(std::path::Path::new(".")).map_err(config_err)?;
    Ok(root_from_config(config.root, config_dir.as_deref()))
}

/// Resolve the default root from the config's `root` value: a relative value is
/// joined to the config directory, an absolute value is used as-is, and an absent
/// one falls back to [`DEFAULT_ROOT`]. Split out so the precedence and relative-join
/// logic is unit-testable without touching the current directory.
fn root_from_config(config_root: Option<String>, config_dir: Option<&std::path::Path>) -> PathBuf {
    match config_root {
        Some(r) => {
            let p = PathBuf::from(r);
            match config_dir {
                Some(dir) if p.is_relative() => dir.join(p),
                _ => p,
            }
        }
        None => PathBuf::from(DEFAULT_ROOT),
    }
}

/// Whether the plan loaded cleanly; if not, print the load errors to stderr.
///
/// Commands other than `check` answer questions *about* the plan, and must not
/// answer them from a plan that did not load: `aplan ready` printed a confident
/// list, and exited 0, on a plan `check` rejects. A partial answer that looks
/// whole is the quiet lie this tool exists to prevent, so the caller stops with
/// [`EXIT_VIOLATIONS`], the same verdict `check` gives the same plan, rather
/// than [`EXIT_USAGE`], which would claim the invocation was wrong.
fn loaded_cleanly(plan: &aplan_core::load::Plan) -> bool {
    let errors: Vec<_> = plan
        .load_diagnostics
        .iter()
        .filter(|d| d.severity == aplan_core::Severity::Error)
        .collect();
    if errors.is_empty() {
        return true;
    }
    eprintln!(
        "aplan: {} task file(s) failed to load, any answer would be incomplete. \
         Run `aplan check` for details:",
        errors.len()
    );
    for d in errors {
        match &d.file {
            Some(f) => eprintln!("  {}: {} ({})", d.code, d.message, f.display()),
            None => eprintln!("  {}: {}", d.code, d.message),
        }
    }
    false
}

/// Run a renderer, treating a closed downstream pipe (`aplan check | head`) as
/// success rather than an exit-2 "internal failure".
fn render_tolerant(render: impl FnOnce() -> std::io::Result<()>) -> Result<()> {
    match render() {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(anyhow::Error::new(e).context("rendering report")),
    }
}

/// The exit-code contract for a report: clean → 0, at least one error → 1.
fn verdict(report: &aplan_core::Report) -> u8 {
    if report.is_clean() {
        EXIT_OK
    } else {
        EXIT_VIOLATIONS
    }
}

/// Merge `.aplan.yml` with the check flags into the validator options.
/// Precedence: an explicit flag wins over the file, which wins over the built-in
/// default. `traces` defaults to on, rooted at the current directory.
fn resolve_check_options(
    config: &aplan_config::Config,
    config_dir: Option<&std::path::Path>,
    no_require_effort: bool,
    traces_root: Option<PathBuf>,
    no_traces: bool,
) -> Options {
    let require_effort = if no_require_effort {
        false
    } else {
        config.require_effort.unwrap_or(true)
    };

    let traces_root = if no_traces {
        None
    } else if let Some(p) = traces_root {
        Some(p)
    } else if config.no_traces == Some(true) {
        None
    } else if let Some(p) = &config.traces_root {
        // A relative `traces_root` from the file is resolved against the
        // `.aplan.yml` directory, so `traces_root: docs` means the same thing
        // whatever directory the command is invoked from.
        let p = PathBuf::from(p);
        match config_dir {
            Some(dir) if p.is_relative() => Some(dir.join(p)),
            _ => Some(p),
        }
    } else {
        Some(PathBuf::from("."))
    };

    // APLAN017 is config-only (no flag): the runner and the strength policy are
    // repo policy, not per-run knobs.
    let (require_min_strength, exempt_tags) = match &config.evidence {
        Some(ev) => (
            ev.require_min_strength.clone(),
            ev.exempt_tags.clone().unwrap_or_default(),
        ),
        None => (None, Vec::new()),
    };

    Options {
        require_effort,
        traces_root,
        require_min_strength,
        exempt_tags,
        // APLAN018 is config-only (repo policy, not a per-run knob), like APLAN017.
        detect_drift: config.detect_drift.unwrap_or(false),
        // APLAN022 is config-only too: membership totality is a repo policy.
        require_cut_membership: config.require_cut_membership.unwrap_or(false),
    }
}

/// `aplan check`, hermetic structural validation (rung L0). No git, no network,
/// no subprocess.
fn cmd_check(
    root: &std::path::Path,
    format: &str,
    no_require_effort: bool,
    traces_root: Option<PathBuf>,
    no_traces: bool,
) -> Result<u8> {
    let format: Format = format.parse().map_err(anyhow::Error::msg)?;
    let plan = load_checked(root)?;
    // `.aplan.yml` is found by walking up from the plan directory, so `check` and
    // `verify` converge on the same top-level file. Absent is fine; an unknown key
    // in it is an error, never silently ignored.
    let (config, config_dir) = aplan_config::load_config_located(root).map_err(config_err)?;
    let opts = resolve_check_options(
        &config,
        config_dir.as_deref(),
        no_require_effort,
        traces_root,
        no_traces,
    );
    let report = validate(&plan, &opts);
    render_tolerant(|| render(&report, format))?;
    Ok(verdict(&report))
}

/// Surface a `.aplan.yml` problem as a usage error (exit 2), with context.
fn config_err(e: aplan_config::Error) -> anyhow::Error {
    anyhow::Error::new(e).context("loading .aplan.yml")
}

/// `aplan verify`, resolve evidence against the repository (rungs L1/L2, and L3
/// with `--execute`).
fn cmd_verify(
    root: &std::path::Path,
    format: &str,
    repo: &std::path::Path,
    execute: bool,
) -> Result<u8> {
    let format: Format = format.parse().map_err(anyhow::Error::msg)?;
    let plan = load_checked(root)?;
    // Verify cannot speak honestly about a plan that did not load, so it refuses
    // rather than resolve a partial one, the stance `ready`/`why`/`graph` take.
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    // The runner template comes only from the trusted `.aplan.yml`, never from a
    // task file (HLD §5.5).
    let config = aplan_config::load_config(repo).map_err(config_err)?;
    if execute && config.test.is_none() {
        eprintln!(
            "aplan: --execute was given but `.aplan.yml` configures no `test` runner; \
             `test` evidence cannot be run."
        );
    }
    let opts = aplan_evidence::VerifyOptions {
        execute,
        test_runner: config.test,
    };
    let report = aplan_evidence::verify(&plan, repo, &opts)
        .context("resolving evidence against the repository")?;
    render_tolerant(|| render_verified(&report, format))?;
    Ok(verdict(&report))
}

fn run() -> Result<u8> {
    let cli = Cli::parse();
    // Resolved lazily inside the arms that read the plan; `schema`/`facts`/`init`
    // do not, so they never touch `.aplan.yml`.
    let cli_root = cli.root;

    match cli.command {
        Command::Schema => {
            let schema = aplan_core::load::derived_schema();
            write_stdout(&format!("{}\n", serde_json::to_string_pretty(&schema)?))?;
            Ok(EXIT_OK)
        }

        Command::Check {
            format,
            no_require_effort,
            traces_root,
            no_traces,
        } => cmd_check(
            &resolve_root(cli_root)?,
            &format,
            no_require_effort,
            traces_root,
            no_traces,
        ),

        Command::Verify {
            format,
            repo,
            execute,
        } => cmd_verify(&resolve_root(cli_root)?, &format, &repo, execute),

        Command::Ready => {
            let plan = load_checked(&resolve_root(cli_root)?)?;
            if !loaded_cleanly(&plan) {
                return Ok(EXIT_VIOLATIONS);
            }
            let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
            let graph = Graph::new(tasks.iter().copied());
            let mut out = String::new();
            for t in graph.ready() {
                let effort = t
                    .effort_days
                    .map_or_else(String::new, |e| format!(" ({e}d)"));
                let _ = writeln!(out, "{}  {}{}", t.id, t.title, effort);
            }
            write_stdout(&out)?;
            Ok(EXIT_OK)
        }

        Command::Why { id } => {
            let plan = load_checked(&resolve_root(cli_root)?)?;
            if !loaded_cleanly(&plan) {
                return Ok(EXIT_VIOLATIONS);
            }
            let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
            let graph = Graph::new(tasks.iter().copied());

            let Some(task) = graph.get(&id) else {
                anyhow::bail!("unknown task `{id}`");
            };
            let blockers = graph.why_blocked(&id);
            let unknown = graph.direct_unknown_blockers(&id);
            // A parked task is blocked by a decision, not by an edge. Saying
            // "ready: nothing blocks it" about a task whose status is literally
            // `blocked` is the tool contradicting its own plan.
            let parked = !task.status.is_workable() && !task.status.is_terminal();
            let mut out = String::new();
            if parked {
                let _ = writeln!(
                    out,
                    "{id} is `blocked`: explicitly parked, not waiting on the graph"
                );
            } else if blockers.is_empty() && unknown.is_empty() {
                let _ = writeln!(out, "{id} is ready: nothing blocks it");
            }
            if !blockers.is_empty() || !unknown.is_empty() {
                let _ = writeln!(out, "{id} is blocked by:");
                for b in blockers {
                    let _ = writeln!(out, "  {} [{}]  {}", b.id, b.status, b.title);
                }
                for u in unknown {
                    let _ = writeln!(out, "  {u} [unknown]  referenced but not defined");
                }
            }
            write_stdout(&out)?;
            Ok(EXIT_OK)
        }

        Command::Graph => {
            let plan = load_checked(&resolve_root(cli_root)?)?;
            if !loaded_cleanly(&plan) {
                return Ok(EXIT_VIOLATIONS);
            }
            let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
            let graph = Graph::new(tasks.iter().copied());
            write_stdout(&mermaid(&graph))?;
            Ok(EXIT_OK)
        }

        Command::Summary { format } => cmd_summary(&resolve_root(cli_root)?, &format),

        Command::CriticalPath { format } => cmd_critical_path(&resolve_root(cli_root)?, &format),
        Command::Roadmap { format } => cmd_roadmap(&resolve_root(cli_root)?, &format),

        Command::Close {
            id,
            evidence,
            repo,
            restamp,
        } => cmd_close(&resolve_root(cli_root)?, &id, &evidence, &repo, restamp),

        Command::Import(args) => cmd_import(&resolve_root(cli_root)?, &args),
        Command::Init { dir } => cmd_init(&dir),

        Command::Facts => cmd_facts(),
    }
}

/// Parse one `kind:ref` evidence argument. `note` is refused: it needs a dated
/// `captured`, and the point of `close` is to record *strong*, derivable proof.
fn parse_evidence(arg: &str) -> Result<aplan_core::Evidence> {
    let (kind, reference) = arg
        .split_once(':')
        .filter(|(k, r)| !k.is_empty() && !r.is_empty())
        .ok_or_else(|| anyhow::anyhow!("evidence must be `kind:ref` (got `{arg}`)"))?;
    let kind = match kind {
        "commit" => aplan_core::EvidenceKind::Commit,
        "test" => aplan_core::EvidenceKind::Test,
        "file" => aplan_core::EvidenceKind::File,
        "url" => aplan_core::EvidenceKind::Url,
        "note" => anyhow::bail!(
            "`note` evidence needs a `captured` date; add it by hand rather than via `close`"
        ),
        other => anyhow::bail!("unknown evidence kind `{other}` (commit, test, file, url)"),
    };
    Ok(aplan_core::Evidence {
        kind,
        reference: reference.to_string(),
        captured: None,
    })
}

/// `aplan close`, set a task `done` and attach evidence, in place.
/// Everything `aplan import` was asked to do, as one `clap` group so the
/// dispatch stays a single line and the flags live beside the command that owns
/// them.
#[derive(clap::Args, Debug)]
pub struct ImportArgs {
    /// Source document to read (for `--format kep`, a `kep.yaml`).
    source: PathBuf,
    /// Source shape to expect.
    #[arg(long, default_value = "kep")]
    format: String,
    /// Directory to write task files into (default: `<root>/tasks` when it
    /// exists, else the plan root).
    #[arg(long)]
    out: Option<PathBuf>,
    /// Print what would be imported, and write nothing.
    #[arg(long)]
    dry_run: bool,
    /// `phase` for the imported tasks. Proposal repositories have no equivalent
    /// field, and `phase` gates nothing.
    #[arg(long, default_value_t = 1)]
    phase: u32,
    /// How to treat a source that claims `implemented`: `todo` (the default,
    /// because a claim is not proof) or `done`, which needs `--as-of` and
    /// records note-level provenance only.
    #[arg(long, default_value = "todo")]
    implemented_as: String,
    /// ISO-8601 `YYYY-MM-DD` recorded as the note's `captured` date. Only
    /// meaningful with `--implemented-as done`.
    #[arg(long)]
    as_of: Option<String>,
    /// Milestone (cut) to add the imported ids to, so they are not left in no
    /// cut. Refuses a cut that is already `done` or `dropped`.
    #[arg(long)]
    cut: Option<String>,
}

/// Turn the validated flags into the importer's options.
///
/// Split out so the invocation is checked in one place, before anything is
/// read or written: a bad `--as-of` is a usage error, not a task file that
/// fails `APLAN010` after it lands.
fn import_options(args: &ImportArgs) -> Result<aplan_interop::Options> {
    use aplan_interop::ImplementedAs;

    if args.format != "kep" {
        anyhow::bail!("unknown --format `{}` (this build reads: kep)", args.format);
    }
    if args.as_of.is_some() && args.implemented_as != "done" {
        anyhow::bail!("`--as-of` only applies with `--implemented-as done`");
    }

    let implemented_as = match args.implemented_as.as_str() {
        "todo" => ImplementedAs::Todo,
        "done" => {
            let as_of = args.as_of.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "`--implemented-as done` needs `--as-of YYYY-MM-DD`: a `note` must carry \
                     the date it was captured (APLAN010), and aplan does not invent dates"
                )
            })?;
            if !aplan_core::is_iso_date(as_of) {
                anyhow::bail!("`--as-of {as_of}` is not an ISO-8601 date (YYYY-MM-DD)");
            }
            ImplementedAs::Done {
                as_of: as_of.to_string(),
            }
        }
        other => anyhow::bail!("unknown --implemented-as `{other}` (expected: todo, done)"),
    };

    Ok(aplan_interop::Options {
        phase: args.phase,
        implemented_as,
    })
}

/// Read one proposal document into tasks, report what could not be carried, and
/// write the tasks unless this is a dry run (ADR-0012).
/// The consequences a reader should hear once per run rather than once per task.
///
/// Both are warnings rather than errors, and both name the fix: an import that
/// stayed quiet about them would leave the user to discover its own side effects
/// from a later `check`.
fn push_run_notices(report: &mut aplan_interop::Report, source_name: &str, placed_in_cut: bool) {
    use aplan_interop::report::{Notice, NoticeKind};

    if !placed_in_cut {
        report.push(Notice::new(
            source_name,
            NoticeKind::Placement,
            "imported tasks belong to no cut; where `require_cut_membership` is set, `check` \
             reports APLAN022 until `--cut <milestone>` or a `cross-cutting` tag places them",
        ));
    }
    report.push(Notice::new(
        source_name,
        NoticeKind::Placement,
        "imported tasks carry no `effort_days`, because there is nothing to derive one from; \
         `check` warns (APLAN009) until you estimate them",
    ));
}

fn cmd_import(root: &std::path::Path, args: &ImportArgs) -> Result<u8> {
    use aplan_interop::{ProposalSource, cut, emit, kep::Kep, report};

    let options = import_options(args)?;
    let source_name = args.source.display().to_string();
    let contents =
        std::fs::read_to_string(&args.source).with_context(|| format!("reading {source_name}"))?;
    let imported = Kep.parse(&source_name, &contents, &options)?;

    // --- refuse to collide with a task that already exists -------------------
    // `emit` refuses by path; identity is the `id` field, not the path
    // (ADR-0010), so the id check has to happen here, where the plan is loaded.
    let plan = aplan_core::load_plan(root);
    let mut report = imported.report;
    for task in &imported.tasks {
        if let Some(existing) = plan.tasks.iter().find(|lt| lt.task.id == task.id) {
            anyhow::bail!(
                "task `{}` already exists in {}; import will not overwrite it",
                task.id,
                existing.path.display()
            );
        }
    }

    // Land beside the plan's existing tasks. The loader walks the root
    // recursively, so either location works, but writing to `<root>/tasks` when
    // that is where the tasks live is what a reader expects; a plan that keeps
    // its tasks directly in the root gets them there instead.
    let out_dir = args.out.clone().unwrap_or_else(|| {
        let conventional = root.join("tasks");
        if conventional.is_dir() {
            conventional
        } else {
            root.to_path_buf()
        }
    });

    // --- say what will happen, or make it happen ----------------------------
    let mut lines = String::new();
    for task in &imported.tasks {
        let _ = writeln!(
            lines,
            "  {}  {}  [{}]  {}\n    -> {}",
            task.id,
            status_word(task.status),
            task.tags.join(", "),
            task.title,
            emit::task_path(&out_dir, &task.id).display()
        );
    }

    // Validate the destination cut BEFORE writing anything. `cut::place` guards
    // itself, but it runs after the task files are written, so a refusal there
    // would strand freshly written tasks in no cut behind an error implying
    // nothing had happened. Checking first makes the whole import all-or-nothing,
    // and makes `--dry-run` catch a settled cut instead of promising a run that
    // would fail.
    if let Some(cut_id) = &args.cut {
        cut::check_target(&plan, cut_id)?;
    }

    let ids: Vec<String> = imported.tasks.iter().map(|t| t.id.clone()).collect();

    if args.dry_run {
        if let Some(cut_id) = &args.cut {
            report.push(report::Notice::new(
                &source_name,
                report::NoticeKind::Placement,
                format!("would add {} to `{cut_id}`", ids.join(", ")),
            ));
        }
    } else {
        for task in &imported.tasks {
            let body = emit::provenance_body(&source_name, Kep.flavor());
            emit::write_task(&out_dir, task, &body)?;
        }
        if let Some(cut_id) = &args.cut {
            // Reload: the tasks just written are the ones being placed.
            let plan = aplan_core::load_plan(root);
            let placed = cut::place(&plan, cut_id, &ids)?;
            report.push(report::Notice::new(
                &source_name,
                report::NoticeKind::Placement,
                format!(
                    "added {} to `{cut_id}` in {}",
                    placed.added.join(", "),
                    placed.path.display()
                ),
            ));
        }
    }

    push_run_notices(&mut report, &source_name, args.cut.is_some());

    let verb = if args.dry_run {
        "would import"
    } else {
        "imported"
    };
    let mut outp = format!(
        "{verb} {} task(s) from {source_name}\n{lines}",
        imported.tasks.len()
    );
    if !report.is_empty() {
        outp.push_str("report:\n");
        outp.push_str(&report.to_string());
    }
    if args.dry_run {
        outp.push_str("(dry run: nothing was written)\n");
    }
    write_stdout(&outp)?;
    Ok(EXIT_OK)
}

/// The `status:` value as written in a task file.
fn status_word(s: aplan_core::Status) -> &'static str {
    use aplan_core::Status;
    match s {
        Status::Todo => "todo",
        Status::InProgress => "in_progress",
        Status::Blocked => "blocked",
        Status::Done => "done",
        Status::Dropped => "dropped",
    }
}

fn cmd_close(
    root: &std::path::Path,
    id: &str,
    evidence_args: &[String],
    repo: &std::path::Path,
    restamp: bool,
) -> Result<u8> {
    let plan = load_checked(root)?;
    // Never edit a task inside a plan that does not load, its neighbours might be
    // the reason this one should not close.
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }

    let lt = plan
        .tasks
        .iter()
        .find(|lt| lt.task.id == id)
        .ok_or_else(|| anyhow::anyhow!("unknown task `{id}`"))?;

    // Config drives drift stamping (`APLAN018`): whether it is on, and the
    // `traces_root` the traced documents are read from.
    let (config, config_dir) = aplan_config::load_config_located(root).map_err(config_err)?;
    let opts = resolve_check_options(&config, config_dir.as_deref(), false, None, false);
    let fingerprints = if opts.detect_drift {
        trace_fingerprints(&lt.task, opts.traces_root.as_deref())
    } else {
        std::collections::BTreeMap::new()
    };

    let original = std::fs::read_to_string(&lt.path)
        .with_context(|| format!("reading {}", lt.path.display()))?;

    // `--restamp`: re-take a `done` task's trace fingerprints, nothing else.
    if restamp {
        let updated = aplan_core::restamp(&original, &fingerprints)
            .map_err(|e| close_err("restamp", id, &e))?;
        std::fs::write(&lt.path, updated)
            .with_context(|| format!("writing {}", lt.path.display()))?;
        write_stdout(&format!(
            "restamped {id}: {} trace fingerprint(s)\n",
            fingerprints.len()
        ))?;
        return Ok(EXIT_OK);
    }

    // A task whose blockers are not all `done` cannot legitimately close: doing so
    // would create the `APLAN007` this tool exists to catch.
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    let unmet: Vec<&str> = lt
        .task
        .blocked_by
        .iter()
        .filter(|b| !graph.is_done(b))
        .map(String::as_str)
        .collect();
    if !unmet.is_empty() {
        anyhow::bail!(
            "cannot close `{id}`: blocker(s) not done: {}",
            unmet.join(", ")
        );
    }

    // Evidence: the explicit list, or the current HEAD commit derived for free.
    let evidence = if evidence_args.is_empty() {
        let sha = aplan_evidence::head_sha(repo)
            .context("deriving HEAD (pass `--evidence kind:ref` to give it explicitly)")?;
        vec![aplan_core::Evidence {
            kind: aplan_core::EvidenceKind::Commit,
            reference: sha,
            captured: None,
        }]
    } else {
        evidence_args
            .iter()
            .map(|a| parse_evidence(a))
            .collect::<Result<Vec<_>>>()?
    };

    let updated = aplan_core::close(&original, &evidence, &fingerprints)
        .map_err(|e| close_err("close", id, &e))?;
    std::fs::write(&lt.path, updated).with_context(|| format!("writing {}", lt.path.display()))?;

    let refs: Vec<String> = evidence
        .iter()
        .map(|e| format!("{}:{}", e.kind.as_str(), e.reference))
        .collect();
    let stamped = if fingerprints.is_empty() {
        String::new()
    } else {
        format!(" ({} trace fingerprint(s) stamped)", fingerprints.len())
    };
    // Tolerate a closed pipe (`aplan close ... | head`), like every other
    // stdout-writing command: the file is already written, so a `println!` panic
    // here would exit 101 after a successful mutation.
    write_stdout(&format!(
        "closed {id} with evidence: {}{stamped}\n",
        refs.join(", ")
    ))?;
    Ok(EXIT_OK)
}

/// Content fingerprints for a task's *anchored*, resolvable `traces`, for
/// `APLAN018`. Only sections that read cleanly are stamped; anything else is left
/// unverifiable rather than guessed at.
fn trace_fingerprints(
    task: &aplan_core::Task,
    traces_root: Option<&std::path::Path>,
) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let Some(root) = traces_root else {
        return out;
    };
    for trace in &task.traces {
        if let Some((rel, anchor)) = trace.split_once('#')
            && let Ok(text) = std::fs::read_to_string(root.join(rel))
            && let Some(fp) = aplan_core::anchor_fingerprint(&text, anchor)
        {
            out.insert(trace.clone(), fp);
        }
    }
    out
}

/// `aplan init`, scaffold a starter `.aplan.yml`. Refusing to overwrite is the
/// point: it never clobbers a policy a repo has already committed to.
fn cmd_init(dir: &std::path::Path) -> Result<u8> {
    let path = aplan_config::init(dir)?;
    write_stdout(&format!(
        "wrote {}\nedit it to set your policy; every setting is optional and documented inline.\n",
        path.display()
    ))?;
    Ok(EXIT_OK)
}

/// `aplan facts`, print the machine-readable facts (the source for `facts.json`).
fn cmd_facts() -> Result<u8> {
    write_stdout(&format!("{}\n", facts()))?;
    Ok(EXIT_OK)
}

/// The project's facts as pretty-printed JSON: the enumerable registries a doc or
/// an integration would otherwise hand-copy and get wrong (the "six invariants
/// over a table of ten" incident, HLD Section 16). Every value is *derived* from
/// the code, the rules from [`aplan_core::Rule::ALL`], the formats from
/// [`aplan_output::Format::ALL`], the commands introspected from this crate's clap
/// tree, so the committed `facts.json` (gated by `tests/facts.rs`) can never drift
/// from what the tool actually does.
pub fn facts() -> String {
    use serde_json::json;

    let rules: Vec<_> = aplan_core::Rule::ALL
        .iter()
        .map(|r| {
            json!({
                "code": r.code(),
                "severity": r.severity().to_string(),
                "title": r.title(),
            })
        })
        .collect();
    let formats: Vec<_> = Format::ALL.iter().map(|f| f.as_str()).collect();
    let evidence_kinds: Vec<_> = aplan_core::EvidenceKind::ALL
        .iter()
        .map(|k| k.as_str())
        .collect();
    // Subcommands, introspected from clap so the list cannot drift from the CLI.
    let commands: Vec<_> = Cli::command()
        .get_subcommands()
        .map(|c| {
            json!({
                "name": c.get_name(),
                "about": c.get_about().map(ToString::to_string),
            })
        })
        .collect();

    let value = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "schema_version": "v1",
        "counts": {
            "rules": rules.len(),
            "formats": formats.len(),
            "commands": commands.len(),
            "evidence_kinds": evidence_kinds.len(),
        },
        "exit_codes": {
            "clean": EXIT_OK,
            "error": EXIT_VIOLATIONS,
            "usage": EXIT_USAGE,
            "internal": EXIT_INTERNAL,
        },
        "rules": rules,
        "formats": formats,
        "commands": commands,
        "evidence_kinds": evidence_kinds,
    });
    serde_json::to_string_pretty(&value).expect("facts are serialisable")
}

/// `aplan summary`, roll the plan up by phase (tasks, effort, ready count).
fn cmd_summary(root: &std::path::Path, format: &str) -> Result<u8> {
    let format: Format = format.parse().map_err(anyhow::Error::msg)?;
    // `summary` renders only a table or a JSON document; reject the diagnostic
    // formats (`agent`, `github`) rather than silently collapsing them to JSON,
    // which is what the help promises.
    if !matches!(format, Format::Human | Format::Json) {
        anyhow::bail!("`aplan summary` supports only `--format human` or `--format json`");
    }
    let plan = load_checked(root)?;
    // Like `ready`/`why`/`graph`, a summary of a plan that did not load would be a
    // partial answer that looks whole.
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    let summary = graph.summary();
    render_tolerant(|| render_summary(&summary, format))?;
    Ok(EXIT_OK)
}

/// `aplan critical-path`, the longest `effort_days`-weighted chain of dependent
/// tasks. Advisory: it never fails the build, a plan is not wrong for being long.
fn cmd_critical_path(root: &std::path::Path, format: &str) -> Result<u8> {
    let format: Format = format.parse().map_err(anyhow::Error::msg)?;
    if !matches!(format, Format::Human | Format::Json) {
        anyhow::bail!("`aplan critical-path` supports only `--format human` or `--format json`");
    }
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    let Some(path) = graph.critical_path() else {
        // A cycle has no longest path; `check` reports it as APLAN004. This is a
        // problem with the plan (exit 2), not with aplan.
        anyhow::bail!(
            "cannot compute the critical path: the plan has a dependency cycle (run `aplan check`)"
        );
    };
    render_tolerant(|| render_critical_path(&path, format))?;
    Ok(EXIT_OK)
}

/// `aplan roadmap`, render the plan grouped by cut (ADR-0009). Human and json go
/// through the shared `Format`; `markdown`, the committed companion artifact, is
/// roadmap-specific and so is parsed here rather than added to `Format` (which
/// would make `check --format markdown` a thing and pull in the coverage audit).
/// Hermetic and advisory: it renders a clean plan's claimed status, never git.
fn cmd_roadmap(root: &std::path::Path, format: &str) -> Result<u8> {
    // Validate the format before touching the plan, so a bad format is a usage
    // error (exit 2) and is not masked by a plan error, matching `summary` and
    // `critical-path`. `markdown` is roadmap-specific (not in the shared `Format`).
    enum Out {
        Markdown,
        Shared(Format),
    }
    let out = if format == "markdown" {
        Out::Markdown
    } else {
        let f: Format = format.parse().map_err(anyhow::Error::msg)?;
        if !matches!(f, Format::Human | Format::Json) {
            anyhow::bail!(
                "`aplan roadmap` supports only `--format human`, `--format json`, or `--format markdown`"
            );
        }
        Out::Shared(f)
    };

    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let roadmap = Graph::new(tasks.iter().copied()).roadmap();
    let text = match out {
        Out::Markdown => render_roadmap_markdown(&roadmap),
        Out::Shared(f) => render_roadmap(&roadmap, f),
    };
    render_tolerant(|| {
        use std::io::Write as _;
        std::io::stdout().write_all(text.as_bytes())
    })?;
    Ok(EXIT_OK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aplan_config::Config;

    /// The exit-code contract (ADR-0007): an ordinary error is a usage/config
    /// problem (2); only an error tagged `Bug` is internal (3).
    #[test]
    fn errors_map_to_usage_unless_tagged_a_bug() {
        let usage = anyhow::anyhow!("unknown task `X`");
        assert_eq!(exit_code_for(&usage), EXIT_USAGE);

        let internal = anyhow::anyhow!("close produced invalid frontmatter").context(Bug);
        assert_eq!(exit_code_for(&internal), EXIT_INTERNAL);

        // The tag survives further context wrapping (the real close path adds a
        // "cannot close" message on top).
        let wrapped = anyhow::anyhow!("root cause")
            .context(Bug)
            .context("cannot close `A`");
        assert_eq!(exit_code_for(&wrapped), EXIT_INTERNAL);
    }

    #[test]
    fn root_resolves_flag_then_config_then_default() {
        // No config `root`: the built-in default (`aplan`), namespaced and visible.
        assert_eq!(root_from_config(None, None), PathBuf::from("aplan"));
        // A relative config `root` is resolved against the config directory, so the
        // verdict does not depend on the invocation directory.
        assert_eq!(
            root_from_config(Some("plan".into()), Some(std::path::Path::new("/repo"))),
            PathBuf::from("/repo/plan")
        );
        // An absolute config `root` is used as-is.
        assert_eq!(
            root_from_config(
                Some("/elsewhere/tasks".into()),
                Some(std::path::Path::new("/repo"))
            ),
            PathBuf::from("/elsewhere/tasks")
        );
        // The `--root` flag wins over everything and never consults the config.
        assert_eq!(
            resolve_root(Some(PathBuf::from("cli-root"))).unwrap(),
            PathBuf::from("cli-root")
        );
    }

    #[test]
    fn defaults_apply_with_no_config_and_no_flags() {
        let o = resolve_check_options(&Config::default(), None, false, None, false);
        assert!(o.require_effort, "effort required by default");
        assert_eq!(
            o.traces_root,
            Some(PathBuf::from(".")),
            "traces on, rooted at cwd"
        );
    }

    #[test]
    fn the_file_sets_options_when_no_flag_is_given() {
        let c = Config {
            require_effort: Some(false),
            traces_root: Some("docs".into()),
            ..Default::default()
        };
        let o = resolve_check_options(&c, None, false, None, false);
        assert!(!o.require_effort);
        assert_eq!(o.traces_root, Some(PathBuf::from("docs")));
    }

    #[test]
    fn a_relative_file_traces_root_resolves_against_the_config_dir() {
        // The verdict must not depend on the invocation directory: `traces_root:
        // docs` in `.aplan.yml` at /repo means /repo/docs, wherever `check` runs.
        let c = Config {
            traces_root: Some("docs".into()),
            ..Default::default()
        };
        let o = resolve_check_options(&c, Some(std::path::Path::new("/repo")), false, None, false);
        assert_eq!(o.traces_root, Some(PathBuf::from("/repo/docs")));
        // An absolute config value is left as-is.
        let abs = Config {
            traces_root: Some("/elsewhere".into()),
            ..Default::default()
        };
        let o = resolve_check_options(
            &abs,
            Some(std::path::Path::new("/repo")),
            false,
            None,
            false,
        );
        assert_eq!(o.traces_root, Some(PathBuf::from("/elsewhere")));
    }

    #[test]
    fn a_flag_overrides_the_file() {
        // The file asks to require effort and root traces at `docs`; the flags
        // say otherwise, and the flags win.
        let c = Config {
            require_effort: Some(true),
            traces_root: Some("docs".into()),
            ..Default::default()
        };
        let o = resolve_check_options(&c, None, true, Some(PathBuf::from("src")), false);
        assert!(!o.require_effort, "--no-require-effort overrides the file");
        assert_eq!(
            o.traces_root,
            Some(PathBuf::from("src")),
            "--traces-root overrides the file"
        );
    }

    #[test]
    fn traces_can_be_disabled_from_either_flag_or_file() {
        // From the flag.
        assert_eq!(
            resolve_check_options(&Config::default(), None, false, None, true).traces_root,
            None
        );
        // From the file.
        let c = Config {
            no_traces: Some(true),
            ..Default::default()
        };
        assert_eq!(
            resolve_check_options(&c, None, false, None, false).traces_root,
            None
        );
    }
}
