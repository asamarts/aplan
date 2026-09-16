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

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use aplan_core::{Graph, Options, load_plan, validate};
use aplan_output::{
    Format, Surface, render, render_critical_path, render_partial, render_proposed, render_roadmap,
    render_roadmap_markdown, render_summary, render_tasks, render_verified,
    render_verified_partial,
};
use clap::{CommandFactory, Parser, Subcommand};

/// `eprintln!`, on the stream that knows the colour choice (`A1-286`).
///
/// Used exactly like `eprintln!`, with two deliberate differences. It writes
/// through `anstream`, so a refusal is subject to `--color` the way a finding
/// already was -- before this, `aplan show NOPE` was plain text under every
/// setting, because `eprintln!` is not a stream anything can strip or style. And
/// it drops a write error rather than panicking, which `eprintln!` does not.
/// That is not only the broken-pipe case: an error writing to stderr has nowhere
/// to be reported, since reporting it would use the stream that just failed, and
/// the exit code still carries the failure. `write_stdout` surfaces its errors
/// because stdout's reader is not the one being told about them.
macro_rules! errln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(::anstream::stderr().lock(), $($arg)*);
    }};
}

/// [`errln!`] without the newline, for a fragment that carries its own.
macro_rules! err {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = write!(::anstream::stderr().lock(), $($arg)*);
    }};
}

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

/// How long a live task may sit before `aplan stale` names it, absent any
/// configuration. Three weeks is git's own eviction window for an inactive topic
/// (FIELD-RESEARCH 7.3), rounded to a month so a fortnight's holiday does not
/// fill the report.
const DEFAULT_STALE_DAYS: u32 = 30;

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

    /// When to emit ANSI colour.
    ///
    /// `auto`, the default, follows the stream: a terminal gets colour and a pipe
    /// gets plain bytes, and `NO_COLOR`, `CLICOLOR_FORCE` and `CLICOLOR=0` are
    /// honoured. `always` and `never` are for the two cases detection cannot
    /// serve: colour into a pager or a screen capture, and none at a terminal.
    ///
    /// The machine formats carry no colour at any setting.
    #[arg(
        long,
        global = true,
        value_name = "WHEN",
        default_value = "auto",
        value_parser = clap::builder::PossibleValuesParser::new(aplan_output::color::ColorChoice::NAMES),
    )]
    color: String,

    // `A1-155`, pre-public adoption evidence. The pilot stays in this comment
    // (`A1-044`).
    /// Read exactly this configuration file, and discover none.
    ///
    /// `.aplan.yml` names the programs a `test` or `url` citation runs, and it is
    /// normally discovered by walking up from the plan root or the repository --
    /// which is right for your own repository and wrong when you are reviewing
    /// somebody else's checkout with a trusted binary. Naming a file here means
    /// nothing in the tree under review is consulted, and a file that is missing
    /// is an error rather than a quiet return to discovery.
    ///
    /// Relative values inside it (`root`, `traces_root`, a runner's `dir`)
    /// resolve against *its* directory, so a trusted policy can live outside the
    /// tree it judges.
    #[arg(long, global = true, value_name = "FILE", conflicts_with = "no_config")]
    config: Option<PathBuf>,

    /// Read no configuration at all: defaults only, and no configured programs.
    ///
    /// Distinct from finding none. `verify` can then resolve `commit` and `file`
    /// evidence and nothing that would run a command, which is the fail-closed
    /// shape for reviewing an untrusted tree.
    #[arg(long, global = true)]
    no_config: bool,

    #[command(subcommand)]
    command: Command,
}

/// **Changing what an existing command *does* earns a `Behaviour`** (`A1-142`).
///
/// Adding a subcommand or a flag is already visible: `aplan facts` lists the
/// command, and a build without it fails loudly. Changing what one of these
/// *answers* is not. `--require ready` passes on the build that went silent at a
/// cut boundary and on the build that names it; `--require add` passes on the
/// build that writes a plan `check` would reject and on the build that refuses
/// it. Both shipped unnamed, and `A1-142`'s audit found them by asking this
/// question here, which is why the reminder is here.
#[derive(Subcommand, Debug)]
enum Command {
    /// Validate the plan. Fails on any error. Reads `.aplan.yml` if present;
    /// flags below override it.
    Check(CheckArgs),
    /// Resolve evidence against the git repository (L1/L2, and L3 with
    /// `--execute`). Unlike `check`, this opens the repo: a `commit` must exist
    /// and touch the task's `paths`, a `file` must exist at the cited rev, a
    /// `test` must be one the configured runner lists.
    Verify(VerifyArgs),
    /// List tasks whose blockers are all `done`, best first.
    Ready(ReadyArgs),
    /// Parked tasks whose `revisit_at` has arrived: a hand-off to a scheduler.
    ///
    /// Reaching the instant means *look at this again*, never that the wait is
    /// over. The condition a task is parked on lives in its body, and no clock
    /// can check it. `check`, `ready` and `verify` never read a clock; this is
    /// the one command that does, and `--as-of` makes it reproducible.
    Due(DueArgs),
    /// Explain what is blocking a task: its own blockers, and any unfinished cut.
    Why {
        /// Task id.
        id: String,
        /// Walk through cuts into their membership as well.
        ///
        /// Off by default. A sequenced plan puts every earlier cut behind the
        /// one in front of it, so the complete closure of a late task is mostly
        /// the roadmap: a pilot asked why one task was blocked and got 86 lines,
        /// three of which could be acted on. A cut is reported as itself, with
        /// a count of what is behind it, and this asks for that list. An
        /// answer that folded anything says so; one that folded nothing is the
        /// complete answer already.
        #[arg(long)]
        all: bool,
        /// Output format: human (a list) or json.
        #[arg(long, default_value = "human")]
        format: String,
    },
    /// Print the dependency graph as Mermaid, whole or scoped.
    Graph(GraphArgs),
    /// Show one task whole: its fields, what it waits on and releases, and the
    /// findings currently attached to it.
    Show(ShowArgs),
    // The announced default change is `A1-235`, and `A1-236` is the removal half
    // of the runway. The number stays here rather than in the help: help is read
    // beside the consumer's own `docs/adr/`, where this project's ids resolve to
    // nothing (`A1-044`).
    /// Show one task's neighbourhood: what must precede it, and what it releases.
    ///
    /// **Announced, not yet done:** in a later cut this will show only
    /// the ancestry that can still block, which is what `--live` does today, and
    /// `--all` will restore the full view. Nothing changes yet. A `done` task
    /// cannot block anything, so on a plan with any history today's default
    /// spends most of its lines on constraints already met. What is withheld is
    /// counted either way, so the answer never goes quiet about what it left
    /// out.
    Tree {
        /// The task at the centre.
        id: String,
        /// How many levels to walk in each direction. Unlimited by default; a
        /// subtree cut short says so rather than ending silently.
        #[arg(long)]
        depth: Option<usize>,
        /// Only the part that can still block: hide satisfied ancestry, which
        /// is most of it on a plan with history. **This becomes the default in a
        /// later cut**, with `--all` for the view you get now.
        /// A task that is `done` cannot
        /// block anything, so it is a constraint already met, printed to
        /// somebody asking what is in their way. What was hidden is counted.
        #[arg(long)]
        live: bool,
    },
    // `A1-148`, pre-public adoption evidence item 3. The task ids and the pilot stay
    // in this comment: help is read beside the consumer's own plan, where they
    // do not resolve (`A1-044`).
    // `A1-038`, pre-public adoption evidence section 5. The pilot ids stay in this
    // comment: help is read beside the consumer's own plan (`A1-044`).
    // `A1-151`, pre-public adoption evidence item 11. Ids stay here (`A1-044`).
    /// Measure how much of recent history each task's `paths` would match.
    ///
    /// A `commit` citation is checked by asking whether that commit touched the
    /// task's declared `paths`. Where the paths cover most of what the project
    /// does, almost any commit satisfies them and the check proves little.
    ///
    /// This measures and judges nothing: read a row against the median this
    /// prints beside it, because the same share means "almost anything would
    /// pass" in a plan whose median is low and nothing at all in a plan where
    /// one component really is the work. Exits `0` whatever it finds.
    Paths {
        /// How many recent commits to sample. Fewer than about 50 lets one quiet
        /// week decide the answer.
        #[arg(long, default_value_t = DEFAULT_PATH_SAMPLE, value_name = "N")]
        sample: usize,
        /// Where the git repository is (default: the current directory).
        #[arg(long, default_value = ".")]
        repo: std::path::PathBuf,
        /// Output format: human (a table) or json.
        #[arg(long, default_value = "human")]
        format: String,
        /// Narrow to a subset of the plan. Selection changes what is shown,
        /// never what is computed: a task whose blocker is outside the subset
        /// is still blocked.
        #[command(flatten)]
        select: SelectArgs,
    },
    // `A1-114`, pre-public adoption evidence item 2.4. The task id stays in this
    // comment (`A1-044`).
    /// Report what each rule could actually have failed on.
    ///
    /// A rule that evaluates to "pass" and a rule that had nothing to evaluate
    /// produce the same output, and the second is the more common state. This
    /// counts, per rule, the tasks it had a subject in and the tasks whose
    /// predicate had nothing to compare against -- the ones it ran on and could
    /// not have failed, however wrong they were.
    ///
    /// Derived from the rules themselves: each declares what a subject must
    /// carry and what its predicate binds against, on the row that already
    /// generates its code and its catalogue entry. Advisory, and never a gate: a
    /// repository mid-adoption is blind nearly everywhere, and what it needs is
    /// to watch the number come down.
    Vacuity {
        /// Output format: human (a table) or json.
        #[arg(long, default_value = "human")]
        format: String,
        /// Narrow to a subset of the plan. Selection changes what is shown,
        /// never what is computed: a task whose blocker is outside the subset
        /// is still blocked.
        #[command(flatten)]
        select: SelectArgs,
    },
    // `A1-020`, FIELD-RESEARCH 7.3. Ids stay in the comment (`A1-044`).
    /// Report live tasks that have sat unchanged, and which of them could start.
    ///
    /// "How long has this sat" is the highest-signal drift metric there is, and
    /// git already knows it: this walks the history once and reports live tasks
    /// whose file has not changed inside the window.
    ///
    /// **It never fails a build.** A task file's date is a proxy for the work's
    /// state -- a file can sit while the work moves, or move while the work sits
    /// -- so this reports and exits `0`. What it is good for is the case it
    /// leads with: a task whose blockers are all done, which could be started
    /// now and has not been.
    Stale {
        /// Report live tasks unchanged for longer than this many days.
        /// Overrides `stale_after_days`; the default is 30.
        #[arg(long, value_name = "DAYS")]
        older_than: Option<u32>,
        /// Measure ages from this date (`YYYY-MM-DD`) instead of from `HEAD`.
        ///
        /// Ages are measured from `HEAD`'s own commit time by default, so two
        /// runs at one commit agree. That is the right answer for a gate and the
        /// wrong one for a repository that has stopped moving: every age freezes
        /// with the last commit, and a plan abandoned six months ago reports
        /// nothing as idle.
        ///
        /// Naming a date is the explicit, still-reproducible way to ask "how
        /// does this look *now*": the same date at the same commit gives the
        /// same report, which a wall clock inside the tool could not promise.
        ///
        /// The whole day counts, so today works on a day you have committed.
        #[arg(long, value_name = "DATE")]
        as_of: Option<String>,
        /// Where the git repository is (default: the current directory).
        #[arg(long, default_value = ".")]
        repo: std::path::PathBuf,
        /// Output format: human (a table) or json.
        #[arg(long, default_value = "human")]
        format: String,
        /// Narrow to a subset of the plan. Selection changes what is shown,
        /// never what is computed: a task whose blocker is outside the subset
        /// is still blocked.
        #[command(flatten)]
        select: SelectArgs,
    },
    // `A1-021`, FIELD-RESEARCH section 6 P2. The task id stays in this comment
    // (`A1-044`).
    /// Report the deprecation runways this plan declares.
    ///
    /// "Announce the removal in one release, do it in a later one" is the
    /// cleanest checkable planning promise there is, and it needs no schema
    /// here: a runway is two ordinary tasks, one tagged `deprecation` that
    /// announces and one tagged `removal` that does it, the second listing the
    /// first in its `blocked_by` and living in the cut it lands in.
    ///
    /// This reads that back -- what was announced, where, what removes it and
    /// when -- and names each way the pair can be broken, including a removal
    /// nothing announced. Hermetic and advisory: it never opens git and exits
    /// `0` whatever it finds.
    Deprecations {
        /// Output format: human, json, or markdown (a table to publish).
        #[arg(long, default_value = "human")]
        format: String,
    },
    // `A1-040`, RESEARCH section 15. The task id stays in this comment (`A1-044`).
    /// Record what this plan proved, bound to the commit it proved it at.
    ///
    /// The end of the evidence ladder: `verify` resolves every citation against
    /// git, and this writes that verdict down in a form somebody outside the
    /// repository can check without rerunning it. Each `commit` citation is
    /// recorded as the full sha it resolved to and each `file` citation as its
    /// blob sha, so a reader re-derives both rather than trusting the document.
    ///
    /// It refuses a plan that does not verify, and a work tree with uncommitted
    /// changes: both would name a state nobody else can reproduce. There is no
    /// timestamp in the output, which is what makes two runs at one commit agree
    /// byte for byte.
    Attest {
        /// Where the git repository is (default: the current directory).
        #[arg(long, default_value = ".")]
        repo: std::path::PathBuf,
        /// Also attach the document to `HEAD` as a git note under this ref.
        ///
        /// Attachment, not signature: a note is tamper-evident because git
        /// hashes it, and a cryptographic signer is a second implementation of
        /// the same seam.
        #[arg(long, value_name = "REF")]
        attach_note: Option<String>,
        /// Run configured test commands while verifying, as `verify --execute`.
        #[arg(long)]
        execute: bool,
    },
    /// Write the agent contract this repository's policy implies.
    ///
    /// The first pilot enforced its plan in CI while nothing told an agent the
    /// rule existed, so an agent learned it by failing a check after doing the
    /// work the old way. The file it then hand-wrote is a second, ungated copy
    /// of the policy; this generates that half from `.aplan.yml` and the plan,
    /// fenced so the repository keeps its own prose around it.
    ///
    /// With no flags it prints the block. `--write` updates a file in place
    /// between the markers, and `--check` fails when the file disagrees, which
    /// is the gate shape `roadmap --format markdown` already has.
    ///
    /// To put the block somewhere other than the end of a file, write the two
    /// markers where you want it. The opening one is recognised by its start:
    /// a line opening `<!-- aplan:begin`, then a space or a dash, and closing
    /// `-->`. So `<!-- aplan:begin -->` will do, `--write` replaces it with the
    /// full marker that says what the block is, and a longer word such as
    /// `<!-- aplan:beginning ... -->` is your comment rather than the marker.
    ExportAgentsMd {
        /// Update this file in place, between the markers, appending the block
        /// if it has none.
        #[arg(long, value_name = "FILE")]
        write: Option<std::path::PathBuf>,
        /// Compare this file against what would be generated and exit 1 if they
        /// differ. Never writes.
        #[arg(long, value_name = "FILE", conflicts_with = "write")]
        check: Option<std::path::PathBuf>,
    },
    // `A1-033`, ADOPTION-ANALYSIS section 4. The task id stays in this comment
    // (`A1-044`).
    /// Write the plan's statuses back into a `roadmap.json` contract.
    ///
    /// The first pilot's repository cannot simply drop its `roadmap.json`: a web
    /// build and a spec linter read it. The honest end state is not deleting the
    /// file but generating the half of it that drifts, so its statuses stop
    /// being hand-maintained.
    ///
    /// A merge, not a render. That document carries fields this model does not
    /// hold, and a file generated from the plan alone would validate here and
    /// break there. So the file is its own template: the plan supplies status
    /// and `depends_on`, and every other byte of meaning is carried through. A
    /// milestone the plan has and the file does not is reported rather than
    /// invented, because where it belongs in a foreign schema is not something
    /// this can know.
    ExportRoadmapJson {
        /// The `roadmap.json` to regenerate, which is also the template.
        file: std::path::PathBuf,
        /// Compare the file against what would be generated and exit 1 if they
        /// differ. Never writes.
        #[arg(long)]
        check: bool,
    },
    // `A1-189`. The task id stays in this comment (`A1-044`).
    /// Write the plan out in somebody else's shape.
    ///
    /// The other half of `import`. A repository that adopted this tool from a
    /// markdown checklist still has colleagues reading that checklist, and the
    /// day the plan becomes the source of truth the document starts going
    /// stale. This regenerates it, so a hand-maintained file that disagrees
    /// with the plan gets the answer everything else here gets: generate the
    /// half that drifts.
    ///
    /// Lossy, and it says so. What the target shape cannot carry is reported on
    /// standard error, the way an import reports what the task model cannot
    /// hold, and a shape that cannot be written honestly is refused by name
    /// with the field it could not carry rather than approximated.
    Export(ExportArgs),
    /// Check that a document names tasks that exist.
    ///
    /// An item reading "tracked as `T-099`" when no such task exists reads as
    /// *handled*, which is worse than an item with no disposition at all. Every
    /// project with a plan document ends up writing this check; here it is.
    ///
    /// Ids come from the plan, never from a pattern: the prefixes its own ids
    /// use are what a word has to carry to be read as a reference. Exits 1 when
    /// a document names a task the plan does not have.
    ///
    /// A second check, off unless `docs.membership` in `.aplan.yml` turns it on:
    /// where a document lists a cut's tasks beside a `M-<cut>` milestone, those
    /// are two copies of one fact and this reads both. The config says where the
    /// document declares membership -- a marked line, or a task table's first
    /// cell -- because a document's shape means whatever its author intended.
    /// Fenced code is never a declaration, so a document may show its own format.
    /// A milestone the document has no section for is not a finding, and neither
    /// is a release: a release holds cuts rather than work, so a cut's task list
    /// is not where its membership lives.
    CheckDocs {
        /// The documents to read. Directories are not walked: name the files, so
        /// what is gated is a decision rather than whatever happened to be on
        /// disk.
        #[arg(required = true)]
        paths: Vec<std::path::PathBuf>,
        /// Output format: human (one line per finding) or json.
        #[arg(long, default_value = "human")]
        format: String,
    },
    /// Roll the plan up by phase: tasks, effort, and how many are ready.
    ///
    /// A plan with an unreadable task file is refused rather than rolled up: this
    /// document is a rollup and not a report, so carrying diagnostics would make
    /// its shape conditional on the plan being broken, which is worse for a
    /// consumer than an empty stream. The reason goes to stderr and the exit code
    /// is 1; `aplan check --format json` is the command that answers *why*.
    Summary {
        /// Output format: human (a table) or json.
        #[arg(long, default_value = "human")]
        format: String,
        /// Narrow to a subset of the plan. Selection changes what is shown,
        /// never what is computed: a task whose blocker is outside the subset
        /// is still blocked.
        #[command(flatten)]
        select: SelectArgs,
    },
    /// Drop a task, or a whole cut, recording the decision rather than deleting
    /// the record of it. Refuses to revive anything already terminal.
    Drop {
        /// Task ids to drop. With `--cut`, the cuts whose members go too, named
        /// by milestone id or by the label each reports itself under.
        #[arg(required = true)]
        ids: Vec<String>,
        /// Treat each id as a milestone: drop its members, then the milestone.
        #[arg(long)]
        cut: bool,
        /// Whether a warning blocks this write, as `check` and `verify` take it.
        #[command(flatten)]
        verdict: VerdictFlags,
    },
    /// Move tasks between cuts, editing both milestones or neither.
    Mv {
        /// The tasks to move. Several are one move: every refusal runs before
        /// any file is written, so a member that cannot move refuses the whole
        /// set rather than leaving it half applied.
        #[arg(required = true, num_args = 1..)]
        ids: Vec<String>,
        /// The cut it moves into, by its milestone id or by the label the cut
        /// reports itself under (`M-s1` or `s1`).
        #[arg(long, required_unless_present = "phase")]
        cut: Option<String>,
        /// The phase it moves into. Optional, and combinable with `--cut`: a
        /// task that changes cut usually changes phase with it, and the two
        /// together are one post-state judged once rather than two edits to
        /// reconcile.
        ///
        /// `phase` groups a plan for `summary`'s rollup and nothing gates on it,
        /// so this is the one move that cannot be refused for what it produces
        /// -- only for a file it cannot write.
        #[arg(long)]
        phase: Option<u32>,
        /// Whether a warning blocks this write, as `check` and `verify` take it.
        #[command(flatten)]
        verdict: VerdictFlags,
    },
    /// Emit the plan's tasks as data, so a rollup this tool does not have is a
    /// `jq` line rather than a feature request. Carries what a rollup needs
    /// resolved: the cut a task belongs to, and whether it is ready.
    ///
    /// A plan with an unreadable task file is refused rather than rolled up: this
    /// document is a rollup and not a report, so carrying diagnostics would make
    /// its shape conditional on the plan being broken, which is worse for a
    /// consumer than an empty stream. The reason goes to stderr and the exit code
    /// is 1; `aplan check --format json` is the command that answers *why*.
    Tasks {
        /// Output format: human (a table) or json.
        #[arg(long, default_value = "human")]
        format: String,
        /// Narrow to a subset of the plan. Selection changes what is shown,
        /// never what is computed: a task whose blocker is outside the subset
        /// is still blocked.
        #[command(flatten)]
        select: SelectArgs,
    },
    /// Show the critical path: the longest `effort_days`-weighted chain of
    /// dependent tasks. Advisory, never a gate.
    CriticalPath {
        /// Output format: human (a table) or json.
        #[arg(long, default_value = "human")]
        format: String,
    },
    // The doc comment below is `--help` text, so it carries no ADR number
    // (`A1-044`): a consumer reads it beside their own `docs/adr/`. The decision
    // it implements is this project's ADR-0009, recorded here where only a
    // contributor reading the source will meet it.
    /// Render the plan grouped by cut (the milestones): per-cut rollups, the
    /// critical path, and cross-cutting tasks. Advisory: it reports, and never
    /// fails a build.
    Roadmap(RoadmapArgs),
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
        /// Record the evidence without resolving it against the repository.
        ///
        /// `close` otherwise refuses a citation `verify` would reject. Use when
        /// offline or mid-repair; it reports every check it skipped, because a
        /// closure that skipped its checks must not read like one that passed.
        #[arg(long)]
        no_resolve: bool,
        /// Whether a warning blocks this write, as `check` and `verify` take it.
        #[command(flatten)]
        verdict: VerdictFlags,
    },
    // As above: `--help` is consumer-facing, so the number stays here rather than
    // in the text (`A1-044`). The decision is this project's ADR-0012.
    /// Repair a closed task's evidence: add, replace or remove a citation, in
    /// place, without reopening it.
    ///
    // The summary names all three because that line is what `aplan --help`
    // shows, and naming only the replace mode hid the other two: two pilots
    // needed `--add-evidence` on the same day, one hand-editing five task files
    // instead and the other finding it "by reading `amend --help` on a guess"
    // (`A1-265`). A `//` comment, not a doc one: this text is read beside a
    // consumer's own repository, where a task id of ours does not resolve
    // (`A1-044`).
    /// For a citation that has rotted rather than one that was wrong: a renamed
    /// test leaves closed tasks pointing at nothing, and the answer used to be
    /// to edit the file by hand. Both citations are named, so the diff shows
    /// what was replaced and by what. Every mode resolves what it records
    /// exactly as `close` resolves it, so a repair cannot record what `verify`
    /// would reject.
    ///
    /// The other repair is `close --restamp`, which re-takes a closed task's
    /// `traces` fingerprints after re-reviewing a section that drifted. This
    /// command does not touch them: repairing a citation is not re-reviewing a
    /// traced section.
    Amend(AmendArgs),
    /// File the successor to a closed task whose traced outcome no longer holds,
    /// and re-stamp the closed one as reviewed.
    ///
    /// The third repair, beside `amend` and `close --restamp`. There is no
    /// reopen, deliberately: the work *was* done, and what changed is the ground
    /// under it, so a plan that reopened would be claiming the work never
    /// happened, and would invalidate the cut it completed.
    // The reasoning is ADR-0020's. The number stays in this comment: help is read
    // beside the consumer's own `docs/adr/`, where ours do not resolve (`A1-044`).
    Supersede(SupersedeArgs),
    /// Add or remove a dependency edge: `dep add A --after B` means A comes after
    /// B, and `dep rm` takes it back.
    ///
    /// `blocked_by` is the only per-task ordering the model has, and it was
    /// written at creation and never again, so re-sequencing work meant editing
    /// frontmatter by hand. Whether the result is legal is decided by the same
    /// post-state check every other mutation runs.
    #[command(subcommand)]
    Dep(DepCommand),
    /// Import proposals from another tool's format into `aplan` tasks. Lossy by
    /// design, and says so: every field the task model cannot hold is reported.
    Import(ImportArgs),
    /// Scaffold a new task file: the next id in this plan's own convention, the
    /// fields `check` needs, and a body to fill in. It fills in nothing it cannot
    /// know, so a new task carries no estimate and no evidence unless you give
    /// them. `--from <file>` adds every task a file names instead, as one change.
    Add(AddArgs),
    /// Run an MCP server over stdio, exposing a deliberately small tool set
    /// (`identity`, `ready`, `why`, `check`, `verify`, `add`, `show`, `dep`, `close`) to an agent. `close` goes through the
    /// same evidence gate as the CLI, in both directions: an agent cannot close
    /// through MCP what it could not close here, and is not refused what this
    /// command records.
    Mcp,
    /// Scaffold a starter `.aplan.yml`, refusing to overwrite an existing one.
    Init {
        /// Directory to write `.aplan.yml` into (default: the current directory).
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Print this tool's facts as JSON: its rules, output formats, commands, and
    /// evidence kinds, generated from the code (the source for `facts.json`).
    Facts {
        /// Assert this build advertises a capability, and exit non-zero if not.
        ///
        /// Anything this build advertises: a config key (`test.covers_by_test`),
        /// a rule code (`APLAN024`), a command (`tasks`), an output format
        /// (`sarif`), an evidence kind (`commit`), a named behaviour
        /// (`traces.table_rows`), which is how a change that widens an existing
        /// rule is asserted, or an MCP tool (`aplan_verify`), so the agent
        /// surface is assertable and not only the CLI. Repeatable. With any
        /// `--require`, the JSON is replaced by a one-line verdict, which is
        /// what a container build wants in its log.
        #[arg(long, value_name = "CAPABILITY")]
        require: Vec<String>,
        // `A1-256`. Three pilots could not tell which build they were running,
        // and the third is the one capability pinning cannot answer: a change
        // that alters a message earns no capability, so `facts` is byte-identical
        // across it and `--require` has nothing to require.
        /// Print which build this is, rather than what it can do.
        ///
        /// A JSON document naming the package version, the commit the build
        /// script saw, the digest of this build's own `facts` document, and the
        /// digest of the running binary. It needs no plan, no repository and no
        /// clean tree, which `attest` does. `binary_sha256` is the field that
        /// distinguishes two builds differing only in wording; it is
        /// self-reported, so it identifies rather than authenticates.
        #[arg(long, conflicts_with = "require")]
        identity: bool,
    },
    /// Print the JSON Schema derived from the Rust types.
    Schema,
}

/// The clap command tree, for tests that need to inspect what the CLI *says*
/// rather than what it does.
///
/// Exists so the consumer-facing-text audit (`A1-044`) can walk every
/// subcommand's help, including ones added after it was written. Reading the
/// rendered help is the only way to catch a leak: the text comes from doc
/// comments, so a source-level grep for a string literal would never see it.
/// Say what the configured commands cost, when it is worth saying (`A1-309`).
///
/// **Only when a runner reached something worth knowing.** A note nobody needs
/// is a note nobody reads, and most runs are small: `cargo test -q -p aplan-core`
/// warm peaks at four processes and 156 MB. The thresholds are the shape of the
/// problem rather than round numbers -- sixteen processes is twice a common core
/// count, and a gibibyte is where a laptop starts to notice.
///
/// **Summed RSS, said out loud.** Adding each process's resident set overcounts
/// what they share: measured, a cargo run reached 30.5 GB summed against 5.1 GB
/// proportional, and a vitest run 33.0 GB against 25.6 GB. The cheap figure is
/// the one taken, so the line names it; an adopter sizing a limit from the wrong
/// one would size it six times too large.
///
/// **Absence is reported, not zeroed.** A platform with no `/proc` measures
/// nothing, and "0 processes, 0 bytes" would be a lie rather than a silence.
/// A declared cap this platform could not apply, said once per reason and not
/// once per runner (`A1-310`, ADR-0033).
///
/// The platform is one fact, so a repository with ten runners hears it once and
/// is told which of them asked for something. Which case it is in decides the
/// advice, so the two are separate lines rather than one: a machine with no
/// cgroup v2 has no remedy to apply, and a container has one its operator does.
///
/// **Not a rule code**, deliberately. Every rule here owes the corpus a fires
/// *and* a silent scenario, and no scenario can produce this one: where cgroups
/// work the condition cannot be created, and a test cannot pretend otherwise
/// because `std::env::set_var` is `unsafe` in this edition and the workspace
/// forbids that. The cost, stated here and in ADR-0033 rather than buried:
/// `deny_warnings` does not turn an unenforced limit into a blocked build.
fn note_what_could_not_be_held(spend: &[(String, aplan_evidence::Cost)]) {
    let named = |why: aplan_evidence::Unenforced| -> String {
        spend
            .iter()
            .filter(|(_, c)| c.unenforced == Some(why))
            .map(|(subject, _)| subject.clone())
            .collect::<Vec<_>>()
            .join(", ")
    };

    let absent = named(aplan_evidence::Unenforced::NoCgroups);
    if !absent.is_empty() {
        errln!(
            "note: this platform has no cgroup v2, so the resource limits declared by {absent} \
             were not applied and those commands ran with nothing holding them. Bound the \
             command itself instead: `test.wrap` takes a prefix such as `nice`, and a harness's \
             own flags reach it through the environment."
        );
    }
    let undelegated = named(aplan_evidence::Unenforced::NotDelegated);
    if !undelegated.is_empty() {
        errln!(
            "note: this process may not create a cgroup of its own, which is what a container \
             without a delegated subtree looks like, so the resource limits declared by \
             {undelegated} were not applied and those commands ran with nothing holding them. \
             Give the container a delegated cgroup subtree, or use `test.wrap`."
        );
    }
}

fn note_what_the_run_cost(spend: &[(String, aplan_evidence::Cost)], executed: bool) {
    const BUSY_PROCESSES: usize = 16;
    const BUSY_RSS_KB: u64 = 1024 * 1024;

    note_what_could_not_be_held(spend);

    for (subject, cost) in spend {
        if !cost.measured {
            if executed {
                errln!(
                    "note: what {subject} cost could not be measured on this platform, so \
                     nothing is reported rather than a zero."
                );
            }
            continue;
        }
        if cost.peak_processes < BUSY_PROCESSES && cost.peak_rss_kb < BUSY_RSS_KB {
            continue;
        }
        // Integer arithmetic, because the workspace denies a lossy cast and this
        // is a number for a person to read rather than compute with. Megabytes
        // below a gibibyte: a run that trips the process threshold while using
        // little memory would otherwise report "0.0 GB", which reads as a
        // measurement failure rather than as a small number.
        let megabytes = cost.peak_rss_kb / 1024;
        let size = if megabytes >= 1024 {
            let tenths = cost.peak_rss_kb / 104_858;
            format!("{}.{} GB", tenths / 10, tenths % 10)
        } else {
            format!("{megabytes} MB")
        };
        errln!(
            "note: {subject} peaked at {} process(es) and {size} (summed RSS, which overcounts \
             memory they share) over {}s. `aplan` itself spawns one command at a time; the \
             rest is the harness's.",
            cost.peak_processes,
            cost.elapsed.as_secs(),
        );
    }
}

#[must_use]
pub fn cli_command() -> clap::Command {
    // `MAX_WIDTH` rather than the terminal-aware width the renderers use.
    // `A1-205` decided piped help is byte-stable at the cap, because `--help |
    // less` is still a person reading; a finding piped into a CI log is not, and
    // folding that would only break `grep` (`A1-206`).
    let width = aplan_output::MAX_WIDTH;
    if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        // A person is reading it: use their terminal, capped so a very wide one
        // does not produce lines nobody can scan.
        Cli::command().max_term_width(width)
    } else {
        // A pipe, a file, or a test corpus. `max_term_width` still honours
        // `COLUMNS`, so help piped into a golden file would depend on the
        // environment that ran it and the corpus would pass on one machine and
        // fail on another. `term_width` ignores `COLUMNS` outright, which is
        // what makes a redirected run byte-stable (`A1-205`).
        Cli::command().term_width(width)
    }
}

/// The CLI entry point. `src/main.rs` is a thin shim over this.
pub fn run_cli() -> ExitCode {
    // Before `clap` sees the arguments, and that ordering is the point. `clap`
    // renders its own usage errors and `--help` through an `anstream` stream
    // built with `Auto`, which resolves against the same process-global this
    // writes -- so setting it first is what makes `--color=never` silence the
    // red `error:` on a usage mistake, and what makes `--color=always` colour it
    // into a pipe. Setting it after parsing would have covered every renderer
    // and left the one output a reader meets *before* any command runs.
    color_from_argv(std::env::args_os().skip(1)).apply();
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            // Every error the tool reports leaves through here, so folding once
            // at the exit covers each `bail!` without one of them being missed
            // (`A1-206`). Seven, so a folded reason continues under its own text
            // rather than under `aplan:`. `fold_block`, not `fold`: a refusal that
            // lists findings under a headline carries its own line breaks, and
            // folding the whole message at once ran them together.
            // The one style this task adds, and it is here because a criterion
            // needs a witness: "the CLI's own writes are on the stream" is not
            // observable while nothing on that stream is ever styled. `A1-287`
            // owns the palette; this is the single role it cannot wait for,
            // since it is also the asymmetry this task's own argument names --
            // `clap` prints a red `error:` for a mistyped flag while the tool's
            // own refusal, the more serious of the two, was plain.
            //
            // Only the prefix is styled, which is what keeps it safe to fold: a
            // seven-column run cannot be split by any width, so the style opens
            // and closes on the first line whatever `fold_block` does with the
            // rest. A styled run long enough to fold is a defect, and it is
            // `A1-287`'s to fix.
            let red = aplan_output::style::ERROR;
            errln!(
                "{}",
                aplan_output::fold::fold_block(
                    &format!("{red}aplan:{red:#} {e:#}"),
                    aplan_output::fold::err_width(),
                    7
                )
            );
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
    // The remedy, where there is one, from the one place both surfaces read it
    // (`A1-265`). The core error states the fact; naming the command that does
    // the thing is the CLI's half, and it can fill in the id.
    let err = match aplan_core::close::advice(e, id) {
        Some(advice) => anyhow::anyhow!("cannot {action} `{id}`: {e}. {advice}"),
        None => anyhow::anyhow!("cannot {action} `{id}`: {e}"),
    };
    if matches!(e, aplan_core::CloseError::Produced(_)) {
        err.context(Bug)
    } else {
        err
    }
}

/// The `--color` value, read from the raw arguments before `clap` parses them.
///
/// A second reader of one flag, which is a cost worth naming, and
/// `the_pre_scan_agrees_with_clap_on_every_shape_clap_accepts` is what stops it
/// becoming a drift. What it buys is that `clap`'s own usage error is already
/// subject to the choice when it is printed.
///
/// `clap` is the authority on the value: it rejects anything outside
/// [`ColorChoice::NAMES`](aplan_output::color::ColorChoice::NAMES), and this
/// falls back to `Auto` rather than guessing, so a *misspelled* value does not
/// change the terminal on its way to being refused. That is the only refusal
/// this fallback covers. `--color` given twice at one level is refused too
/// ("cannot be used multiple times"), and there the last value is applied
/// first and then the run is rejected -- which is visible only as the colour of
/// the error, and is the same answer `clap` would have reached had it accepted.
/// Across the subcommand a repeat *is* accepted, and the later one wins in both
/// readers.
///
/// Both spellings, since `clap` accepts both. `--color` with no value is left at
/// `Auto` for `clap` to complain about. Nothing after a bare `--` is read: those
/// are operands, and a task titled `--color=always` is not a request.
fn color_from_argv(
    args: impl Iterator<Item = std::ffi::OsString>,
) -> aplan_output::color::ColorChoice {
    use aplan_output::color::ColorChoice;
    let mut args = args.take_while(|a| a != "--");
    while let Some(arg) = args.next() {
        let Some(arg) = arg.to_str() else { continue };
        let value = if let Some(v) = arg.strip_prefix("--color=") {
            Some(v.to_string())
        } else if arg == "--color" {
            args.next().and_then(|v| v.to_str().map(str::to_string))
        } else {
            None
        };
        if let Some(value) = value {
            // The *last* wins, matching `clap`, so the two readers cannot
            // disagree about a repeated flag.
            let mut found = ColorChoice::parse(&value).unwrap_or_default();
            for rest in args.by_ref() {
                if let Some(rest) = rest.to_str()
                    && let Some(v) = rest.strip_prefix("--color=")
                {
                    found = ColorChoice::parse(v).unwrap_or_default();
                }
            }
            return found;
        }
    }
    ColorChoice::default()
}

/// Write `s` to stdout, treating a closed downstream pipe (e.g. `aplan ready |
/// head`) as a clean exit rather than the panic `println!` would raise.
///
/// Through `anstream`, so what this crate prints is subject to the same colour
/// choice as what `aplan-output` renders. Before `A1-286` it wrote to
/// `std::io::stdout()` directly, which meant the CLI's own lines could be
/// neither styled nor stripped: `A1-206`'s claim that these went "through the
/// same writer as `render_human`" was about the *fold* helper, not the stream.
fn write_stdout(s: &str) -> Result<()> {
    use std::io::Write as _;
    match anstream::stdout().write_all(s.as_bytes()) {
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
    // Loaded even when the root is given, and this is the point: `--root` chooses
    // where the plan is, not whether the config is checked. Returning early made
    // it a bypass, so `ready --root aplan` printed a confident list and exited 0
    // in a repository whose `.aplan.yml` could not load, along with `summary`,
    // `tasks`, `critical-path` and `graph`. That is the failure this file already
    // names for a plan that did not load, one input over: the same run without
    // `--root` fails, so the flag was deciding the verdict (`A1-068`).
    let (config, config_dir) = config_at(std::path::Path::new("."))?;
    if let Some(r) = cli_root {
        return Ok(r);
    }
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
/// The task files that would not load at all.
///
/// Split from [`loaded_cleanly`] because two commands need the *fact* without its
/// retelling: `verify` renders a report carrying these same diagnostics, and
/// printing them to stderr as well said everything twice (`A1-106`).
fn load_errors(plan: &aplan_core::load::Plan) -> Vec<&aplan_core::Diagnostic> {
    plan.load_diagnostics
        .iter()
        .filter(|d| d.severity == aplan_core::Severity::Error)
        .collect()
}

fn loaded_cleanly(plan: &aplan_core::load::Plan) -> bool {
    let errors = load_errors(plan);
    if errors.is_empty() {
        return true;
    }
    errln!(
        "{}",
        aplan_output::fold::fold(
            &format!(
                "aplan: {} task file(s) failed to load, any answer would be incomplete. \
                 Run `aplan check` for details:",
                errors.len()
            ),
            aplan_output::fold::err_width(),
            0
        )
    );
    for d in errors {
        // Through the same writer as `render_human`, so a finding reads the
        // same wherever it is printed (`A1-206`). Four, so a folded message
        // lines up under its own text rather than under the rule code.
        let body = match &d.file {
            Some(f) => format!("{}: {} ({})", d.code, d.message, f.display()),
            None => format!("{}: {}", d.code, d.message),
        };
        // Folded against the room left after the indent, then indented -- not
        // folded with the indent inside the text, which `fold` would collapse
        // along with every other run of whitespace.
        for line in
            aplan_output::fold::fold(&body, aplan_output::fold::err_width().saturating_sub(2), 2)
                .lines()
        {
            errln!("  {line}");
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

/// The exit-code contract for a report: clean -> 0, at least one error -> 1, and
/// with `deny_warnings`, a warning counts as a finding rather than as advice.
///
/// Still two outcomes, deliberately. A repository that turns this on is saying a
/// warning should block, not that it wants a third exit code: `1` goes on meaning
/// "at least one finding" to a consumer's CI, which is the whole value of the
/// code being a contract (ADR-0007).
fn verdict(report: &aplan_core::Report, policy: &aplan_core::Verdict) -> u8 {
    if report.denied_by(policy) {
        EXIT_VIOLATIONS
    } else {
        EXIT_OK
    }
}

/// The two flags that say whether a warning blocks, for a command that writes.
///
/// `check` and `verify` have taken these since `A1-112`; the mutating commands
/// did not, and read `deny_warnings` from `.aplan.yml` alone. So a repository
/// whose CI runs `aplan check --deny-warnings` -- the flag, not the config key --
/// had a `close` applying a *weaker* policy than its own gate: the command
/// reported what it added and proceeded, and CI failed on it ten minutes later.
/// `p0000000000005` hit that twice and read it as one rule's ergonomics
/// (`A1-266`).
///
/// Flattened rather than repeated, so "every command that writes takes them" is
/// one declaration and `every_mutating_command_can_be_told_the_policy` can say
/// so by walking clap's own tree.
#[derive(Debug, Clone, Copy, clap::Args)]
pub struct VerdictFlags {
    /// Fail on warnings too, so every finding blocks. Overrides `deny_warnings`
    /// in `.aplan.yml`.
    #[arg(long)]
    deny_warnings: bool,
    /// Do not fail on warnings, whatever `.aplan.yml` says.
    #[arg(long, conflicts_with = "deny_warnings")]
    no_deny_warnings: bool,
}

/// The repository's verdict policy: whether a warning blocks, and which rules it
/// hears without being stopped by (`A1-112`).
fn verdict_policy(
    config: &aplan_config::Config,
    deny_warnings: bool,
    no_deny_warnings: bool,
) -> aplan_core::Verdict {
    aplan_core::Verdict {
        deny_warnings: resolve_deny_warnings(config, deny_warnings, no_deny_warnings),
        advisory: config.advisory.clone().unwrap_or_default(),
    }
}

/// Whether a warning blocks: an explicit flag in either direction wins over
/// `.aplan.yml`, which wins over the default (off).
fn resolve_deny_warnings(
    config: &aplan_config::Config,
    deny_warnings: bool,
    no_deny_warnings: bool,
) -> bool {
    if deny_warnings {
        return true;
    }
    if no_deny_warnings {
        return false;
    }
    config.deny_warnings.unwrap_or(false)
}

/// Say why an otherwise-clean run is failing.
///
/// On stderr, so a `--format json` consumer's stdout stays a single JSON
/// document. Without this the human output reads `ok: 12 tasks, 3 warning(s)`
/// and then exits `1`, which looks like a bug in the tool rather than the policy
/// the repository asked for.
fn note_denied_warnings(report: &aplan_core::Report, policy: &aplan_core::Verdict) {
    if !policy.deny_warnings {
        return;
    }
    // Counted against the policy, like the verdict and the summary: a repository
    // with an advisory *error* rule still needs to be told that its warnings are
    // what stopped the build, and `report.errors()` would count the downgraded
    // findings and stay silent (`A1-112`).
    let warnings = report.blocking(policy, aplan_core::Severity::Warning);
    if report.blocking(policy, aplan_core::Severity::Error) == 0 && warnings > 0 {
        errln!(
            "{e}denied:{e:#} {warnings} warning(s), and `deny_warnings` is set (no errors)",
            e = aplan_output::style::ERROR
        );
    }
}

/// Say that `check` did not resolve the evidence the plan carries (`A1-173`).
///
/// `check` is structural -- schema, DAG, and the rules that need no repository --
/// so a `done` task whose cited commit or test stopped resolving passes it
/// clean. That is the design. What it did not do is *say so*, and
/// `ok: 1 tasks, 0 warning(s)` beside `--deny-warnings` reads like a thorough
/// answer. The fifth pilot hit it in two repositories and called it "the single
/// most likely misconfiguration, because `check` alone looks like it is
/// working" (pre-public adoption evidence item 9); one of the two sat
/// undetected until a later re-audit.
///
/// **Only where it would matter**, which is the judgement this records rather
/// than assumes. A plan holding no resolvable citation loses nothing by silence,
/// and a note on every run of the fastest command in the tool is the flood
/// ADR-0015 refuses. The count is [`aplan_core::unresolved_by_check`], shared
/// with the MCP surface so the two cannot describe one plan differently.
///
/// Not printed when the plan did not load: `cmd_check` returns before this, and
/// a partial plan's count would be wrong as well as beside the point.
///
/// The third instance of a shape this tool already has twice: `--no-config`
/// announces the policy it did not apply, and `check-docs` refuses outright
/// rather than report a clean run it cannot back. A gate that cannot fire has to
/// say so.
fn note_unresolved_evidence(plan: &aplan_core::Plan) {
    let left = aplan_core::unresolved_by_check(plan);
    if !left.any() {
        return;
    }
    let (citations, tasks) = (left.citations, left.tasks);
    errln!(
        "{}",
        aplan_output::fold::fold(
            &format!(
                "note: `check` is structural and did not open the repository, so {citations} \
                 citation(s) on {tasks} task(s) are unchecked rather than passing -- a cited \
                 commit or test that has rotted passes here. `aplan verify` is the half that \
                 resolves them; run it in the same gate."
            ),
            aplan_output::fold::err_width(),
            0,
        )
    );
}

/// **What `verify` could not open, said once, against the configuration.**
///
/// The twin of [`note_unresolved_evidence`], and the same argument one rung up.
/// `check` is structural and says so; `verify` opens what it has a resolver for,
/// and a resolver joins only when `.aplan.yml` declares it. Without a `test:`
/// runner the test resolver never joins, so a `test` citation stays at L0 and
/// the run reported `evidence resolved` over it (`A1-253`).
///
/// This repository was the first instance and fixed it locally by configuring a
/// runner (`A1-120`), recording that the general version "generalises past this
/// repository: any adopter can be in it, and nothing tells them". The ninth
/// pilot was that adopter (pre-public adoption evidence item 3).
///
/// Against the configuration rather than against each task: the cause is one
/// missing declaration, and a finding per citation would report a repository's
/// single choice twenty-eight times. The count is on the summary line as well,
/// because stderr is what a pipe separates (`A1-091`, and `Kind::failure_cause`
/// for the same reason).
fn note_unopened_evidence(unopened: &aplan_evidence::Unopened) {
    if !unopened.any() {
        return;
    }
    let (citations, tasks, kinds) = (unopened.citations, unopened.tasks, unopened.kinds_said());
    errln!(
        "{}",
        aplan_output::fold::fold(
            &format!(
                "note: nothing is configured to check {kinds}, so {citations} citation(s) on \
                 {tasks} task(s) were recorded rather than resolved. Declare a runner or a \
                 checker in `.aplan.yml`, or read the verdict as being about the other kinds."
            ),
            aplan_output::fold::err_width(),
            0,
        )
    );
}

/// The declared vocabularies as the rules see them.
///
/// The config's two shapes collapse here: a bare list is a family with no
/// cardinality, which is exactly what it has always meant, so an existing
/// `.aplan.yml` produces the same `Options` it did before `A1-133`.
fn tag_families_of(
    config: &aplan_config::Config,
) -> std::collections::BTreeMap<String, aplan_core::TagFamily> {
    let Some(families) = &config.tag_families else {
        return std::collections::BTreeMap::new();
    };
    families
        .iter()
        .map(|(name, family)| {
            let (status, milestones) = family.scope();
            (
                name.clone(),
                aplan_core::TagFamily {
                    members: family.members().to_vec(),
                    require: match family.require() {
                        aplan_config::Cardinality::Optional => aplan_core::Cardinality::Optional,
                        aplan_config::Cardinality::AtMostOne => aplan_core::Cardinality::AtMostOne,
                        aplan_config::Cardinality::AtLeastOne => {
                            aplan_core::Cardinality::AtLeastOne
                        }
                        aplan_config::Cardinality::ExactlyOne => {
                            aplan_core::Cardinality::ExactlyOne
                        }
                    },
                    status: match status {
                        aplan_config::StatusScope::Any => aplan_core::StatusScope::Any,
                        aplan_config::StatusScope::Live => aplan_core::StatusScope::Live,
                        aplan_config::StatusScope::Done => aplan_core::StatusScope::Done,
                        aplan_config::StatusScope::Dropped => aplan_core::StatusScope::Dropped,
                        aplan_config::StatusScope::Terminal => aplan_core::StatusScope::Terminal,
                    },
                    milestones: match milestones {
                        aplan_config::MilestoneScope::Include => {
                            aplan_core::MilestoneScope::Include
                        }
                        aplan_config::MilestoneScope::Exclude => {
                            aplan_core::MilestoneScope::Exclude
                        }
                        aplan_config::MilestoneScope::Only => aplan_core::MilestoneScope::Only,
                    },
                },
            )
        })
        .collect()
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

    // Resolved against the *config file's* directory, not against `traces_root`:
    // a declaration is the repository's stated position and must mean the same
    // thing however a run is invoked. Resolving it against the root was the first
    // attempt, and `--traces-root docs` then silently redefined `../sibling` as
    // `<repo>/sibling`, so a flag changed what a config key meant (`A1-143`,
    // found by its audit). The comparison itself is done in absolute terms, so
    // the two frames still meet.
    let external = config
        .traces_external
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|d| config_dir.map_or_else(|| PathBuf::from(&d), |base| base.join(&d)))
        .collect();

    // `.aplan.yml`'s `policy` list, in file order so the documented tie rule
    // (narrowest wins, then the earlier entry) is the one that applies (`A1-183`).
    let evidence_scopes: Vec<aplan_core::validate::EvidenceScope> = config
        .policy
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|p| aplan_core::validate::EvidenceScope {
            // One parser, shared with `Scope::validate`, which the loader has
            // already run: an unparseable status is refused before it reaches
            // here rather than dropped into a scope that then matches everything.
            selector: p.when.selector().unwrap_or_default(),
            named_dimensions: p.when.named_dimensions(),
            require_min_strength: p
                .evidence
                .as_ref()
                .and_then(|e| e.require_min_strength.clone()),
            exempt_tags: p.evidence.as_ref().and_then(|e| e.exempt_tags.clone()),
        })
        .collect();

    Options {
        require_effort,
        // `None` unless the plan asked: a repository that has never heard of
        // version discipline meets none of it (`A1-312`, ADR-0034).
        require_impact: config
            .version
            .as_ref()
            .filter(|v| v.requires_impact())
            .map(aplan_config::VersionPolicy::floors),
        traces_root,
        // Resolved against `traces_root`, exactly as a trace target is: the two
        // are compared with each other, so they have to be in one frame, and the
        // frame that needs no absolute paths is the one traces already live in.
        // Absolutising the root instead was the first attempt, and it changed the
        // path `APLAN018`'s fix line tells a reader to paste (`A1-143`).
        traces_external: external,
        require_min_strength,
        evidence_scopes,
        exempt_tags,
        // APLAN041's comparison needs the plan, which is not read yet: filled in
        // by `with_agents_block` once it is.
        agents_block: None,
        // APLAN018 is config-only (repo policy, not a per-run knob), like APLAN017.
        detect_drift: config.detect_drift.unwrap_or(false),
        // APLAN022 is config-only too: membership totality is a repo policy.
        require_cut_membership: config.require_cut_membership.unwrap_or(false),
        // APLAN026 likewise: a tag vocabulary is something a repository declares
        // once, not something a run turns on.
        tag_families: tag_families_of(config),
        // So `APLAN029` can point at the declaration it is about.
        config_file: config_dir.map(|d| d.join(aplan_config::CONFIG_FILE)),
    }
}

/// Refuse a run whose declared external traces roots are not there (`A1-143`).
///
/// The point of declaring one is that its absence stops being a per-task
/// `APLAN008` on the day a job runs without the sibling. So the absence is
/// reported here, once, as what it is: a **configuration** error, exit `2` per
/// ADR-0007, naming the key and the path rather than the tasks that happen to
/// trace it. A repository that declares nothing cannot reach this.
fn refuse_absent_external_roots(opts: &Options) -> Result<()> {
    let missing: Vec<String> = opts
        .traces_external
        .iter()
        .filter(|d| !d.is_dir())
        // Normalised, so the message names the directory rather than the way it
        // was spelled: `/repo/../gone` is where it looked, `/gone` is where a
        // reader would go.
        .map(|d| aplan_core::anchor::normalise(d).display().to_string())
        .collect();
    match missing.as_slice() {
        [] => Ok(()),
        [one] => anyhow::bail!(
            "`traces_external` declares `{one}`, which is not a directory here. A trace into \
             another repository needs that repository checked out, and this run has nothing to \
             resolve one against."
        ),
        many => anyhow::bail!(
            "`traces_external` declares {} roots that are not directories here ({}). A trace \
             into another repository needs that repository checked out, and this run has nothing \
             to resolve them against.",
            many.len(),
            many.join(", ")
        ),
    }
}

/// `aplan check`, hermetic structural validation (rung L0). No git, no network,
/// no subprocess.
fn cmd_check(root: &std::path::Path, args: &CheckArgs) -> Result<u8> {
    refuse_selection_on_a_gate("check", &args.select)?;
    let format = format_for("check", &args.format)?;
    let plan = load_checked(root)?;
    // `.aplan.yml` is found by walking up from the plan directory, so `check` and
    // `verify` converge on the same top-level file. Absent is fine; an unknown key
    // in it is an error, never silently ignored.
    let (config, config_dir) = config_at(root)?;
    let opts = resolve_check_options(
        &config,
        config_dir.as_deref(),
        args.no_require_effort,
        args.traces_root.clone(),
        args.no_traces,
    );
    refuse_absent_external_roots(&opts)?;
    let opts = with_agents_block(opts, root, config_dir.as_deref(), &config, &plan);
    let report = validate(&plan, &opts);
    let policy = verdict_policy(&config, args.deny_warnings, args.no_deny_warnings);
    // A plan that did not load is refused whatever the policy says, so that the
    // four commands that read a plan give one answer about it: `ready`, `why` and
    // `verify` all already stop here (`A1-112`).
    let unloadable = load_errors(&plan).len();
    if unloadable > 0 {
        render_tolerant(|| render_partial(&report, format, &policy, unloadable))?;
        return Ok(EXIT_VIOLATIONS);
    }
    render_tolerant(|| render(&report, format, &policy))?;
    note_denied_warnings(&report, &policy);
    note_unresolved_evidence(&plan);
    Ok(verdict(&report, &policy))
}

/// The plan `add` would produce: the new task, and the cut that would list it.
fn post_state_for_add(
    plan: &aplan_core::load::Plan,
    task: &aplan_core::Task,
    cut: Option<&str>,
) -> aplan_core::load::Plan {
    let mut after = aplan_core::mutation::with_added(
        plan,
        std::path::PathBuf::from(format!("{}.md", task.id)),
        task.clone(),
    );
    if let Some(cut_id) = cut
        && let Some(slot) = after.tasks.iter_mut().find(|lt| lt.task.id == cut_id)
    {
        slot.task.blocked_by.push(task.id.clone());
    }
    after
}

/// The task a produced file would load as, named for the message on failure.
fn task_of(content: &str, id: &str) -> Result<aplan_core::Task> {
    aplan_core::mutation::task_of(content).map_err(|e| anyhow::anyhow!("`{id}`: {e}"))
}

/// The line `APLAN022` cannot say for itself: there is no cut to join (`A1-252`).
///
/// The rule's remedy is "add the task to exactly one milestone's `blocked_by`",
/// which is right and, on an empty plan, names something that does not exist.
/// Two pilots stopped on the first command of their adoption -- `p00002` here,
/// and `p0000000000005` on the id convention a line earlier -- and each log
/// judged its own instance too small to file (pre-public adoption evidence item 1,
/// pre-public adoption evidence item 1).
///
/// A rule cannot say it: `APLAN022` sees one task and reports one fact about it,
/// and a rule that reworded itself when the plan happened to be empty would be
/// two rules under one code. The mutation path has the whole plan in hand and is
/// where the author is standing, so the sentence goes here.
///
/// Beside the findings rather than instead of them, and only when the plan holds
/// no milestone at all: on a plan that has cuts, "add it to one" is complete
/// advice and a second paragraph would be noise.
fn no_cut_to_join(
    before: &aplan_core::load::Plan,
    delta: &aplan_core::mutation::Delta,
    policy: &aplan_core::Verdict,
) -> String {
    use aplan_core::mutation::Bootstrap;

    // Which half of the deadlock this is, asked of the crate that owns the
    // rules: whether an empty cut blocks is a fact about `APLAN021` and this
    // repository's policy, and the wording is all that belongs here.
    match delta.bootstrap(before, policy) {
        Bootstrap::No => String::new(),
        // Reached only when the empty cut was refused, so there is no policy
        // question left to ask.
        Bootstrap::EmptyCut => "\n\nA cut is created with its work, not before it: an empty \
                                one is this finding wherever it is added. Name the members in \
                                the same command (`--blocked-by <ID>`, for work the plan \
                                already has and that is in no cut), or write the cut and its \
                                first tasks in one file and run `aplan add --from plan.yml`, \
                                which is what a plan with nothing in it yet needs."
            .to_string(),
        // The two-step, which is simpler and which this policy allows.
        Bootstrap::NoCut => "\n\nThis plan has no cut yet, so there is no milestone to add it \
                             to. A cut is an ordinary task tagged `milestone` whose \
                             `blocked_by` lists its members: `aplan add \"the first cut\" \
                             --id <ID> --tag milestone` makes one, and `--cut <ID>` puts work \
                             in it."
            .to_string(),
        // The deadlock. This branch used to give the sentence above whatever the
        // policy was, sending the reader at a command their own `deny_warnings`
        // declines -- so the two refusals each named the other (`A1-292`).
        Bootstrap::NoCutAndNoneCanBeEmpty => "\n\nThis plan has no cut yet, so there is no \
             milestone to add it to, and this repository's policy refuses a cut with no \
             members -- so neither can be added first. Write the cut and its first tasks in \
             one file and run `aplan add --from plan.yml`: the entries are applied as one \
             change, and a task naming `cut:` joins a milestone the same file declares. \
             `README.md`, under \"Starting a plan\", has the file."
            .to_string(),
    }
}

/// Refuse a mutation the hermetic rules would reject, before anything is written.
///
/// One path for every mutating command, which is the point: the rule engine had
/// four call sites and all four were read commands, so each mutation hand-rolled
/// a partial pre-check and they drifted apart (`A1-113`).
///
/// A new *error* refuses and names the rule. A new *warning* is reported and left
/// to `deny_warnings`, because refusing one would block a legitimate two-step
/// rearrangement. Diagnostics the plan already had are not attributed to the
/// mutation.
fn refuse_bad_mutation(
    root: &std::path::Path,
    what: &str,
    before: &aplan_core::load::Plan,
    after: &aplan_core::load::Plan,
    verdict: VerdictFlags,
) -> Result<()> {
    refuse_bad_mutation_unless(root, what, before, after, false, verdict)
}

/// The same, with the escape an author repairing a broken plan needs.
///
/// One shape, not two: `--no-resolve` already means "record this without the
/// check", and a second flag for the rules would be a second way to say the same
/// thing. It says what it skipped, because silence about a check nothing ran
/// reads exactly like a check that passed (`A1-109`, `A1-113`).
fn refuse_bad_mutation_unless(
    root: &std::path::Path,
    what: &str,
    before: &aplan_core::load::Plan,
    after: &aplan_core::load::Plan,
    skip: bool,
    verdict: VerdictFlags,
) -> Result<()> {
    let (config, config_dir) = config_at(root)?;
    let opts = resolve_check_options(&config, config_dir.as_deref(), false, None, false);
    // The flags, not just the config key: a repository whose CI is
    // `check --deny-warnings` was refused less here than there (`A1-266`).
    let policy = verdict_policy(&config, verdict.deny_warnings, verdict.no_deny_warnings);
    let delta = aplan_core::mutation::delta(before, after, &opts);
    if delta.added.is_empty() {
        return Ok(());
    }
    if skip {
        // `fold_block`, like the refusal below and the exit printer: these notes
        // are a headline with findings listed under it, and folding the whole
        // string at once would run the list into the headline (`A1-206`).
        write_stdout(&aplan_output::fold::fold_block(
            &format!(
                "note: `--no-resolve`: {} finding(s) the rules report were not checked:\n  {}\n",
                delta.added.len(),
                delta.lines().join("\n  ")
            ),
            aplan_output::fold::out_width(),
            6,
        ))?;
        return Ok(());
    }
    if delta.refuses(&policy) {
        anyhow::bail!(
            "refusing to {what}: it would add {} finding(s) `check` reports:\n  {}{}",
            delta.added.len(),
            delta.lines().join("\n  "),
            no_cut_to_join(before, &delta, &policy)
        );
    }
    // Reported, not refused: the mutation goes ahead and the author is told what
    // it cost, which is what `deny_warnings` exists to escalate.
    write_stdout(&aplan_output::fold::fold_block(
        &format!(
            "note: this adds {} finding(s) `check` will report:\n  {}\n",
            delta.added.len(),
            delta.lines().join("\n  ")
        ),
        aplan_output::fold::out_width(),
        6,
    ))?;
    Ok(())
}

/// Surface a `.aplan.yml` problem as a usage error (exit 2), with context.
fn config_err(e: aplan_config::Error) -> anyhow::Error {
    // An unknown key is the one config failure with two causes, and the message
    // that could not tell them apart is what stopped the first adopter: they
    // added `test.covers` before staging the binary that knows it, and read a
    // sentence identical to a misspelling. The one fact that settles it is the
    // running version, and it was the one thing the tool did not print
    // (`A1-068`).
    if let aplan_config::Error::UnknownKeys(unknown) = &e {
        return anyhow::anyhow!(
            "loading .aplan.yml: {unknown}\nnote: this is aplan {}, which accepts {} config \
             key(s); `aplan facts` lists them. A key added by a newer aplan reads exactly like \
             a typo from here, so if you staged this binary, upgrade it before adopting the key \
             (`docs/upgrading.md` leads with that ordering).",
            env!("CARGO_PKG_VERSION"),
            aplan_config::accepted_keys().len(),
        );
    }
    // Name the file that was actually read. With `--config` the failure is about
    // the file the caller chose, and "loading .aplan.yml" sent them looking for
    // a file they had deliberately not used (`A1-155`, found by its audit).
    //
    // For a discovered config there is no prefix: every message below already
    // names `.aplan.yml`, and routing every command through one loader made the
    // duplication visible where half the call sites had been adding it and half
    // had not.
    match CONFIG_SOURCE.get() {
        Some(aplan_config::ConfigSource::File(path)) => {
            anyhow::Error::new(e).context(format!("loading --config {}", path.display()))
        }
        _ => anyhow::Error::new(e),
    }
}

/// `aplan verify`, resolve evidence against the repository (rungs L1/L2, and L3
/// with `--execute`).
fn cmd_verify(root: &std::path::Path, args: &VerifyArgs) -> Result<u8> {
    refuse_selection_on_a_gate("verify", &args.select)?;
    let format = format_for("verify", &args.format)?;
    let repo = args.repo.as_path();
    let execute = args.execute;
    let plan = load_checked(root)?;
    // Verify cannot speak honestly about a plan that did not load, so it does not
    // resolve a partial one: a task in the unreadable file may be the blocker or
    // the owner of a cited path, and the findings would read as complete.
    //
    // It still *answers*, in the format it was asked for: see below, once the
    // repository's own options are known. Counted rather than reported here,
    // because the report carries these same diagnostics and saying them twice
    // reads as two problems.
    let unloadable = load_errors(&plan).len();
    // The runner template comes only from the trusted `.aplan.yml`, never from a
    // task file (HLD §5.5).
    // Located rather than bare, so the hermetic rules below resolve `traces`
    // against the same directory `check` would: two commands disagreeing about
    // where a trace points would be a new way to say different things about one
    // plan, which is the whole complaint this task answers.
    let (config, config_dir) = config_at(repo)?;
    // Beside `note_unopened_evidence`, and not folded into it: with no runner and
    // no `test` citation at all, that note is silent and this is the only thing
    // that says the flag did nothing. The two overlap on one plan and say
    // different things about it -- the flag was inert, and these citations went
    // unopened -- so neither subsumes the other (`A1-253`, audit).
    if execute && config.test.is_none() {
        errln!(
            "aplan: --execute was given but `.aplan.yml` configures no `test` runner; \
             `test` evidence cannot be run."
        );
    }
    // Read the warning policy before the runner template is moved out of the
    // config: `verify` reports the same warnings `check` does, so it honours the
    // same policy rather than being the one command where a warning is advice.
    let policy = verdict_policy(&config, args.deny_warnings, args.no_deny_warnings);

    // The hermetic rules run here too, so the ladder is cumulative the way its
    // name says (`A1-058`). Before this, `verify` resolved evidence and reported
    // `0 warning(s)` on a plan `check` rejected: a claim about the subset of
    // rules it had run, phrased as a claim about the plan. An adopter wiring one
    // command into CI reaches for this one, because it sounds like the deeper
    // check, and got a permanently green build.
    //
    // The stance was already half-taken two lines above: `verify` refuses a plan
    // that did not *load*, because it "cannot speak honestly about" one. A plan
    // that loads but does not validate is the same sentence.
    let opts = aplan_evidence::VerifyOptions {
        execute,
        test_runner: config.test.clone(),
        max_unrelated_files: config.evidence.as_ref().and_then(|e| e.max_unrelated_files),
        url_check: config.url.clone(),
        // So a finding about a runner or the url checker lands on the file that
        // configured it (`A1-066`).
        config_file: config_dir
            .as_deref()
            .map(|d| d.join(aplan_config::CONFIG_FILE)),
    };
    let check_opts = resolve_check_options(
        &config,
        config_dir.as_deref(),
        args.no_require_effort,
        args.traces_root.clone(),
        args.no_traces,
    );
    // The same refusal `check` makes: `verify` is a superset of it, so a
    // configuration it cannot honour must stop both commands the same way
    // (`A1-058`, `A1-143`).
    refuse_absent_external_roots(&check_opts)?;
    // `verify` is a superset of `check` (`A1-058`), so it reports `APLAN041` too:
    // an adopter who wires one command into CI reaches for this one.
    let check_opts = with_agents_block(check_opts, root, config_dir.as_deref(), &config, &plan);
    let mut report = validate(&plan, &check_opts);
    // A plan that did not load whole is reported and not resolved. `verify`
    // renders a report, the same document `check` renders, and `check` already
    // hands a machine consumer `APLAN001` with the file and the parse error. Only
    // this command stayed silent, so a consumer reading `verify --format json`
    // learned "something is wrong, go run another command" from an empty stream
    // (`A1-106`).
    //
    // Reported with the repository's *own* options, which is why the check sits
    // here and not before the config is loaded: the MCP surface does exactly this
    // (`A1-102`), and two surfaces validating one plan under different settings
    // would be a new way to disagree about it.
    if unloadable > 0 {
        errln!(
            "aplan: {unloadable} task file(s) failed to load; below is what did, and no \
             evidence was resolved."
        );
        render_tolerant(|| render_verified_partial(&report, format, &policy, unloadable))?;
        return Ok(EXIT_VIOLATIONS);
    }
    let (evidence, spend) = aplan_evidence::verify_measured(&plan, repo, &opts)
        .context("resolving evidence against the repository")?;
    note_what_the_run_cost(&spend, opts.execute);
    report.diagnostics.extend(evidence.diagnostics);
    report.diagnostics.sort_by(|a, b| {
        a.code
            .cmp(b.code)
            .then(a.task.cmp(&b.task))
            .then(a.file.cmp(&b.file))
            .then(a.message.cmp(&b.message))
    });

    // What this configuration had no resolver for, computed from the same list
    // `verify` ran, so the summary and the note cannot disagree with the run.
    let unopened = aplan_evidence::unresolved_by_verify(&plan, repo, &opts);
    render_tolerant(|| render_verified(&report, format, &policy, &unopened))?;
    note_unopened_evidence(&unopened);
    note_denied_warnings(&report, &policy);
    Ok(verdict(&report, &policy))
}

fn run() -> Result<u8> {
    // Through `cli_command` rather than `Cli::parse`, so the width above applies
    // to the help clap prints for `--help` and for a usage error.
    let cli = <Cli as clap::FromArgMatches>::from_arg_matches(&cli_command().get_matches())
        .unwrap_or_else(|e| e.exit());
    // Chosen once, before any command runs, and read-only afterwards: the
    // selection is a property of the invocation rather than of a call site, and
    // a command that read it from somewhere else would be the hole this exists
    // to close (`A1-155`).
    select_config(if let Some(path) = cli.config {
        aplan_config::ConfigSource::File(path)
    } else if cli.no_config {
        aplan_config::ConfigSource::None
    } else {
        aplan_config::ConfigSource::Discover
    });
    // Resolved lazily inside the arms that read the plan; `schema`/`facts`/`init`
    // do not, so they never touch `.aplan.yml`.
    dispatch(cli.command, cli.root)
}

/// The configuration source this invocation chose.
///
/// A `OnceLock` rather than a parameter on fifteen `cmd_*` signatures: it is set
/// once from the parsed arguments and never again, and `config_at` is the only
/// reader. `crates/aplan/tests/trusted_config.rs` holds the line structurally --
/// no call site may reach for `aplan_config::load_config_located` and bypass it.
static CONFIG_SOURCE: std::sync::OnceLock<aplan_config::ConfigSource> = std::sync::OnceLock::new();

/// Record the selection. Called once, from `run`.
fn select_config(source: aplan_config::ConfigSource) {
    let _ = CONFIG_SOURCE.set(source);
}

/// Load configuration for `dir`, honouring this invocation's selection.
///
/// Every command reads configuration through here. `dir` is where *discovery*
/// would start, and is ignored when a file was named or configuration was
/// refused -- which is what makes `--config` a boundary rather than a preference
/// (`A1-155`).
fn config_at(dir: &std::path::Path) -> Result<(aplan_config::Config, Option<PathBuf>)> {
    let source = CONFIG_SOURCE
        .get()
        .cloned()
        .unwrap_or(aplan_config::ConfigSource::Discover);
    // Said here rather than by each command, so none can forget it: the first
    // version wired the note into `check` and `verify` only, and `close`,
    // `add` and `import` -- the commands that *write* -- ran under a refused
    // config saying nothing at all (`A1-155`, found by its audit).
    note_config_refused();
    aplan_config::load_from(&source, dir).map_err(config_err)
}

/// Say, once, that no configuration was read.
///
/// `--no-config` is deliberate, and still worth announcing: the repository's own
/// policy -- `deny_warnings`, `require_min_strength`, its runners -- was not
/// applied, and a run that looked at a plan and reported nothing about it should
/// not read like a clean bill of health from that repository's own rules
/// (`A1-155`).
fn note_config_refused() {
    static NOTED: std::sync::Once = std::sync::Once::new();
    if config_refused() {
        NOTED.call_once(|| errln!(
            "aplan: --no-config: no configuration was read, so this repository's own policy was \
             not applied and no `test` or `url` citation could be resolved -- those stay \
             unchecked rather than passing."
        ));
    }
}

/// The repository the plan sits in.
///
/// `root.parent()` of a relative `aplan` is the empty path, which is not the
/// current directory to anything that opens it -- the discovered config's
/// absolute directory hid that until an explicit `--config` stopped standing in
/// for it (`A1-155`, found by its audit).
fn plan_repository(root: &std::path::Path) -> PathBuf {
    match root.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Whether this invocation named its configuration file.
///
/// Read where a *discovered* config's directory was standing in for "the
/// repository": a named file may live anywhere, so it says where relative config
/// values resolve and nothing about where the plan's git history is (`A1-155`).
fn is_explicit_config() -> bool {
    matches!(
        CONFIG_SOURCE.get(),
        Some(aplan_config::ConfigSource::File(_))
    )
}

/// Whether this invocation deliberately read no configuration.
fn config_refused() -> bool {
    matches!(CONFIG_SOURCE.get(), Some(aplan_config::ConfigSource::None))
}

/// One arm per subcommand, and nothing else: the arms stay a readable table of
/// what each command is wired to, which is why the work lives in `cmd_*`.
/// `aplan schema`, the JSON Schema derived from the Rust types.
///
/// Its own function rather than an inline arm, so [`dispatch`] stays a table of
/// commands: it was the one arm carrying a body, and it pushed the match past
/// the hundred-line limit when `amend` arrived.
fn cmd_schema() -> Result<u8> {
    let schema = aplan_core::load::derived_schema();
    write_stdout(&format!("{}\n", serde_json::to_string_pretty(&schema)?))?;
    Ok(EXIT_OK)
}

fn dispatch(command: Command, cli_root: Option<PathBuf>) -> Result<u8> {
    match command {
        Command::Schema => cmd_schema(),
        Command::Check(args) => cmd_check(&resolve_root(cli_root)?, &args),
        Command::Verify(args) => cmd_verify(&resolve_root(cli_root)?, &args),

        Command::Ready(args) => cmd_ready(&resolve_root(cli_root)?, &args),

        Command::Why { id, all, format } => cmd_why(&resolve_root(cli_root)?, &id, all, &format),
        Command::Due(args) => cmd_due(&resolve_root(cli_root)?, &args),
        Command::Graph(args) => cmd_graph(&resolve_root(cli_root)?, &args),

        Command::Show(args) => cmd_show(&resolve_root(cli_root)?, &args),

        Command::Tree { id, depth, live } => cmd_tree(&resolve_root(cli_root)?, &id, depth, live),

        Command::Summary { format, select } => {
            cmd_summary(&resolve_root(cli_root)?, &format, &select)
        }

        Command::Paths {
            sample,
            repo,
            format,
            select,
        } => cmd_paths(&resolve_root(cli_root)?, &repo, sample, &format, &select),

        Command::Deprecations { format } => cmd_deprecations(&resolve_root(cli_root)?, &format),

        Command::Vacuity { format, select } => {
            cmd_vacuity(&resolve_root(cli_root)?, &format, &select)
        }

        Command::Stale {
            older_than,
            as_of,
            repo,
            format,
            select,
        } => stale(
            cli_root,
            older_than,
            as_of.as_deref(),
            &repo,
            &format,
            &select,
        ),

        Command::Attest {
            repo,
            attach_note,
            execute,
        } => cmd_attest(
            &resolve_root(cli_root)?,
            &repo,
            attach_note.as_deref(),
            execute,
        ),

        Command::ExportRoadmapJson { file, check } => export_roadmap_json(cli_root, &file, check),

        Command::Export(args) => cmd_export(&resolve_root(cli_root)?, &args),

        Command::ExportAgentsMd { write, check } => {
            cmd_export_agents_md(&resolve_root(cli_root)?, write.as_deref(), check.as_deref())
        }

        Command::CheckDocs { paths, format } => {
            cmd_check_docs(&resolve_root(cli_root)?, &paths, &format)
        }

        Command::CriticalPath { format } => cmd_critical_path(&resolve_root(cli_root)?, &format),
        Command::Drop { ids, cut, verdict } => {
            cmd_drop(&resolve_root(cli_root)?, &ids, cut, verdict)
        }
        Command::Mv {
            ids,
            cut,
            phase,
            verdict,
        } => cmd_mv(
            &resolve_root(cli_root)?,
            &ids,
            cut.as_deref(),
            phase,
            verdict,
        ),
        Command::Tasks { format, select } => cmd_tasks(&resolve_root(cli_root)?, &format, &select),
        Command::Roadmap(args) => cmd_roadmap(&resolve_root(cli_root)?, &args),

        Command::Close {
            id,
            evidence,
            repo,
            restamp,
            no_resolve,
            verdict,
        } => cmd_close(
            &resolve_root(cli_root)?,
            &id,
            &evidence,
            &repo,
            restamp,
            no_resolve,
            verdict,
        ),

        Command::Add(args) => cmd_add(&resolve_root(cli_root.clone())?, &args),
        Command::Amend(args) => cmd_amend(&resolve_root(cli_root)?, &args),

        Command::Supersede(args) => cmd_supersede(&resolve_root(cli_root)?, &args),

        Command::Dep(args) => cmd_dep(&resolve_root(cli_root)?, &args),
        Command::Import(args) => cmd_import(&resolve_root(cli_root)?, &args),
        Command::Mcp => cmd_mcp(cli_root),
        Command::Init { dir } => cmd_init(&dir),

        Command::Facts { require, identity } => cmd_facts(&require, identity),
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
/// Serve MCP over stdio.
///
/// The policy comes from `.aplan.yml` through the same resolution `check` uses,
/// so a plan judged over MCP is judged the way the CLI judges it. Nothing is
/// printed to stdout: that is the protocol channel, and a stray line would
/// corrupt the stream.
/// Serve a session that can say what this build is, and refuse everything else.
///
/// An old binary rejects a config key a newer aplan added, and until this that
/// stopped `aplan mcp` before it served -- so an agent met a dead process in
/// exactly the case where *which build are you talking to* is the answer.
/// `p0000014`'s server was a stale binary, and a rejected key is how a stale
/// binary usually announces itself (`A1-259`, audit).
///
/// The error is not softened: every tool but `aplan_identity` returns it, so
/// nothing runs under a policy nobody chose.
fn serve_only_identity(said: &anyhow::Error) -> Result<u8> {
    let identity = identity_value();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the MCP runtime")?;
    runtime.block_on(aplan_mcp::serve_unconfigured(format!("{said:#}"), identity))?;
    Ok(EXIT_OK)
}

fn cmd_mcp(cli_root: Option<PathBuf>) -> Result<u8> {
    // The root is resolved here rather than by the dispatcher, because resolving
    // it *reads the configuration* -- deliberately, so `--root` cannot bypass a
    // config check (`A1-068`) -- and this is the one command that must survive a
    // configuration it cannot read (`A1-259`, audit).
    let root = match resolve_root(cli_root) {
        Ok(root) => root,
        Err(e) => return serve_only_identity(&e),
    };
    let root = root.as_path();
    // Resolved from the *plan* root, exactly as `close` and `check` do, and never
    // from the process's working directory. An MCP client launches the server
    // from wherever it likes: reading `.aplan.yml` relative to that found no
    // config at all for a client started elsewhere, so `detect_drift` came back
    // false and an agent's close stamped nothing, on a repository that had asked
    // for drift detection (`A1-100`). Passing the config's own directory is the
    // other half, since a relative `traces_root` resolves against it.
    // A configuration this build cannot read does not stop the server, it stops
    // every tool *except* the one that says what this build is. That is the
    // whole point of `A1-259`: an old binary rejects a config key a newer aplan
    // added, and until this it exited before serving -- so an agent met a dead
    // process in exactly the case where "which build are you talking to" is the
    // answer. `p0000014`'s server was a stale binary, and this is how a stale
    // binary usually announces itself. The error is not softened: every other
    // tool returns it, so nothing proceeds under a policy nobody chose.
    let (config, config_dir) = match config_at(root) {
        Ok(pair) => pair,
        Err(e) => return serve_only_identity(&e),
    };
    // Traces resolve against the repository the plan lives in, never against `.`.
    // The default `traces_root` is the process's working directory, which is the
    // right answer for a command a person runs inside their repository and a
    // meaningless one for a server a client starts wherever it likes. Without
    // this, `detect_drift` was on, every traced document was unreadable, and an
    // agent's close stamped nothing while reporting success.
    // The plan's own repository, which is what the config directory stood in for
    // while a config was always discovered inside it. With `--config` naming a
    // trusted file *outside* the tree under review, the two part company: a
    // relative value inside that file resolves against the file, but where the
    // plan and its git history live is still the plan's own location. Without
    // this, a reviewing server resolved evidence against the trusted directory
    // and answered "not a git repository" (`A1-155`, found by its audit).
    let plan_repo = Some(plan_repository(root));
    let default_traces_root = if config_refused() || is_explicit_config() {
        plan_repo.clone()
    } else {
        config_dir.clone().or_else(|| plan_repo.clone())
    };
    let options = resolve_check_options(
        &config,
        config_dir.as_deref(),
        false,
        default_traces_root,
        false,
    );
    // `APLAN041` reaches this surface too, and it is the surface that needs it
    // most: the block is what an *agent* reads, and this is the agent's
    // `check`. Rendered once here, because the server has no plan until a tool
    // call arrives and rendering per call would be a second implementation of
    // `export-agents-md` living in `aplan-mcp` -- the shape both known MCP
    // divergences had (`A1-100`, `A1-101`).
    //
    // The limit that buys, stated rather than discovered: `id_examples` is the
    // one part of the contract read from the plan, so a session that changes
    // which id families are *live* and regenerates `AGENTS.md` from the CLI
    // meanwhile will go on reporting a block that is in fact current, until the
    // server restarts. `A1-180` made that field a policy rather than a tally
    // precisely so it does not move on ordinary work, and the failure is a
    // warning whose fix is a no-op rather than a wrong answer about the plan.
    let options = mcp_agents_block(options, root, config_dir.as_deref(), &config);
    // The same refusal the commands make, at the one moment this surface reads
    // the config: a server that cannot resolve a declared external root would
    // otherwise answer every `aplan_check` with `APLAN008` about tasks that are
    // fine, which is precisely what declaring the root is supposed to prevent
    // (`A1-143`, found by its audit -- the CLI had it and this did not).
    refuse_absent_external_roots(&options)?;
    // The warning policy travels with the options, or the surface an agent uses
    // reports a denied plan as clean (`A1-101`).
    let deny_warnings = resolve_deny_warnings(&config, false, false);
    // Where git lives: the repository the plan sits in, on the same reasoning as
    // the traces root above, and never the caller's directory.
    let repo_root = if config_refused() || is_explicit_config() {
        plan_repo.clone()
    } else {
        config_dir.clone().or_else(|| plan_repo.clone())
    }
    .unwrap_or_else(|| std::path::PathBuf::from("."));
    let policy = aplan_mcp::Policy {
        options,
        id_strategy: config.id_strategy.unwrap_or_default(),
        // The agent surface reads the same verdict policy as the CLI, or the two
        // disagree about whether a plan passes (`A1-101`, `A1-112`).
        verdict: aplan_core::Verdict {
            deny_warnings,
            advisory: config.advisory.clone().unwrap_or_default(),
        },
        verify: aplan_evidence::VerifyOptions {
            // Never from this surface, whatever the config says (`A1-102`).
            execute: false,
            test_runner: config.test.clone(),
            max_unrelated_files: config.evidence.as_ref().and_then(|e| e.max_unrelated_files),
            // The agent surface reaches the network only where the CLI does, and
            // `aplan_verify` is the CLI's `verify` (`A1-104`).
            url_check: config.url.clone(),
            config_file: config_dir
                .as_deref()
                .map(|d| d.join(aplan_config::CONFIG_FILE)),
        },
        repo_root,
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the MCP runtime")?;
    runtime
        // The identity this build reports, computed by the one function that
        // computes it: `aplan_identity` must answer with the same bytes as
        // `aplan facts --identity`, and a second implementation in the server
        // is exactly what `A1-259` came to prevent.
        .block_on(aplan_mcp::serve(
            root.to_path_buf(),
            policy,
            identity_value(),
        ))
        .context("serving MCP over stdio")?;
    Ok(EXIT_OK)
}

#[derive(Subcommand, Debug)]
pub enum DepCommand {
    /// Record that a task comes after another.
    Add(DepArgs),
    /// Remove that edge again.
    Rm(DepArgs),
}

#[derive(clap::Args, Debug)]
pub struct DepArgs {
    /// The task that comes second.
    id: String,
    /// The task it comes after, which must finish first.
    #[arg(long)]
    after: String,
    /// Record a *preference* rather than a dependency: `ready` offers this task
    /// after the other one when both are offerable, and nothing else changes.
    /// `critical-path`, `why` and the rules that judge a `done` task do not read
    /// it, so a preference can never stop work or lengthen a chain.
    #[arg(long)]
    prefer: bool,
    /// Whether a warning blocks this write, as `check` and `verify` take it.
    #[command(flatten)]
    verdict: VerdictFlags,
}

#[derive(clap::Args, Debug)]
pub struct SupersedeArgs {
    /// The closed task whose traced outcome no longer holds.
    id: String,
    /// The successor's title: what now needs doing.
    title: String,
    /// Place the successor in this cut, as `add --cut` does: by id or by label.
    #[arg(long)]
    cut: Option<String>,
    /// Estimated days for the successor.
    #[arg(long)]
    effort: Option<f64>,
    /// Owner of the successor. Defaults to the predecessor's.
    #[arg(long)]
    owner: Option<String>,
    /// Whether a warning blocks this write, as `check` and `verify` take it.
    #[command(flatten)]
    verdict: VerdictFlags,
}

#[derive(clap::Args, Debug)]
pub struct RoadmapArgs {
    /// Output format: human, json, or markdown (the committed companion).
    #[arg(long, default_value = "human")]
    format: String,
    /// The command your repository regenerates the committed file with, named in
    /// its banner. Give this when a wrapper (`make roadmap`, `cargo xtask
    /// gen-roadmap`) is what people actually run; without it the banner names the
    /// plain `aplan` command and no output path, since only you know where the
    /// file goes (markdown only).
    #[arg(long, value_name = "COMMAND")]
    regenerate_with: Option<String>,
}

/// Everything `aplan check` was asked to do.
///
/// A `clap` group rather than inline fields: the flags are a policy, and a
/// policy passed as six positional arguments is one nobody can add to safely.
// Each bool is one command-line switch, which is what `clap` derives them from;
// collapsing them into an enum would make the CLI worse to serve the lint.
#[allow(clippy::struct_excessive_bools)]
#[derive(clap::Args, Debug)]
pub struct CheckArgs {
    /// Accepted so it can be refused by name: a gate over a subset would
    /// report a clean plan while ignoring the rest of it (`A1-214`).
    #[command(flatten)]
    select: SelectArgs,
    /// Output format: human, json, agent, github, sarif.
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
    /// Fail on warnings too, so every finding blocks. Overrides `deny_warnings`
    /// in `.aplan.yml`.
    #[arg(long)]
    deny_warnings: bool,
    /// Do not fail on warnings, whatever `.aplan.yml` says.
    #[arg(long, conflicts_with = "deny_warnings")]
    no_deny_warnings: bool,
}

/// Everything `aplan verify` was asked to do.
///
/// It carries `check`'s hermetic flags as well as its own, because it runs those
/// rules (`A1-058`). Without them the two commands could still be made to
/// disagree about one plan: `check --no-traces` passing while `verify` failed on
/// the trace it was told to ignore is the same false-reassurance defect wearing
/// a flag.
// Each bool is one command-line switch, which is what `clap` derives them from.
#[allow(clippy::struct_excessive_bools)]
#[derive(clap::Args, Debug)]
pub struct VerifyArgs {
    /// Accepted so it can be refused by name: a gate over a subset would
    /// report a clean plan while ignoring the rest of it (`A1-214`).
    #[command(flatten)]
    select: SelectArgs,
    /// Output format: human, json, agent, github, sarif.
    #[arg(long, default_value = "human")]
    format: String,
    /// Repository to resolve evidence against (default: the current directory).
    #[arg(long, default_value = ".")]
    repo: PathBuf,
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
    /// Run `test` evidence (rung L3), not just resolve it. Off by default; CI
    /// must not enable it for pull requests from forks (see SECURITY.md).
    ///
    /// A pass here is your `test.run` command exiting `0`, which is worth what
    /// your runner makes it worth: one that selects by a filter and exits `0`
    /// when the filter matches nothing would report every cited test as passing.
    /// You no longer have to check that by hand -- this runs your `run` command
    /// once against a name no test can have, and `APLAN037` reports any runner
    /// whose passes are the same bytes as that miss.
    #[arg(long)]
    execute: bool,
    /// Fail on warnings too, so every finding blocks. Overrides `deny_warnings`
    /// in `.aplan.yml`.
    #[arg(long)]
    deny_warnings: bool,
    /// Do not fail on warnings, whatever `.aplan.yml` says.
    #[arg(long, conflicts_with = "deny_warnings")]
    no_deny_warnings: bool,
}

/// Everything `aplan add` was asked to do.
///
/// The flag set is what the first pilot retyped into every one of its
/// thirty-three hand-written files, and nothing else: a task nobody has thought
/// about yet has no estimate, no owner, and no paths, and inventing them would
/// make the plan claim things nobody decided.
#[derive(clap::Args, Debug)]
#[command(group = clap::ArgGroup::new("what_to_add").required(true).args(["title", "from"]))]
pub struct AddArgs {
    /// Title of the new task, as a person would read it.
    title: Option<String>,
    // `A1-241`. Two pilots arrived with a written plan and no way to get it in,
    // and one of them wrote a generator that emits frontmatter directly, which
    // is a second implementation of the file format gated by nothing. An entry
    // carries what this command already takes, so this is the same command
    // applied to a list rather than a new way to describe a task.
    /// Add every task in this file as one change, instead of one task from the
    /// arguments. The file is a YAML list whose entries carry the fields above:
    /// `title` and any of `id`, `phase`, `cut`, `effort`, `owner`, `tags`,
    /// `paths`, `traces`, `blocked_by`. Nothing is written unless every task and
    /// every cut it joins can be.
    ///
    /// **Order is for reading, not for resolving.** An entry may wait on, or
    /// join, any id the file declares -- above it or below it -- so a milestone
    /// can be written above the work it lists, the way a roadmap reads. Ids the
    /// file does not declare are minted in order, and one minted for a later
    /// entry has no name to be referred to by.
    ///
    /// An entry may also carry `body`: the Markdown after the frontmatter,
    /// written verbatim. Leave it out to keep the scaffold, which is what an
    /// author who has not written the body yet wants; give it to author the
    /// reason and the acceptance criteria in the same pass, which is what makes
    /// the tasks closeable without a second edit through the file format. It
    /// replaces the scaffold whole rather than merging into it, an empty string
    /// is refused (leave the key out instead), and `null` means the same as
    /// leaving it out.
    #[arg(long, value_name = "FILE")]
    from: Option<PathBuf>,
    /// Task id. Default: the next free id in the convention the plan already
    /// uses, keeping its prefix and its digit width and incrementing the
    /// number. Under `id_strategy: ulid` it is that same prefix followed by a
    /// ULID, which no counter derives and so no two authors can both derive.
    #[arg(long)]
    id: Option<String>,
    // `A1-108`: the default follows the placement, and the number stays in this
    // comment because help is read beside a consumer's own plan, where it does
    // not resolve (`A1-044`).
    /// `phase` for the new task. Default: the `phase` of the `--cut` it joins, or
    /// the highest phase already in use when there is no cut.
    #[arg(long)]
    phase: Option<u32>,
    /// A task this one is blocked by. Repeatable. Refused if no such task
    /// exists, because `APLAN003` would reject the file anyway.
    #[arg(long = "blocked-by")]
    blocked_by: Vec<String>,
    /// A tag. Repeatable.
    #[arg(long = "tag")]
    tags: Vec<String>,
    /// Estimate in days (`APLAN009` warns without one). Omitted by default: an
    /// invented estimate is worse than a visible gap.
    #[arg(long)]
    effort: Option<f64>,
    /// Who owns the task.
    #[arg(long)]
    owner: Option<String>,
    /// A glob naming files this task is responsible for. Repeatable. This is
    /// what lets `verify` bind a commit to the task (`APLAN012`, `APLAN023`).
    #[arg(long = "paths")]
    paths: Vec<String>,
    // `A1-116`. Refused through the same post-state check every mutation runs
    // (`A1-113`), so the refusal is `APLAN008` itself, with its own remedy,
    // rather than a copy of it here.
    /// A document section this task answers to, as `path/to/doc.md#anchor`.
    /// Repeatable. Refused if the anchor does not resolve, because `APLAN008`
    /// would reject the file anyway. A trace names the *cause* of the task, which
    /// exists, not the document the task will write.
    #[arg(long = "traces")]
    traces: Vec<String>,
    /// Milestone (cut) to add the new task to, by its id or by the label it
    /// reports itself under (`M-s1` or `s1`). Required in practice
    /// when `require_cut_membership` is on, and refused for a settled cut. Its
    /// `phase` is the new task's, unless `--phase` says otherwise.
    #[arg(long)]
    cut: Option<String>,
    /// Directory to write the task file into (default: `<root>/tasks` when it
    /// exists, else the plan root).
    #[arg(long)]
    out: Option<PathBuf>,
    /// Print what would be written, and write nothing.
    #[arg(long)]
    dry_run: bool,
    /// Whether a warning blocks this write, as `check` and `verify` take it.
    #[command(flatten)]
    verdict: VerdictFlags,
}

/// Everything `aplan export` was asked to do, as one `clap` group, for the
/// reason [`ImportArgs`] is one: the dispatch stays a single line and the flags
/// live beside the command that owns them.
#[derive(clap::Args, Debug)]
pub struct ExportArgs {
    /// Shape to write: `checklist`, a markdown roadmap of `- [ ]` items under
    /// `##` headings, where a heading is a cut.
    ///
    /// The other shapes `import` reads are refused, each naming the one field a
    /// generated document would have had to invent.
    #[arg(long, default_value = "checklist")]
    format: String,
    /// Write to this file instead of standard output.
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// Compare this file against what would be generated and exit 1 if they
    /// differ. Never writes.
    #[arg(long, value_name = "FILE", conflicts_with = "out")]
    check: Option<PathBuf>,
}

/// Everything `aplan import` was asked to do, as one `clap` group so the
/// dispatch stays a single line and the flags live beside the command that owns
/// them.
#[derive(clap::Args, Debug)]
pub struct ImportArgs {
    /// Source document to read (for `--format kep`, a `kep.yaml`).
    source: PathBuf,
    /// Source shape to expect: `kep` (a Kubernetes `kep.yaml`), `checklist` (a
    /// markdown roadmap of `- [ ]` items under `##` headings), `roadmap-json` (a
    /// hand-maintained status contract with a `depends_on` graph), `spec-kit` (a
    /// `specs/NNN/tasks.md` of `- [ ] T001 ...` items), or `openspec` (a
    /// `changes/<name>/tasks.md` of `- [ ] 1.1 ...` items).
    ///
    /// The two task-list flavors keep the ids their document wrote, qualified
    /// with `--id-prefix`, and report the ordering prose they will not turn into
    /// dependency edges.
    #[arg(long, default_value = "kep")]
    format: String,
    /// Directory to write task files into (default: `<root>/tasks` when it
    /// exists, else the plan root).
    ///
    /// It may point outside the plan, which is how you import a copy to compare
    /// against the plan you already have: the id-collision check then has no
    /// plan to collide with, and the import says so. `--cut` is refused with an
    /// outside destination, because the cut would name tasks the plan cannot
    /// see.
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
    /// Milestone (cut) to add the imported ids to, by id or by the label it
    /// reports itself under, so they are not left in no
    /// cut. Refuses a cut that is already `done` or `dropped`.
    #[arg(long)]
    cut: Option<String>,
    /// Prefix for the ids a checklist cannot supply, e.g. `AL1` giving
    /// `AL1-001`. Default `T`. Only meaningful with `--format checklist`.
    #[arg(long)]
    id_prefix: Option<String>,
}

/// Turn the validated flags into the importer's options.
///
/// Split out so the invocation is checked in one place, before anything is
/// read or written: a bad `--as-of` is a usage error, not a task file that
/// fails `APLAN010` after it lands.
fn import_options(args: &ImportArgs) -> Result<aplan_interop::Options> {
    use aplan_interop::ImplementedAs;

    // The one table, not a second list of the same five names: a flavor added to
    // `aplan_interop::FLAVORS` is readable here without an edit, and one removed
    // stops being accepted (`A1-189`).
    if !aplan_interop::FLAVORS.iter().any(|f| f.name == args.format) {
        anyhow::bail!(
            "unknown --format `{}` (this build reads: {})",
            args.format,
            aplan_interop::flavor_names().join(", ")
        );
    }
    // `spec-kit` and `openspec` documents carry ids of their own, and the prefix
    // qualifies them rather than replacing them: two features in one repository
    // both write `T001` (`A1-039`).
    if args.id_prefix.is_some()
        && !matches!(args.format.as_str(), "checklist" | "spec-kit" | "openspec")
    {
        anyhow::bail!(
            "`--id-prefix` applies to `--format checklist`, whose source carries no ids, and to \
             `--format spec-kit`/`openspec`, whose ids it qualifies"
        );
    }
    if args.as_of.is_some() && args.implemented_as != "done" {
        anyhow::bail!("`--as-of` only applies with `--implemented-as done`");
    }

    // A document that carries its own ids keeps them when nothing was asked for:
    // the default prefix is for the flavor whose source has no ids, and applying
    // it there produced `T-T001`, a wart an adopter meets on the first line of
    // output (`A1-039`). Empty means "the document's own id", and a second
    // feature's import then collides loudly rather than silently renumbering.
    let carries_ids = matches!(args.format.as_str(), "spec-kit" | "openspec");
    let id_prefix = args.id_prefix.clone().unwrap_or_else(|| {
        if carries_ids {
            String::new()
        } else {
            "T".to_string()
        }
    });
    if !(carries_ids && id_prefix.is_empty())
        && (id_prefix.trim().is_empty()
            || !id_prefix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-'))
    {
        anyhow::bail!(
            "`--id-prefix {id_prefix}` must be alphanumeric: it becomes part of a task id, and so a file name"
        );
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
        id_prefix,
    })
}

/// Read one proposal document into tasks, report what could not be carried, and
/// write the tasks unless this is a dry run (ADR-0012).
/// The consequences a reader should hear once per run rather than once per task.
///
/// Both are warnings rather than errors, and both name the fix: an import that
/// stayed quiet about them would leave the user to discover its own side effects
/// from a later `check`.
/// How many of these tasks carry a control character in a field a person reads.
///
/// Walked over `title`, `owner`, `tags` and `paths` rather than the title alone,
/// and that width is **required rather than defensive** (`A1-290`, audit).
///
/// The first version of this said `import` could only carry one in a title,
/// because every importer writes `owner: None`. `kep` does not: it takes `owner`
/// from the document's `authors` and builds tags from its `status`, `stage` and
/// `owning-sig`, lowercased and otherwise unchanged. So a `kep.yaml` whose
/// author line was pasted from a terminal puts one straight into `owner`, and
/// the walk that was written as future-proofing is what catches it.
///
/// The rest do carry it in the title alone: `checklist` slugifies its section
/// tag to alphanumerics, and `roadmap-json`, `spec-kit` and `openspec` write
/// `owner: None` and derive no tag from source text. `add --from` carries every
/// field, because a manifest supplies them all.
fn steering_fields(tasks: &[aplan_core::task::Task]) -> usize {
    tasks
        .iter()
        .flat_map(|t| {
            std::iter::once(t.title.clone())
                .chain(t.owner.clone())
                .chain(t.tags.iter().cloned())
                .chain(t.paths.iter().cloned())
        })
        .filter(|text| aplan_output::sanitize::visible(text) != *text)
        .count()
}

fn push_run_notices(
    report: &mut aplan_interop::Report,
    source_name: &str,
    placed_in_cut: bool,
    plan_has_a_cut: bool,
    steering: usize,
) {
    use aplan_interop::report::{Notice, NoticeKind};

    // **The one path by which a control character reaches a plan by accident**
    // (`A1-290`). A source document holding a terminal capture imports cleanly:
    // the character is written back as YAML's escaped form, `check` is silent
    // -- correctly, since nothing about the plan is invalid -- and the author
    // never learns. `A1-289` defends the *reader* of that plan; this tells the
    // one person who can fix it, at the moment they can.
    if steering > 0 {
        report.push(Notice::new(
            source_name,
            NoticeKind::Provenance,
            format!(
                "{steering} imported field(s) carry a control character, which the source \
                 document most likely picked up from a terminal capture. They are written \
                 back as an escape (`\\e`) and are shown as `{}` rather than run, so nothing \
                 here steers a terminal; edit the task if it was not meant",
                aplan_output::sanitize::MARKER
            ),
        ));
    }

    if !placed_in_cut {
        // Naming `--cut <milestone>` to a plan that has no milestone is the
        // defect `A1-252` fixed one surface over, and this is the surface an
        // adopter with an existing checklist meets first (`A1-252`, audit).
        report.push(Notice::new(
            source_name,
            NoticeKind::Placement,
            if plan_has_a_cut {
                "imported tasks belong to no cut; where `require_cut_membership` is set, \
                 `check` reports APLAN022 until `--cut <milestone>` or a `cross-cutting` tag \
                 places them"
            } else {
                "imported tasks belong to no cut, and this plan has none yet; where \
                 `require_cut_membership` is set, `check` reports APLAN022 until a cut exists \
                 to place them in. A cut is an ordinary task tagged `milestone` whose \
                 `blocked_by` lists its members"
            },
        ));
    }
    report.push(Notice::new(
        source_name,
        NoticeKind::Placement,
        "imported tasks carry no `effort_days`, because there is nothing to derive one from; \
         `check` warns (APLAN009) until you estimate them",
    ));
}

/// How this repository mints ids, from its config (`A1-047`).
///
/// Read here rather than threaded from the caller because both minting commands
/// (`add` and `supersede`) need it and neither had the config in hand. A config
/// that fails to load is an error rather than a silent `sequential`: the whole
/// point of the key is that a repository chose something, and defaulting past a
/// broken file would mint the ids the choice exists to prevent.
fn id_strategy(root: &std::path::Path) -> Result<aplan_core::IdStrategy> {
    let (config, _) = config_at(root)?;
    Ok(config.id_strategy.unwrap_or_default())
}

/// `aplan add`, scaffold one task file (`A1-029`).
///
/// The command exists because every adopter starts by hand-authoring YAML, which
/// is friction that arrives *before* any of the tool's value does. What it must
/// not become is a generator of plausible-looking plans: it writes the fields a
/// plan cannot do without, refuses what `check` would reject anyway, and leaves
/// every judgement (the estimate, the paths, why the task exists) to a person.
/// What `add` prints for one task it wrote: a row of the plan, in the same roles
/// every other row-printing surface uses (`A1-288`).
///
/// A named function because `add` and `add --from` print the identical shape and
/// `cmd_add` is at its line ceiling, which is what asked the question.
fn wrote_row(dry_run: bool, id: &str, title: &str, path: &std::path::Path) -> String {
    format!(
        "{} {i}{id}{i:#}  {st}todo{st:#}  {title}\n  {dim}-> {}{dim:#}",
        if dry_run { "would write" } else { "wrote" },
        path.display(),
        i = aplan_output::style::ID,
        st = aplan_output::style::status("todo"),
        dim = aplan_output::style::DIM
    )
}

fn cmd_add(root: &std::path::Path, args: &AddArgs) -> Result<u8> {
    use aplan_interop::{cut, emit, ids};

    if let Some(from) = &args.from {
        return cmd_add_from(root, args, from);
    }
    // clap requires one of the two, so this is the shape of the invocation
    // rather than a value to defend against.
    let title = args
        .title
        .as_deref()
        .expect("clap requires a title when `--from` is absent");
    let plan = aplan_core::load_plan(root);
    // A `--cut` may be the milestone id or the label the cut reports itself
    // under, resolved once here so everything downstream works in ids
    // (`A1-139`). `roadmap` prints the label, so refusing it was the tool asking
    // a reader to translate between two names for one thing.
    let cut = match &args.cut {
        Some(name) => Some(cut::resolve_id(&plan, name)?),
        None => None,
    };
    let cut = cut.as_deref();
    // The cut is the better answer than the highest phase in use, and `add` has
    // it here (`A1-108`).
    let phase = match args.phase {
        Some(p) => p,
        None => ids::phase_for(&plan, cut),
    };
    let id = match &args.id {
        Some(id) => id.clone(),
        None => ids::mint(&plan, phase, id_strategy(root)?)?,
    };

    // --- refuse before writing, not after -----------------------------------
    // Identity is the `id` field rather than the path (ADR-0010), so a collision
    // is checked here where the plan is loaded; `emit` also refuses by path, and
    // both matter.
    if let Some(existing) = plan.tasks.iter().find(|lt| lt.task.id == id) {
        anyhow::bail!(
            "task `{id}` already exists in {}; refusing to overwrite it",
            existing.path.display()
        );
    }
    // Refuse a task the loader would reject, or one nobody could identify. Both
    // of these were written happily by the first version of this command, which
    // then left the author with a file `check` fails on and no obvious cause: an
    // `--effort -1` is a schema error, and `--effort inf` was worse, because it
    // loaded cleanly and poisoned every rollup that adds the field up.
    if title.trim().is_empty() {
        anyhow::bail!("a task needs a title: it is how anyone finds this task again");
    }
    if let Some(days) = args.effort {
        if !days.is_finite() || days < 0.0 {
            anyhow::bail!(
                "`--effort {days}` is not a number of days; give a finite, non-negative estimate \
                 or omit it"
            );
        }
    }
    let unknown: Vec<&String> = args
        .blocked_by
        .iter()
        .filter(|b| !plan.tasks.iter().any(|lt| lt.task.id == **b))
        .collect();
    if !unknown.is_empty() {
        anyhow::bail!(
            "blocked by task(s) that do not exist: {}. `APLAN003` would reject the file, so \
             the id is checked before the file is written.",
            unknown
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if let Some(cut_id) = cut {
        cut::check_target(&plan, cut_id)?;
    }

    let task = aplan_core::Task {
        id: id.clone(),
        title: title.to_string(),
        phase,
        status: aplan_core::Status::Todo,
        owner: args.owner.clone(),
        effort_days: args.effort,
        blocked_by: args.blocked_by.clone(),
        prefer_after: Vec::new(),
        evidence: Vec::new(),
        // Repeats removed on both surfaces, in one place (`A1-116`).
        paths: emit::deduped(&args.paths),
        traces: emit::deduped(&args.traces),
        trace_fingerprints: std::collections::BTreeMap::new(),
        tags: emit::deduped(&args.tags),
        revisit_at: None,
        version_impact: None,
        version_impact_reason: None,
    };

    // The whole post-state, through the one shared path (`A1-113`). The
    // milestone edit `--cut` performs is part of it, so a placement that would
    // break the plan is refused here rather than reported on the next `check`.
    let after = post_state_for_add(&plan, &task, cut);
    refuse_bad_mutation(root, &format!("add `{id}`"), &plan, &after, args.verdict)?;

    let out_dir = args.out.clone().unwrap_or_else(|| successor_dir(root));
    let path = emit::task_path(&out_dir, &id);

    let mut out = String::new();
    let _ = writeln!(out, "{}", wrote_row(args.dry_run, &id, &task.title, &path));

    if args.dry_run {
        if let Some(cut_id) = cut {
            let _ = writeln!(out, "  would add {id} to `{cut_id}`");
        }
    } else {
        // Created *and* placed, or nothing (`A1-103`).
        let placed = emit::write_task_placed(&out_dir, root, &task, emit::ADD_BODY, cut)?;
        if let (Some(cut_id), Some(path)) = (cut, placed) {
            let _ = writeln!(out, "  added {id} to `{cut_id}` in {}", path.display());
        }
    }

    out.push_str(&add_notes(args));
    write_stdout(&out)?;
    Ok(EXIT_OK)
}

/// `aplan add --from`, a written plan turned into tasks as one change
/// (`A1-241`).
///
/// **Why this is `add` and not an importer.** ADR-0012's honesty rules exist for
/// a format the task model cannot hold, and an entry here is the set of
/// arguments this command already takes, so there is nothing to lose in
/// translation. What it removes is the invocation count, and the thing the
/// invocation count drove an adopter to: `p00016` turned a 1,400-line document
/// into 153 tasks, 11 cuts and 3 releases with a generator that emitted
/// frontmatter directly, which makes the plan's own file format a second
/// implementation nothing gates (pre-public adoption evidence item 3). `p00002` shelled
/// out 64 times instead and every task landed, which is the same finding without
/// the damage (pre-public adoption evidence item 2).
///
/// **The whole batch, or none of it.** Every entry is resolved against the state
/// the entries before it produce, so an entry may join a cut an earlier one
/// creates and wait on a task an earlier one adds. The rules run once, on the
/// plan the whole file would produce, rather than once per entry against a plan
/// that is half written -- a hundred and fifty three separate gates would each
/// judge a plan no author ever intends to have. The write is one batch through
/// `save::replace_all`, so a failure anywhere writes nothing at all.
fn cmd_add_from(root: &std::path::Path, args: &AddArgs, from: &std::path::Path) -> Result<u8> {
    use aplan_interop::{cut, emit, manifest};

    let entries = manifest::read(from)?;
    let plan = aplan_core::load_plan(root);
    let out_dir = args.out.clone().unwrap_or_else(|| successor_dir(root));
    let Planned {
        after,
        created,
        joined,
        bodies,
    } = plan_the_batch(root, &plan, &entries, &out_dir)?;

    // One gate, on the plan the whole file produces (`A1-113`).
    refuse_bad_mutation(
        root,
        &format!("add {} task(s) from {}", created.len(), from.display()),
        &plan,
        &after,
        args.verdict,
    )?;

    let mut out = String::new();
    let mut batch: Vec<aplan_core::save::Replacement> = Vec::new();
    for id in &created {
        let lt = after
            .tasks
            .iter()
            .find(|lt| lt.task.id == *id)
            .expect("just added");
        // From `after`, so a cut created here carries the members later entries
        // gave it. Its own file is the only place those can be written.
        batch.push(aplan_core::save::Replacement {
            path: lt.path.clone(),
            expected: String::new(),
            updated: emit::task_file(
                &lt.task,
                bodies.get(id).map_or(emit::ADD_BODY, String::as_str),
            )?,
        });
        let _ = writeln!(
            out,
            "{}",
            wrote_row(args.dry_run, id, &lt.task.title, &lt.path)
        );
    }
    // What the reader cannot otherwise see. A manifest is generated, so a
    // template that emitted `body` for some entries and not others produces a
    // run that looks identical to one that worked -- and the difference only
    // surfaces at closure, when `APLAN038` reports every task that kept the
    // scaffold. That is `p0000014`'s own finding arriving one level up, so the
    // count is stated rather than left to be discovered (`A1-260`, audit).
    //
    // Only when a body was supplied at all: a manifest that carries none is the
    // shape every manifest had before this, and a line about it would be noise
    // on every existing run.
    if !bodies.is_empty() {
        let with = bodies.len();
        let total = created.len();
        let _ = if with == total {
            writeln!(out, "  all {total} carry a body")
        } else {
            writeln!(
                out,
                "  {with} of {total} carry a body; the rest keep the scaffold"
            )
        };
    }
    // **The second path a control character reaches a plan without being typed**
    // (`A1-290`). A manifest is generated, and a generator reading a terminal
    // capture puts one in whatever field it captured -- which here is any of
    // them, not the title alone as `import` is limited to. The character is
    // written back as YAML's escaped form and `check` is silent, correctly,
    // because nothing about the plan is invalid.
    let made: Vec<aplan_core::task::Task> = created
        .iter()
        .filter_map(|id| after.tasks.iter().find(|lt| lt.task.id == *id))
        .map(|lt| lt.task.clone())
        .collect();
    let steering = steering_fields(&made);
    if steering > 0 {
        let _ = writeln!(
            out,
            "  {steering} field(s) carry a control character, most likely from a terminal \
             capture the manifest was generated from; they are written back as an escape \
             and shown as `{}` rather than run",
            aplan_output::sanitize::MARKER
        );
    }
    // A cut the plan already had is edited in place; one this file creates was
    // written with its members above.
    for (cut_id, ids) in &joined {
        if !plan.tasks.iter().any(|lt| lt.task.id == *cut_id) {
            let _ = writeln!(
                out,
                "  `{cut_id}` {} {} of them, in its own file",
                if args.dry_run { "would hold" } else { "holds" },
                ids.len()
            );
            continue;
        }
        let pending = cut::plan_placement(&plan, cut_id, ids)?;
        if let Some(replacement) = pending.replacement() {
            let _ = writeln!(
                out,
                "  {} {} task(s) to `{cut_id}` in {}",
                if args.dry_run { "would add" } else { "added" },
                pending.added.len(),
                replacement.path.display()
            );
            batch.push(replacement);
        }
    }

    if !args.dry_run {
        save_batch(&batch, "add --from")?;
    }
    write_stdout(&out)?;
    Ok(EXIT_OK)
}

/// The plan a manifest would produce, and what it took to get there.
struct Planned {
    /// The whole post-state, for the one gate run over it.
    after: aplan_core::load::Plan,
    /// Ids created, in the order the file names them.
    created: Vec<String>,
    /// Which cut gained which ids.
    joined: std::collections::BTreeMap<String, Vec<String>>,
    /// The body each entry supplied, by id. Absent means the scaffold.
    ///
    /// By id rather than by position, though the two agree today: `created` and
    /// `entries` are parallel only for as long as every entry makes exactly one
    /// task, and a body written into the wrong file is the kind of mistake that
    /// looks like a plan rather than a bug (`A1-260`).
    bodies: std::collections::BTreeMap<String, String>,
}

/// Resolve every entry against the state the entries before it produce.
///
/// The ordering is the feature: an entry may join a cut an earlier one declares,
/// wait on a task an earlier one adds, and take an id minted from the convention
/// the earlier ones establish. Expressed as N invocations, an author has to work
/// all three out in advance, which is what drove a pilot to write frontmatter
/// directly (`A1-241`).
fn plan_the_batch(
    root: &std::path::Path,
    plan: &aplan_core::load::Plan,
    entries: &[aplan_interop::manifest::Entry],
    out_dir: &std::path::Path,
) -> Result<Planned> {
    use aplan_interop::{emit, ids};

    let strategy = id_strategy(root)?;
    let mut after = aplan_core::mutation::clone_of(plan);
    let mut created: Vec<String> = Vec::new();
    let mut joined: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut bodies: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    // **The ids this file declares, so an entry may name one a later entry
    // creates.** A manifest is applied as one change and validated as one plan,
    // so there is nothing an order buys here -- and the two spellings of
    // membership pulled in opposite directions: `blocked_by` on the milestone
    // wanted its members first, `cut:` on a task wanted the milestone first, and
    // a file cannot do both (`A1-293`).
    //
    // Explicit ids only. An entry with no `id` gets one minted from what came
    // before, so it has no name to be referred to by until it exists.
    let declared: std::collections::BTreeMap<&str, &aplan_interop::manifest::Entry> = entries
        .iter()
        .filter_map(|e| e.id.as_deref().map(|id| (id, e)))
        .collect();

    for (n, entry) in entries.iter().enumerate() {
        // Named by position *and* title: a manifest is generated, so "entry 87"
        // alone sends the author counting through a file they did not type.
        let at = |what: String| anyhow::anyhow!("entry {} (`{}`): {what}", n + 1, entry.title);
        if entry.title.trim().is_empty() {
            return Err(at("a task needs a title".to_string()));
        }
        if let Some(days) = entry.effort
            && (!days.is_finite() || days < 0.0)
        {
            return Err(at(format!(
                "`effort: {days}` is not a number of days; give a finite, non-negative estimate"
            )));
        }
        let cut = entry_cut(entry, &after, &declared).map_err(at)?;
        let phase = entry
            .phase
            .unwrap_or_else(|| ids::phase_for(&after, cut.as_deref()));
        let id = match &entry.id {
            Some(id) => id.clone(),
            None => ids::mint(&after, phase, strategy).map_err(|e| at(e.to_string()))?,
        };
        // The same containment check `emit::write_task` makes, and the reason it
        // is shared rather than repeated: this path writes the file itself, so
        // without it a manifest id of `../../elsewhere/x` wrote outside the plan
        // directory entirely (`A1-241`, audit).
        if !emit::is_usable_id(&id) {
            return Err(at(format!(
                "`{id}` cannot name a file: an id may not be empty, be `.` or `..`, contain \
                 `/` or `\\`, or begin or end with whitespace"
            )));
        }
        if let Some(existing) = after.tasks.iter().find(|lt| lt.task.id == id) {
            return Err(at(format!(
                "task `{id}` already exists in {}",
                existing.path.display()
            )));
        }
        let unknown: Vec<&str> = entry
            .blocked_by
            .iter()
            .filter(|b| {
                !after.tasks.iter().any(|lt| lt.task.id == **b)
                    && !declared.contains_key(b.as_str())
            })
            .map(String::as_str)
            .collect();
        if !unknown.is_empty() {
            return Err(at(format!(
                "blocked by task(s) the plan does not have and this file does not declare: {}",
                unknown.join(", ")
            )));
        }

        let task = aplan_core::Task {
            id: id.clone(),
            title: entry.title.clone(),
            phase,
            status: aplan_core::Status::Todo,
            owner: entry.owner.clone(),
            effort_days: entry.effort,
            blocked_by: entry.blocked_by.clone(),
            prefer_after: Vec::new(),
            evidence: Vec::new(),
            paths: emit::deduped(&entry.paths),
            traces: emit::deduped(&entry.traces),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: emit::deduped(&entry.tags),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
        };
        if let Some(body) = &entry.body {
            // An empty body is refused rather than written. A manifest is
            // generated, so `body: ""` is a lookup that came back empty far more
            // often than it is an author asking for a task with no prose -- and
            // the way to ask for that is to leave the key out, which says the
            // same thing and keeps the scaffold to write into (`A1-260`).
            if body.trim().is_empty() {
                return Err(at(
                    "`body` is empty. Leave the key out to keep the scaffold `aplan add` \
                     writes, or give the Markdown this task's file should carry"
                        .to_string(),
                ));
            }
            bodies.insert(id.clone(), body.clone());
        }
        after = aplan_core::mutation::with_added(&after, emit::task_path(out_dir, &id), task);
        if let Some(cut_id) = cut {
            joined.entry(cut_id).or_default().push(id.clone());
        }
        created.push(id);
    }

    write_membership(&mut after, &joined);

    Ok(Planned {
        after,
        created,
        joined,
        bodies,
    })
}

/// Write each cut's new members into its milestone.
///
/// **After the entry loop, not inside it.** A cut an entry names may be declared
/// further down, so the milestone to write the edge into does not exist yet
/// while that entry is being read -- and the in-loop version simply found no
/// slot and dropped the membership without a word (`A1-293`).
fn write_membership(
    after: &mut aplan_core::load::Plan,
    joined: &std::collections::BTreeMap<String, Vec<String>>,
) {
    for (cut_id, members) in joined {
        if let Some(slot) = after.tasks.iter_mut().find(|lt| lt.task.id == *cut_id) {
            slot.task.blocked_by.extend(members.iter().cloned());
        }
    }
}

/// The cut one manifest entry joins, or `None`.
///
/// Resolved against the plan the entries so far would produce *and* against the
/// ids the file itself declares, so a milestone may be written above the work
/// that names it (`A1-293`). **By id, never by label**, when it is declared
/// later: a cut's label is derived from the milestone the plan holds, and one
/// the file has not written yet has nothing to derive it from.
fn entry_cut(
    entry: &aplan_interop::manifest::Entry,
    after: &aplan_core::load::Plan,
    declared: &std::collections::BTreeMap<&str, &aplan_interop::manifest::Entry>,
) -> std::result::Result<Option<String>, String> {
    use aplan_interop::cut;

    let Some(name) = &entry.cut else {
        return Ok(None);
    };
    let id = match cut::resolve_id(after, name) {
        Ok(id) => {
            // Only a cut that already exists is asked whether it can take work.
            // One this file declares later is a new entry, so it is `todo` and
            // tagged `milestone` by construction: there is no settled cut and no
            // non-milestone target to refuse.
            cut::check_target(after, &id).map_err(|e| e.to_string())?;
            id
        }
        // **Only when the plan has no such cut.** An ambiguous label, or a name
        // that is one cut's id and another's label, is a refusal this tool added
        // on purpose: taking one silently sends a reader who typed what `roadmap`
        // printed into a cut they did not mean. Reading past it to a milestone
        // this file declares would answer that with a *third* cut, which is the
        // same defect one layer down (`A1-293`, audit).
        Err(e @ cut::CutError::NoSuchCut { .. }) => match declared.get(name.as_str()) {
            Some(later)
                if later
                    .tags
                    .iter()
                    .any(|t| t == aplan_core::task::MILESTONE_TAG) =>
            {
                name.clone()
            }
            // Declared here, and not a cut. Saying "no cut `X` in this plan" of
            // an id the reader can see in front of them is the message that
            // sends them back to the bootstrap deadlock.
            other => {
                return Err(manifest_cut_refusal(
                    &e.to_string(),
                    name,
                    other.is_some(),
                    declared,
                ));
            }
        },
        Err(e) => return Err(e.to_string()),
    };
    Ok(Some(id))
}

/// The cut refusal, plus what the file itself declares.
///
/// A manifest may name a cut it creates further down, and it names it by `id`:
/// a cut's *label* is derived from the milestone the plan holds, and one this
/// file has not written yet has nothing to derive it from. So a reader who typed
/// the label gets the ids they could have typed instead, and one who named an
/// entry that is not a milestone is told which of the two it is (`A1-293`).
///
/// The upstream refusal ends in a full stop, so it is trimmed before this
/// continues the sentence: pasting the two together gave `no cuts.. This file`.
fn manifest_cut_refusal(
    refusal: &str,
    name: &str,
    declared_here: bool,
    declared: &std::collections::BTreeMap<&str, &aplan_interop::manifest::Entry>,
) -> String {
    let head = refusal.trim_end().trim_end_matches('.');
    if declared_here {
        return format!(
            "{head}. This file declares `{name}`, and it carries no `{tag}` tag, so it is a \
             task rather than a cut",
            tag = aplan_core::task::MILESTONE_TAG
        );
    }
    let cuts: Vec<&str> = declared
        .iter()
        .filter(|(_, e)| e.tags.iter().any(|t| t == aplan_core::task::MILESTONE_TAG))
        .map(|(id, _)| *id)
        .collect();
    if cuts.is_empty() {
        return format!("{head}, and this file declares none either");
    }
    format!(
        "{head}. This file declares the cut(s) {}, and a cut it declares is named by its \
         `id` rather than by the label it will report itself under; `{name}` is neither",
        cuts.join(", ")
    )
}

/// What `why` says about a task nothing is blocking.
///
/// "ready" is what the `ready` command offers, and it offers neither a checkpoint
/// nor a task that is over. Using the word for anything else makes this command
/// contradict the one that defines it, which is the reasoning the parked branch
/// already used for a third case (`A1-105`).
fn unblocked_verdict(task: &aplan_core::Task, closeable: bool) -> String {
    // An id carries the id role wherever the tool prints one, which is the rule
    // `A1-288`'s census made necessary to state. These four sentences were the
    // ones that did not (`A1-296`, audit). The `aplan close` in the second is an
    // id in a command a reader is meant to copy, so it takes it too.
    let styled = |id: &str| format!("{i}{id}{i:#}", i = aplan_output::style::ID);
    let id = styled(&task.id);
    let id = id.as_str();
    if task.status.is_terminal() {
        format!("{id} is `{}`: there is nothing to do\n", task.status)
    } else if closeable {
        format!(
            "{id} is a cut and its members are all done: `aplan close {id}` opens what \
             follows it\n"
        )
    } else if task.is_milestone() {
        format!("{id} is a cut: nothing blocks it\n")
    } else {
        format!("{id} is ready: nothing blocks it\n")
    }
}

/// What to say when `ready` has nothing to offer.
///
/// An empty list answers two different questions, and this is the command an
/// adopter opens every session with. Saying which one it is costs a line and
/// saves the reader working out that milestones are excluded from `ready`, that a
/// cut is closed like any other task, and that closing it is what opens the next
/// one (`A1-105`).
fn why_nothing_is_ready(graph: &Graph<'_>) -> String {
    let closeable = graph.closeable_milestones();
    match closeable.as_slice() {
        [] => "nothing is ready, and no cut is waiting to be closed.\n".to_string(),
        [m] => format!(
            "nothing is ready. `{}`'s members are all done: `aplan close {}` opens what \
             follows it.\n",
            m.id, m.id
        ),
        many => format!(
            "nothing is ready. {} cuts have all their members done: {}. Close one to open \
             what follows it.\n",
            many.len(),
            many.iter()
                .map(|m| format!("`{}`", m.id))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// What the new task is missing, said rather than filled in.
///
/// Each of these is a judgement only the author can make, and each is silent by
/// default in a way that has already cost this project: a task with no `paths`
/// degrades its own future evidence (`APLAN023`), and a plan with
/// `require_cut_membership` on rejects a task in no cut, which the author would
/// otherwise meet on their next `check` instead of here, where it is one flag away.
fn add_notes(args: &AddArgs) -> String {
    // The `effort_days`, `paths` and cut-membership notes lived here, each a
    // partial answer to "what will `check` say about this". `A1-113` made the
    // whole answer available by running the rules on the post-state, so keeping
    // these would say everything twice, which is the redundancy `A1-106` removed
    // elsewhere. `APLAN023` is the one the delta cannot reach: it judges a *done*
    // task's evidence, and this task is `todo`, so an author who will cite a
    // commit later still needs telling now.
    let mut out = String::new();
    if args.paths.is_empty() {
        let _ = writeln!(
            out,
            "note: no `paths`; add them, or a commit cited later proves only that it exists \
             (APLAN023)"
        );
    }
    out
}

/// The body a scaffolded task starts with.
/// Refuse a batch that would leave a half-written import.
///
/// Every check runs before the first file is written, because an import that
/// stops in the middle leaves the author with files to delete as well as a
/// mistake to find. Both of these were found by handing a source its own bad
/// data: a `roadmap.json` declaring one id twice wrote the first task and failed
/// on the second with a message that read as though the file had been there all
/// along, and a source with an empty title wrote a task `check` rejected on sight
/// (`APLAN001`, since a title is `minLength: 1`).
fn check_importable(tasks: &[aplan_core::Task]) -> Result<()> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for task in tasks {
        if task.title.trim().is_empty() {
            anyhow::bail!(
                "`{}` has no title in the source; a task nobody can name is one nobody can \
                 find, and `check` would reject the file (APLAN001)",
                task.id
            );
        }
        if !seen.insert(task.id.as_str()) {
            anyhow::bail!(
                "the source declares `{}` twice; ids are identities, so importing both would \
                 mean one overwriting the other",
                task.id
            );
        }
    }
    Ok(())
}

/// One `ProposalSource` per flavor, chosen here and nowhere else: everything
/// downstream reads `imported`, so adding a flavor is an arm, not a branch
/// through the write path (ADR-0012).
fn read_source(
    format: &str,
    source_name: &str,
    contents: &str,
    options: &aplan_interop::Options,
) -> Result<aplan_interop::Import> {
    use aplan_interop::{
        ProposalSource,
        checklist::Checklist,
        kep::Kep,
        roadmap_json::RoadmapJson,
        spec_kit::{OpenSpec, SpecKit},
    };
    Ok(match format {
        "checklist" => Checklist.parse(source_name, contents, options)?,
        "roadmap-json" => RoadmapJson.parse(source_name, contents, options)?,
        "spec-kit" => SpecKit.parse(source_name, contents, options)?,
        "openspec" => OpenSpec.parse(source_name, contents, options)?,
        _ => Kep.parse(source_name, contents, options)?,
    })
}

/// Where these tasks will land, and what that means for the checks around them.
///
/// Identity is the `id` field (ADR-0010), so an import refuses to write a task
/// whose id the plan already holds -- a question that belongs to the plan the
/// files will *join*. Outside the plan root they join none, so there is nothing
/// to collide with, and the unscoped check refused the ordinary comparison
/// workflow: importing a source somewhere else to diff against the plan already
/// in the tree (`A1-163`).
fn place_import(
    root: &std::path::Path,
    out_dir: &std::path::Path,
    args: &ImportArgs,
    plan: &aplan_core::load::Plan,
    imported: &[aplan_core::task::Task],
    source_name: &str,
    report: &mut aplan_interop::report::Report,
) -> Result<()> {
    if under(root, out_dir) {
        for task in imported {
            if let Some(existing) = plan.tasks.iter().find(|lt| lt.task.id == task.id) {
                anyhow::bail!(
                    "task `{}` is already in this plan, at {}; import will not overwrite it. To \
                     import a copy for comparison, write it outside {} with `--out`",
                    task.id,
                    existing.path.display(),
                    root.display()
                );
            }
        }
        return Ok(());
    }
    // `--cut` edits a milestone *in the plan* to name these tasks, and the plan
    // cannot see them, so the cut would be left naming ids that do not resolve
    // (`APLAN003`). Refused rather than reported: it is the one combination that
    // leaves the plan worse than it started.
    if args.cut.is_some() {
        anyhow::bail!(
            "`--cut` places tasks in a milestone in the plan at {}, and `--out {}` writes them \
             outside it, so the cut would name tasks the plan cannot see. Drop one of the two",
            root.display(),
            out_dir.display()
        );
    }
    // Not a refusal: writing outside the plan is a real thing to want. It is
    // also silent about the most important fact, that nothing will check these
    // files until something points a root at them.
    report.push(aplan_interop::report::Notice::new(
        source_name,
        aplan_interop::report::NoticeKind::Placement,
        format!(
            "`--out {}` is outside the plan at {}, so these tasks join no plan and `check` will \
             not see them until you pass `--root {}`",
            out_dir.display(),
            root.display(),
            out_dir.display()
        ),
    ));
    Ok(())
}

/// Whether `path` lies inside `root`, for a destination that may not exist yet.
///
/// Symlinks are followed as far as the path exists, and only the part that does
/// not exist yet is normalised lexically. Both halves matter: `--out` names a
/// directory the import is about to create, so `canonicalize` cannot answer for
/// it, and a lexical answer alone let a symlink pointing back into the plan be
/// judged outside it -- which skipped the id check and wrote a duplicate id into
/// the plan, the one thing that check exists to prevent (`A1-163`, found by its
/// own audit).
fn under(root: &std::path::Path, path: &std::path::Path) -> bool {
    resolved(path).starts_with(resolved(root))
}

/// An absolute path with every symlink in its existing part followed.
///
/// Walks up to the longest prefix that exists, canonicalises that, and puts the
/// rest back, so a destination two directories deep in nothing still lands
/// wherever its existing ancestor really is.
fn resolved(path: &std::path::Path) -> std::path::PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let absolute = aplan_core::anchor::normalise(&absolute);

    let mut probe = absolute.as_path();
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
    loop {
        if let Ok(real) = probe.canonicalize() {
            let mut out = real;
            out.extend(tail.iter().rev());
            return out;
        }
        let (Some(parent), Some(name)) = (probe.parent(), probe.file_name()) else {
            return absolute;
        };
        tail.push(name);
        probe = parent;
    }
}

/// The "what will happen" block: one line per imported task, with the file it
/// would be written to beneath it.
///
/// Split out of `cmd_import` because folding the title made that function longer
/// than the lint allows, and a rendering is a thing on its own (`A1-206`).
fn imported_lines(tasks: &[aplan_core::task::Task], out_dir: &std::path::Path) -> String {
    let mut lines = String::new();
    for task in tasks {
        // The title is the trailing column; the path below it is one token and
        // stays whole.
        // A source document is untrusted in exactly the way a plan is, and this
        // is what `import` is for: reading somebody else's roadmap (`A1-289`,
        // audit -- it was passing an escape from the source straight through).
        let head = format!(
            "  {id}{}{id:#}  {st}{}{st:#}  [{}]  ",
            aplan_output::sanitize::visible(&task.id),
            status_word(task.status),
            aplan_output::sanitize::visible(&task.tags.join(", ")),
            id = aplan_output::style::ID,
            st = aplan_output::style::status(task.status.as_str()),
        );
        let _ = writeln!(
            lines,
            "{}\n    {dim}-> {}{dim:#}",
            aplan_output::fold::row(&head, &aplan_output::sanitize::visible(&task.title)),
            aplan_interop::emit::task_path(out_dir, &task.id).display(),
            dim = aplan_output::style::DIM
        );
    }
    lines
}

fn cmd_import(root: &std::path::Path, args: &ImportArgs) -> Result<u8> {
    use aplan_interop::{cut, emit, report};

    let options = import_options(args)?;
    let source_name = args.source.display().to_string();
    let contents =
        std::fs::read_to_string(&args.source).with_context(|| format!("reading {source_name}"))?;
    let imported = read_source(&args.format, &source_name, &contents, &options)?;

    check_importable(&imported.tasks)?;

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
    let lines = imported_lines(&imported.tasks, &out_dir);

    let plan = aplan_core::load_plan(root);
    let mut report = imported.report;
    place_import(
        root,
        &out_dir,
        args,
        &plan,
        &imported.tasks,
        &source_name,
        &mut report,
    )?;

    // The same `--cut` vocabulary the read commands take: id or label, resolved
    // once (`A1-139`).
    let cut = match &args.cut {
        Some(name) => Some(aplan_interop::cut::resolve_id(&plan, name)?),
        None => None,
    };

    // Validate the destination cut BEFORE writing anything. `cut::place` guards
    // itself, but it runs after the task files are written, so a refusal there
    // would strand freshly written tasks in no cut behind an error implying
    // nothing had happened. Checking first makes the whole import all-or-nothing,
    // and makes `--dry-run` catch a settled cut instead of promising a run that
    // would fail.
    if let Some(cut_id) = cut.as_deref() {
        cut::check_target(&plan, cut_id)?;
    }

    let ids: Vec<String> = imported.tasks.iter().map(|t| t.id.clone()).collect();

    if args.dry_run {
        if let Some(cut_id) = cut.as_deref() {
            report.push(report::Notice::new(
                &source_name,
                report::NoticeKind::Placement,
                format!("would add {} to `{cut_id}`", ids.join(", ")),
            ));
        }
    } else {
        for (task, body) in imported.tasks.iter().zip(&imported.bodies) {
            emit::write_task(&out_dir, task, body)?;
        }
        if let Some(cut_id) = cut.as_deref() {
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

    push_run_notices(
        &mut report,
        &source_name,
        args.cut.is_some(),
        plan.tasks.iter().any(|lt| lt.task.is_milestone()),
        steering_fields(&imported.tasks),
    );

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
        // Folded here rather than in the `Display` impl: `aplan-interop` does
        // not know a terminal width and a `Display` should not be choosing a
        // layout. Four, so a folded notice continues under its own text rather
        // than under the two-space bullet (`A1-206`).
        for line in report.to_string().lines() {
            outp.push_str(&aplan_output::fold::fold(
                line,
                aplan_output::fold::out_width(),
                4,
            ));
            outp.push('\n');
        }
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

/// Arguments to `aplan amend`, a struct so the dispatch arm is one line, as
/// `add` and `import` already are.
#[derive(clap::Args, Debug)]
#[command(group(
    clap::ArgGroup::new("repair")
        .required(true)
        .args(["replace_evidence", "add_evidence", "remove_evidence"])
))]
// `clap` wraps descriptions and does not wrap the usage line, and a required
// group of three renders as one 121-column string -- the only command in the
// tree that overflows (`A1-205`, found by the gate rather than by reading). One
// form per line says the same thing and fits, and each line is an invocation a
// reader can copy.
#[command(
    override_usage = "aplan amend <ID> --replace-evidence <FROM> <TO>\n       \
                            aplan amend <ID> --add-evidence <CITATION>\n       \
                            aplan amend <ID> --remove-evidence <CITATION>"
)]
pub struct AmendArgs {
    /// Task id to repair.
    id: String,
    /// The citation to replace and the one to replace it with, each `kind:ref`,
    /// e.g. `--replace-evidence test:old_name test:new_name`.
    #[arg(long, num_args = 2, value_names = ["FROM", "TO"])]
    replace_evidence: Vec<String>,
    /// A citation to add to a closed task, `kind:ref`.
    ///
    /// For proof that was incomplete rather than proof that rotted: a fix whose
    /// tests were written and never cited. Resolved exactly as a replacement is,
    /// so it cannot record what `verify` would reject, and refused if the task
    /// already carries it.
    #[arg(long, value_name = "CITATION")]
    add_evidence: Option<String>,
    /// A citation to remove from a closed task, `kind:ref`.
    ///
    /// For proof that was never proof: a citation chosen to satisfy a gate, or
    /// one that named the deliverable rather than what proves it. Removing it is
    /// refused when what remains would fail the rules, so a task cannot be
    /// repaired below the bar its repository set.
    #[arg(long, value_name = "CITATION")]
    remove_evidence: Option<String>,
    /// Repository to resolve the replacement against (default: the current
    /// directory).
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    /// Record the replacement without resolving it against the repository,
    /// exactly as `close --no-resolve` does, and say what was skipped.
    #[arg(long)]
    no_resolve: bool,
    /// Whether a warning blocks this write, as `check` and `verify` take it.
    #[command(flatten)]
    verdict: VerdictFlags,
}

/// What `aplan show` prints (`A1-118`).
#[derive(Debug, clap::Args)]
pub struct ShowArgs {
    /// The task to show.
    id: String,
    /// `human` (default) or `json`, which carries the same content: a consumer
    /// reading one task must not be pushed back to parsing the file. The
    /// report formats (`agent`, `github`, `sarif`) render a set of findings and
    /// this renders a task, so they are refused rather than silently ignored.
    #[arg(long, default_value = "human")]
    format: String,
    /// Include the task's body, which is prose and can be long.
    #[arg(long)]
    body: bool,
    /// Repository to resolve the evidence against (default: the current
    /// directory).
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    /// Skip the evidence ladder, exactly as `close --no-resolve` does, and say
    /// what was skipped. Resolution is what this command adds to reading the
    /// file, so it is on by default.
    #[arg(long)]
    no_resolve: bool,
}

/// `aplan show`, the read half of the surface (`A1-118`).
///
/// `why` gives blockers, `tasks --format json` omits `paths`, `traces` and
/// `evidence`, and `cat` gives a file that cannot know whether its own citations
/// still resolve. What this adds is resolution: the cut and readiness from the
/// graph, what the task releases from the graph's other direction, and every
/// finding the rules and the ladder attach to this one id.
fn cmd_show(root: &std::path::Path, args: &ShowArgs) -> Result<u8> {
    // The tenth allow-list, and the one that was written longhand rather than
    // as a call: accepting `--format sarif` and printing the human form is a
    // flag that takes a value and ignores it, which is worse than refusing it
    // (`A1-118`). Through the surface table now, so `show` cannot drift from
    // the nine that were already going through one gate (`A1-184`).
    let format = format_for("show", &args.format)?;
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let (config, config_dir) = config_at(&args.repo)?;
    let check_opts = resolve_check_options(&config, config_dir.as_deref(), false, None, false);

    let mut report = validate(&plan, &check_opts);
    // The same convention `close` follows (`A1-109`): resolve unless told not to,
    // and say what was skipped rather than showing an unresolved task as a clean
    // one. A plan kept outside a repository is a real situation and not a reason
    // to refuse to show a task, so it reads as "not resolved" for the same reason
    // `--no-resolve` does, which is the answer `close` already gives.
    let mut resolved = !args.no_resolve;
    if resolved {
        let opts = aplan_evidence::VerifyOptions {
            execute: false,
            test_runner: config.test.clone(),
            max_unrelated_files: config.evidence.as_ref().and_then(|e| e.max_unrelated_files),
            url_check: config.url.clone(),
            config_file: config_dir
                .as_deref()
                .map(|d| d.join(aplan_config::CONFIG_FILE)),
        };
        // Only this task's citations: see `Show::only`.
        let one = aplan_core::show::only(&plan, &args.id);
        match aplan_evidence::verify(&one, &args.repo, &opts) {
            Ok(evidence) => report.diagnostics.extend(evidence.diagnostics),
            Err(
                e
                @ (aplan_evidence::Error::NotARepository(_) | aplan_evidence::Error::NoCommits(_)),
            ) => {
                errln!("aplan: {e}; showing what the rules alone can say.");
                resolved = false;
            }
            Err(e) => return Err(anyhow::Error::new(e).context("resolving evidence")),
        }
    }

    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    let rows = graph.task_rows();
    let body = if args.body {
        plan.tasks
            .iter()
            .find(|lt| lt.task.id == args.id)
            .and_then(|lt| std::fs::read_to_string(&lt.path).ok())
            .and_then(|text| {
                aplan_core::load::split_frontmatter(&text).map(|(_, body)| body.to_string())
            })
    } else {
        None
    };
    let Some(view) =
        aplan_core::Show::assemble(&graph, &rows, &report.diagnostics, &args.id, resolved, body)
    else {
        anyhow::bail!("no task `{}` in this plan", args.id);
    };

    match format {
        Format::Json => render_tolerant(|| {
            // `anstream` passes every byte that is not an SGR sequence through
            // untouched, so the JSON is identical at any `--color` (`A1-286`).
            let mut out = anstream::stdout().lock();
            serde_json::to_writer_pretty(&mut out, &view)?;
            writeln!(out)
        })?,
        _ => write_stdout(&aplan_output::render_show(&view))?,
    }
    Ok(EXIT_OK)
}

/// What `aplan graph` renders, when it is not the whole plan (`A1-126`).
#[derive(Debug, clap::Args)]
pub struct GraphArgs {
    /// Narrow to a subset of the plan. The same flags every reporting command
    /// takes (`A1-214`), and they intersect with `--reach`.
    #[command(flatten)]
    select: SelectArgs,
    /// Scope by reachability: `upstream:<id>`, `downstream:<id>`,
    /// `between:<id>..<id>`, or `unreachable`. Intersects with `--cut` and
    /// `--tag`, so "what this unblocks, in this cut" is one question. The view
    /// says which graph it walked, because reachability over grouping answers
    /// something else entirely.
    #[arg(long, value_name = "QUERY")]
    reach: Option<String>,
    /// Also draw the edges by which a cut lists its members. Off by default:
    /// three quarters of this plan's edges were grouping rather than
    /// constraint, and drawing them made the graph nine layers deep and 177
    /// nodes wide. What is left out is always reported, never silent.
    #[arg(long)]
    membership: bool,
    /// Output format: mermaid (the default, renders in a README and a browser),
    /// text (drawn at this terminal), dot (Graphviz, which every other layout
    /// and analysis tool reads), or json. `--cut` and `--tag` scope every one of
    /// them, so the scope and the rendering are independent.
    #[arg(long, default_value = "mermaid")]
    format: String,
}

/// `aplan graph`, scoped rather than all-or-nothing (`A1-126`).
///
/// `--cut` and `--tag` intersect when both are given, because a reader asking
/// for "the output tasks in this cut" means the tasks in both sets, and a union
/// would answer a question nobody asked and grow the diagram this exists to
/// shrink.
fn cmd_graph(root: &std::path::Path, args: &GraphArgs) -> Result<u8> {
    let format = format_for("graph", &args.format)?;
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());

    let mut include: std::collections::BTreeSet<&str> = graph.by_id.keys().copied().collect();
    let mut scoped = Vec::new();

    let selector = args.select.selector()?;
    // A cut still means "the milestone and its members", which is a graph
    // question the general selector cannot answer: it resolves a label through
    // the same resolver every writing command uses, and pulls in the milestone
    // itself so the diagram has something to hang the members on.
    if !args.select.cut.is_empty() {
        // The *union* of the cuts named, then one intersection with the view.
        // Retaining once per cut intersects them, so `--cut a --cut b` matched
        // nothing while `aplan tasks` with the same flags returned both -- two
        // commands, one documented rule, opposite answers, from the task that
        // exists to remove exactly that (`A1-214`).
        refuse_a_release_as_a_cut(&plan, &graph, &args.select.cut)?;
        let mut members: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for cut in &args.select.cut {
            members.extend(cut_members(&plan, &graph, cut)?);
        }
        include.retain(|id| members.contains(id));
    }
    if !selector.is_empty() {
        let rows = graph.task_rows();
        let admitted: std::collections::BTreeSet<&str> = rows
            .tasks
            .iter()
            .filter(|row| {
                graph.get(&row.id).is_some_and(|t| {
                    // The cut dimension is applied above, against the milestone
                    // and its members; applying it again here would drop the
                    // milestone, which is a member of no cut but is the thing
                    // the members hang from.
                    aplan_core::select::Selector {
                        cuts: Vec::new(),
                        ..selector.clone()
                    }
                    .matches(t, row.cut.as_deref(), row.milestone.as_deref())
                })
            })
            .map(|row| row.id.as_str())
            .collect();
        include.retain(|id| admitted.contains(id));
        scoped.extend(selector.describe());
    }
    // A *selection*, intersected with the others rather than replacing them:
    // `--cut X --reach downstream:Y` is "what Y unblocks, within X", which is
    // one question and not two (`A1-221`). It walks the same edges the view
    // draws, so the scope and the picture cannot disagree about which graph
    // this is.
    if let Some(query) = &args.reach {
        let reach = parse_reach(query)?;
        let edges = if args.membership {
            aplan_core::reach::Edges::Any
        } else {
            aplan_core::reach::Edges::Dependency
        };
        for id in reach.ids() {
            if graph.get(id).is_some() {
                continue;
            }
            // A id with a space in it that names nothing is almost always
            // somebody reaching for a conjunction. Telling them "no task
            // `R-1 and status=todo`" is true and answers a question they did
            // not ask; there is no grammar, and that is what to say. An id that
            // really does contain a space still resolves above (`A1-221`).
            if id.split_whitespace().count() > 1 {
                return Err(reach_grammar_refusal(query));
            }
            anyhow::bail!("no task `{id}` in this plan");
        }
        let selected = reach.select(&graph, edges).ok_or_else(|| {
            anyhow::anyhow!("`--reach {query}` names a task this plan does not have")
        })?;
        include.retain(|id| selected.contains(id));
        scoped.push(format!("`{query}` over {}", reach.walks(edges).describe()));
    }

    // An empty diagram is three characters of valid Mermaid that renders as a
    // blank rectangle, which reads as "this plan has nothing in it" rather than
    // as "your filter matched nothing" (`A1-126`).
    if include.is_empty() {
        // "to draw" was written when Mermaid was the only shape. The refusal is
        // about the scope, which every shape shares, so it says so without
        // assuming which one was asked for (`A1-193`).
        anyhow::bail!(
            "nothing matches {}: no task is in view, so there is no graph",
            scoped.join(" and ")
        );
    }
    // One scoped view, three renderings of it. Computing the scope once is what
    // stops the shapes disagreeing about what is in view (`A1-193`).
    let mut view = aplan_output::GraphView::of_kinds(&graph, &include, args.membership);
    // Said by the view rather than by this command, so every shape carries it:
    // a reachability scope that did not name its graph would leave a reader to
    // assume which one (`A1-221`).
    view.scope.clone_from(&scoped);
    // The narrowing can empty a view that the scope did not: a cut whose members
    // depend on nothing has edges only by membership. Refused for the same
    // reason an empty filter is, and saying which of the two happened.
    if view.nodes.is_empty() {
        anyhow::bail!(
            "no task in view has a dependency on another, so there is no graph. \
             `--membership` draws the {} cut-membership edge(s) instead",
            view.grouping.edges
        );
    }
    match format {
        Format::Mermaid => write_stdout(&aplan_output::mermaid_of_view(&view))?,
        Format::Dot => write_stdout(&aplan_output::dot_of_view(&view))?,
        Format::Text => {
            // Box drawing is what makes this readable and is not guaranteed in
            // every terminal or CI log, so it follows the same signal colour
            // does: a terminal gets it, a pipe gets ASCII (`A1-219`).
            let glyphs = if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                aplan_output::lanes::Glyphs::UNICODE
            } else {
                aplan_output::lanes::Glyphs::ASCII
            };
            let drawn =
                aplan_output::lanes::text_of_view(&view, glyphs, aplan_output::fold::out_width())
                    .map_err(|too_wide| anyhow::anyhow!("{too_wide}"))?;
            write_stdout(&drawn)?;
        }
        Format::Json => return write_json(&view, "aplan/graph/v1"),
        // Refused by `Surface::Graph` before it reaches here.
        other => anyhow::bail!("`{}` does not render a graph", other.as_str()),
    }
    Ok(EXIT_OK)
}

/// What to say when a `--reach` value is not one of the four.
///
/// One wording, so the two ways of getting it wrong -- a shape the parser does
/// not know, and an expression that parsed as an id -- answer the same way.
fn reach_grammar_refusal(query: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "`--reach {query}` is not one of the four: `upstream:<id>`, `downstream:<id>`, \
         `between:<id>..<id>`, `unreachable`. Anything narrower is a predicate over one \
         task's fields, which `aplan tasks --format json` answers and `jq` filters"
    )
}

/// One of four reachability questions, or a refusal that lists them.
///
/// Deliberately not a grammar. `A1-214` draws the same line for selection -- a
/// conjunction of equality tests, and `aplan tasks` beyond it -- and
/// `docs/adr/0026-reachability-not-a-query-language.md` records why a fifth form
/// is where a filter language starts (`A1-221`).
fn parse_reach(query: &str) -> Result<aplan_core::reach::Reach> {
    use aplan_core::reach::Reach;

    let refuse = || reach_grammar_refusal(query);
    if query == "unreachable" {
        return Ok(Reach::Unreachable);
    }
    let (kind, rest) = query.split_once(':').ok_or_else(refuse)?;
    if rest.is_empty() {
        return Err(refuse());
    }
    match kind {
        "upstream" => Ok(Reach::Upstream(rest.to_string())),
        "downstream" => Ok(Reach::Downstream(rest.to_string())),
        "between" => {
            let (from, to) = rest.split_once("..").ok_or_else(refuse)?;
            if from.is_empty() || to.is_empty() {
                return Err(refuse());
            }
            Ok(Reach::Between(from.to_string(), to.to_string()))
        }
        _ => Err(refuse()),
    }
}

/// The ids a `--cut` names: the milestone and its members.
///
/// Accepts the milestone id or the label the cut reports itself under, through
/// the resolver every writing command shares since `A1-139`. This command had
/// the rule first; the point of moving it is that `roadmap` prints the label and
/// `mv --cut` used to refuse it.
fn cut_members<'a>(
    plan: &aplan_core::load::Plan,
    graph: &Graph<'a>,
    cut: &str,
) -> Result<std::collections::BTreeSet<&'a str>> {
    // The same resolution every writing command uses, rather than a second copy
    // of the id-or-label rule: this command had the rule first, and `A1-139`
    // moved it where both can reach it.
    let id = aplan_interop::cut::resolve_id(plan, cut)?;
    let roadmap = graph.roadmap();
    let Some(found) = roadmap.cuts.iter().find(|c| c.milestone_id == id) else {
        anyhow::bail!("`{id}` is not a cut in this plan");
    };
    let mut ids: std::collections::BTreeSet<&'a str> = std::collections::BTreeSet::new();
    for want in std::iter::once(found.milestone_id.as_str())
        .chain(found.members.iter().map(|m| m.id.as_str()))
    {
        if let Some((id, _)) = graph.by_id.get_key_value(want) {
            ids.insert(id);
        }
    }
    Ok(ids)
}

/// `aplan tree`, the inspection a derived order needs (`A1-126`, ADR-0017).
///
/// Two commands rather than one with a mode flag: `graph --cut` answers "show me
/// this region" and `tree <id>` answers "what is around this task", and the two
/// take different arguments, print different formats and are read at different
/// moments. A single command with `--around` would have to explain which flags
/// apply to which question.
fn cmd_tree(root: &std::path::Path, id: &str, depth: Option<usize>, live: bool) -> Result<u8> {
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    let Some(out) = aplan_output::tree(&graph, id, depth, live) else {
        anyhow::bail!("no task `{id}` in this plan");
    };
    write_stdout(&out)?;
    Ok(EXIT_OK)
}

/// `aplan ready`, in a derived order rather than in id order (`A1-125`).
///
/// The order is `Rank`'s, and `--explain` prints it: `ready` answers "what
/// should I do next", and a list presented as an answer in an order that means
/// nothing teaches a reader that the order means nothing, after which a real
/// order is not believed either.
fn cmd_ready(root: &std::path::Path, args: &ReadyArgs) -> Result<u8> {
    let (explain, select) = (args.explain, &args.select);
    let format = format_for("ready", &args.format)?;
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    // One value, whichever surface asks. The human rendering below reads the
    // same fields the JSON serialises, so the sentence `--explain` prints and
    // the fields a script sorts by cannot say different things (`A1-182`).
    let selector = select.selector()?;
    let mut answer = aplan_mcp::tools::ready_of(&plan);
    if !selector.is_empty() {
        // Ranked over the whole plan and then narrowed: a rank is an order over
        // everything that could be started, and recomputing it inside a subset
        // would answer a different question (`A1-214`).
        let keep = selected_ids(&plan, &selector)?;
        answer.ready.retain(|t| keep.contains(&t.id));
        answer.closeable_cuts.retain(|t| keep.contains(&t.id));
    }
    if format == Format::Json {
        // `--explain` is not consulted: the reasons are fields here, and a
        // machine shape conditional on a flag is one a consumer has to know the
        // invocation to read.
        return write_json(&answer, "aplan/ready/v1");
    }

    let mut out = String::new();
    for t in &answer.ready {
        let effort = t
            .effort_days
            .map_or_else(String::new, |e| format!(" ({e}d)"));
        // The title is the trailing column and the effort rides with it, so a
        // long title folds under itself rather than under the id (`A1-206`).
        let _ = writeln!(
            out,
            "{}",
            aplan_output::fold::row(
                &format!("{id}{}{id:#}  ", t.id, id = aplan_output::style::ID),
                &format!(
                    "{}{dim}{}{dim:#}",
                    aplan_output::sanitize::visible(&t.title),
                    effort,
                    dim = aplan_output::style::DIM
                ),
            )
        );
        if explain {
            // Built from the payload's own fields rather than recomputed from
            // the graph. Both were here at once for a while, which is the
            // duplication this task removed between the CLI and MCP appearing
            // again inside the CLI.
            let rank = aplan_core::Rank {
                releases: t.releases,
                downstream_depth: t.downstream_depth,
                critical: t.critical,
                effort_days: t.effort_days.unwrap_or(0.0),
            };
            // A preference is the reason a task is *not* first, so it is said
            // beside the rank rather than folded into it: the rank is derived
            // from the plan and this is a judgement somebody recorded
            // (`A1-150`, ADR-0025).
            let because = if t.prefers_to_follow.is_empty() {
                rank.because()
            } else {
                format!(
                    "{}; prefers to follow {}",
                    rank.because(),
                    t.prefers_to_follow.join(", ")
                )
            };
            let _ = writeln!(out, "    {because}");
        }
    }
    if answer.ready.is_empty() {
        let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
        out.push_str(&why_nothing_is_ready(&Graph::new(tasks.iter().copied())));
    }
    write_stdout(&out)?;
    Ok(EXIT_OK)
}

/// `aplan dep`, the only per-task ordering the model has, editable at last.
///
/// `add` wrote `blocked_by` and nothing edited it, so saying "this comes first"
/// after the fact meant hand-editing frontmatter in a repository whose premise is
/// that the tool owns those files (`A1-124`, pre-public adoption evidence item 6).
///
/// Every refusal it needs already exists: a cycle is `APLAN004`, an id that does
/// not exist is `APLAN003`, a live task blocked by a cancelled one is `APLAN020`,
/// and a `done` task acquiring an unfinished blocker is `APLAN007`. `A1-113`'s
/// post-state check reports all four, so this hand-rolls none of them, which is
/// the whole reason that task came first.
///
/// The one refusal that is *not* a rule is a cut's membership: a milestone's
/// `blocked_by` is which tasks are in it, and editing one end of that here is
/// `mv` with its guards removed, in the two ways
/// [`aplan_core::mutation::membership_edit`] measures. Sequencing one cut after
/// another is the same field and stays, because that is what `mv` sends you here
/// to do.
/// The one task an id names, refusing a plan where more than one declares it
/// (`A1-135`).
///
/// Every mutating command used a bare `find`, so on a plan with a duplicate id
/// two commands edited two different files and neither said anything. The
/// refusal lives in `mutation::` so both surfaces ask the same question, and
/// this is the CLI's one call site for it.
fn locate<'p>(
    plan: &'p aplan_core::load::Plan,
    id: &str,
) -> Result<&'p aplan_core::load::LoadedTask> {
    if let Some(why) = aplan_core::mutation::ambiguous_id(plan, id) {
        anyhow::bail!(why);
    }
    plan.tasks
        .iter()
        .find(|lt| lt.task.id == id)
        // "no task `X` in this plan", which is what `drop` and `mv` already
        // said: it names where it looked, and one question deserves one wording.
        .ok_or_else(|| anyhow::anyhow!("no task `{id}` in this plan"))
}

fn cmd_dep(root: &std::path::Path, args: &DepCommand) -> Result<u8> {
    let (
        adding,
        DepArgs {
            id,
            after,
            prefer,
            verdict,
        },
    ) = match args {
        DepCommand::Add(a) => (true, a),
        DepCommand::Rm(a) => (false, a),
    };
    if id == after {
        anyhow::bail!("`{id}` cannot come after itself");
    }
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let lt = locate(&plan, id)?;

    // Cut membership is `mv`'s, and only a dependency edge can express it: a
    // preference is not membership, so this guard reads the edge being edited
    // rather than the command (`A1-150`).
    if !*prefer && let Some(why) = aplan_core::mutation::membership_edit(&plan, id, after, adding) {
        anyhow::bail!(why);
    }

    // A preference edits `prefer_after`, a dependency edits `blocked_by`, and the
    // two are never the same list: writing a preference into `blocked_by` is the
    // conflation ADR-0017 measured and ADR-0025 exists to keep separate.
    // Two forms of the verb, because "does not comes after" is what one form
    // produces and a message a person reads has to be a sentence.
    let (verb, bare, mut edges) = if *prefer {
        (
            "prefers to follow",
            "prefer to follow",
            lt.task.prefer_after.clone(),
        )
    } else {
        ("comes after", "come after", lt.task.blocked_by.clone())
    };
    let present = edges.iter().any(|b| b == after);
    match (adding, present) {
        (true, true) => {
            write_stdout(&format!("`{id}` already {verb} `{after}`\n"))?;
            return Ok(EXIT_OK);
        }
        (false, false) => {
            write_stdout(&format!("`{id}` does not {bare} `{after}`\n"))?;
            return Ok(EXIT_OK);
        }
        (true, false) => edges.push(after.clone()),
        (false, true) => edges.retain(|b| b != after),
    }

    let original = std::fs::read_to_string(&lt.path)
        .with_context(|| format!("reading {}", lt.path.display()))?;
    let updated = if *prefer {
        aplan_core::close::set_prefer_after(&original, &edges)
    } else {
        aplan_core::close::set_blocked_by(&original, &edges)
    }
    .map_err(|e| close_err("dep", id, &e))?;

    let after_plan = aplan_core::mutation::with_replaced(&plan, id, task_of(&updated, id)?);
    let what = match (adding, *prefer) {
        (true, false) => format!("make `{id}` come after `{after}`"),
        (false, false) => format!("stop `{id}` coming after `{after}`"),
        (true, true) => format!("make `{id}` prefer to follow `{after}`"),
        (false, true) => format!("stop `{id}` preferring to follow `{after}`"),
    };
    refuse_bad_mutation(root, &what, &plan, &after_plan, *verdict)?;

    // The *subcommand*, not the command: `dep` on its own prints help and exits
    // `0`, so "Run `aplan dep` again" named an invocation that does nothing. The
    // invocation gate (`A1-089`) accepts it, because it parses -- parsing and
    // doing the work are different questions (`A1-246`'s audit).
    save_one(
        &lt.path,
        &original,
        &updated,
        if adding { "dep add" } else { "dep rm" },
    )?;
    write_stdout(&format!(
        "{} `{id}` {verb} `{after}` in {}\n",
        if adding { "now" } else { "no longer" },
        lt.path.display()
    ))?;
    Ok(EXIT_OK)
}

/// `aplan supersede`, the repair for a closed task whose traced outcome stopped
/// holding.
///
/// `APLAN018` told an adopter to "reopen the task". There is no reopen, `drop`
/// refuses to revive terminal work, and hand-editing `status` is the thing this
/// tool exists to stop. The published remedy was not executable (`A1-130`,
/// pre-public adoption evidence).
///
/// It is not built either, and that is the decision rather than the shortcut
/// (ADR-0020). The work *was* done: its commit really did touch those paths, and
/// the cut it completed really did complete. What changed is the ground under it.
/// Reopening would rewrite the plan to say the work never happened, invalidate a
/// finished cut and everything downstream of it, and leave the old evidence
/// either attached to unfinished work or deleted. Superseding says what is true:
/// this was finished, and then it stopped being enough.
///
/// One change, or none: the successor is created, the predecessor is re-stamped
/// as reviewed, and the cut that gains the successor is edited -- all in a
/// single batch since `A1-246`, because the ordering that preceded it left an
/// orphan under contention. The stamp records *review*, not validity, which is
/// what makes it honest here and what stops the warning nagging forever about a
/// drift somebody has already dispositioned.
/// The refusals that do not depend on the plan, before any of the work.
///
/// Extracted to keep `cmd_supersede` readable once it grew a verdict flag
/// (`A1-266`); the set is unchanged. A milestone, a task that is not `done`,
/// and a successor with no title are each a fact about the arguments, so they
/// belong together and before anything is resolved.
fn refusals_before_a_successor(
    lt: &aplan_core::load::LoadedTask,
    args: &SupersedeArgs,
) -> Result<()> {
    if lt.task.is_milestone() {
        anyhow::bail!(
            "`{}` is a milestone, and a cut is not superseded: its members are. Supersede the \
             member whose outcome stopped holding, or `aplan add` the new work into a live cut.",
            args.id
        );
    }
    if lt.task.status != aplan_core::Status::Done {
        anyhow::bail!(
            "`{}` is `{}`, and only a `done` task has an outcome that can stop holding. \
             Edit a live task, or `aplan drop` one you are cancelling.",
            args.id,
            lt.task.status
        );
    }
    if args.title.trim().is_empty() {
        anyhow::bail!("a successor needs a title: it is what now needs doing");
    }
    Ok(())
}

fn cmd_supersede(root: &std::path::Path, args: &SupersedeArgs) -> Result<u8> {
    use aplan_interop::{cut, emit, ids};

    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let lt = locate(&plan, &args.id)?;
    // Only a closed task has an outcome to stop holding. A `todo` task that needs
    // different work is edited, and a `dropped` one was already dispositioned.
    // A cut is not superseded, its members are. A milestone's outcome is that its
    // members are terminal, so filing a successor to one would file work the cut
    // does not contain, into no cut, under a title nobody asked for. Accepting it
    // produced exactly that.
    refusals_before_a_successor(lt, args)?;
    if let Some(days) = args.effort {
        if !days.is_finite() || days < 0.0 {
            anyhow::bail!(
                "`--effort {days}` is not a number of days; give a finite, non-negative estimate \
                 or omit it"
            );
        }
    }
    // Id or label, like every other `--cut` (`A1-139`).
    let cut = match &args.cut {
        Some(name) => Some(cut::resolve_id(&plan, name)?),
        None => None,
    };
    if let Some(cut_id) = cut.as_deref() {
        cut::check_target(&plan, cut_id)?;
    }

    let phase = lt.task.phase;
    let id = ids::mint(&plan, phase, id_strategy(root)?)?;
    let successor = aplan_core::Task {
        id: id.clone(),
        title: args.title.clone(),
        phase,
        status: aplan_core::Status::Todo,
        owner: args.owner.clone().or_else(|| lt.task.owner.clone()),
        effort_days: args.effort,
        blocked_by: Vec::new(),
        prefer_after: Vec::new(),
        evidence: Vec::new(),
        // Inherited, because the successor is about the same code and the same
        // document: the `traces` are what drifted, and the `paths` are what the
        // work will touch. Both are editable afterwards, and neither is a guess:
        // they are the predecessor's own answers to the same questions.
        paths: lt.task.paths.clone(),
        traces: lt.task.traces.clone(),
        trace_fingerprints: std::collections::BTreeMap::new(),
        tags: Vec::new(),
        revisit_at: None,
        version_impact: None,
        version_impact_reason: None,
    };

    let (config, config_dir) = config_at(root)?;
    let opts = resolve_check_options(&config, config_dir.as_deref(), false, None, false);
    let fingerprints = if opts.detect_drift {
        aplan_core::trace_fingerprints(&lt.task, opts.traces_root.as_deref())
    } else {
        std::collections::BTreeMap::new()
    };
    let original = std::fs::read_to_string(&lt.path)
        .with_context(|| format!("reading {}", lt.path.display()))?;
    // Built before anything is written, so a predecessor this cannot re-stamp
    // leaves the plan byte-identical rather than a successor filed against a task
    // still reported as drifted.
    let restamped = aplan_core::restamp(&original, &fingerprints)
        .map_err(|e| close_err("supersede", &args.id, &e))?;

    // Both writes are prepared, so the only failures left are at the point of
    // writing. Ordering no longer carries the argument it used to -- both files
    // go in one batch since `A1-246`, so an unwritable predecessor cannot leave
    // the successor filed -- but the probe still earns its place: it names the
    // file and the permission before any of the resolution work, where the
    // batch's own failure would name it after. It opens for writing without
    // truncating, so it proves the permission rather than assuming it from
    // metadata.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&lt.path)
        .with_context(|| format!("{} is not writable", lt.path.display()))?;

    let after = aplan_core::mutation::with_added(
        &aplan_core::mutation::with_replaced(&plan, &args.id, task_of(&restamped, &args.id)?),
        std::path::PathBuf::from(format!("{id}.md")),
        successor.clone(),
    );
    refuse_bad_mutation(
        root,
        &format!("supersede `{}`", args.id),
        &plan,
        &after,
        args.verdict,
    )?;

    let out_dir = successor_dir(root);
    let body = supersede_body(&lt.task, &args.id);

    // One change, or none (`A1-246`); the argument is on `supersede_writes`.
    let (batch, placed) = supersede_writes(
        &plan,
        &out_dir,
        &id,
        &successor,
        &body,
        cut.as_deref(),
        &lt.path,
        &original,
        restamped,
    )?;
    save_batch(&batch, "supersede")?;

    write_stdout(&superseded_said(
        &args.id,
        &id,
        fingerprints.len(),
        &emit::task_path(&out_dir, &id),
        placed.as_deref(),
    ))?;
    Ok(EXIT_OK)
}

/// What `supersede` reports: the ids, the stamp count, and each file it wrote.
///
/// Only claims a stamp where one was taken: a task with no `traces`, or a
/// repository with drift off, has nothing to re-stamp, and "re-stamped 0 trace
/// fingerprint(s)" is a repair reported for work that did not happen.
fn superseded_said(
    predecessor: &str,
    successor: &str,
    stamps: usize,
    filed: &std::path::Path,
    placed: Option<&std::path::Path>,
) -> String {
    let mut out = match stamps {
        0 => format!("superseded {predecessor}: filed {successor}\n"),
        n => format!(
            "superseded {predecessor}: filed {successor} and re-stamped {n} trace fingerprint(s)\n"
        ),
    };
    let _ = writeln!(out, "  -> {}", filed.display());
    if let Some(p) = placed {
        let _ = writeln!(out, "  -> {}", p.display());
    }
    out
}

/// Every file a `supersede` touches, as one batch (`A1-246`).
///
/// Extracted so `cmd_supersede` stays readable, and because the *set* is the
/// interesting thing: the successor, the predecessor's re-stamp, and the cut
/// that gains the successor. Returns the milestone path for the message, if a
/// cut was named.
///
/// **It used to be two writes in an order.** The successor first, the
/// predecessor second, and the comment defending that said a failure between
/// them left "a plan with one extra `todo` task and a drift warning still open,
/// and both are visible to `check`". The visibility is real -- `APLAN009`
/// reports the orphan. What it missed is that the refusal says *nothing was
/// written* and tells the author to run the command again, and the retry mints
/// a *second* successor: following the tool's own advice under contention
/// leaves `T-003` and `T-004` for one supersede. `A1-226` made that contention
/// ordinary rather than exotic, which is what turned a defensible trade into a
/// defect.
#[allow(clippy::too_many_arguments)]
fn supersede_writes(
    plan: &aplan_core::load::Plan,
    out_dir: &std::path::Path,
    id: &str,
    successor: &aplan_core::Task,
    body: &str,
    cut: Option<&str>,
    predecessor: &std::path::Path,
    original: &str,
    restamped: String,
) -> Result<(
    Vec<aplan_core::save::Replacement>,
    Option<std::path::PathBuf>,
)> {
    let mut batch = vec![
        aplan_core::save::Replacement {
            path: aplan_interop::emit::task_path(out_dir, id),
            // Empty: this file must not exist yet, and `SaveError::Exists` is
            // the distinct answer when the id was taken while we worked.
            expected: String::new(),
            updated: aplan_interop::emit::task_file(successor, body)?,
        },
        aplan_core::save::Replacement {
            path: predecessor.to_path_buf(),
            expected: original.to_string(),
            updated: restamped,
        },
    ];
    // The cut that gains the successor joins the same batch rather than going
    // through a writer of its own, so a locked milestone leaves no orphan
    // either. `plan_placement` resolves against the milestone this plan already
    // has; the successor need not be loadable yet.
    let placed = match cut {
        Some(cut_id) => {
            let pending = aplan_interop::cut::plan_placement(
                plan,
                cut_id,
                std::slice::from_ref(&id.to_string()),
            )?;
            match pending.replacement() {
                Some(r) => {
                    let path = r.path.clone();
                    batch.push(r);
                    Some(path)
                }
                None => Some(pending.path.clone()),
            }
        }
        None => None,
    };
    Ok((batch, placed))
}

/// Where a new task file is written: `tasks/` where the plan has one, else the
/// plan root.
///
/// One implementation, because there were two: `add` decided this and
/// `supersede` decided it again, and two commands filing tasks into different
/// directories is a plan split across two places by an accident of copying.
fn successor_dir(root: &std::path::Path) -> std::path::PathBuf {
    let conventional = root.join("tasks");
    if conventional.is_dir() {
        conventional
    } else {
        root.to_path_buf()
    }
}

/// The successor's body, naming what it succeeds and why.
fn supersede_body(predecessor: &aplan_core::Task, id: &str) -> String {
    let mut body = String::new();
    let _ = writeln!(body, "## Why\n");
    let _ = writeln!(
        body,
        "This succeeds `{id}` (\"{}\"), whose traced outcome no longer holds.\n",
        predecessor.title
    );
    let _ = writeln!(
        body,
        "That task stays `done` and keeps its evidence: the work was finished, and what \
         changed afterwards is the ground under it. This task is what now needs doing.\n"
    );
    if !predecessor.traces.is_empty() {
        let _ = writeln!(
            body,
            "It traces the same section(s), which is what drifted:\n"
        );
        for t in &predecessor.traces {
            let _ = writeln!(body, "- `{t}`");
        }
        body.push('\n');
    }
    let _ = writeln!(
        body,
        "(Say what changed, and what is different once this is done.)\n"
    );
    let _ = writeln!(body, "## Acceptance criteria\n");
    let _ = writeln!(
        body,
        "- [ ] state what done means for this task, in this repository's terms"
    );
    body
}

/// `aplan amend`, repair a closed task's evidence in place (`A1-110`).
///
/// The pilot's case: a renamed test left two closed tasks citing seven names
/// that resolved to nothing, and the tool refused twice and told the author to
/// edit the files by hand. This is the supported repair.
fn cmd_amend(root: &std::path::Path, args: &AmendArgs) -> Result<u8> {
    let (id, replace, repo, no_resolve) = (
        args.id.as_str(),
        args.replace_evidence.as_slice(),
        args.repo.as_path(),
        args.no_resolve,
    );
    let plan = load_checked(root)?;
    // Never edit a task inside a plan that does not load: its neighbours may be
    // the reason this one should not be touched. `close` takes the same position.
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let lt = locate(&plan, id)?;

    let original = std::fs::read_to_string(&lt.path)
        .with_context(|| format!("reading {}", lt.path.display()))?;

    // Two shapes of repair, and the same gate for both. Structural refusals come
    // first, as `close` orders them: an unknown citation or a live task must be
    // told that, not told the replacement would fail `verify` (`A1-109`, audit).
    let (repaired, amended, said) = if let Some(drop) = &args.remove_evidence {
        let drop = parse_evidence(drop)?;
        let remaining: Vec<aplan_core::Evidence> = lt
            .task
            .evidence
            .iter()
            .filter(|e| **e != drop)
            .cloned()
            .collect();
        (
            aplan_core::remove_evidence(&original, &drop)
                .map_err(|e| close_err("amend", id, &e))?,
            remaining,
            format!("- {}:{}", drop.kind.as_str(), drop.reference),
        )
    } else if let Some(add) = &args.add_evidence {
        let add = parse_evidence(add)?;
        let mut amended = lt.task.evidence.clone();
        if amended.contains(&add) {
            anyhow::bail!(
                "cannot amend `{id}`: this task already cites `{}:{}`",
                add.kind.as_str(),
                add.reference
            );
        }
        amended.push(add.clone());
        (
            aplan_core::add_evidence(&original, &add).map_err(|e| close_err("amend", id, &e))?,
            amended,
            format!("+ {}:{}", add.kind.as_str(), add.reference),
        )
    } else {
        let [from, to] = replace else {
            anyhow::bail!(
                "`--replace-evidence` takes exactly two citations: the one to replace, then \
                 the one to replace it with"
            );
        };
        let (from, to) = (parse_evidence(from)?, parse_evidence(to)?);
        let amended: Vec<aplan_core::Evidence> = lt
            .task
            .evidence
            .iter()
            .map(|e| if *e == from { to.clone() } else { e.clone() })
            .collect();
        (
            aplan_core::replace_evidence(&original, &from, &to)
                .map_err(|e| close_err("amend", id, &e))?,
            amended,
            format!(
                "{}:{} -> {}:{}",
                from.kind.as_str(),
                from.reference,
                to.kind.as_str(),
                to.reference
            ),
        )
    };

    // The same gate `close` applies, through the same function. A repair that
    // accepts any citation without resolving it would be `A1-109`'s defect with a
    // second door, which is what this task set out not to build.
    let (config, _) = config_at(root)?;
    if no_resolve {
        write_stdout(
            "note: `--no-resolve`: the change was recorded without being resolved against \
             the repository. Run `aplan verify` before trusting this repair.\n",
        )?;
    // Judged as a *delta*, not absolutely. A repair is answerable for what it
    // introduces, not for what was already broken: a task with two rotted
    // citations could otherwise not be repaired one at a time, because each
    // repair is refused for the defect it is not fixing. That is the pilot's own
    // case -- p0000004 item 9 was seven citations across two tasks -- and it is
    // the "right about a citation and wrong about the task" shape `p00025` item
    // 14 names (`A1-110`, found dogfooding on this plan).
    } else if !replacement_resolves(id, lt, &lt.task.evidence, &amended, repo, &config)? {
        return Ok(EXIT_VIOLATIONS);
    }

    let after = aplan_core::mutation::with_replaced(&plan, id, task_of(&repaired, id)?);
    refuse_bad_mutation_unless(
        root,
        &format!("amend `{id}`"),
        &plan,
        &after,
        no_resolve,
        args.verdict,
    )?;
    save_one(&lt.path, &original, &repaired, "amend")?;
    // Both sides, because the point of naming them is that the change is legible
    // here and in the diff.
    write_stdout(&format!(
        "amended {i}{id}{i:#}: {said}\n",
        i = aplan_output::style::ID
    ))?;
    Ok(EXIT_OK)
}

/// The sentence a surface prints about evidence nothing resolved.
///
/// One wording for both repairs, and one that names the cause a reader can act
/// on: a missing `test` runner is a config question and a plan kept outside a
/// repository is not, and the two reached this as the same empty result
/// (`A1-238`, audit).
fn unresolved_note(kinds: &[&str], repository_unreadable: bool) -> String {
    let because = if repository_unreadable {
        "there is no repository here to resolve it against"
    } else {
        "nothing is configured to check it"
    };
    format!(
        "note: {} evidence was recorded unresolved: {because}\n",
        kinds.join(", ")
    )
}

/// Whether a repair introduces a finding the task did not already carry.
///
/// `close` can ask absolutely, because it only ever adds a task's *first*
/// evidence and there is nothing pre-existing to be wrong. A repair cannot: the
/// task is closed, something on it has rotted, and refusing the fix because a
/// neighbouring citation is also rotten makes the tool unusable exactly where it
/// is most needed (`A1-110`).
fn replacement_resolves(
    id: &str,
    lt: &aplan_core::load::LoadedTask,
    before: &[aplan_core::Evidence],
    after: &[aplan_core::Evidence],
    repo: &std::path::Path,
    config: &aplan_config::Config,
) -> Result<bool> {
    let verify_opts = aplan_evidence::VerifyOptions {
        execute: false,
        test_runner: config.test.clone(),
        max_unrelated_files: config.evidence.as_ref().and_then(|e| e.max_unrelated_files),
        url_check: None,
        // No location to attach: this resolves one proposed citation rather than
        // running the plan, and its findings are about that citation.
        config_file: None,
    };
    let resolve = |ev: &[aplan_core::Evidence]| -> Result<Vec<String>> {
        Ok(
            aplan_evidence::resolve_proposed(&lt.task, &lt.path, ev, repo, &verify_opts)
                .context("resolving the evidence before recording it")?
                .report
                .diagnostics
                .iter()
                .map(|d| format!("{}\t{}", d.code, d.message))
                .collect(),
        )
    };
    let existing = resolve(before)?;
    let proposed = aplan_evidence::resolve_proposed(&lt.task, &lt.path, after, repo, &verify_opts)
        .context("resolving the evidence before recording it")?;

    let findings: Vec<&aplan_core::Diagnostic> = proposed
        .report
        .diagnostics
        .iter()
        .filter(|d| !existing.contains(&format!("{}\t{}", d.code, d.message)))
        .collect();
    if !findings.is_empty() {
        for d in &findings {
            write_stdout(&format!("{}: {}\n", d.code, d.message))?;
            if let Some(fix) = &d.agent_instruction {
                write_stdout(&format!("  fix: {fix}\n"))?;
            }
        }
        write_stdout(&format!(
            "aplan: refusing to amend `{id}`: this change would fail `aplan verify`. \
             Fix the citation, or pass `--no-resolve` to record it unchecked.\n"
        ))?;
        return Ok(false);
    }

    // **A repair says what it could not check**, exactly as `close` does and for
    // the reason written beside it there: silence about a check nothing ran reads
    // exactly like a check that passed. `amend` had the resolution and not the
    // sentence, so outside a repository -- or with `--repo` pointing at one that
    // cannot answer -- it recorded a citation `verify` would reject and said
    // nothing at all (`A1-238`).
    //
    // **After the refusal, not before it**, which is where `close` puts the same
    // line and says why: a message may name only what happened (`A1-091`).
    // Printed first, this told a reader that evidence "was recorded unresolved"
    // and then refused to record it -- reachable whenever the added citation is
    // both uncheckable and enough to wake a rule, as a `test` citation is for
    // `APLAN019` on a task with a dead `paths` glob (`A1-238`, audit).
    //
    // Keyed on the citations this repair *introduces*, not on a delta of the
    // unchecked kinds: a task already carrying an unchecked `commit` would
    // otherwise absorb a second one in silence, which is the case that found
    // this. A repair that only removes evidence introduces nothing and says
    // nothing.
    let unchecked = proposed.unchecked_names();
    let now_unchecked: std::collections::BTreeSet<&str> = after
        .iter()
        .filter(|e| !before.contains(e))
        .map(|e| e.kind.as_str())
        .filter(|kind| unchecked.contains(kind))
        .collect();
    if !now_unchecked.is_empty() {
        write_stdout(&unresolved_note(
            &now_unchecked.into_iter().collect::<Vec<_>>(),
            proposed.repository_unreadable,
        ))?;
    }
    Ok(true)
}

/// Resolve a proposed closure's evidence, report it, and say whether to proceed.
///
/// `close` is the sentence this project stakes its claim on, and it once accepted
/// three `test` citations that `verify` rejected seconds later (`A1-109`). The
/// refusal reuses `denied`, the predicate the build already uses, rather than a
/// second severity policy invented for closing.
fn evidence_resolves(
    op: &str,
    id: &str,
    lt: &aplan_core::load::LoadedTask,
    evidence: &[aplan_core::Evidence],
    repo: &std::path::Path,
    config: &aplan_config::Config,
    verdict: VerdictFlags,
) -> Result<bool> {
    let verify_opts = aplan_evidence::VerifyOptions {
        execute: false,
        test_runner: config.test.clone(),
        max_unrelated_files: config.evidence.as_ref().and_then(|e| e.max_unrelated_files),
        // Deliberately not `config.url`: `close` must not reach the network to
        // record a citation. A link down for an hour would block a closure, and
        // `APLAN016` is a warning precisely because it is not a statement about
        // the plan (`A1-041`).
        url_check: None,
        // No location to attach: this resolves one proposed citation rather than
        // running the plan.
        config_file: None,
    };
    let resolution =
        aplan_evidence::resolve_proposed(&lt.task, &lt.path, evidence, repo, &verify_opts)
            .context("resolving the evidence before recording it")?;

    // The repository's own policy, both for the verdict and for the summary line
    // above it: a preview rendered under a different policy from the one that
    // decides the refusal is a second opinion nobody asked for (`A1-112`).
    // The flags too, not just the config key (`A1-266`'s audit). `close` runs
    // two gates -- this one over the citation, and the post-state rules over the
    // plan it would produce -- and a flag that stopped one and not the other
    // makes the same command answer two ways about the same word. Measured: a
    // diluted commit (`APLAN024`, a warning) was refused under
    // `deny_warnings: true` and closed under `--deny-warnings`.
    let policy = verdict_policy(config, verdict.deny_warnings, verdict.no_deny_warnings);
    if !resolution.report.diagnostics.is_empty() {
        render_proposed(&resolution.report, &policy)?;
    }
    if resolution.refuses(&policy) {
        write_stdout(&format!(
            "aplan: refusing to {op} `{id}`: this evidence would fail `aplan verify`. \
             Fix the citation, or pass `--no-resolve` to record it unchecked.\n"
        ))?;
        return Ok(false);
    }
    // Only once the closure is going ahead. Said before the refusal, this claimed
    // the evidence "was recorded" while the command was about to record nothing,
    // which is the shape `A1-091` forbids: a message may name only what happened.
    //
    // Silence about a check nothing ran would read exactly like a check that
    // passed, which is the defect this task is a case of.
    let unchecked = resolution.unchecked_names();
    if !unchecked.is_empty() {
        write_stdout(&unresolved_note(
            &unchecked,
            resolution.repository_unreadable,
        ))?;
    }
    Ok(true)
}

fn cmd_close(
    root: &std::path::Path,
    id: &str,
    evidence_args: &[String],
    repo: &std::path::Path,
    restamp: bool,
    no_resolve: bool,
    verdict: VerdictFlags,
) -> Result<u8> {
    let plan = load_checked(root)?;
    // Never edit a task inside a plan that does not load, its neighbours might be
    // the reason this one should not close.
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }

    let lt = locate(&plan, id)?;

    // Config drives drift stamping (`APLAN018`): whether it is on, and the
    // `traces_root` the traced documents are read from.
    let (config, config_dir) = config_at(root)?;
    let opts = resolve_check_options(&config, config_dir.as_deref(), false, None, false);
    let fingerprints = if opts.detect_drift {
        aplan_core::trace_fingerprints(&lt.task, opts.traces_root.as_deref())
    } else {
        std::collections::BTreeMap::new()
    };

    let original = std::fs::read_to_string(&lt.path)
        .with_context(|| format!("reading {}", lt.path.display()))?;

    // `--restamp`: re-take a `done` task's trace fingerprints, nothing else.
    if restamp {
        let updated = aplan_core::restamp(&original, &fingerprints)
            .map_err(|e| close_err("restamp", id, &e))?;
        let after = aplan_core::mutation::with_replaced(&plan, id, task_of(&updated, id)?);
        refuse_bad_mutation(root, &format!("restamp `{id}`"), &plan, &after, verdict)?;
        save_one(&lt.path, &original, &updated, "close")?;
        write_stdout(&format!(
            "restamped {id}: {} trace fingerprint(s)\n",
            fingerprints.len()
        ))?;
        return Ok(EXIT_OK);
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

    // Structural refusals first: already `done`, already carrying evidence. They
    // are hermetic and instant, and running the ladder ahead of them spent a
    // runner subprocess to report the *wrong reason* -- an already-closed task
    // was told its citation would fail `verify`, which was true and irrelevant.
    let updated = aplan_core::close(&original, &evidence, &fingerprints)
        .map_err(|e| close_err("close", id, &e))?;

    // Then the gate, still before anything is written (`A1-109`).
    if !no_resolve && !evidence_resolves("close", id, lt, &evidence, repo, &config, verdict)? {
        return Ok(EXIT_VIOLATIONS);
    }

    if no_resolve {
        write_stdout(
            "note: `--no-resolve`: the evidence was recorded without being resolved against \
             the repository. Run `aplan verify` before trusting this closure.\n",
        )?;
    }

    let after = aplan_core::mutation::with_replaced(&plan, id, task_of(&updated, id)?);
    refuse_bad_mutation_unless(
        root,
        &format!("close `{id}`"),
        &plan,
        &after,
        no_resolve,
        verdict,
    )?;
    save_one(&lt.path, &original, &updated, "close")?;

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
        "closed {i}{id}{i:#} with evidence: {}{stamped}\n",
        refs.join(", "),
        i = aplan_output::style::ID
    ))?;
    Ok(EXIT_OK)
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
fn cmd_facts(require: &[String], identity: bool) -> Result<u8> {
    if identity {
        write_stdout(&identity_document())?;
        return Ok(EXIT_OK);
    }
    if require.is_empty() {
        write_stdout(&facts_document())?;
        return Ok(EXIT_OK);
    }
    assert_capabilities(require)
}

/// Which build this is (`A1-256`).
///
/// **Not part of `facts.json`.** That document is committed and gated byte for
/// byte against what this build emits, and a digest of the running binary
/// differs on every build, so putting identity in it would make the drift gate
/// unpassable. It is a separate answer to a separate question: `facts` says what
/// a build can do, and this says which build is saying it.
///
/// The distinction is the whole reason this exists. `A1-252` changed the
/// sentence `add` prints on an empty plan and added no rule, command, config
/// key, format, evidence kind or behaviour, so `facts` cannot tell that build
/// from the one before it -- and the ninth pilot spent five weeks quoting the
/// older sentence into a friction log as current (pre-public adoption evidence,
/// second pass).
///
/// A field nothing could learn is absent rather than empty: a binary built from
/// a tarball has no commit to name, and saying so beats naming a guess.
fn identity_document() -> String {
    identity_json(&tool_identity())
}

/// The same document as a value, for the surface that hands it over rather than
/// printing it.
///
/// The MCP server is given this rather than a string it would have to parse:
/// a round trip through text has a failure mode, and its only honest answer
/// would be an identity document with nothing in it -- which is worse than no
/// tool, since an agent asked *which build* and would be told nothing while
/// believing it had been told something (`A1-259`, audit).
fn identity_value() -> serde_json::Value {
    identity_object(&tool_identity())
}

/// This build's identity, computed once for both surfaces that report it.
///
/// `attest` embeds it in a provenance document and `facts --identity` prints it
/// on its own. Two call sites hashing "the facts document" independently is how
/// they would come to disagree about one build, which is the defect `A1-154`
/// fixed for the digest's *input* and this keeps fixed for its callers
/// (`A1-256`, audit).
fn tool_identity() -> aplan_attest::ToolIdentity {
    // Hashed exactly as `aplan facts` writes it, trailing newline and all, so an
    // auditor reproduces the number with `aplan facts | sha256sum` rather than
    // with a byte-fiddling variant of it (`A1-154`).
    aplan_attest::identify(&facts_document())
}

/// The identity as a document, as a function of the identity alone.
///
/// Separate from reading it so the "absent rather than empty" rule can be
/// tested: on a build from a checkout every field is populated, so a test
/// against the running binary passes whether the rule is applied or not
/// (`A1-256`, found by mutating it).
fn identity_json(id: &aplan_attest::ToolIdentity) -> String {
    format!(
        "{}\n",
        serde_json::to_string_pretty(&identity_object(id)).unwrap_or_default()
    )
}

/// The identity document itself, before anyone decides how to write it.
///
/// One builder for the printed form and the served one, so the two surfaces
/// cannot describe one build differently -- the same reason `tool_identity`
/// above is one function (`A1-259`).
fn identity_object(id: &aplan_attest::ToolIdentity) -> serde_json::Value {
    let mut doc = serde_json::json!({
        "schema": "aplan/tool-identity/v1",
        "version": id.version,
        "facts_sha256": id.facts_sha256,
    });
    if let Some(map) = doc.as_object_mut() {
        for (key, value) in [("source", &id.source), ("binary_sha256", &id.binary_sha256)] {
            if !value.is_empty() {
                map.insert(key.to_string(), serde_json::json!(value));
            }
        }
    }
    doc
}

/// The bytes `aplan facts` writes.
///
/// One definition, because `attest` digests this document as the build's
/// capability contract: hashing anything but what the command emits would give
/// an auditor a number their own `aplan facts | sha256sum` cannot reproduce
/// (`A1-154`).
fn facts_document() -> String {
    format!("{}\n", facts())
}

/// `aplan facts --require`, so a consumer staging a binary asserts what it needs
/// instead of grepping the JSON for it.
///
/// `A1-060` published the fact and stopped there, and the first adopter wrote
/// thirty lines of shell to use it: `aplan facts | grep -q '"key"'`, once per
/// key, plus a separate probe for whether the field exists at all. That is a JSON
/// contract matched with a substring search, in every repository that stages a
/// binary (`A1-082`).
///
/// **The two failure modes stay distinguishable, which is the point.** A build
/// that answers "no" exits 1 and names every capability it lacks. A build too old
/// to be asked does not have this flag, so `clap` refuses the invocation and
/// exits 2: the adopter's first draft conflated those, and told an operator a
/// binary lacked a key it supports.
/// One place a `--require` capability can come from, in `aplan facts`.
///
/// The list exists so the flag's help and the set it validates against cannot
/// drift (`A1-093`). `--require` once accepted output formats and evidence kinds
/// while its help named only commands, so an adopter asserting `--require test`
/// was doing something supported and undocumented, and one reading the help would
/// not have known to try. Adding a sixth source here fails
/// `the_require_help_names_every_source_it_accepts` until the help names it too.
struct CapabilitySource {
    /// The key in the `facts` document.
    key: &'static str,
    /// The field to read from each element, or `None` when the array holds
    /// strings directly.
    field: Option<&'static str>,
    /// The words the flag's help must use for this source, so a reader learns it
    /// is assertable.
    help_word: &'static str,
}

const CAPABILITY_SOURCES: &[CapabilitySource] = &[
    CapabilitySource {
        key: "config_keys",
        field: None,
        help_word: "config key",
    },
    CapabilitySource {
        key: "formats",
        field: None,
        help_word: "output format",
    },
    CapabilitySource {
        key: "evidence_kinds",
        field: None,
        help_word: "evidence kind",
    },
    CapabilitySource {
        key: "rules",
        field: Some("code"),
        help_word: "rule code",
    },
    CapabilitySource {
        key: "commands",
        field: Some("name"),
        help_word: "command",
    },
    CapabilitySource {
        key: "behaviours",
        field: Some("name"),
        help_word: "behaviour",
    },
    CapabilitySource {
        key: "mcp_tools",
        field: Some("name"),
        help_word: "MCP tool",
    },
];

/// The tool names `aplan mcp`'s help must contain, one per tool the server exposes.
///
/// Short form (`ready`, not `aplan_ready`), which is how the help reads and how a
/// person refers to them. Exposed for the same reason as
/// [`capability_help_words`]: the help is a compile-time doc comment and cannot be
/// *derived* from [`aplan_mcp::TOOLS`], so a gate holding the two together is the
/// achievable half (`A1-119`).
#[must_use]
pub fn mcp_tool_help_words() -> Vec<&'static str> {
    aplan_mcp::TOOLS
        .iter()
        .map(|t| t.name.strip_prefix("aplan_").unwrap_or(t.name))
        .collect()
}

/// The words `--require`'s help must contain, one per source it validates against.
///
/// Exposed so the docs gate can hold the help to the list rather than to a
/// reviewer's memory (`A1-093`).
#[must_use]
pub fn capability_help_words() -> Vec<&'static str> {
    CAPABILITY_SOURCES.iter().map(|s| s.help_word).collect()
}

/// Every capability name a `facts` document advertises, from the sources listed
/// in `CAPABILITY_SOURCES` (private: a consumer reads the names, not the table).
///
/// Takes the document rather than reading this build's own, so a test can ask what
/// an *older* build advertised. That is the question `--require` exists to answer
/// and the one `A1-099` claimed without checking: a build predating a named
/// behaviour has no `behaviours` key, the source is skipped, and the name is
/// reported unadvertised.
#[must_use]
pub fn advertised_names(doc: &serde_json::Value) -> std::collections::BTreeSet<String> {
    let mut advertised = std::collections::BTreeSet::new();
    for source in CAPABILITY_SOURCES {
        let Some(items) = doc.get(source.key).and_then(|v| v.as_array()) else {
            continue;
        };
        advertised.extend(items.iter().filter_map(|v| match source.field {
            None => v.as_str().map(str::to_string),
            Some(f) => v.get(f).and_then(|x| x.as_str()).map(str::to_string),
        }));
    }
    advertised
}

fn assert_capabilities(require: &[String]) -> Result<u8> {
    // A blank name is not a question, and reporting it as absent would print an
    // empty bullet and exit 1, which reads as a real answer.
    if require.iter().any(|r| r.trim().is_empty()) {
        anyhow::bail!("`--require` needs a capability name; one was empty");
    }
    // One capability named twice is one question. Asked four times now across
    // this codebase, and answered the same way each time: `blocked_by`,
    // `APLAN026`'s tags, `drop`'s ids, and here.
    let mut seen = std::collections::BTreeSet::new();
    let require: Vec<&String> = require.iter().filter(|r| seen.insert(*r)).collect();

    let doc: serde_json::Value =
        serde_json::from_str(&facts()).context("this build's own facts are not valid JSON")?;

    let advertised = advertised_names(&doc);

    // Every missing one, not the first: an operator fixing them one build at a
    // time is the slow version of this command's whole purpose.
    let missing: Vec<&String> = require
        .iter()
        .copied()
        .filter(|r| !advertised.contains(r.as_str()))
        .collect();
    if missing.is_empty() {
        write_stdout(&format!(
            "aplan {} advertises every required capability ({} checked)\n",
            env!("CARGO_PKG_VERSION"),
            require.len()
        ))?;
        return Ok(EXIT_OK);
    }
    let mut out = anstream::stderr().lock();
    // No pluralised noun: this message is read at one missing capability as
    // often as at five, and `capabilit(ies)` reads worse than either.
    writeln!(
        out,
        "aplan {} does not advertise:",
        env!("CARGO_PKG_VERSION")
    )?;
    for m in &missing {
        writeln!(out, "  {m}")?;
    }
    writeln!(
        out,
        "{}",
        aplan_output::fold::fold(
            &format!(
                "It advertises {} others; `aplan facts` lists them. A build that predates \
                 `--require` refuses the flag instead, and exits 2.",
                advertised.len()
            ),
            aplan_output::fold::err_width(),
            0
        )
    )?;
    Ok(EXIT_VIOLATIONS)
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
    // Named behaviours, so a change that widens what an existing name accepts is
    // assertable rather than a release note (`A1-099`).
    let behaviours: Vec<_> = aplan_core::behaviour::ALL
        .iter()
        .map(|b| json!({ "name": b.name, "what": b.what }))
        .collect();
    let evidence_kinds: Vec<_> = aplan_core::EvidenceKind::ALL
        .iter()
        .map(|k| k.as_str())
        .collect();
    // Subcommands, introspected from clap so the list cannot drift from the CLI.
    let commands: Vec<_> = Cli::command()
        .get_subcommands()
        .map(|c| {
            // The formats *this command* accepts, from the same table its own
            // refusal reads. `formats` at the top level answers "what does this
            // build know how to render", which is true of no single command;
            // this answers the question a consumer actually has before it
            // invokes one (`A1-184`).
            let surface = surface_of(c.get_name());
            let formats: Vec<&str> = surface
                .map(|s| s.formats().iter().map(|f| f.as_str()).collect())
                .unwrap_or_default();
            // Which of them it emits when `--format` is not passed. Trivial
            // while every command defaulted to `human`, and a real question
            // since `graph` began defaulting to `mermaid` (`A1-193`): a
            // consumer that omits the flag cannot otherwise tell what it gets.
            json!({
                "name": c.get_name(),
                "about": c.get_about().map(ToString::to_string),
                "formats": formats,
                "default_format": surface.map(|s| s.default_format().as_str()),
            })
        })
        .collect();

    // The agent surface, derived from `aplan_mcp::TOOLS` for the same reason the
    // commands are introspected from clap: `A1-119` found the `mcp` help naming
    // four tools where the server exposed six, and a surface an adopter cannot
    // assert is one they discover by trying it (`A1-082`).
    let mcp_tools: Vec<_> = aplan_mcp::TOOLS
        .iter()
        .map(|t| json!({ "name": t.name, "description": t.description }))
        .collect();

    // What this build *accepts*, not what it is called. A version string cannot
    // answer "does this binary know `test.covers`?", and that is the question an
    // adopter staging a build actually has (`A1-060`).
    let config_keys = aplan_config::accepted_keys();

    let value = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "schema_version": "v1",
        "counts": {
            "rules": rules.len(),
            "behaviours": behaviours.len(),
            "formats": formats.len(),
            "commands": commands.len(),
            "evidence_kinds": evidence_kinds.len(),
            "config_keys": config_keys.len(),
            "mcp_tools": mcp_tools.len(),
        },
        "exit_codes": {
            "clean": EXIT_OK,
            "error": EXIT_VIOLATIONS,
            "usage": EXIT_USAGE,
            "internal": EXIT_INTERNAL,
        },
        "rules": rules,
        "behaviours": behaviours,
        "formats": formats,
        "commands": commands,
        "evidence_kinds": evidence_kinds,
        "config_keys": config_keys,
        "mcp_tools": mcp_tools,
    });
    serde_json::to_string_pretty(&value).expect("facts are serialisable")
}

/// How many recent commits `aplan paths` samples by default.
///
/// Measured across five repositories before it was chosen (`A1-151`): at 30 the
/// answer swings with a quiet week -- this repository's count of tasks over 40%
/// doubles -- and at 300 the ranking is the same as at 100 while the absolute
/// shares drift down. 100 is where the ordering settles and the sample still
/// describes the repository as it is now.
const DEFAULT_PATH_SAMPLE: usize = 100;

/// Reject a format this command does not take, naming the ones it does.
///
/// `Format::from_str` lists every rendering the *tool* has, which is the wrong
/// list for a command that takes two of them: `aplan roadmap --format markdwn`
/// was answered with "expected human, json, agent, github, sarif" -- three
/// renderings that command refuses, and not the one the reader was reaching for.
/// Eight commands restrict the set and every one of them said it twice, once
/// wrongly (`A1-021`'s audit).
///
/// `command` is the whole invocation (`aplan paths`) rather than the subcommand
/// alone: a backticked `aplan {..}` in a message literal is a template, and the
/// gate that checks every invocation the tool names actually parses reads it as
/// one.
/// Serialise `value` as this tool's JSON, under the `schema` key every other
/// machine-readable output carries.
///
/// The envelope is the CLI's, the payload is the value: `aplan ready --format
/// json` minus its `schema` key is byte-for-byte what `aplan_ready` returns over
/// MCP, which is what makes the two surfaces checkable against each other
/// (`A1-182`).
fn write_json<T: serde::Serialize>(value: &T, schema: &str) -> Result<u8> {
    let mut doc = serde_json::to_value(value)?;
    // An `if let` here would print a document with no `schema` key when the
    // value serialised to something other than an object, which a consumer
    // reads as a build too old to carry one. Every machine format this tool
    // emits is an object, so the other case is a mistake, not a shape.
    let map = doc
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("`{schema}` did not serialise to a JSON object"))?;
    map.insert("schema".into(), serde_json::Value::String(schema.into()));
    write_stdout(&format!("{}\n", serde_json::to_string_pretty(&doc)?))?;
    Ok(EXIT_OK)
}

/// The ids a selector admits, with each task's cut resolved once.
///
/// Computed over the *whole* plan and used to filter what is printed, never to
/// decide what exists: a task whose blocker is outside the subset is still
/// blocked, and filtering first would answer that it is ready (`A1-214`).
fn selected_ids(
    plan: &aplan_core::load::Plan,
    selector: &aplan_core::select::Selector,
) -> Result<std::collections::BTreeSet<String>> {
    // An empty selector is the whole plan, and answering that costs a graph, a
    // roadmap and a ready ranking. Every command that can select pays it on
    // every run otherwise, including the runs where nobody selected anything.
    if selector.is_empty() {
        return Ok(plan.tasks.iter().map(|lt| lt.task.id.clone()).collect());
    }
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    refuse_a_release_as_a_cut(plan, &graph, &selector.cuts)?;
    Ok(graph
        .task_rows()
        .tasks
        .iter()
        .filter(|row| {
            graph
                .get(&row.id)
                .is_some_and(|t| selector.matches(t, row.cut.as_deref(), row.milestone.as_deref()))
        })
        .map(|row| row.id.clone())
        .collect())
}

/// Refuse a selection on a command that gates rather than reports.
///
/// **The one decision here that can produce a false green.** `check` and
/// `verify` answer a yes-or-no question about a plan, and one that examined a
/// subset would report a clean plan while ignoring part of it -- in CI, under a
/// flag somebody added to a script months earlier. Qualifying the verdict was
/// the alternative, and a qualified green is still green to an exit code
/// (`A1-214`).
/// A `--cut` that names a **release** is refused, and told what it closes.
///
/// ADR-0022: a release is a checkpoint over cuts and claims no task. Before
/// `A1-240` the derived `cut` field answered with the release, so selecting by
/// its label returned its cuts' work under the wrong name; after, the same
/// selection matched nothing at all and said nothing about why -- while `graph`,
/// which resolves `--cut` through the milestone's members instead, went on
/// returning them. Two commands, one flag, opposite answers, which is the defect
/// `A1-214` exists to remove (`A1-240`, audit).
///
/// Refused rather than expanded to the cuts it closes: `--cut` names a cut, the
/// rows it would return report a different cut than the one asked for, and the
/// message is one keystroke from the answer.
fn refuse_a_release_as_a_cut(
    plan: &aplan_core::load::Plan,
    graph: &Graph<'_>,
    cuts: &[String],
) -> Result<()> {
    let roadmap = graph.roadmap();
    for named in cuts {
        let Ok(id) = aplan_interop::cut::resolve_id(plan, named) else {
            continue;
        };
        let Some(found) = roadmap.cuts.iter().find(|c| c.milestone_id == id) else {
            continue;
        };
        if !found.release {
            continue;
        }
        let closes: Vec<String> = found
            .closes
            .iter()
            .map(|closed| {
                let label = roadmap
                    .cuts
                    .iter()
                    .find(|other| &other.milestone_id == closed)
                    .map_or("", |other| other.version.as_str());
                format!("`{}`", if label.is_empty() { closed } else { label })
            })
            .collect();
        anyhow::bail!(
            "`--cut {named}` names a release, and a release holds no work of its own: it is a \
             checkpoint over the cuts it closes, {}. Select one of those, or name several to \
             widen",
            if closes.is_empty() {
                "and it closes none".to_string()
            } else {
                and_list(&closes)
            }
        );
    }
    Ok(())
}

fn refuse_selection_on_a_gate(command: &str, select: &SelectArgs) -> Result<()> {
    let selector = select.selector()?;
    if selector.is_empty() {
        return Ok(());
    }
    let invocation = format!("aplan {command}");
    anyhow::bail!(
        "`{invocation}` is a gate, so it does not take a selection: a verdict over {} would \
         report a clean plan while ignoring the rest of it, and an exit code cannot be \
         qualified. Use `aplan tasks --format json` with the same selection to inspect a \
         subset, or drop the flag to gate the whole plan",
        and_list(&selector.describe())
    )
}

/// The one selector, flattened into every command where a subset is meaningful
/// (`A1-214`).
///
/// **One definition, not one grammar.** `--select cut:v0.9` would be a second
/// spelling beside the `--cut` five commands already take, which is the drift
/// this exists to remove; flags stay discoverable in `--help`, and "parsed in
/// one place" is satisfied by there being one `SelectArgs` rather than by there
/// being a parser.
///
/// **`phase` is deliberately not here.** `A1-187` is weighing retiring it into a
/// tag family, and a `--phase` on nine commands would turn that from one field
/// into a break across the whole surface. `--tag` covers it the moment it
/// becomes a tag.
#[derive(Debug, Clone, clap::Args)]
pub struct SelectArgs {
    /// Only this cut: its milestone id (`M-v0.9`) or the label it reports itself
    /// under (`v0.9`). Repeatable, and any of them matches.
    #[arg(long, value_name = "CUT")]
    pub cut: Vec<String>,
    /// Only tasks carrying this tag. Repeatable, and any of them matches.
    #[arg(long, value_name = "TAG")]
    pub tag: Vec<String>,
    /// Only tasks with this owner. Repeatable, and any of them matches.
    #[arg(long, value_name = "OWNER")]
    pub owner: Vec<String>,
    /// Only tasks with this status. Repeatable, and any of them matches.
    #[arg(long, value_name = "STATUS")]
    pub status: Vec<String>,
}

impl SelectArgs {
    /// The subset these flags name.
    ///
    /// # Errors
    ///
    /// When a `--status` is not one the model has, listed so the reader does not
    /// have to find them.
    pub fn selector(&self) -> Result<aplan_core::select::Selector> {
        let mut statuses = Vec::new();
        for word in &self.status {
            let parsed = aplan_core::Status::ALL
                .iter()
                .copied()
                .find(|s| s.to_string() == *word)
                .ok_or_else(|| {
                    let names: Vec<String> = aplan_core::Status::ALL
                        .iter()
                        .map(ToString::to_string)
                        .collect();
                    anyhow::anyhow!(
                        "`--status {word}` is not a status: {}",
                        list_of_str(&names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>())
                    )
                })?;
            statuses.push(parsed);
        }
        Ok(aplan_core::select::Selector {
            cuts: self.cut.clone(),
            tags: self.tag.clone(),
            owners: self.owner.clone(),
            statuses,
        })
    }
}

/// What each command prints, and therefore which `--format` values it takes.
///
/// One table, read by the refusal and by `aplan facts`, so a consumer can ask
/// what a command accepts before invoking it and get the same answer the
/// command itself would give. It replaced ten hand-written allow-lists: nine
/// `narrow_format` calls carrying a literal list, and a tenth written longhand
/// inside `show`.
///
/// Keyed by the `clap` subcommand name, and gated against `clap`'s own tree by
/// `every_formatted_command_declares_a_surface`, so a new command with a
/// `--format` cannot quietly accept everything.
pub const COMMAND_SURFACES: &[(&str, Surface, Option<&str>)] = &[
    ("check", Surface::Diagnostics, None),
    ("verify", Surface::Diagnostics, None),
    // The third column is what this command prints, when the surface's own noun
    // undersells it. `show` renders *one task*, and saying so is why a reader
    // who asked for `sarif` understands the refusal rather than only receiving
    // it (`A1-118`). It is prose, not an allow-list: the formats still come from
    // the surface, so this cannot drift into a second answer about what is
    // accepted.
    ("show", Surface::Report, Some("one task")),
    (
        "ready",
        Surface::Report,
        Some("the tasks that can start now"),
    ),
    (
        "due",
        Surface::Report,
        Some("parked tasks whose revisit has arrived"),
    ),
    ("why", Surface::Report, Some("what a task is waiting on")),
    ("paths", Surface::Report, None),
    ("vacuity", Surface::Report, None),
    ("stale", Surface::Report, None),
    ("check-docs", Surface::Report, None),
    ("summary", Surface::Report, None),
    ("tasks", Surface::Report, None),
    ("critical-path", Surface::Report, None),
    ("graph", Surface::Graph, None),
    ("roadmap", Surface::Document, None),
    ("deprecations", Surface::Document, None),
];

/// Commands whose `--format` is not an output format at all.
///
/// `import --format kep` names the *source* shape to read, a vocabulary with
/// nothing in common with this one. It is held here rather than left out, so the
/// census over `clap`'s tree can tell "not an output surface" from "somebody
/// forgot".
pub const NOT_AN_OUTPUT_FORMAT: &[(&str, &str)] = &[
    (
        "import",
        "`--format` names the source shape to read, not the shape to print",
    ),
    (
        "export",
        "`--format` names the target shape to write, which is the same foreign vocabulary \
         `import` reads and not this one",
    ),
];

/// The surface a command prints, if it prints a formatted one.
#[must_use]
pub fn surface_of(command: &str) -> Option<Surface> {
    COMMAND_SURFACES
        .iter()
        .find(|(name, ..)| *name == command)
        .map(|(_, surface, _)| *surface)
}

/// What a command prints, in the words a refusal should use.
fn prints(command: &str, surface: Surface) -> &'static str {
    COMMAND_SURFACES
        .iter()
        .find(|(name, ..)| *name == command)
        .and_then(|(.., noun)| *noun)
        .unwrap_or_else(|| surface.describe())
}

/// Parse `value` as a format this command can actually emit.
///
/// One gate for two failures that used to be separate: a name the tool does not
/// know at all, and a name it knows but this command cannot render. The second
/// was nine literal lists and one longhand `match`; it is the surface now.
fn format_for(command: &str, value: &str) -> Result<Format> {
    // The invocation is built rather than written inside the literal, so the
    // consumer-facing audit -- which scans this source for backticked `aplan
    // ...` and checks each one parses -- does not read a format placeholder as
    // a subcommand name.
    let invocation = format!("aplan {command}");
    let Some(surface) = surface_of(command) else {
        anyhow::bail!("`{invocation}` does not take `--format`");
    };
    // A name the tool does not know and a name this command cannot render are
    // one question to the reader: "what may I pass here?". Parsing first and
    // letting the failure through would answer a misspelt `markdwn` with the
    // tool's whole vocabulary -- including the four this command refuses, and
    // not the one that was being reached for.
    let known = match value.parse::<Format>() {
        Ok(format) if format.renders(surface) => return Ok(format),
        Ok(format) => Some(format),
        Err(_) => None,
    };

    let names = list_of(&surface.formats());
    // Name what the rejected format *is* for, not only that it is wrong here: a
    // reader who asked for `sarif` wants to know where sarif lives, and the
    // surfaces answer that from the same table (`A1-118`).
    // Where the rejected format *does* live, but only when it is a format at
    // all: a misspelling has no home to point at.
    let where_it_lives = match known {
        None => String::new(),
        Some(format) => {
            let elsewhere: Vec<String> = COMMAND_SURFACES
                .iter()
                .filter(|(_, s, _)| format.renders(*s))
                .map(|(name, ..)| format!("`{name}`"))
                .collect();
            if elsewhere.is_empty() {
                String::new()
            } else {
                format!(
                    "; `{}` renders {}, which {} {}",
                    format.as_str(),
                    format
                        .surfaces()
                        .first()
                        .map_or("something else", |s| s.describe()),
                    and_list(&elsewhere),
                    if elsewhere.len() == 1 { "does" } else { "do" }
                )
            }
        }
    };
    anyhow::bail!(
        "`{invocation}` renders {}, so it takes --format {names}, not `{value}`{where_it_lives}",
        prints(command, surface),
    )
}

/// `a`, `b` and `c`, for a list of things that all apply.
fn and_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `a`, `b` or `c`.
fn list_of(formats: &[Format]) -> String {
    let names: Vec<String> = formats
        .iter()
        .map(|f| format!("`{}`", f.as_str()))
        .collect();
    list_of_str(&names)
}

fn list_of_str(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// `aplan vacuity`, what each rule could actually have failed on (`A1-114`).
///
/// Hermetic: it reads the plan and never git, because what it measures is
/// whether a rule's predicate had anything to bind against, which is a property
/// of the task file. Advisory for the reason `paths` is: a number nobody can act
/// on yet must not stop a build.
fn cmd_vacuity(root: &std::path::Path, format: &str, select: &SelectArgs) -> Result<u8> {
    let format = format_for("vacuity", format)?;
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let selector = select.selector()?;
    let keep = selected_ids(&plan, &selector)?;
    let tasks: Vec<&aplan_core::task::Task> = plan
        .tasks
        .iter()
        .map(|lt| &lt.task)
        .filter(|t| keep.contains(&t.id))
        .collect();
    // The configuration too, so a rule it switched off is reported as not run
    // rather than as having checked its subjects (`A1-269`).
    let (config, config_dir) = config_at(root)?;
    let opts = resolve_check_options(&config, config_dir.as_deref(), false, None, false);
    let mut off = aplan_core::vacuity::switched_off(&plan, &opts);
    off.extend(resolver_side_off(&config, &tasks));
    let report = aplan_core::vacuity::of_with(&tasks, &off);
    render_tolerant(|| aplan_output::render_vacuity(&report, format))?;
    // Always `0`: a measurement is not a verdict (`A1-151`, `A1-020`).
    Ok(EXIT_OK)
}

/// Rules a *resolver's* configuration leaves nothing for (`A1-274`).
///
/// `A1-269` measures the switches in `validate::Options` by running the rules
/// again with each one on. That reaches every hermetic rule and none of the
/// evidence ones: they are produced by `verify`, which opens the repository, so
/// the same trick would cost this report its hermeticity.
///
/// **So these are stated rather than measured, and what stops the statement
/// drifting is a test.** Each line below asserts "this rule cannot speak in this
/// configuration", and `every_resolver_gated_rule_is_really_silent` puts a plan
/// in that configuration and fails if the rule speaks. A claim about a rule that
/// stops being true fails there, which is the property the measured half gets
/// for free and this half has to buy.
///
/// Reads configuration and cited test *names* only: no subprocess and no
/// repository, so `vacuity` is still hermetic.
fn resolver_side_off(
    config: &aplan_config::Config,
    tasks: &[&aplan_core::task::Task],
) -> std::collections::BTreeMap<&'static str, &'static str> {
    let mut out = std::collections::BTreeMap::new();
    // Every `test` citation in the plan, which is what a runner would have to
    // claim locality over.
    let cited: Vec<String> = tasks
        .iter()
        .flat_map(|t| t.evidence.iter())
        .filter(|e| e.kind == aplan_core::EvidenceKind::Test)
        .map(|e| e.reference.clone())
        .collect();

    match &config.test {
        // No runner at all: nothing resolves a `test` citation, so every rule
        // over one is inert.
        None => {
            // One `insert` per code rather than a loop over an array: the audit
            // that asks which crate *raises* a rule reads `insert(code::X,` as
            // the read-only idiom it is, and a code buried in an array literal
            // reads as raising one (`A1-274`).
            out.insert(aplan_core::diagnostic::code::TEST_UNRESOLVED, "test");
            out.insert(aplan_core::diagnostic::code::TEST_FAILED, "test");
            out.insert(aplan_core::diagnostic::code::TEST_IRRELEVANT, "test");
            out.insert(aplan_core::diagnostic::code::RUNNER_LISTS_NOTHING, "test");
            out.insert(aplan_core::diagnostic::code::RUN_PROVES_NOTHING, "test");
        }
        // A runner that claims no locality cannot demonstrate a mismatch, which
        // is `APLAN025`'s own early return. `p00002` is the case: its single
        // runner declares nothing, and declaring something would be false --
        // those tests genuinely read the whole tree.
        Some(runners) => {
            if !aplan_evidence::claims_locality(runners, &cited) {
                out.insert(aplan_core::diagnostic::code::TEST_IRRELEVANT, "test.covers");
            }
        }
    }
    if config.url.is_none() {
        out.insert(aplan_core::diagnostic::code::URL_UNREACHABLE, "url.check");
    }
    out
}

/// `aplan deprecations`, the runway a plan declares (`A1-021`).
///
/// Hermetic: it reads the plan and never git, so it sits beside `roadmap` rather
/// than in the `verify` family. Advisory for the reason `paths` is: across the
/// six repositories this tool has been pointed at, none declares a runway, and a
/// rule firing on nobody's plan is a rule nobody can act on. Where somebody does
/// declare one, every shape this names is decidable from the plan alone.
fn cmd_deprecations(root: &std::path::Path, format: &str) -> Result<u8> {
    // `markdown` is parsed here rather than added to `Format`, exactly as
    // `roadmap` does it: a table to publish is this command's artifact, not a
    // rendering every command should suddenly accept.
    let format = format_for("deprecations", format)?;
    let markdown = format == Format::Markdown;
    // The markdown renderer builds the document itself, so the shared renderer
    // is handed `human` behind it. One `Format` throughout now: this used to be
    // a string compared against `"markdown"` and then parsed again.
    let format = if markdown { Format::Human } else { format };
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let tasks: Vec<&aplan_core::task::Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = aplan_core::Graph::new(tasks.iter().copied());
    let report = aplan_core::deprecation::of(&tasks, &graph.roadmap());

    if markdown {
        write_stdout(&aplan_output::deprecations_markdown(&report))?;
        return Ok(EXIT_OK);
    }
    render_tolerant(|| aplan_output::render_deprecations(&report, format))?;
    // Always `0`: a report is not a verdict (`A1-151` and `A1-020` before it).
    Ok(EXIT_OK)
}

/// `aplan paths`, breadth measured against history rather than the file tree
/// (`A1-151`).
fn cmd_paths(
    root: &std::path::Path,
    repo: &std::path::Path,
    sample: usize,
    format: &str,
    select: &SelectArgs,
) -> Result<u8> {
    let format = format_for("paths", format)?;
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    // Milestones carry no `paths` and no work, exactly as every other rollup
    // reads them.
    let selector = select.selector()?;
    let keep = selected_ids(&plan, &selector)?;
    let tasks: Vec<&aplan_core::task::Task> = plan
        .tasks
        .iter()
        .map(|lt| &lt.task)
        .filter(|t| !t.is_milestone())
        .filter(|t| keep.contains(&t.id))
        .collect();
    let with_paths: Vec<(String, Vec<String>)> = tasks
        .iter()
        .filter(|t| !t.paths.is_empty())
        .map(|t| (t.id.clone(), t.paths.clone()))
        .collect();
    let without_paths = tasks.len() - with_paths.len();

    // A sample of nothing measures nothing, and the report it produces is a
    // full table of confident zeroes. Refused at the flag rather than explained
    // in the output (`A1-151`'s audit).
    if sample == 0 {
        anyhow::bail!("`--sample 0` measures nothing; ask for at least one commit");
    }

    let breadth = aplan_evidence::commit_breadth(repo, &with_paths, sample)
        .context("reading the repository's history")?;

    let by_id: std::collections::BTreeMap<&str, &aplan_core::task::Task> =
        tasks.iter().map(|t| (t.id.as_str(), *t)).collect();
    let mut rows: Vec<aplan_output::PathBreadth> = breadth
        .counts
        .iter()
        .map(|(id, hits)| {
            let task = by_id.get(id.as_str());
            aplan_output::PathBreadth {
                id: id.clone(),
                title: task.map(|t| t.title.clone()).unwrap_or_default(),
                hits: *hits,
                // Integer percent of what was *sampled*, never of what was asked
                // for: a repository shorter than the sample is common, and the
                // other denominator would understate every row in it.
                percent: u32::try_from(hits * 100 / breadth.sampled.max(1)).unwrap_or(u32::MAX),
                first_path: task
                    .and_then(|t| t.paths.first().cloned())
                    .unwrap_or_default(),
            }
        })
        .collect();
    // By hits, not by the rounded percent: two rows that both print `<1%` still
    // have an order, and the truncated number would lose it.
    rows.sort_by(|a, b| b.hits.cmp(&a.hits).then(a.id.cmp(&b.id)));

    let report = aplan_output::PathsReport {
        sampled: breadth.sampled,
        asked_for: sample,
        tasks: rows,
        without_paths,
        unmeasurable: breadth.unmeasurable.len(),
    };
    render_tolerant(|| aplan_output::render_paths(&report, format))?;
    // Always `0`: a measurement is not a verdict (`A1-151`, and `A1-020` before
    // it).
    Ok(EXIT_OK)
}

/// `Command::Stale`'s arm, lifted out so `dispatch` stays inside its line limit.
fn stale(
    cli_root: Option<PathBuf>,
    older_than: Option<u32>,
    as_of: Option<&str>,
    repo: &std::path::Path,
    format: &str,
    select: &SelectArgs,
) -> Result<u8> {
    cmd_stale(
        &resolve_root(cli_root)?,
        repo,
        older_than,
        as_of,
        format,
        select,
    )
}

/// `aplan stale`, the question a tracker cannot ask (`A1-020`).
///
/// In the `verify` family because it reads git, and outside `check` for the same
/// reason: the hermetic path may not open a repository, and the structural test
/// in `aplan-e2e` holds that line by forbidding `git2` in the crates `check`
/// runs through.
fn cmd_stale(
    root: &std::path::Path,
    repo: &std::path::Path,
    older_than: Option<u32>,
    as_of: Option<&str>,
    format: &str,
    select: &SelectArgs,
) -> Result<u8> {
    let format = format_for("stale", format)?;
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let (config, _) = config_at(repo)?;
    let window = i64::from(
        older_than
            .or(config.stale_after_days)
            .unwrap_or(DEFAULT_STALE_DAYS),
    );

    let selector = select.selector()?;
    let keep = selected_ids(&plan, &selector)?;
    // Paths as git names them: relative to the repository, whatever the caller
    // typed to reach the plan.
    let repo_root = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let tasks: Vec<&aplan_core::load::LoadedTask> = plan
        .tasks
        .iter()
        // Live work only, and a milestone is not work: a cut sits `todo` by
        // design until its members close, so reporting one as neglected would
        // put a permanent row at the top of a report about neglect. Every other
        // rollup here draws the same line (`A1-020`).
        //
        // **Live includes parked.** `Status::Blocked` is *explicitly parked for a
        // reason outside the dependency graph*, and it is live work: somebody
        // means to do it. `ready` excludes it correctly -- it is not startable --
        // and this excluded it too, so between them the only work nothing
        // tracked was the work nobody was doing. A park is a decision with a
        // half-life, and how long it has held is the question this command
        // already answers for everything else (`A1-275`).
        .filter(|lt| {
            (lt.task.status.is_workable() || lt.task.status == aplan_core::Status::Blocked)
                && !lt.task.is_milestone()
        })
        .filter(|lt| keep.contains(&lt.task.id))
        .collect();
    let relative: Vec<std::path::PathBuf> = tasks
        .iter()
        .map(|lt| {
            std::fs::canonicalize(&lt.path)
                .ok()
                .and_then(|p| {
                    p.strip_prefix(&repo_root)
                        .ok()
                        .map(std::path::Path::to_path_buf)
                })
                .unwrap_or_else(|| lt.path.clone())
        })
        .collect();

    let (touched, head_time) = aplan_evidence::last_touched(repo, &relative)
        .context("reading the repository's history")?;
    // A named date replaces `HEAD`'s commit time as the point ages are measured
    // from. Refused when it precedes that commit, because every age would then
    // be negative and the report would say nothing has sat -- the false green
    // this command's own audit was about (`A1-020`, `A1-162`).
    let now = match as_of {
        None => head_time,
        Some(date) => {
            // The *end* of the named day, not its midnight. A date names a whole
            // day, and an adopter's obvious invocation is `--as-of $(date +%F)`
            // in a weekly review: measured from midnight that is refused on any
            // day the repository saw a commit, which is most of them
            // (`A1-162`'s audit). It also reads right -- a task last touched on
            // the 11th has sat eight days *by* the 19th, not seven.
            let Some(seconds) = aplan_core::validate::iso_date_to_epoch(date).map(|s| s + 86_399)
            else {
                anyhow::bail!("`--as-of {date}` is not an ISO-8601 date (YYYY-MM-DD)");
            };
            if seconds < head_time {
                anyhow::bail!(
                    "`--as-of {date}` is before `HEAD`'s own commit, so every task would read as \
                     not yet written. Name a date at or after the last commit."
                );
            }
            seconds
        }
    };

    let graph = Graph::new(plan.tasks.iter().map(|lt| &lt.task));
    let ready: std::collections::BTreeSet<&str> =
        graph.ready().iter().map(|t| t.id.as_str()).collect();

    let mut stale = Vec::new();
    let (mut fresh, mut untracked) = (0usize, 0usize);
    for (lt, path) in tasks.iter().zip(&relative) {
        let Some(seconds) = touched.get(path) else {
            untracked += 1;
            continue;
        };
        // Rounded down: a task one hour past the window has not sat for a day.
        let idle_days = (now - seconds).max(0) / 86_400;
        if idle_days <= window {
            fresh += 1;
            continue;
        }
        stale.push(aplan_output::StaleTask {
            id: lt.task.id.clone(),
            title: lt.task.title.clone(),
            status: lt.task.status.to_string(),
            idle_days,
            ready: ready.contains(lt.task.id.as_str()),
        });
    }
    // Readiest first, then idlest, then by id: what a reader can pick up now,
    // longest-neglected first, and deterministic among equals.
    stale.sort_by(|a, b| {
        b.ready
            .cmp(&a.ready)
            .then(b.idle_days.cmp(&a.idle_days))
            .then(a.id.cmp(&b.id))
    });

    let report = aplan_output::StaleReport {
        as_of: as_of.map(str::to_string),
        window_days: window,
        tasks: stale,
        fresh,
        untracked,
    };
    render_tolerant(|| aplan_output::render_stale(&report, format))?;
    // Always `0`: this is advice, and advice does not fail a build (`A1-020`).
    Ok(EXIT_OK)
}

/// `aplan attest`, the verdict written down (`A1-040`).
///
/// This command *records*; `verify` judges. The verdict is computed by the same
/// path `verify` uses, under the repository's own policy, so there is one answer
/// to "does this plan verify" rather than a second one that only `attest`
/// believes.
fn cmd_attest(
    root: &std::path::Path,
    repo: &std::path::Path,
    attach_note: Option<&str>,
    execute: bool,
) -> Result<u8> {
    let plan = load_checked(root)?;
    let (config, config_dir) = config_at(repo)?;
    let policy = verdict_policy(&config, false, false);
    let check_opts = resolve_check_options(&config, config_dir.as_deref(), false, None, false);
    refuse_absent_external_roots(&check_opts)?;

    let unloadable = load_errors(&plan);
    if !unloadable.is_empty() {
        // Name them. "1 task file(s) failed to load" sent a reader -- and this
        // author, reading a CI log -- looking for a file the message knew and
        // did not say (`A1-151`'s session, found while diagnosing a fixture).
        for diagnostic in &unloadable {
            for line in aplan_output::fold::fold(
                &diagnostic.message,
                aplan_output::fold::err_width().saturating_sub(2),
                2,
            )
            .lines()
            {
                errln!("  {line}");
            }
        }
        anyhow::bail!(
            "{} task file(s) failed to load, so there is no plan to attest; run `aplan check`",
            unloadable.len()
        );
    }
    // The tree first, before a single citation is resolved: a plan read partly
    // from files git has never seen is not the plan anybody else can check, and
    // reporting one of *its* tasks as a verify finding sends the reader to fix
    // the wrong thing (`A1-040`, its audit).
    aplan_attest::refuse_dirty(repo).map_err(anyhow::Error::from)?;
    let mut report = validate(&plan, &check_opts);
    let opts = aplan_evidence::VerifyOptions {
        execute,
        test_runner: config.test.clone(),
        max_unrelated_files: config.evidence.as_ref().and_then(|e| e.max_unrelated_files),
        url_check: config.url.clone(),
        config_file: config_dir
            .as_deref()
            .map(|d| d.join(aplan_config::CONFIG_FILE)),
    };
    let evidence = aplan_evidence::verify(&plan, repo, &opts)
        .context("resolving evidence against the repository")?;
    report.diagnostics.extend(evidence.diagnostics);
    if verdict(&report, &policy) != EXIT_OK {
        errln!(
            "aplan: this plan does not verify ({} finding(s) block under this repository's \
             policy), and an attestation of an unproven plan is worse than none. Run `aplan \
             verify` for what to fix.",
            report.blocking(&policy, aplan_core::Severity::Error)
                + report.blocking(&policy, aplan_core::Severity::Warning)
        );
        return Ok(EXIT_VIOLATIONS);
    }

    // The identity is derived from this build, never described by it: the
    // version it reports, the commit it was built from, the digest of its own
    // facts document, and the digest of the running executable (`A1-154`).
    let tool = tool_identity();
    // `--execute` is what separates "a runner lists this test" from "this test
    // passed", and the document has to say which it is (`A1-040`, its audit).
    //
    // And how much that is worth, which `A1-167` showed is not always the same:
    // a runner whose executions cannot be told from a run that selected nothing
    // is counted here, from the report this run already produced, so the
    // document carries the qualification `verify` printed rather than only the
    // claim (`A1-178`). Counted from the codes rather than read out of the
    // messages: a document assembled by parsing this tool's own prose would
    // break the next time the prose improved.
    let unproven = report
        .diagnostics
        .iter()
        .filter(|d| d.code == aplan_core::diagnostic::code::RUN_PROVES_NOTHING)
        .count();
    let attestation =
        aplan_attest::attest(&plan, repo, tool, execute, unproven).map_err(anyhow::Error::from)?;
    let document = attestation.to_json();
    if let Some(note_ref) = attach_note {
        let oid =
            aplan_attest::attach_note(repo, note_ref, &document).map_err(anyhow::Error::from)?;
        errln!(
            "attached {oid} to {} on `refs/notes/{note_ref}`; read it with `git notes \
             --ref={note_ref} show`",
            attestation.commit
        );
    }
    write_stdout(&document)?;
    Ok(EXIT_OK)
}

/// `aplan why`, what is blocking a task, transitively.
///
/// Lifted out of `dispatch` (`A1-033`), which was carrying forty lines of this
/// command's logic inline while every sibling was one call.
/// `aplan ready`'s arguments.
#[derive(Debug, clap::Args)]
pub struct ReadyArgs {
    /// Say why each task is where it is: what it releases, whether it is on
    /// the critical path, and its effort. The order is derived from the plan,
    /// never declared on a task, so every reason is one you can check.
    #[arg(long)]
    pub explain: bool,
    /// Output format: human (a list) or json. The `json` shape carries the
    /// reasons as fields -- `releases`, `critical`, `prefers_to_follow` --
    /// which `--explain` renders as a sentence, so a consumer can sort by
    /// what it is being told.
    #[arg(long, default_value = "human")]
    pub format: String,
    /// Narrow to a subset of the plan. Selection changes what is shown,
    /// never what is computed: a task whose blocker is outside the subset
    /// is still blocked.
    #[command(flatten)]
    pub select: SelectArgs,
}

/// `aplan due`'s arguments.
#[derive(Debug, clap::Args)]
pub struct DueArgs {
    /// The instant to answer as of, RFC 3339. Defaults to now.
    ///
    /// A scheduler passes the moment it woke, so two runs of one plan at one
    /// instant agree. Without it the system clock is used and the report says
    /// so.
    #[arg(long, value_name = "INSTANT")]
    pub as_of: Option<String>,
    /// Output format: human (a list) or json.
    #[arg(long, default_value = "human")]
    pub format: String,
    #[command(flatten)]
    pub select: SelectArgs,
}

/// `aplan due`, the hand-off a parked task can give a scheduler (`A1-264`).
///
/// The only command that reads a clock. `check`, `ready` and `verify` do not,
/// and this one says which clock it used, for the reason `stale` does: a reader
/// told a task is due has to know due *by when*.
fn cmd_due(root: &std::path::Path, args: &DueArgs) -> Result<u8> {
    let as_of = args.as_of.as_deref();
    let select = &args.select;
    let format = format_for("due", &args.format)?;
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let (now, stated) = match as_of {
        Some(at) => {
            let Some(seconds) = aplan_core::validate::rfc3339_to_epoch(at) else {
                anyhow::bail!(
                    "`--as-of {at}` is not an RFC 3339 instant. Write it as \
                     `YYYY-MM-DDTHH:MM:SSZ`, or with an offset (`+02:00`)."
                );
            };
            (seconds, Some(at.to_string()))
        }
        None => (
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
                .unwrap_or_default(),
            None,
        ),
    };

    let selector = select.selector()?;
    let keep = selected_ids(&plan, &selector)?;
    let mut due = Vec::new();
    let mut waiting = 0usize;
    for lt in &plan.tasks {
        // Parked only. A revisit anywhere else is what `APLAN042` reports, and
        // acting on one here would make this command disagree with that rule.
        if lt.task.status != aplan_core::Status::Blocked || !keep.contains(&lt.task.id) {
            continue;
        }
        let Some(at) = &lt.task.revisit_at else {
            continue;
        };
        let Some(seconds) = aplan_core::validate::rfc3339_to_epoch(at) else {
            continue;
        };
        if seconds > now {
            waiting += 1;
            continue;
        }
        due.push(aplan_output::DueTask {
            id: lt.task.id.clone(),
            title: lt.task.title.clone(),
            revisit_at: at.clone(),
            overdue_seconds: now - seconds,
        });
    }
    // Longest overdue first, then by id: the one that has waited most is the one
    // a scheduler woke for, and equals are ordered so two runs agree.
    due.sort_by(|a, b| {
        b.overdue_seconds
            .cmp(&a.overdue_seconds)
            .then(a.id.cmp(&b.id))
    });

    let report = aplan_output::DueReport {
        as_of: stated,
        tasks: due,
        waiting,
    };
    render_tolerant(|| aplan_output::render_due(&report, format))?;
    Ok(EXIT_OK)
}

fn cmd_why(root: &std::path::Path, id: &str, all: bool, format: &str) -> Result<u8> {
    let format = format_for("why", format)?;
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let walk = aplan_mcp::tools::walk_for(all);
    if format == Format::Json {
        let answer =
            aplan_mcp::tools::why_of(&plan, id, walk).map_err(|e| anyhow::anyhow!("{e}"))?;
        return write_json(&answer, "aplan/why/v1");
    }
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());

    let Some(task) = graph.get(id) else {
        anyhow::bail!("no task `{id}` in this plan");
    };
    let blockers = graph.why_blocked(id, walk);
    let unknown = graph.direct_unknown_blockers(id);
    // A parked task is blocked by a decision, not by an edge. Saying
    // "ready: nothing blocks it" about a task whose status is literally
    // `blocked` is the tool contradicting its own plan.
    let parked = !task.status.is_workable() && !task.status.is_terminal();
    // A cut whose members are all done, which `ready` will not offer and
    // which somebody has to close. What the word "ready" may be used for
    // lives in `unblocked_verdict`.
    let closeable = graph.closeable_milestones().iter().any(|m| m.id == task.id);
    let mut out = String::new();
    if parked {
        // *No field carries it* was true when `A1-175` wrote it and `A1-264`
        // made it false for a task that names a revisit: the condition is still
        // prose, but *when to look again* is now a field, and a reader told
        // otherwise would not go looking for it. The instant is reported as what
        // it is -- when to re-read, never that the wait is over (`A1-264`,
        // audit).
        let waits = match &task.revisit_at {
            Some(at) => format!(
                "What it waits on is in the task body. It names {at} as worth looking again, \
                 which is a time to re-read rather than a claim the wait is over (`aplan due`)"
            ),
            None => "What it waits on is in the task body; no field carries it".to_string(),
        };
        let _ = writeln!(
            out,
            "{}",
            aplan_output::fold::note(&format!(
                "{id} is `blocked`: explicitly parked, not waiting on the graph. {waits}"
            ))
        );
    } else if blockers.is_empty() && unknown.is_empty() {
        out.push_str(&unblocked_verdict(task, closeable));
    }
    if !blockers.is_empty() || !unknown.is_empty() {
        // The id takes the id role and the label takes the heading role, as
        // everywhere else (`A1-296`, audit: this line carried neither).
        let _ = writeln!(
            out,
            "{i}{id}{i:#} {hd}is blocked by:{hd:#}",
            i = aplan_output::style::ID,
            hd = aplan_output::style::HEADING
        );
        let mut folded = false;
        for b in blockers {
            // The title is the trailing column and folds under itself. It did
            // not until `A1-291`: at eighty columns a blocker row ran to 138 and
            // the terminal broke it at column zero, under the id.
            let _ = writeln!(
                out,
                "{}",
                aplan_output::fold::row(
                    &format!(
                        "  {id}{}{id:#} [{st}{}{st:#}]  ",
                        b.id,
                        b.status,
                        id = aplan_output::style::ID,
                        st = aplan_output::style::status(b.status.as_str()),
                    ),
                    &aplan_output::sanitize::visible(&b.title),
                )
            );
            // What the reader is not being shown, and how to see it. A fold that
            // did not say its own size would be a truncation reading as the
            // whole answer (`A1-263`).
            if walk == aplan_core::graph::Walk::CutBounded && b.is_milestone() {
                let behind = graph.folded_behind(&b.id);
                if behind > 0 {
                    folded = true;
                    let _ = writeln!(
                        out,
                        "      {dim}(a cut: {behind} unmet behind it, `--all` lists them){dim:#}",
                        dim = aplan_output::style::DIM
                    );
                }
            }
        }
        for u in unknown {
            let _ = writeln!(
                out,
                "{}",
                aplan_output::fold::row(
                    &format!(
                        "  {id}{u}{id:#} [{w}unknown{w:#}]  ",
                        id = aplan_output::style::ID,
                        w = aplan_output::style::WARNING,
                    ),
                    "referenced but not defined",
                )
            );
        }
        // Exactly when the answer is partial, which is the only time the word
        // changes what the reader should believe. A bounded walk that folded
        // nothing *is* the complete answer, and labelling it would put a caveat
        // on two lines that have none -- noise on the common case, and the
        // shape a reader learns to skip. The JSON carries `traversal`
        // unconditionally, because a consumer diffs fields rather than reading
        // for emphasis (`A1-263`).
        if folded {
            let _ = writeln!(out, "traversal: cut-bounded");
        }
    }
    write_stdout(&out)?;
    Ok(EXIT_OK)
}

/// `Command::ExportRoadmapJson`'s arm, lifted out so `dispatch` stays inside its
/// line limit.
fn export_roadmap_json(
    cli_root: Option<PathBuf>,
    file: &std::path::Path,
    check: bool,
) -> Result<u8> {
    cmd_export_roadmap_json(&resolve_root(cli_root)?, file, check)
}

/// `aplan export-roadmap-json`, the contract generated rather than kept by hand
/// (`A1-033`).
fn cmd_export_roadmap_json(
    root: &std::path::Path,
    file: &std::path::Path,
    check: bool,
) -> Result<u8> {
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let existing =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let template: serde_json::Value = serde_json::from_str(&existing)
        .with_context(|| format!("{} is not JSON", file.display()))?;

    let tasks: Vec<&aplan_core::task::Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let source_name = file.display().to_string();
    let mut report = aplan_interop::report::Report::new();
    let rendered =
        aplan_interop::roadmap_json::render(&template, &tasks, &source_name, &mut report)?;
    // Pretty, two-space, newline-terminated: the shape the file already has, and
    // stable across runs because `serde_json` orders an object's keys.
    let mut text = serde_json::to_string_pretty(&rendered)?;
    text.push('\n');

    if !report.is_empty() {
        err!("{report}");
    }
    if check {
        if existing == text {
            write_stdout(&format!("{} is up to date\n", file.display()))?;
            return Ok(EXIT_OK);
        }
        errln!(
            "{} and the plan disagree. Run `aplan export-roadmap-json {}` and commit the result.",
            file.display(),
            file.display()
        );
        return Ok(EXIT_VIOLATIONS);
    }
    // Through the same writer as everything else (`A1-246`'s audit). This was
    // exempt from that scan on the reasoning that an artefact writer has "no
    // concurrent author to lose", and for this command that is false: `A1-033`
    // built it to *merge* rather than render, "so every field this model does
    // not hold is carried through untouched" -- which presupposes somebody
    // maintaining the file. `existing` is the text this merge was computed
    // from, so the comparison costs nothing.
    save_one(file, &existing, &text, "export-roadmap-json")?;
    write_stdout(&format!("wrote {}\n", file.display()))?;
    Ok(EXIT_OK)
}

/// `aplan export`, the other half of `import` (`A1-189`).
///
/// Reads the plan, hands it to the flavor's [`PlanTarget`], and puts the
/// conversion report on **standard error**. That last part is not a style
/// choice: with no `--out` the document itself is on standard output, and a
/// report mixed into it would corrupt the file the reader is redirecting.
///
/// [`PlanTarget`]: aplan_interop::export::PlanTarget
fn cmd_export(root: &std::path::Path, args: &ExportArgs) -> Result<u8> {
    let (format, out, check) = (
        args.format.as_str(),
        args.out.as_deref(),
        args.check.as_deref(),
    );
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let tasks: Vec<&aplan_core::task::Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let destination = out.or(check).map_or_else(
        || "standard output".to_string(),
        |p| p.display().to_string(),
    );
    let options = aplan_interop::export::ExportOptions {
        destination: destination.clone(),
    };
    let export = aplan_interop::export::write(format, &tasks, &options)?;

    if !export.report.is_empty() {
        // Folded here rather than in the `Display` impl, for the reason
        // `cmd_import` folds there: `aplan-interop` does not know a terminal
        // width, and a `Display` should not be choosing a layout.
        let mut text = String::from("report:\n");
        for line in export.report.to_string().lines() {
            text.push_str(&aplan_output::fold::fold(
                line,
                aplan_output::fold::err_width(),
                4,
            ));
            text.push('\n');
        }
        err!("{text}");
    }

    if let Some(path) = check {
        let existing =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        if existing == export.contents {
            // On standard error like every other line this command prints, and
            // unlike the gate on the generated `roadmap.json`. **Standard output
            // is the document and nothing else**, in all three modes, which is
            // what lets a reader redirect it without reading the flags first.
            errln!("{} is up to date", path.display());
            return Ok(EXIT_OK);
        }
        errln!(
            "{}",
            aplan_output::fold::fold(
                &format!(
                    "{} and the plan disagree. Regenerate it with `aplan export --format \
                     {format} --out {}`.",
                    path.display(),
                    path.display()
                ),
                aplan_output::fold::err_width(),
                0
            )
        );
        return Ok(EXIT_VIOLATIONS);
    }
    if let Some(path) = out {
        // Through the same writer as every other artefact this tool writes: the
        // destination is a document somebody else maintains, so a concurrent
        // edit is refused rather than overwritten.
        let existing = std::fs::read_to_string(path).unwrap_or_default();
        save_one(path, &existing, &export.contents, "export")?;
        errln!("wrote {}", path.display());
        return Ok(EXIT_OK);
    }
    write_stdout(&export.contents)?;
    Ok(EXIT_OK)
}

/// `aplan export-agents-md`, the contract an agent reads before it fails a gate
/// (`A1-038`).
fn cmd_export_agents_md(
    root: &std::path::Path,
    write: Option<&std::path::Path>,
    check: Option<&std::path::Path>,
) -> Result<u8> {
    let (config, config_dir) = config_at(root)?;
    // The plan is read for one thing only: the id convention, which is a
    // property of the plan rather than of its progress. A plan that did not load
    // still has ids, so this does not refuse -- but a plan with none teaches
    // nothing and the block simply says less.
    let plan = aplan_core::load_plan(root);
    let contract = agent_contract(root, config_dir.as_deref(), &config, &plan);
    let block = aplan_output::render_agents_md(&contract);

    if let Some(path) = check {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        refuse_malformed_block(&text, path)?;
        return Ok(match replace_block(&text, &block) {
            Some(updated) if updated == text => EXIT_OK,
            _ => {
                errln!(
                    "{}",
                    aplan_output::fold::fold(
                        &format!(
                            "{} disagrees with `aplan export-agents-md`. Regenerate it with \
                             `aplan export-agents-md --write {}`.",
                            path.display(),
                            path.display()
                        ),
                        aplan_output::fold::err_width(),
                        0
                    )
                );
                EXIT_VIOLATIONS
            }
        });
    }
    if let Some(path) = write {
        let existing = std::fs::read_to_string(path).unwrap_or_default();
        refuse_malformed_block(&existing, path)?;
        let updated = replace_block(&existing, &block).unwrap_or_else(|| {
            // No markers: append, keeping whatever the repository already wrote.
            // Never prepend -- a file starting with a generated block reads as a
            // generated file, and this one is not.
            let mut out = existing.clone();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&block);
            out
        });
        // Same reasoning as `export-roadmap-json` above (`A1-246`'s audit).
        // This file is the clearest case of a concurrent author: its whole
        // design is that the repository keeps its own prose around a generated
        // block, and the branch above says so -- "keeping whatever the
        // repository already wrote". An empty `existing` means the file is
        // being created, which `save` reads as must-not-exist.
        save_one(path, &existing, &updated, "export-agents-md --write")?;
        errln!("wrote {}", path.display());
        return Ok(EXIT_OK);
    }
    write_stdout(&block)?;
    Ok(EXIT_OK)
}

/// How the plan's location should read in a committed file.
///
/// Relative to where `.aplan.yml` sits, because that is the repository root as
/// far as this tool is concerned, and both a machine-specific absolute path and
/// a bare `.` are wrong in prose an adopter reads on someone else's clone.
fn plan_dir_for_prose(
    root: &std::path::Path,
    config_dir: Option<&std::path::Path>,
) -> Option<String> {
    // Both sides resolved before they are compared, because one is a discovered
    // *absolute* directory and the other is whatever the caller typed. Stripping
    // them as given worked only from the repository root: run from a
    // subdirectory with `--root ../aplan`, the strip failed and the block was
    // written saying planning lives under `../aplan/` -- a path relative to one
    // machine's working directory, in a committed artefact whose whole point is
    // that it reads the same on the next checkout (`A1-273`, audit; found by
    // `APLAN041` reporting a contract that was not in fact stale).
    let resolve = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let here = resolve(root);
    let relative = config_dir
        .and_then(|dir| here.strip_prefix(resolve(dir)).ok().map(PathBuf::from))
        .unwrap_or_else(|| root.to_path_buf());
    let text = relative.display().to_string();
    // `None` is the repository root, which the block says in words. Returning
    // the phrase from here put "the repository root/" inside backticks.
    (!text.is_empty() && text != ".").then_some(text)
}

/// Refuse a file this command cannot own safely (this task's audit).
///
/// Two failures, both silent before: a file with **two** blocks had its first
/// one updated and its second left stale, and `--check` then reported the file
/// as current while the stale copy went on telling agents the wrong policy --
/// which is the exact failure a generated artifact exists to prevent. And a file
/// with an opening marker and no closing one had a *second* block appended,
/// because there was nothing to replace.
///
/// Both are refusals rather than repairs: where the block ends is the author's
/// intent, and a tool that guesses at it eats the prose after it.
fn refuse_malformed_block(text: &str, path: &std::path::Path) -> Result<()> {
    let begins = aplan_core::contract::begin_spans(text).len();
    let ends = text.matches(aplan_output::AGENTS_END).count();
    let file = path.display();
    if begins > 1 || ends > 1 {
        anyhow::bail!(
            "{file} has {begins} `aplan:begin` and {ends} `aplan:end` markers; this command owns \
             one block per file. Keep the one you want and delete the rest."
        );
    }
    if begins != ends {
        // **Naming the marker, when the file has none.** A begin marker is any
        // line opening `<!-- aplan:begin` and closing `-->`, so an adopter who
        // wrote a plausible one is not here at all -- but one who wrote nothing
        // recognisable was told a count and left to find the shape in the source
        // (`A1-294`).
        let wanted = if begins == 0 {
            format!(
                "\n\nA begin marker is a line opening `{}`, then a space or a dash, and \
                 closing with `-->`. This command writes it as:\n\n{}",
                aplan_core::contract::BEGIN_PREFIX,
                aplan_output::AGENTS_BEGIN
            )
        } else {
            String::new()
        };
        anyhow::bail!(
            "{file} has {begins} `aplan:begin` and {ends} `aplan:end` markers; a block with only \
             one end is a half-finished edit, and guessing where it stops would eat the prose \
             after it.{wanted}"
        );
    }
    Ok(())
}

/// Replace the generated block in `text`, or `None` when it carries no markers.
///
/// Both markers or neither: a file with an opening marker and no close is a
/// half-written edit, and guessing where the block ends would eat the prose
/// after it.
fn replace_block(text: &str, block: &str) -> Option<String> {
    // The marker as the file wrote it, which may be the short form an adopter
    // typed. Replacing from its *start* is what rewrites it to the canonical one:
    // `block` opens with `AGENTS_BEGIN`, so the file ends up self-describing
    // without anybody copying a hundred characters exactly (`A1-294`).
    let begin = aplan_core::contract::begin_spans(text).first()?.start;
    let end = text.find(aplan_output::AGENTS_END)?;
    if end < begin {
        return None;
    }
    let after = end + aplan_output::AGENTS_END.len();
    let mut out = String::with_capacity(text.len() + block.len());
    out.push_str(&text[..begin]);
    out.push_str(block.trim_end_matches('\n'));
    out.push_str(&text[after..]);
    Some(out)
}

/// The MCP server's half of `APLAN041`: read the plan once and fill the block in.
///
/// A named function rather than four lines inside `cmd_mcp`, because the wiring
/// is the half that can go wrong and `mcp_parity.rs` says in its own header that
/// a defect in how `aplan mcp` builds its `Options` would pass there. This is
/// what a test can hold (`A1-273`).
fn mcp_agents_block(
    options: Options,
    root: &std::path::Path,
    config_dir: Option<&std::path::Path>,
    config: &aplan_config::Config,
) -> Options {
    let plan = aplan_core::load_plan(root);
    with_agents_block(options, root, config_dir, config, &plan)
}

/// Whether the policy this run is using is the one the tree under review
/// declares.
///
/// `APLAN041` compares a *policy-derived artefact* against a file, so it means
/// something only when both came from the same repository. Two invocations break
/// that, and both were reporting a current contract as stale (`A1-273`, audit):
///
/// - `--no-config` reads no policy at all, so the block is rendered from this
///   tool's defaults. Measured on this repository: a clean `AGENTS.md` reported
///   as stale.
/// - `--config` naming a file **outside** the plan's repository is the reviewing
///   case the flag exists for -- a trusted policy, deliberately not the tree's --
///   and judging somebody else's contract against a policy they never adopted is
///   a sentence about nothing.
///
/// A named config *inside* the repository is the other use of the same flag,
/// being explicit in your own tree, and keeps the rule: losing a gate silently
/// because a workflow spelled out its config file would be worse than either.
fn policy_is_this_trees_own(root: &std::path::Path) -> bool {
    match CONFIG_SOURCE.get() {
        Some(aplan_config::ConfigSource::None) => false,
        Some(aplan_config::ConfigSource::File(named)) => {
            // *The* file discovery would have chosen, not merely one inside the
            // repository. `<repo>/outside/trusted.yml` is inside it and is not
            // the policy the repository declares, and a `starts_with` test said
            // it was (`A1-273`, audit -- the first version of this guard).
            //
            // Fails closed: discovery finding nothing, or a path that will not
            // canonicalise, means this cannot be the tree's own config, and a
            // rule that would otherwise judge a contract against a foreign
            // policy is better silent.
            let abs = |p: &std::path::Path| p.canonicalize().ok();
            match (
                abs(named),
                aplan_config::discovered_config(&plan_repository(root)).and_then(|p| abs(&p)),
            ) {
                (Some(named), Some(discovered)) => named == discovered,
                _ => false,
            }
        }
        _ => true,
    }
}

/// The `AGENTS.md` a repository's generated block would live in.
///
/// The same root the MCP server resolves for git: the config's directory when
/// one was discovered, and the plan's repository otherwise. A named `--config`
/// says where config values resolve and nothing about where the repository is,
/// which is the distinction `A1-155` drew.
fn agents_md_path(root: &std::path::Path, config_dir: Option<&std::path::Path>) -> PathBuf {
    let base = if config_refused() || is_explicit_config() {
        plan_repository(root)
    } else {
        config_dir.map_or_else(|| plan_repository(root), std::path::Path::to_path_buf)
    };
    let path = base.join("AGENTS.md");
    // Named as a reader would type it. A discovered config's directory is
    // absolute, and `APLAN041`'s fix line is a command to paste: an absolute
    // path from the machine that ran the gate is the wrong thing to paste on
    // any other, which is the frame `A1-143` drew for `APLAN018`.
    match std::env::current_dir() {
        Ok(cwd) => path.strip_prefix(&cwd).unwrap_or(&path).to_path_buf(),
        Err(_) => path,
    }
}

/// Fill in the comparison `APLAN041` needs, once the plan is loaded.
///
/// Separate from [`resolve_check_options`] because the block is rendered from
/// the *plan* as well as the config, and the options are built before the plan
/// is read. Rendered only when the file is there: a repository that has never
/// run `export-agents-md` is the common case, and the rule is silent for it
/// anyway (`A1-273`).
fn with_agents_block(
    mut opts: Options,
    root: &std::path::Path,
    config_dir: Option<&std::path::Path>,
    config: &aplan_config::Config,
    plan: &aplan_core::load::Plan,
) -> Options {
    if !policy_is_this_trees_own(root) {
        return opts;
    }
    // A plan that did not load whole cannot say what its own contract should
    // contain: the block names the id convention the plan teaches, so a plan
    // read from a subset of its files renders a subset's block. Measured before
    // this: a repository whose task files were all unreadable reported `failed:
    // 0 error(s), 1 warning(s) across 0 tasks`, the single finding being a stale
    // contract that was not stale -- and the fix it named would have written a
    // block claiming the plan teaches no id convention (`A1-273`, audit).
    //
    // `load_errors` rather than `loaded_cleanly`, which prints. The rule comes
    // back the moment the plan parses, and the reader has `APLAN001` to act on
    // meanwhile.
    if !load_errors(plan).is_empty() {
        return opts;
    }
    let path = agents_md_path(root, config_dir);
    if !path.is_file() {
        return opts;
    }
    let contract = agent_contract(root, config_dir, config, plan);
    opts.agents_block = Some(aplan_core::AgentsBlock {
        path,
        current: block_between_markers(&aplan_output::render_agents_md(&contract)),
    });
    opts
}

/// The generated block without its own markers, which is what a file carries
/// between them.
fn block_between_markers(rendered: &str) -> String {
    match aplan_core::contract::block_in(rendered) {
        aplan_core::contract::Block::Present(inner) => inner.to_string(),
        // The renderer writes both markers, so this is unreachable in practice;
        // returning the whole thing makes a broken renderer a loud mismatch
        // rather than a silent pass.
        _ => rendered.to_string(),
    }
}

/// Assemble the contract from the repository's policy and its plan's shape.
fn agent_contract(
    root: &std::path::Path,
    config_dir: Option<&std::path::Path>,
    config: &aplan_config::Config,
    plan: &aplan_core::Plan,
) -> aplan_output::AgentContract {
    let evidence = config.evidence.as_ref();
    aplan_output::AgentContract {
        // Relative to the repository, never as resolved on this machine. The
        // block is a committed file with a drift gate on it, and an absolute
        // path makes it disagree with itself on the next developer's checkout --
        // which is a generated artifact failing for a reason nothing in the plan
        // caused (ADR-0009).
        plan_dir: plan_dir_for_prose(root, config_dir),
        id_examples: aplan_interop::ids::example_ids(plan),
        // One line per scope, in file order, described in the same words the
        // selector uses everywhere else (`A1-183`).
        scoped_evidence: config
            .policy
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|p| {
                let sel = p.when.selector().unwrap_or_default();
                let kinds = p.evidence.as_ref()?.require_min_strength.as_ref()?;
                let requirement = if kinds.is_empty() {
                    "no evidence strength is required".to_string()
                } else {
                    format!(
                        "a `done` task must cite {} evidence",
                        kinds
                            .iter()
                            .map(|k| format!("`{}`", k.as_str()))
                            .collect::<Vec<_>>()
                            .join(" or ")
                    )
                };
                Some((sel.describe().join(", "), requirement))
            })
            .collect(),
        min_strength: evidence
            .and_then(|e| e.require_min_strength.as_ref())
            .map(|k| k.iter().map(|kind| kind.as_str().to_string()).collect())
            .unwrap_or_default(),
        exempt_tags: evidence
            .and_then(|e| e.exempt_tags.as_ref())
            .cloned()
            .unwrap_or_default(),
        advisory: config.advisory.clone().unwrap_or_default(),
        switches: [
            (
                aplan_output::Switch::DenyWarnings,
                config.deny_warnings.unwrap_or(false),
            ),
            (
                aplan_output::Switch::DetectDrift,
                config.detect_drift.unwrap_or(false),
            ),
            (
                aplan_output::Switch::RequireCutMembership,
                config.require_cut_membership.unwrap_or(false),
            ),
            (
                aplan_output::Switch::RequireEffort,
                config.require_effort.unwrap_or(true),
            ),
            (aplan_output::Switch::UrlCheck, config.url.is_some()),
        ]
        .into_iter()
        .filter_map(|(switch, on)| on.then_some(switch))
        .collect(),
        runners: config
            .test
            .as_ref()
            // A runner may be unnamed, which is the single-runner form: it has
            // nothing to call itself and the sentence below reads better for it.
            .map(|rs| rs.all().iter().filter_map(|r| r.name.clone()).collect())
            .unwrap_or_default(),
        runner_count: config.test.as_ref().map_or(0, |rs| rs.all().len()),
        tools: aplan_mcp::tools::TOOLS
            .iter()
            .map(|t| (t.name.to_string(), t.description.to_string()))
            .collect(),
    }
}

/// `aplan check-docs`, the gate every adopter rebuilds (`A1-148`).
///
/// The plan is the vocabulary and the document is the input, which is the
/// opposite direction from every other command here: nothing about the document
/// is trusted, and nothing about the plan is changed. A reference is a word
/// carrying one of the prefixes the plan's own ids use, so the check works on
/// `A1-042`, `WH2-06`, `K0-3` and `M-phase-2` without this tool ever owning an
/// id format (`A1-029`).
const SUGGESTIONS: usize = 3;

/// The `did you mean` clause for a dangling reference, or nothing.
///
/// Capped, because the audit measured what an uncapped list looks like: `A1-04`
/// in this repository's own plan is one edit from twenty ids, and the line that
/// names all twenty is unreadable and says nothing. Three, then a count -- which
/// is itself the answer to "was this a typo or a truncation?". The JSON carries
/// every candidate, because a machine reader is not the one being helped.
fn suggestion_hint(near: &[&str]) -> String {
    match near.len() {
        0 => String::new(),
        1..=SUGGESTIONS => format!(" (did you mean {}?)", near.join(", ")),
        n => format!(
            " (did you mean {}, or {} others?)",
            near[..SUGGESTIONS].join(", "),
            n - SUGGESTIONS
        ),
    }
}

/// `check-docs`'s machine-readable answer.
fn check_docs_json(
    documents: usize,
    findings: &[(String, aplan_interop::docs::Reference, Vec<&str>)],
    disagreements: &[(String, aplan_interop::docs::MembershipProblem)],
) -> String {
    let rows: Vec<serde_json::Value> = findings
        .iter()
        .map(|(file, r, near)| {
            serde_json::json!({
                "file": file,
                "line": r.line,
                "id": r.id,
                "did_you_mean": near,
                // `null` unless the prefix itself is the typo.
                "mistyped_prefix": r.mistyped_prefix,
            })
        })
        .collect();
    let cuts: Vec<serde_json::Value> = disagreements
        .iter()
        .map(|(file, p)| {
            serde_json::json!({
                "file": file,
                "milestone": p.milestone,
                "undeclared": p.undeclared,
                "unplanned": p.unplanned,
                "no_declaration": p.no_declaration,
                "repeated_section": p.repeated_section,
            })
        })
        .collect();
    let doc = serde_json::json!({
        "schema": "aplan/check-docs/v1",
        "documents": documents,
        "findings": rows,
        // Always present, empty when `docs.membership` is not configured: a
        // key that comes and goes with a config setting makes every consumer
        // handle two shapes of the same document.
        "membership": cuts,
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&doc).expect("check-docs json serialises")
    )
}

/// `check-docs`'s human answer: every finding, then the one-line verdict.
fn check_docs_human(
    documents: usize,
    findings: &[(String, aplan_interop::docs::Reference, Vec<&str>)],
    disagreements: &[(String, aplan_interop::docs::MembershipProblem)],
) -> String {
    let mut out = String::new();

    for (file, r, near) in findings {
        let hint = suggestion_hint(near);
        // Where the *prefix* is the mistake, saying so is the whole value:
        // `DN8-01` reads as tracked in a plan whose cuts stop at `DN7-`, and
        // "not a task in this plan" leaves a reader hunting the number
        // (`A1-164`).
        if r.mistyped_prefix.is_empty() {
            let _ = writeln!(
                out,
                "{file}:{}: `{}` is not a task in this plan{hint}",
                r.line, r.id
            );
        } else {
            let _ = writeln!(
                out,
                "{file}:{}: `{}` is not a task in this plan, whose ids use {}{hint}",
                r.line,
                r.id,
                r.mistyped_prefix
                    .iter()
                    .map(|p| format!("`{p}`"))
                    .collect::<Vec<_>>()
                    .join(" or ")
            );
        }
    }
    for (file, p) in disagreements {
        if p.repeated_section {
            let _ = writeln!(
                out,
                "{}",
                aplan_output::fold::fold(
                    &format!(
                        "{file}: more than one heading here names `{}`, so which one declares \
                         its membership is ambiguous",
                        p.milestone
                    ),
                    aplan_output::fold::out_width(),
                    2
                )
            );
        }
        if p.no_declaration {
            let _ = writeln!(
                out,
                "{file}: `{}` has members and this document's section for it declares none",
                p.milestone
            );
        }
        // Named individually rather than counted: the fix begins with knowing
        // which id, and a count sends the reader to diff two lists by hand.
        if !p.undeclared.is_empty() {
            let _ = writeln!(
                out,
                "{file}: `{}` includes {}, which this document does not declare",
                p.milestone,
                quoted(&p.undeclared)
            );
        }
        if !p.unplanned.is_empty() {
            let _ = writeln!(
                out,
                "{file}: this document declares {}, which `{}` does not include",
                quoted(&p.unplanned),
                p.milestone
            );
        }
    }
    let cuts = if disagreements.is_empty() {
        String::new()
    } else {
        format!(
            ", {} cut(s) whose membership disagrees",
            disagreements.len()
        )
    };
    // The same verdict pair `check` and `verify` render, in the same roles, so a
    // reader running the three in a row does not meet three vocabularies
    // (`A1-288`).
    let clean = findings.is_empty() && disagreements.is_empty();
    let _ = writeln!(
        out,
        "{v}{}{v:#}: {} document(s), {} reference(s) naming no task{cuts}",
        if clean { "ok" } else { "failed" },
        documents,
        findings.len(),
        v = if clean {
            aplan_output::style::OK
        } else {
            aplan_output::style::ERROR
        }
    );
    out
}

/// The membership declaration configured for `path`, if any.
///
/// Matched on the resolved path rather than the spelling, so `./ROADMAP.md`,
/// `ROADMAP.md` and an absolute path are one document. A config entry naming a
/// document this run was not given is not a finding: `check-docs docs/foo.md`
/// must not report every cut in `ROADMAP.md` because the config mentions it.
fn membership_shape(
    config: &aplan_config::Config,
    config_dir: Option<&std::path::Path>,
    path: &std::path::Path,
) -> Option<aplan_interop::docs::DeclaredBy> {
    let entries = config.docs.as_ref()?.membership.as_ref()?;
    // Canonicalised, which resolves the relative spelling *and* the symlinks a
    // temporary directory is usually reached through. A document the config names
    // and the filesystem does not have cannot be one of `paths`, so it matches
    // nothing, which is the same silence as not naming it.
    let want = std::fs::canonicalize(path).ok()?;
    let entry = entries.iter().find(|e| {
        let declared =
            config_dir.map_or_else(|| std::path::PathBuf::from(&e.doc), |d| d.join(&e.doc));
        std::fs::canonicalize(&declared).is_ok_and(|d| d == want)
    })?;
    Some(match entry.declared_by {
        // Validated at load: `line` without a `marker` never reaches here.
        aplan_config::DeclaredBy::Line => {
            aplan_interop::docs::DeclaredBy::Line(entry.marker.clone().unwrap_or_default())
        }
        aplan_config::DeclaredBy::Table => aplan_interop::docs::DeclaredBy::Table,
    })
}

fn cmd_check_docs(
    root: &std::path::Path,
    paths: &[std::path::PathBuf],
    format: &str,
) -> Result<u8> {
    // The diagnostic formats belong to the rules, and this carries no rule code:
    // a finding here is a document naming a task, not a plan violating an
    // invariant, which is the same line `aplan-interop`'s import notices draw.
    let format = format_for("check-docs", format)?;
    // A plan that did not load cannot say which ids exist, and a gate that
    // answers "none of your references resolve" because the plan is broken is
    // worse than one that refuses.
    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let ids: std::collections::BTreeSet<String> =
        plan.tasks.iter().map(|lt| lt.task.id.clone()).collect();

    // A plan whose ids teach no prefix -- `auth-login`, or the ULIDs
    // `id_strategy: ulid` mints -- gives this nothing to recognise, and every
    // document would come back clean. A gate that cannot fire must say so:
    // reporting `ok` here is the "reads as handled" failure this command exists
    // to prevent, arriving from the tool itself (`A1-148`).
    if aplan_interop::docs::Vocabulary::of(&ids).is_empty() {
        anyhow::bail!(
            "this plan's ids teach no prefix (an id ending in digits, like `T-042`, teaches \
             `T-`), so nothing in a document can be told from an ordinary word; `check-docs` \
             checked nothing"
        );
    }

    // Every cut's membership, from the milestones' side (ADR-0008). Read once
    // rather than per document: it is the plan's copy of the fact, and the
    // documents hold the other one.
    let (config, config_dir) = config_at(root)?;
    //
    // Cuts only. A release is a milestone too (ADR-0022), and its `blocked_by`
    // holds the *cuts* it closes rather than work, so asking a document to
    // declare it on the line that lists a cut's tasks is a category error --
    // and the audit of `A1-165` watched it become one, reporting a release
    // whose section was the paragraph of prose such a section usually is.
    let milestones: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> = plan
        .tasks
        .iter()
        .filter(|lt| lt.task.is_milestone() && !lt.task.is_release())
        .map(|lt| {
            (
                lt.task.id.clone(),
                lt.task.blocked_by.iter().cloned().collect(),
            )
        })
        .collect();

    let mut findings: Vec<(String, aplan_interop::docs::Reference, Vec<&str>)> = Vec::new();
    let mut disagreements: Vec<(String, aplan_interop::docs::MembershipProblem)> = Vec::new();
    for path in paths {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        for r in aplan_interop::docs::dangling(&text, &ids) {
            let near = aplan_interop::docs::nearest(&r.id, &ids);
            findings.push((path.display().to_string(), r, near));
        }
        if let Some(declared_by) = membership_shape(&config, config_dir.as_deref(), path) {
            for p in aplan_interop::docs::membership(&text, &milestones, &declared_by, &ids) {
                disagreements.push((path.display().to_string(), p));
            }
        }
    }

    let out = if format == Format::Json {
        check_docs_json(paths.len(), &findings, &disagreements)
    } else {
        check_docs_human(paths.len(), &findings, &disagreements)
    };
    write_stdout(&out)?;
    Ok(if findings.is_empty() && disagreements.is_empty() {
        EXIT_OK
    } else {
        EXIT_VIOLATIONS
    })
}

/// A list of ids, backticked, for a human-facing line.
fn quoted(ids: &[String]) -> String {
    ids.iter()
        .map(|i| format!("`{i}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `aplan summary`, roll the plan up by phase (tasks, effort, ready count).
fn cmd_summary(root: &std::path::Path, format: &str, select: &SelectArgs) -> Result<u8> {
    // `summary` renders only a table or a JSON document; reject the diagnostic
    // formats (`agent`, `github`) rather than silently collapsing them to JSON,
    // which is what the help promises.
    let format = format_for("summary", format)?;
    let plan = load_checked(root)?;
    // Like `ready`/`why`/`graph`, a summary of a plan that did not load would be a
    // partial answer that looks whole.
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    // The declared throughput, if any: `summary` derives its durations from it
    // (`A1-147`, ADR-0023). A repository whose `.aplan.yml` does not load has
    // already failed above, so this cannot silently report nothing.
    let (config, _) = config_at(root)?;
    let selector = select.selector()?;
    let keep = selected_ids(&plan, &selector)?;
    let summary = graph
        .summary_of(&|id| keep.contains(id))
        .with_throughput(config.effort_days_per_week);
    render_tolerant(|| render_summary(&summary, format))?;
    Ok(EXIT_OK)
}

/// `aplan drop`, the cancellation a plan records rather than deletes.
///
/// Dropping a cut was forty hand edits in the pilot that asked for this, and a
/// half-applied one leaves a plan that says something nobody decided (`A1-077`).
fn cmd_drop(
    root: &std::path::Path,
    ids: &[String],
    by_cut: bool,
    verdict: VerdictFlags,
) -> Result<u8> {
    let plan = aplan_core::load_plan(root);
    if !loaded_cleanly(&plan) {
        anyhow::bail!("the plan did not load; run `aplan check` before mutating it");
    }
    let by_id: std::collections::BTreeMap<&str, &aplan_core::load::LoadedTask> = plan
        .tasks
        .iter()
        .map(|lt| (lt.task.id.as_str(), lt))
        .collect();

    // Under `--cut` the arguments name cuts, so they take the same vocabulary
    // every other `--cut` does: the id or the label the cut reports itself under
    // (`A1-139`, extended by its audit -- this was the one command that names a
    // cut and still refused the label `roadmap` prints).
    let ids: Vec<String> = if by_cut {
        ids.iter()
            .map(|name| aplan_interop::cut::resolve_id(&plan, name).map_err(anyhow::Error::from))
            .collect::<Result<_>>()?
    } else {
        ids.to_vec()
    };

    // Resolve everything first: a partial write is worse than a refusal
    // (`A1-030`), and half a cancelled cut is a plan nobody decided on.
    let mut targets: Vec<&aplan_core::load::LoadedTask> = Vec::new();
    for id in &ids {
        // Through the shared refusal, not the map: a `BTreeMap` keyed by id
        // keeps one entry per key, so a duplicate id is invisible to it and
        // whichever file lost the race would have been dropped silently
        // (`A1-135`).
        let lt = locate(&plan, id)?;
        let _ = &by_id;
        if by_cut {
            if !lt.task.is_milestone() {
                anyhow::bail!("`{id}` is not a milestone; drop it without `--cut`");
            }
            // A milestone's `blocked_by` holds its members *and*, since
            // ADR-0013, the cut it follows. Treating the whole list as
            // membership cancelled the predecessor cut and left its work live,
            // and `check` called that clean (`A1-084`). The roadmap has always
            // drawn this line the same way.
            for member in &lt.task.blocked_by {
                if let Some(m) = by_id.get(member.as_str()) {
                    if !m.task.is_milestone() && !m.task.status.is_terminal() {
                        targets.push(m);
                    }
                }
            }
        }
        if lt.task.status.is_terminal() {
            anyhow::bail!(
                "`{id}` is already `{}`; reviving a terminal task would launder a decision the \
                 plan recorded",
                lt.task.status
            );
        }
        targets.push(lt);
    }

    // One task named twice, or named alongside the cut that contains it, is one
    // cancellation. Without this the count says two, which is the same defect
    // `APLAN026` had for a repeated tag and `blocked_by` has always avoided.
    let mut seen = std::collections::BTreeSet::new();
    targets.retain(|lt| seen.insert(lt.task.id.clone()));

    // Rewrite every file in memory before writing any of them, keeping what was
    // read so the save can refuse rather than clobber (`A1-226`).
    let mut writes: Vec<aplan_core::save::Replacement> = Vec::new();
    for lt in &targets {
        let text = std::fs::read_to_string(&lt.path)
            .with_context(|| format!("reading {}", lt.path.display()))?;
        let updated = aplan_core::close::set_status(&text, aplan_core::task::Status::Dropped)
            .map_err(|e| anyhow::anyhow!("{}: {e}", lt.path.display()))?;
        writes.push(aplan_core::save::Replacement {
            path: lt.path.clone(),
            expected: text,
            updated,
        });
    }
    // Every rewrite at once, because dropping a cut drops its members too and a
    // post-state built one file at a time would judge a plan half-cancelled
    // (`A1-113`).
    let mut after = aplan_core::mutation::clone_of(&plan);
    for (write, lt) in writes.iter().zip(&targets) {
        after = aplan_core::mutation::with_replaced(
            &after,
            &lt.task.id,
            task_of(&write.updated, &lt.task.id)?,
        );
    }
    let what = match targets.as_slice() {
        [one] => format!("drop `{}`", one.task.id),
        many => format!("drop {} task(s)", many.len()),
    };
    refuse_bad_mutation(root, &what, &plan, &after, verdict)?;

    // Proved writable before any of them is written, the way `supersede` and `mv`
    // do it. `drop --cut` writes a file per member, so one unwritable file left
    // half a cut cancelled and half of it live: a plan that says something nobody
    // decided, which is the state every refusal here exists to avoid (`A1-030`,
    // `A1-130`, `A1-127`). The probe opens for writing without truncating.
    for write in &writes {
        std::fs::OpenOptions::new()
            .write(true)
            .open(&write.path)
            .with_context(|| format!("{} is not writable", write.path.display()))?;
    }
    // **Through the writer that refuses to clobber, as one change** (`A1-234`).
    // These were `std::fs::write` calls: no comparison, so a concurrent editor's
    // change was discarded silently -- the defect `A1-226` measured at
    // twenty-four agents, still live here because that task fixed the
    // cut-membership call site and this one writes task files. No lock either,
    // and no atomic rename, so an interrupted `drop --cut` left a *truncated*
    // milestone, which is every member of the cut orphaned at once.
    save_batch(&writes, "drop")?;

    let mut out = anstream::stdout().lock();
    for lt in &targets {
        writeln!(
            out,
            "dropped {i}{}{i:#}  {}",
            lt.task.id,
            lt.task.title,
            i = aplan_output::style::ID
        )?;
    }
    writeln!(out, "{} task(s) dropped", targets.len())?;
    // `--help` promises this records the decision rather than deleting the
    // record of it, and what it records is a status. The reason is prose and the
    // body is where prose lives; nothing said so, which is the gap the pilot
    // reported (`A1-084`, pre-public adoption evidence item 12).
    writeln!(
        out,
        "Say why in each task's body: the status records the decision, not the reason for it."
    )?;
    Ok(EXIT_OK)
}

/// Every refusal a move must survive, and the cuts each task is leaving.
///
/// Split from [`cmd_mv`] so the checks are one thing a reader can hold: nothing
/// here writes, and a caller that reaches the end has a plan it may apply.
fn cuts_being_left(
    plan: &aplan_core::load::Plan,
    ids: &[String],
    cut: &str,
) -> Result<std::collections::BTreeMap<String, Vec<String>>> {
    for id in ids {
        let Some(moving) = plan.tasks.iter().find(|lt| &lt.task.id == id) else {
            anyhow::bail!("no task `{id}` in this plan");
        };
        // Moving a milestone would rewrite the edges that sequence the cuts, not
        // the membership it looks like it is rewriting: the source cut stops
        // following this one and the destination claims it as a member. `check`
        // reports that as clean and `ready` starts offering work the sequence had
        // held back, which is `A1-073` undone by one command (`A1-084`).
        if moving.task.is_milestone() {
            anyhow::bail!(
                "`{id}` is a cut, not a task in one: moving it would rewrite which cut follows \
                 which, not its membership. Sequence the cuts instead: `aplan dep add {id} --after \
                 <cut>`, and `aplan dep rm` to undo it."
            );
        }
    }
    // Both ends are checked before either is touched, so a refusal changes
    // nothing.
    aplan_interop::cut::check_target(plan, cut)?;

    // Every cut any of them is leaving, and which of its members are going.
    let mut leaving: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for lt in plan.tasks.iter().filter(|lt| lt.task.is_milestone()) {
        if lt.task.id == cut {
            continue;
        }
        let going: Vec<String> = ids
            .iter()
            .filter(|id| lt.task.blocked_by.iter().any(|b| b == *id))
            .cloned()
            .collect();
        if !going.is_empty() {
            leaving.insert(lt.task.id.clone(), going);
        }
    }

    // The work-member refusal that used to live here is gone (`A1-134`).
    //
    // It existed because `APLAN021` asked whether `blocked_by` was *empty*, and a
    // sequenced cut keeps its predecessor there, so a cut emptied of work was
    // invisible to every rule and `mv` had to hand-roll the check. The rule now
    // counts live work members, so the post-state check reports it like every
    // other consequence of a mutation (`A1-113`), and a second implementation of
    // the same judgement is exactly the divergence that check exists to remove.
    //
    // What that changes for a caller: emptying a cut is now a *warning* rather
    // than a refusal, so it goes through where the repository tolerates warnings
    // and is refused under `deny_warnings`. That is the repository's policy
    // deciding, which is the point.

    Ok(leaving)
}

/// `aplan mv`, the two-milestone edit `add --cut` already proves is possible.
///
/// Takes a **set** since `A1-127`: re-prioritising a group meant one invocation
/// per task, and the fourth pilot measured what that costs -- a footnote at 33
/// tasks and "the dominant cost after authoring" at 105 (pre-public adoption evidence
/// item 6). Nine invocations are also nine chances to stop half way, which is the
/// property the single form was careful to have and the caller had no way to keep
/// across a group.
///
/// All-or-nothing across the whole set, in the sense `A1-030` fixed and `A1-084`
/// made `mv`'s reason for being trustworthy: every refusal runs, and every write
/// is computed, before any file is touched. A member that cannot move refuses the
/// move rather than leaving the ones before it in the new cut and the rest where
/// they were.
/// Everything `mv`'s cut half will change, worked out before anything is written.
///
/// Split out of [`cmd_mv`] when `--phase` arrived (`A1-171`): the two halves of a
/// move are judged as one post-state and written as two steps, and neither
/// function should have to be read whole to follow either.
type CutMove = (
    std::collections::BTreeMap<String, Vec<String>>,
    Vec<(String, aplan_interop::cut::PendingRemoval)>,
);

fn plan_cut_move(plan: &aplan_core::Plan, ids: &[String], cut: &str) -> Result<CutMove> {
    let leaving = cuts_being_left(plan, ids, cut)?;

    // Work out every removal before writing anything. Adding to the destination
    // and then discovering the source cannot be edited surgically leaves the
    // task in two cuts and the plan failing `APLAN022`: a partial write, and
    // worse than a refusal (`A1-030`).
    let mut pending = Vec::new();
    for (from, going) in &leaving {
        pending.push((
            from.clone(),
            aplan_interop::cut::plan_removal(plan, from, going)?,
        ));
    }
    Ok((leaving, pending))
}

fn cmd_mv(
    root: &std::path::Path,
    ids: &[String],
    cut: Option<&str>,
    phase: Option<u32>,
    verdict: VerdictFlags,
) -> Result<u8> {
    let plan = aplan_core::load_plan(root);
    if !loaded_cleanly(&plan) {
        anyhow::bail!("the plan did not load; run `aplan check` before mutating it");
    }
    // Deduplicated first: naming a task twice is one move, and a repeat would
    // otherwise count twice when working out whether a cut is being emptied of
    // its work.
    let mut ids: Vec<String> = ids.to_vec();
    ids.sort();
    ids.dedup();

    // Every id this move names must identify one task, checked before anything
    // is read: `mv` writes to the file it finds, and on a duplicate id that file
    // is whichever the search reached first (`A1-135`).
    for id in &ids {
        locate(&plan, id)?;
    }

    // The destination may be named by id or by the label the cut reports itself
    // under, resolved once so the writes downstream work in ids (`A1-139`).
    let cut = match cut {
        Some(name) => Some(aplan_interop::cut::resolve_id(&plan, name)?),
        None => None,
    };
    let cut = cut.as_deref();
    let moving = match cut {
        Some(cut) => Some(plan_cut_move(&plan, &ids, cut)?),
        None => None,
    };
    let no_cuts = std::collections::BTreeMap::new();
    let leaving = moving.as_ref().map_or(&no_cuts, |(l, _)| l);

    // The destination edit and every source removal, judged as one post-state: a
    // move is one change to the plan even though it is two writes, and a bulk
    // move is one change however many files it touches.
    let mut after = aplan_core::mutation::clone_of(&plan);
    if let Some(slot) = after
        .tasks
        .iter_mut()
        .find(|lt| Some(lt.task.id.as_str()) == cut)
    {
        for id in &ids {
            if !slot.task.blocked_by.iter().any(|b| b == id) {
                slot.task.blocked_by.push(id.clone());
            }
        }
    }
    for (from, going) in leaving {
        if let Some(slot) = after.tasks.iter_mut().find(|lt| &lt.task.id == from) {
            slot.task.blocked_by.retain(|b| !going.contains(b));
        }
    }

    // A phase is a rollup key and nothing gates on it, so this cannot make the
    // plan illegal. It goes through the same post-state anyway: a move that
    // changes both is one change, and the day a rule does read `phase` this is
    // already where it would be caught (`A1-171`).
    if let Some(phase) = phase {
        for slot in &mut after.tasks {
            if ids.iter().any(|id| id == &slot.task.id) {
                slot.task.phase = phase;
            }
        }
    }

    let what = describe_move(&ids, cut, phase);
    refuse_bad_mutation(root, &what, &plan, &after, verdict)?;

    probe_writable(&plan, &ids, cut, phase, moving.as_ref())?;

    // **Everything this move writes, as one change** (`A1-234`). Written one at
    // a time, a failure between the destination and a source left the task in
    // both cuts -- and `APLAN022` says so only where the repository asked for
    // `require_cut_membership`, so the usual outcome was a plan somebody had to
    // repair by hand with nothing reporting it. Rare while "a file that changed
    // under us" meant a permission problem; `A1-226` made it reachable by
    // ordinary contention.
    //
    // The **phase** edits are in the same batch, not a raw write after it: a
    // move that changes both is one change, and `mv --cut X --phase N` could
    // otherwise land the cut and half the phases (this task's audit). The help
    // says "editing both milestones or neither", and the command should mean it
    // about everything it touches.
    //
    // `replace_all` takes every lock, checks every file against what was read,
    // and proves each writable before renaming any of them, so the window this
    // closes is everything but the rename loop itself.
    let placement = match (cut, moving.as_ref()) {
        (Some(cut), Some(_)) => Some(aplan_interop::cut::plan_placement(&plan, cut, &ids)?),
        _ => None,
    };
    let phase_edits = match phase {
        Some(phase) => plan_phase(&plan, &ids, phase)?,
        None => Vec::new(),
    };
    let batch: Vec<aplan_core::save::Replacement> = placement
        .as_ref()
        .and_then(aplan_interop::cut::PendingPlacement::replacement)
        .into_iter()
        .chain(
            moving
                .as_ref()
                .into_iter()
                .flat_map(|(_, pending)| pending.iter().filter_map(|(_, r)| r.replacement())),
        )
        .chain(phase_edits.iter().filter_map(|e| e.replacement.clone()))
        .collect();
    save_batch(&batch, "mv")?;

    let mut out = anstream::stdout().lock();
    if let (Some(cut), Some((_, pending)), Some(placement)) = (cut, moving, placement) {
        let placed = aplan_interop::cut::Placed {
            path: placement.path,
            added: placement.added,
        };
        // Named, never counted. The set is exactly what the author is unsure of, so
        // "moved 9" is a worse answer than the nine ids (`A1-127`).
        for id in &ids {
            if placed.added.iter().any(|a| a == id) {
                writeln!(out, "added {id} to `{cut}` in {}", placed.path.display())?;
            } else {
                writeln!(out, "`{id}` was already in `{cut}`")?;
            }
        }
        // Reporting only: the batch above wrote every one of these or none of
        // them. This loop used to be where each source was written, and where
        // the message describing a task left in two cuts had to live (`A1-234`).
        for (from, removal) in pending {
            let removed = removal.into_removed();
            for id in &removed.removed {
                writeln!(
                    out,
                    "removed {id} from `{from}` in {}",
                    removed.path.display()
                )?;
            }
        }
    }

    if let Some(phase) = phase {
        report_phase(&phase_edits, phase, &mut out)?;
    }
    Ok(EXIT_OK)
}

/// Write a batch through the one writer that refuses to clobber, saying which
/// command is asking (`A1-234`).
///
/// Shared because both callers need the same three sentences and the same
/// promise in each of them: **nothing was written**. A refusal a reader cannot
/// act on is a refusal they work around, and "run it again" is only honest
/// while the plan is untouched.
/// One task file, replaced only while it still holds what the caller read.
///
/// The single-file shape of [`save_batch`], and the reason it exists rather than
/// five call sites each building a one-element vector: a command that edits one
/// task should not have to look like a batch to be safe. Every mutating command
/// reaches the filesystem through one of these two, which is the property
/// `A1-246` exists to establish and `every_plan_write_goes_through_save` keeps.
fn save_one(path: &std::path::Path, expected: &str, updated: &str, command: &str) -> Result<()> {
    save_batch(
        &[aplan_core::save::Replacement {
            path: path.to_path_buf(),
            expected: expected.to_string(),
            updated: updated.to_string(),
        }],
        command,
    )
}

fn save_batch(batch: &[aplan_core::save::Replacement], command: &str) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    // Built rather than interpolated in place: the gate that reads every
    // invocation this tool prints parses what it finds in the source, and a
    // format placeholder where a subcommand goes is not one (`A1-089`).
    let retry = format!("aplan {command}");
    aplan_core::save::replace_all(batch)
        .map_err(|e| anyhow::anyhow!("{}", aplan_core::save::advice(&e, &retry)))
}

/// Open every file this move will write, before any of them is written.
///
/// `mv` writes the destination first and then each source, so a source that
/// cannot be written leaves a task in two cuts -- and with a set, some tasks
/// moved and others in both places at once. Measured rather than assumed: with
/// one source read-only, the task ended in two cuts and `check` still said `ok`,
/// because `APLAN022` is off unless `require_cut_membership` is set. The probe
/// opens for writing without truncating, so it proves the permission rather than
/// inferring it from metadata (`A1-130`, `A1-127`).
fn probe_writable(
    plan: &aplan_core::Plan,
    ids: &[String],
    cut: Option<&str>,
    phase: Option<u32>,
    moving: Option<&CutMove>,
) -> Result<()> {
    let mut will_write: Vec<std::path::PathBuf> = Vec::new();
    if let (Some(cut), Some((_, pending))) = (cut, moving) {
        will_write.extend(
            plan.tasks
                .iter()
                .find(|lt| lt.task.id == cut)
                .map(|lt| lt.path.clone()),
        );
        will_write.extend(
            pending
                .iter()
                .filter(|(_, r)| r.writes())
                .map(|(_, r)| r.path().to_path_buf()),
        );
    }
    // A phase move writes the task's own file, which the cut half never does.
    if phase.is_some() {
        for id in ids {
            will_write.push(locate(plan, id)?.path.clone());
        }
    }
    for path in &will_write {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .with_context(|| format!("{} is not writable", path.display()))?;
    }
    Ok(())
}

/// What a move is, in one clause, for the refusal that names it.
fn describe_move(ids: &[String], cut: Option<&str>, phase: Option<u32>) -> String {
    let who = match ids {
        [one] => format!("`{one}`"),
        many => format!("{} tasks", many.len()),
    };
    match (cut, phase) {
        (Some(cut), Some(phase)) => format!("move {who} to `{cut}` and phase {phase}"),
        (Some(cut), None) => format!("move {who} to `{cut}`"),
        (None, Some(phase)) => format!("move {who} to phase {phase}"),
        (None, None) => format!("move {who}"),
    }
}

/// The phase half of `mv`: one line per task file, after the probe (`A1-171`).
///
/// Named and not counted, like the cut half beside it: the set is what the
/// author is unsure of. A task already in the phase is said so rather than
/// rewritten, so a re-run reads as a no-op instead of a diff.
/// One task's phase edit, worked out but not written.
struct PhaseEdit {
    id: String,
    path: std::path::PathBuf,
    /// `None` when the task is already in that phase.
    replacement: Option<aplan_core::save::Replacement>,
}

/// Work out every phase edit without writing any.
///
/// Separated from the writing so a `mv` that changes both a cut and a phase is
/// **one change**: these were raw `std::fs::write` calls after the cut batch, so
/// a failure among them left some tasks moved and rephased and others moved
/// only -- a partial `mv` inside the command whose help says "editing both
/// milestones or neither" (`A1-234`, audit).
fn plan_phase(plan: &aplan_core::Plan, ids: &[String], phase: u32) -> Result<Vec<PhaseEdit>> {
    let mut edits = Vec::with_capacity(ids.len());
    for id in ids {
        let path = locate(plan, id)?.path.clone();
        let file = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let updated = aplan_core::close::set_phase(&file, phase)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let replacement = (updated != file).then(|| aplan_core::save::Replacement {
            path: path.clone(),
            expected: file,
            updated,
        });
        edits.push(PhaseEdit {
            id: id.clone(),
            path,
            replacement,
        });
    }
    Ok(edits)
}

/// Say what the phase edits did, once the batch carrying them has been written.
fn report_phase(edits: &[PhaseEdit], phase: u32, out: &mut impl std::io::Write) -> Result<()> {
    for edit in edits {
        if edit.replacement.is_none() {
            writeln!(out, "`{}` was already in phase {phase}", edit.id)?;
        } else {
            writeln!(
                out,
                "moved {} to phase {phase} in {}",
                edit.id,
                edit.path.display()
            )?;
        }
    }
    Ok(())
}

/// `aplan tasks`, the plan as data.
///
/// `summary` rolls up by phase and by nothing else, so a project that types its
/// work another way kept a hand-maintained second copy of the effort table, of
/// exactly the fact this tool owns (`A1-075`). A `--group-by` flag would have
/// answered one question and invited the next.
fn cmd_tasks(root: &std::path::Path, format: &str, select: &SelectArgs) -> Result<u8> {
    let format = format_for("tasks", format)?;
    let plan = load_checked(root)?;
    // A partial export looks whole, and a rollup built on one is wrong without
    // saying so.
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let selector = select.selector()?;
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    refuse_a_release_as_a_cut(&plan, &graph, &selector.cuts)?;
    // Rows built over the whole plan, then narrowed: every derived field on a
    // row -- `ready`, `rank`, `cut` -- is the answer the whole plan gives, which
    // is the only answer that is true (`A1-214`).
    let mut export = graph.task_rows();
    export.tasks.retain(|row| {
        graph
            .get(&row.id)
            .is_some_and(|t| selector.matches(t, row.cut.as_deref(), row.milestone.as_deref()))
    });
    render_tolerant(|| render_tasks(&export, format))?;
    Ok(EXIT_OK)
}

/// `aplan critical-path`, the longest `effort_days`-weighted chain of dependent
/// tasks. Advisory: it never fails the build, a plan is not wrong for being long.
fn cmd_critical_path(root: &std::path::Path, format: &str) -> Result<u8> {
    let format = format_for("critical-path", format)?;
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
/// Refuse a `--regenerate-with` that would corrupt the banner or do nothing.
///
/// The value is written inside an HTML comment in a *generated, committed*
/// artifact, so `-->` ends the comment early and the rest of the command becomes
/// document content, and a newline moves the banner off the line every consumer
/// reads it on. Neither is caught by anything downstream: the file is diffed
/// against its generator, so a corrupted banner regenerates identically corrupt.
///
/// And on a format with no banner the flag would be accepted and ignored, which
/// reads exactly like a flag that worked (`A1-059`).
fn validate_regenerate_with(cmd: &str, markdown: bool) -> Result<&str> {
    if !markdown {
        anyhow::bail!(
            "`--regenerate-with` names the command in the markdown banner, and only \
             `--format markdown` has one. Drop the flag, or ask for markdown."
        );
    }
    if cmd.trim().is_empty() {
        anyhow::bail!(
            "`--regenerate-with` is empty. Give the command your repository regenerates \
             this file with, or drop the flag to name the plain `aplan` command."
        );
    }
    if cmd.contains("-->") {
        anyhow::bail!(
            "`--regenerate-with` contains `-->`, which would close the banner's HTML \
             comment and spill the rest into the document."
        );
    }
    // The value is written between backticks, so a backtick in it makes the field
    // it lands in unparseable: nothing downstream can tell where the command ends,
    // and a drift gate reading the line reads a different command than the one
    // given. Refused for the same reason as `-->`, one delimiter in.
    if cmd.contains('`') {
        anyhow::bail!(
            "`--regenerate-with` contains a backtick, which delimits the command in \
             the banner, so nothing reading the line could tell where yours ends."
        );
    }
    // `char::is_control` does not cover U+2028 and U+2029, which are line and
    // paragraph separators to the Unicode line-breaking algorithm and to plenty of
    // parsers a consumer's drift gate might be written in, even though Rust's own
    // `lines()` ignores them. A banner that is one line here and two there is the
    // corruption this function exists to prevent.
    if cmd
        .chars()
        .any(|c| c.is_control() || c == '\u{2028}' || c == '\u{2029}')
    {
        anyhow::bail!(
            "`--regenerate-with` contains a newline, control character or Unicode line \
             separator, which would break the banner across lines. Give a single-line \
             command."
        );
    }
    // Trimmed, not merely non-blank: the value goes into a byte-exact generated
    // file, so `make roadmap` and ` make roadmap ` would otherwise produce two
    // files that differ in a line claiming to name the same command. The blank
    // check above already reads the trimmed form, so accepting padding here would
    // have been the inconsistency, not the trim.
    Ok(cmd.trim())
}

fn cmd_roadmap(root: &std::path::Path, args: &RoadmapArgs) -> Result<u8> {
    // Validate the format before touching the plan, so a bad format is a usage
    // error (exit 2) and is not masked by a plan error, matching `summary` and
    // `critical-path`. `markdown` is roadmap-specific (not in the shared `Format`).
    enum Out {
        Markdown,
        Shared(Format),
    }
    let format = args.format.as_str();
    let regenerate_with = args.regenerate_with.as_deref();
    let format = format_for("roadmap", format)?;
    let out = if format == Format::Markdown {
        Out::Markdown
    } else {
        Out::Shared(format)
    };
    let regenerate_with = match regenerate_with {
        Some(cmd) => Some(validate_regenerate_with(cmd, matches!(out, Out::Markdown))?),
        None => None,
    };

    let plan = load_checked(root)?;
    if !loaded_cleanly(&plan) {
        return Ok(EXIT_VIOLATIONS);
    }
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let mut roadmap = Graph::new(tasks.iter().copied()).roadmap();
    // Grouped only where the repository asked for it; the config load already
    // refused a `group_by` naming a family `tag_families` does not declare, so
    // the members are here to be grouped by (`A1-078`).
    let (config, _) = config_at(root)?;
    roadmap = roadmap.with_throughput(config.effort_days_per_week);
    if let Some(members) = config
        .roadmap
        .as_ref()
        .and_then(|r| r.group_by.as_ref())
        .and_then(|family| config.tag_families.as_ref().and_then(|f| f.get(family)))
    {
        roadmap = roadmap.grouped_by(members.members());
    }
    let text = match out {
        Out::Markdown => render_roadmap_markdown(&roadmap, regenerate_with),
        Out::Shared(f) => render_roadmap(&roadmap, f),
    };
    render_tolerant(|| {
        use std::io::Write as _;
        anstream::stdout().write_all(text.as_bytes())
    })?;
    Ok(EXIT_OK)
}

#[cfg(test)]
mod suggestion_tests {
    use super::suggestion_hint;

    #[test]
    fn a_long_list_of_candidates_is_counted_rather_than_printed() {
        assert_eq!(suggestion_hint(&[]), "");
        assert_eq!(suggestion_hint(&["A1-042"]), " (did you mean A1-042?)");
        assert_eq!(
            suggestion_hint(&["A1-042", "A1-043", "A1-044"]),
            " (did you mean A1-042, A1-043, A1-044?)"
        );
        // Four is where it stops naming them. The count is the useful part: a
        // reference one edit from twenty ids was truncated, not mistyped.
        assert_eq!(
            suggestion_hint(&["A1-042", "A1-043", "A1-044", "A1-045", "A1-046"]),
            " (did you mean A1-042, A1-043, A1-044, or 2 others?)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aplan_config::Config;

    /// **The two readers of `--color` agree, on every shape `clap` accepts.**
    ///
    /// `A1-286` reads the flag twice: once from the raw arguments, before `clap`
    /// runs, so that `clap`'s own usage errors obey it, and once by `clap`
    /// itself. Two parsers of one flag is a drift risk by construction, and the
    /// answer this repository gives elsewhere is to make them answer the same
    /// question and check it, rather than to reason that they will.
    ///
    /// Only the shapes `clap` accepts: where it refuses, there is no second
    /// answer to agree with, and `the_colour_choice_is_read_from_the_raw_arguments`
    /// covers what the pre-scan does with those.
    #[test]
    fn the_pre_scan_agrees_with_clap_on_every_shape_clap_accepts() {
        use aplan_output::color::ColorChoice;

        let shapes: &[&[&str]] = &[
            &["check"],
            &["--color=always", "check"],
            &["--color", "always", "check"],
            &["check", "--color=never"],
            &["check", "--color", "never"],
            &["--color=auto", "check"],
            // Repeated *across* the subcommand, which is the only repeat
            // `clap` accepts for a global flag: twice at one level is
            // "cannot be used multiple times". The later wins, and so must the
            // pre-scan, or one run means two things.
            &["--color=always", "check", "--color=never"],
            &["--color=never", "check", "--color=always"],
            // Around a flag that takes a value of its own, which is the case
            // where a scanner that counted positions would go wrong.
            &["--root", "aplan", "--color=always", "check"],
            &["--color=always", "--root", "aplan", "check"],
            // After the subcommand and after one of *its* flags.
            &["check", "--format", "json", "--color=always"],
            // A bare `--`, after which everything is an operand.
            &["--color=always", "why", "--", "T-001"],
            &["why", "--", "T-001"],
        ];

        let mut agreed = 0_usize;
        for shape in shapes {
            let mut argv = vec!["aplan"];
            argv.extend_from_slice(shape);
            let Ok(matches) = cli_command().try_get_matches_from(&argv) else {
                panic!(
                    "`{}` is not a shape clap accepts, so it does not belong in this list",
                    shape.join(" ")
                );
            };
            let cli = <Cli as clap::FromArgMatches>::from_arg_matches(&matches)
                .expect("clap built its own view");
            let theirs = ColorChoice::parse(&cli.color).expect("clap validated the value");
            let ours = color_from_argv(shape.iter().map(std::ffi::OsString::from));
            assert_eq!(
                ours,
                theirs,
                "the pre-scan and `clap` disagree about `aplan {}`: the terminal \
                 would be set one way and the parsed value would say another",
                shape.join(" ")
            );
            agreed += 1;
        }
        assert!(agreed >= 13, "only {agreed} shapes were compared");
    }

    /// The pre-scan reads what `clap` will read (`A1-286`).
    ///
    /// Two readers of one flag is a drift risk by construction, so the cases
    /// where they could disagree are the ones pinned here: both spellings, a
    /// repeat, a value `clap` will refuse, and an operand that merely looks like
    /// the flag.
    #[test]
    fn the_colour_choice_is_read_from_the_raw_arguments() {
        use aplan_output::color::ColorChoice;
        let scan = |args: &[&str]| color_from_argv(args.iter().map(std::ffi::OsString::from));

        assert_eq!(scan(&["check"]), ColorChoice::Auto, "the default");
        assert_eq!(scan(&["--color=always", "check"]), ColorChoice::Always);
        assert_eq!(scan(&["--color", "always", "check"]), ColorChoice::Always);
        assert_eq!(scan(&["check", "--color=never"]), ColorChoice::Never);

        // `clap` takes the last, so this must too, or a repeated flag means one
        // thing to the terminal and another to the parser.
        assert_eq!(
            scan(&["--color=always", "check", "--color=never"]),
            ColorChoice::Never
        );

        // A value `clap` is about to refuse must not have changed the terminal
        // on the way past.
        assert_eq!(scan(&["--color=sparkles", "check"]), ColorChoice::Auto);
        assert_eq!(scan(&["--color"]), ColorChoice::Auto, "no value at all");

        // After `--` these are operands. A task titled `--color=always` is a
        // title, and `add -- --color=always` must not be a colour request.
        assert_eq!(scan(&["add", "--", "--color=always"]), ColorChoice::Auto);

        // A near miss is not a match, or `--colorful` would set the terminal.
        assert_eq!(scan(&["--colorful=always", "check"]), ColorChoice::Auto);
    }

    /// What a refusal calls the move it is refusing (`A1-171`).
    ///
    /// Pinned directly because it only reaches a reader through
    /// `refuse_bad_mutation`, and a *phase* move cannot produce an error to be
    /// refused for: no rule reads `phase`. A cut move can, and when it does the
    /// sentence has to say the whole of what was asked, not the half that
    /// happened to be refusable.
    #[test]
    fn a_move_is_described_by_everything_it_would_change() {
        let one = vec!["A-01".to_string()];
        let two = vec!["A-01".to_string(), "A-02".to_string()];
        assert_eq!(
            describe_move(&one, Some("M-s1"), Some(3)),
            "move `A-01` to `M-s1` and phase 3"
        );
        assert_eq!(
            describe_move(&one, Some("M-s1"), None),
            "move `A-01` to `M-s1`"
        );
        assert_eq!(describe_move(&one, None, Some(3)), "move `A-01` to phase 3");
        assert_eq!(
            describe_move(&two, Some("M-s1"), Some(3)),
            "move 2 tasks to `M-s1` and phase 3"
        );
    }

    fn report(errors: usize, warnings: usize) -> aplan_core::Report {
        use aplan_core::diagnostic::{Diagnostic, code};
        let mut diagnostics = Vec::new();
        for _ in 0..errors {
            diagnostics.push(Diagnostic::error(code::DONE_WITHOUT_EVIDENCE, "e"));
        }
        for _ in 0..warnings {
            diagnostics.push(Diagnostic::warning(code::MISSING_EFFORT, "w"));
        }
        aplan_core::Report {
            diagnostics,
            tasks_checked: 1,
        }
    }

    /// The agent surface carries `APLAN041` too, which is the half of that rule
    /// nothing else can hold (`A1-273`).
    ///
    /// `mcp_parity.rs` compares the tools against the commands and says in its
    /// own header that a defect in how `aplan mcp` builds its `Options` would
    /// pass it, which is exactly where this rule would go missing: the tools
    /// call `validate` with whatever the server handed them, so an unfilled
    /// `agents_block` reads as a repository whose contract is current. Held here
    /// because it is the only place the wiring itself is callable.
    #[test]
    fn the_mcp_servers_options_carry_the_generated_block() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path().join("aplan");
        std::fs::create_dir_all(root.join("tasks")).expect("plan dir");
        std::fs::write(
            dir.path().join("AGENTS.md"),
            format!(
                "prose\n\n{}\n- an old rule\n{}\n",
                aplan_core::contract::BEGIN,
                aplan_core::contract::END
            ),
        )
        .expect("AGENTS.md");

        let config = aplan_config::Config::default();
        let filled = mcp_agents_block(Options::default(), &root, Some(dir.path()), &config);
        let block = filled
            .agents_block
            .expect("the server must hand the tools a block to compare");
        assert!(
            block.path.ends_with("AGENTS.md"),
            "pointed somewhere else: {}",
            block.path.display()
        );
        assert!(
            !block.current.is_empty(),
            "an empty expectation would call every block stale"
        );
        assert!(
            !block.current.contains(aplan_core::contract::BEGIN),
            "compared against the fence rather than the block: {}",
            block.current
        );
    }

    /// And `cmd_mcp` actually calls it.
    ///
    /// Read from the source, which is not this repository's habit and is the
    /// only thing available here: the server is driven over stdio by a client,
    /// `mcp_parity.rs` calls the tools layer *underneath* this wiring, and the
    /// test above holds the function rather than the call -- deleting the call
    /// leaves every one of them green (measured, `A1-273`). What is asserted is
    /// narrow on purpose: that this one function is named inside this one body.
    #[test]
    fn cmd_mcp_calls_it() {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
        )
        .expect("read this file");
        let start = src
            .find("\nfn cmd_mcp(")
            .expect("cmd_mcp is still called that");
        let body = &src[start + 1..];
        let end = body.find("\nfn ").unwrap_or(body.len());
        assert!(
            body[..end].contains("mcp_agents_block("),
            "the MCP server no longer fills `agents_block`, so `aplan_check` would report a \
             stale contract as current -- the surface that reads the contract"
        );
    }

    /// A policy with no advisory rules, for the truth table below.
    fn deny(deny_warnings: bool) -> aplan_core::Verdict {
        aplan_core::Verdict {
            deny_warnings,
            advisory: Vec::new(),
        }
    }

    #[test]
    fn a_warning_blocks_only_when_the_repository_asked_for_that() {
        assert_eq!(verdict(&report(0, 0), &deny(false)), EXIT_OK);
        assert_eq!(
            verdict(&report(0, 3), &deny(false)),
            EXIT_OK,
            "advisory by default"
        );
        assert_eq!(verdict(&report(0, 3), &deny(true)), EXIT_VIOLATIONS);
        // A clean plan is clean under either policy: a strict flag that failed a
        // plan with nothing wrong would be a reason never to turn it on.
        assert_eq!(verdict(&report(0, 0), &deny(true)), EXIT_OK);
    }

    #[test]
    fn denying_warnings_does_not_invent_a_third_exit_code() {
        // ADR-0007: `1` means "at least one finding". A repository that turns
        // warnings into findings must not make its consumers learn a new code.
        assert_eq!(verdict(&report(2, 0), &deny(false)), EXIT_VIOLATIONS);
        assert_eq!(verdict(&report(2, 0), &deny(true)), EXIT_VIOLATIONS);
        assert_eq!(verdict(&report(2, 3), &deny(true)), EXIT_VIOLATIONS);
    }

    #[test]
    fn an_explicit_flag_wins_over_the_file_in_both_directions() {
        let off = Config::default();
        let on = Config {
            deny_warnings: Some(true),
            ..Config::default()
        };
        assert!(!resolve_deny_warnings(&off, false, false), "default is off");
        assert!(resolve_deny_warnings(&on, false, false), "the file decides");
        assert!(resolve_deny_warnings(&off, true, false), "--deny-warnings");
        assert!(
            !resolve_deny_warnings(&on, false, true),
            "--no-deny-warnings must be able to relax a repository policy for one run"
        );
    }

    /// **A field nothing could learn is left out** (`A1-256`).
    ///
    /// A binary built from a tarball has no commit to name, and one that cannot
    /// read its own image has no digest. An empty string reads as an answer, and
    /// a reader scripting against the document would have to know which value
    /// means "unknown".
    ///
    /// Tested here rather than against the running binary, because a build from
    /// a checkout populates every field: the rule is invisible from there, which
    /// is how the first version of this test passed with the rule deleted.
    #[test]
    fn an_identity_leaves_out_what_it_could_not_learn() {
        let known = aplan_attest::ToolIdentity {
            version: "0.3.0-dev".into(),
            source: "abc123".into(),
            facts_sha256: "f".repeat(64),
            binary_sha256: "b".repeat(64),
        };
        let doc: serde_json::Value = serde_json::from_str(&identity_json(&known)).expect("json");
        assert_eq!(doc["source"], "abc123");
        assert_eq!(doc["binary_sha256"], "b".repeat(64));

        let tarball = aplan_attest::ToolIdentity {
            source: String::new(),
            binary_sha256: String::new(),
            ..known
        };
        let doc: serde_json::Value = serde_json::from_str(&identity_json(&tarball)).expect("json");
        assert!(
            doc.get("source").is_none() && doc.get("binary_sha256").is_none(),
            "a field with nothing to say is absent: {doc}"
        );
        assert_eq!(
            doc["version"], "0.3.0-dev",
            "and the ones that can be answered still are: {doc}"
        );
    }

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
