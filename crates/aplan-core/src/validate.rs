//! The invariants. This is the whole point of the tool.
//!
//! A tracker that lets you write `status: done` and walk away is a tracker that
//! lies. Every rule here exists because a plan can assert something no one
//! checked.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::diagnostic::{Diagnostic, Report, code};
use crate::graph::Graph;
use crate::load::{LoadedTask, Plan};
use crate::task::{EvidenceCapture, EvidenceKind, Status, is_vacuous_ref};

/// Deterministic traced-document work accumulated on the current thread.
///
/// Like [`crate::graph_constructions`], callers measure one synchronous
/// operation by subtracting the value before it from the value afterwards. The
/// counters expose work only and retain no document or plan state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TraceDocumentWork {
    /// Successful source reads, including documents that later fail to parse.
    pub reads: usize,
    /// UTF-8 source bytes returned by those reads.
    pub bytes_read: u64,
    /// Markdown index construction attempts.
    pub markdown_parses: usize,
    /// YAML or JSON parse attempts, successful or not.
    pub structured_parses: usize,
}

std::thread_local! {
    static TRACE_DOCUMENT_WORK: std::cell::Cell<TraceDocumentWork> =
        const { std::cell::Cell::new(TraceDocumentWork {
            reads: 0,
            bytes_read: 0,
            markdown_parses: 0,
            structured_parses: 0,
        }) };
}

/// Cumulative traced-document work on the current thread.
#[must_use]
pub fn trace_document_work() -> TraceDocumentWork {
    TRACE_DOCUMENT_WORK.get()
}

fn record_trace_document(text: &str, format: Option<crate::structured::Format>) {
    TRACE_DOCUMENT_WORK.set(TRACE_DOCUMENT_WORK.get().then_one(text, format));
}

impl TraceDocumentWork {
    fn then_one(mut self, text: &str, format: Option<crate::structured::Format>) -> Self {
        self.reads = self.reads.saturating_add(1);
        self.bytes_read = self
            .bytes_read
            .saturating_add(u64::try_from(text.len()).unwrap_or(u64::MAX));
        match format {
            Some(_) => self.structured_parses = self.structured_parses.saturating_add(1),
            None => self.markdown_parses = self.markdown_parses.saturating_add(1),
        }
        self
    }
}

enum TraceDocument {
    Unreadable,
    Markdown(crate::anchor::MarkdownDocument),
    Structured {
        format: crate::structured::Format,
        document: Option<crate::structured::Document>,
    },
}

impl TraceDocument {
    fn fingerprint(&self, anchor: &str) -> Option<String> {
        match self {
            Self::Unreadable | Self::Structured { document: None, .. } => None,
            Self::Markdown(document) => document.fingerprint(anchor),
            Self::Structured {
                document: Some(document),
                ..
            } => document.fingerprint(&crate::structured::parse_anchor(anchor).ok()?),
        }
    }

    fn holds(&self, anchor: &str, stored: &str) -> Option<bool> {
        match self {
            Self::Unreadable | Self::Structured { document: None, .. } => None,
            Self::Markdown(document) => document.holds(anchor, stored),
            Self::Structured {
                document: Some(document),
                ..
            } => Some(
                document.fingerprint(&crate::structured::parse_anchor(anchor).ok()?)? == stored,
            ),
        }
    }
}

#[derive(Default)]
struct TraceDocuments {
    by_path: BTreeMap<(PathBuf, Option<crate::structured::Format>), TraceDocument>,
}

impl TraceDocuments {
    fn get(&mut self, root: &Path, rel: &str) -> &TraceDocument {
        let path = root.join(rel);
        let format = crate::structured::format_for(rel);
        let key_path = path.canonicalize().unwrap_or_else(|_| comparable(&path));
        self.by_path.entry((key_path, format)).or_insert_with(|| {
            let Ok(text) = std::fs::read_to_string(&path) else {
                return TraceDocument::Unreadable;
            };
            record_trace_document(&text, format);
            match format {
                Some(format) => TraceDocument::Structured {
                    format,
                    document: crate::structured::Document::parse(&text, format),
                },
                None => TraceDocument::Markdown(crate::anchor::MarkdownDocument::parse(text)),
            }
        })
    }

    fn fingerprint(&mut self, root: &Path, trace: &str) -> Option<String> {
        let (rel, anchor) = trace.split_once('#')?;
        self.get(root, rel).fingerprint(anchor)
    }

    fn holds(&mut self, root: &Path, trace: &str, stored: &str) -> Option<bool> {
        let (rel, anchor) = trace.split_once('#')?;
        self.get(root, rel).holds(anchor, stored)
    }
}

struct ValidationContext<'a> {
    opts: &'a Options,
    traces: TraceDocuments,
}

impl<'a> ValidationContext<'a> {
    fn new(opts: &'a Options) -> Self {
        Self {
            opts,
            traces: TraceDocuments::default(),
        }
    }
}

/// Select the capture rule from the exhaustive evidence-kind model.
fn capture_detector(kind: EvidenceKind) -> &'static str {
    kind.capabilities().capture.detector()
}

/// A declared tag vocabulary as the rules see it.
///
/// The config's two shapes, a bare list and a mapping, arrive here as one type:
/// a list is `Optional` over every task, which is what it has always meant.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TagFamily {
    /// The tags this family contains, as whole tags.
    pub members: Vec<String>,
    /// How many of them a task in scope carries.
    pub require: Cardinality,
    /// Which statuses the requirement applies to.
    pub status: StatusScope,
    /// Whether milestone tasks are subject to it.
    pub milestones: MilestoneScope,
}

impl TagFamily {
    /// A family with no cardinality, which is what a bare list declares.
    #[must_use]
    pub fn members(members: Vec<String>) -> Self {
        Self {
            members,
            require: Cardinality::Optional,
            status: StatusScope::Any,
            milestones: MilestoneScope::Include,
        }
    }

    /// The declared members this task carries, deduplicated by the declaration.
    ///
    /// Only declared members: a tag that merely looks like one is `APLAN026`'s
    /// finding and is not evidence the requirement was met, or the rule would
    /// accept a typo as proof.
    #[must_use]
    pub fn carried<'a>(&'a self, task: &crate::task::Task) -> Vec<&'a str> {
        self.members
            .iter()
            .filter(|m| task.tags.iter().any(|t| t == *m))
            .map(String::as_str)
            .collect()
    }

    /// Whether this task meets the requirement, which it trivially does when the
    /// family does not judge it.
    ///
    /// One implementation, because there were two: the rule counted here and
    /// `aplan add` counted again to refuse before writing, and the MCP `add`
    /// counted nowhere at all, so an agent could write through one surface what
    /// the other refused (`A1-133`).
    #[must_use]
    pub fn satisfied_by(&self, task: &crate::task::Task) -> bool {
        if !self.in_scope(task) {
            return true;
        }
        let n = self.carried(task).len();
        match self.require {
            Cardinality::Optional => true,
            Cardinality::AtMostOne => n <= 1,
            Cardinality::AtLeastOne => n >= 1,
            Cardinality::ExactlyOne => n == 1,
        }
    }

    /// Whether this family's cardinality judges the given task.
    #[must_use]
    pub fn judges(&self, task: &crate::task::Task) -> bool {
        self.require != Cardinality::Optional && self.in_scope(task)
    }

    /// Whether this task is one the family's scope covers, cardinality aside.
    ///
    /// Split from [`judges`](Self::judges) so `satisfied_by`'s `Optional` arm is
    /// reachable: with the requirement folded into the scope test, that arm was
    /// dead code the compiler still demanded, which is the shape these audits
    /// keep deleting (`A1-113`).
    fn in_scope(&self, task: &crate::task::Task) -> bool {
        let by_status = match self.status {
            StatusScope::Any => true,
            StatusScope::Live => !matches!(task.status, Status::Done | Status::Dropped),
            StatusScope::Done => task.status == Status::Done,
            StatusScope::Dropped => task.status == Status::Dropped,
            StatusScope::Terminal => matches!(task.status, Status::Done | Status::Dropped),
        };
        let milestone = task.is_milestone();
        let by_kind = match self.milestones {
            MilestoneScope::Include => true,
            MilestoneScope::Exclude => !milestone,
            MilestoneScope::Only => milestone,
        };
        by_status && by_kind
    }
}

/// How many members of a family a task in scope must carry (`APLAN030`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Cardinality {
    /// Any number, including none.
    Optional,
    /// At most one.
    AtMostOne,
    /// At least one.
    AtLeastOne,
    /// Exactly one.
    ExactlyOne,
}

impl Cardinality {
    /// How the requirement reads in a message.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Cardinality::Optional => "optional",
            Cardinality::AtMostOne => "at-most-one",
            Cardinality::AtLeastOne => "at-least-one",
            Cardinality::ExactlyOne => "exactly-one",
        }
    }
}

/// Which statuses a cardinality applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum StatusScope {
    /// Every status.
    Any,
    /// Work that has not finished: `todo`, `in_progress`, `blocked`.
    Live,
    /// `done` only.
    Done,
    /// `dropped` only.
    Dropped,
    /// `done` or `dropped`.
    Terminal,
}

impl StatusScope {
    /// How the scope reads in a message.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            StatusScope::Any => "every",
            StatusScope::Live => "live",
            StatusScope::Done => "done",
            StatusScope::Dropped => "dropped",
            StatusScope::Terminal => "terminal",
        }
    }
}

/// Whether milestone tasks are subject to a cardinality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum MilestoneScope {
    /// Milestones are judged like any other task.
    Include,
    /// Milestones are exempt.
    Exclude,
    /// Only milestones.
    Only,
}

/// A generated agent contract, and the file that is meant to carry it.
///
/// Two halves of one fact, so a caller cannot supply a block without saying
/// where it belongs, or a path without what should be in it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AgentsBlock {
    /// The document carrying the markers, as a reader would name it.
    pub path: std::path::PathBuf,
    /// What `export-agents-md` would write between them now.
    pub current: String,
}

/// Options that soften rules for early-stage plans.
///
/// `PartialEq` so a caller can ask whether a switch it was about to flip is
/// already on: `vacuity` runs the rules once per switch to measure what each is
/// stopping, and a run that could reveal nothing is pure cost (`A1-269`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Options {
    /// Require an `effort_days` estimate on non-terminal, non-milestone tasks.
    pub require_effort: bool,
    /// Whether a closed task has to account for what it does to the released
    /// version, and the per-level floors it is held to (`A1-312`, ADR-0034).
    ///
    /// `None` is the requirement switched off, which is the default: a plan that
    /// has not asked for version discipline sees `APLAN044` never. `Some`
    /// carries the floors already resolved, so this crate never learns what
    /// `.aplan.yml` looks like. Named for the key rather than for what it holds,
    /// because `vacuity` reports the key an adopter would set.
    pub require_impact: Option<crate::task::VersionFloors>,
    /// Root against which `traces` paths are resolved. `None` disables
    /// [`code::DANGLING_TRACE`] entirely.
    pub traces_root: Option<std::path::PathBuf>,
    /// Repository boundary used by [`code::EXTERNAL_TRACE`] (`APLAN034`).
    ///
    /// This is deliberately separate from [`Self::traces_root`], which says
    /// where a trace is resolved, and from [`Self::config_file`], which says
    /// where policy and its diagnostic location came from. `None` retains the
    /// library-level default of treating `traces_root` as the boundary.
    pub traces_repository: Option<std::path::PathBuf>,
    /// Roots outside [`Self::traces_repository`] that a `traces` target may name
    /// (`APLAN034`, `A1-143`). Empty means a trace may not leave the repository,
    /// which is the default.
    pub traces_external: Vec<std::path::PathBuf>,
    /// Evidence kinds strong enough to close a task. `Some(kinds)` turns on
    /// [`code::WEAK_EVIDENCE`] (`APLAN017`): a `done` task must carry at least one
    /// evidence of one of these kinds. `None` disables the rule.
    pub require_min_strength: Option<Vec<EvidenceKind>>,
    /// Evidence policy scoped to part of the plan, narrowest match winning
    /// (`A1-183`). Empty means every dial here is whole-plan, which is what it
    /// was before scoping existed.
    pub evidence_scopes: Vec<EvidenceScope>,
    /// Tags that exempt a task from [`code::WEAK_EVIDENCE`], for work where
    /// nothing stronger than a note exists (a phone call, a regulator letter).
    pub exempt_tags: Vec<String>,
    /// Turn on drift detection ([`code::TRACE_DRIFT`], `APLAN018`): compare a
    /// `done` task's stored `trace_fingerprints` against the live traced
    /// sections. Off by default; needs `traces_root` to read the documents.
    pub detect_drift: bool,
    /// Turn on membership totality ([`code::MEMBERSHIP_TOTALITY`], `APLAN022`):
    /// every non-milestone task must be a member of exactly one cut, unless
    /// tagged `cross-cutting`. Off by default.
    pub require_cut_membership: bool,
    /// Declared tag vocabularies, family name to declaration (`APLAN026`,
    /// `APLAN029`, `APLAN030`). Empty means the repository declared none, and
    /// every tag stays legal.
    pub tag_families: BTreeMap<String, TagFamily>,
    /// Canonical task owners (`APLAN056`). Empty means the repository declared
    /// no vocabulary and every owner spelling stays legal.
    pub owners: BTreeSet<String>,
    /// The generated agent-contract block, for [`code::STALE_AGENT_CONTRACT`]
    /// (`APLAN041`). `None` disables the rule, and is what a caller that cannot
    /// render one passes.
    ///
    /// The *rendered block* rather than the inputs it came from, because
    /// rendering it needs the config, the plan and the MCP tool list, and a
    /// validator that assembled those would be a second implementation of
    /// `export-agents-md` -- which is the defect this rule exists to catch,
    /// rebuilt inside the rule (`A1-273`).
    pub agents_block: Option<AgentsBlock>,
    /// The `.aplan.yml` the options came from, when there was one.
    ///
    /// Only `APLAN029` reads it, and only to point at the file its finding is
    /// about. Every other rule concerns a task and names that task's file; this
    /// one concerns a declaration, and without the path it was the one rule in
    /// twenty-nine that could point nowhere, so a second unused family rendered
    /// as a bare sentence with no location and no fix line, `check` printing that
    /// once per rule (`A1-085`, `A1-086`).
    pub config_file: Option<std::path::PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            require_effort: true,
            require_impact: None,
            traces_root: None,
            traces_repository: None,
            traces_external: Vec::new(),
            require_min_strength: None,
            evidence_scopes: Vec::new(),
            exempt_tags: Vec::new(),
            detect_drift: false,
            require_cut_membership: false,
            tag_families: BTreeMap::new(),
            owners: BTreeSet::new(),
            agents_block: None,
            config_file: None,
        }
    }
}

/// Run every invariant over a loaded plan.
/// Give each finding the line of the field its rule is about.
///
/// Public because `verify` needs it too. The evidence half builds its own
/// diagnostics from its own resolvers, and without this a single `verify` run
/// answered two ways about one file: `APLAN023`, raised by the structural half,
/// named the `evidence:` line, while `APLAN011` about that same field named
/// nothing. "One tool, one run, two answers" is the complaint `A1-202` was
/// filed about, and it was reproducible inside the task's own fix.
///
/// One pass at the end, rather than at each of the sixty-odd sites that build a
/// diagnostic. A finding already carries the code and the task, which is
/// everything the answer needs, and a location added at fifty sites and
/// forgotten at ten would be worse than none: a reader cannot tell "this rule
/// has no line" from "this rule was missed".
///
/// Three things stop a finding from getting one, and each is a real answer:
/// the rule is about no single field ([`crate::diagnostic::Rule::points_at`] is `None`), the task
/// has no source text (it was built in memory by a mutating command), or the
/// field is simply absent from the frontmatter -- `APLAN005` is *about* missing
/// evidence, so there is no `evidence:` line to point at.
pub fn locate_findings(diagnostics: &mut [Diagnostic], plan: &Plan) {
    // Keyed by *path*, not by task id. An id is not unique in exactly the plan
    // that reports `APLAN002`, and collecting by id keeps the last of a
    // collision: with three files claiming one id, the finding about the second
    // took the third file's line number and pointed a reader at a line that is
    // not in the file the finding names.
    let fields_of: BTreeMap<&Path, &crate::load::FieldLines> = plan
        .tasks
        .iter()
        .map(|lt| (lt.path.as_path(), &lt.fields))
        .collect();

    for d in diagnostics.iter_mut() {
        // The loader already placed a parse failure at the line the parser
        // named, which is better than any field could be: the file did not
        // parse, so it has no fields.
        if d.location.is_some() {
            continue;
        }
        // The file, not the task: a location is a place in a file, and a
        // finding that names no file has nowhere to put one.
        let (Some(file), Some(rule)) = (
            d.file.as_deref(),
            crate::diagnostic::Rule::from_code(d.code),
        ) else {
            continue;
        };
        let Some(field) = rule.points_at() else {
            continue;
        };
        if let Some(location) = fields_of.get(file).and_then(|f| f.get(field)) {
            d.location = Some(*location);
        }
    }
}

/// Evidence policy that applies to a subset of the plan (`A1-183`).
///
/// Built from `.aplan.yml`'s `policy` list. The selector is the one the command
/// line uses (`A1-214`, ADR-0026), so a subset means the same thing whether it is
/// written in a file or typed as flags.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct EvidenceScope {
    /// The subset this applies to.
    pub selector: crate::select::Selector,
    /// How many dimensions the scope names, which is what decides precedence: the
    /// narrowest match wins, and the whole-plan settings name none.
    pub named_dimensions: usize,
    /// The kinds strong enough to close a task here. `Some(vec![])` is a real
    /// answer -- "nothing is required in this subset" -- and is how
    /// `evidence.exempt_tags` is expressible in this form.
    pub require_min_strength: Option<Vec<EvidenceKind>>,
    /// Tags exempt from the requirement here.
    pub exempt_tags: Option<Vec<String>>,
}

impl Options {
    /// The evidence policy that applies to `task`, narrowest scope first.
    ///
    /// **Narrowest wins; among equals, the first entry in the file.** Both halves
    /// are stated because both are guesses a reader would otherwise have to make.
    /// Counting dimensions is the usual reading of "more specific", and taking
    /// the earlier entry when two are equally specific is the rule that needs no
    /// merge: a reader resolves it by reading down the list, and nothing depends
    /// on the order a map happened to iterate in.
    #[must_use]
    pub fn evidence_for(
        &self,
        task: &crate::task::Task,
        cut: Option<&str>,
        milestone: Option<&str>,
    ) -> (Option<&[EvidenceKind]>, &[String]) {
        // **Per key, not per scope.** A scope that adjusts one dial must not
        // silently reset the other to the whole-plan value: each key comes from
        // the narrowest matching scope that *sets* it, and only falls to the
        // whole-plan value when no matching scope does. Resolving the winning
        // scope as a whole was the first version, and it made a scope setting
        // only `exempt_tags` quietly discard a wider scope's requirement
        // (`A1-183`'s audit).
        //
        // Not `max_by_key` either: it returns the *last* of several equal
        // maxima, which would resolve a tie by the opposite of the documented
        // rule.
        fn narrowest<'a, T>(
            scopes: &'a [EvidenceScope],
            task: &crate::task::Task,
            cut: Option<&str>,
            milestone: Option<&str>,
            get: impl Fn(&'a EvidenceScope) -> Option<&'a T>,
        ) -> Option<&'a T> {
            let mut best: Option<(usize, &T)> = None;
            for scope in scopes {
                if let Some(value) = get(scope)
                    && scope.selector.matches(task, cut, milestone)
                    && best.is_none_or(|(dims, _)| scope.named_dimensions > dims)
                {
                    best = Some((scope.named_dimensions, value));
                }
            }
            best.map(|(_, v)| v)
        }

        let strength = narrowest(&self.evidence_scopes, task, cut, milestone, |s| {
            s.require_min_strength.as_ref()
        })
        .map(Vec::as_slice)
        .or(self.require_min_strength.as_deref());
        let exempt = narrowest(&self.evidence_scopes, task, cut, milestone, |s| {
            s.exempt_tags.as_ref()
        })
        .map_or(self.exempt_tags.as_slice(), Vec::as_slice);
        (strength, exempt)
    }
}

/// Which cut each task is in, keyed by task id, built once.
///
/// A scoped policy asks this per task (`A1-183`), and asking the graph each time
/// is the shape `A1-125` and `A1-227` both had to undo.
fn cut_index(plan: &Plan) -> BTreeMap<&str, (&str, &str)> {
    let mut out: BTreeMap<&str, (&str, &str)> = BTreeMap::new();
    for m in plan
        .tasks
        .iter()
        .map(|lt| &lt.task)
        .filter(|t| t.is_milestone())
    {
        for member in &m.blocked_by {
            out.insert(
                member.as_str(),
                (m.id.as_str(), crate::roadmap::cut_label(m)),
            );
        }
    }
    out
}

pub fn validate(plan: &Plan, opts: &Options) -> Report {
    let tasks: Vec<_> = plan.tasks.iter().map(|loaded| &loaded.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    validate_with_graph(plan, opts, &graph)
}

/// Validate a plan with graph state the enclosing operation already built.
///
/// The graph must describe `plan`. Keeping the borrowing graph at the call site
/// avoids a self-referential plan owner while letting an operation such as
/// `show` or the interactive snapshot use the same derived graph for validation
/// and projection.
#[must_use]
pub fn validate_with_graph(plan: &Plan, opts: &Options, graph: &Graph<'_>) -> Report {
    let mut report = Report {
        diagnostics: plan.load_diagnostics.clone(),
        tasks_checked: plan.tasks.len(),
    };

    // Duplicate ids. Detected before building the graph, which dedupes.
    let mut seen: BTreeMap<&str, &Path> = BTreeMap::new();
    for lt in &plan.tasks {
        if let Some(first) = seen.insert(lt.task.id.as_str(), &lt.path) {
            report.diagnostics.push(
                Diagnostic::error(
                    code::DUPLICATE_ID,
                    format!("duplicate task id, first defined in {}", first.display()),
                )
                .with_task(&lt.task.id)
                .with_file(&lt.path)
                .with_agent_instruction("Rename one of the two tasks to a unique id."),
            );
        }
    }

    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let file_of: BTreeMap<&str, &Path> = plan
        .tasks
        .iter()
        .map(|lt| (lt.task.id.as_str(), lt.path.as_path()))
        .collect();

    let cut_of = cut_index(plan);
    let mut context = ValidationContext::new(opts);

    for lt in &plan.tasks {
        check_task(
            lt,
            plan,
            graph,
            &mut context,
            &cut_of,
            &mut report.diagnostics,
        );
    }

    // Cycles. Reported once for the whole graph, not once per member.
    //
    // `ratified_by` joins the same question. "X must be done before Y can be
    // done" is what both edges assert -- one about starting, one about
    // finishing -- so a loop through either is a set of tasks none of which can
    // ever be closed. Checked only when the `blocked_by` graph is acyclic, so
    // one loop is never reported twice (`A1-306`, audit).
    if graph.find_cycle().is_none() {
        report_ratification_cycle(plan, &file_of, &mut report.diagnostics);
        check_contains(plan, &file_of, &mut report.diagnostics);
    }
    if let Some(cycle) = graph.find_cycle() {
        let path = cycle.join(" → ");
        let anchor = cycle.first().cloned().unwrap_or_default();
        let mut d = Diagnostic::error(code::CYCLE, format!("dependency cycle: {path}"))
            .with_task(&anchor)
            .with_agent_instruction(
                "Break the cycle by removing one `blocked_by` edge; a plan with a cycle has no \
                 valid execution order.",
            );
        if let Some(p) = file_of.get(anchor.as_str()) {
            d = d.with_file(*p);
        }
        report.diagnostics.push(d);
    }

    check_membership_totality(plan, opts, &tasks, &mut report.diagnostics);

    for lt in &plan.tasks {
        check_revisit(lt, &mut report.diagnostics);
    }
    check_vocabularies(plan, opts, &mut report.diagnostics);
    check_agents_block(opts, &mut report.diagnostics);
    check_cancelled_cuts(plan, graph, &mut report.diagnostics);
    check_preference_cycles(plan, graph, &mut report.diagnostics);

    // A finding names the line it is about, not only the file (`A1-202`).
    locate_findings(&mut report.diagnostics, plan);

    // Stable output: a *total* order, so two diagnostics that tie on
    // (code, task) can't reorder with the filesystem walk across CI runners.
    report.diagnostics.sort_by(|a, b| {
        a.code
            .cmp(b.code)
            .then(a.task.cmp(&b.task))
            .then(a.file.cmp(&b.file))
            .then(a.message.cmp(&b.message))
    });
    report
}

/// APLAN022: every non-milestone task belongs to exactly one cut unless its
/// `cross-cutting` tag says that it deliberately belongs to none.
fn check_membership_totality(
    plan: &Plan,
    opts: &Options,
    tasks: &[&crate::task::Task],
    out: &mut Vec<Diagnostic>,
) {
    if !opts.require_cut_membership {
        return;
    }
    let mut cut_count: BTreeMap<&str, usize> = BTreeMap::new();
    for task in tasks.iter().filter(|task| task.is_milestone()) {
        for blocker in &task.blocked_by {
            *cut_count.entry(blocker.as_str()).or_default() += 1;
        }
    }
    for loaded in &plan.tasks {
        let task = &loaded.task;
        if task.is_milestone() {
            continue;
        }
        let count = cut_count.get(task.id.as_str()).copied().unwrap_or(0);
        // The waiver applies only where its claim is true (`A1-212`):
        // `cross-cutting` means that this task belongs to no cut, not that
        // duplicate membership is acceptable.
        if count == 0 && task.tags.iter().any(|tag| tag == "cross-cutting") {
            continue;
        }
        if count != 1 {
            let message = if count == 0 {
                "belongs to no cut".to_string()
            } else {
                format!("belongs to {count} cuts (a task belongs to exactly one)")
            };
            out.push(
                Diagnostic::error(code::MEMBERSHIP_TOTALITY, message)
                    .with_task(&task.id)
                    .with_file(&loaded.path)
                    .with_agent_instruction(
                        "Add the task to exactly one milestone's `blocked_by` (its cut), or \
                         tag it `cross-cutting` if it deliberately belongs to none.",
                    ),
            );
        }
    }
}

/// The declared and built-in task vocabularies, kept together so the top-level
/// validation pass remains an inventory of domains rather than individual
/// rules.
fn check_vocabularies(plan: &Plan, opts: &Options, out: &mut Vec<Diagnostic>) {
    check_tag_families(plan, opts, out);
    check_owner_vocabulary(plan, opts, out);
    check_reserved_tags(plan, opts, out);
}

/// Per-task invariants.
///
/// Split out of [`validate`] so each function stays readable: the graph-wide
/// checks (duplicate ids, cycles) belong to the caller, which owns the whole
/// plan; everything here needs only one task plus the graph to look blockers up.
fn check_task(
    lt: &LoadedTask,
    plan: &Plan,
    graph: &Graph<'_>,
    context: &mut ValidationContext<'_>,
    cut_of: &BTreeMap<&str, (&str, &str)>,
    out: &mut Vec<Diagnostic>,
) {
    let t = &lt.task;
    let opts = context.opts;

    check_edges(lt, graph, &plan.unloadable, out);
    check_evidence(lt, opts, cut_of, out);
    check_path_globs(lt, out);
    check_duplicate_paths(lt, out);
    check_preferences(lt, graph, out);
    check_criteria(lt, out);
    check_scaffold(lt, out);
    check_scaffold_leftover(lt, out);
    check_version_impact(lt, opts, out);
    check_version_record(lt, out);

    // Effort, for phase rollups. Milestones are checkpoints, not work: they
    // carry no effort by design (and are excluded from every rollup), so asking
    // one for an estimate would be noise.
    if opts.require_effort
        && !t.status.is_terminal()
        && !t.is_milestone()
        && t.effort_days.is_none()
    {
        out.push(
            Diagnostic::warning(code::MISSING_EFFORT, "no `effort_days` estimate")
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction("Add an `effort_days` estimate, or drop the task."),
        );
    }

    check_milestone_shape(lt, graph, out);

    // Traces must resolve. This is the link that stops a research document
    // and a plan from silently diverging.
    if let Some(root) = &opts.traces_root {
        // Deduplicate, exactly as `check_edges` does for `blocked_by`: a trace
        // listed twice is one relationship, and two identical diagnostics is just
        // noise (two identical annotations under `--format github`).
        let traces: BTreeSet<&str> = t.traces.iter().map(String::as_str).collect();

        // APLAN028: a closed task that stamped *some* of its traces. Checked
        // before the drift loop, because the loop is precisely what goes quiet.
        if let Some(d) = check_trace_stamp(root, t, &traces, opts, &mut context.traces) {
            out.push(d.with_task(&t.id).with_file(&lt.path));
        }

        for tr in traces {
            // Leaving the repository is asked first: a target outside it
            // resolves perfectly whenever the sibling is checked out, so
            // `check_trace` would say nothing at all (`A1-143`).
            if let Some(diag) = check_external_trace(root, opts, tr) {
                out.push(diag.with_task(&t.id).with_file(&lt.path));
                continue;
            }
            if let Some(diag) = check_trace(root, tr, &mut context.traces) {
                out.push(diag.with_task(&t.id).with_file(&lt.path));
            }
            // APLAN018: a closed task whose traced section has drifted from the
            // fingerprint it stamped at close time. Advisory, and only where a
            // stamp exists (no stamp is unverifiable, not a violation).
            if opts.detect_drift
                && t.status == Status::Done
                && let Some(stamp) = t.trace_fingerprints.get(tr)
                && let Some(diag) =
                    check_drift(root, tr, stamp, sole_commit(t), &mut context.traces)
            {
                out.push(diag.with_task(&t.id).with_file(&lt.path));
            }
        }
    }
}

/// `APLAN041`: the generated block in `AGENTS.md` no longer says what the policy
/// says.
///
/// The comparison is on the text between the markers, trimmed at both ends, so a
/// trailing newline the writer does or does not leave is not a finding: what an
/// agent reads is the block, and whitespace around it is the file's.
fn check_agents_block(opts: &Options, out: &mut Vec<Diagnostic>) {
    let Some(want) = &opts.agents_block else {
        return;
    };
    // A file that is not there, or that cannot be read, carries no block -- the
    // same silence as one carrying no markers, through the same branch. Written
    // as one path deliberately: an early return here would guarantee that
    // silence twice, and a guarantee with two implementations is one no single
    // change can be shown to break (`A1-273`, found by mutating it).
    let text = std::fs::read_to_string(&want.path).unwrap_or_default();
    let file = want.path.display();
    let diag = match crate::contract::block_in(&text) {
        crate::contract::Block::Malformed { begins, ends } => Diagnostic::warning(
            code::STALE_AGENT_CONTRACT,
            format!(
                "{begins} `aplan:begin` and {ends} `aplan:end` marker(s), so the generated \
                 block cannot be read and nothing can say whether it is current"
            ),
        )
        .with_agent_instruction(format!(
            "Leave one `aplan:begin` and one `aplan:end`, in that order, then run `aplan \
             export-agents-md --write {file}`."
        )),
        // A block that says the same thing and names a different build is not a
        // stale policy, and saying so sends the reader looking for a policy that
        // did not change (`A1-325`). It is still reported: two builds writing
        // one committed artifact is how the pilot got a thirty-line diff that
        // could not be reproduced where it was made.
        crate::contract::Block::Present(found)
            if crate::contract::differs_only_by_build(found, &want.current) =>
        {
            Diagnostic::warning(
                code::STALE_AGENT_CONTRACT,
                "the generated block says what this repository's policy says, and names a \
                 different build as its author: the block was written by one `aplan` and is \
                 being checked by another",
            )
            .with_agent_instruction(format!(
                "Run `aplan export-agents-md --write {file}` with the build this repository \
                 pins, so the committed block and the one CI generates come from the same \
                 `aplan`. If you run mixed builds deliberately, put `APLAN041` in `advisory` \
                 so this reports without failing."
            ))
        }
        crate::contract::Block::Present(found) if found.trim() != want.current.trim() => {
            Diagnostic::warning(
                code::STALE_AGENT_CONTRACT,
                "the generated block is not what this repository's policy produces now: it is \
                 what an agent reads before touching the plan, so a stale one teaches a rule \
                 the tool would refuse",
            )
            .with_agent_instruction(format!("Run `aplan export-agents-md --write {file}`."))
        }
        // `Absent`: the markers are the request, and this file makes none, so
        // nothing here has standing. `Present`: it already says what the policy
        // says. Both variants named rather than a wildcard, so a new one is a
        // compile error and not a silent third silence.
        crate::contract::Block::Absent | crate::contract::Block::Present(_) => return,
    };
    out.push(diag.with_file(&want.path));
}

/// `APLAN042`: a `revisit_at` is usable, and on a task it can mean anything for.
fn check_revisit(lt: &LoadedTask, out: &mut Vec<Diagnostic>) {
    let t = &lt.task;
    let Some(at) = &t.revisit_at else {
        return;
    };
    if rfc3339_to_epoch(at).is_none() {
        out.push(
            Diagnostic::warning(
                code::REVISIT_UNUSABLE,
                format!("`revisit_at: {at}` is not an RFC 3339 instant, so nothing can act on it"),
            )
            .with_agent_instruction(
                concat!(
                    "Write the instant as `YYYY-MM-DDTHH:MM:SSZ`, or with an offset ",
                    "(`YYYY-MM-DDTHH:MM:SS+02:00`). A date alone is not an instant: a ",
                    "scheduler has to know which moment of the day to return at.",
                )
                .to_string(),
            )
            .with_task(&t.id)
            .with_file(&lt.path),
        );
        return;
    }
    // Parked is the only status it means anything for. `ready` already offers a
    // `todo` task, `done` and `dropped` are over, and `due` reports parked work:
    // a revisit anywhere else is a note to nobody.
    if t.status != Status::Blocked {
        out.push(
            Diagnostic::warning(
                code::REVISIT_UNUSABLE,
                format!(
                    "carries `revisit_at` and is `{}`: a revisit says when to look at *parked* \
                     work again, and nothing reads it here",
                    t.status
                ),
            )
            .with_agent_instruction(
                concat!(
                    "Park the task (`status: blocked`) if it is waiting on something outside ",
                    "the plan, or drop the `revisit_at`. Reaching the instant never changes a ",
                    "status, so it cannot be what makes this task start.",
                )
                .to_string(),
            )
            .with_task(&t.id)
            .with_file(&lt.path),
        );
    }
}

/// `APLAN021`: a task tagged `milestone` is shaped like one (ADR-0008), and a
/// release like a release (ADR-0022).
///
/// Its own function since `A1-146`, because the checks it holds outgrew the
/// length this workspace lints for -- membership counted as live work
/// (`A1-134`), the two exemptions its audit added, and the release cases.
fn check_milestone_shape(lt: &LoadedTask, graph: &Graph<'_>, out: &mut Vec<Diagnostic>) {
    let t = &lt.task;
    // A task tagged `milestone` must be shaped like one (ADR-0008): it scopes
    // work via `blocked_by`, and carries no effort or paths, because a checkpoint
    // is not work and owns no files. The tag silently excludes it from every
    // work-view, so this warning is what makes a mis-tag, or an empty cut,
    // visible instead of a quiet reclassification.
    if t.is_milestone() {
        let mut problems = Vec::new();
        // Members, counted as *live work* rather than as entries (`A1-134`).
        //
        // The empty test was the wrong one for what this rule means. ADR-0013
        // reuses `blocked_by` to sequence cuts, so a cut emptied of all its work
        // still lists its predecessor and looked healthy; and `A1-081`
        // deliberately exempts a cut's cancelled member from `APLAN020`, so a cut
        // whose work was all dropped looked healthy too. Both are a cut with
        // nothing in it, which is the thing this rule is named for, and both were
        // silent. `roadmap` has excluded dropped members from a cut's rollup all
        // along, so this is the gate agreeing with the view rather than a new
        // opinion.
        let members: Vec<&crate::task::Task> =
            t.blocked_by.iter().filter_map(|b| graph.get(b)).collect();
        let live_work = members
            .iter()
            .filter(|m| !m.is_milestone() && m.status != Status::Dropped)
            .count();
        // A *cancelled* cut is supposed to hold cancelled work, and `APLAN027`
        // owns the mirror -- a cancelled cut still holding live members. Reporting
        // it here would fire on every `aplan drop --cut` and tell the author to
        // cancel a cut they have just cancelled (`A1-134`, found by its audit).
        let cancelled_cut = t.status == Status::Dropped;
        // A *release* holds cuts on purpose (`A1-146`, ADR-0022). Without this it
        // reads as a cut emptied of work, which is the same state and the
        // opposite intent: the tag is how an author says which they meant.
        let release = t.is_release();
        // A container is not empty, for the reason a release is not: the shape
        // is endorsed (ADR-0042), its work is one level down, and warning on it
        // would be the rule being wrong about a model this project chose. A
        // milestone that only *sequences* others is still reported, because
        // that one really does hold nothing (ADR-0013).
        //
        // Separate from `release` deliberately: the rule below -- a release's
        // members must be cuts -- is about a *release*, and a container's
        // members are the work beneath it, which are not cuts by construction.
        // Folding the two together made every container report holding work
        // directly, which is ADR-0014's finding aimed at a shape it endorses.
        let container = release || !t.contains.is_empty();
        // A `blocked_by` naming nothing that exists is `APLAN003`'s finding. Left
        // to it, because "every member of this cut is unknown" is one defect and
        // two sentences about it is noise -- and because `all()` over no members
        // is vacuously true, which made this rule say the members were cuts.
        let members_exist = !members.is_empty();
        if t.blocked_by.is_empty() {
            problems.push("has no members (`blocked_by` is empty)".to_string());
        } else if live_work == 0 && members_exist && !cancelled_cut && !container {
            // Which of the two it is, because the remedies differ: a cut that
            // only sequences needs work moved in, and a cut whose work was all
            // cancelled needs cancelling too.
            let only_cuts = members.iter().all(|m| m.is_milestone());
            problems.push(if only_cuts {
                "has no work in it: its `blocked_by` names only other cuts".to_string()
            } else {
                "has no live work in it: every task it lists is `dropped`".to_string()
            });
        }
        if t.effort_days.is_some() {
            problems.push("carries `effort_days`".to_string());
        }
        if !t.paths.is_empty() {
            problems.push("declares `paths`".to_string());
        }
        // A checkpoint's members must be cuts. One holding work directly is the
        // second grouping axis ADR-0014 refused, and `APLAN022` reports it from
        // the other end (a task in two cuts); this names the checkpoint rather
        // than every task caught by it. Its own finding, because the sentence
        // and the remedy are its own (`A1-146`).
        //
        // **Extended from releases to every container** (`A1-333`, audit). A
        // milestone that declares `contains` *and* lists work is the same
        // mistake, and it is the shape that makes the arithmetic wrong rather
        // than merely untidy: the rollup rows overlap, so the header skips the
        // whole borrowing row and the work it owned alone vanishes from the
        // total -- measured at `1 task(s), 1.0d` for a plan with two tasks and
        // three days. Every number this tool prints about a plan is correct for
        // a plan that passes `check`, and this rule is what makes that true
        // here.
        if container && let Some(work) = members.iter().find(|m| !m.is_milestone()) {
            out.push(
                Diagnostic::warning(
                    code::MILESTONE_HYGIENE,
                    format!(
                        "is a checkpoint over cuts and holds `{}` directly: {}, so a task in it \
                         is a task in two cuts",
                        work.id,
                        if release {
                            "a release closes cuts"
                        } else {
                            "a milestone declaring `contains` holds the checkpoints beneath it"
                        }
                    ),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(if release {
                    "List the cuts this release closes in its `blocked_by`, and leave the \
                     work in them. A release is a checkpoint over cuts, not a second cut."
                } else {
                    "Move the work into one of the checkpoints this one contains, or stop \
                     declaring `contains` if this is a cut rather than a checkpoint over \
                     cuts. A checkpoint borrows the work beneath it; work it also holds \
                     itself belongs to two cuts at once."
                }),
            );
        }

        if !problems.is_empty() {
            // The remedy differs by case, and a fix line that does not fit is
            // one an author learns to skip: a cut emptied of work does not need
            // its tag removed, it needs work moved into it or cancelling
            // (`A1-134`).
            let emptied = t.blocked_by.is_empty() || (live_work == 0 && members_exist);
            let instruction = if emptied && problems.len() == 1 {
                "Move the work that belongs to this cut into its `blocked_by` (`aplan mv <id>... \
                 --cut <this>`), or cancel the cut too (`aplan drop --cut <this>`) if it is not \
                 going to happen."
            } else {
                "Remove the `milestone` tag if this is work, or make it a checkpoint: give it \
                 its cut's tasks as `blocked_by`, and drop `effort_days` and `paths`."
            };
            out.push(
                Diagnostic::warning(
                    code::MILESTONE_HYGIENE,
                    format!("is tagged `milestone` but {}", problems.join(" and ")),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(instruction),
            );
        }
    }
}

/// `APLAN028`: a `done` task that did not stamp a trace it could have stamped.
///
/// Checked before the drift loop, because that loop is precisely what goes quiet:
/// `trace_fingerprints` is keyed by the trace string, so editing a `traces` entry
/// on a closed task orphans the old key and leaves the new trace unstamped, and
/// `APLAN018` then has nothing to compare (`A1-098`).
///
/// **Stamping nothing is the same sentence about every trace**, and used to be
/// exempt. The measurement behind removing that is on the rule's row in
/// `diagnostic.rs` (`A1-297`).
fn check_trace_stamp(
    root: &Path,
    t: &crate::task::Task,
    traces: &BTreeSet<&str>,
    opts: &Options,
    documents: &mut TraceDocuments,
) -> Option<Diagnostic> {
    if !opts.detect_drift || t.status != Status::Done {
        return None;
    }
    // Which traces carry no stamp. Asked first because it needs no file at all,
    // and a plan where every trace is stamped is both the common case and the one
    // `check` runs on every save: computing fingerprints for it read and hashed
    // every traced document to prove there was nothing to say, which cost this
    // repository's own plan 22% of `check` (`A1-297`, audit).
    let candidates: Vec<&str> = traces
        .iter()
        .copied()
        .filter(|tr| !t.trace_fingerprints.contains_key(*tr))
        // Leaving the repository is asked first, the same precedence the loop
        // below states (`A1-143`): a trace `APLAN034` is refusing is one the
        // author is being told to declare or drop, and telling them to stamp it
        // in the same breath is two errors for one cause whose fixes disagree.
        .filter(|tr| check_external_trace(root, opts, tr).is_none())
        .collect();
    if candidates.is_empty() {
        return None;
    }
    // Of those, only what `close --restamp` can actually stamp -- asked of the
    // function that does the stamping rather than approximated here. A diagnostic
    // whose own fix provably would not clear it is worse than silence, because an
    // author learns to ignore the rule, and there are three ways to be
    // unstampable: a trace with no anchor, one whose document cannot be read, and
    // one whose anchor matches nothing. The second and third are `APLAN008`'s to
    // report, and the first is `A1-191`'s question. Spelling the predicate out
    // here instead got the first and missed the others.
    let unstamped: Vec<&str> = candidates
        .into_iter()
        .filter(|tr| documents.fingerprint(root, tr).is_some())
        .collect();
    if unstamped.is_empty() {
        return None;
    }
    let orphans: Vec<&str> = t
        .trace_fingerprints
        .keys()
        .map(String::as_str)
        .filter(|k| !traces.contains(k))
        .collect();
    // Name the retarget when the evidence for it is there: an orphaned key is the
    // only surviving record of what the task was closed against, and "you changed
    // this trace" is a better sentence than "this trace is unstamped".
    let how = match orphans.as_slice() {
        [was] => format!(", and stamps `{was}`, which it no longer traces"),
        [] => String::new(),
        many => format!(", and stamps {} keys it no longer traces", many.len()),
    };
    // Every unstamped trace, not the first. Naming one would read as the only one,
    // and an author repairing them one at a time is the slow version of what
    // `--restamp` does in a single pass.
    let named = unstamped
        .iter()
        .map(|tr| format!("`{tr}`"))
        .collect::<Vec<_>>()
        .join(", ");
    // Neutral about what the anchor names, because one task's list can hold a
    // markdown section and a YAML node at once and a single sentence has to fit
    // both (`A1-191`, audit).
    let (them, what) = if unstamped.len() == 1 {
        ("it", "what it traces")
    } else {
        ("them", "what they trace")
    };
    Some(
        Diagnostic::error(
            code::UNSTAMPED_TRACE,
            format!(
                "is `done` and does not stamp {named}{how}: drift is not checked for \
                 {them}, so {what} can change and nothing will say so"
            ),
        )
        .with_agent_instruction(format!(
            "Re-review {named}, then `aplan close {} --restamp`, which stamps every \
             trace and drops keys no trace names.",
            t.id
        )),
    )
}

/// Compare a stored trace fingerprint against the live traced section. `None`
/// when they match, the document or anchor cannot be read (that is `APLAN008`'s
/// job, not a drift), or the trace names no anchor (only sections are stamped).
fn check_drift(
    root: &Path,
    trace: &str,
    stamp: &str,
    closed_at: Option<&str>,
    documents: &mut TraceDocuments,
) -> Option<Diagnostic> {
    let (rel, _) = trace.split_once('#')?;
    // Markdown accepts the pre-`A1-122` form as well as the current one, so a
    // plan whose stamps predate the trailing-blank fix is not re-flagged
    // wholesale. Structured documents compare their canonical node directly.
    if documents.holds(root, trace, stamp)? {
        return None;
    }
    // Where to look, when the task cites a commit to look *from*. The reader is
    // otherwise told a section moved and left to work out which of its lines did,
    // often for a closure somebody else made (pre-public adoption evidence, item 14).
    //
    // A command, not a diff: `trace_fingerprints` stores a hash and not the
    // content, so there is nothing here to compare against, and storing the text
    // to make one possible would put a copy of every traced section in every task
    // file. Printing the command costs nothing and keeps the rule hermetic.
    // The path in the command is joined onto the traces root, not the trace's own
    // spelling. `traces_root` is configurable, so with `traces_root: docs` a trace
    // of `d.md#intro` is `docs/d.md` from the repository, and `git diff <sha> --
    // d.md` matches nothing and exits *zero*: a command that looks like it worked
    // and shows an empty diff is worse than one that fails.
    let shown = command_path(root, rel);
    let where_to_look = closed_at.map_or_else(
        || format!("Re-review the task against `{shown}`"),
        |sha| format!("See what moved with `git diff {sha} -- {shown}`, then re-review the task"),
    );
    // "section" was the only thing an anchor could name until ADR-0035. A YAML or
    // JSON address names a node, and calling it a section sends the reader
    // looking for a heading that is not there (`A1-191`, audit).
    let what = if crate::structured::format_for(rel).is_some() {
        "node"
    } else {
        "section"
    };
    Some(
        Diagnostic::warning(
            code::TRACE_DRIFT,
            format!("traced {what} `{trace}` has changed since this task was closed"),
        )
        .with_agent_instruction(format!(
            "{where_to_look}; if it still holds, re-stamp with \
             `aplan close --restamp`, otherwise `aplan supersede` it with the task that now \
             needs doing."
        )),
    )
}

/// The traced document as a reader would name it from the repository.
///
/// Joined onto the traces root rather than taken from the trace's own spelling.
/// `traces_root` is configurable, so under `traces_root: docs` a trace of
/// `d.md#intro` is `docs/d.md` from the repository, and `git diff <sha> -- d.md`
/// would match nothing and exit *zero*: a command that looks like it worked and
/// shows an empty diff is worse than one that fails.
///
/// The default root is `.`, so the common case is the plain relative path and the
/// leading `./` is dropped. A root the user made absolute stays absolute, which
/// git accepts and which they chose.
fn command_path(root: &Path, rel: &str) -> String {
    let joined = root.join(rel);
    joined
        .strip_prefix(".")
        .unwrap_or(&joined)
        .display()
        .to_string()
}

/// The one commit a task cites, or `None` when it cites none or several.
///
/// Deliberately silent at "several" rather than picking. `aplan close` refuses a
/// task that already carries evidence, so every citation arrives in one block and
/// nothing distinguishes which of them the stamp was taken at. Naming one anyway
/// would put a specific SHA in a sentence that cannot justify it, which is the
/// defect `APLAN024` shipped and `AGENTS.md` now forbids: a diagnostic may name
/// only what the predicate returned.
fn sole_commit(t: &crate::task::Task) -> Option<&str> {
    let mut commits = t
        .evidence
        .iter()
        .filter(|e| e.kind == EvidenceKind::Commit)
        .map(|e| e.reference.trim())
        .filter(|r| !is_vacuous_ref(r));
    let first = commits.next()?;
    commits.next().is_none().then_some(first)
}

/// `APLAN027`: a cancelled cut's members are cancelled too.
///
/// A `dropped` milestone whose members are live says the cut is cancelled and
/// the work in it is going ahead. A bug produced exactly that and nothing
/// reported it (`A1-084`); a hand-edit reaches it too, as does dropping a
/// milestone without `--cut`.
fn check_cancelled_cuts(plan: &Plan, graph: &Graph<'_>, out: &mut Vec<Diagnostic>) {
    for lt in &plan.tasks {
        let t = &lt.task;
        if !t.is_milestone() || t.status != Status::Dropped {
            continue;
        }
        // Members only. A cut this one follows is a sequencing edge, and a live
        // predecessor of a cancelled cut is ordinary.
        let live: Vec<&str> = t
            .blocked_by
            .iter()
            .filter_map(|b| graph.get(b))
            .filter(|m| !m.is_milestone() && !m.status.is_terminal())
            .map(|m| m.id.as_str())
            .collect();
        if live.is_empty() {
            continue;
        }
        out.push(
            Diagnostic::warning(
                code::CANCELLED_CUT_LIVE_WORK,
                format!(
                    "is `dropped` but {} member(s) are still live ({}): the plan says this cut \
                     is cancelled and that its work is going ahead",
                    live.len(),
                    live.join(", ")
                ),
            )
            .with_task(&t.id)
            .with_file(&lt.path)
            .with_agent_instruction(
                "Cancel the work too (`aplan drop <id>...`), or move it to a cut that is still \
                 live, or revive the cut.",
            ),
        );
    }
}

/// `APLAN026`: a tag that looks like a member of a declared family is one.
///
/// A tag is judged to belong to a family when it carries the family's `name-`
/// prefix, or when it is one edit from a declared member. The prefix catches
/// `epic-x`, an epic nobody declared; the edit distance catches `epci-k`, which
/// carries no recognisable prefix at all. Both were named by the pilot that
/// produced this (`A1-076`).
/// The words this tool acts on, wherever they appear in a task's `tags`.
///
/// Declared once, here, because two copies of this list is how one of them goes
/// stale: `APLAN051` reports a near miss of any of them and `APLAN052` is about
/// the fifth, and both read this rather than spelling the words again
/// (`A1-212`).
///
/// Unprefixed, and that was decided rather than inherited -- see `A1-212`'s body
/// for the migration this project declined and the count it was measured
/// against.
pub const RESERVED_TAGS: &[&str] = &[
    "cross-cutting",
    "deprecation",
    "milestone",
    "release",
    "removal",
];

/// `APLAN051` and `APLAN052`: the five words, and the two things that go wrong
/// with them (`A1-212`).
fn check_reserved_tags(plan: &Plan, opts: &Options, out: &mut Vec<Diagnostic>) {
    // A word the repository declared in its own family is reported against the
    // configuration, once, rather than once per task that carries it: the
    // mistake is one line of `.aplan.yml` and the remedy is to edit that line.
    for (family, declared) in &opts.tag_families {
        for member in &declared.members {
            if RESERVED_TAGS.contains(&member.as_str()) {
                out.push(
                    Diagnostic::warning(
                        code::RESERVED_TAG_NEAR_MISS,
                        format!(
                            "`tag_families.{family}` lists `{member}`, which this tool already acts on: a task carrying it is judged by the tool's meaning and not by this family"
                        ),
                    )
                    .with_agent_instruction(format!(
                        "Remove `{member}` from `tag_families.{family}` in `.aplan.yml`, or rename the family's member if it was meant to be the repository's own word."
                    )),
                );
            }
        }
    }

    let cut_members = members_of_any_cut(plan);
    for lt in &plan.tasks {
        let tags: BTreeSet<&str> = lt.task.tags.iter().map(String::as_str).collect();
        for tag in tags {
            if RESERVED_TAGS.contains(&tag) {
                // The word itself, used as the tool reads it. The only thing to
                // say about it is `APLAN052`'s case.
                if tag == "cross-cutting" && cut_members.contains(lt.task.id.as_str()) {
                    out.push(
                        Diagnostic::warning(
                            code::CROSS_CUTTING_IN_A_CUT,
                            "tagged `cross-cutting` and a member of a cut: the tag is how `APLAN022` is told a task belongs to no cut, which this task's own membership contradicts"
                                .to_string(),
                        )
                        .with_task(&lt.task.id)
                        .with_file(&lt.path)
                        .with_agent_instruction(
                            "Remove `cross-cutting` from this task's `tags`: it is in a cut, so the requirement it waives is one this task already meets. If the word was meant to say the work spans components, declare a family for that in `tag_families` and use a word of your own."
                                .to_string(),
                        ),
                    );
                }
                continue;
            }
            // **A tag a declared family claims is that family's, and one rule
            // speaks about it** (`A1-212`, audit). Not just a declared member:
            // anything `APLAN026` will report -- a family's prefix, or a near
            // miss of one of its members -- because otherwise a repository with
            // a `releas` family gets two findings on `releasa`, one saying "did
            // you mean `releas-a`?" and the other "did you mean `release`?".
            // Two remedies for one mistake is the "two rules disagreeing" this
            // task was filed to prevent.
            //
            // The family wins because the repository declared it, which is
            // evidence of intent that a built-in list cannot have. A tag no
            // family claims is judged here and nowhere else.
            if claimed_by(&opts.tag_families, tag).is_some()
                || opts
                    .tag_families
                    .values()
                    .any(|f| f.members.iter().any(|m| m == tag))
            {
                continue;
            }
            if let Some(reserved) = RESERVED_TAGS
                .iter()
                .find(|word| is_one_edit_apart(word, tag))
            {
                out.push(
                    Diagnostic::warning(
                        code::RESERVED_TAG_NEAR_MISS,
                        format!(
                            "tag `{tag}` is one edit from `{reserved}`, which changes what this tool does: did you mean `{reserved}`?"
                        ),
                    )
                    .with_task(&lt.task.id)
                    .with_file(&lt.path)
                    .with_agent_instruction(format!(
                        "Correct the tag to `{reserved}` if that is what was meant. If `{tag}` is the repository's own word, declare it in `tag_families` so it is a vocabulary rather than a near miss."
                    )),
                );
            }
        }
    }
}

/// Every task some milestone lists as a member.
fn members_of_any_cut(plan: &Plan) -> BTreeSet<&str> {
    let milestones: Vec<&crate::task::Task> = plan
        .tasks
        .iter()
        .map(|lt| &lt.task)
        .filter(|t| t.is_milestone())
        .collect();
    let known: BTreeSet<&str> = plan.tasks.iter().map(|lt| lt.task.id.as_str()).collect();
    let mut out = BTreeSet::new();
    for m in milestones {
        for b in &m.blocked_by {
            if let Some(id) = known.get(b.as_str()) {
                out.insert(*id);
            }
        }
    }
    out
}

/// `APLAN056`: every present owner is in the declared vocabulary.
///
/// Unlike a tag family, `owner` has one namespace: once a repository names its
/// canonical spellings, an unfamiliar value cannot be an unrelated kind of
/// owner. The vocabulary is opt-in, and an unassigned task remains valid --
/// assignment is workflow state, not a cardinality requirement.
fn check_owner_vocabulary(plan: &Plan, opts: &Options, out: &mut Vec<Diagnostic>) {
    if opts.owners.is_empty() {
        return;
    }
    for lt in &plan.tasks {
        let Some(owner) = lt.task.owner.as_deref() else {
            continue;
        };
        if opts.owners.contains(owner) {
            continue;
        }
        let near: Vec<&str> = opts
            .owners
            .iter()
            .map(String::as_str)
            .filter(|known| is_one_edit_apart(known, owner))
            .collect();
        let (message, instruction) = match near.as_slice() {
            [] => (
                format!("owner `{owner}` is not declared in `owners`"),
                format!(
                    "Add `{owner}` to `owners` in `.aplan.yml` if it is a canonical identity, or correct this task's `owner`."
                ),
            ),
            [known] => (
                format!("owner `{owner}` is not declared in `owners`: did you mean `{known}`?"),
                format!(
                    "Change this task's `owner` to `{known}` if that is who it means, or add `{owner}` to `owners` in `.aplan.yml` if it is a distinct identity."
                ),
            ),
            many => {
                let listed = many
                    .iter()
                    .map(|known| format!("`{known}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    format!(
                        "owner `{owner}` is not declared in `owners` and is one edit from {listed}"
                    ),
                    format!(
                        "Choose the intended declared owner ({listed}), or add `{owner}` to `owners` in `.aplan.yml` if it is a distinct identity."
                    ),
                )
            }
        };
        out.push(
            Diagnostic::warning(code::OWNER_VOCABULARY, message)
                .with_task(&lt.task.id)
                .with_file(&lt.path)
                .with_agent_instruction(instruction),
        );
    }
}

fn check_tag_families(plan: &Plan, opts: &Options, out: &mut Vec<Diagnostic>) {
    if opts.tag_families.is_empty() {
        return;
    }
    // Which families the plan touches at all, gathered while judging the tags so
    // there is one pass and one definition of "touches" (`APLAN029`, below).
    let mut used: BTreeSet<&str> = BTreeSet::new();
    for lt in &plan.tasks {
        // Deduplicated, as `blocked_by` is: a tag listed twice is one mistake,
        // and the identical diagnostic twice is noise, especially as two
        // identical annotations under `--format github`.
        let tags: BTreeSet<&str> = lt.task.tags.iter().map(String::as_str).collect();
        for tag in tags {
            // A declared member of any family is correct by definition, and a
            // family's own name is not a member of itself.
            if let Some((family, _)) = opts
                .tag_families
                .iter()
                .find(|(_, family)| family.members.iter().any(|m| m == tag))
            {
                used.insert(family.as_str());
                continue;
            }
            let Some((family, near)) = claimed_by(&opts.tag_families, tag) else {
                continue; // Outside every declared vocabulary, and so not judged.
            };
            // A tag in a family's shape counts as using it even though it is
            // wrong, so one mistake produces one finding: `APLAN026` already
            // names this tag, and adding "and nothing uses `epic`" underneath
            // would be the same situation reported twice.
            used.insert(family);
            let message = match &near {
                Some(member) => format!(
                    "tag `{tag}` is one edit from `{member}` in the `{family}` family: did you \
                     mean `{member}`?"
                ),
                None => format!(
                    "tag `{tag}` carries the `{family}-` prefix, but `{family}` does not list it"
                ),
            };
            out.push(
                Diagnostic::warning(code::UNKNOWN_TAG, message)
                .with_task(&lt.task.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!(
                    "Correct the tag, or add it to `tag_families.{family}` in `.aplan.yml` if it \
                     is a real member."
                )),
            );
        }
    }
    report_unused_families(opts, &used, out);
    report_cardinality(plan, opts, &used, out);
}

/// `APLAN030`: a task carries the number of members a declared family requires.
///
/// Counted over *declared members only*. A tag that merely looks like one is
/// `APLAN026`'s finding and is not evidence that the requirement was met: a task
/// tagged `priority-p9` under `exactly-one` has no priority this repository
/// declared, and reporting that it does would be the rule taking a typo as proof.
///
/// Silent on a family nothing in the plan carries, which is `APLAN029`'s finding
/// and is why that rule is this one's dependency. A requirement over a
/// vocabulary no tag matches is unbindable, not unmet: `exactly-one` would fail
/// every task in the plan and `at-most-one` would pass every one, and both would
/// be confident verdicts about a declaration that was never bound. One
/// mis-declaration is one finding, not one per task plus one (`A1-133`).
fn report_cardinality(
    plan: &Plan,
    opts: &Options,
    used: &BTreeSet<&str>,
    out: &mut Vec<Diagnostic>,
) {
    for lt in &plan.tasks {
        let task = &lt.task;
        for (name, family) in &opts.tag_families {
            if !used.contains(name.as_str()) || !family.judges(task) {
                continue;
            }
            if family.satisfied_by(task) {
                continue;
            }
            // Deduplicated by the declaration: a member listed twice on one task
            // is one tag, and `exactly-one` must not be failed by writing it out
            // twice.
            let carried = family.carried(task);
            let found = if carried.is_empty() {
                "none".to_string()
            } else {
                carried
                    .iter()
                    .map(|t| format!("`{t}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let scope = if family.status == StatusScope::Any {
                String::new()
            } else {
                format!(" on {} tasks", family.status.as_str())
            };
            out.push(
                Diagnostic::error(
                    code::TAG_CARDINALITY,
                    format!(
                        "`{name}` is declared {}{scope} and this task carries {found}",
                        family.require.as_str()
                    ),
                )
                .with_task(&task.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!(
                    "Carry {} of `tag_families.{name}` on this task, or change what \
                     `.aplan.yml` requires of it.",
                    match family.require {
                        Cardinality::AtMostOne => "at most one",
                        Cardinality::AtLeastOne => "at least one",
                        _ => "exactly one",
                    }
                )),
            );
        }
    }
}

/// `APLAN029`: a declared tag family is carried by at least one task.
///
/// Reported as an observation, not a diagnosis. A family nothing carries is as
/// likely to be a vocabulary declared before its first use as the fourth pilot's
/// mis-declaration, and the two have different remedies, so the message names
/// what it saw and the fix line names both (`A1-070`'s lesson, `A1-085`).
fn report_unused_families(opts: &Options, used: &BTreeSet<&str>, out: &mut Vec<Diagnostic>) {
    for (family, declared) in &opts.tag_families {
        if used.contains(family.as_str()) {
            continue;
        }
        let members = &declared.members;
        let listed = members
            .iter()
            .map(|m| format!("`{m}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut d = Diagnostic::warning(
            code::UNUSED_TAG_FAMILY,
            format!(
                "no task carries the `{family}` family: it declares {} member(s), {listed}, \
                     and a task joins by carrying one of them as a whole tag",
                members.len()
            ),
        )
        .with_agent_instruction(format!(
            "Tag a task with one of them, or correct `tag_families.{family}` in `.aplan.yml` \
                 if the members are not the tags your tasks actually carry."
        ));
        // The declaration's own file, so the finding has a location like every
        // other one: an annotation lands on `.aplan.yml` in the diff, and a
        // second unused family is not a locationless sentence.
        if let Some(path) = &opts.config_file {
            d = d.with_file(path);
        }
        out.push(d);
    }
}

/// The family a tag appears to belong to, and the member it is nearest to.
fn claimed_by<'a>(
    families: &'a BTreeMap<String, TagFamily>,
    tag: &str,
) -> Option<(&'a str, Option<&'a String>)> {
    // The prefix first. A tag carrying the family's own prefix is an undeclared
    // member, and saying so beats suggesting the nearest declared one:
    // `epic-x` is one substitution from `epic-a`, and "did you mean `epic-a`?"
    // is a worse answer than "`epic` does not list it".
    //
    // Longest match wins, because one family's name can be a prefix of
    // another's: `epic-sub-y` belongs to `epic-sub`, and naming `epic` would
    // send the author to add it to the wrong list.
    if let Some(family) = families
        .keys()
        .filter(|family| tag.starts_with(&format!("{family}-")))
        .max_by_key(|family| family.len())
    {
        return Some((family, None));
    }
    // Otherwise the nearest member, which is what catches a tag whose prefix is
    // itself the typo (`epci-b`, `kimd-m`).
    for (family, declared) in families {
        if let Some(member) = declared.members.iter().find(|m| is_one_edit_apart(m, tag)) {
            return Some((family, Some(member)));
        }
    }
    None
}

/// Whether two strings are within one insertion, deletion, substitution, or
/// transposition of adjacent characters.
///
/// Damerau-Levenshtein bounded at one, written out rather than pulled in: the
/// bound makes it a handful of comparisons, and a transposition is exactly the
/// case a prefix rule cannot see (`epci-k` for `epic-k`).
pub fn is_one_edit_apart(member: &str, tag: &str) -> bool {
    if member == tag {
        return false; // Identical is a match, not a near miss.
    }
    let left: Vec<char> = member.chars().collect();
    let right: Vec<char> = tag.chars().collect();
    match left.len().abs_diff(right.len()) {
        0 => {
            let diffs: Vec<usize> = (0..left.len()).filter(|&i| left[i] != right[i]).collect();
            match diffs.as_slice() {
                [_] => true,
                // Adjacent transposition: `epic` -> `epci`.
                [first, second] if *second == first + 1 => {
                    left[*first] == right[*second] && left[*second] == right[*first]
                }
                _ => false,
            }
        }
        1 => {
            // One insertion or deletion: the longer string with one character
            // skipped must equal the shorter.
            let (long, short) = if left.len() > right.len() {
                (&left, &right)
            } else {
                (&right, &left)
            };
            let mut skipped = false;
            let mut matched = 0usize;
            for &c in long {
                if matched < short.len() && short[matched] == c {
                    matched += 1;
                } else if skipped {
                    return false;
                } else {
                    skipped = true;
                }
            }
            matched == short.len()
        }
        _ => false,
    }
}

/// The invariants that read the `blocked_by` edges: `APLAN003`, `APLAN006`,
/// `APLAN007`, `APLAN020`.
fn check_edges(
    lt: &LoadedTask,
    graph: &Graph<'_>,
    unloadable: &BTreeMap<String, PathBuf>,
    out: &mut Vec<Diagnostic>,
) {
    check_declared_edges(lt, graph, unloadable, out);
    check_inherited_cut_edges(lt, graph, out);
    check_ratifiers(lt, graph, unloadable, out);
}

/// `APLAN004` for a loop that runs through `ratified_by` (`A1-306`).
fn report_ratification_cycle(
    plan: &Plan,
    file_of: &BTreeMap<&str, &Path>,
    out: &mut Vec<Diagnostic>,
) {
    let Some(cycle) = ratification_cycle(plan) else {
        return;
    };
    let path = cycle.join(" → ");
    let anchor = cycle.first().cloned().unwrap_or_default();
    let mut d = Diagnostic::error(
        code::CYCLE,
        format!("dependency cycle through `ratified_by`: {path}"),
    )
    .with_task(&anchor)
    .with_agent_instruction(
        "Break the cycle by removing one `ratified_by` edge: each of these waits on \
         the next to be finished before it may be finished, so none of them ever \
         can be.",
    );
    if let Some(p) = file_of.get(anchor.as_str()) {
        d = d.with_file(*p);
    }
    out.push(d);
}

/// `APLAN047`: every `contains` names a milestone that exists, and the
/// containment closes no loop (`A1-333`, ADR-0042).
///
/// Three refusals in one rule because they are one mistake at three depths: a
/// name that resolves to nothing, a name that resolves to a task rather than a
/// checkpoint, and a loop. Each says what is wrong with *this* edge, which is
/// what an author can act on.
fn check_contains(plan: &Plan, file_of: &BTreeMap<&str, &Path>, out: &mut Vec<Diagnostic>) {
    let kind: BTreeMap<&str, bool> = plan
        .tasks
        .iter()
        .map(|lt| (lt.task.id.as_str(), lt.task.is_milestone()))
        .collect();
    for lt in &plan.tasks {
        let t = &lt.task;
        if t.contains.is_empty() {
            continue;
        }
        // Only a checkpoint may contain. A work task doing so is the second
        // grouping axis ADR-0014 refused, arriving by a new door.
        if !t.is_milestone() {
            out.push(
                Diagnostic::error(
                    code::CONTAINS_UNUSABLE,
                    "declares `contains` but is not tagged `milestone`: only a checkpoint \
                     holds other checkpoints"
                        .to_string(),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(
                    "Remove `contains`, or tag this task `milestone` if it really is a \
                     checkpoint over other cuts.",
                ),
            );
            continue;
        }
        for child in &t.contains {
            let problem = match kind.get(child.as_str()) {
                None => format!("`contains: {child}` names no task in this plan"),
                Some(false) => format!(
                    "`contains: {child}` names a task rather than a milestone: containment \
                     is between checkpoints, and a cut lists its work in `blocked_by`"
                ),
                // A self-edge, which `contains_cycle` deliberately skips. That
                // skip is copied from `ratification_cycle`, where it is right
                // because `APLAN006` reports a `blocked_by` self-loop -- and a
                // `contains` self-edge is not a `blocked_by` edge, so nothing
                // reported it at all. Found by writing one (`A1-333`, audit).
                Some(true) if child == &t.id => {
                    "`contains` names itself, so this checkpoint is inside itself".to_string()
                }
                // **Containment must also wait.** A part unfinished means the
                // whole is unfinished, and the gate that says so is
                // `APLAN007`'s, which keys on `blocked_by`. Without this a
                // checkpoint sits at `done` over an open child and the rollup
                // prints `[done] 0/1 done` beside itself, which is the shape
                // `A1-306` found for `ratified_by` and fixed with a rule rather
                // than left to an author to remember.
                Some(true) if !t.blocked_by.iter().any(|b| b == child) => format!(
                    "`contains: {child}` is not also in `blocked_by`, so this checkpoint does \
                     not wait for what it holds and may be closed while it is unfinished"
                ),
                Some(true) => continue,
            };
            out.push(
                Diagnostic::error(code::CONTAINS_UNUSABLE, problem)
                    .with_task(&t.id)
                    .with_file(&lt.path)
                    .with_agent_instruction(
                        "Name a milestone that exists and list it in `blocked_by` too, or \
                         move the id to `blocked_by` alone if it is work this cut holds \
                         rather than a checkpoint beneath it.",
                    ),
            );
        }
    }
    if let Some(cycle) = contains_cycle(plan) {
        let path = cycle.join(" \u{2192} ");
        let anchor = cycle.first().cloned().unwrap_or_default();
        let mut d = Diagnostic::error(
            code::CONTAINS_UNUSABLE,
            format!("containment cycle through `contains`: {path}"),
        )
        .with_task(&anchor)
        .with_agent_instruction(
            "Decide which of these contains the other and remove the reverse edge. A \
             rollup over a loop has no answer: whichever checkpoint is walked first \
             swallows the other, and the number changes with iteration order.",
        );
        if let Some(p) = file_of.get(anchor.as_str()) {
            d = d.with_file(*p);
        }
        out.push(d);
    }
}

/// A loop in `contains`, as the ids on it (`A1-333`).
///
/// The same iterative walk as `ratification_cycle`, over the containment
/// relation alone: a `blocked_by` loop is `APLAN006`'s and saying it twice
/// helps nobody.
fn contains_cycle(plan: &Plan) -> Option<Vec<String>> {
    let mut edges: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for lt in &plan.tasks {
        let id = lt.task.id.as_str();
        edges.entry(id).or_default().extend(
            lt.task
                .contains
                .iter()
                .map(String::as_str)
                .filter(|d| *d != id),
        );
    }
    let mut mark: BTreeMap<&str, Mark> = BTreeMap::new();
    for start in edges.keys().copied() {
        if mark.contains_key(start) {
            continue;
        }
        let mut path: Vec<&str> = vec![start];
        let mut at: Vec<(usize, &str)> = vec![(0, start)];
        mark.insert(start, Mark::Open);
        while let Some((i, node)) = at.pop() {
            let nexts = edges.get(node).map_or(&[][..], Vec::as_slice);
            if i >= nexts.len() {
                mark.insert(node, Mark::Done);
                path.pop();
                continue;
            }
            at.push((i + 1, node));
            let next = nexts[i];
            match mark.get(next) {
                Some(Mark::Open) => {
                    let from = path.iter().position(|p| *p == next).unwrap_or(0);
                    let mut cycle: Vec<String> =
                        path[from..].iter().map(|s| (*s).to_string()).collect();
                    cycle.push(next.to_string());
                    return Some(cycle);
                }
                Some(Mark::Done) => {}
                None => {
                    mark.insert(next, Mark::Open);
                    path.push(next);
                    at.push((0, next));
                }
            }
        }
    }
    None
}

/// Where the walk below has got to with a node.
#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Open,
    Done,
}

/// A loop in the combined "must be finished first" relation (`A1-306`).
///
/// `blocked_by` says a task cannot *start* until another is done; `ratified_by`
/// says it cannot be *finished* until another is. Both mean "that one first", so
/// a loop through any mixture of them is a set of tasks none of which can ever
/// be closed -- the same deadlock `APLAN004` reports, reached by a different
/// edge. Returns the cycle as a path, in the shape the existing report prints.
fn ratification_cycle(plan: &Plan) -> Option<Vec<String>> {
    let mut edges: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for lt in &plan.tasks {
        let id = lt.task.id.as_str();
        let out = edges.entry(id).or_default();
        // A self-edge is `APLAN006`'s, and reporting it here too would say the
        // same mistake twice, which the graph's own walk already refuses.
        out.extend(
            lt.task
                .blocked_by
                .iter()
                .chain(lt.task.ratified_by.iter())
                .map(String::as_str)
                .filter(|d| *d != id),
        );
    }
    // Iterative depth-first search with an explicit stack: a plan is allowed to
    // be deep, and a recursive walk would trade one deadlock for another.
    let mut mark: BTreeMap<&str, Mark> = BTreeMap::new();
    for start in edges.keys().copied() {
        if mark.contains_key(start) {
            continue;
        }
        let mut path: Vec<&str> = vec![start];
        let mut at: Vec<(usize, &str)> = vec![(0, start)];
        mark.insert(start, Mark::Open);
        while let Some((i, node)) = at.pop() {
            let nexts = edges.get(node).map_or(&[][..], Vec::as_slice);
            if i >= nexts.len() {
                mark.insert(node, Mark::Done);
                path.pop();
                continue;
            }
            at.push((i + 1, node));
            let next = nexts[i];
            match mark.get(next) {
                Some(Mark::Open) => {
                    // The loop, from where it closes back round to here.
                    let from = path.iter().position(|p| *p == next).unwrap_or(0);
                    let mut out: Vec<String> =
                        path[from..].iter().map(|s| (*s).to_string()).collect();
                    out.push(next.to_string());
                    return Some(out);
                }
                Some(Mark::Done) => {}
                None => {
                    // A name nothing defines is `APLAN003`'s, not this one's.
                    if edges.contains_key(next) {
                        mark.insert(next, Mark::Open);
                        path.push(next);
                        at.push((0, next));
                    }
                }
            }
        }
    }
    None
}

/// `APLAN046`: a `done` task whose ratifier has not finished (`A1-306`,
/// ADR-0036).
///
/// The mirror of `APLAN007` for the other half of what `blocked_by` was being
/// asked to mean. A producer edge stops a task *starting*; a ratification stops
/// it *counting as finished*, so `ready` never reads `ratified_by` and this
/// does. A name nothing in the plan defines is reported the way a `blocked_by`
/// ghost is, because a ratifier who does not exist cannot bless anything.
fn check_ratifiers(
    lt: &LoadedTask,
    graph: &Graph<'_>,
    unloadable: &BTreeMap<String, PathBuf>,
    out: &mut Vec<Diagnostic>,
) {
    let t = &lt.task;
    // Deduplicated, as `check_declared_edges` does: one relationship named twice
    // is one relationship, and two identical annotations are noise.
    for r in t
        .ratified_by
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
    {
        if r == t.id {
            out.push(
                Diagnostic::error(code::SELF_DEP, "task lists itself in `ratified_by`")
                    .with_task(&t.id)
                    .with_file(&lt.path)
                    .with_agent_instruction(format!(
                        "Remove `{}` from its own `ratified_by`: nothing blesses itself.",
                        t.id
                    )),
            );
            continue;
        }
        let Some(dep) = graph.get(r) else {
            // An unloadable file is a different finding, already reported
            // against the file that would not parse (`A1-066`).
            if !unloadable.contains_key(r) {
                out.push(
                    Diagnostic::error(
                        code::UNKNOWN_DEP,
                        format!("`ratified_by` references unknown task `{r}`"),
                    )
                    .with_task(&t.id)
                    .with_file(&lt.path)
                    .with_agent_instruction(format!(
                        "Either create task `{r}` or remove it from `{}`'s `ratified_by`.",
                        t.id
                    )),
                );
            }
            continue;
        };
        // A ratifier that was cancelled can never bless anything, so the task can
        // never legitimately be closed. `APLAN020`'s defect with the other
        // endpoint, and reported under the same code because it is the same
        // mistake: an edge to something that can no longer satisfy it
        // (`A1-306`, audit).
        if !t.status.is_terminal() && dep.status == Status::Dropped {
            out.push(
                Diagnostic::error(
                    code::DROPPED_BLOCKER,
                    format!(
                        "is `{}` but its ratifier `{r}` is `dropped`: it can never legitimately \
                         be closed",
                        t.status
                    ),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!(
                    "Either revive `{r}`, or remove it from `{}`'s `ratified_by` and name who \
                     actually signs this off. The work is not blocked either way.",
                    t.id
                )),
            );
        }
        if t.status == Status::Done && !dep.status.satisfies_dependency() {
            out.push(
                Diagnostic::error(
                    code::UNRATIFIED,
                    format!("is `done` but its ratifier `{r}` is `{}`", dep.status),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!(
                    "Close `{r}` first, or drop the ratification if this task does not \
                     actually need its blessing. The work itself was never blocked by it: \
                     `ratified_by` says who signs off, not what this waits on."
                )),
            );
        }
    }
}

/// The status rules on the cut-sequencing edges a task inherits (ADR-0013).
///
/// `close` refuses a task whose cut predecessor is unfinished, because it reads
/// the same blocker set the graph does. `check` read only the declared edges, so
/// a status hand-edited to `done` passed the gate that the write path would have
/// refused: the tool enforcing a rule at write time and not at check time is the
/// gap this project exists to close (`A1-073`).
///
/// The message says the edge comes from the cut, because an author who looks for
/// `M-s0` in their own `blocked_by` will not find it there.
fn check_inherited_cut_edges(lt: &LoadedTask, graph: &Graph<'_>, out: &mut Vec<Diagnostic>) {
    let t = &lt.task;
    let declared: BTreeSet<&str> = t.blocked_by.iter().map(String::as_str).collect();

    for b in graph.blockers_of(&t.id) {
        if declared.contains(b) {
            continue; // already judged above, and one edge must not report twice
        }
        let Some(dep) = graph.get(b) else { continue };

        if t.status == Status::Done && !dep.status.satisfies_dependency() {
            out.push(
                Diagnostic::error(
                    code::DONE_BEFORE_BLOCKER,
                    format!(
                        "is `done` but its cut follows `{b}`, which is `{}`",
                        dep.status
                    ),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!(
                    "Either close `{b}` first, or drop the cut-sequencing edge if this task \
                     does not really wait for it."
                )),
            );
        }

        if !t.status.is_terminal() && dep.status == Status::Dropped {
            out.push(
                Diagnostic::error(
                    code::DROPPED_BLOCKER,
                    format!(
                        "is `{}` but its cut follows `{b}`, which is `dropped`: it can never \
                         legitimately start",
                        t.status
                    ),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!(
                    "Either drop `{}` too, revive `{b}`, or remove the cut-sequencing edge.",
                    t.id
                )),
            );
        }
    }
}

/// The invariants on the edges a task writes down itself.
fn check_declared_edges(
    lt: &LoadedTask,
    graph: &Graph<'_>,
    unloadable: &BTreeMap<String, PathBuf>,
    out: &mut Vec<Diagnostic>,
) {
    let t = &lt.task;

    // Self-dependency. A cycle of one, reported specifically because the
    // generic cycle message reads badly for it.
    if t.blocked_by.iter().any(|b| b == &t.id) {
        out.push(
            Diagnostic::error(code::SELF_DEP, "task lists itself in `blocked_by`")
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!("Remove `{}` from its own `blocked_by`.", t.id)),
        );
    }

    // Edges are deduplicated: `blocked_by: [GHOST, GHOST]` is one broken edge,
    // not two, and emitting the identical diagnostic twice is just noise (two
    // identical annotations, in `--format github`). Sorted, so the order does
    // not depend on how the author happened to type the list.
    let blockers: BTreeSet<&str> = t.blocked_by.iter().map(String::as_str).collect();

    for b in blockers {
        if b == t.id {
            continue; // already `SELF_DEP`
        }
        let Some(dep) = graph.get(b) else {
            // A task whose file is present but did not load is not unknown, and
            // this edge is not the fault. Reporting it anyway buries the real
            // error under a cascade proportional to how well connected the plan
            // is: one unquoted colon in three files produced sixteen of these in
            // the fourth pilot's shape, and fifty-two in its real plan
            // (`A1-072`).
            //
            // Suppressed rather than reworded, because the count is the first
            // thing an adopter reads and "19 errors" for three faults is the
            // wrong number however carefully each line is phrased. This is what
            // a compiler does with a module that failed to parse, and it is safe
            // because only edges into *this* id are dropped: a reference to
            // something genuinely absent still reports below.
            if unloadable.contains_key(b) {
                continue;
            }
            out.push(
                Diagnostic::error(
                    code::UNKNOWN_DEP,
                    format!("`blocked_by` references unknown task `{b}`"),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!(
                    "Either create task `{b}` or remove it from `{}`'s `blocked_by`.",
                    t.id
                )),
            );
            continue;
        };

        // A done task whose blocker is not done means one of the two is lying,
        // unless a cut's own member was cancelled: a cut is complete when its
        // live members are (`A1-081`). A cancelled predecessor cut still counts,
        // for the reason below.
        let cancelled_member = crate::edge_kind(dep, t) == crate::EdgeKind::Membership;
        if t.status == Status::Done
            && !dep.status.satisfies_dependency()
            && !(cancelled_member && dep.status == Status::Dropped)
        {
            out.push(
                Diagnostic::error(
                    code::DONE_BEFORE_BLOCKER,
                    // Format the status via Display, not by lowercasing the whole
                    // message, that mangled the task id (`C-001` -> `c-001`),
                    // which is exactly the sort of thing this tool should notice.
                    format!("is `done` but blocker `{b}` is `{}`", dep.status),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!(
                    "Either close `{b}` with evidence, or `{}` was not actually done.",
                    t.id
                )),
            );
        }

        // A live task blocked by a dropped one can never legitimately start: the
        // plan is deadlocked. `Dropped` failing to satisfy a dependency already
        // shaped `ready`/`why`, but `check`, the gate, stayed silent, so a
        // provably unworkable plan passed CI clean.
        //
        // Live only: a `done` dependent is `APLAN007` above (the two must not
        // double-report one edge), and a dropped task waiting on a dropped task
        // is just two abandoned things.
        //
        // Not when a cut's own *member* was cancelled. A cut is a checkpoint over
        // its members, and one that will never be done should not stop the
        // checkpoint being reached; `roadmap` has excluded dropped members from a
        // cut's rollup all along, so reporting here made the gate disagree with
        // the view. Without this a task in a cut could not be cancelled at all:
        // leaving it in was this error and taking it out was `APLAN022`
        // (`A1-081`).
        //
        // A cancelled *predecessor cut* is a different fact and still reported: a
        // milestone in another milestone's `blocked_by` is a sequencing edge
        // (ADR-0013), and a cut whose stated prerequisite was abandoned is a plan
        // its author has to resolve, by dropping this cut too or by dropping the
        // edge. Exempting it would also contradict the members, who inherit that
        // edge and are told about it.
        let cancelled_member_of_this_cut = crate::edge_kind(dep, t) == crate::EdgeKind::Membership;
        if !t.status.is_terminal() && !cancelled_member_of_this_cut && dep.status == Status::Dropped
        {
            out.push(
                Diagnostic::error(
                    code::DROPPED_BLOCKER,
                    format!(
                        "is `{}` but blocker `{b}` is `dropped`: it can never legitimately start",
                        t.status
                    ),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!(
                    "Either drop `{}` too, revive `{b}`, or remove `{b}` from its `blocked_by` \
                     and name a real blocker.",
                    t.id
                )),
            );
        }
    }
}

/// `APLAN023`: a `done` task's `commit`/`file` citation can be path-checked.
///
/// Split out of [`check_evidence`] to keep that function readable, not because it
/// is a different concern.
/// `APLAN032`: every `paths` entry is a glob this tool can compile (`A1-066`).
///
/// Hermetic, and that is the repair rather than an implementation detail. The
/// ladder compiled these globs and returned an error that aborted the whole run:
/// one task's `paths: ['src/[']` hid `APLAN005` in a *different* task and exited
/// `2`, which ADR-0007 reserves for the invocation or `.aplan.yml`. Asking the
/// question here means `check` and `verify` give the same answer about the same
/// plan, and the rest of the plan is still judged.
///
/// The same builder the ladder uses, with the same options, so "compiles here"
/// and "compiles there" cannot drift: a glob this accepts and `globset` later
/// rejects would put the abort back.
/// `APLAN033`: every `prefer_after` entry is a preference that can be honoured
/// (`A1-150`).
///
/// Hermetic, like the edge checks beside it: all four shapes are answerable from
/// the plan alone. The cycle case needs the whole plan, so it is checked once in
/// [`check_preference_cycles`] rather than per task.
fn check_preferences(lt: &LoadedTask, graph: &Graph<'_>, out: &mut Vec<Diagnostic>) {
    let t = &lt.task;
    for id in &t.prefer_after {
        let finding = if id == &t.id {
            Some((
                "`prefer_after` names this task itself, which orders nothing".to_string(),
                "Remove the self-reference. A preference orders this task against another one."
                    .to_string(),
            ))
        } else if graph.get(id).is_none() {
            Some((
                format!("`prefer_after` names unknown task `{id}`"),
                format!(
                    "Either create task `{id}` or remove it from `{}`'s `prefer_after`.",
                    t.id
                ),
            ))
        } else if t.blocked_by.iter().any(|b| b == id) {
            Some((
                format!(
                    "`prefer_after` repeats `{id}`, which is already a `blocked_by` edge: the \
                     plan says both that this task cannot start until `{id}` is done and that it \
                     would merely rather follow it"
                ),
                format!(
                    "Remove `{id}` from `prefer_after`. A mandatory edge already orders the two, \
                     and states it more strongly."
                ),
            ))
        } else {
            None
        };
        if let Some((message, fix)) = finding {
            out.push(
                Diagnostic::error(code::UNUSABLE_PREFERENCE, message)
                    .with_task(&t.id)
                    .with_file(&lt.path)
                    .with_agent_instruction(fix),
            );
        }
    }
}

/// `APLAN033`'s whole-plan half: a cycle of preferences has no order to honour.
///
/// Reported once, on the task whose id sorts first in the cycle, because a cycle
/// reported N times is one defect wearing N hats -- the same choice `APLAN004`
/// makes for a dependency cycle, and the message names the ring the way that one
/// does.
fn check_preference_cycles(plan: &Plan, graph: &Graph<'_>, out: &mut Vec<Diagnostic>) {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for lt in &plan.tasks {
        if seen.contains(&lt.task.id) {
            continue;
        }
        // Walk preferences from this task; a revisit inside one walk is a ring.
        let mut path: Vec<String> = Vec::new();
        let mut on_path: BTreeSet<String> = BTreeSet::new();
        let mut cursor = lt.task.id.clone();
        loop {
            if on_path.contains(&cursor) {
                let start = path.iter().position(|p| *p == cursor).unwrap_or(0);
                let mut ring: Vec<String> = path[start..].to_vec();
                ring.push(cursor.clone());
                // Once per ring: every member is marked seen.
                for member in &ring {
                    seen.insert(member.clone());
                }
                let owner = ring.iter().min().cloned().unwrap_or_default();
                let file = plan
                    .tasks
                    .iter()
                    .find(|l| l.task.id == owner)
                    .map_or_else(|| lt.path.clone(), |l| l.path.clone());
                out.push(
                    Diagnostic::error(
                        code::UNUSABLE_PREFERENCE,
                        format!("preference cycle: {}", ring.join(" -> ")),
                    )
                    .with_task(&owner)
                    .with_file(&file)
                    .with_agent_instruction(
                        "Remove one `prefer_after` entry from the ring. A cycle of preferences \
                         has no order to offer them in.",
                    ),
                );
                break;
            }
            on_path.insert(cursor.clone());
            path.push(cursor.clone());
            let Some(task) = graph.get(&cursor) else {
                break;
            };
            // One outgoing edge is enough to find a ring reachable from here;
            // the others are walked from their own tasks.
            let Some(next) = task.prefer_after.first() else {
                break;
            };
            cursor = next.clone();
        }
    }
}

fn check_path_globs(lt: &LoadedTask, out: &mut Vec<Diagnostic>) {
    let t = &lt.task;
    // Once per distinct entry: a task that writes `src/[` twice has one typo,
    // and saying the same sentence about it twice is noise `APLAN040` already
    // reports as a duplicate (`A1-267`'s audit).
    let mut said: std::collections::BTreeSet<&String> = std::collections::BTreeSet::new();
    for p in &t.paths {
        if !said.insert(p) {
            continue;
        }
        // A blank entry is `APLAN023`'s business (it names nothing), and it
        // compiles, so it never reaches here.
        let Err(err) = crate::glob::compile(p) else {
            continue;
        };
        // `globset`'s own words, which name the position and the character, are
        // more use than any sentence this rule could invent: "unclosed character
        // class; missing ']'" is the whole diagnosis.
        out.push(
            Diagnostic::error(
                code::BAD_PATH_GLOB,
                format!(
                    "`paths` entry `{p}` is not a glob this tool can compile: {err}, so this \
                     task's evidence cannot be bound to its `paths` at all"
                ),
            )
            .with_task(&t.id)
            .with_file(&lt.path)
            .with_agent_instruction(
                "Correct the `paths` entry so it is a valid glob. `*` matches within one path \
                 segment, `**` crosses directories, and `[` opens a character class that has \
                 to be closed.",
            ),
        );
    }
}

/// `APLAN040`: the same `paths` entry, twice.
///
/// Walks the list once and reports each spelling that appears more than once,
/// naming the entry as the author wrote it -- because that is the text they have
/// to find in their own file -- while comparing the normalised form, so two
/// writings of one path are one entry (`A1-254`, `A1-267`).
fn check_duplicate_paths(lt: &LoadedTask, out: &mut Vec<Diagnostic>) {
    let t = &lt.task;
    // Spelling -> (how many, the writings the author used, in order).
    let mut seen: std::collections::BTreeMap<String, Vec<&String>> =
        std::collections::BTreeMap::new();
    for p in &t.paths {
        // An entry that does not compile is `APLAN032`'s, and "you wrote it
        // twice" is not the useful thing to say about `src/[`. The same line
        // this rule declines to cross for containment: it speaks about entries
        // that name files, not about ones that name nothing at all
        // (`A1-267`'s audit).
        if crate::glob::compile(p).is_err() {
            continue;
        }
        seen.entry(crate::glob::spelling(p)).or_default().push(p);
    }
    for (_, written) in seen {
        if written.len() < 2 {
            continue;
        }
        let count = written.len();
        // Named as the author wrote them, because that is the text they have to
        // find in their own file. Where the writings differ, both are named:
        // saying `src/**` twice at somebody who wrote `./src/**` once sends them
        // looking for a line their file does not contain.
        let distinct: std::collections::BTreeSet<&&String> = written.iter().collect();
        // An entry nobody filled in renders as empty backticks otherwise, which
        // reads as a bug in the message rather than a fact about the file. A
        // single blank is nobody's finding today; two of them are still an edit
        // that went wrong (`A1-267`'s audit).
        let name = |w: &str| {
            if w.trim().is_empty() {
                "an empty entry".to_string()
            } else {
                format!("`{w}`")
            }
        };
        let how = if distinct.len() == 1 {
            format!("{} {count} times", name(written[0]))
        } else {
            let list: Vec<String> = distinct.iter().map(|w| name(w)).collect();
            // With the count: three entries written two ways is not the same
            // fact as two, and the branch above says how many while this one
            // said only which (`A1-267`'s audit).
            // Not "`./x` and `x` resolve alike", which names one of the two
            // shapes `A1-254` normalises and would be the wrong sentence for the
            // other: `a/../b` reduces to `b` as well.
            format!(
                "{} -- {count} entries, and one path once `.` and `..` segments are resolved",
                list.join(" and ")
            )
        };
        out.push(
            Diagnostic::warning(
                code::DUPLICATE_PATH,
                format!(
                    "`paths` lists {how}. The copies match the same files, so nothing is \
                     weakened -- but a repeated entry is usually an edit that went wrong"
                ),
            )
            .with_task(&t.id)
            .with_file(&lt.path)
            .with_agent_instruction(
                "Delete the repeated entry. If the two were meant to name different trees, one \
                 of them is not the path you meant.",
            ),
        );
    }
}

fn check_uncheckable_evidence(lt: &LoadedTask, out: &mut Vec<Diagnostic>) {
    let t = &lt.task;
    // A `done` task whose evidence cannot be bound to it. `APLAN012` makes a
    // `commit` citation mean "a commit that touched this work" by checking the
    // diff against `paths`; with no `paths` there is nothing to check, so the
    // same citation degrades to "a commit exists" with no diagnostic anywhere.
    // Every configuration-independent path-local kind is included for the same
    // reason. Test locality is configuration-dependent: without a runner making
    // a coverage claim there is no check to lose, so this hermetic rule stays
    // silent just as it did before the capability was modelled.
    //
    // Deliberately scoped to `done`. A `todo` task often cannot know its paths
    // yet, and warning there would train people to add `paths: ["**"]` to quiet
    // it, which is the bypass this rule must not encourage. Milestones are exempt
    // because `APLAN007` already refuses to mark one `done` while a member is
    // undone, which proves more than a path check could (`APLAN021` also forbids
    // them carrying `paths` at all).
    //
    // A blank entry counts as no entry, by the same predicate that decides an
    // evidence `ref` cites nothing. `paths: [""]` binds no commit that `paths: []`
    // would not, and a single zero-width space would otherwise silence this rule
    // while looking, in every editor, like a declared path. How *broad* a real
    // glob is stays a review question (see the task): this only asks whether
    // anything was named at all.
    let names_nothing = t.paths.iter().all(|p| is_vacuous_ref(p));
    if t.status == Status::Done && names_nothing && !t.is_milestone() {
        let mut kinds: Vec<&str> = t
            .evidence
            .iter()
            .filter(|e| !is_vacuous_ref(&e.reference))
            .filter(|e| {
                e.kind
                    .capabilities()
                    .locality
                    .is_configuration_independent()
            })
            .map(|e| e.kind.as_str())
            .collect();
        kinds.sort_unstable();
        kinds.dedup();

        if !kinds.is_empty() {
            let cited = kinds.join(" and ");
            let citation = if kinds.len() > 1 {
                "citations resolve"
            } else {
                "citation resolves"
            };
            let referent = if kinds.len() > 1 { "them" } else { "it" };
            // Say which it is: told "no `paths`" when the key is right there, a
            // reader looks for a bug in the rule rather than at their blank entry.
            let missing = if t.paths.is_empty() {
                "no `paths`"
            } else {
                "no `paths` that name anything"
            };
            out.push(
                Diagnostic::warning(
                    code::UNCHECKABLE_EVIDENCE,
                    format!(
                        "`done` with {cited} evidence but {missing}: even if the {citation}, \
                         nothing binds {referent} to this task; resolution alone is not \
                         evidence that the task's files changed"
                    ),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                // Only promise the `APLAN012` check when a `commit` is actually
                // cited. An instruction is the field a coding agent acts on
                // without reading the rest, so telling it `verify` will check "the
                // cited commit" of a task citing only a `file` is a plain untruth,
                // and nothing binds a `file` to `paths` yet (`A1-045`).
                .with_agent_instruction(if kinds.contains(&"commit") {
                    "Add `paths` naming the files this task is responsible for, so `verify` \
                     can check the cited commit actually touches them (APLAN012)."
                } else {
                    "Add `paths` naming the files this task is responsible for, so the \
                     citation is bound to this task rather than only proving the file exists."
                }),
            );
        }
    }
}

/// The invariants that read `evidence`: `APLAN005`, `APLAN010`, `APLAN055`.
fn check_evidence(
    lt: &LoadedTask,
    opts: &Options,
    cut_of: &BTreeMap<&str, (&str, &str)>,
    out: &mut Vec<Diagnostic>,
) {
    let t = &lt.task;

    // Evidence with a blank `ref` is vacuous, a citation pointing at nothing, so
    // it must not count toward "done has proof". Otherwise `status: done` with
    // `ref: ""` sails through the hermetic gate: `verify` catches an empty
    // commit/file/test ref (it fails to resolve), but `url` and `note` have no
    // resolver, so `check` is the only rung that can object for those kinds.
    let has_substantive_evidence = t.evidence.iter().any(|e| !is_vacuous_ref(&e.reference));

    // The keystone: done requires substantive evidence.
    if t.status == Status::Done && !has_substantive_evidence {
        // Keep the no-evidence wording byte-for-byte (it is a pinned contract);
        // name the blank-`ref` case distinctly.
        let message = if t.evidence.is_empty() {
            "`status: done` with no `evidence`: a status is a claim, not a proof".to_string()
        } else {
            "`status: done` but every `evidence` entry has a blank `ref`: a status is a claim, \
             not a proof"
                .to_string()
        };
        out.push(
            Diagnostic::error(code::DONE_WITHOUT_EVIDENCE, message)
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(
                    "Add at least one evidence entry (commit, url, file, test, or a dated note), \
                     or move the task back to `in_progress`.",
                ),
        );
    }

    // A note's authored date is provenance. On a machine-resolved kind the same
    // field would be unchecked decoration that looks like freshness, so reject
    // it rather than silently carrying a claim no resolver reads.
    for e in &t.evidence {
        let capture = e.kind.capabilities().capture;
        let detector = capture_detector(e.kind);
        match (capture, &e.captured) {
            (EvidenceCapture::AuthoredDate { .. }, None) => out.push(
                Diagnostic::error(
                    detector,
                    "evidence of kind `note` requires a `captured` date",
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction("Add `captured: YYYY-MM-DD` to the note evidence entry."),
            ),
            // A present-but-nonsense date defeats the rule's purpose ("a note
            // carries a date") just as an absent one does.
            (EvidenceCapture::AuthoredDate { .. }, Some(d)) if !is_iso_date(d) => out.push(
                Diagnostic::error(
                    detector,
                    format!("note evidence `captured: {d}` is not an ISO-8601 date (YYYY-MM-DD)"),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(
                    "Write the note's `captured` date as YYYY-MM-DD (for example 2026-07-18).",
                ),
            ),
            (EvidenceCapture::NotApplicable { reason, .. }, Some(d)) => out.push(
                Diagnostic::error(
                    detector,
                    format!(
                        "`{}` evidence carries `captured: {d}`, but that date has no provenance \
                         semantics for this kind: {reason}",
                        e.kind.as_str()
                    ),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!(
                    "Remove `captured` from this `{}` evidence entry. `aplan` accepts the field \
                     only on authored `note` evidence; use this kind's resolver or freshness \
                     mechanism instead.",
                    e.kind.as_str()
                )),
            ),
            (EvidenceCapture::AuthoredDate { .. }, Some(_))
            | (EvidenceCapture::NotApplicable { .. }, None) => {}
        }
    }

    check_uncheckable_evidence(lt, out);

    // Configured minimum evidence strength. A `done` task must carry at least one
    // piece of an accepted kind *with a real reference*, unless a tag exempts it.
    // Vacuous evidence is `APLAN005`'s business, so this only speaks when there is
    // substantive evidence but none of it is strong enough, which is the
    // shallow-`done` this rule exists to catch.
    // The policy that applies *here*, which is the whole-plan one unless a scope
    // names a narrower subset this task is in (`A1-183`).
    let (milestone, cut) = cut_of
        .get(t.id.as_str())
        .map_or((None, None), |(m, c)| (Some(*m), Some(*c)));
    let (required, exempt) = opts.evidence_for(t, cut, milestone);
    // An empty list is a real answer: "nothing is required in this subset". It is
    // how `evidence.exempt_tags` is expressible as a scope (`A1-183`), and
    // without this the rule fired asking for one of nothing.
    if let Some(required) = required.filter(|r| !r.is_empty())
        && t.status == Status::Done
        && has_substantive_evidence
        && !t.tags.iter().any(|tag| exempt.contains(tag))
        && !t
            .evidence
            .iter()
            .any(|e| required.contains(&e.kind) && !is_vacuous_ref(&e.reference))
    {
        let kinds = required
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        out.push(
            Diagnostic::error(
                code::WEAK_EVIDENCE,
                format!("`done` but no evidence of a required kind (need one of: {kinds})"),
            )
            .with_task(&t.id)
            .with_file(&lt.path)
            .with_agent_instruction(format!(
                "Add evidence of a stronger kind ({kinds}), or exempt the task with a configured \
                 tag if nothing stronger exists."
            )),
        );
    }
}

/// `APLAN038`: a `done` task has answered the criteria its own body states
/// (`A1-224`).
///
/// Reads the loader's count rather than the body: the model does not carry
/// prose, and one fact out of it is all a rule needs.
///
/// Silent where the body states none. Ten of this plan's closed tasks do, and a
/// rule about a markdown convention must not demand the convention.
/// `APLAN039`: the scaffold, ticked as though somebody had answered it.
///
/// Independent of `APLAN038` and deliberately so: a task may tick the scaffold
/// *and* write real criteria beside it, in which case the count is honest and
/// the false claim is still recorded. Both can fire on one task, saying
/// different things.
fn check_scaffold(lt: &crate::load::LoadedTask, out: &mut Vec<Diagnostic>) {
    let t = &lt.task;
    let (Status::Done, Some(line)) = (t.status, lt.criteria.scaffold_ticked) else {
        return;
    };
    // The second half of one finding rather than a second rule: a task carrying
    // both has answered neither of the two questions `add` asked it.
    let and_why = if lt.criteria.why_untouched {
        ", and its `## Why` is still the prompt too"
    } else {
        ""
    };
    out.push(
        Diagnostic::warning(
            code::SCAFFOLD_TICKED,
            format!(
                "is `done` and ticks a prompt this tool wrote{and_why}: `{}` asks for a \
                 criterion, and is not one",
                lt.criteria.scaffold_prompt.unwrap_or_default()
            ),
        )
        .with_task(&t.id)
        .with_file(&lt.path)
        .at(crate::diagnostic::Location { line, column: 1 })
        .with_agent_instruction(
            "Replace that line with what done means for this task, in this repository's terms, \
             and tick it only when it is met. If the task is finished and nobody wrote the \
             criterion, say what was delivered instead. Do not delete the line to silence this.",
        ),
    );
}

/// `APLAN043`: the scaffold left behind beside the criteria somebody wrote
/// (`A1-299`).
///
/// There is no command that edits a task's body, so the real criteria go in by
/// hand -- and appending a `## Acceptance criteria` of your own to a scaffolded
/// file leaves two of them, with the prompt still under the first. The sixth
/// pilot did that on its first day and a human found it, not the tool.
///
/// **Live tasks only.** On a `done` task the same leftover is already reported:
/// unticked by `APLAN038`, ticked by `APLAN039`. The window this covers is
/// between authoring and closing, where the author is still in the file.
///
/// **Beside other criteria, never alone.** A task whose only criterion is the
/// prompt has not answered what `add` asked, which is a different thing and
/// `APLAN038`'s at close. This one is about the scaffold *surviving* an answer.
fn check_scaffold_leftover(lt: &crate::load::LoadedTask, out: &mut Vec<Diagnostic>) {
    let t = &lt.task;
    // **Ticked or not.** The rule is about the scaffold *surviving* an answer,
    // and a tick does not make it survive less -- it makes the line assert that
    // the instruction was met, which is worse. `APLAN039` owns the ticked case on
    // a *closed* task, so widening here reports nothing twice; it closes the same
    // window this rule exists for. Zero tasks on this host carry the shape today,
    // so the widening costs no finding and removes a hole an author reaches by
    // ticking a box (`A1-299`, audit).
    let Some(line) = lt
        .criteria
        .scaffold_unticked
        .or(lt.criteria.scaffold_ticked)
    else {
        return;
    };
    let own = lt.criteria.total.saturating_sub(lt.criteria.prompts);
    if t.status.is_terminal() || own == 0 {
        return;
    }
    out.push(
        Diagnostic::warning(
            code::SCAFFOLD_LEFTOVER,
            format!(
                "carries the scaffold's own prompt beside {own} criteri{} of its own: `{}` is \
                 the line `aplan add` writes to be replaced, not kept",
                if own == 1 { "on" } else { "a" },
                lt.criteria.scaffold_prompt.unwrap_or_default()
            ),
        )
        .with_task(&t.id)
        .with_file(&lt.path)
        .at(crate::diagnostic::Location { line, column: 1 })
        .with_agent_instruction(
            "Delete that line, and the `## Acceptance criteria` heading it was written under if \
             that heading now has nothing else beneath it. Keep the criteria you wrote. A body \
             with two of that heading is the scaffold left beside the answer.",
        ),
    );
}

/// `APLAN044`: a closed task claims less than its level implies and says why
/// nowhere (`A1-312`, ADR-0034).
///
/// **Declaring nothing is not a finding.** A task's level supplies a floor, so
/// the ordinary task writes no impact at all and the plan still knows what it
/// owes -- which is what makes this adoptable on a plan of three hundred tasks
/// rather than a demand that every one of them be annotated.
///
/// What is reported is the author overriding that floor *downwards* in silence.
/// Overriding it is legitimate -- a pilot log really does change nothing that
/// ships -- and a reviewer should be told, because the claim is the one thing
/// nothing else can check: no hermetic rule knows whether a task broke an API.
///
/// Closed tasks only, and `done` rather than terminal: a `dropped` task ships
/// nothing, so what it would have done to the version is not a question anyone
/// needs answered.
/// `APLAN050`: a recorded `version` is on a release, and is a version
/// (`A1-339`, ADR-0038).
///
/// Hermetic, and unconditional: unlike `APLAN044` this needs no opt-in, because
/// the field does not exist until something writes it and the only thing that
/// writes it is `close` on a release. A `version` anywhere else was put there by
/// hand, and both ways of getting it wrong are answerable from the plan alone.
fn check_version_record(lt: &crate::load::LoadedTask, out: &mut Vec<Diagnostic>) {
    let t = &lt.task;
    let Some(recorded) = &t.version else {
        return;
    };
    let mut report = |message: String, fix: &str| {
        let mut d = Diagnostic::error(code::VERSION_RECORD_UNUSABLE, message)
            .with_task(&t.id)
            .with_file(&lt.path)
            .with_agent_instruction(fix.to_string());
        if let Some(at) = lt.fields.get("version") {
            d = d.at(*at);
        }
        out.push(d);
    };
    if !t.is_release() {
        report(
            format!(
                "records `version: {recorded}`, which says what a *release* produced. This \
                 node is not one, and a cut is a planning bucket rather than a delivery"
            ),
            "Remove `version`, or tag this node `release` if it really is the thing that \
             ships.",
        );
        return;
    }
    if crate::version::triple(recorded).is_none() {
        report(
            format!(
                "records `version: {recorded}`, which is not a version this tool can read \
                 back. The point of writing it down is that a later run compares against it, \
                 and a record that reads as an absence is worse than none"
            ),
            "Write the version the release produced as `major.minor.patch`, or remove the \
             field and let it be derived.",
        );
    }
}

fn check_version_impact(lt: &crate::load::LoadedTask, opts: &Options, out: &mut Vec<Diagnostic>) {
    let Some(floors) = opts.require_impact else {
        return;
    };
    let t = &lt.task;
    if t.status != Status::Done {
        return;
    }
    let Some(claimed) = t.version_impact else {
        return;
    };
    let floor = floors.of(t);
    if claimed >= floor || t.version_impact_reason.is_some() {
        return;
    }
    let mut d = Diagnostic::warning(
        code::VERSION_IMPACT_UNEXPLAINED,
        format!(
            "claims `{claimed}` of the released version where its level implies `{floor}`, and \
             says nowhere why"
        ),
    )
    .with_task(&t.id)
    .with_file(&lt.path)
    .with_agent_instruction(
        "Add `version_impact_reason` saying what this task changed that the level's floor gets \
         wrong -- a plan edit or a pilot log really does change nothing that ships. If it did \
         change the artifact after all, raise `version_impact` instead.",
    );
    if let Some(at) = lt.fields.get("version_impact") {
        d = d.at(*at);
    }
    out.push(d);
}

fn check_criteria(lt: &crate::load::LoadedTask, out: &mut Vec<Diagnostic>) {
    let t = &lt.task;
    if t.status != Status::Done || !lt.criteria.unanswered() {
        return;
    }
    let (ticked, total) = (lt.criteria.ticked, lt.criteria.total);
    let mut d = Diagnostic::warning(
        code::CRITERIA_UNANSWERED,
        format!("is `done` with {ticked} of {total} criteria in its body ticked"),
    )
    .with_task(&t.id)
    .with_file(&lt.path)
    .with_agent_instruction(
        "Re-read the criteria and tick the ones that are met. If one is not met, the task is \
         not done: say what is left, supersede it, or hand that criterion to the task that will \
         do it and tick it as a decision, naming where it went. Nothing ticks them for you.",
    );
    // *In its body*, not "acceptance criteria": nine of this plan's own
    // milestones head that section `## Exit criteria`, and an adopter's
    // convention is theirs to choose. Naming a heading this rule never read
    // would be a message describing a file it is not looking at, and the line
    // below takes the reader to the box itself anyway.
    //
    // The first unticked box, so a reader opens the file at the line rather than
    // at the top of it (`A1-202`).
    if let Some(line) = lt.criteria.first_unticked {
        d = d.at(crate::diagnostic::Location { line, column: 1 });
    }
    out.push(d);
}

/// An RFC 3339 instant as seconds since the epoch, or `None` if it is not one.
///
/// Strict and dependency-free, for the reason [`is_iso_date`] gives: this is a
/// validation gate rather than a date library. It accepts
/// `YYYY-MM-DDTHH:MM:SSZ` and `YYYY-MM-DDTHH:MM:SS+HH:MM` (or `-HH:MM`), in
/// upper case, and nothing else.
///
/// Fractional seconds are refused deliberately. A revisit instant is a hand-off
/// to a scheduler that will not run to the millisecond anyway, and accepting a
/// form nobody needs is a second spelling of one value for every consumer to
/// handle (`A1-264`).
#[must_use]
pub fn rfc3339_to_epoch(s: &str) -> Option<i64> {
    // ASCII first, and this is not a nicety: everything below slices by byte
    // offset, and a multi-byte character straddling one of them panics. An
    // instant is ASCII by definition, so anything else is a `None` -- but this
    // reads a *task file*, and a validator that crashes on the plan it was
    // asked to judge is the worst answer available (`A1-264`, audit: measured
    // as `aplan check` exiting 101 on `revisit_at: 2026-09-10T05:00:0é`).
    if !s.is_ascii() {
        return None;
    }
    let bytes = s.as_bytes();
    if bytes.len() < 20 || bytes[10] != b'T' {
        return None;
    }
    let day = iso_date_to_epoch(&s[0..10])?;
    let time = &s[11..19];
    let digits = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = time.get(r)?;
        part.bytes().all(|b| b.is_ascii_digit()).then_some(())?;
        part.parse().ok()
    };
    if time.as_bytes().get(2) != Some(&b':') || time.as_bytes().get(5) != Some(&b':') {
        return None;
    }
    let (h, m, sec) = (digits(0..2)?, digits(3..5)?, digits(6..8)?);
    // A leap second is a real RFC 3339 value and is not a real instant to a
    // scheduler; 60 is refused rather than silently folded into the next minute.
    if h > 23 || m > 59 || sec > 59 {
        return None;
    }
    let offset = match &s[19..] {
        "Z" => 0,
        rest => {
            let sign = match rest.as_bytes().first() {
                Some(b'+') => 1,
                Some(b'-') => -1,
                _ => return None,
            };
            if rest.len() != 6 || rest.as_bytes()[3] != b':' {
                return None;
            }
            let oh: i64 = rest[1..3].parse().ok()?;
            let om: i64 = rest[4..6].parse().ok()?;
            if oh > 23 || om > 59 {
                return None;
            }
            sign * (oh * 3600 + om * 60)
        }
    };
    // The offset is *removed* to reach UTC: `12:00+02:00` is 10:00 UTC.
    Some(day + h * 3600 + m * 60 + sec - offset)
}

/// An ISO-8601 date as seconds since the Unix epoch, at midnight UTC.
///
/// `None` for anything [`is_iso_date`] rejects. Hand-rolled rather than pulling
/// in a date library for one conversion: the civil-to-days algorithm is short,
/// exact for every date this tool will meet, and has no timezone opinion to get
/// wrong -- which matters, because the value it produces is compared against a
/// git commit time (`A1-162`).
#[must_use]
pub fn iso_date_to_epoch(s: &str) -> Option<i64> {
    if !is_iso_date(s) {
        return None;
    }
    let y: i64 = s[0..4].parse().ok()?;
    let m: i64 = s[5..7].parse().ok()?;
    let d: i64 = s[8..10].parse().ok()?;
    // Howard Hinnant's `days_from_civil`: shift the year so March is month 1,
    // which makes the leap day the last day of the year and the arithmetic
    // branch-free.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400)
}

/// Whether `s` is an ISO-8601 calendar date (`YYYY-MM-DD`) with a real month and
/// day. Deliberately strict and dependency-free: this is a validation gate, not a
/// date library, so it accepts only the one unambiguous format the schema comment
/// and the docs ask for, and rejects impossible dates like `2026-13-40`.
///
/// Public because anything that *produces* a `captured` date must agree with the
/// rule that judges it. A command validating its own input against a private
/// re-implementation is how you get a date accepted at the boundary and then
/// rejected by `APLAN010` after the file is written.
pub fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    if !(b[0..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..10].iter().all(u8::is_ascii_digit))
    {
        return false;
    }
    let year: i32 = s[0..4].parse().unwrap_or(0);
    let month: u32 = s[5..7].parse().unwrap_or(0);
    let day: u32 = s[8..10].parse().unwrap_or(0);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=max_day).contains(&day)
}

/// Verify that `trace` (`path/to/doc.md#anchor`) resolves to a real file, and,
/// when an anchor is given, that a heading slugifying to it exists.
/// Whether `target` lies under `base`, both already absolute and normalised.
fn is_inside(base: &Path, target: &Path) -> bool {
    target.starts_with(base)
}

/// A path made absolute and lexically normalised, for comparing one against
/// another (`A1-143`).
///
/// `std::path::absolute` is lexical -- it consults the current directory for a
/// relative path and touches nothing else -- and it keeps `..` components, so
/// the normalisation still has to run afterwards.
fn comparable(path: &Path) -> std::path::PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    crate::anchor::normalise(&absolute)
}

/// `APLAN034`: a `traces` target that leaves the repository must be declared
/// (`A1-143`, ADR-0021).
///
/// Two tests, and both have to pass. **Lexically**, because the question has to
/// be answerable from the plan and the config alone: resolving on the filesystem
/// would make the answer depend on whether the sibling happens to be checked
/// out, which is the dependency this rule exists to surface. And **through
/// symlinks when the target exists**, because a link committed inside the
/// repository is lexically inside it and reads whatever it points at -- and a
/// task file and a symlink can arrive in the same fork pull request (`A1-143`,
/// found by its audit).
fn check_external_trace(root: &Path, opts: &Options, trace: &str) -> Option<Diagnostic> {
    let (rel, _) = trace.split_once('#').unwrap_or((trace, ""));
    let joined = root.join(rel);
    // The boundary is the *plan's repository*, not the policy file and not
    // necessarily `traces_root`. A selected config may deliberately live
    // outside an untrusted tree (`A1-237`); its directory remains the frame for
    // relative config values but says nothing about what repository the plan
    // belongs to. With no repository supplied, the root is the only boundary a
    // library caller has.
    let boundary = opts.traces_repository.as_deref().unwrap_or(root);
    let declared = &opts.traces_external;
    let bases: Vec<std::path::PathBuf> = std::iter::once(comparable(boundary))
        .chain(declared.iter().map(|d| comparable(d)))
        .collect();

    let target = comparable(&joined);
    let inside_lexically = bases.iter().any(|b| is_inside(b, &target));
    // The link test only runs where the file is there to follow. A path that
    // does not exist cannot be a link, and a *declared* root that is absent is
    // reported as a configuration error rather than here.
    let inside_really = joined.canonicalize().is_ok_and(|real| {
        let bases: Vec<std::path::PathBuf> = std::iter::once(boundary.to_path_buf())
            .chain(declared.iter().cloned())
            .filter_map(|b| b.canonicalize().ok())
            .collect();
        bases.is_empty() || bases.iter().any(|b| real.starts_with(b))
    }) || joined.canonicalize().is_err();
    if inside_lexically && inside_really {
        return None;
    }

    let how = if inside_lexically {
        "resolves outside this repository through a symbolic link"
    } else {
        "is outside this repository"
    };
    Some(
        Diagnostic::error(
            code::EXTERNAL_TRACE,
            format!(
                "`traces` target `{rel}` {how}, and no `traces_external` declares it: a plan \
                 that reads another repository's files passes only where that repository \
                 happens to be checked out"
            ),
        )
        .with_agent_instruction(
            "Trace a document in this repository, or declare the sibling root in \
             `traces_external` so its absence is a configuration error rather than a surprise \
             in one CI job.",
        ),
    )
}

fn check_trace(root: &Path, trace: &str, documents: &mut TraceDocuments) -> Option<Diagnostic> {
    let (rel, anchor) = match trace.split_once('#') {
        Some((r, a)) => (r, Some(a)),
        None => (trace, None),
    };
    let path = root.join(rel);
    if !path.is_file() {
        return Some(
            Diagnostic::error(
                code::DANGLING_TRACE,
                format!("`traces` target `{rel}` does not exist"),
            )
            .with_agent_instruction(format!("Create `{rel}` or correct the trace path.")),
        );
    }
    let anchor = anchor?;
    // `DOC.md#` is a typo, not a trace of the whole document: the author meant to
    // name a section and stopped. Saying "add a heading whose text slugifies to
    // ``" sends them to fix the document rather than the trace (`A1-116`).
    if anchor.trim().is_empty() {
        return Some(
            Diagnostic::error(
                code::DANGLING_TRACE,
                format!("`traces` entry `{rel}#` ends in an empty anchor"),
            )
            .with_agent_instruction(format!(
                "Name the section after the `#`, or write `{rel}` on its own to trace the \
                 whole document."
            )),
        );
    }
    match documents.get(root, rel) {
        TraceDocument::Unreadable => None,
        TraceDocument::Structured { format, document } => {
            check_structured_trace(rel, document.as_ref(), anchor, *format)
        }
        TraceDocument::Markdown(document) => {
            // GitHub assigns each heading an anchor by slugifying it and
            // disambiguating repeats with a `-N` suffix. The snapshot index
            // reproduces that set once, then answers every task from it.
            if document.resolves(anchor) {
                None
            } else {
                Some(
                    Diagnostic::error(
                        code::DANGLING_TRACE,
                        format!(
                            "`traces` anchor `#{anchor}` matches no heading or table row in \
                             `{rel}`"
                        ),
                    )
                    .with_agent_instruction(format!(
                        "Add to `{rel}` a heading, or a table row, whose text slugifies to \
                         `{anchor}`; or correct the anchor to one the document already has. A \
                         **list item is not an anchor**, deliberately: a bullet's identity would \
                         be its whole text, so rewording it would break the reference rather than \
                         report drift. Give the item a heading or a table row, which is what an \
                         author tracking items one by one wants anyway."
                    )),
                )
            }
        }
    }
}

/// `APLAN008` for a trace into YAML or JSON (`A1-191`, ADR-0035).
///
/// Three different failures, told apart because the remedy differs: the document
/// is not addressable at all, the anchor is not an address, or it is an address
/// to a node that is not there. The last is the one an author hits most, so it
/// carries the near miss, offered in whichever spelling they were already using.
fn check_structured_trace(
    rel: &str,
    document: Option<&crate::structured::Document>,
    anchor: &str,
    format: crate::structured::Format,
) -> Option<Diagnostic> {
    use crate::structured::{Spelling, parse_anchor, spelling_of};

    let kind = match format {
        crate::structured::Format::Yaml => "YAML",
        crate::structured::Format::Json => "JSON",
    };
    let Some(document) = document else {
        return Some(
            Diagnostic::error(
                code::DANGLING_TRACE,
                format!(
                    "`traces` target `{rel}` is not one {kind} document this tool can address \
                     into: it does not parse, holds more than one document, nests deeper than \
                     128 levels, or uses a shape with no JSON equivalent such as a non-string key"
                ),
            )
            .with_agent_instruction(format!(
                "Fix `{rel}` so it parses, or trace a document that does. Addressing half of a \
                 file this tool could not fully read would make the fingerprint mean something \
                 nobody chose."
            )),
        );
    };
    let steps = match parse_anchor(anchor) {
        Ok(steps) => steps,
        Err(refusal) => {
            return Some(
                Diagnostic::error(
                    code::DANGLING_TRACE,
                    format!(
                        "`traces` anchor `#{anchor}` is not an address into `{rel}`: `{}` cannot \
                         be part of one, because {}",
                        refusal.construct, refusal.because
                    ),
                )
                // The decision number stays in this comment: the instruction is
                // printed into a consumer's terminal, where this project's ADRs
                // do not resolve (`A1-044`). ADR-0026 is the one that refuses a
                // query language, and ADR-0035 the one that accepts the singular
                // subset; the sentence has to carry the reason without them.
                .with_agent_instruction(format!(
                    "Address one node of `{rel}`: a JSON Pointer (`#/states/provisioning`, \
                     RFC 6901) or a JSONPath (`#$.states.provisioning`, RFC 9535). Only the \
                     singular subset of JSONPath is accepted, names and whole-number indices, \
                     because a fingerprint is taken of one node, and this tool resolves an \
                     address rather than running a query."
                )),
            );
        }
    };
    if document.resolve(&steps).is_some() {
        return None;
    }
    let spelling = spelling_of(anchor).unwrap_or(Spelling::Pointer);
    let near = near_address(&document.addresses(spelling, 5_000), anchor);
    let suggestion = near.map_or_else(
        || format!("correct the anchor to one `{rel}` has."),
        |n| format!("did you mean `#{n}`?"),
    );
    Some(
        Diagnostic::error(
            code::DANGLING_TRACE,
            format!("`traces` anchor `#{anchor}` addresses no node in `{rel}`"),
        )
        .with_agent_instruction(format!(
            "Add that node to `{rel}`, or {suggestion} An address is checked against the \
             document as it is now, so a key that was renamed is reported here rather than \
             silently tracing nothing."
        )),
    )
}

/// The closest address to what the author wrote, if one is close enough to be
/// worth printing.
///
/// One edit, the same bound and the same helper `APLAN026` uses for a tag: a
/// suggestion that is not nearly right is worse than none, because the reader
/// has to rule it out. A wrong segment rather than a mistyped one is many edits
/// away and gets the general remedy instead.
fn near_address(known: &BTreeSet<String>, anchor: &str) -> Option<String> {
    known.iter().find(|k| is_one_edit_apart(k, anchor)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::LoadedTask;
    use crate::task::{Evidence, Task};
    use std::path::PathBuf;

    fn scope(
        dims: usize,
        sel: crate::select::Selector,
        kinds: Option<Vec<EvidenceKind>>,
    ) -> EvidenceScope {
        EvidenceScope {
            selector: sel,
            named_dimensions: dims,
            require_min_strength: kinds,
            exempt_tags: None,
        }
    }

    /// **The narrowest matching scope wins, and the whole-plan setting is the one
    /// that names nothing** (`A1-183`).
    #[test]
    fn a_narrower_scope_beats_a_wider_one_and_both_beat_the_global() {
        let mut task = t("T-1", Status::Done);
        task.tags = vec!["spike".into()];
        task.owner = Some("kim".into());
        let opts = Options {
            require_min_strength: Some(vec![EvidenceKind::Test]),
            evidence_scopes: vec![
                // One dimension: every spike.
                scope(
                    1,
                    crate::select::Selector {
                        tags: vec!["spike".into()],
                        ..Default::default()
                    },
                    Some(vec![EvidenceKind::Commit]),
                ),
                // Two: this owner's spikes, which is narrower and must win.
                scope(
                    2,
                    crate::select::Selector {
                        tags: vec!["spike".into()],
                        owners: vec!["kim".into()],
                        ..Default::default()
                    },
                    Some(vec![EvidenceKind::File]),
                ),
            ],
            ..Options::default()
        };
        let (required, _) = opts.evidence_for(&task, None, None);
        assert_eq!(
            required,
            Some([EvidenceKind::File].as_slice()),
            "the two-dimension scope did not win"
        );

        // A task in neither scope falls through to the whole-plan setting.
        let other = t("T-2", Status::Done);
        let (required, _) = opts.evidence_for(&other, None, None);
        assert_eq!(required, Some([EvidenceKind::Test].as_slice()));
    }

    /// **A scope that sets one dial does not reset the other**, which is what
    /// resolving the winning scope as a whole did (`A1-183`'s audit).
    ///
    /// Each key comes from the narrowest matching scope that *sets* it. Here the
    /// narrow scope sets only the exemption, so the requirement must still come
    /// from the wider scope that sets one, not from the whole-plan value.
    #[test]
    fn a_scope_setting_one_dial_does_not_discard_a_wider_scopes_other_one() {
        let mut task = t("T-1", Status::Done);
        task.tags = vec!["spike".into()];
        task.owner = Some("kim".into());
        let opts = Options {
            require_min_strength: Some(vec![EvidenceKind::Note]),
            evidence_scopes: vec![
                scope(
                    1,
                    crate::select::Selector {
                        tags: vec!["spike".into()],
                        ..Default::default()
                    },
                    Some(vec![EvidenceKind::Test]),
                ),
                EvidenceScope {
                    selector: crate::select::Selector {
                        tags: vec!["spike".into()],
                        owners: vec!["kim".into()],
                        ..Default::default()
                    },
                    named_dimensions: 2,
                    require_min_strength: None,
                    exempt_tags: Some(vec!["waived".into()]),
                },
            ],
            ..Options::default()
        };
        let (required, exempt) = opts.evidence_for(&task, None, None);
        assert_eq!(
            required,
            Some([EvidenceKind::Test].as_slice()),
            "the narrow scope set no requirement, so the wider scope's must stand \
             rather than falling all the way to the whole-plan value"
        );
        assert_eq!(exempt, ["waived".to_string()], "and its own dial applies");
    }

    /// And among scopes of equal narrowness, the **first in the file** wins,
    /// which is the half of the rule a reader would otherwise have to guess
    /// (`A1-183`).
    #[test]
    fn two_equally_narrow_scopes_are_resolved_by_reading_down_the_list() {
        let mut task = t("T-1", Status::Done);
        task.tags = vec!["spike".into(), "research".into()];
        let opts = Options {
            require_min_strength: Some(vec![EvidenceKind::Test]),
            evidence_scopes: vec![
                scope(
                    1,
                    crate::select::Selector {
                        tags: vec!["spike".into()],
                        ..Default::default()
                    },
                    Some(vec![EvidenceKind::Commit]),
                ),
                scope(
                    1,
                    crate::select::Selector {
                        tags: vec!["research".into()],
                        ..Default::default()
                    },
                    Some(vec![EvidenceKind::File]),
                ),
            ],
            ..Options::default()
        };
        let (required, _) = opts.evidence_for(&task, None, None);
        assert_eq!(
            required,
            Some([EvidenceKind::Commit].as_slice()),
            "the later entry won, so the tie is being resolved by iteration order \
             rather than by the documented rule"
        );
    }

    fn t(id: &str, status: Status) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            phase: 0,
            status,
            owner: None,
            effort_days: Some(1.0),
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            evidence_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    fn plan_of(tasks: Vec<Task>) -> Plan {
        Plan {
            tasks: tasks
                .into_iter()
                .map(|task| LoadedTask {
                    task,
                    path: PathBuf::from("plan/tasks/x.md"),
                    fields: crate::load::FieldLines::new(),
                    criteria: crate::load::Criteria::default(),
                })
                .collect(),
            load_diagnostics: Vec::new(),
            unloadable: BTreeMap::new(),
        }
    }

    fn opts() -> Options {
        Options {
            require_effort: true,
            traces_root: None,
            ..Default::default()
        }
    }

    /// The diagnostic with this code, so a test can assert on what it *says*.
    ///
    /// `A1-090`: proving a rule fired is half a test. The message and the
    /// `agent_instruction` are what an adopter and an agent act on, and they
    /// change without any code-only assertion noticing.
    fn pick<'a>(r: &'a Report, code: &str) -> &'a Diagnostic {
        r.diagnostics
            .iter()
            .find(|d| d.code == code)
            .unwrap_or_else(|| panic!("expected {code}, got {:?}", r.diagnostics))
    }

    /// A plan of one `done` task whose body was counted as `criteria` says.
    ///
    /// `criteria` is on `LoadedTask` rather than on `Task` (`A1-224`), so it is
    /// set here rather than reached through the task, which is the whole point:
    /// the model has no field for it and importers have nothing to map.
    fn plan_with_criteria(criteria: crate::load::Criteria) -> Plan {
        let mut task = t("A-1", Status::Done);
        task.evidence = vec![Evidence {
            kind: crate::task::EvidenceKind::Commit,
            reference: "abc1234".to_string(),
            captured: None,
        }];
        let mut plan = plan_of(vec![task]);
        plan.tasks[0].criteria = criteria;
        plan
    }

    #[test]
    fn a_done_task_over_an_unanswered_criterion_says_how_far_it_got() {
        let r = validate(
            &plan_with_criteria(crate::load::Criteria {
                total: 3,
                ticked: 1,
                first_unticked: Some(24),
                ..crate::load::Criteria::default()
            }),
            &opts(),
        );
        let d = pick(&r, code::CRITERIA_UNANSWERED);
        assert_eq!(d.severity, crate::diagnostic::Severity::Warning);
        assert_eq!(
            d.message,
            "is `done` with 1 of 3 criteria in its body ticked"
        );
        // Not "acceptance criteria": nine of this plan's own milestones head
        // that section `## Exit criteria`, and the rule counts boxes without
        // reading a heading, so naming one would name a file it never read.
        assert!(
            !d.message.contains("acceptance"),
            "the message names no heading: {}",
            d.message
        );
        assert_eq!(
            d.location.as_ref().map(|l| l.line),
            Some(24),
            "at the first unticked box, so a reader opens the file at it"
        );
        assert!(
            d.agent_instruction
                .as_deref()
                .is_some_and(|i| i.contains("Nothing ticks them for you")),
            "and an agent is told the one thing it must not do: {:?}",
            d.agent_instruction
        );
    }

    /// `APLAN043`: the scaffold left beside the answer, and the two shapes that
    /// are not it (`A1-299`).
    #[test]
    fn the_scaffold_left_beside_real_criteria_is_reported() {
        let leftover = || crate::load::Criteria {
            total: 3,
            ticked: 0,
            first_unticked: Some(20),
            scaffold_ticked: None,
            scaffold_unticked: Some(24),
            prompts: 1,
            scaffold_prompt: Some(crate::scaffold::criterion_prompt()),
            why_untouched: false,
        };
        // `plan_with_criteria` builds a closed task, and this rule is live-only.
        let mut live = plan_with_criteria(leftover());
        live.tasks[0].task.status = Status::Todo;
        let r = validate(&live, &opts());
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::SCAFFOLD_LEFTOVER)
            .expect("the leftover prompt is reported");
        assert!(d.message.contains("2 criteria of its own"), "{}", d.message);
        assert_eq!(d.location.map(|l| l.line), Some(24));

        // The prompt alone: `add` wrote it and nobody has answered yet, which is
        // `APLAN038`'s at close and not a finding here.
        let mut alone = plan_with_criteria(leftover());
        alone.tasks[0].task.status = Status::Todo;
        alone.tasks[0].criteria.total = 1;
        assert!(
            !validate(&alone, &opts())
                .diagnostics
                .iter()
                .any(|d| d.code == code::SCAFFOLD_LEFTOVER),
            "a task with only the prompt was reported"
        );

        // And a closed task, where the same leftover is `APLAN038`'s or
        // `APLAN039`'s already.
        let mut closed = plan_with_criteria(leftover());
        closed.tasks[0].task.status = Status::Done;
        assert!(
            !validate(&closed, &opts())
                .diagnostics
                .iter()
                .any(|d| d.code == code::SCAFFOLD_LEFTOVER),
            "a closed task was reported twice over"
        );

        // Ticked on a live task: still the scaffold surviving an answer, and
        // `APLAN039` does not reach it because that rule is closed-tasks-only.
        let mut ticked = plan_with_criteria(crate::load::Criteria {
            ticked: 1,
            first_unticked: Some(20),
            scaffold_ticked: Some(24),
            scaffold_unticked: None,
            ..leftover()
        });
        ticked.tasks[0].task.status = Status::InProgress;
        let r = validate(&ticked, &opts());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::SCAFFOLD_LEFTOVER),
            "a ticked prompt on a live task is reported by nothing"
        );
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::SCAFFOLD_TICKED),
            "`APLAN039` reached a live task"
        );
    }

    /// `APLAN050`: a recorded `version` is on a release, and is a version
    /// (`A1-339`, ADR-0038).
    ///
    /// Two ways to get it wrong and one shape that is right, because the field
    /// is a record and a record nobody can read back is worse than none.
    #[test]
    fn a_recorded_version_belongs_on_a_release_and_has_to_be_one() {
        let recorded = |tags: &[&str], version: &str| {
            let mut node = t("M-1", Status::Todo);
            node.tags = tags.iter().map(|g| (*g).to_string()).collect();
            node.version = Some(version.to_string());
            plan_of(vec![node])
        };

        // On a cut: a planning bucket is not a delivery, and the field on one
        // reads as a claim that it shipped.
        let r = validate(&recorded(&["milestone"], "1.2.3"), &opts());
        let d = pick(&r, code::VERSION_RECORD_UNUSABLE);
        assert_eq!(d.severity, crate::diagnostic::Severity::Error);
        assert!(
            d.message.contains("1.2.3") && d.message.contains("release"),
            "{}",
            d.message
        );
        assert!(
            d.agent_instruction
                .as_deref()
                .is_some_and(|i| i.contains("Remove `version`")),
            "{:?}",
            d.agent_instruction
        );

        // On a release, but not a version this tool can read back.
        let r = validate(&recorded(&["milestone", "release"], "next"), &opts());
        let d = pick(&r, code::VERSION_RECORD_UNUSABLE);
        assert!(
            d.message.contains("next") && d.message.contains("read back"),
            "{}",
            d.message
        );

        // The shape it exists to allow.
        assert!(
            !validate(&recorded(&["milestone", "release"], "1.2.3"), &opts())
                .diagnostics
                .iter()
                .any(|d| d.code == code::VERSION_RECORD_UNUSABLE),
            "a release recording a readable version is the point of the field"
        );
        // And a plan that records nothing is untouched: the field does not exist
        // until `close` writes it.
        assert!(
            !validate(&plan_of(vec![t("W-1", Status::Todo)]), &opts())
                .diagnostics
                .iter()
                .any(|d| d.code == code::VERSION_RECORD_UNUSABLE)
        );
    }

    /// `APLAN044`: a closed task claiming less of the version than its level
    /// implies, and the five shapes that are not it (`A1-312`, ADR-0034).
    #[test]
    fn a_closed_task_claiming_under_its_level_is_reported_unless_it_says_why() {
        use crate::task::{VersionFloors, VersionImpact};

        fn plan_claiming(impact: Option<VersionImpact>, reason: Option<&str>) -> Plan {
            let mut task = t("A-1", Status::Done);
            task.evidence = vec![Evidence {
                kind: crate::task::EvidenceKind::Commit,
                reference: "abc1234".to_string(),
                captured: None,
            }];
            task.version_impact = impact;
            task.version_impact_reason = reason.map(str::to_string);
            let mut plan = plan_of(vec![task]);
            plan.tasks[0].fields.insert(
                "version_impact".to_string(),
                crate::diagnostic::Location { line: 9, column: 1 },
            );
            plan
        }
        let on = || Options {
            require_impact: Some(VersionFloors::default()),
            ..opts()
        };

        let r = validate(&plan_claiming(Some(VersionImpact::None), None), &on());
        let d = pick(&r, code::VERSION_IMPACT_UNEXPLAINED);
        assert_eq!(d.severity, crate::diagnostic::Severity::Warning);
        assert_eq!(
            d.message,
            "claims `none` of the released version where its level implies `fix`, and says \
             nowhere why",
            "both numbers are named: which claim, and what it is being held to"
        );
        assert_eq!(
            d.location.map(|l| l.line),
            Some(9),
            "at the claim, which is the line an author edits"
        );
        assert!(
            d.agent_instruction
                .as_deref()
                .is_some_and(|i| i.contains("version_impact_reason")),
            "the instruction names the field that settles it: {:?}",
            d.agent_instruction
        );

        let quiet = |plan: &Plan, o: &Options, why: &str| {
            assert!(
                !validate(plan, o)
                    .diagnostics
                    .iter()
                    .any(|d| d.code == code::VERSION_IMPACT_UNEXPLAINED),
                "{why}"
            );
        };

        // Said why: the override is the author's to make, and this rule only
        // ever asked for the sentence.
        quiet(
            &plan_claiming(
                Some(VersionImpact::None),
                Some("a pilot log, nothing ships"),
            ),
            &on(),
            "a claim with a reason was still reported",
        );
        // Declared nothing, which is the ordinary task and the reason this is
        // adoptable at three hundred of them.
        quiet(
            &plan_claiming(None, None),
            &on(),
            "a task that declared nothing was reported",
        );
        // Claimed more than the floor: nothing here polices generosity.
        quiet(
            &plan_claiming(Some(VersionImpact::Breaking), None),
            &on(),
            "a task claiming more than its level implies was reported",
        );
        // Not asked for, which is the default.
        quiet(
            &plan_claiming(Some(VersionImpact::None), None),
            &opts(),
            "the rule spoke without `version.require_impact`",
        );
        // `dropped` ships nothing, so what it would have done is nobody's
        // question -- and `done` rather than terminal is what the rule reads.
        let mut dropped = plan_claiming(Some(VersionImpact::None), None);
        dropped.tasks[0].task.status = Status::Dropped;
        quiet(&dropped, &on(), "a dropped task was held to a floor");

        // The level is what moves the floor, and a cut's is higher.
        let mut cut = plan_claiming(Some(VersionImpact::Fix), None);
        cut.tasks[0].task.tags = vec![crate::task::MILESTONE_TAG.to_string()];
        cut.tasks[0].task.effort_days = None;
        let r = validate(&cut, &on());
        assert_eq!(
            pick(&r, code::VERSION_IMPACT_UNEXPLAINED).message,
            "claims `fix` of the released version where its level implies `feature`, and says \
             nowhere why",
            "a cut is held to the cut floor, not the task floor"
        );
    }

    #[test]
    fn ticking_the_scaffold_is_reported_and_a_lookalike_is_not() {
        let mut plan = plan_with_criteria(crate::load::Criteria {
            total: 1,
            ticked: 1,
            first_unticked: None,
            scaffold_ticked: Some(21),
            scaffold_prompt: Some(crate::scaffold::criterion_prompt()),
            why_untouched: false,
            scaffold_unticked: None,
            prompts: 0,
        });
        let r = validate(&plan, &opts());
        let d = pick(&r, code::SCAFFOLD_TICKED);
        assert_eq!(d.severity, crate::diagnostic::Severity::Warning);
        assert!(
            d.message.contains(crate::scaffold::criterion_prompt()),
            "the finding quotes the line, so a reader can find it: {}",
            d.message
        );
        assert!(
            !d.message.contains("`## Why`"),
            "and says nothing about a `Why` that was written: {}",
            d.message
        );
        assert_eq!(d.location.as_ref().map(|l| l.line), Some(21));

        // The same task with a real criterion instead: silent. The counting
        // fields are identical, so only `scaffold_ticked` can be driving it.
        plan.tasks[0].criteria.scaffold_ticked = None;
        let r = validate(&plan, &opts());
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::SCAFFOLD_TICKED),
            "{:?}",
            r.diagnostics
        );
    }

    #[test]
    fn an_untouched_why_is_the_second_sentence_and_not_a_second_finding() {
        let r = validate(
            &plan_with_criteria(crate::load::Criteria {
                total: 1,
                ticked: 1,
                first_unticked: None,
                scaffold_ticked: Some(21),
                scaffold_prompt: Some(crate::scaffold::criterion_prompt()),
                why_untouched: true,
                scaffold_unticked: None,
                prompts: 1,
            }),
            &opts(),
        );
        assert_eq!(
            r.diagnostics
                .iter()
                .filter(|d| d.code == code::SCAFFOLD_TICKED)
                .count(),
            1,
            "one finding, not two"
        );
        assert!(
            pick(&r, code::SCAFFOLD_TICKED)
                .message
                .contains("`## Why` is still the prompt too")
        );
    }

    #[test]
    fn a_live_task_may_carry_the_scaffold_it_was_created_with() {
        let mut plan = plan_with_criteria(crate::load::Criteria {
            total: 1,
            ticked: 1,
            first_unticked: None,
            scaffold_ticked: Some(21),
            scaffold_prompt: Some(crate::scaffold::criterion_prompt()),
            why_untouched: true,
            scaffold_unticked: None,
            prompts: 1,
        });
        plan.tasks[0].task.status = Status::InProgress;
        let r = validate(&plan, &opts());
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::SCAFFOLD_TICKED),
            "every task `add` creates carries it, so this must speak only about `done`: {:?}",
            r.diagnostics
        );
    }

    #[test]
    fn a_task_stating_no_criteria_is_not_unanswered() {
        // Silence, not a demand for the convention: two of this plan's closed
        // tasks state none, and `APLAN005` is what asks for evidence.
        let r = validate(
            &plan_with_criteria(crate::load::Criteria::default()),
            &opts(),
        );
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::CRITERIA_UNANSWERED),
            "{:?}",
            r.diagnostics
        );
    }

    #[test]
    fn a_live_task_is_not_asked_to_have_answered_them() {
        let mut plan = plan_with_criteria(crate::load::Criteria {
            total: 2,
            ticked: 0,
            first_unticked: Some(9),
            ..crate::load::Criteria::default()
        });
        plan.tasks[0].task.status = Status::InProgress;
        let r = validate(&plan, &opts());
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::CRITERIA_UNANSWERED),
            "an unticked criterion on live work is the normal state of a plan: {:?}",
            r.diagnostics
        );
    }

    /// A task carrying `paths`, for the duplicate rule.
    fn plan_with_paths(paths: &[&str]) -> Plan {
        let mut task = t("A-1", Status::Todo);
        task.paths = paths.iter().map(|p| (*p).to_string()).collect();
        plan_of(vec![task])
    }

    #[test]
    fn the_same_path_twice_is_reported_and_once_is_not() {
        let r = validate(&plan_with_paths(&["src/**", "src/**", "ci/**"]), &opts());
        let d = pick(&r, code::DUPLICATE_PATH);
        assert_eq!(d.severity, crate::diagnostic::Severity::Warning);
        assert!(
            d.message.contains("`src/**` 2 times"),
            "names the entry and how many: {}",
            d.message
        );
        assert!(
            !d.message.contains("ci/**"),
            "and only the one that repeats: {}",
            d.message
        );

        let clean = validate(&plan_with_paths(&["src/**", "ci/**"]), &opts());
        assert!(
            !clean
                .diagnostics
                .iter()
                .any(|d| d.code == code::DUPLICATE_PATH),
            "{:?}",
            clean.diagnostics
        );
    }

    #[test]
    fn two_spellings_of_one_path_are_one_entry() {
        // `A1-254` made these the same path to a citation and to a glob, so a
        // rule that called them different would be the one disagreeing.
        let r = validate(&plan_with_paths(&["src/**", "./src/**"]), &opts());
        let d = pick(&r, code::DUPLICATE_PATH);
        assert!(
            d.message.contains("`./src/**`") && d.message.contains("`src/**`"),
            "both writings are named, since the author has to find one of them: {}",
            d.message
        );
    }

    #[test]
    fn a_task_with_no_paths_or_one_is_silent() {
        for paths in [&[][..], &["src/**"][..]] {
            let r = validate(&plan_with_paths(paths), &opts());
            assert!(
                !r.diagnostics.iter().any(|d| d.code == code::DUPLICATE_PATH),
                "{paths:?} reported a duplicate: {:?}",
                r.diagnostics
            );
        }
    }

    /// The shapes `A1-254`'s normalisation folds together, and the one it does
    /// not (`A1-267`'s audit, which found the message wrong for two of these).
    #[test]
    fn the_spellings_that_are_one_entry_and_the_one_that_is_not() {
        // A trailing slash, and `..` walked back: both reduce to the same path,
        // and the explanation must fit both -- it named only the `./x` case.
        for pair in [["src/", "src"], ["src/../src/**", "src/**"]] {
            let r = validate(&plan_with_paths(&pair), &opts());
            let d = pick(&r, code::DUPLICATE_PATH);
            assert!(
                d.message.contains("`.` and `..` segments are resolved"),
                "{pair:?} needs the reason that covers both shapes: {}",
                d.message
            );
        }

        // A trailing *space* is not the same entry: `src/** ` is a glob that
        // matches a path ending in a space, so calling it a duplicate would be
        // this rule guessing that the author did not mean what they typed.
        let spaced = validate(&plan_with_paths(&["src/** ", "src/**"]), &opts());
        assert!(
            !spaced
                .diagnostics
                .iter()
                .any(|d| d.code == code::DUPLICATE_PATH),
            "{:?}",
            spaced.diagnostics
        );
    }

    /// Counting, and an entry with nothing in it.
    #[test]
    fn it_counts_the_copies_and_names_a_blank_in_words() {
        let three = validate(&plan_with_paths(&["src/**", "src/**", "src/**"]), &opts());
        assert!(
            pick(&three, code::DUPLICATE_PATH)
                .message
                .contains("3 times")
        );

        // Two duplicated entries in one task are two findings, not one.
        let two = validate(
            &plan_with_paths(&["src/**", "src/**", "ci/**", "ci/**"]),
            &opts(),
        );
        assert_eq!(
            two.diagnostics
                .iter()
                .filter(|d| d.code == code::DUPLICATE_PATH)
                .count(),
            2
        );

        // An entry nobody filled in renders as empty backticks otherwise, which
        // reads as a defect in the message rather than a fact about the file.
        let blank = validate(&plan_with_paths(&["", ""]), &opts());
        let d = pick(&blank, code::DUPLICATE_PATH);
        assert!(
            d.message.contains("an empty entry 2 times") && !d.message.contains("`` "),
            "{}",
            d.message
        );
    }

    /// `contains` refuses the three ways it can be unusable (`A1-333`,
    /// ADR-0042).
    ///
    /// Each is a different mistake with a different remedy, which is why
    /// one rule says three things rather than one sentence covering all.
    #[test]
    fn a_contains_that_cannot_be_walked_is_refused() {
        for (child, needle, what) in [
            ("M-NOPE", "names no task in this plan", "unknown id"),
            (
                "W-1",
                "names a task rather than a milestone",
                "not a checkpoint",
            ),
        ] {
            let mut m = t("M-1", Status::Todo);
            m.tags = vec![crate::task::MILESTONE_TAG.to_string()];
            m.effort_days = None;
            m.contains = vec![child.to_string()];
            let plan = plan_of(vec![m, t("W-1", Status::Todo)]);
            let r = validate(&plan, &opts());
            let d = pick(&r, code::CONTAINS_UNUSABLE);
            assert!(d.message.contains(needle), "{what}: {}", d.message);
        }
    }

    /// The three shapes the audit of `A1-333` found unrefused, each of which
    /// passed `check` cleanly when `contains` shipped.
    ///
    /// One test over a table because they are one question -- can this
    /// containment be walked and trusted -- and what matters is that each
    /// *says* which it is.
    #[test]
    fn the_containments_that_passed_when_they_should_not_have() {
        let cases: &[(&str, &[&str], &[&str], &str)] = &[
            // `contains_cycle` skips a self-edge, copied from
            // `ratification_cycle` where `APLAN006` covers it. It does not
            // cover this one: a `contains` self-edge is not a `blocked_by` edge.
            ("self", &["M-1", "M-2"], &["M-2"], "names itself"),
            // Containment that does not wait: the checkpoint sits at `done`
            // over an open child, and the rollup prints `[done] 0/1 done`.
            ("unwaited", &["M-2"], &[], "is not also in `blocked_by`"),
        ];
        for (what, contains, blocked, needle) in cases {
            let mut m = t("M-1", Status::Todo);
            m.tags = vec![crate::task::MILESTONE_TAG.to_string()];
            m.effort_days = None;
            m.contains = contains.iter().map(|s| (*s).to_string()).collect();
            m.blocked_by = blocked.iter().map(|s| (*s).to_string()).collect();
            let mut child = t("M-2", Status::Todo);
            child.tags = vec![crate::task::MILESTONE_TAG.to_string()];
            child.effort_days = None;
            child.blocked_by = vec!["W-1".to_string()];
            let plan = plan_of(vec![m, child, t("W-1", Status::Todo)]);
            let r = validate(&plan, &opts());
            let d = pick(&r, code::CONTAINS_UNUSABLE);
            assert!(d.message.contains(needle), "{what}: {}", d.message);
        }
    }

    /// A checkpoint that both contains and holds work is the shape that makes
    /// the arithmetic wrong rather than merely untidy (`A1-333`, audit).
    ///
    /// Measured before the rule existed: a plan with two tasks and three days
    /// reported `1 task(s), 1.0d effort`, because the header skips a borrowing
    /// row whole and the work this one owned alone went with it. The same
    /// sentence `A1-146` already made about a release.
    #[test]
    fn a_checkpoint_that_also_holds_work_is_reported() {
        let mut top = t("M-1", Status::Todo);
        top.tags = vec![crate::task::MILESTONE_TAG.to_string()];
        top.effort_days = None;
        top.contains = vec!["M-2".to_string()];
        top.blocked_by = vec!["M-2".to_string(), "W-2".to_string()];
        let mut sub = t("M-2", Status::Todo);
        sub.tags = vec![crate::task::MILESTONE_TAG.to_string()];
        sub.effort_days = None;
        sub.blocked_by = vec!["W-1".to_string()];
        let plan = plan_of(vec![
            top,
            sub,
            t("W-1", Status::Todo),
            t("W-2", Status::Todo),
        ]);
        let r = validate(&plan, &opts());
        let d = pick(&r, code::MILESTONE_HYGIENE);
        assert!(d.message.contains("holds `W-2` directly"), "{}", d.message);
        assert!(
            d.message.contains("holds the checkpoints beneath it"),
            "the message does not say which kind of checkpoint it is: {}",
            d.message
        );
    }

    /// A checkpoint containing a real checkpoint is the endorsed shape and
    /// must stay silent, which is the half a stricter rule would break.
    #[test]
    fn a_checkpoint_containing_a_checkpoint_is_silent() {
        let mut outer = t("M-1", Status::Todo);
        outer.tags = vec![crate::task::MILESTONE_TAG.to_string()];
        outer.effort_days = None;
        outer.contains = vec!["M-2".to_string()];
        outer.blocked_by = vec!["M-2".to_string()];
        let mut inner = t("M-2", Status::Todo);
        inner.tags = vec![crate::task::MILESTONE_TAG.to_string()];
        inner.effort_days = None;
        inner.blocked_by = vec!["W-1".to_string()];
        let plan = plan_of(vec![outer, inner, t("W-1", Status::Todo)]);
        let codes: Vec<String> = validate(&plan, &opts())
            .diagnostics
            .iter()
            .map(|d| d.code.to_string())
            .collect();
        assert!(
            !codes.contains(&code::CONTAINS_UNUSABLE.to_string()),
            "the endorsed shape was refused: {codes:?}"
        );
    }

    /// A work task may not contain: that is ADR-0014's second grouping
    /// axis arriving by a new door.
    #[test]
    fn a_work_task_may_not_contain() {
        let mut w = t("W-1", Status::Todo);
        w.contains = vec!["M-2".to_string()];
        let mut m = t("M-2", Status::Todo);
        m.tags = vec![crate::task::MILESTONE_TAG.to_string()];
        m.effort_days = None;
        let plan = plan_of(vec![w, m]);
        let r = validate(&plan, &opts());
        let d = pick(&r, code::CONTAINS_UNUSABLE);
        assert!(
            d.message.contains("not tagged `milestone`"),
            "{}",
            d.message
        );
    }

    /// `APLAN041`, the second copy of the policy going stale (`A1-273`).
    ///
    /// The block `export-agents-md` writes is what an agent reads before
    /// touching the plan, and `p00002`'s said `commit` was acceptable evidence
    /// for a day after `require_min_strength` had dropped it. Each case here is
    /// a state a real file can be in, and the silent ones matter as much as the
    /// loud one: this rule reads a document the plan does not own, so getting
    /// its scope wrong means nagging repositories that never asked.
    mod agents_block {
        use super::*;

        /// Write `body` between the markers, with prose either side, and return
        /// the options pointing at it. The directory is returned too, because
        /// dropping it deletes the file before the rule can read it.
        fn repo(body: &str, current: &str) -> (tempfile::TempDir, Options) {
            let dir = tempfile::Builder::new()
                .tempdir_in(crate::fixture::fixture_root())
                .expect("tempdir");
            let path = dir.path().join("AGENTS.md");
            std::fs::write(&path, body).expect("write");
            let o = Options {
                agents_block: Some(AgentsBlock {
                    path,
                    current: current.to_string(),
                }),
                ..opts()
            };
            (dir, o)
        }

        fn fenced(body: &str) -> String {
            format!(
                "# A repository\n\nits own prose\n\n{}\n{body}\n{}\n\nmore prose\n",
                crate::contract::BEGIN,
                crate::contract::END
            )
        }

        fn codes(r: &Report) -> Vec<String> {
            r.diagnostics.iter().map(|d| d.code.to_string()).collect()
        }

        /// The measured case, in the shape it was found in.
        #[test]
        fn a_block_that_no_longer_matches_the_policy_is_reported() {
            let (_d, o) = repo(
                &fenced("- A `done` task must cite `commit`, `file` or `test` evidence."),
                "- A `done` task must cite `file` or `test` evidence.",
            );
            let r = validate(&plan_of(vec![]), &o);
            let d = pick(&r, code::STALE_AGENT_CONTRACT);
            assert!(
                d.message.contains("what an agent reads"),
                "says why it matters: {}",
                d.message
            );
            // The remedy is the command, with the file it was given: the
            // caller decides how that path is spelled, and the CLI hands over
            // one relative to the invocation so the line can be pasted.
            let fix = d.agent_instruction.clone().unwrap_or_default();
            assert!(
                fix.starts_with("Run `aplan export-agents-md --write ")
                    && fix.ends_with("AGENTS.md`."),
                "not a command a reader can paste: {fix}"
            );
        }

        /// Two builds, one policy: reported, and reported as the build
        /// (`A1-325`, ADR-0037).
        ///
        /// Proves the named behaviour `generated.names_the_build_that_wrote_it`.
        ///
        /// The pilot's case. Blaming the policy here sends the reader looking
        /// for a policy change that did not happen, which is the same wasted
        /// hour the thirty-line diff cost in the first place.
        #[test]
        fn a_block_written_by_another_build_is_reported_as_a_build_difference() {
            let rule = "- A `done` task must cite `file` or `test` evidence.";
            let (_d, o) = repo(
                &fenced(&format!(
                    "{}0.2.0, surface aaaaaaaaaaaa -->\n{rule}",
                    crate::contract::WRITTEN_BY_PREFIX
                )),
                &format!(
                    "{}0.4.0, surface bbbbbbbbbbbb -->\n{rule}",
                    crate::contract::WRITTEN_BY_PREFIX
                ),
            );
            let r = validate(&plan_of(vec![]), &o);
            let d = pick(&r, code::STALE_AGENT_CONTRACT);
            assert!(
                d.message.contains("names a different build as its author"),
                "reported as a policy change: {}",
                d.message
            );
            assert!(
                !d.message
                    .contains("is not what this repository's policy produces now"),
                "still blaming the policy: {}",
                d.message
            );
            // The remedy is not "regenerate", which is what the reader would
            // have done anyway and would leave the two builds disagreeing
            // again on the next run.
            let fix = d.agent_instruction.clone().unwrap_or_default();
            assert!(
                fix.contains("the build this repository pins"),
                "the fix does not say which build to use: {fix}"
            );
            assert!(
                fix.contains("advisory"),
                "no way out for a repository that runs mixed builds on purpose: {fix}"
            );
        }

        /// The policy *and* the build differing is still a policy report: the
        /// policy is the one the reader has to act on, and the build is noise
        /// beside it.
        #[test]
        fn a_block_differing_in_policy_and_build_is_reported_as_the_policy() {
            let (_d, o) = repo(
                &fenced(&format!(
                    "{}0.2.0, surface aaaaaaaaaaaa -->\n- an old rule",
                    crate::contract::WRITTEN_BY_PREFIX
                )),
                &format!(
                    "{}0.4.0, surface bbbbbbbbbbbb -->\n- a new rule",
                    crate::contract::WRITTEN_BY_PREFIX
                ),
            );
            let r = validate(&plan_of(vec![]), &o);
            let d = pick(&r, code::STALE_AGENT_CONTRACT);
            assert!(
                d.message.contains("what an agent reads"),
                "a real policy drift was reported as a build difference: {}",
                d.message
            );
        }

        /// Same policy, same build: the provenance line is not itself a
        /// finding. Guards the obvious mistake in the branch above, which is to
        /// strip the line from both sides and then report on what is left.
        #[test]
        fn a_block_with_a_matching_provenance_line_is_silent() {
            let body = format!(
                "{}0.4.0, surface bbbbbbbbbbbb -->\n- one rule",
                crate::contract::WRITTEN_BY_PREFIX
            );
            let (_d, o) = repo(&fenced(&body), &body);
            assert!(
                codes(&validate(&plan_of(vec![]), &o)).is_empty(),
                "reported a block that matches in every way"
            );
        }

        /// Whitespace around the block is the file's, not the contract's.
        #[test]
        fn a_matching_block_is_silent_through_the_padding() {
            let (_d, o) = repo(&fenced("\n  - one rule\n\n"), "- one rule");
            assert!(
                codes(&validate(&plan_of(vec![]), &o)).is_empty(),
                "reported a newline"
            );
        }

        /// The markers are the repository asking for the block to stay current.
        /// Without them it never asked, and this rule has no standing.
        #[test]
        fn a_file_without_the_markers_is_silent() {
            let (_d, o) = repo("# A repository\n\nhand-written prose only\n", "- one rule");
            assert!(codes(&validate(&plan_of(vec![]), &o)).is_empty());
        }

        /// An adopter who has never run `export-agents-md` is not being nagged
        /// about a feature they did not ask for.
        #[test]
        fn a_file_that_is_not_there_is_silent() {
            let dir = tempfile::Builder::new()
                .tempdir_in(crate::fixture::fixture_root())
                .expect("tempdir");
            let o = Options {
                agents_block: Some(AgentsBlock {
                    path: dir.path().join("AGENTS.md"),
                    current: "- one rule".into(),
                }),
                ..opts()
            };
            assert!(codes(&validate(&plan_of(vec![]), &o)).is_empty());
        }

        /// And a caller that cannot render a block turns the rule off entirely,
        /// rather than comparing against nothing and calling every file stale.
        ///
        /// Structural rather than conditional, and said here because a mutation
        /// pass will not kill this one: `None` carries no path, so there is no
        /// file to read and no block to compare, whatever the code does next.
        /// It pins the meaning of the field for a caller reading the type.
        #[test]
        fn no_block_supplied_is_the_rule_switched_off() {
            let dir = tempfile::Builder::new()
                .tempdir_in(crate::fixture::fixture_root())
                .expect("tempdir");
            std::fs::write(dir.path().join("AGENTS.md"), fenced("- stale")).expect("write");
            let o = Options {
                agents_block: None,
                ..opts()
            };
            assert!(codes(&validate(&plan_of(vec![]), &o)).is_empty());
        }

        /// **The rule cannot be switched off by hand without that being the
        /// visible thing that happened.** Deleting one comment line is the
        /// quietest edit that would silence a marker-scoped rule, so a
        /// half-pair is a finding.
        #[test]
        fn deleting_one_marker_is_a_finding_and_not_a_silence() {
            let half = format!(
                "prose\n\n{}\n- a rule\n\nmore prose\n",
                crate::contract::BEGIN
            );
            let (_d, o) = repo(&half, "- a rule");
            let r = validate(&plan_of(vec![]), &o);
            let d = pick(&r, code::STALE_AGENT_CONTRACT);
            assert!(
                d.message.contains("1 `aplan:begin` and 0 `aplan:end`"),
                "names what it found: {}",
                d.message
            );
            assert!(
                d.agent_instruction
                    .as_deref()
                    .is_some_and(|f| f.contains("Leave one")),
                "the remedy repairs the markers first: {:?}",
                d.agent_instruction
            );
        }

        /// Two blocks is the case the writer refuses: one would be updated and
        /// the other left telling agents the old policy.
        #[test]
        fn two_blocks_are_reported_even_when_one_of_them_matches() {
            let both = format!("{}\n{}", fenced("- a rule"), fenced("- an old rule"));
            let (_d, o) = repo(&both, "- a rule");
            let r = validate(&plan_of(vec![]), &o);
            let d = pick(&r, code::STALE_AGENT_CONTRACT);
            assert!(d.message.contains("2 `aplan:begin`"), "{}", d.message);
        }
    }

    /// `APLAN042`, the revisit hand-off (`A1-264`).
    ///
    /// The field says *when to look again*, never that the wait is over, so the
    /// rule's whole job is to keep it usable and keep it where something reads
    /// it.
    mod revisit {
        use super::*;

        fn task_with(status: Status, revisit: Option<&str>) -> Plan {
            let mut t = t("A-1", status);
            t.revisit_at = revisit.map(str::to_string);
            plan_of(vec![t])
        }

        #[test]
        fn an_instant_nothing_can_parse_is_reported() {
            let r = validate(&task_with(Status::Blocked, Some("next tuesday")), &opts());
            let d = pick(&r, code::REVISIT_UNUSABLE);
            assert!(
                d.message.contains("not an RFC 3339 instant"),
                "{}",
                d.message
            );
            assert!(
                d.agent_instruction
                    .as_deref()
                    .is_some_and(|f| f.contains("YYYY-MM-DDTHH:MM:SSZ")),
                "the remedy shows the shape: {:?}",
                d.agent_instruction
            );
        }

        /// A date is not an instant: a scheduler has to know which moment.
        #[test]
        fn a_date_alone_is_not_an_instant() {
            let r = validate(&task_with(Status::Blocked, Some("2026-09-10")), &opts());
            assert_eq!(
                pick(&r, code::REVISIT_UNUSABLE).code,
                code::REVISIT_UNUSABLE
            );
        }

        /// `ready` already offers a `todo` task, and `due` reports parked work:
        /// a revisit anywhere else is a note to nobody.
        #[test]
        fn a_revisit_on_a_task_that_is_not_parked_is_reported() {
            for status in [Status::Todo, Status::InProgress, Status::Done] {
                let r = validate(&task_with(status, Some("2026-09-10T05:00:00Z")), &opts());
                let d = pick(&r, code::REVISIT_UNUSABLE);
                assert!(
                    d.message.contains("nothing reads it here"),
                    "{status}: {}",
                    d.message
                );
            }
        }

        #[test]
        fn a_real_instant_on_a_parked_task_is_silent() {
            for at in [
                "2026-09-10T05:00:00Z",
                "2026-09-10T05:00:00+02:00",
                "2026-09-10T05:00:00-05:30",
            ] {
                let r = validate(&task_with(Status::Blocked, Some(at)), &opts());
                assert!(
                    !r.diagnostics
                        .iter()
                        .any(|d| d.code == code::REVISIT_UNUSABLE),
                    "{at}: {:?}",
                    r.diagnostics
                );
            }
        }

        #[test]
        fn no_revisit_is_silent_in_every_status() {
            for status in [Status::Todo, Status::Blocked, Status::Done] {
                let r = validate(&task_with(status, None), &opts());
                assert!(
                    !r.diagnostics
                        .iter()
                        .any(|d| d.code == code::REVISIT_UNUSABLE),
                    "{status}: {:?}",
                    r.diagnostics
                );
            }
        }

        /// The offset is removed to reach UTC, and a leap second is refused: a
        /// scheduler does not run at 23:59:60 (`A1-264`).
        #[test]
        fn the_parser_reads_offsets_and_refuses_what_is_not_an_instant() {
            assert_eq!(rfc3339_to_epoch("1970-01-01T00:00:00Z"), Some(0));
            assert_eq!(rfc3339_to_epoch("1970-01-01T02:00:00+02:00"), Some(0));
            assert_eq!(rfc3339_to_epoch("1969-12-31T22:00:00-02:00"), Some(0));
            for bad in [
                "1970-01-01",
                "1970-01-01T00:00:00",
                "1970-01-01 00:00:00Z",
                "1970-01-01T00:00:60Z",
                "1970-01-01T24:00:00Z",
                "1970-01-01T00:00:00.500Z",
                "1970-01-01T00:00:00+2:00",
                "1970-13-01T00:00:00Z",
            ] {
                assert_eq!(rfc3339_to_epoch(bad), None, "accepted `{bad}`");
            }
        }
    }

    /// How the `paths` rules divide one badly-edited list between them
    /// (`A1-267`'s audit, which found all three of these).
    #[test]
    fn a_broken_entry_written_twice_is_one_finding_from_each_rule() {
        let r = validate(&plan_with_paths(&["src/[", "src/["]), &opts());

        // `APLAN032` once, not once per copy: one typo is one sentence.
        assert_eq!(
            r.diagnostics
                .iter()
                .filter(|d| d.code == code::BAD_PATH_GLOB)
                .count(),
            1,
            "{:?}",
            r.diagnostics
        );
        // And `APLAN040` says nothing: an entry that names no files is the
        // other rule's, and "you wrote it twice" is not the useful thing to say
        // about `src/[`.
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::DUPLICATE_PATH),
            "{:?}",
            r.diagnostics
        );
    }

    /// Three entries written two ways is not the same fact as two, and the
    /// two-spelling branch said only *which* and not *how many*.
    #[test]
    fn the_count_survives_a_second_spelling() {
        let r = validate(&plan_with_paths(&["src/**", "./src/**", "src/**"]), &opts());
        let d = pick(&r, code::DUPLICATE_PATH);
        assert!(
            d.message.contains("3 entries"),
            "a reader deleting one copy would still have two: {}",
            d.message
        );
    }

    /// Containment is not duplication, and is deliberately out of scope: a glob
    /// that *subsumes* another is a much larger claim than two writings of one
    /// entry, and a rule making it would be guessing at intent.
    #[test]
    fn a_glob_that_subsumes_another_is_not_a_duplicate() {
        let r = validate(&plan_with_paths(&["src/**", "src/a.rs"]), &opts());
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::DUPLICATE_PATH),
            "{:?}",
            r.diagnostics
        );
    }

    #[test]
    fn done_without_evidence_is_an_error() {
        let p = plan_of(vec![t("A", Status::Done)]);
        let r = validate(&p, &opts());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::DONE_WITHOUT_EVIDENCE)
        );
        let d = pick(&r, code::DONE_WITHOUT_EVIDENCE);
        assert!(
            d.message.contains("`status: done`") && d.message.contains("`evidence`"),
            "the message must name the claim and the field that would back it: {}",
            d.message
        );
        assert!(
            d.agent_instruction
                .as_deref()
                .is_some_and(|i| i.contains("commit") && i.contains("note")),
            "the fix must enumerate what would satisfy it, since the whole rule is \
             about what counts as proof: {:?}",
            d.agent_instruction
        );
        assert!(!r.is_clean());
    }

    #[test]
    fn done_with_evidence_is_clean() {
        let mut task = t("A", Status::Done);
        task.evidence = vec![Evidence {
            kind: EvidenceKind::Commit,
            reference: "d213c9a".into(),
            captured: None,
        }];
        let r = validate(&plan_of(vec![task]), &opts());
        assert!(r.is_clean(), "{:?}", r.diagnostics);
    }

    #[test]
    fn undated_note_is_an_error() {
        let mut task = t("A", Status::Done);
        task.evidence = vec![Evidence {
            kind: EvidenceKind::Note,
            reference: "spoke to the Division".into(),
            captured: None,
        }];
        let r = validate(&plan_of(vec![task]), &opts());
        assert!(r.diagnostics.iter().any(|d| d.code == code::UNDATED_NOTE));
    }

    /// APLAN017: the configured minimum evidence strength.
    fn note_ev() -> Evidence {
        Evidence {
            kind: EvidenceKind::Note,
            reference: "phone call".into(),
            captured: Some("2026-07-01".into()),
        }
    }

    fn commit_ev() -> Evidence {
        Evidence {
            kind: EvidenceKind::Commit,
            reference: "abc1234".into(),
            captured: None,
        }
    }

    fn strength(kinds: &[EvidenceKind], exempt: &[&str]) -> Options {
        Options {
            require_min_strength: Some(kinds.to_vec()),
            exempt_tags: exempt.iter().map(|s| (*s).to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn weak_evidence_fires_when_only_a_note_backs_a_done_task() {
        let mut task = t("A", Status::Done);
        task.evidence = vec![note_ev()];
        let o = strength(
            &[EvidenceKind::Commit, EvidenceKind::Test, EvidenceKind::File],
            &[],
        );
        let r = validate(&plan_of(vec![task]), &o);
        assert!(r.diagnostics.iter().any(|d| d.code == code::WEAK_EVIDENCE));
    }

    #[test]
    fn weak_evidence_is_silent_with_one_strong_piece() {
        let mut task = t("A", Status::Done);
        task.evidence = vec![note_ev(), commit_ev()]; // one weak, one strong
        let o = strength(
            &[EvidenceKind::Commit, EvidenceKind::Test, EvidenceKind::File],
            &[],
        );
        let r = validate(&plan_of(vec![task]), &o);
        assert!(!r.diagnostics.iter().any(|d| d.code == code::WEAK_EVIDENCE));
    }

    #[test]
    fn weak_evidence_exempts_a_tagged_task() {
        let mut task = t("A", Status::Done);
        task.tags = vec!["phone".into()];
        task.evidence = vec![note_ev()];
        let o = strength(&[EvidenceKind::Commit], &["phone"]);
        let r = validate(&plan_of(vec![task]), &o);
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::WEAK_EVIDENCE),
            "an exempt tag must silence APLAN017: {:?}",
            r.diagnostics
        );
    }

    #[test]
    fn weak_evidence_is_off_by_default_and_never_doubles_with_005() {
        // Off by default: a note-only `done` task is fine.
        let mut note_only = t("A", Status::Done);
        note_only.evidence = vec![note_ev()];
        let r = validate(&plan_of(vec![note_only]), &opts());
        assert!(!r.diagnostics.iter().any(|d| d.code == code::WEAK_EVIDENCE));

        // With the rule on, empty evidence is APLAN005's business, not 017's: a
        // `done` task with no evidence must not fire both.
        let empty = t("B", Status::Done);
        let o = strength(&[EvidenceKind::Commit], &[]);
        let r = validate(&plan_of(vec![empty]), &o);
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::DONE_WITHOUT_EVIDENCE)
        );
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::WEAK_EVIDENCE),
            "empty evidence must not also fire APLAN017"
        );
    }

    /// A `done` task whose only evidence has a blank `ref` is not proven: the
    /// keystone must fire, or `status: done` plus `ref: ""` sails through `check`.
    #[test]
    fn done_with_only_blank_ref_evidence_is_an_error() {
        for blank in ["", "   ", "\t"] {
            let mut task = t("A", Status::Done);
            task.evidence = vec![Evidence {
                kind: EvidenceKind::Commit,
                reference: blank.to_string(),
                captured: None,
            }];
            let r = validate(&plan_of(vec![task]), &opts());
            assert!(
                r.diagnostics
                    .iter()
                    .any(|d| d.code == code::DONE_WITHOUT_EVIDENCE),
                "a blank ref ({blank:?}) must not satisfy `done` needs evidence"
            );
        }
    }

    /// A `ref` made only of invisible characters (a zero-width space) cites
    /// nothing, so it must not satisfy the keystone. `str::trim` alone misses it.
    #[test]
    fn done_with_only_an_invisible_ref_is_an_error() {
        for invisible in [
            "\u{200B}",
            "\u{FEFF}",
            "\u{200B}\u{200D}\u{2060}",
            " \u{200B} ",
        ] {
            let mut task = t("A", Status::Done);
            task.evidence = vec![Evidence {
                kind: EvidenceKind::Commit,
                reference: invisible.to_string(),
                captured: None,
            }];
            let r = validate(&plan_of(vec![task]), &opts());
            assert!(
                r.diagnostics
                    .iter()
                    .any(|d| d.code == code::DONE_WITHOUT_EVIDENCE),
                "an invisible ref ({invisible:?}) must not satisfy `done` needs evidence"
            );
        }
        // A real ref with an incidental invisible char is still substantive.
        let mut ok = t("B", Status::Done);
        ok.evidence = vec![Evidence {
            kind: EvidenceKind::Commit,
            reference: "abc1234".into(),
            captured: None,
        }];
        assert!(validate(&plan_of(vec![ok]), &opts()).is_clean());
    }

    /// A blank-`ref` note of an accepted kind must not satisfy APLAN017 either:
    /// strength is about a real citation, not just the right `kind`.
    #[test]
    fn weak_evidence_ignores_a_blank_ref_of_an_accepted_kind() {
        let mut task = t("A", Status::Done);
        task.evidence = vec![
            note_ev(), // substantive, so APLAN005 stays silent
            Evidence {
                kind: EvidenceKind::Commit,
                reference: "  ".into(),
                captured: None,
            },
        ];
        let o = strength(&[EvidenceKind::Commit], &[]);
        let r = validate(&plan_of(vec![task]), &o);
        assert!(
            r.diagnostics.iter().any(|d| d.code == code::WEAK_EVIDENCE),
            "a blank-ref commit must not count as strong evidence: {:?}",
            r.diagnostics
        );
        let d = pick(&r, code::WEAK_EVIDENCE);
        assert!(
            d.message.contains("commit"),
            "the message must name the kinds this repository configured, or the \
             reader cannot tell what would satisfy it: {}",
            d.message
        );
    }

    #[test]
    fn a_note_with_a_nonsense_captured_date_is_an_error() {
        for bad in [
            "someday",
            "2026-13-40",
            "2026-02-30",
            "07/18/2026",
            "2026-7-1",
        ] {
            let mut task = t("A", Status::Done);
            task.evidence = vec![Evidence {
                kind: EvidenceKind::Note,
                reference: "spoke to the Division".into(),
                captured: Some(bad.to_string()),
            }];
            let r = validate(&plan_of(vec![task]), &opts());
            assert!(
                r.diagnostics.iter().any(|d| d.code == code::UNDATED_NOTE),
                "captured {bad:?} is not an ISO date and must fire APLAN010"
            );
            let d = pick(&r, code::UNDATED_NOTE);
            assert!(
                d.message.contains(bad),
                "the message must quote the value it rejected: told only that a \
                 date is invalid, an author with several notes cannot tell which \
                 one to fix: {}",
                d.message
            );
        }
        // A real date is clean.
        assert!(is_iso_date("2026-07-18"));
        assert!(is_iso_date("2024-02-29")); // leap day
        assert!(!is_iso_date("2026-02-29")); // 2026 is not a leap year
    }

    #[test]
    fn a_capture_date_on_machine_resolved_evidence_is_an_error() {
        for status in Status::ALL {
            for milestone in [false, true] {
                for kind in [
                    EvidenceKind::Commit,
                    EvidenceKind::Url,
                    EvidenceKind::File,
                    EvidenceKind::Test,
                ] {
                    let mut task = t("A", *status);
                    if milestone {
                        task.tags.push("milestone".into());
                    }
                    task.evidence = vec![Evidence {
                        kind,
                        reference: "proof".into(),
                        captured: Some("2026-10-10".into()),
                    }];
                    let report = validate(&plan_of(vec![task]), &opts());
                    let diagnostic = pick(&report, code::INAPPLICABLE_CAPTURE);
                    assert!(
                        diagnostic.message.contains(kind.as_str())
                            && diagnostic.message.contains("2026-10-10")
                            && diagnostic.message.contains("provenance"),
                        "the finding must identify the kind, value and false claim for status \
                         `{status}` (milestone={milestone}): {}",
                        diagnostic.message
                    );
                    assert!(
                        diagnostic
                            .agent_instruction
                            .as_deref()
                            .is_some_and(|instruction| instruction.contains("Remove `captured`")
                                && instruction.contains(kind.as_str())
                                && instruction.contains("`note`")),
                        "the remedy must distinguish this kind from the one that accepts the \
                         field for status `{status}` (milestone={milestone}): {:?}",
                        diagnostic.agent_instruction
                    );
                }
            }
        }
    }

    #[test]
    fn an_authored_note_keeps_its_existing_capture_contract() {
        let mut task = t("A", Status::Todo);
        task.evidence = vec![Evidence {
            kind: EvidenceKind::Note,
            reference: "reviewed by the operator".into(),
            captured: Some("2026-10-10".into()),
        }];
        let report = validate(&plan_of(vec![task]), &opts());
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.code == code::UNDATED_NOTE || d.code == code::INAPPLICABLE_CAPTURE),
            "the existing valid-note shape changed: {:?}",
            report.diagnostics
        );
    }

    /// Regression: a trace listed twice must produce one diagnostic, not two
    /// identical PR annotations (matching how `blocked_by` is deduplicated).
    #[test]
    fn a_repeated_trace_is_reported_once() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let mut a = t("A", Status::Todo);
        a.traces = vec!["NOPE.md#x".into(), "NOPE.md#x".into()];
        let o = Options {
            require_effort: false,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let r = validate(&plan_of(vec![a]), &o);
        let n = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::DANGLING_TRACE)
            .count();
        assert_eq!(n, 1, "one trace, one diagnostic: {:?}", r.diagnostics);
    }

    #[test]
    fn unknown_dependency_is_an_error() {
        let mut a = t("A", Status::Todo);
        a.blocked_by = vec!["GHOST".into()];
        let r = validate(&plan_of(vec![a]), &opts());
        assert!(r.diagnostics.iter().any(|d| d.code == code::UNKNOWN_DEP));
    }

    #[test]
    fn an_edge_into_a_file_that_did_not_load_is_not_a_second_error() {
        // The fourth pilot's shape: a file that fails to parse is absent from
        // the graph, so every edge into it looks dangling. It is not, and
        // reporting it buries the real fault under a cascade proportional to how
        // well connected the plan is (`A1-072`).
        let mut a = t("A", Status::Todo);
        a.blocked_by = vec!["BROKEN".into(), "GHOST".into()];
        let mut plan = plan_of(vec![a]);
        plan.unloadable
            .insert("BROKEN".into(), PathBuf::from("plan/tasks/BROKEN.md"));

        let r = validate(&plan, &opts());
        let unknown: Vec<&str> = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::UNKNOWN_DEP)
            .map(|d| d.message.as_str())
            .collect();
        assert_eq!(
            unknown.len(),
            1,
            "only the genuinely absent id is unknown: {unknown:?}"
        );
        assert!(unknown[0].contains("GHOST"), "{unknown:?}");
    }

    #[test]
    fn suppression_is_limited_to_the_ids_that_failed_to_load() {
        // The risk the task named: hiding a real dangling reference behind an
        // unrelated parse error. Only edges into *that* id are dropped.
        let mut a = t("A", Status::Todo);
        a.blocked_by = vec!["GHOST".into()];
        let mut plan = plan_of(vec![a]);
        plan.unloadable
            .insert("SOMETHING_ELSE".into(), PathBuf::from("plan/tasks/x.md"));

        let r = validate(&plan, &opts());
        assert!(
            r.diagnostics.iter().any(|d| d.code == code::UNKNOWN_DEP),
            "an unrelated parse failure must not silence a real dangling edge"
        );
    }

    #[test]
    fn a_cancelled_cut_with_live_work_is_reported() {
        // The state a bug produced and nothing reported: `drop --cut` followed a
        // sequencing edge, cancelled the predecessor cut and left its work live
        // (`A1-084`). A hand-edit reaches it too, as does dropping a milestone
        // without `--cut`.
        let mut m = t("M-s0", Status::Dropped);
        m.tags = vec!["milestone".into(), "s0".into()];
        m.effort_days = None;
        m.blocked_by = vec!["A-01".into()];
        let mut live = t("A-01", Status::Todo);
        live.effort_days = Some(1.0);

        let r = validate(&plan_of(vec![m, live]), &opts());
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::CANCELLED_CUT_LIVE_WORK)
            .expect("a cancelled cut whose work is going ahead");
        assert!(d.message.contains("A-01"), "it names them: {}", d.message);
    }

    #[test]
    fn a_cancelled_cut_whose_work_is_cancelled_is_silent() {
        // Coherent: the cut and its work were cancelled together, which is what
        // `drop --cut` does. The live cut it follows is a sequencing edge and
        // ordinary.
        let mut prev = t("M-prev", Status::Todo);
        prev.tags = vec!["milestone".into(), "prev".into()];
        prev.effort_days = None;
        let mut m = t("M-s0", Status::Dropped);
        m.tags = vec!["milestone".into(), "s0".into()];
        m.effort_days = None;
        m.blocked_by = vec!["A-01".into(), "M-prev".into()];

        let r = validate(&plan_of(vec![prev, m, t("A-01", Status::Dropped)]), &opts());
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::CANCELLED_CUT_LIVE_WORK),
            "{:?}",
            r.diagnostics
        );
    }

    #[test]
    fn a_cancelled_member_does_not_block_its_cut() {
        // Leaving a cancelled task in its cut was `APLAN020` and taking it out
        // was `APLAN022`, so a task in a cut could not be cancelled at all. The
        // rollup has excluded dropped members all along; the gate now agrees
        // (`A1-081`).
        let mut m = t("M-s0", Status::Todo);
        m.tags = vec!["milestone".into(), "s0".into()];
        m.blocked_by = vec!["A-01".into(), "A-02".into()];
        let mut dropped = t("A-01", Status::Dropped);
        dropped.effort_days = Some(1.0);
        let mut live = t("A-02", Status::Todo);
        live.effort_days = Some(1.0);

        let r = validate(&plan_of(vec![m, dropped, live]), &opts());
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::DROPPED_BLOCKER),
            "{:?}",
            r.diagnostics
        );
    }

    #[test]
    fn a_cancelled_predecessor_cut_is_still_reported() {
        // The exemption is for a cut's own *members*. A milestone in another
        // milestone's `blocked_by` is a sequencing edge (ADR-0013), and a cut
        // whose stated prerequisite was abandoned is a plan its author has to
        // resolve. Exempting it would also contradict the members, who inherit
        // that edge and are told about it (`A1-081`).
        let mut dropped_cut = t("M-s0", Status::Dropped);
        dropped_cut.tags = vec!["milestone".into(), "s0".into()];
        let mut live_cut = t("M-s1", Status::Todo);
        live_cut.tags = vec!["milestone".into(), "s1".into()];
        live_cut.blocked_by = vec!["B-01".into(), "M-s0".into()];
        let mut member = t("B-01", Status::Todo);
        member.effort_days = Some(1.0);

        let r = validate(&plan_of(vec![dropped_cut, live_cut, member]), &opts());
        let reported: Vec<&str> = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::DROPPED_BLOCKER)
            .filter_map(|d| d.task.as_deref())
            .collect();
        assert!(reported.contains(&"M-s1"), "the cut itself: {reported:?}");
        let d = pick(&r, code::DROPPED_BLOCKER);
        assert!(
            d.message.contains("M-s0"),
            "the message must name the *dropped* blocker, not just the task that \
             waits on it: with several blockers the reader cannot otherwise tell \
             which edge is dead: {}",
            d.message
        );
        assert!(
            reported.contains(&"B-01"),
            "and its members, who inherit the edge: {reported:?}"
        );
    }

    #[test]
    fn the_rollup_and_the_validator_agree_about_a_cancelled_member() {
        // The gap this fix closed was the two disagreeing: `roadmap` excluded a
        // dropped member from a cut's rollup while the gate treated it as a
        // permanent blocker. Asserted rather than assumed.
        let mut m = t("M-s0", Status::Todo);
        m.tags = vec!["milestone".into(), "s0".into()];
        m.blocked_by = vec!["A-01".into(), "A-02".into()];
        m.effort_days = None; // a checkpoint weighs nothing (`APLAN021`)
        let mut dropped = t("A-01", Status::Dropped);
        dropped.effort_days = Some(1.0);
        let mut live = t("A-02", Status::Todo);
        live.effort_days = Some(1.0);
        let tasks = vec![m, dropped, live];

        let r = validate(&plan_of(tasks.clone()), &opts());
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);

        let graph = crate::graph::Graph::new(tasks.iter());
        let cut = &graph.roadmap().cuts[0];
        assert_eq!(cut.total, 1, "the cancelled member is not scope");
        assert_eq!(cut.done + cut.ready + cut.blocked, cut.total);
    }

    #[test]
    fn a_live_task_blocked_by_a_cancelled_one_is_still_reported() {
        // The exemption is for milestones only. An ordinary task waiting on
        // cancelled work genuinely cannot start, which is the whole point of the
        // rule and must not move.
        let mut a = t("A", Status::Todo);
        a.blocked_by = vec!["B".into()];
        let r = validate(&plan_of(vec![a, t("B", Status::Dropped)]), &opts());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::DROPPED_BLOCKER),
            "{:?}",
            r.diagnostics
        );
    }

    #[test]
    fn a_cut_can_be_done_once_its_live_members_are() {
        // ... and not before: the cancelled member is excused, the unfinished
        // one is not.
        let mut m = t("M-s0", Status::Todo);
        m.tags = vec!["milestone".into(), "s0".into()];
        m.blocked_by = vec!["A-01".into(), "A-02".into()];
        m.status = Status::Done;
        let mut done_member = t("A-02", Status::Done);
        done_member.evidence = vec![Evidence {
            kind: EvidenceKind::Note,
            reference: "done".into(),
            captured: None,
        }];

        let complete = validate(
            &plan_of(vec![m.clone(), t("A-01", Status::Dropped), done_member]),
            &opts(),
        );
        assert!(
            !complete
                .diagnostics
                .iter()
                .any(|d| d.code == code::DONE_BEFORE_BLOCKER),
            "a cancelled member must not hold the cut open: {:?}",
            complete.diagnostics
        );

        let premature = validate(
            &plan_of(vec![m, t("A-01", Status::Dropped), t("A-02", Status::Todo)]),
            &opts(),
        );
        assert!(
            premature
                .diagnostics
                .iter()
                .any(|d| d.code == code::DONE_BEFORE_BLOCKER),
            "an unfinished live member still holds it open: {:?}",
            premature.diagnostics
        );
    }

    #[test]
    fn check_enforces_the_cut_sequencing_that_close_enforces() {
        // `close` refuses a task whose cut predecessor is unfinished, because it
        // reads the graph's blocker set. `check` read only the declared edges,
        // so a status hand-edited to `done` passed the gate the write path would
        // have refused (`A1-073`).
        let mut a0 = t("A0", Status::Todo);
        a0.effort_days = Some(1.0);
        let mut b1 = t("B1", Status::Done);
        b1.evidence = vec![Evidence {
            kind: EvidenceKind::Note,
            reference: "done by hand".into(),
            captured: None,
        }];
        let mut m0 = t("M-s0", Status::Todo);
        m0.tags = vec!["milestone".into(), "s0".into()];
        m0.blocked_by = vec!["A0".into()];
        let mut m1 = t("M-s1", Status::Todo);
        m1.tags = vec!["milestone".into(), "s1".into()];
        m1.blocked_by = vec!["B1".into(), "M-s0".into()];

        let r = validate(&plan_of(vec![a0, b1, m0, m1]), &opts());
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::DONE_BEFORE_BLOCKER)
            .expect("a done task whose cut predecessor is unfinished");
        // The author will not find `M-s0` in this task's own `blocked_by`, so
        // the message has to say where the edge came from.
        assert!(d.message.contains("its cut follows"), "{}", d.message);
    }

    #[test]
    fn an_inherited_edge_is_not_reported_twice_when_also_declared() {
        // A plan that declares the cross-cut edge by hand, as the pilot did
        // before this shipped, must not now get two errors for one edge.
        let a0 = t("A0", Status::Todo);
        let mut b1 = t("B1", Status::Done);
        b1.blocked_by = vec!["M-s0".into()];
        b1.evidence = vec![Evidence {
            kind: EvidenceKind::Note,
            reference: "done by hand".into(),
            captured: None,
        }];
        let mut m0 = t("M-s0", Status::Todo);
        m0.tags = vec!["milestone".into(), "s0".into()];
        m0.blocked_by = vec!["A0".into()];
        let mut m1 = t("M-s1", Status::Todo);
        m1.tags = vec!["milestone".into(), "s1".into()];
        m1.blocked_by = vec!["B1".into(), "M-s0".into()];

        let r = validate(&plan_of(vec![a0, b1, m0, m1]), &opts());
        let n = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::DONE_BEFORE_BLOCKER && d.task.as_deref() == Some("B1"))
            .count();
        assert_eq!(n, 1, "one edge, one finding: {:?}", r.diagnostics);
    }

    fn families() -> BTreeMap<String, TagFamily> {
        let mut f = BTreeMap::new();
        f.insert(
            "epic".to_string(),
            TagFamily::members(vec!["epic-a".to_string(), "epic-b".to_string()]),
        );
        f.insert(
            "kind".to_string(),
            TagFamily::members(vec!["kind-b".to_string(), "kind-m".to_string()]),
        );
        f
    }

    /// `APLAN056` judges every owner only after the repository declares the
    /// namespace, and its product is the correction it can actually justify.
    #[test]
    fn a_declared_owner_vocabulary_reports_only_values_outside_it() {
        let owned = |id: &str, owner: Option<&str>| {
            let mut task = t(id, Status::Todo);
            task.effort_days = Some(1.0);
            task.owner = owner.map(str::to_string);
            task
        };
        let declared_opts = Options {
            owners: ["abc", "abe", "kaminsod"]
                .map(str::to_string)
                .into_iter()
                .collect(),
            ..opts()
        };
        let report = validate(
            &plan_of(vec![
                owned("EXACT", Some("kaminsod")),
                owned("TYPO", Some("kaminsdo")),
                owned("AMBIGUOUS", Some("abd")),
                owned("UNKNOWN", Some("outsider")),
                owned("FREE", None),
            ]),
            &declared_opts,
        );
        let findings: Vec<&Diagnostic> = report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == code::OWNER_VOCABULARY)
            .collect();
        assert_eq!(findings.len(), 3, "{:?}", report.diagnostics);

        let typo = findings
            .iter()
            .find(|diagnostic| diagnostic.task.as_deref() == Some("TYPO"))
            .expect("the misspelled owner is reported");
        assert!(
            typo.message.contains("owner `kaminsdo`"),
            "{}",
            typo.message
        );
        assert!(
            typo.message.contains("did you mean `kaminsod`?"),
            "{}",
            typo.message
        );
        let fix = typo.agent_instruction.as_deref().expect("a fix line");
        assert!(fix.contains("Change this task's `owner`"), "{fix}");
        assert!(fix.contains("add `kaminsdo` to `owners`"), "{fix}");

        let ambiguous = findings
            .iter()
            .find(|diagnostic| diagnostic.task.as_deref() == Some("AMBIGUOUS"))
            .expect("an owner with two neighbours is reported once");
        assert!(
            ambiguous.message.contains("`abc`, `abe`"),
            "{}",
            ambiguous.message
        );
        assert!(
            ambiguous
                .agent_instruction
                .as_deref()
                .is_some_and(|instruction| instruction.contains("Choose the intended")),
            "{:?}",
            ambiguous.agent_instruction
        );

        let unknown = findings
            .iter()
            .find(|diagnostic| diagnostic.task.as_deref() == Some("UNKNOWN"))
            .expect("a non-near undeclared owner is still outside the vocabulary");
        assert_eq!(
            unknown.message,
            "owner `outsider` is not declared in `owners`"
        );
        assert!(
            unknown
                .agent_instruction
                .as_deref()
                .is_some_and(|instruction| instruction.contains("correct this task's `owner`")),
            "{:?}",
            unknown.agent_instruction
        );

        let open = validate(&plan_of(vec![owned("OPEN", Some("any-spelling"))]), &opts());
        assert!(
            open.diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != code::OWNER_VOCABULARY),
            "an absent vocabulary changed existing owner semantics: {:?}",
            open.diagnostics
        );
    }

    /// `APLAN056` has no status or milestone binding: canonical identity is a
    /// property of every stored owner, including history and cut checkpoints.
    /// Keep every lifecycle neighbour in one exact subject audit so a later
    /// exemption cannot narrow the rule while its ordinary `todo` case passes.
    #[test]
    fn owner_vocabulary_applies_to_every_lifecycle_status_and_milestones() {
        let mut tasks = Vec::new();
        for status in Status::ALL {
            let id = format!("STATE-{}", status.as_str());
            let mut task = t(&id, *status);
            task.effort_days = Some(1.0);
            task.owner = Some(format!("undeclared-{}", status.as_str()));
            tasks.push(task);
        }
        let mut milestone = t("M-CUT", Status::Todo);
        milestone.tags = vec!["milestone".into(), "cut".into()];
        milestone.owner = Some("undeclared-milestone".into());
        tasks.push(milestone);

        let expected: BTreeSet<String> = tasks.iter().map(|task| task.id.clone()).collect();
        let report = validate(
            &plan_of(tasks),
            &Options {
                owners: ["ada".to_string()].into_iter().collect(),
                ..opts()
            },
        );
        let actual: BTreeSet<String> = report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == code::OWNER_VOCABULARY)
            .map(|diagnostic| {
                diagnostic
                    .task
                    .clone()
                    .expect("an owner finding names its task")
            })
            .collect();
        assert_eq!(actual, expected, "{:?}", report.diagnostics);
    }

    fn tagged(id: &str, tags: &[&str]) -> Task {
        let mut t = t(id, Status::Todo);
        t.effort_days = Some(1.0);
        t.tags = tags.iter().map(|s| (*s).to_string()).collect();
        t
    }

    fn tag_opts() -> Options {
        Options {
            tag_families: families(),
            ..opts()
        }
    }

    /// A family with a cardinality, for the `APLAN030` cases below.
    fn required(require: Cardinality, status: StatusScope, milestones: MilestoneScope) -> Options {
        let mut f = BTreeMap::new();
        f.insert(
            "priority".to_string(),
            TagFamily {
                members: vec!["priority-p0".to_string(), "priority-p1".to_string()],
                require,
                status,
                milestones,
            },
        );
        Options {
            tag_families: f,
            ..opts()
        }
    }

    /// The tasks under test plus one that carries a declared member.
    ///
    /// Without the binder the family is one no tag in the plan matches, which is
    /// `APLAN029`'s finding and silences this rule deliberately, so every case
    /// below would pass for the wrong reason: not "the requirement holds" but
    /// "the vocabulary was never bound".
    fn bound_plan(tasks: Vec<Task>) -> Plan {
        let mut all = tasks;
        all.push(tagged("BIND", &["priority-p0"]));
        plan_of(all)
    }

    fn cardinality_findings(r: &Report) -> Vec<&Diagnostic> {
        r.diagnostics
            .iter()
            .filter(|d| d.code == code::TAG_CARDINALITY)
            .collect()
    }

    /// `judges` on its own, because it is public API and the rule's own use of it
    /// would pass without it: the count arm answers `true` for `Optional` too, so
    /// deleting the guard changes no diagnostic. What it changes is what the
    /// predicate means to anyone else who asks it, and `add` is already someone
    /// else.
    #[test]
    fn a_family_with_no_requirement_judges_nothing() {
        let family = TagFamily::members(vec!["epic-a".to_string()]);
        assert!(!family.judges(&tagged("A", &["epic-a"])));
        assert!(!family.judges(&tagged("B", &[])));
        // And one with a requirement does judge, or the assertion above would
        // hold for a predicate that always answered `false`.
        let required = TagFamily {
            require: Cardinality::ExactlyOne,
            ..TagFamily::members(vec!["epic-a".to_string()])
        };
        assert!(required.judges(&tagged("A", &["epic-a"])));

        // And `satisfied_by` answers for both: a family with no requirement is
        // met by anything, and one with a requirement is met by meeting it.
        assert!(family.satisfied_by(&tagged("B", &[])));
        assert!(required.satisfied_by(&tagged("A", &["epic-a"])));
        assert!(!required.satisfied_by(&tagged("B", &[])));
        // Out of scope is met too, whatever the task carries.
        let done_only = TagFamily {
            require: Cardinality::ExactlyOne,
            status: StatusScope::Done,
            ..TagFamily::members(vec!["epic-a".to_string()])
        };
        assert!(done_only.satisfied_by(&tagged("B", &[])));
    }

    /// The counts, in both directions, over every declared cardinality.
    #[test]
    fn each_cardinality_fires_on_the_counts_it_forbids_and_is_silent_on_the_rest() {
        let none: &[&str] = &[];
        let one: &[&str] = &["priority-p0"];
        let two: &[&str] = &["priority-p0", "priority-p1"];
        for (require, fires_on) in [
            (Cardinality::Optional, vec![]),
            (Cardinality::AtMostOne, vec![2usize]),
            (Cardinality::AtLeastOne, vec![0]),
            (Cardinality::ExactlyOne, vec![0, 2]),
        ] {
            for (count, tags) in [(0usize, none), (1, one), (2, two)] {
                let opts = required(require, StatusScope::Any, MilestoneScope::Include);
                let r = validate(&bound_plan(vec![tagged("A", tags)]), &opts);
                let fired = !cardinality_findings(&r).is_empty();
                assert_eq!(
                    fired,
                    fires_on.contains(&count),
                    "{:?} with {count} member(s): {:?}",
                    require,
                    r.diagnostics
                );
            }
        }
    }

    /// The message has to name the requirement and what it found, or the author
    /// cannot tell "you have two" from "you have none".
    #[test]
    fn the_message_names_the_requirement_and_what_the_task_carries() {
        let opts = required(
            Cardinality::ExactlyOne,
            StatusScope::Live,
            MilestoneScope::Include,
        );
        let r = validate(
            &bound_plan(vec![tagged("A", &["priority-p0", "priority-p1"])]),
            &opts,
        );
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::TAG_CARDINALITY)
            .expect("reported");
        assert!(
            d.message.contains("`priority` is declared exactly-one"),
            "{}",
            d.message
        );
        assert!(d.message.contains("on live tasks"), "{}", d.message);
        assert!(
            d.message.contains("carries `priority-p0`, `priority-p1`"),
            "{}",
            d.message
        );

        // And the other direction says `none` rather than listing nothing.
        let r = validate(&bound_plan(vec![tagged("A", &["unrelated"])]), &opts);
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::TAG_CARDINALITY && d.task.as_deref() == Some("A"))
            .expect("reported");
        assert!(d.message.ends_with("carries none"), "{}", d.message);
        // The fix names the count, not just the family.
        let fix = d.agent_instruction.as_deref().expect("a fix line");
        assert!(
            fix.contains("Carry exactly one of `tag_families.priority`"),
            "{fix}"
        );
    }

    /// Scope by status: the case the fifth pilot needs, where a priority is
    /// required while the work is live and meaningless once it is not.
    #[test]
    fn a_status_scope_judges_only_the_statuses_it_names() {
        let opts = required(
            Cardinality::ExactlyOne,
            StatusScope::Live,
            MilestoneScope::Include,
        );
        for (status, judged) in [
            (Status::Todo, true),
            (Status::InProgress, true),
            (Status::Blocked, true),
            (Status::Done, false),
            (Status::Dropped, false),
        ] {
            let mut t = tagged("A", &["unrelated"]);
            t.status = status;
            let r = validate(&bound_plan(vec![t]), &opts);
            assert_eq!(
                !cardinality_findings(&r).is_empty(),
                judged,
                "{status:?}: {:?}",
                r.diagnostics
            );
        }
    }

    /// The remaining status scopes, and the words each puts in a message. The
    /// fifth pilot needs `dropped` for a disposition tag, and `terminal` is the
    /// pair, so neither can be left to a scope test that only covers `live`.
    #[test]
    fn the_terminal_scopes_judge_and_read_as_they_say() {
        for (scope, judges) in [
            (
                StatusScope::Done,
                vec![
                    (Status::Done, true),
                    (Status::Dropped, false),
                    (Status::Todo, false),
                ],
            ),
            (
                StatusScope::Dropped,
                vec![
                    (Status::Done, false),
                    (Status::Dropped, true),
                    (Status::Todo, false),
                ],
            ),
            (
                StatusScope::Terminal,
                vec![
                    (Status::Done, true),
                    (Status::Dropped, true),
                    (Status::Todo, false),
                ],
            ),
            (
                StatusScope::Any,
                vec![
                    (Status::Done, true),
                    (Status::Dropped, true),
                    (Status::Todo, true),
                ],
            ),
        ] {
            let opts = required(Cardinality::AtLeastOne, scope, MilestoneScope::Include);
            for (status, judged) in judges {
                let mut t = tagged("A", &["unrelated"]);
                t.status = status;
                let r = validate(&bound_plan(vec![t]), &opts);
                assert_eq!(
                    r.diagnostics
                        .iter()
                        .any(|d| d.code == code::TAG_CARDINALITY && d.task.as_deref() == Some("A")),
                    judged,
                    "{scope:?} on {status:?}: {:?}",
                    r.diagnostics
                );
            }
        }
        // And the word each contributes to the message, since a scope that reads
        // as another is a message naming the wrong tasks.
        assert_eq!(StatusScope::Any.as_str(), "every");
        assert_eq!(StatusScope::Live.as_str(), "live");
        assert_eq!(StatusScope::Done.as_str(), "done");
        assert_eq!(StatusScope::Dropped.as_str(), "dropped");
        assert_eq!(StatusScope::Terminal.as_str(), "terminal");
        assert_eq!(Cardinality::Optional.as_str(), "optional");
        assert_eq!(Cardinality::AtMostOne.as_str(), "at-most-one");
        assert_eq!(Cardinality::AtLeastOne.as_str(), "at-least-one");
        assert_eq!(Cardinality::ExactlyOne.as_str(), "exactly-one");
    }

    /// Scope by kind: a vocabulary that classifies work should not be demanded of
    /// a milestone, and one that classifies cuts should be demanded of nothing
    /// else.
    #[test]
    fn a_milestone_scope_judges_only_the_kind_it_names() {
        for (scope, milestone_judged, task_judged) in [
            (MilestoneScope::Include, true, true),
            (MilestoneScope::Exclude, false, true),
            (MilestoneScope::Only, true, false),
        ] {
            let opts = required(Cardinality::AtLeastOne, StatusScope::Any, scope);
            let m = tagged("M-1", &["milestone"]);
            let r = validate(&bound_plan(vec![m]), &opts);
            assert_eq!(
                !cardinality_findings(&r).is_empty(),
                milestone_judged,
                "milestone under {scope:?}: {:?}",
                r.diagnostics
            );
            let r = validate(&bound_plan(vec![tagged("A", &["unrelated"])]), &opts);
            assert_eq!(
                cardinality_findings(&r)
                    .iter()
                    .any(|d| d.task.as_deref() == Some("A")),
                task_judged,
                "task under {scope:?}: {:?}",
                r.diagnostics
            );
        }
    }

    /// Only declared members count. A tag that merely looks like one is
    /// `APLAN026`'s finding, and treating it as proof the requirement was met
    /// would be the rule accepting a typo as evidence.
    #[test]
    fn a_near_miss_tag_does_not_satisfy_a_requirement() {
        let opts = required(
            Cardinality::ExactlyOne,
            StatusScope::Any,
            MilestoneScope::Include,
        );
        let r = validate(&bound_plan(vec![tagged("A", &["priority-p9"])]), &opts);
        assert!(
            !cardinality_findings(&r).is_empty(),
            "a typo satisfied the requirement: {:?}",
            r.diagnostics
        );
        // Both findings, because they are two different things to fix.
        assert!(
            r.diagnostics.iter().any(|d| d.code == code::UNKNOWN_TAG),
            "{:?}",
            r.diagnostics
        );
    }

    /// A member written twice on one task is one tag, and must not fail
    /// `exactly-one`: the plan says the same thing twice, which `APLAN026`'s own
    /// deduplication already treats as one mistake.
    #[test]
    fn a_member_repeated_on_one_task_counts_once() {
        let opts = required(
            Cardinality::ExactlyOne,
            StatusScope::Any,
            MilestoneScope::Include,
        );
        let r = validate(
            &bound_plan(vec![tagged("A", &["priority-p0", "priority-p0"])]),
            &opts,
        );
        assert!(
            cardinality_findings(&r).is_empty(),
            "a repeated tag was counted twice: {:?}",
            r.diagnostics
        );
    }

    /// Two families with different requirements are judged independently, and an
    /// overlapping member counts for whichever families declare it.
    #[test]
    fn overlapping_families_are_judged_independently() {
        let mut f = BTreeMap::new();
        f.insert(
            "priority".to_string(),
            TagFamily {
                members: vec!["shared".to_string()],
                require: Cardinality::ExactlyOne,
                status: StatusScope::Any,
                milestones: MilestoneScope::Include,
            },
        );
        f.insert(
            "kind".to_string(),
            TagFamily {
                members: vec!["shared".to_string(), "kind-b".to_string()],
                require: Cardinality::AtMostOne,
                status: StatusScope::Any,
                milestones: MilestoneScope::Include,
            },
        );
        let opts = Options {
            tag_families: f,
            ..opts()
        };
        // `shared` satisfies `priority` and is one of `kind`: both hold.
        let r = validate(&plan_of(vec![tagged("A", &["shared"])]), &opts);
        assert!(cardinality_findings(&r).is_empty(), "{:?}", r.diagnostics);
        // Adding `kind-b` breaks `kind` alone, and the message says which.
        let r = validate(&plan_of(vec![tagged("A", &["shared", "kind-b"])]), &opts);
        let found = cardinality_findings(&r);
        assert_eq!(found.len(), 1, "{:?}", r.diagnostics);
        let message = found[0].message.clone();
        assert!(message.starts_with("`kind` is declared"), "{message}");
    }

    /// The fourth pilot's exact configuration, which every layer accepted.
    #[test]
    fn a_family_no_task_carries_is_reported_with_what_it_would_take_to_join() {
        // `epic: [a, b]` with a task tagged `epic:a`: the family name read as a
        // prefix and the members as suffixes, where the documented shape is the
        // whole tag (pre-public adoption evidence, item 9).
        let mut mis: BTreeMap<String, TagFamily> = BTreeMap::new();
        mis.insert(
            "epic".to_string(),
            TagFamily::members(vec!["a".to_string(), "b".to_string()]),
        );
        let opts = Options {
            tag_families: mis,
            ..opts()
        };
        let r = validate(&plan_of(vec![tagged("A", &["epic:a"])]), &opts);
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::UNUSED_TAG_FAMILY)
            .expect("reported");
        // The family, and what a task would have to carry: without the members
        // the reader cannot see that `epic:a` is not one of them.
        assert!(d.message.contains("`epic` family"), "{}", d.message);
        assert!(d.message.contains("`a`, `b`"), "{}", d.message);
        assert!(d.message.contains("as a whole tag"), "{}", d.message);
        // An observation, not a diagnosis: a vocabulary declared before its
        // first use is as likely as a mis-declaration, and the two remedies
        // differ, so the fix line carries both (`A1-070`'s lesson).
        let fix = d.agent_instruction.as_deref().expect("a fix line");
        assert!(fix.contains("Tag a task with one of them"), "{fix}");
        assert!(fix.contains("correct `tag_families.epic`"), "{fix}");
        // Advisory, on `APLAN026`'s reasoning.
        assert!(r.is_clean(), "{:?}", r.diagnostics);
    }

    /// The finding is about a declaration, so it points at the file the
    /// declaration is in. Without that it was the one rule in twenty-nine that
    /// could point nowhere, and a second unused family rendered as a bare
    /// sentence with no location and no fix line, since `check` prints the fix
    /// once per rule.
    #[test]
    fn the_finding_points_at_the_config_it_is_about() {
        let mut fams: BTreeMap<String, TagFamily> = BTreeMap::new();
        fams.insert(
            "epic".to_string(),
            TagFamily::members(vec!["a".to_string()]),
        );
        fams.insert(
            "kind".to_string(),
            TagFamily::members(vec!["x".to_string()]),
        );
        let opts = Options {
            tag_families: fams,
            config_file: Some(std::path::PathBuf::from("/repo/.aplan.yml")),
            ..opts()
        };
        let r = validate(&plan_of(vec![tagged("A", &["unrelated"])]), &opts);
        let found: Vec<&Diagnostic> = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::UNUSED_TAG_FAMILY)
            .collect();
        assert_eq!(found.len(), 2, "{:?}", r.diagnostics);
        for d in found {
            assert_eq!(
                d.file.as_deref(),
                Some(std::path::Path::new("/repo/.aplan.yml")),
                "{}",
                d.message
            );
        }
    }

    /// One family used and one not: exactly one finding, naming the right
    /// family. A rule that reported both, or neither, would be indistinguishable
    /// from this one on a plan that uses every family it declares.
    #[test]
    fn only_the_unused_family_of_several_is_reported() {
        let r = validate(&plan_of(vec![tagged("A", &["epic-a"])]), &tag_opts());
        let found: Vec<&Diagnostic> = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::UNUSED_TAG_FAMILY)
            .collect();
        assert_eq!(found.len(), 1, "{:?}", r.diagnostics);
        let message = found[0].message.clone();
        assert!(message.contains("`kind` family"), "{message}");
    }

    #[test]
    fn a_family_carried_by_any_task_is_silent_and_so_is_declaring_none() {
        // One task carrying one declared member is all it takes, for each family.
        let plan = plan_of(vec![tagged("A", &["epic-a", "kind-b"])]);
        let used = validate(&plan, &tag_opts());
        let fired = used
            .diagnostics
            .iter()
            .any(|d| d.code == code::UNUSED_TAG_FAMILY);
        assert!(!fired, "a used family was reported: {:?}", used.diagnostics);
        // And a plan declaring no family is untouched, or the rule would be a
        // migration for every repository that has never opened `.aplan.yml`.
        let undeclared = validate(&plan, &opts());
        let fired = undeclared
            .diagnostics
            .iter()
            .any(|d| d.code == code::UNUSED_TAG_FAMILY);
        assert!(
            !fired,
            "a plan with no families was reported: {:?}",
            undeclared.diagnostics
        );
    }

    /// One mistake, one finding. A tag in a family's shape counts as using it
    /// even though `APLAN026` is about to reject it, or a single typo would be
    /// reported twice: once as an unknown tag and once as an unused vocabulary.
    #[test]
    fn a_family_whose_only_tag_is_a_typo_is_not_reported_as_unused_as_well() {
        for tag in ["epic-x", "epci-b"] {
            let r = validate(&plan_of(vec![tagged("A", &[tag])]), &tag_opts());
            assert!(
                r.diagnostics.iter().any(|d| d.code == code::UNKNOWN_TAG),
                "{tag}: {:?}",
                r.diagnostics
            );
            assert!(
                !r.diagnostics
                    .iter()
                    .any(|d| d.code == code::UNUSED_TAG_FAMILY
                        && d.message.contains("`epic` family")),
                "{tag} was reported as a typo and as an unused family: {:?}",
                r.diagnostics
            );
        }
    }

    #[test]
    fn an_undeclared_member_of_a_declared_family_is_reported() {
        // The case a prefix catches: an epic nobody wrote down. Saying the
        // family does not list it beats suggesting the nearest declared one,
        // which `epic-x` is also one substitution from.
        let r = validate(&plan_of(vec![tagged("A", &["epic-x"])]), &tag_opts());
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::UNKNOWN_TAG)
            .expect("reported");
        assert!(d.message.contains("does not list it"), "{}", d.message);
        assert!(!d.message.contains("did you mean"), "{}", d.message);
    }

    #[test]
    fn a_transposition_the_prefix_cannot_see_is_reported_with_the_fix() {
        // `epci-b` carries no recognisable prefix, which is exactly why a
        // prefix-only rule would miss the most common kind of typo.
        let r = validate(&plan_of(vec![tagged("A", &["epci-b"])]), &tag_opts());
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::UNKNOWN_TAG)
            .expect("reported");
        assert!(d.message.contains("did you mean `epic-b`"), "{}", d.message);
    }

    #[test]
    fn declared_members_and_unrelated_tags_stay_silent() {
        // Tags outside every declared family must stay legal, or declaring one
        // family would be a migration of every tag in the plan.
        let plan = plan_of(vec![
            tagged("A", &["epic-a", "kind-b"]),
            tagged("B", &["core", "cross-cutting", "milestone"]),
        ]);
        let r = validate(&plan, &tag_opts());
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::UNKNOWN_TAG),
            "{:?}",
            r.diagnostics
        );
    }

    #[test]
    fn a_plan_declaring_nothing_is_untouched() {
        // The default, and this repository's own plan: no families, no findings,
        // whatever the tags say.
        let plan = plan_of(vec![tagged("A", &["epci-b", "epic-x", "anything"])]);
        let r = validate(&plan, &opts());
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::UNKNOWN_TAG),
            "{:?}",
            r.diagnostics
        );
    }

    #[test]
    fn a_tag_listed_twice_is_reported_once() {
        // A tag repeated in one task is one mistake. `blocked_by` deduplicates
        // for the same reason: two identical annotations under `--format github`
        // is noise, not thoroughness.
        let r = validate(
            &plan_of(vec![tagged("A", &["epic-x", "epic-x"])]),
            &tag_opts(),
        );
        let n = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::UNKNOWN_TAG)
            .count();
        assert_eq!(n, 1, "{:?}", r.diagnostics);
    }

    #[test]
    fn the_longest_matching_family_is_the_one_named() {
        // One family's name can be a prefix of another's. Naming `epic` for a
        // tag that belongs to `epic-sub` sends the author to the wrong list,
        // which is worse than saying nothing.
        let mut f = BTreeMap::new();
        f.insert(
            "epic".to_string(),
            TagFamily::members(vec!["epic-a".to_string()]),
        );
        f.insert(
            "epic-sub".to_string(),
            TagFamily::members(vec!["epic-sub-x".to_string()]),
        );
        let opts = Options {
            tag_families: f,
            ..opts()
        };
        let r = validate(&plan_of(vec![tagged("A", &["epic-sub-y"])]), &opts);
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::UNKNOWN_TAG)
            .expect("reported");
        assert!(d.message.contains("`epic-sub`"), "{}", d.message);
        assert!(
            d.agent_instruction
                .as_deref()
                .is_some_and(|i| i.contains("tag_families.epic-sub")),
            "the instruction must name the list to edit: {:?}",
            d.agent_instruction
        );
    }

    /// The five words, and what a near miss of one actually costs (`A1-212`).
    #[test]
    fn a_tag_one_edit_from_a_reserved_word_is_reported() {
        let t = tagged("W-1", &["deprecatoin"]);
        let r = validate(&bound_plan(vec![t]), &Options::default());
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::RESERVED_TAG_NEAR_MISS)
            .expect("one edit from `deprecation`");
        assert_eq!(d.severity, crate::Severity::Warning);
        assert!(d.message.contains("deprecation"), "{}", d.message);
        assert!(
            d.message.contains("changes what this tool does"),
            "the cost is the content: a typo here does not misname, it changes \
             the plan: {}",
            d.message
        );
    }

    /// The word itself, and a tag nowhere near one, are both left alone.
    #[test]
    fn a_correct_reserved_word_and_an_ordinary_tag_are_silent() {
        for tags in [&["milestone"][..], &["backend"][..], &["release"][..]] {
            let t = tagged("W-1", tags);
            let r = validate(&bound_plan(vec![t]), &Options::default());
            assert!(
                !r.diagnostics
                    .iter()
                    .any(|d| d.code == code::RESERVED_TAG_NEAR_MISS),
                "{tags:?} was reported"
            );
        }
    }

    /// A repository declaring one of the five in its own family is told that
    /// the tool's meaning is the one that applies -- once, against the config.
    #[test]
    fn declaring_a_reserved_word_in_a_family_is_reported_once() {
        let t = tagged("W-1", &["release"]);
        let mut opts = required(
            Cardinality::Optional,
            StatusScope::Live,
            MilestoneScope::Include,
        );
        opts.tag_families.insert(
            "kind".to_string(),
            TagFamily {
                members: vec!["release".to_string(), "spike".to_string()],
                require: Cardinality::Optional,
                status: StatusScope::Live,
                milestones: MilestoneScope::Include,
            },
        );
        let r = validate(&bound_plan(vec![t]), &opts);
        let found: Vec<_> = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::RESERVED_TAG_NEAR_MISS)
            .collect();
        assert_eq!(found.len(), 1, "once, not once per task: {found:?}");
        assert!(found[0].message.contains("tag_families.kind"), "{found:?}");
        assert!(
            found[0].task.is_none(),
            "it is a fact about the configuration, not about a task: {found:?}"
        );
    }

    /// The waiver applies where the tag can mean something, and nowhere else
    /// (`A1-212`, audit).
    ///
    /// All four cases, because the defect was in the one nobody wrote down: a
    /// task in *two* cuts carrying the tag passed as clean, since `APLAN022`
    /// took the tag as the answer without looking at the count.
    #[test]
    fn cross_cutting_waives_belonging_to_no_cut_and_nothing_else() {
        let opts = Options {
            require_cut_membership: true,
            ..Options::default()
        };
        let cut = |id: &str| {
            let mut m = tagged(id, &["milestone"]);
            m.blocked_by = vec!["W-1".to_string()];
            m
        };
        let codes = |tasks: Vec<Task>| -> Vec<&'static str> {
            validate(&bound_plan(tasks), &opts)
                .diagnostics
                .into_iter()
                .filter(|d| d.task.as_deref() == Some("W-1"))
                .map(|d| d.code)
                .collect()
        };

        let tagged_work = tagged("W-1", &["cross-cutting"]);
        let plain = tagged("W-1", &[]);

        // In two cuts: an error, and the tag does not excuse it.
        let two = codes(vec![tagged_work.clone(), cut("M-a"), cut("M-b")]);
        assert!(
            two.contains(&code::MEMBERSHIP_TOTALITY),
            "a task in two cuts was masked by the tag: {two:?}"
        );
        assert!(two.contains(&code::CROSS_CUTTING_IN_A_CUT), "{two:?}");

        // In exactly one: membership is satisfied, and only the tag is wrong.
        assert_eq!(
            codes(vec![tagged_work.clone(), cut("M-a")]),
            vec![code::CROSS_CUTTING_IN_A_CUT]
        );

        // In none, tagged: the case the waiver exists for.
        assert!(codes(vec![tagged_work]).is_empty());

        // In none, untagged: the error the waiver waives.
        assert_eq!(codes(vec![plain]), vec![code::MEMBERSHIP_TOTALITY]);
    }

    /// One tag, one mistake, one finding (`A1-212`, audit).
    ///
    /// A repository with a `releas` family and a task tagged `releasa` got two
    /// warnings that disagreed: `APLAN026` offering `releas-a` and `APLAN051`
    /// offering `release`. Two remedies for one tag is the "two rules
    /// disagreeing" this task exists to prevent, so a tag a declared family
    /// claims is that family's and is judged there.
    #[test]
    fn a_tag_a_declared_family_claims_is_judged_once() {
        let t = tagged("W-1", &["releasa"]);
        let mut opts = required(
            Cardinality::Optional,
            StatusScope::Live,
            MilestoneScope::Include,
        );
        opts.tag_families.insert(
            "releas".to_string(),
            TagFamily {
                members: vec!["releas-a".to_string(), "releas-b".to_string()],
                require: Cardinality::Optional,
                status: StatusScope::Live,
                milestones: MilestoneScope::Include,
            },
        );
        let r = validate(&bound_plan(vec![t.clone()]), &opts);
        let codes: Vec<&str> = r
            .diagnostics
            .iter()
            .filter(|d| d.task.as_deref() == Some("W-1"))
            .map(|d| d.code)
            .collect();
        assert_eq!(
            codes,
            vec![code::UNKNOWN_TAG],
            "the family the repository declared is the one that judges it"
        );

        // And with no family to claim it, the reserved list is the only judge.
        let alone = validate(&bound_plan(vec![t]), &Options::default());
        assert!(
            alone
                .diagnostics
                .iter()
                .any(|d| d.code == code::RESERVED_TAG_NEAR_MISS),
            "a tag nobody declared is still one edit from `release`"
        );
    }

    /// `cross-cutting` on a task in a cut is reported; on a task in no cut it is
    /// what `APLAN022` reads it as, and is silent. That asymmetry is the design.
    #[test]
    fn cross_cutting_is_reported_only_where_it_cannot_mean_what_it_says() {
        let member = tagged("W-1", &["cross-cutting"]);
        let loose = tagged("W-2", &["cross-cutting"]);
        let mut cut = tagged("M-1", &["milestone"]);
        cut.blocked_by = vec!["W-1".to_string()];

        let r = validate(&bound_plan(vec![member, loose, cut]), &Options::default());
        let found: Vec<&str> = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::CROSS_CUTTING_IN_A_CUT)
            .filter_map(|d| d.task.as_deref())
            .collect();
        assert_eq!(
            found,
            vec!["W-1"],
            "the one in a cut is reported and the one in none is not"
        );

        // What it says is the product. The message has to name the
        // contradiction *and* the cost, because "remove this tag" without
        // "it switches `APLAN022` off" reads as tidying.
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::CROSS_CUTTING_IN_A_CUT)
            .expect("reported");
        assert_eq!(d.severity, crate::Severity::Warning);
        assert!(d.message.contains("member of a cut"), "{}", d.message);
        assert!(d.message.contains("APLAN022"), "{}", d.message);
        assert!(
            d.message.contains("belongs to no cut"),
            "it names what the tag claims, which is what the membership \
             contradicts: {}",
            d.message
        );
        assert!(
            !d.message.contains("switches"),
            "the waiver is narrowed now, so the message must not still describe \
             a check being switched off: {}",
            d.message
        );
        let fix = d.agent_instruction.as_deref().expect("a fix line");
        assert!(
            fix.contains("tag_families"),
            "and the alternative for an author who meant the English sense: {fix}"
        );
    }

    #[test]
    fn one_edit_apart_is_the_edits_it_claims() {
        // A hand-rolled distance, so the boundary is pinned rather than assumed.
        assert!(is_one_edit_apart("epic-b", "epci-b"), "transposition");
        assert!(is_one_edit_apart("kind-m", "kimd-m"), "substitution");
        assert!(is_one_edit_apart("epic-a", "epic-"), "deletion");
        assert!(is_one_edit_apart("epic-a", "epic-ab"), "insertion");
        assert!(
            !is_one_edit_apart("epic-a", "epic-a"),
            "identical is not near"
        );
        assert!(!is_one_edit_apart("epic-a", "kind-m"), "two edits or more");
        assert!(!is_one_edit_apart("a", "abc"), "length gap of two");
    }

    #[test]
    fn self_dependency_is_an_error() {
        let mut a = t("A", Status::Todo);
        a.blocked_by = vec!["A".into()];
        let r = validate(&plan_of(vec![a]), &opts());
        assert!(r.diagnostics.iter().any(|d| d.code == code::SELF_DEP));
        let d = pick(&r, code::SELF_DEP);
        assert!(d.message.contains("`blocked_by`"), "{}", d.message);
        assert!(
            d.agent_instruction
                .as_deref()
                .is_some_and(|i| i.contains("`A`")),
            "the fix must name the task whose edge to remove: {:?}",
            d.agent_instruction
        );
    }

    /// Regression: a self-loop fired both `SELF_DEP` and `CYCLE`, though the
    /// former exists precisely so the generic cycle message isn't used for it.
    #[test]
    fn self_loop_is_self_dep_not_also_cycle() {
        let mut a = t("A", Status::Todo);
        a.blocked_by = vec!["A".into()];
        let r = validate(&plan_of(vec![a]), &opts());
        assert!(r.diagnostics.iter().any(|d| d.code == code::SELF_DEP));
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::CYCLE),
            "self-loop double-reported as a cycle: {:?}",
            r.diagnostics
        );
    }

    /// Regression: two diagnostics that tie on (code, task) must order
    /// deterministically, the sort key is now total (..., file, message).
    #[test]
    fn tied_diagnostics_sort_deterministically() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let mut a = t("A", Status::Todo);
        a.traces = vec!["ZZZ.md#x".into(), "AAA.md#y".into()];
        let o = Options {
            require_effort: false,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let r = validate(&plan_of(vec![a]), &o);
        let msgs: Vec<_> = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::DANGLING_TRACE)
            .map(|d| d.message.clone())
            .collect();
        assert_eq!(msgs.len(), 2);
        let mut sorted = msgs.clone();
        sorted.sort();
        assert_eq!(msgs, sorted, "tied diagnostics must be message-ordered");
    }

    #[test]
    fn cycle_is_an_error() {
        let mut a = t("A", Status::Todo);
        let mut b = t("B", Status::Todo);
        a.blocked_by = vec!["B".into()];
        b.blocked_by = vec!["A".into()];
        let r = validate(&plan_of(vec![a, b]), &opts());
        assert!(r.diagnostics.iter().any(|d| d.code == code::CYCLE));
    }

    #[test]
    fn done_before_blocker_is_an_error() {
        let mut a = t("A", Status::Done);
        a.evidence = vec![Evidence {
            kind: EvidenceKind::Commit,
            reference: "abc1234".into(),
            captured: None,
        }];
        a.blocked_by = vec!["B".into()];
        let b = t("B", Status::Todo);
        let r = validate(&plan_of(vec![a, b]), &opts());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::DONE_BEFORE_BLOCKER)
        );
    }

    /// The plan is deadlocked: `B` is live but waits on a task that was
    /// abandoned, so it can never legitimately start. `ready`/`why` already knew
    /// this (`Dropped` does not satisfy a dependency); `check` used to say `ok`.
    #[test]
    fn live_task_blocked_by_a_dropped_one_is_an_error() {
        let dropped = t("A", Status::Dropped);
        let mut live = t("B", Status::Todo);
        live.blocked_by = vec!["A".into()];
        let r = validate(&plan_of(vec![dropped, live]), &opts());
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::DROPPED_BLOCKER)
            .expect("deadlock reported");
        assert_eq!(d.task.as_deref(), Some("B"));
        assert!(!r.is_clean(), "a deadlocked plan must not pass `check`");
    }

    /// The rule is scoped to *live* tasks: a `done` dependent is already
    /// `APLAN007`, and two abandoned tasks are not a deadlock.
    #[test]
    fn dropped_blocker_rule_does_not_fire_on_terminal_dependents() {
        let dropped = t("A", Status::Dropped);
        let mut also_dropped = t("B", Status::Dropped);
        also_dropped.blocked_by = vec!["A".into()];
        let r = validate(&plan_of(vec![dropped, also_dropped]), &opts());
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::DROPPED_BLOCKER),
            "a dropped task waiting on a dropped task is not a deadlock: {:?}",
            r.diagnostics
        );

        // A `done` dependent is `APLAN007`'s business, not this rule's, the two
        // must not double-report the same edge.
        let dropped = t("A", Status::Dropped);
        let mut done = t("C", Status::Done);
        done.evidence = vec![Evidence {
            kind: EvidenceKind::Commit,
            reference: "abc1234".into(),
            captured: None,
        }];
        done.blocked_by = vec!["A".into()];
        let r = validate(&plan_of(vec![dropped, done]), &opts());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::DONE_BEFORE_BLOCKER)
        );
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::DROPPED_BLOCKER),
            "APLAN007 and APLAN020 must not both fire on one edge: {:?}",
            r.diagnostics
        );
    }

    /// Regression: `blocked_by: [GHOST, GHOST]` is one broken edge, but emitted
    /// the identical diagnostic twice (two identical PR annotations).
    #[test]
    fn a_repeated_blocker_is_reported_once() {
        let mut a = t("A", Status::Todo);
        a.blocked_by = vec!["GHOST".into(), "GHOST".into()];
        let r = validate(&plan_of(vec![a]), &opts());
        let n = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::UNKNOWN_DEP)
            .count();
        assert_eq!(n, 1, "one broken edge, one diagnostic: {:?}", r.diagnostics);
    }

    #[test]
    fn duplicate_id_is_an_error() {
        // Two *distinct* paths on purpose. With both tasks at the same file the
        // message names it either way, so the fixture cannot tell "names the
        // first definition" from "names whichever it happened to hold", which is
        // the shape of defect AGENTS.md's naming convention is about.
        let plan = Plan {
            tasks: vec![
                LoadedTask {
                    task: t("A", Status::Todo),
                    path: PathBuf::from("plan/tasks/first.md"),
                    fields: crate::load::FieldLines::new(),
                    criteria: crate::load::Criteria::default(),
                },
                LoadedTask {
                    task: t("A", Status::Todo),
                    path: PathBuf::from("plan/tasks/second.md"),
                    fields: crate::load::FieldLines::new(),
                    criteria: crate::load::Criteria::default(),
                },
            ],
            load_diagnostics: Vec::new(),
            unloadable: BTreeMap::new(),
        };
        let r = validate(&plan, &opts());
        assert!(r.diagnostics.iter().any(|d| d.code == code::DUPLICATE_ID));
        let d = pick(&r, code::DUPLICATE_ID);
        assert!(
            d.message.contains("duplicate task id") && d.message.contains("first.md"),
            "the message must name where the *first* definition was, or the reader \
             has two files and no idea which to open: {}",
            d.message
        );
    }

    /// Regression: the `DONE_BEFORE_BLOCKER` message once lowercased the whole
    /// string to normalise the status, mangling the task id (`C-001` ->
    /// `c-001`). Pin the casing.
    #[test]
    fn done_before_blocker_preserves_task_id_case() {
        let mut a = t("D-001", Status::Done);
        a.evidence = vec![Evidence {
            kind: EvidenceKind::Commit,
            reference: "abc1234".into(),
            captured: None,
        }];
        a.blocked_by = vec!["C-001".into()];
        let b = t("C-001", Status::Todo);
        let r = validate(&plan_of(vec![a, b]), &opts());
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::DONE_BEFORE_BLOCKER)
            .expect("diagnostic raised");
        assert!(d.message.contains("C-001"), "message: {}", d.message);
        assert!(
            !d.message.contains("c-001"),
            "id was lowercased: {}",
            d.message
        );
        assert!(
            d.message.contains("todo"),
            "status not rendered: {}",
            d.message
        );
    }

    /// A cycle longer than one hop must be reported with its full path.
    #[test]
    fn multi_node_cycle_is_reported_with_path() {
        let mut first = t("A", Status::Todo);
        let mut second = t("B", Status::Todo);
        let mut third = t("C", Status::Todo);
        first.blocked_by = vec!["B".into()];
        second.blocked_by = vec!["C".into()];
        third.blocked_by = vec!["A".into()];
        let report = validate(&plan_of(vec![first, second, third]), &opts());
        let diag = report
            .diagnostics
            .iter()
            .find(|d| d.code == code::CYCLE)
            .expect("cycle reported");
        for id in ["A", "B", "C"] {
            assert!(
                diag.message.contains(id),
                "cycle path incomplete: {}",
                diag.message
            );
        }
    }

    /// `APLAN032`: an unusable `paths` glob is this task's finding (`A1-066`).
    ///
    /// Hermetic on purpose. The ladder used to compile these and return an error
    /// that aborted the run, so a second task's `APLAN005` went unreported and
    /// the exit code said "configuration error" about a task file.
    #[test]
    fn a_paths_glob_that_will_not_compile_is_reported_against_its_task() {
        let mut a = t("A", Status::Todo);
        a.paths = vec!["src/[".into()];
        let r = validate(&plan_of(vec![a]), &opts());
        let d = pick(&r, code::BAD_PATH_GLOB);
        assert_eq!(d.task.as_deref(), Some("A"), "attributed to its own task");
        assert!(
            d.message.contains("src/["),
            "the message must name the entry that does not compile: {}",
            d.message
        );
        assert!(
            d.message.contains("unclosed character class"),
            "and why, in the compiler's own words: {}",
            d.message
        );
        assert!(
            !r.is_clean(),
            "a task no path-based rule can judge must fail the build"
        );
    }

    /// The near-miss, so the rule is about compilation and not about `[`.
    #[test]
    fn a_closed_character_class_is_a_glob_this_tool_can_use() {
        let mut a = t("A", Status::Todo);
        a.paths = vec!["src/[a-z]*.rs".into(), "src/**".into()];
        let r = validate(&plan_of(vec![a]), &opts());
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::BAD_PATH_GLOB),
            "a valid glob was reported: {:?}",
            r.diagnostics
        );
    }

    /// One task's typo must not cost another task its finding, which is the
    /// whole of `A1-066` stated as a property.
    #[test]
    fn a_bad_glob_in_one_task_does_not_hide_a_finding_in_another() {
        let mut a = t("A", Status::Todo);
        a.paths = vec!["src/[".into()];
        let b = t("B", Status::Done); // `done` with no evidence: APLAN005.
        let r = validate(&plan_of(vec![a, b]), &opts());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::DONE_WITHOUT_EVIDENCE && d.task.as_deref() == Some("B")),
            "the other task's finding went missing: {:?}",
            r.diagnostics
        );
    }

    /// `APLAN033`: the four shapes of a preference that cannot be honoured
    /// (`A1-150`).
    #[test]
    fn a_preference_that_cannot_be_honoured_is_reported() {
        // Unknown id.
        let mut a = t("A", Status::Todo);
        a.prefer_after = vec!["GHOST".into()];
        let r = validate(&plan_of(vec![a]), &opts());
        let d = pick(&r, code::UNUSABLE_PREFERENCE);
        assert!(d.message.contains("GHOST"), "{}", d.message);

        // Itself.
        let mut a = t("A", Status::Todo);
        a.prefer_after = vec!["A".into()];
        let r = validate(&plan_of(vec![a]), &opts());
        assert!(
            pick(&r, code::UNUSABLE_PREFERENCE)
                .message
                .contains("itself")
        );

        // A pair that is already a dependency: the plan would say two things.
        let mut a = t("A", Status::Todo);
        a.blocked_by = vec!["B".into()];
        a.prefer_after = vec!["B".into()];
        let b = t("B", Status::Todo);
        let r = validate(&plan_of(vec![a, b]), &opts());
        assert!(
            pick(&r, code::UNUSABLE_PREFERENCE)
                .message
                .contains("already a `blocked_by` edge")
        );

        // A cycle, reported once, naming the ring.
        let mut a = t("A", Status::Todo);
        a.prefer_after = vec!["B".into()];
        let mut b = t("B", Status::Todo);
        b.prefer_after = vec!["A".into()];
        let r = validate(&plan_of(vec![a, b]), &opts());
        let rings: Vec<&Diagnostic> = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::UNUSABLE_PREFERENCE)
            .collect();
        assert_eq!(rings.len(), 1, "one defect, one finding: {rings:?}");
        assert!(rings[0].message.contains("preference cycle"), "{rings:?}");
        assert!(!r.is_clean(), "an order nobody can act on must fail");
    }

    /// And the shape that is fine, so the rule is about *unusable* preferences
    /// rather than about the field existing.
    #[test]
    fn an_ordinary_preference_is_silent() {
        let mut a = t("A", Status::Todo);
        a.prefer_after = vec!["B".into()];
        let b = t("B", Status::Todo);
        let r = validate(&plan_of(vec![a, b]), &opts());
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::UNUSABLE_PREFERENCE),
            "{:?}",
            r.diagnostics
        );
        assert!(r.is_clean());
    }

    /// `APLAN034`: a trace that leaves the repository is refused unless the
    /// repository declares the root (`A1-143`).
    #[test]
    fn a_trace_that_leaves_the_repository_is_refused_unless_declared() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let repo = dir.path().join("repo");
        let sibling = dir.path().join("sibling");
        std::fs::create_dir_all(&repo).expect("repo");
        std::fs::create_dir_all(&sibling).expect("sibling");
        std::fs::write(sibling.join("DOC.md"), "# Doc\n\n## A section\n\ntext\n").expect("doc");

        let mut a = t("A", Status::Todo);
        a.traces = vec!["../sibling/DOC.md#a-section".to_string()];
        let plan = plan_of(vec![a]);

        let mut undeclared = opts();
        undeclared.traces_root = Some(repo.clone());
        let r = validate(&plan, &undeclared);
        let d = pick(&r, code::EXTERNAL_TRACE);
        assert!(
            d.message.contains("../sibling/DOC.md") && d.message.contains("traces_external"),
            "the message must name the target and the way to declare it: {}",
            d.message
        );
        // And `APLAN008` is not also raised: the target resolves perfectly, which
        // is the whole point of this rule existing beside that one.
        assert!(
            !r.diagnostics.iter().any(|x| x.code == code::DANGLING_TRACE),
            "{:?}",
            r.diagnostics
        );

        // Declared: it resolves, and nothing is reported.
        let mut declared = opts();
        declared.traces_root = Some(repo.clone());
        declared.traces_external = vec![sibling.clone()];
        let r = validate(&plan, &declared);
        assert!(r.is_clean(), "a declared root was still refused: {r:?}");

        // Declared, and the anchor is wrong: `APLAN008` speaks again, so
        // declaring a root buys resolution rather than silence.
        let mut bad = t("B", Status::Todo);
        bad.traces = vec!["../sibling/DOC.md#no-such-heading".to_string()];
        let r = validate(&plan_of(vec![bad]), &declared);
        assert_eq!(pick(&r, code::DANGLING_TRACE).task.as_deref(), Some("B"));
    }

    /// A symbolic link inside the repository pointing out of it (`A1-143`, found
    /// by its audit).
    ///
    /// The lexical test says this is inside, because lexically it is. A task file
    /// and the link it follows can arrive in the same fork pull request, so the
    /// rule follows the link where the target exists.
    #[test]
    fn a_link_out_of_the_repository_is_outside_it() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let repo = dir.path().join("repo");
        let secret = dir.path().join("secret");
        std::fs::create_dir_all(repo.join("docs")).expect("repo");
        std::fs::create_dir_all(&secret).expect("secret");
        std::fs::write(secret.join("DOC.md"), "# D\n\n## A section\n\nx\n").expect("doc");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, repo.join("docs/link")).expect("link");
        #[cfg(not(unix))]
        return;

        let mut o = opts();
        o.traces_root = Some(repo.clone());
        o.config_file = Some(repo.join(".aplan.yml"));
        let mut a = t("A", Status::Todo);
        a.traces = vec!["docs/link/DOC.md#a-section".to_string()];
        let r = validate(&plan_of(vec![a]), &o);
        let d = pick(&r, code::EXTERNAL_TRACE);
        assert!(
            d.message.contains("symbolic link"),
            "the message must say which of the two tests refused it: {}",
            d.message
        );
    }

    /// The boundary is the repository, not `traces_root` (`A1-143`, audit).
    ///
    /// Under `traces_root: docs`, a trace of `../README.md` climbs out of the
    /// root and stays inside the repository. Refusing it would be this rule
    /// answering a different question from the one it is named for.
    #[test]
    fn a_trace_that_climbs_out_of_the_traces_root_stays_inside_the_repository() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).expect("docs");
        std::fs::write(dir.path().join("README.md"), "# R\n\n## A section\n\nx\n").expect("doc");

        let mut o = opts();
        o.traces_root = Some(docs);
        o.traces_repository = Some(dir.path().to_path_buf());
        let mut a = t("A", Status::Todo);
        a.traces = vec!["../README.md#a-section".to_string()];
        let r = validate(&plan_of(vec![a]), &o);
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::EXTERNAL_TRACE),
            "a trace inside the repository was called external: {:?}",
            r.diagnostics
        );
    }

    /// The config is a diagnostic location and never the repository boundary
    /// (`A1-237`). A trusted policy outside the tree used to make an ordinary
    /// trace look external because these two values were conflated.
    /// Proves the named behaviour `traces.boundary_is_plan_repository`.
    #[test]
    fn a_selected_config_does_not_become_the_trace_repository() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let repo = dir.path().join("candidate");
        let trusted = dir.path().join("trusted");
        std::fs::create_dir_all(repo.join("docs")).expect("repo");
        std::fs::create_dir_all(&trusted).expect("trusted");
        std::fs::write(repo.join("docs/DOC.md"), "# Doc\n\n## One\n\nx\n").expect("doc");

        let mut o = opts();
        o.traces_root = Some(repo.clone());
        o.traces_repository = Some(repo);
        o.config_file = Some(trusted.join("policy.yml"));
        let mut a = t("A", Status::Todo);
        a.traces = vec!["docs/DOC.md#one".to_string()];
        let r = validate(&plan_of(vec![a]), &o);
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::EXTERNAL_TRACE),
            "a trace inside the plan repository was called external: {:?}",
            r.diagnostics
        );
    }

    /// The near-miss that decides whether the rule is worth having: an ordinary
    /// in-repository trace, including one that climbs and comes back.
    #[test]
    fn an_in_repository_trace_is_untouched_however_it_is_spelled() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).expect("docs");
        std::fs::write(docs.join("DOC.md"), "# Doc\n\n## A section\n\ntext\n").expect("doc");

        let mut o = opts();
        o.traces_root = Some(dir.path().to_path_buf());
        for spelling in [
            "docs/DOC.md#a-section",
            "./docs/DOC.md#a-section",
            "docs/../docs/DOC.md#a-section",
        ] {
            let mut a = t("A", Status::Todo);
            a.traces = vec![spelling.to_string()];
            let r = validate(&plan_of(vec![a]), &o);
            assert!(
                !r.diagnostics.iter().any(|d| d.code == code::EXTERNAL_TRACE),
                "`{spelling}` was read as leaving the repository: {:?}",
                r.diagnostics
            );
        }
    }

    /// An absolute target is outside by construction, and this is the half that
    /// is about a task file arriving from a fork rather than about a fleet.
    #[test]
    fn an_absolute_trace_target_is_outside_the_repository() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let mut o = opts();
        o.traces_root = Some(dir.path().to_path_buf());
        let mut a = t("A", Status::Todo);
        a.traces = vec!["/etc/passwd#anything".to_string()];
        let r = validate(&plan_of(vec![a]), &o);
        assert_eq!(pick(&r, code::EXTERNAL_TRACE).task.as_deref(), Some("A"));
    }

    /// `APLAN021` counts live work, not entries (`A1-134`).
    /// Proves the named behaviour `milestone.members_are_live_work` (`A1-211`).
    #[test]
    fn a_cut_emptied_of_work_is_reported_and_says_which_case_it_is() {
        // Sequenced: the later cut keeps its predecessor in `blocked_by`, which
        // is why the empty test was silent here.
        let mut m0 = t("M-s0", Status::Todo);
        m0.tags = vec!["milestone".into()];
        m0.blocked_by = vec!["A-1".into()];
        m0.effort_days = None;
        let mut m1 = t("M-s1", Status::Todo);
        m1.tags = vec!["milestone".into()];
        m1.blocked_by = vec!["M-s0".into()];
        m1.effort_days = None;
        let a = t("A-1", Status::Todo);
        let r = validate(&plan_of(vec![m0.clone(), m1.clone(), a.clone()]), &opts());
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::MILESTONE_HYGIENE && d.task.as_deref() == Some("M-s1"))
            .expect("the emptied cut is reported");
        assert!(d.message.contains("only other cuts"), "{}", d.message);
        assert!(
            d.agent_instruction
                .as_deref()
                .is_some_and(|f| f.contains("aplan mv")),
            "the fix must fit the case: {:?}",
            d.agent_instruction
        );

        // The other way to empty a cut: every task in it is cancelled. `A1-081`
        // exempts that from `APLAN020` deliberately, so this is the only rule
        // that can say it.
        let mut m2 = t("M-s2", Status::Todo);
        m2.tags = vec!["milestone".into()];
        m2.blocked_by = vec!["A-2".into()];
        m2.effort_days = None;
        let mut dropped = t("A-2", Status::Dropped);
        dropped.effort_days = None;
        let r = validate(&plan_of(vec![m2, dropped]), &opts());
        let d = pick(&r, code::MILESTONE_HYGIENE);
        assert!(
            d.message.contains("every task it lists is `dropped`"),
            "{}",
            d.message
        );

        // And the near-miss: the same sequenced pair with work in it is silent.
        let mut m1_with_work = m1;
        m1_with_work.blocked_by = vec!["M-s0".into(), "A-3".into()];
        let a3 = t("A-3", Status::Todo);
        let r = validate(&plan_of(vec![m0, m1_with_work, a, a3]), &opts());
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::MILESTONE_HYGIENE),
            "a cut with work in it was reported: {:?}",
            r.diagnostics
        );
    }

    /// The two exemptions the audit found (`A1-134`).
    #[test]
    fn a_cancelled_cut_and_an_unknown_member_are_not_this_rules_business() {
        // A cancelled cut holding cancelled work is what `aplan drop --cut`
        // produces, and `APLAN027` owns the mirror. Reporting it would advise
        // cancelling a cut that is already cancelled.
        let mut cut = t("M-s0", Status::Dropped);
        cut.tags = vec!["milestone".into()];
        cut.blocked_by = vec!["A-1".into()];
        cut.effort_days = None;
        let mut member = t("A-1", Status::Dropped);
        member.effort_days = None;
        let r = validate(&plan_of(vec![cut, member]), &opts());
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::MILESTONE_HYGIENE),
            "a cancelled cut was told to cancel itself: {:?}",
            r.diagnostics
        );

        // A member that does not exist is `APLAN003`'s finding. Before this,
        // `all()` over no known members was vacuously true and this rule
        // announced that the cut named only other cuts.
        let mut ghosts = t("M-s9", Status::Todo);
        ghosts.tags = vec!["milestone".into()];
        ghosts.blocked_by = vec!["GHOST".into()];
        ghosts.effort_days = None;
        let r = validate(&plan_of(vec![ghosts]), &opts());
        assert!(
            r.diagnostics.iter().any(|d| d.code == code::UNKNOWN_DEP),
            "the unknown member must still be reported: {:?}",
            r.diagnostics
        );
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::MILESTONE_HYGIENE),
            "one defect, two sentences: {:?}",
            r.diagnostics
        );
    }

    /// A release is a checkpoint over cuts, and the rule knows the difference
    /// (`A1-146`, ADR-0022).
    ///
    /// The state is identical to a cut emptied of work -- a milestone whose
    /// `blocked_by` names only milestones -- and the intent is the opposite, so
    /// the tag is how an author says which they meant.
    /// Proves the named behaviour `milestone.release_holds_cuts` (`A1-211`).
    #[test]
    fn a_release_holds_cuts_and_an_emptied_cut_still_reports() {
        let cut = |id: &str, members: &[&str], tags: &[&str]| {
            let mut m = t(id, Status::Todo);
            m.tags = tags.iter().map(|s| (*s).to_string()).collect();
            m.blocked_by = members.iter().map(|s| (*s).to_string()).collect();
            m.effort_days = None;
            m
        };
        let work = t("A-1", Status::Todo);
        let s0 = cut("M-s0", &["A-1"], &["milestone", "s0"]);
        let release = cut("R-v0.1", &["M-s0"], &["milestone", "release", "v0.1"]);
        let r = validate(&plan_of(vec![work.clone(), s0.clone(), release]), &opts());
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::MILESTONE_HYGIENE),
            "a release was reported as a cut with no work: {:?}",
            r.diagnostics
        );

        // The same shape without the tag is still the emptied cut `A1-134`
        // reports: the marker is the whole difference.
        let untagged = cut("M-s1", &["M-s0"], &["milestone", "s1"]);
        let r = validate(&plan_of(vec![work.clone(), s0.clone(), untagged]), &opts());
        assert_eq!(
            pick(&r, code::MILESTONE_HYGIENE).task.as_deref(),
            Some("M-s1")
        );

        // A release holding work directly is the second grouping axis ADR-0014
        // refused, and it says so in its own words.
        let holds_work = cut("R-bad", &["A-1"], &["milestone", "release", "vbad"]);
        let r = validate(&plan_of(vec![work, s0, holds_work]), &opts());
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::MILESTONE_HYGIENE && d.task.as_deref() == Some("R-bad"))
            .expect("reported");
        assert!(d.message.contains("holds `A-1` directly"), "{}", d.message);
        assert!(
            d.agent_instruction
                .as_deref()
                .is_some_and(|f| f.contains("checkpoint over cuts")),
            "the fix must be the release's own: {:?}",
            d.agent_instruction
        );
    }

    /// A finished cut is not an empty one: `done` members are members
    /// (`A1-134`).
    #[test]
    fn a_cut_whose_work_is_finished_is_not_reported_as_empty() {
        let mut m = t("M-s0", Status::Todo);
        m.tags = vec!["milestone".into()];
        m.blocked_by = vec!["A-1".into()];
        m.effort_days = None;
        let mut done_task = t("A-1", Status::Done);
        done_task.evidence = vec![commit_ev()];
        done_task.paths = vec!["src/**".into()];
        let r = validate(&plan_of(vec![m, done_task]), &opts());
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::MILESTONE_HYGIENE),
            "a finished cut was called empty: {:?}",
            r.diagnostics
        );
    }

    #[test]
    fn missing_effort_is_only_a_warning() {
        let mut a = t("A", Status::Todo);
        a.effort_days = None;
        let r = validate(&plan_of(vec![a]), &opts());
        assert!(r.diagnostics.iter().any(|d| d.code == code::MISSING_EFFORT));
        let d = pick(&r, code::MISSING_EFFORT);
        assert!(d.message.contains("`effort_days`"), "{}", d.message);
        assert!(r.is_clean(), "missing effort must not fail the build");
    }

    /// APLAN023: a `done` task's configuration-independent locality can be
    /// path-checked.
    fn uncheckable(tasks: Vec<Task>) -> Vec<Diagnostic> {
        validate(&plan_of(tasks), &opts())
            .diagnostics
            .into_iter()
            .filter(|d| d.code == code::UNCHECKABLE_EVIDENCE)
            .collect()
    }

    #[test]
    fn uncheckable_evidence_is_only_a_warning() {
        let mut a = t("A", Status::Done);
        a.evidence = vec![commit_ev()];
        let r = validate(&plan_of(vec![a]), &opts());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::UNCHECKABLE_EVIDENCE)
        );
        let d = pick(&r, code::UNCHECKABLE_EVIDENCE);
        assert!(
            d.message.contains("commit") && d.message.contains("`paths`"),
            "the message must name the kind cited and the field that is missing: {}",
            d.message
        );
        assert!(
            r.is_clean(),
            "an unbindable citation must not fail the build"
        );
    }

    #[test]
    fn uncheckable_evidence_scope_comes_from_the_capability_model() {
        for kind in EvidenceKind::ALL {
            let mut a = t("A", Status::Done);
            a.evidence = vec![Evidence {
                kind: *kind,
                reference: "citation".into(),
                captured: (*kind == EvidenceKind::Note).then(|| "2026-10-09".into()),
            }];
            assert_eq!(
                !uncheckable(vec![a]).is_empty(),
                kind.capabilities().locality.is_configuration_independent(),
                "{} must follow its declared locality availability",
                kind.as_str()
            );
        }
    }

    #[test]
    fn uncheckable_evidence_names_both_kinds_and_agrees_with_itself() {
        let mut a = t("A", Status::Done);
        a.evidence = vec![
            Evidence {
                kind: EvidenceKind::File,
                reference: "README.md".into(),
                captured: None,
            },
            commit_ev(),
            // A second commit must not make it "commit and commit".
            commit_ev(),
        ];
        let d = uncheckable(vec![a]);
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(
            d[0].message.contains("commit and file evidence"),
            "{}",
            d[0].message
        );
        assert!(
            d[0].message.contains("citations resolve"),
            "{}",
            d[0].message
        );
        assert!(
            d[0].message
                .ends_with("resolution alone is not evidence that the task's files changed"),
            "the message must not reduce commit resolution to object existence: {}",
            d[0].message
        );
    }

    #[test]
    fn a_blank_path_entry_does_not_silence_uncheckable_evidence() {
        // `paths: [""]` binds no commit that `paths: []` would not, and a lone
        // zero-width space looks in every editor exactly like a declared path.
        for blank in ["", "   ", "\u{200B}"] {
            let mut a = t("A", Status::Done);
            a.evidence = vec![commit_ev()];
            a.paths = vec![blank.to_string()];
            let d = uncheckable(vec![a]);
            assert_eq!(
                d.len(),
                1,
                "a blank path must not count as a binding: {blank:?}"
            );
            assert!(
                d[0].message.contains("no `paths` that name anything"),
                "the message must not claim `paths` is absent when it is blank: {}",
                d[0].message
            );
        }
    }

    #[test]
    fn uncheckable_evidence_only_promises_the_commit_check_when_a_commit_is_cited() {
        // The instruction is what an agent acts on without reading the message,
        // so it must not tell a task citing only a `file` that `verify` will check
        // "the cited commit" against the paths it is being asked to add.
        let file_ev = Evidence {
            kind: EvidenceKind::File,
            reference: "README.md".into(),
            captured: None,
        };
        let mut file_only = t("A", Status::Done);
        file_only.evidence = vec![file_ev.clone()];
        let d = uncheckable(vec![file_only]);
        let instruction = d[0].agent_instruction.as_deref().unwrap_or_default();
        assert!(
            !instruction.contains("commit"),
            "no commit is cited here: {instruction}"
        );

        let mut both = t("A", Status::Done);
        both.evidence = vec![file_ev, commit_ev()];
        let d = uncheckable(vec![both]);
        assert!(
            d[0].agent_instruction
                .as_deref()
                .unwrap_or_default()
                .contains("APLAN012"),
            "a cited commit is checkable, and the instruction should say so"
        );
    }

    #[test]
    fn uncheckable_evidence_is_silent_where_it_should_be() {
        let bound = {
            let mut a = t("A", Status::Done);
            a.evidence = vec![commit_ev()];
            a.paths = vec!["src/**".into()];
            a
        };
        let unfinished = {
            // Not `done`: a task that has not claimed anything yet is not lying,
            // and warning here would teach `paths: ["**"]` as the way to quiet it.
            let mut a = t("B", Status::InProgress);
            a.evidence = vec![commit_ev()];
            a
        };
        let not_path_checkable = {
            // `note` and `url` are not checked against `paths` by anything, so
            // their being unbindable is not this rule's finding.
            let mut a = t("C", Status::Done);
            a.evidence = vec![
                note_ev(),
                Evidence {
                    kind: EvidenceKind::Url,
                    reference: "https://example.invalid/proof".into(),
                    captured: None,
                },
            ];
            a
        };
        let configured_locality = {
            // Test locality uses runner coverage. This hermetic rule has no
            // runner to ask, so it must not claim a check was lost.
            let mut a = t("T", Status::Done);
            a.evidence = vec![Evidence {
                kind: EvidenceKind::Test,
                reference: "suite::case".into(),
                captured: None,
            }];
            a
        };
        let vacuous = {
            // An empty `ref` is `APLAN005`'s business; two diagnostics for one
            // defect would just be noise.
            let mut a = t("D", Status::Done);
            a.evidence = vec![
                Evidence {
                    kind: EvidenceKind::Commit,
                    reference: "  ".into(),
                    captured: None,
                },
                note_ev(),
            ];
            a
        };
        let milestone = {
            // `APLAN007` refuses to mark a milestone `done` while a member is
            // undone, which proves more than a path check could, and `APLAN021`
            // forbids it carrying `paths` at all: warning would be unsatisfiable.
            let mut m = t("M", Status::Done);
            m.tags = vec!["milestone".into()];
            m.blocked_by = vec!["A".into()];
            m.evidence = vec![commit_ev()];
            m
        };
        let d = uncheckable(vec![
            bound,
            unfinished,
            not_path_checkable,
            configured_locality,
            vacuous,
            milestone,
        ]);
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn a_milestone_without_effort_is_not_warned() {
        // A milestone is a checkpoint, not work, so `APLAN009` must not ask it for
        // an estimate, even though it is non-terminal and has no `effort_days`.
        let mut m = t("M", Status::Todo);
        m.effort_days = None;
        m.tags = vec!["milestone".into()];
        m.blocked_by = vec!["X".into()];
        let x = t("X", Status::Todo);
        let r = validate(&plan_of(vec![m, x]), &opts());
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::MISSING_EFFORT),
            "a milestone must be exempt from the effort-estimate warning"
        );
    }

    #[test]
    fn milestone_hygiene_flags_mis_shaped_milestones() {
        // APLAN021: a well-formed milestone (has a member, no effort, no paths) is
        // silent; an empty cut or a work task mis-tagged `milestone` fires.
        let x = t("X", Status::Todo);

        let mut good = t("M", Status::Todo);
        good.tags = vec!["milestone".into()];
        good.blocked_by = vec!["X".into()];
        good.effort_days = None;
        let r = validate(&plan_of(vec![good, x.clone()]), &opts());
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::MILESTONE_HYGIENE),
            "a well-formed milestone must be silent"
        );

        // No members: the cut scopes no work.
        let mut empty = t("E", Status::Todo);
        empty.tags = vec!["milestone".into()];
        empty.effort_days = None;
        let r = validate(&plan_of(vec![empty]), &opts());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::MILESTONE_HYGIENE),
            "an empty cut must fire APLAN021"
        );
        let d = pick(&r, code::MILESTONE_HYGIENE);
        assert!(
            d.message.contains("no members"),
            "the rule has two reasons and the message must say which: {}",
            d.message
        );

        // Work carrying effort but tagged `milestone`: the mis-tag would otherwise
        // silently delete it from every work-view.
        let mut mistag = t("W", Status::Todo); // `t` sets effort_days = Some(1.0)
        mistag.tags = vec!["milestone".into()];
        mistag.blocked_by = vec!["X".into()];
        let r = validate(&plan_of(vec![mistag, x]), &opts());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::MILESTONE_HYGIENE),
            "work carrying effort but tagged `milestone` must fire APLAN021"
        );
        let d = pick(&r, code::MILESTONE_HYGIENE);
        assert!(
            d.message.contains("`effort_days`"),
            "the other reason, and it must not be reported as the first one: {}",
            d.message
        );
    }

    #[test]
    fn membership_totality_flags_orphans_and_double_membership() {
        // APLAN022 (opt-in): a task in exactly one cut is silent; a task in no cut
        // fires (unless `cross-cutting`); a task in two cuts fires.
        let w1 = t("W1", Status::Todo); // in one cut -> silent
        let orphan = t("O", Status::Todo); // in no cut -> fires
        let mut xcut = t("X", Status::Todo); // in no cut but cross-cutting -> silent
        xcut.tags = vec!["cross-cutting".into()];
        let dbl = t("D", Status::Todo); // in two cuts -> fires

        let mut m1 = t("M1", Status::Todo);
        m1.tags = vec!["milestone".into(), "v0.1".into()];
        m1.blocked_by = vec!["W1".into(), "D".into()];
        let mut m2 = t("M2", Status::Todo);
        m2.tags = vec!["milestone".into(), "v0.2".into()];
        m2.blocked_by = vec!["D".into()];

        let plan = plan_of(vec![w1, orphan, xcut, dbl, m1, m2]);
        let on = Options {
            require_cut_membership: true,
            require_effort: false,
            ..Default::default()
        };
        let fires: Vec<(String, String)> = validate(&plan, &on)
            .diagnostics
            .into_iter()
            .filter(|d| d.code == code::MEMBERSHIP_TOTALITY)
            .map(|d| (d.task.unwrap_or_default(), d.message))
            .collect();
        assert!(
            fires.iter().any(|(id, _)| id == "O"),
            "a task in no cut must fire: {fires:?}"
        );
        assert!(
            fires
                .iter()
                .any(|(id, m)| id == "D" && m.contains("2 cuts")),
            "a task in two cuts must fire: {fires:?}"
        );
        assert!(
            !fires.iter().any(|(id, _)| id == "W1" || id == "X"),
            "one-cut and cross-cutting tasks are silent: {fires:?}"
        );

        // Off by default: no APLAN022 without the opt-in.
        let off = Options {
            require_effort: false,
            ..Default::default()
        };
        assert!(
            !validate(&plan, &off)
                .diagnostics
                .iter()
                .any(|d| d.code == code::MEMBERSHIP_TOTALITY),
            "APLAN022 must be off unless require_cut_membership is set"
        );
    }

    #[test]
    fn dangling_trace_is_an_error() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let mut a = t("A", Status::Todo);
        a.traces = vec!["NOPE.md#x".into()];
        let o = Options {
            require_effort: false,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let r = validate(&plan_of(vec![a]), &o);
        assert!(r.diagnostics.iter().any(|d| d.code == code::DANGLING_TRACE));
    }

    /// `DOC.md#` is a typo, and the message must send the author to the trace
    /// rather than to the document (`A1-116`).
    #[test]
    fn an_empty_anchor_is_named_as_one_rather_than_as_a_missing_heading() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(dir.path().join("R.md"), "# Intro\n").expect("write");
        let mut a = t("A", Status::Todo);
        a.traces = vec!["R.md#".into()];
        let o = Options {
            require_effort: false,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let r = validate(&plan_of(vec![a]), &o);
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::DANGLING_TRACE)
            .expect("the finding");
        assert!(d.message.contains("ends in an empty anchor"), "{d:?}");
        let fix = d.agent_instruction.clone().unwrap_or_default();
        assert!(fix.contains("write `R.md` on its own"), "{fix}");
        // Never the old advice, which sent the author to add a heading whose
        // text slugifies to nothing.
        assert!(!fix.contains("slugifies"), "{fix}");
    }

    #[test]
    fn resolving_trace_anchor_is_clean() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(dir.path().join("R.md"), "# Intro\n\n## Delaware ICIS\n").expect("write");
        let mut a = t("A", Status::Todo);
        a.traces = vec!["R.md#delaware-icis".into()];
        let o = Options {
            require_effort: false,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let r = validate(&plan_of(vec![a]), &o);
        assert!(r.is_clean(), "{:?}", r.diagnostics);
    }

    /// APLAN018: a `done` task whose stamped traced section later changes fires a
    /// (advisory) warning; a matching stamp stays silent.
    #[test]
    fn trace_drift_fires_only_when_a_stamped_section_changes() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let doc = "# Intro\n\noriginal content\n";
        std::fs::write(dir.path().join("D.md"), doc).expect("write");
        let stamp = crate::anchor::anchor_fingerprint(doc, "intro").expect("fingerprint");

        let mut task = t("A", Status::Done);
        task.evidence = vec![commit_ev()];
        task.traces = vec!["D.md#intro".into()];
        task.trace_fingerprints = BTreeMap::from([("D.md#intro".to_string(), stamp)]);
        let on = Options {
            require_effort: false,
            detect_drift: true,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        // Stamp matches the live section: silent.
        assert!(
            !validate(&plan_of(vec![task.clone()]), &on)
                .diagnostics
                .iter()
                .any(|d| d.code == code::TRACE_DRIFT)
        );

        // The section changes; the stamp is now stale: fires, but advisory.
        std::fs::write(dir.path().join("D.md"), "# Intro\n\nCHANGED content\n").expect("write");
        let r = validate(&plan_of(vec![task]), &on);
        assert!(
            r.diagnostics.iter().any(|d| d.code == code::TRACE_DRIFT),
            "drift must fire: {:?}",
            r.diagnostics
        );
        let d = pick(&r, code::TRACE_DRIFT);
        assert!(
            d.message.contains("D.md#intro"),
            "the message must name the section that drifted: a task may trace \
             several, and the reader has to know which to re-review: {}",
            d.message
        );
        assert!(
            d.agent_instruction
                .as_deref()
                .is_some_and(|i| i.contains("--restamp")),
            "the fix must name the command that clears it: {:?}",
            d.agent_instruction
        );
        assert!(
            r.is_clean(),
            "APLAN018 is advisory, it must not fail `check`"
        );
    }

    /// `APLAN034` is asked first, so one cause does not produce two errors whose
    /// fixes disagree (`A1-143`'s precedence, `A1-297`'s audit).
    #[test]
    fn an_external_trace_is_not_also_asked_for_a_stamp() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let sib = dir.path().join("sib");
        std::fs::create_dir_all(&sib).expect("sib");
        std::fs::write(sib.join("S.md"), "# Intro\n\nprose\n").expect("write");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let outside = Options {
            require_effort: false,
            detect_drift: true,
            traces_root: Some(repo.clone()),
            config_file: Some(repo.join(".aplan.yml")),
            ..Default::default()
        };
        let task = || {
            let mut t = t("A-1", Status::Done);
            t.traces = vec!["../sib/S.md#intro".to_string()];
            t.evidence = vec![Evidence {
                kind: EvidenceKind::Commit,
                reference: "abc1234".into(),
                captured: None,
            }];
            t
        };

        // Undeclared: the author is being told to declare or drop this target,
        // and "stamp it" is the opposite instruction.
        let r = validate(&plan_of(vec![task()]), &outside);
        assert!(
            r.diagnostics.iter().any(|d| d.code == code::EXTERNAL_TRACE),
            "the rule that owns an undeclared external target must speak: {:?}",
            r.diagnostics
        );
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::UNSTAMPED_TRACE),
            "and it must not also be told to stamp what it is told to drop: {:?}",
            r.diagnostics
        );

        // Declared, so the target is legitimate and the stamp question is real
        // again. Without this the assertion above would pass on a rule that had
        // simply stopped looking at external traces at all.
        let declared = Options {
            traces_external: vec![sib.clone()],
            ..outside.clone()
        };
        let r = validate(&plan_of(vec![task()]), &declared);
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::UNSTAMPED_TRACE),
            "a declared external trace is stampable, so it is asked for: {:?}",
            r.diagnostics
        );
    }

    /// End to end through the rules: a task traces a YAML node, `APLAN008`
    /// resolves it, and `APLAN018` reports it when the value changes and stays
    /// quiet when only the layout does (`A1-191`, ADR-0035).
    ///
    /// Proves `traces.structured_documents`, with the resolver tests beside it.
    #[test]
    fn a_yaml_node_is_traceable_and_drifts_when_its_value_changes() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let spec = dir.path().join("spec.yaml");
        let write = |text: &str| std::fs::write(&spec, text).expect("write");
        write("states:\n  provisioning:\n    next: [ready, failed]\n    timeout_s: 300\n");
        let on = Options {
            require_effort: false,
            detect_drift: true,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let stamped = |fp: Option<&str>| {
            let mut t = t("A-1", Status::Done);
            t.traces = vec!["spec.yaml#/states/provisioning".to_string()];
            if let Some(fp) = fp {
                t.trace_fingerprints =
                    [("spec.yaml#/states/provisioning".to_string(), fp.to_string())]
                        .into_iter()
                        .collect();
            }
            t.evidence = vec![Evidence {
                kind: EvidenceKind::Commit,
                reference: "abc1234".into(),
                captured: None,
            }];
            t
        };

        // The address resolves, so the rule that refuses a dangling trace is
        // silent. Without this the drift assertions below could pass on a trace
        // that was never resolvable at all.
        let r = validate(&plan_of(vec![stamped(None)]), &on);
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::DANGLING_TRACE),
            "a YAML node address must resolve: {:?}",
            r.diagnostics
        );

        // The stamp `close --restamp` would take, through the same dispatch the
        // command uses, so this cannot pass on a fingerprint only the test knows.
        let fp = crate::close::trace_fingerprints(&stamped(None), Some(dir.path()));
        let fp = fp
            .get("spec.yaml#/states/provisioning")
            .expect("the node is stampable")
            .clone();

        let drifted = |plan: &Plan| {
            validate(plan, &on)
                .diagnostics
                .iter()
                .any(|d| d.code == code::TRACE_DRIFT)
        };
        assert!(!drifted(&plan_of(vec![stamped(Some(&fp))])), "unchanged");

        // Layout, key order and comments are not the value (ADR-0035).
        write(
            "states:\n    provisioning:\n        # a note\n        timeout_s: 300\n        next: [ready, failed]\n",
        );
        assert!(
            !drifted(&plan_of(vec![stamped(Some(&fp))])),
            "reindenting, reordering keys and adding a comment are not drift"
        );

        // A node the task does not address changing is not its drift either.
        write(
            "states:\n  provisioning:\n    next: [ready, failed]\n    timeout_s: 300\nother:\n  k: 1\n",
        );
        assert!(
            !drifted(&plan_of(vec![stamped(Some(&fp))])),
            "an unrelated node changed, which this task never traced"
        );

        // The value it does address changing is.
        write("states:\n  provisioning:\n    next: [ready, failed]\n    timeout_s: 600\n");
        assert!(
            drifted(&plan_of(vec![stamped(Some(&fp))])),
            "the addressed value changed and nothing said so"
        );
    }

    /// A1-322's deterministic performance measurement: 200 stamped traces over
    /// two documents still perform exactly two reads and one parse of each
    /// format. Both alternate lexical spellings canonicalize to the same
    /// document, so the count measures document identity rather than authored
    /// string identity.
    #[test]
    fn each_trace_document_is_read_and_parsed_once_per_validation_snapshot() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir(dir.path().join("sub")).expect("alias directory");
        let markdown = "# Intro\n\nThe fact.\n";
        let structured = "states:\n  provisioning:\n    timeout_s: 300\n";
        std::fs::write(dir.path().join("D.md"), markdown).expect("markdown");
        std::fs::write(dir.path().join("spec.yaml"), structured).expect("yaml");
        let markdown_stamp = crate::anchor::anchor_fingerprint(markdown, "intro").expect("stamp");
        let structured_stamp =
            crate::anchor::fingerprint_in("spec.yaml", structured, "/states/provisioning")
                .expect("stamp");

        let tasks = (0..200)
            .map(|number| {
                let mut task = t(&format!("T-{number:03}"), Status::Done);
                task.evidence = vec![commit_ev()];
                let (trace, stamp) = if number % 2 == 0 {
                    let trace = if number % 4 == 0 {
                        "D.md#intro"
                    } else {
                        "sub/../D.md#intro"
                    };
                    (trace, markdown_stamp.as_str())
                } else {
                    let trace = if number % 4 == 1 {
                        "spec.yaml#/states/provisioning"
                    } else {
                        "sub/../spec.yaml#/states/provisioning"
                    };
                    (trace, structured_stamp.as_str())
                };
                task.traces = vec![trace.to_string()];
                task.trace_fingerprints = BTreeMap::from([(trace.to_string(), stamp.to_string())]);
                task
            })
            .collect();
        let options = Options {
            require_effort: false,
            detect_drift: true,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let before = trace_document_work();
        let report = validate(&plan_of(tasks), &options);
        let after = trace_document_work();

        for code in [
            code::DANGLING_TRACE,
            code::TRACE_DRIFT,
            code::UNSTAMPED_TRACE,
        ] {
            assert!(
                !report
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "{code} changed while caching documents: {:?}",
                report.diagnostics
            );
        }
        assert_eq!(after.reads - before.reads, 2);
        assert_eq!(after.markdown_parses - before.markdown_parses, 1);
        assert_eq!(after.structured_parses - before.structured_parses, 1);
        assert_eq!(
            after.bytes_read - before.bytes_read,
            u64::try_from(markdown.len() + structured.len()).expect("fixture byte count")
        );
    }

    /// A failed parse is cacheable work too. Otherwise a malformed shared
    /// catalogue would be parsed once per task while still satisfying the happy
    /// path measurement above, exactly when the author most needs a fast answer.
    #[test]
    fn an_invalid_structured_trace_document_is_parsed_once_without_hiding_findings() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let malformed = "states: [unterminated\n";
        std::fs::write(dir.path().join("spec.yaml"), malformed).expect("yaml");
        let tasks = (0..25)
            .map(|number| {
                let mut task = t(&format!("T-{number:03}"), Status::Done);
                task.evidence = vec![commit_ev()];
                let trace = "spec.yaml#/states/provisioning".to_string();
                task.traces = vec![trace.clone()];
                if number % 2 == 1 {
                    task.trace_fingerprints =
                        BTreeMap::from([(trace, "0000000000000000".to_string())]);
                }
                task
            })
            .collect();
        let options = Options {
            require_effort: false,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let before = trace_document_work();
        let report = validate(&plan_of(tasks), &options);
        let after = trace_document_work();

        assert_eq!(after.reads - before.reads, 1);
        assert_eq!(after.structured_parses - before.structured_parses, 1);
        assert_eq!(
            report
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == code::DANGLING_TRACE)
                .count(),
            25,
            "caching the failed parse must not coalesce task-scoped findings"
        );
    }

    /// A failed read is cached just like a failed parse, but is not reported as
    /// successful trace work. Exercise both the stampability and drift paths:
    /// neither may hide the task-scoped dangling-trace finding or retry the
    /// unavailable source for every task.
    #[test]
    fn an_unreadable_trace_document_is_cached_without_claiming_successful_work() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let tasks = (0..24)
            .map(|number| {
                let mut task = t(&format!("T-{number:03}"), Status::Done);
                task.evidence = vec![commit_ev()];
                let trace = "missing.md#section".to_string();
                task.traces = vec![trace.clone()];
                if number % 2 == 1 {
                    task.trace_fingerprints =
                        BTreeMap::from([(trace, "0000000000000000".to_string())]);
                }
                task
            })
            .collect();
        let options = Options {
            require_effort: false,
            detect_drift: true,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let before = trace_document_work();
        let report = validate(&plan_of(tasks), &options);
        let after = trace_document_work();

        assert_eq!(after, before, "failed reads are not successful trace work");
        assert_eq!(
            report
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == code::DANGLING_TRACE)
                .count(),
            24,
            "every affected task must retain its own dangling-trace finding"
        );
        assert!(
            report
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != code::TRACE_DRIFT),
            "an unreadable source cannot truthfully be called drift"
        );
    }

    /// `ratified_by` carries the same structural invariants `blocked_by` does
    /// (`A1-306`, audit).
    ///
    /// The field shipped with half of them: an unknown name and a self-reference
    /// were reported, a cycle and a cancelled endpoint were not. Both are
    /// deadlocks -- a set of tasks none of which can ever be closed, and a task
    /// whose blessing can never come -- and `blocked_by` reports both, so a
    /// reader would reasonably assume this did too.
    #[test]
    fn a_ratification_cannot_deadlock_silently() {
        let ratified = |id: &str, status: Status, by: &str| {
            let mut t = t(id, status);
            t.ratified_by = vec![by.to_string()];
            t
        };

        // A loop: each waits on the next to be finished before it may be
        // finished, so none of them ever can be.
        let r = validate(
            &plan_of(vec![
                ratified("A-1", Status::Todo, "A-2"),
                ratified("A-2", Status::Todo, "A-1"),
            ]),
            &opts(),
        );
        let d = pick(&r, code::CYCLE);
        assert!(
            d.message.contains("ratified_by"),
            "the message must say which edge closed the loop: {}",
            d.message
        );

        // Mixed: `blocked_by` one way, `ratified_by` the other. Both mean "that
        // one first", so the deadlock is the same and neither graph alone sees
        // it.
        let mut a = t("A-1", Status::Todo);
        a.blocked_by = vec!["A-2".to_string()];
        let r = validate(
            &plan_of(vec![a, ratified("A-2", Status::Todo, "A-1")]),
            &opts(),
        );
        assert!(
            r.diagnostics.iter().any(|d| d.code == code::CYCLE),
            "a cycle through one edge of each kind was not reported: {:?}",
            r.diagnostics
        );

        // A cancelled ratifier can never bless anything.
        let r = validate(
            &plan_of(vec![
                ratified("A-1", Status::Todo, "A-2"),
                t("A-2", Status::Dropped),
            ]),
            &opts(),
        );
        let d = pick(&r, code::DROPPED_BLOCKER);
        assert!(
            d.message.contains("ratifier") && d.message.contains("never legitimately be closed"),
            "it says the work is unclosable, not unstartable: {}",
            d.message
        );

        // And a healthy ratification is silent on all three.
        let r = validate(
            &plan_of(vec![
                ratified("A-1", Status::Todo, "A-2"),
                t("A-2", Status::Done),
            ]),
            &opts(),
        );
        for code in [code::CYCLE, code::DROPPED_BLOCKER, code::UNRATIFIED] {
            assert!(
                !r.diagnostics.iter().any(|d| d.code == code),
                "{code} spoke about a healthy ratification: {:?}",
                r.diagnostics
            );
        }
    }

    /// `APLAN046`: a ratification constrains counting done, never starting
    /// (`A1-306`, ADR-0036).
    ///
    /// The silences matter more than the finding here. The whole defect was that
    /// a ratification drawn as `blocked_by` made startable work look blocked, so
    /// a field that quietly did the same thing would be worthless.
    #[test]
    fn a_ratifier_gates_done_and_nothing_else() {
        let ratified = |status: Status, by: &str| {
            let mut t = t("A-1", status);
            t.ratified_by = vec![by.to_string()];
            if status == Status::Done {
                t.evidence = vec![Evidence {
                    kind: crate::task::EvidenceKind::Commit,
                    reference: "abc1234".into(),
                    captured: None,
                }];
            }
            t
        };
        let gov = |status: Status| t("A-2", status);

        // Done while the ratifier is not: the finding.
        let r = validate(
            &plan_of(vec![ratified(Status::Done, "A-2"), gov(Status::Todo)]),
            &opts(),
        );
        let d = pick(&r, code::UNRATIFIED);
        assert_eq!(
            d.severity,
            crate::diagnostic::Severity::Error,
            "the same tier `APLAN007` uses: a status making a claim the plan contradicts"
        );
        assert_eq!(
            d.message, "is `done` but its ratifier `A-2` is `todo`",
            "it names the ratifier and its status"
        );
        assert!(
            d.agent_instruction
                .as_deref()
                .is_some_and(|i| i.contains("says who signs off, not what this waits on")),
            "and says what the field means, since the confusion is the whole task: {:?}",
            d.agent_instruction
        );

        // **The point of the field**: the work is not blocked. A live task whose
        // ratifier is unfinished is ready, reported by nothing, and offered.
        let plan = plan_of(vec![ratified(Status::Todo, "A-2"), gov(Status::Todo)]);
        let r = validate(&plan, &opts());
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::UNRATIFIED),
            "a ratification must never report on work that has not claimed to be done"
        );
        let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
        let graph = Graph::new(tasks.iter().copied());
        assert!(
            graph.ready().iter().any(|t| t.id == "A-1"),
            "the work was startable and `ready` must say so: this is the two days the \
             sixth pilot lost"
        );

        // Ratifier done: silent.
        assert!(
            !validate(
                &plan_of(vec![ratified(Status::Done, "A-2"), gov(Status::Done)]),
                &opts()
            )
            .diagnostics
            .iter()
            .any(|d| d.code == code::UNRATIFIED),
            "a blessed task was reported anyway"
        );

        // A ratifier nothing defines cannot bless anything, and is reported the
        // way a `blocked_by` ghost is.
        let r = validate(&plan_of(vec![ratified(Status::Todo, "GHOST")]), &opts());
        let d = pick(&r, code::UNKNOWN_DEP);
        assert!(
            d.message
                .contains("`ratified_by` references unknown task `GHOST`"),
            "the message says which field named it: {}",
            d.message
        );

        // And nothing blesses itself.
        let r = validate(&plan_of(vec![ratified(Status::Todo, "A-1")]), &opts());
        assert!(
            r.diagnostics.iter().any(|d| d.code == code::SELF_DEP),
            "a self-ratification is a cycle of one: {:?}",
            r.diagnostics
        );
    }

    /// The exemption `A1-297` measured and removed, and the one case it did not
    /// widen into.
    #[test]
    fn a_closed_task_that_stamps_nothing_is_reported_but_an_unstampable_trace_is_not() {
        // Proves `traces.unstamped_is_reported`: the behaviour a consumer pins
        // with `--require`, because this widening adds no rule code, command,
        // config key, format or evidence kind for `facts` to report.
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(dir.path().join("D.md"), "# Intro\n\n| 7 | a |\n").expect("write");
        let on = Options {
            require_effort: false,
            detect_drift: true,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let task = |traces: Vec<&str>| {
            let mut t = t("A-1", Status::Done);
            t.traces = traces.into_iter().map(String::from).collect();
            t.evidence = vec![Evidence {
                kind: EvidenceKind::Commit,
                reference: "abc1234".into(),
                captured: None,
            }];
            t
        };

        // Stamping nothing is the same sentence about every trace, and used to be
        // exempt on the reasoning that a task closed before `detect_drift` was
        // enabled is unverifiable rather than in violation. Measured across 2,410
        // task files: the partial case occurs nowhere, this one nine times.
        let r = validate(&plan_of(vec![task(vec!["D.md#7"])]), &on);
        let d = pick(&r, code::UNSTAMPED_TRACE);
        assert!(
            d.message.contains("D.md#7") && d.message.contains("what it traces"),
            "stamping nothing reports the same sentence as stamping some: {}",
            d.message
        );
        assert!(
            !d.message.contains("no longer traces"),
            "and it has no orphan to name, so it must not invent a retarget: {}",
            d.message
        );
        assert!(!r.is_clean(), "an error must fail the build");

        // A trace `--restamp` cannot stamp is not reported, because the fix the
        // message names provably would not clear it: a stamp covers an anchored
        // trace that resolves, and a whole-file trace has no anchor. Whether it
        // should be reported at all is `A1-191`'s question.
        let r = validate(&plan_of(vec![task(vec!["D.md"])]), &on);
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::UNSTAMPED_TRACE),
            "an unstampable trace was reported with a fix that cannot work: {:?}",
            r.diagnostics
        );

        // An anchored trace whose document cannot be read is unstampable too, and
        // spelling the predicate out as "has an anchor" missed it: the MCP close
        // tool refused a task over an `APLAN028` that `--restamp` could not have
        // cleared. `APLAN008` owns this one, and reports it.
        let r = validate(&plan_of(vec![task(vec!["GONE.md#intro"])]), &on);
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::UNSTAMPED_TRACE),
            "a trace whose document cannot be read was reported as unstamped: {:?}",
            r.diagnostics
        );
        assert!(
            r.diagnostics.iter().any(|d| d.code == code::DANGLING_TRACE),
            "and the rule that does own it must still speak: {:?}",
            r.diagnostics
        );

        // An anchor that matches nothing in a document that reads: same answer,
        // same owner.
        let r = validate(&plan_of(vec![task(vec!["D.md#nope"])]), &on);
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::UNSTAMPED_TRACE),
            "a trace whose anchor resolves to nothing was reported as unstamped: {:?}",
            r.diagnostics
        );

        // And the mixture, which is what keeps the scoping honest rather than
        // merely quiet: the anchored trace is named, the whole-file one is not.
        let r = validate(&plan_of(vec![task(vec!["D.md", "D.md#7"])]), &on);
        let d = pick(&r, code::UNSTAMPED_TRACE);
        assert!(
            d.message.contains("D.md#7") && d.message.contains("what it traces"),
            "the stampable trace is named alone, in the singular: {}",
            d.message
        );
        assert!(
            !d.message.contains("`D.md`,") && !d.message.contains("what they trace"),
            "the unstampable one must not be named beside it: {}",
            d.message
        );
    }

    #[test]
    fn retargeting_a_trace_on_a_closed_task_is_an_error_not_a_silence() {
        // `A1-098`, the fourth pilot's item 16, reproduced. `trace_fingerprints`
        // is keyed by the trace string, so editing a `traces` entry on a `done`
        // task orphans the old key and leaves the new trace unstamped. Before
        // this rule the plan read clean, and the traced section could then change
        // with no diagnostic at all.
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(
            dir.path().join("D.md"),
            "# Intro\n\n| 7 | a |\n| 11 | b |\n",
        )
        .expect("write");
        let on = Options {
            require_effort: false,
            detect_drift: true,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let task = |traces: Vec<&str>, stamps: Vec<(&str, &str)>| {
            let mut t = t("A-1", Status::Done);
            t.traces = traces.into_iter().map(String::from).collect();
            t.trace_fingerprints = stamps
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            t.evidence = vec![Evidence {
                kind: EvidenceKind::Commit,
                reference: "abc1234".into(),
                captured: None,
            }];
            t
        };

        // Retargeted from the section to a row: the state the migration creates.
        let r = validate(
            &plan_of(vec![task(
                vec!["D.md#7"],
                vec![("D.md#intro", "deadbeefdeadbeef")],
            )]),
            &on,
        );
        let d = pick(&r, code::UNSTAMPED_TRACE);
        assert!(
            d.message.contains("D.md#7") && d.message.contains("D.md#intro"),
            "name the trace that lost its stamp *and* the one it no longer traces, \
             since the orphan is the only record of what it was closed against: {}",
            d.message
        );
        assert!(
            d.agent_instruction
                .as_deref()
                .is_some_and(|i| i.contains("--restamp") && i.contains("A-1")),
            "{:?}",
            d.agent_instruction
        );
        assert!(!r.is_clean(), "an error must fail the build");

        // Every unstamped trace, not the first. Naming one reads as the only one,
        // and an author repairing them one at a time is the slow version of what
        // `--restamp` does in a single pass.
        let r = validate(
            &plan_of(vec![task(
                vec!["D.md#7", "D.md#11"],
                vec![("D.md#intro", "deadbeefdeadbeef")],
            )]),
            &on,
        );
        let d = pick(&r, code::UNSTAMPED_TRACE);
        assert!(
            d.message.contains("D.md#7") && d.message.contains("D.md#11"),
            "both unstamped traces must be named: {}",
            d.message
        );
        assert!(
            d.message.contains("what they trace") && d.message.contains("for them"),
            "and the sentence must read at two, without a `(s)` construction: {}",
            d.message
        );

        // Every trace stamped: silent.
        let good = anchor_fp(dir.path(), "D.md", "7");
        let r = validate(
            &plan_of(vec![task(vec!["D.md#7"], vec![("D.md#7", &good)])]),
            &on,
        );
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::UNSTAMPED_TRACE),
            "{:?}",
            r.diagnostics
        );

        // A live task is mid-flight, not a closed record.
        let mut live = task(vec!["D.md#7"], vec![("D.md#intro", "deadbeefdeadbeef")]);
        live.status = Status::InProgress;
        let r = validate(&plan_of(vec![live]), &on);
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::UNSTAMPED_TRACE),
            "{:?}",
            r.diagnostics
        );
    }

    /// The live fingerprint of a section, for fixtures that need a matching stamp.
    fn anchor_fp(root: &Path, rel: &str, anchor: &str) -> String {
        let text = std::fs::read_to_string(root.join(rel)).expect("read");
        crate::anchor::anchor_fingerprint(&text, anchor).expect("resolves")
    }

    #[test]
    fn the_diff_command_names_the_path_from_the_repository() {
        // The default root is `.`, which is the case every adopter meets: the
        // command must read `docs/d.md`, not `./docs/d.md` and not `d.md`.
        assert_eq!(command_path(Path::new("."), "docs/d.md"), "docs/d.md");
        // A configured root is prepended, which is the whole point.
        assert_eq!(command_path(Path::new("docs"), "d.md"), "docs/d.md");
        // One the user made absolute stays absolute; git accepts it.
        assert_eq!(command_path(Path::new("/w/repo"), "d.md"), "/w/repo/d.md");
    }

    #[test]
    fn the_drift_fix_names_the_commit_to_diff_from_only_when_there_is_one() {
        // `A1-096`. The reader is otherwise told a section moved and left to work
        // out which of its lines did, often for a closure somebody else made.
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(dir.path().join("D.md"), "# Intro\n\nCHANGED\n").expect("write");
        let on = Options {
            require_effort: false,
            detect_drift: true,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let stamped = |evidence: Vec<Evidence>| {
            let mut t = t("A-1", Status::Done);
            t.traces = vec!["D.md#intro".into()];
            t.trace_fingerprints = [("D.md#intro".to_string(), "0000000000000000".to_string())]
                .into_iter()
                .collect();
            t.evidence = evidence;
            t
        };
        let commit = |r: &str| Evidence {
            kind: EvidenceKind::Commit,
            reference: r.to_string(),
            captured: None,
        };
        let fix = |task| {
            let r = validate(&plan_of(vec![task]), &on);
            pick(&r, code::TRACE_DRIFT)
                .agent_instruction
                .clone()
                .expect("drift carries an instruction")
        };

        // Exactly one: name it, so the reader has a command to paste.
        let one = fix(stamped(vec![commit("a1b2c3d")]));
        assert!(
            one.contains("git diff a1b2c3d -- ") && one.contains("D.md"),
            "the one commit the task cites is where to diff from: {one}"
        );

        // Several: say nothing about which, rather than picking. Nothing
        // distinguishes them, and a SHA in a sentence that cannot justify it is
        // the defect `APLAN024` shipped.
        let many = fix(stamped(vec![commit("a1b2c3d"), commit("e4f5a6b")]));
        assert!(
            !many.contains("git diff"),
            "with two citations there is no closing commit to name: {many}"
        );
        assert!(
            many.contains("Re-review the task against") && many.contains("D.md"),
            "and the instruction must still be usable: {many}"
        );

        // The path is the one a reader can paste. `traces_root` is configurable,
        // so a trace spelled `D.md#intro` under `traces_root: docs` is `docs/D.md`
        // from the repository, and naming the trace's own spelling would produce
        // a `git diff` that matches nothing and exits zero.
        let nested = dir.path().join("docs");
        std::fs::create_dir_all(&nested).expect("mkdir");
        std::fs::write(nested.join("D.md"), "# Intro\n\nCHANGED\n").expect("write");
        let deep = Options {
            traces_root: Some(dir.path().join("docs")),
            ..on.clone()
        };
        let r = validate(&plan_of(vec![stamped(vec![commit("a1b2c3d")])]), &deep);
        let i = pick(&r, code::TRACE_DRIFT)
            .agent_instruction
            .clone()
            .expect("instruction");
        assert!(
            i.contains("docs/D.md"),
            "the command must name the path from the repository, not the trace's \
             own spelling: {i}"
        );

        // None, and a blank ref, are the same case: no command, no invented SHA.
        for e in [vec![], vec![commit("   ")]] {
            let none = fix(stamped(e));
            assert!(!none.contains("git diff"), "no commit to diff from: {none}");
            assert!(none.contains("re-stamp"), "the remedy survives: {none}");
        }
    }

    /// APLAN018 stays silent without a stamp (unverifiable, not a violation) and
    /// when `detect_drift` is off.
    #[test]
    fn trace_drift_is_silent_without_a_stamp_or_when_off() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(dir.path().join("D.md"), "# Intro\n\ncontent\n").expect("write");
        let mut task = t("A", Status::Done);
        task.evidence = vec![commit_ev()];
        task.traces = vec!["D.md#intro".into()];

        let fires = |task: &Task, detect_drift: bool| {
            let o = Options {
                require_effort: false,
                detect_drift,
                traces_root: Some(dir.path().to_path_buf()),
                ..Default::default()
            };
            validate(&plan_of(vec![task.clone()]), &o)
                .diagnostics
                .iter()
                .any(|d| d.code == code::TRACE_DRIFT)
        };

        // No stamp, drift on: nothing to compare against, so silent.
        assert!(!fires(&task, true));
        // A stale stamp, but drift off: silent.
        task.trace_fingerprints = BTreeMap::from([("D.md#intro".to_string(), "stale".to_string())]);
        assert!(!fires(&task, false));
    }

    /// Regression: the README's documented `traces` example must resolve against
    /// the heading the README shows. The old `slugify` rejected it.
    #[test]
    fn readme_trace_example_resolves() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(
            dir.path().join("INITIAL_RESEARCH.md"),
            "# Research\n\n## 2.1 Delaware: the filing channel\n",
        )
        .expect("write");
        let mut a = t("A", Status::Todo);
        a.traces = vec!["INITIAL_RESEARCH.md#21-delaware-the-filing-channel".into()];
        let o = Options {
            require_effort: false,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let r = validate(&plan_of(vec![a]), &o);
        assert!(r.is_clean(), "{:?}", r.diagnostics);
    }

    #[test]
    fn duplicate_heading_resolves_by_suffix() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(dir.path().join("R.md"), "## Setup\n## Setup\n").expect("write");
        let o = Options {
            require_effort: false,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        // The second `Setup` is reachable only as `setup-1`.
        let mut ok = t("A", Status::Todo);
        ok.traces = vec!["R.md#setup-1".into()];
        assert!(validate(&plan_of(vec![ok]), &o).is_clean());
        // An anchor GitHub would never mint is still an error.
        let mut bad = t("B", Status::Todo);
        bad.traces = vec!["R.md#setup-2".into()];
        assert!(!validate(&plan_of(vec![bad]), &o).is_clean());
    }
    /// Checked against the values `date -u -d <s> +%s` gives, including the
    /// epoch itself, a leap day, and a date before 1970.
    #[test]
    fn an_iso_date_converts_to_the_epoch_second_git_would_use() {
        for (s, want) in [
            ("1970-01-01", 0_i64),
            ("2026-08-19", 1_787_097_600),
            ("2000-02-29", 951_782_400),
            ("1969-12-31", -86_400),
            ("2024-12-31", 1_735_603_200),
        ] {
            assert_eq!(iso_date_to_epoch(s), Some(want), "{s}");
        }
        // Anything the shape check rejects converts to nothing, rather than to a
        // plausible-looking number.
        for bad in [
            "2026-02-30",
            "not-a-date",
            "2026-8-19",
            "2026-08-19T00:00:00Z",
            "",
        ] {
            assert_eq!(iso_date_to_epoch(bad), None, "{bad}");
        }
    }
}
