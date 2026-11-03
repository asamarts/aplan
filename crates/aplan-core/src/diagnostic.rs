//! Diagnostics. Every rule has a stable code so CI output, agent output, and
//! human output all refer to the same thing.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::task::Evidence;

/// A diagnostic's machine-applicable repair.
///
/// This is deliberately narrower than [`Diagnostic::agent_instruction`]. An
/// instruction may ask a person to choose between several valid outcomes; a
/// repair exists only when the diagnostic already computed one exact edit that
/// the ordinary mutation surface can validate before writing (`A1-199`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Repair {
    /// Replace one cited piece of evidence with one exact replacement.
    ReplaceEvidence { from: Evidence, to: Evidence },
}

/// Severity of a diagnostic. `Error` fails `aplan check`; `Warning` does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Warning => write!(f, "warning"),
            Severity::Error => write!(f, "error"),
        }
    }
}

/// What a rule needs a task to carry before its predicate has anything to say.
///
/// A rule that evaluates to "pass" and a rule that had nothing to evaluate
/// produce the same output, and the second is the more common state (`A1-114`,
/// from pre-public adoption evidence item 2.4). These make the difference decidable:
/// a rule declares what a subject must carry for it to *apply*, and what it
/// compares that against, so a run can count the tasks where the comparison was
/// against nothing.
///
/// Declared here, on the same row that generates the code, the severity and the
/// catalogue entry, because a second list of "which rules can be vacuous" is the
/// same defect one level up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// The task claims to be finished. Most evidence rules judge nothing else.
    Done,
    /// The task claims work on it has started (`A1-359`). The subject of
    /// `APLAN053`, and the reason that rule has a vacuity question at all: a
    /// plan with no task under way is a plan the rule could not have fired on,
    /// which `vacuity` should say rather than leave a reader to assume the gate
    /// ran.
    InProgress,
    /// The task is work rather than a checkpoint over it. A milestone carries no
    /// `paths` and no estimate by design, so a rule that exempts one has to say
    /// which, or generated guidance repeats it as a universal claim and a
    /// literal reader "repairs" a valid cut (`A1-157`, pre-public adoption evidence).
    NotMilestone,
    /// The task's `paths`, the fixed point `APLAN012`, `APLAN024`, `APLAN025`
    /// and `APLAN031` all judge evidence against.
    Paths,
    /// The task's `traces`.
    Traces,
    /// The stamps `APLAN018` compares a traced document against.
    TraceFingerprints,
    /// The stamps `APLAN054` compares mutable evidence referents against.
    EvidenceFingerprints,
    /// Evidence of one kind: a rule about commits says nothing about a task
    /// citing only tests.
    Evidence(crate::task::EvidenceKind),
}

impl Binding {
    /// Whether this task carries the thing.
    #[must_use]
    pub fn holds(self, task: &crate::task::Task) -> bool {
        match self {
            Self::Done => task.status == crate::task::Status::Done,
            Self::InProgress => task.status == crate::task::Status::InProgress,
            Self::NotMilestone => !task.is_milestone(),
            Self::Paths => !task.paths.is_empty(),
            Self::Traces => !task.traces.is_empty(),
            Self::TraceFingerprints => !task.trace_fingerprints.is_empty(),
            Self::EvidenceFingerprints => !task.evidence_fingerprints.is_empty(),
            Self::Evidence(kind) => task.evidence.iter().any(|e| e.kind == kind),
        }
    }
}

/// **Changing what an existing rule *does* earns a `Behaviour`** (`A1-142`).
///
/// Adding a row here is already assertable: `--require APLAN031` answers it.
/// Narrowing or widening a row that already shipped is not, and `--require
/// APLAN019` passes on the build that reported 140 findings and on the build
/// that reports none. Name it in [`crate::behaviour::ALL`], in the same commit,
/// or a consumer pins a name and gets a build without the behaviour.
///
/// Measured before this note was written rather than after: no diff-shaped gate
/// catches it. A catalogue-diff gate would have caught three of seven known
/// cases at 25% precision, a corpus-diff gate four at 19%, and a gate that is
/// wrong three times in four teaches the escape hatch rather than the rule
/// (`A1-142`, and `A1-122` refused a rule on the same kind of evidence). The
/// check belongs to the audit, where it has caught two of two.
/// Define every invariant once, as an enum *variant*, a stable `code::`
/// constant, and a one-line title, from a single table, so the three can never
/// drift apart. Adding a rule is one row here.
macro_rules! define_rules {
    // A row's optional clause, or the empty default. Written as an internal rule
    // rather than as `$( return $e; )? &[]`, which compiles but leaves every
    // declaring row with an unreachable expression -- a warning, and this
    // workspace builds with `-D warnings`.
    (@or_empty) => { &[] };
    (@or_empty $e:expr) => { $e };
    ($(
        $(#[$meta:meta])*
        $variant:ident => $const_name:ident = $code:literal, $severity:ident, $title:literal
        $(, applies: $applies:expr, binds: $binds:expr)?
        , points_at: $points:expr
    );+ $(;)?) => {
        /// Every invariant `aplan` can report, as one enumerable registry.
        ///
        /// [`Rule::ALL`] is the authoritative set the coverage-drift audit walks
        /// (`crates/aplan-e2e`): adding a variant here without a *fires* and a
        /// *silent* scenario turns that audit, and CI, red. The `APLANxxx`
        /// string each rule maps to is the stable contract with consumers,
        /// exposed both as [`Rule::code`] and as a matching [`code`] constant
        /// generated from the same row.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Rule {
            $( $(#[$meta])* $variant ),+
        }

        impl Rule {
            /// Every rule, in declaration (code) order. The set the coverage
            /// audit iterates.
            pub const ALL: &'static [Rule] = &[ $( Rule::$variant ),+ ];

            /// The stable `APLANxxx` code, the contract with consumers.
            pub const fn code(self) -> &'static str {
                match self { $( Rule::$variant => $code ),+ }
            }

            /// The severity the rule fires at. Declared once here (not inferred
            /// from the `Diagnostic::error`/`warning` call sites), so `facts.json`
            /// and the docs read one authoritative value; a test asserts each
            /// firing site agrees with it.
            pub const fn severity(self) -> Severity {
                match self { $( Rule::$variant => Severity::$severity ),+ }
            }

            /// A one-line statement of the property the rule asserts, for
            /// generated docs and for the audit's failure messages.
            pub const fn title(self) -> &'static str {
                match self { $( Rule::$variant => $title ),+ }
            }

            /// The rule a `APLANxxx` code names, if any.
            ///
            /// Generated from the same rows as [`Rule::code`], so the two
            /// cannot disagree about which code is which rule.
            #[must_use]
            pub fn from_code(code: &str) -> Option<Rule> {
                match code {
                    $( $code => Some(Rule::$variant), )+
                    _ => None,
                }
            }

            /// The frontmatter field a finding from this rule sits on, or `None`
            /// when no single field answers.
            ///
            /// Declared on the row and *not* optional, so adding a rule is a
            /// choice between naming a field and saying there is not one. An
            /// optional clause would default a new rule to `None` silently,
            /// which is the difference between a held entry and an oversight
            /// (`AGENTS.md`, the exhaustive-audit pattern).
            ///
            /// The field to name is the one this file has to be read at for
            /// the finding to make sense, and `None` whenever more than one
            /// answers. `APLAN021` is the case that makes the second half
            /// matter: it joins several problems into one sentence ("is tagged
            /// `milestone` but carries `effort_days` and has no members"), so
            /// any single field it named would be the right line for some of
            /// its cases and a wrong one for the rest -- and a location that is
            /// sometimes a guess is worse than one that is sometimes absent.
            ///
            /// `None` is also the honest answer for a rule about the shape of
            /// the whole plan (`APLAN022`: a task in no cut is fixed by editing
            /// a *different* task) or about `.aplan.yml` rather than a task
            /// (`APLAN036`, `APLAN037`). `APLAN001` is `None` here because the
            /// file did not parse, so there are no fields to find -- the loader
            /// attaches the parser's own line instead, which is better than any
            /// field would have been.
            pub const fn points_at(self) -> Option<&'static str> {
                match self { $( Rule::$variant => $points ),+ }
            }

            /// What a task must carry for this rule to have a subject at all.
            ///
            /// Empty means every task is a subject. Declared on the row rather
            /// than in a second table, so a rule and its applicability cannot
            /// disagree (`A1-114`).
            pub const fn applies_to(self) -> &'static [Binding] {
                match self { $( Rule::$variant => define_rules!(@or_empty $($applies)?) ),+ }
            }

            /// What this rule's predicate compares a subject against.
            ///
            /// Empty means the predicate needs nothing beyond the subject. A
            /// subject carrying none of these is one the rule *could not have
            /// failed*, which is the count `aplan vacuity` reports.
            pub const fn binds_against(self) -> &'static [Binding] {
                match self { $( Rule::$variant => define_rules!(@or_empty $($binds)?) ),+ }
            }
        }

        /// Stable rule codes as `&str`, for the call sites that build a
        /// [`Diagnostic`]. Generated from the same table as [`Rule`], so a code
        /// and its rule cannot disagree.
        pub mod code {
            $( $(#[$meta])* pub const $const_name: &str = $code; )+
        }
    };
}

define_rules! {
    /// The loader could not turn a task file into a valid `Task`: unreadable,
    /// a frontmatter fence opened but never closed, invalid YAML, a schema
    /// violation, or a deserialisation failure. (An umbrella "the loader
    /// rejected this file", of which the schema case is the common one.)
    Schema => SCHEMA = "APLAN001", Error, "frontmatter parses and satisfies the schema",
        points_at: None;
    /// Two tasks share an `id`.
    DuplicateId => DUPLICATE_ID = "APLAN002", Error, "task ids are unique",
        points_at: Some("id");
    /// `blocked_by` names a task that does not exist.
    UnknownDep => UNKNOWN_DEP = "APLAN003", Error, "blocked_by references a task that exists",
        points_at: Some("blocked_by");
    /// The dependency graph contains a cycle.
    Cycle => CYCLE = "APLAN004", Error, "the dependency graph is acyclic",
        points_at: Some("blocked_by");
    /// `status: done` with an empty `evidence` list.
    DoneWithoutEvidence => DONE_WITHOUT_EVIDENCE = "APLAN005", Error,
        "done carries at least one piece of evidence",
        points_at: Some("evidence");
    /// A task lists itself in `blocked_by`.
    SelfDep => SELF_DEP = "APLAN006", Error, "no task blocks itself",
        points_at: Some("blocked_by");
    /// A `done` task depends on a task that is not `done`.
    DoneBeforeBlocker => DONE_BEFORE_BLOCKER = "APLAN007", Error,
        "a done task does not depend on an unfinished one",
        points_at: Some("blocked_by");
    /// A `traces` anchor does not resolve to a real file, and to a real heading
    /// in it, or -- since ADR-0035 -- a real node of it.
    ///
    /// Which resolver runs is the document's extension: a `.md` file has
    /// headings and table rows, a `.yaml`, `.yml` or `.json` file has nodes
    /// addressed by JSON Pointer or `JSONPath`. So the three failures an author
    /// meets have three sentences, because their remedies differ: the document
    /// is not addressable, the anchor is not an address, or it addresses nothing
    /// (`A1-191`).
    DanglingTrace => DANGLING_TRACE = "APLAN008", Error,
        "traces anchors resolve to a real heading or node",
        points_at: Some("traces");
    /// An active task carries no effort estimate.
    MissingEffort => MISSING_EFFORT = "APLAN009", Warning,
        "active tasks carry an effort estimate",
        applies: &[Binding::NotMilestone], binds: &[],
        points_at: Some("effort_days");
    /// Evidence of kind `note` without a `captured` date.
    UndatedNote => UNDATED_NOTE = "APLAN010", Error, "a note used as evidence carries a date",
        points_at: Some("evidence");
    /// `commit` evidence names an immutable SHA that does not resolve to a
    /// commit reachable from this repository's `HEAD` (L1 "Resolved"). Raised
    /// by `aplan verify`, never by `check`.
    CommitUnresolved => COMMIT_UNRESOLVED = "APLAN011", Error,
        "commit evidence resolves to an immutable commit reachable from HEAD",
        points_at: Some("evidence");
    /// `commit` evidence resolves, but its diff touches nothing in the task's
    /// `paths` (L2 "Relevant"), the citation is real but not bound to the task.
    CommitIrrelevant => COMMIT_IRRELEVANT = "APLAN012", Error,
        "commit evidence touches at least one of the task's paths",
        applies: &[Binding::Done, Binding::Evidence(crate::task::EvidenceKind::Commit)],
        binds: &[Binding::Paths],
        points_at: Some("evidence");
    /// `test` evidence names a test that the configured runner does not list
    /// (L1 "Resolved"). Needs a `test` runner in `.aplan.yml`; without one,
    /// `test` evidence stays at L0, like `url` reachability.
    TestUnresolved => TEST_UNRESOLVED = "APLAN013", Error,
        "test evidence names a test the runner can find",
        points_at: Some("evidence");
    /// `test` evidence names a test that the runner ran and it failed (L3
    /// "Executed"). Only under `aplan verify --execute`, which is off by default.
    ///
    /// It also says the *other* way `--execute` fails to prove anything: a
    /// runner that skips the cited test exits zero, and a zero exit is what
    /// this reads as a pass. A runner that declares `test.skipped` says which
    /// tests those are, and a task whose every test citation is on that list
    /// has no executed proof at all. Not `APLAN037`, which asks whether a pass
    /// is *indistinguishable* from a miss: a skip is plainly distinguishable,
    /// the runner having said so out loud (`A1-300`).
    TestFailed => TEST_FAILED = "APLAN014", Error, "test evidence names a test that passes",
        points_at: Some("evidence");
    /// `file` evidence names a path that does not exist at the cited rev (or at
    /// `HEAD` when none is cited) (L1 "Resolved").
    FileUnresolved => FILE_UNRESOLVED = "APLAN015", Error, "file evidence resolves at the cited rev",
        points_at: Some("evidence");
    /// A `url` citation did not answer when it was checked.
    ///
    /// `url` was the one evidence kind `verify` never resolved: checked for
    /// shape and nothing else, which made it the weakest link in an otherwise
    /// resolved ladder. A regulator page or a vendor quote that has since 404'd
    /// is exactly the kind of proof that rots quietly, because nothing reads it
    /// again.
    ///
    /// A *warning*, and phrased as unreachable *now*: a link down for an hour is
    /// not a plan that is wrong, and reporting a transient failure as bad
    /// evidence would train an adopter to ignore the rule. Off unless
    /// `.aplan.yml` sets `url.check`, so no default run touches the network and
    /// `check` stays hermetic by contract.
    ///
    /// The checker is a configured command rather than a built-in HTTP client,
    /// for the reason the test runner is: the tool must not decide on an
    /// adopter's behalf what touching the network means, and timeout and
    /// redirect policy belong in the command where they are visible and theirs
    /// to set (`A1-041`).
    UrlUnreachable => URL_UNREACHABLE = "APLAN016", Warning,
        "url evidence answers when it is checked",
        points_at: Some("evidence");
    /// A `done` task's evidence is weaker than the configured minimum: it has
    /// evidence, but none of an accepted kind, and no exempting tag. Off unless
    /// `.aplan.yml` sets `evidence.require_min_strength`. (`APLAN016`, url
    /// reachability, is reserved; it needs the network path, still off.)
    WeakEvidence => WEAK_EVIDENCE = "APLAN017", Error, "done evidence meets the configured minimum strength",
        points_at: Some("evidence");
    /// A `done` task's traced document section has changed since the task was
    /// closed: its `trace_fingerprints` no longer match the live content. A
    /// *warning* (advisory), and off unless `.aplan.yml` sets `detect_drift`; a
    /// task with no stamp is unverifiable, not in violation (zero false positives).
    /// Cleared by re-reviewing and re-stamping (`aplan close --restamp`).
    TraceDrift => TRACE_DRIFT = "APLAN018", Warning,
        "a closed task's traced section is unchanged since closure",
        applies: &[Binding::Done, Binding::Traces],
        binds: &[Binding::TraceFingerprints],
        points_at: Some("traces");
    /// A task declares a `paths` glob that matches no file the repository tracks.
    ///
    /// `paths` is the fixed point three shipped rules judge evidence against:
    /// `APLAN012` asks whether a cited commit touches it, `APLAN024` how much of
    /// that commit falls outside it, `APLAN025` whether a cited test covers it.
    /// It is also hand-written, and until this rule nothing checked it, so each of
    /// those phrased its finding as a defect in the *evidence* when the `paths`
    /// were at least as likely to be wrong (pre-public adoption evidence, item 27).
    ///
    /// The shape every pilot hit is a bare directory:
    /// `paths: [libs/p00025-core/src]` matches nothing, because a glob without a
    /// wildcard names one file and that name is a directory, so `APLAN012`
    /// reported the commit touched none of the task's paths *even where it
    /// plainly did*. `libs/p00025-core/src/**` is correct and silent. That is the
    /// first rule an adopter has to be told, and one repository wrote it into its
    /// own `AGENTS.md` rather than have it rediscovered per task
    /// (pre-public adoption evidence, item 13).
    ///
    /// A *warning*, and per-glob rather than per-task, because reporting the whole
    /// list would hide which entry is the dead one. Distinct from `APLAN023`,
    /// which fires when a task declares *no* paths at all: there the remedy is to
    /// add a glob, here it is to fix one, and reporting them the same way is the
    /// confusion the pilot named. Reads the git index, so it is a `verify` rung
    /// rather than a hermetic check, and `dropped` tasks are exempt because a
    /// cancelled task's ownership claim binds nothing.
    ///
    /// **Scoped to what the task cites, not to its status** (`A1-138`). It fires
    /// only where the task carries a `commit`, `file` or `test`: the kinds judged
    /// against `paths`. This paragraph used to say the opposite -- that a `todo`
    /// task naming code not yet written is "a gap rather than a violation", and
    /// warned anyway -- and the fourth pilot showed what that costs at scale: 140
    /// findings across 71 `todo` tasks whose globs name trees a later cut creates,
    /// under `deny_warnings`, and it declined an upgrade rather than take them
    /// (pre-public adoption evidence, item 24 and its correction). The scope is the
    /// rule's own sentence read literally: where nothing is bound to the paths,
    /// nothing binds to nothing. `close` applies the same scope at the moment a
    /// citation appears, so the two agree about when a glob starts mattering.
    ///
    /// A `done` task is judged at the revision its evidence cites, not at `HEAD`.
    /// A closed task describes history: its `paths` claimed ownership of the code
    /// as it stood when the work happened, so deleting that code later must not
    /// turn a correct glob into a finding whose advice is to correct it. A live
    /// task earns no such reading, because its `paths` are a claim about the code
    /// as it stands. A glob that matched nothing at any revision is still
    /// reported on a closed task, which is the pilot's own case (`A1-022`).
    PathsUnmatched => PATHS_UNMATCHED = "APLAN019", Warning,
        "a task's paths globs each match at least one tracked file, once it cites evidence",
        points_at: Some("paths");
    /// A live task is blocked by a `dropped` one, so it can never legitimately
    /// start: the plan is deadlocked.
    ///
    /// `APLAN011`-`APLAN019` are reserved for the planned evidence ladder
    /// (ROADMAP §10), which is why this rule, shipped earlier, takes `020`. A
    /// code is a contract with consumers; it is not renumbered for tidiness.
    DroppedBlocker => DROPPED_BLOCKER = "APLAN020", Error, "a live task is not blocked by a dropped one",
        points_at: Some("blocked_by");
    /// A task tagged `milestone` does not look like one: it has no members
    /// (`blocked_by` empty, or holding no live work), or it carries `effort_days`
    /// or `paths`, which a checkpoint does not.
    ///
    /// **Members are counted as live work** (`A1-134`): ADR-0013 reuses
    /// `blocked_by` to sequence cuts, so a cut emptied of work still lists its
    /// predecessor, and `A1-081` exempts a cut's cancelled member from
    /// `APLAN020` on purpose, so a cut whose work was all dropped was silent for
    /// a second reason. Both are a cut with nothing in it.
    ///
    /// Two exemptions, both found by auditing the widening rather than by
    /// reasoning about it. A **cancelled** cut is supposed to hold cancelled
    /// work, and `APLAN027` owns the mirror; reporting it would fire on every
    /// `aplan drop --cut` and advise cancelling a cut already cancelled. And a
    /// `blocked_by` naming nothing that *exists* is `APLAN003`'s finding, left to
    /// it rather than described twice.
    ///
    /// One interaction is deliberately left open: a milestone whose members are
    /// milestones reads as "a cut holding only other cuts" here, and that is also
    /// the encoding `A1-146` is weighing for a *release* over cuts. If that
    /// encoding is adopted, this rule is what has to learn the difference, and
    /// `A1-146` owns the decision.
    MilestoneHygiene => MILESTONE_HYGIENE = "APLAN021", Warning,
        "a milestone has members and carries no effort or paths",
        points_at: None;
    /// A non-milestone task belongs to no cut, or to more than one: it is in the
    /// `blocked_by` of zero, or of several, milestone meta-tasks (ADR-0008), and is
    /// not tagged `cross-cutting`. Off unless `.aplan.yml` sets
    /// `require_cut_membership`. Enforcing it makes a milestone's `done` prove the
    /// cut is *exhaustively* complete, not merely that its listed members are, and
    /// closes the gap `APLAN021` (empty/mis-tagged cuts) does not: a real task in
    /// no cut. `cross-cutting` is the escape hatch for a task that deliberately
    /// belongs to none (a quality gate spanning every cut).
    MembershipTotality => MEMBERSHIP_TOTALITY = "APLAN022", Error,
        "every task is a member of exactly one cut, or is tagged cross-cutting",
        applies: &[Binding::NotMilestone], binds: &[],
        points_at: None;
    /// A `done` task cites evidence whose relevance cannot be checked, because it
    /// declares no `paths`. `APLAN012` binds a cited `commit` to the task by
    /// requiring its diff to touch one of those globs; with none declared there is
    /// nothing to compare against, so even a citation that resolves proves only
    /// that its referent exists (or, for a commit, belongs to `HEAD` history), not
    /// that the task's files changed. The first pilot found seventeen of
    /// thirty-three tasks
    /// in this state while `check` reported zero warnings and `verify` reported
    /// evidence resolved (pre-public adoption evidence item 6). A warning rather than
    /// an error: the citation may well be honest, and the repository is the judge
    /// of that; what it may not be is *silent*. Hermetic, because the question is
    /// structural (does this task declare a path that names anything) and needs no
    /// git. A blank entry is no entry: `paths: [""]` binds nothing that `paths: []`
    /// would not. How *broad* a real glob is stays a review question, not this
    /// rule's (`A1-043`).
    UncheckableEvidence => UNCHECKABLE_EVIDENCE = "APLAN023", Warning,
        "a done task's path-checkable evidence can actually be path-checked",
        applies: &[Binding::Done, Binding::NotMilestone], binds: &[],
        points_at: Some("evidence");
    /// A `done` task whose whole machine-checkable proof is a commit that is
    /// mostly about something else. `APLAN012` asks whether the diff touches the
    /// task's `paths` at all, which a large enough commit satisfies by accident:
    /// the p00025 pilot measured 31 of 64 citations pointing at commits touching
    /// more than fifty files, 16 of them the same 124-file scaffold, and for
    /// those `APLAN012` could not have rejected anything
    /// (pre-public adoption evidence, third pass). The measure is the count of files the
    /// commit touches *outside* the task's `paths`, not its size: a large change
    /// entirely inside them is focused, and a small one cannot swamp anything.
    /// Silent when the task has other proof, since a passing test says what a
    /// diff cannot. A warning rather than an error, on `APLAN023`'s reasoning:
    /// bootstrapping a plan from a scaffold commit is legitimate and the
    /// repository is the judge of it, but it must not read the same as a focused
    /// citation (`A1-062`).
    DilutedEvidence => DILUTED_EVIDENCE = "APLAN024", Warning,
        "a done task's commit evidence is about that task, not mostly about other work",
        applies: &[Binding::Done, Binding::Evidence(crate::task::EvidenceKind::Commit)],
        binds: &[Binding::Paths],
        points_at: Some("evidence");

    /// A `test` is the strongest evidence kind this tool recognises, the one
    /// `require_min_strength` steers repositories toward, and it was the only one
    /// whose *relevance* went unchecked: `APLAN012` binds a commit to the task's
    /// `paths`, `APLAN015` binds a file, nothing bound a test. The p00025 audit
    /// found `WH6-02` citing `tests::app_builds`, which lives in
    /// `server/p00025-server` and asserts only that the *server's* router
    /// constructs, for a task whose work is entirely under `p00025.org/`
    /// (pre-public adoption evidence, third pass). `APLAN013` accepted it because a
    /// runner really does list that name, which is correct behaviour on a
    /// dishonest citation. A runner declares what it covers, with `covers` or,
    /// failing that, the `dir` it runs in, so a runner whose code the task's
    /// `paths` never reach is the same mechanical mismatch `APLAN012` reports for
    /// a commit. `covers` exists because the first shipped version read only
    /// `dir` and could fire on no real repository: a runner selecting a component
    /// with `--manifest-path` runs at the root, so where it runs is not what it
    /// covers (`A1-065`). Silent when the runner declares neither, since it then
    /// makes no claim about locality, and when a named subset of a runner's tests
    /// covers something else (`covers_by_test`), which is how a repository-wide
    /// invariant living in a tools crate says it proves a fact about a tree it is
    /// not part of (`A1-069`). A warning rather than an error: a
    /// component with no tests of its own may have a legitimate integration test
    /// elsewhere, and a task answers that by declaring the `paths` the work truly
    /// spans, exactly as `APLAN012` is answered (`A1-063`).
    TestIrrelevant => TEST_IRRELEVANT = "APLAN025", Warning,
        "a cited test belongs to the part of the repository this task's `paths` cover",
        applies: &[Binding::Done, Binding::Evidence(crate::task::EvidenceKind::Test)],
        binds: &[Binding::Paths],
        points_at: Some("evidence");

    /// A second task-membership dimension ends up in tags, because tags are
    /// what exists. `ADR-0008` rejected them for cut membership precisely
    /// because a typo is silent, and then a pilot put 22 epics there anyway,
    /// plus a build/measure/review taxonomy beside them: one validated
    /// dimension and one unvalidated one in the same frontmatter, with no way
    /// to tell them apart by looking (pre-public adoption evidence, items 4 and 5).
    /// Declaring the vocabulary in `.aplan.yml` makes a claim the repository
    /// already believes checkable without making tags into containment. A tag
    /// is judged to belong to a family
    /// when it carries the family's `name-` prefix, or when it is one edit away
    /// from a declared member, which is what catches a transposition the prefix
    /// cannot see. A *warning*, not an error: an undeclared tag in a family's
    /// shape is as likely to be a member nobody has declared yet as a mistake,
    /// and the repository, which can set `deny_warnings`, is the judge of which
    /// (`A1-076`).
    UnknownTag => UNKNOWN_TAG = "APLAN026", Warning,
        "a tag that looks like a member of a declared family is one",
        points_at: Some("tags");

    /// A cut is `dropped` while work it lists is still live. The plan then says
    /// two things at once: this cut is cancelled, and these tasks in it are
    /// going ahead. Nothing reported it, and a bug produced exactly that state,
    /// `drop --cut` following a sequencing edge and cancelling the predecessor
    /// cut while leaving its members alone (`A1-084`). A hand-edit reaches it
    /// too, and so does `aplan drop <milestone>` without `--cut`, which cancels
    /// the checkpoint and nothing else. A *warning*, because the repository may
    /// be mid-edit and the remedy is a judgement, to cancel the work as well or
    /// move it to a cut that is still live; `deny_warnings` makes it blocking
    /// for a repository that wants it to be.
    CancelledCutLiveWork => CANCELLED_CUT_LIVE_WORK = "APLAN027", Warning,
        "a cancelled cut's members are cancelled too",
        points_at: Some("status");
    /// A `done` task does not stamp a trace it could stamp, so `APLAN018` is
    /// silently off for the section it cannot compare.
    ///
    /// `trace_fingerprints` is keyed by the trace string, so *editing* a `traces`
    /// entry on a closed task orphans the old key and leaves the new trace
    /// unstamped. Drift then has nothing to compare against and says nothing: the
    /// plan reads clean under `deny_warnings` with `detect_drift` on, and the
    /// traced section can change afterwards with no diagnostic at all. Both of the
    /// fourth pilot's traced tasks sat in that state and passed
    /// (pre-public adoption evidence, item 16), and the ADR-0015 migration is exactly
    /// that edit, so the recommended path walks an adopter through it.
    ///
    /// An *error*, not a warning, because it is a gate that has been switched off
    /// rather than a judgement call, and because `close --restamp` repairs it
    /// exactly.
    ///
    /// **It used to exempt a task that stamped nothing**, on the reasoning that
    /// one closed before `detect_drift` was enabled carries traces and no
    /// fingerprints, and that treating it as unverifiable rather than in
    /// violation is why adopters leave drift on (`A1-098`). That was measured and
    /// removed (`A1-297`): across 2,410 task files in thirteen plans, every one
    /// of them running `detect_drift`, the partial case this rule was written for
    /// occurs **zero** times and the exempted case occurs nine. It fired on the
    /// shape nobody has and was silent on the only shape that happens -- which is
    /// this rule's own sentence about every trace rather than about one. An
    /// adopter who wants the softer landing has `advisory`, which reports the
    /// rule without failing the build and says in the output that it did; an
    /// early return said nothing to anybody.
    ///
    /// **Only traces something could stamp**, which is asked of the function that
    /// does the stamping rather than described here: a trace with no anchor, one
    /// whose document cannot be read, and one whose anchor matches nothing are
    /// all unstampable, and reporting them would be an error whose own fix
    /// provably cannot clear it. The last two are `APLAN008`'s and it reports
    /// them.
    ///
    /// **A trace with no anchor is therefore watched by nothing, and that is
    /// decided rather than overlooked.** ADR-0035 records it with the
    /// measurement behind it: zero of 414 closed-task traces across thirteen
    /// plans carry no anchor, so a rule here would have no subjects at all. A
    /// durable address for the reasoning, rather than a task that has since
    /// closed (`A1-191`).
    ///
    /// **And not a trace `APLAN034` is already refusing.** A target outside the
    /// repository that nothing declares is one the author is being told to
    /// declare or drop, so asking them to stamp it in the same breath is two
    /// errors for one cause whose fixes disagree. That precedence is the one the
    /// per-trace loop states for the same reason (`A1-143`); this rule runs
    /// before the loop and had to be told (`A1-297`, audit).
    UnstampedTrace => UNSTAMPED_TRACE = "APLAN028", Error,
        "a done task stamps every trace that can be stamped",
        points_at: Some("traces");

    /// A declared tag family that nothing in the plan uses.
    ///
    /// The fourth pilot declared its epics the wrong way round, `epic: [a, b]`
    /// with tasks tagged `epic:a`, and every layer accepted it:
    /// `validate_tag_families` sees a named, non-empty family,
    /// `validate_roadmap` sees a `group_by` naming a declared one, and
    /// `APLAN026` judges only tags that already look like members, of which
    /// there were none. `check` reported no findings and the roadmap rendered
    /// every task under `(no group)` (pre-public adoption evidence, item 9).
    ///
    /// **This is the defect `APLAN026` exists to prevent, one level up.** That
    /// rule makes a vocabulary checkable so a typo in a *tag* is not silent;
    /// nothing asked whether the vocabulary itself was used, which is the
    /// question that catches a vocabulary declared in a shape no tag can match.
    ///
    /// Deliberately a question about the *plan* rather than about `.aplan.yml`,
    /// because the config-time form would be wrong: a family may legitimately
    /// list members that do not carry its prefix, `epic: [a, b]` with tasks
    /// tagged `a`, and a check that rejected that would refuse a working
    /// configuration. Only a rule can see whether any tag matches.
    ///
    /// A *warning*, on `APLAN026`'s own reasoning: an unused vocabulary is as
    /// likely to be a member nobody has tagged yet as a mis-declaration, and the
    /// repository, which can set `deny_warnings`, is the judge (`A1-085`).
    UnusedTagFamily => UNUSED_TAG_FAMILY = "APLAN029", Warning,
        "a declared tag family is carried by at least one task",
        points_at: None;

    /// A task carries the number of members a declared family requires.
    ///
    /// `tag_families` declares what a vocabulary contains and, before this, said
    /// nothing about how many of it a task should carry, so `priority-p0` and
    /// `priority-p1` together and no priority at all were both clean. The fifth
    /// pilot needs exactly one priority and kind on live work, one primary
    /// stream on programme work, and one disposition on cancelled work
    /// (pre-public adoption evidence).
    ///
    /// An *error*, not a warning, and the distinction from `APLAN026` and
    /// `APLAN029` is what makes it one. Those two report something that *looks*
    /// wrong: a tag near a vocabulary, a vocabulary nobody uses, either of which
    /// may be a repository mid-authoring. This reports a requirement the
    /// repository wrote down being unmet, which is the shape `APLAN022` and
    /// `APLAN017` already have: both are opt-in, both are errors, because a
    /// declared policy is not a guess. A repository adopting the requirement
    /// before its plan satisfies it has `advisory:` for exactly that
    /// (`A1-112`, `A1-133`).
    TagCardinality => TAG_CARDINALITY = "APLAN030", Error,
        "a task carries the number of members a declared family requires",
        points_at: Some("tags");
    /// A `done` task cites a `file` that exists and lies outside its `paths`, so
    /// the citation proves the file is there and nothing about this task.
    ///
    /// `APLAN012` makes a `commit` citation mean "a commit that touched this
    /// work" by checking its diff against `paths`. `APLAN015` had no equivalent:
    /// it asked whether the file exists at the cited rev and stopped, so
    /// `file: README.md` resolves for a task about the renderer and proves the
    /// repository has a README. `require_min_strength` counts `file` among the
    /// strong kinds, so the same reads-as-a-guarantee problem applies (`A1-045`).
    ///
    /// Two consumers found it independently. The first pilot filed it from
    /// reading the rules (pre-public adoption evidence item 6); the fourth
    /// reproduced it in one command on a real plan, closing a task about ADR
    /// authorship with a Containerfile, and noted the sharper half: its
    /// conventions tell authors to prefer `file:` over `commit:` *because*
    /// `APLAN024` is right that a 116-file commit names a Tuesday. A repository
    /// that took the earlier advice was more exposed, not less
    /// (pre-public adoption evidence item 25).
    ///
    /// An *error*, matching `APLAN012` rather than `APLAN023`'s warning, and the
    /// distinction is deliberate: `APLAN023` is a warning about a claim nothing
    /// can check, and this is a claim that was checked and does not hold.
    /// Separate from `APLAN015` for the reason `APLAN012` is separate from
    /// `APLAN011`: "it is not there" and "it is not yours" are different faults
    /// with different remedies, and one code for both would make the exit code
    /// less informative, not more.
    FileIrrelevant => FILE_IRRELEVANT = "APLAN031", Error,
        "file evidence names a file inside the task's paths",
        applies: &[Binding::Done, Binding::Evidence(crate::task::EvidenceKind::File)],
        binds: &[Binding::Paths],
        points_at: Some("evidence");
    /// A `paths` entry the glob compiler cannot use, so every path-based rule is
    /// silently inapplicable to that task.
    ///
    /// A *hermetic* rule about a task's own field, which is the whole repair.
    /// `paths: ['src/[']` used to abort `verify` before it reported anything:
    /// one task's typo hid `APLAN005` in a *different* task, and it exited `2`,
    /// which ADR-0007 reserves for the invocation or the config and which sent
    /// the reader to neither of the two places the defect was (`A1-066`).
    ///
    /// Checked here rather than in the ladder because a glob's validity needs no
    /// repository: `check` and `verify` therefore report it identically, which
    /// is the containment `A1-058` promised and this case broke.
    ///
    /// An *error*, not `APLAN019`'s warning, and not folded into it. The reader's
    /// question is different -- "this glob cannot be used" is not "this glob
    /// matched nothing" -- and so is the consequence: an unmatched glob leaves
    /// `APLAN012` able to judge the citation against the other entries, while an
    /// unusable one leaves the rules that *bind evidence* through `paths`
    /// (`APLAN012`, `APLAN024`, `APLAN031`) with nothing to say about a task that
    /// may be `done`. `APLAN019` still reports the entries it can compile, which
    /// is why the message says the evidence cannot be bound rather than claiming
    /// every rule goes quiet.
    BadPathGlob => BAD_PATH_GLOB = "APLAN032", Error,
        "a task's paths entries are globs this tool can compile",
        points_at: Some("paths");
    /// A `prefer_after` entry that cannot be honoured: an unknown id, the task
    /// itself, a pair that is already a `blocked_by` edge, or a cycle of
    /// preferences with no order at all (`A1-150`, ADR-0025).
    ///
    /// One rule for four shapes because they are one reader question -- *this
    /// preference cannot be honoured as written* -- and one remedy: correct the
    /// entry or remove it.
    ///
    /// An *error*, though the field is discretionary, because none of these is a
    /// preference the tool declined to follow: each is a preference that says
    /// nothing. A dangling id names no task, a self-reference orders nothing, a
    /// duplicated dependency makes the plan say two different things about one
    /// pair, and a cycle has no satisfying order. A warning here would leave a
    /// plan quietly carrying an order nobody can act on, which is the class of
    /// silence this project exists to remove.
    UnusablePreference => UNUSABLE_PREFERENCE = "APLAN033", Error,
        "a prefer_after entry names a task that can actually be preferred",
        points_at: Some("prefer_after");
    /// A `traces` target that leaves this repository without the repository
    /// saying so (`A1-143`, ADR-0021).
    ///
    /// Not "the anchor did not resolve", which is `APLAN008`. This fires when it
    /// resolves *perfectly*: `traces` is a filesystem read against `traces_root`,
    /// so `../p0000007/docs/STATUS.md#phase-1` resolves whenever that sibling
    /// happens to be checked out, a bad anchor in it is refused, and
    /// `detect_drift` fires across the boundary. The fifth pilot verified all
    /// three (pre-public adoption evidence, item 4c).
    ///
    /// That makes a repository's `check` an **undeclared hard dependency** on
    /// another repository being present at an exact relative path and ref: the
    /// gate passes on the machine that has the sibling and fails with `APLAN008`
    /// on the job that does not, and raises drift for a change this repository
    /// did not make. A plan that passes on one machine and not another, for a
    /// reason the plan does not state, is the failure this tool exists to
    /// remove.
    ///
    /// Declaring the root in `traces_external` makes it the repository's stated
    /// position, and then its *absence* is a configuration error naming the
    /// declaration rather than a finding about a task that is fine.
    ///
    /// The same rule is what keeps a task file from naming `/etc/passwd`: a
    /// citation arrives in a fork's pull request, and `A1-041` made the same
    /// argument for `url` evidence when it refused every scheme but `https`.
    ExternalTrace => EXTERNAL_TRACE = "APLAN034", Error,
        "a traces target outside this repository is declared as external",
        points_at: Some("traces");
    /// `commit` evidence naming a merge commit, which is almost never the commit
    /// that did the work (`A1-145`).
    ///
    /// The first pilot closed a task with `--evidence commit:$(git rev-parse
    /// --short HEAD)` *before* making the commit it meant to cite, and recorded
    /// the merge of an unrelated pull request. Its `paths` were
    /// `crates/p0000004-core/**`, where nearly all that project's work happens,
    /// so `APLAN012` passed and `verify` reported 53 tasks clean
    /// (pre-public adoption evidence, item 11).
    ///
    /// A *warning*, not an error, because "almost never" is not never: a
    /// repository that merges every feature with `--no-ff` has a merge that is
    /// the honest citation for the feature. The message says what it saw --
    /// a commit with more than one parent -- rather than concluding the citation
    /// is wrong, which is a distinction `A1-070` had to learn the hard way.
    ///
    /// Measured before it shipped, and the honest version of the number is the
    /// smaller one: 177 commit citations across four plans name no merge, but
    /// only two of those repositories *have* merges at all (23 in p0000004, one
    /// in p006), so the 26 citations in those two are the evidence and the rest
    /// could not have fired either way. Silent where the plan is right,
    /// including on p0000004 as it stands today; restore the citation that plan
    /// carried before it was repaired by hand and it fires on it (`A1-145`,
    /// checked against the pilot's own repository).
    MergeCommitEvidence => MERGE_COMMIT_EVIDENCE = "APLAN035", Warning,
        "commit evidence names a commit that did work, not a merge",
        points_at: Some("evidence");

    /// A configured test runner that lists nothing is indistinguishable from a
    /// broken one, and the failure lands on the wrong file
    /// (pre-public adoption evidence, third pass).
    ///
    /// A typo in `dir`, a `list` command that quietly stopped matching, a
    /// manifest path that moved: each produces an empty list, no error, and
    /// citations that fail later against what "the configured runners list" --
    /// which blames the *citation*. The author opens the task file, and the
    /// configuration that stopped seeing its tests is the last place they look.
    ///
    /// It matters more since one plan may configure many runners. With one, an
    /// empty listing made `test` evidence unusable everywhere and you noticed
    /// immediately; with five, four can work while one is quietly blind, and the
    /// blindness is scoped to whichever component nobody has cited yet.
    ///
    /// A *warning*, and expressible, because the benign case is real: the
    /// repository that found this configures five runners and two of them list
    /// nothing because those components have no tests yet. They are correct and
    /// inert. `test.expect_tests: false` says so, and without it an adopter has
    /// to choose between a permanent finding and deleting a runner they will need.
    ///
    /// **What it does not catch, measured.** A runner that lists *something* is
    /// silent here however wrong it is. Two of the three cases behind this task
    /// are that shape: a runner built for one feature set lists every test except
    /// the gated ones it was cited for (pre-public adoption evidence item 8),
    /// and `pytest --collect-only -q` prints `no tests collected in 0.00s` when it
    /// finds nothing, so an empty pytest runner lists exactly one "test"
    /// (`A1-034`). No count distinguishes those from a small component, which is
    /// why this rule is scoped to the case a count can see.
    RunnerListsNothing => RUNNER_LISTS_NOTHING = "APLAN036", Warning,
        "a configured test runner lists at least one test, or says it expects none",
        points_at: None;

    /// A test that "passed" under `--execute` was distinguishable from one the
    /// runner never selected.
    ///
    /// `--execute` is the strongest rung of the ladder: L1 says a runner knows
    /// the name, L3 says the test *passed*. L3 is decided by the run command's
    /// exit status, and that is only worth what the runner makes it worth. The
    /// assumption underneath is invisible until it breaks: **the name a runner
    /// lists is the name that selects it**. `vitest list` prints
    /// `file > suite > test` while `vitest run -t` matches a test's own title,
    /// so a citation of the listed name selects nothing, every test is skipped,
    /// and the status is `0`. Measured on vitest 4.1.11: a deliberately failing
    /// test, cited and executed, verified clean (pre-public adoption evidence,
    /// `A1-034`).
    ///
    /// Exit status cannot catch it. `cargo test -- --exact <no such test>` exits
    /// `0` too, and that is what a correctly configured cargo runner does, so a
    /// rule that read a zero-status miss as a fault would fire on cargo, where
    /// it is true and useless. So the runner is asked what a *miss* looks like,
    /// and judged against its own answer rather than against anything this tool
    /// knows about any runner. Measured on the three the pilots use: pytest
    /// 9.1.1 exits `4` and is settled by status alone; cargo exits `0` and
    /// prints `running 0 tests`, which settles it; vitest 4.1.11 exits `0` and
    /// prints the same bytes either way but for a clock and a duration.
    ///
    /// Which is why the comparison masks runs of digits. Learning the volatile
    /// lines instead -- running the sentinel twice and discounting whatever the
    /// two disagreed about -- was built and abandoned: two back-to-back vitest
    /// runs printed the *same* `Start at` second often enough that the verdict
    /// depended on whether a clock had ticked between two processes, and a rule
    /// that does that is worse than no rule. What masking costs is a runner
    /// whose only difference between a hit and a miss is a *number*.
    ///
    /// A *warning*, and declarable: a runner that says nothing at all either way
    /// is indistinguishable in fact, not merely to this tool, and
    /// `test.run_selects: true` is the repository asserting what only it can
    /// know. Reported once per runner rather than per citation, because the
    /// pairing of `list` and `run` is one configuration fault however many
    /// citations it silently passed (`A1-066`).
    RunProvesNothing => RUN_PROVES_NOTHING = "APLAN037", Warning,
        "a test that passed under `--execute` is distinguishable from one the runner never ran",
        points_at: None;
    /// `done` over a definition of done the author never answered (`A1-224`).
    ///
    /// `aplan add` scaffolds `- [ ] state what done means for this task, in this
    /// repository's terms`, and `close` writes the status, the evidence and the
    /// fingerprints and touches nothing in the body. The tool authors the
    /// obligation and never reads it, so the one statement written *by* the
    /// author *for* the author is the one thing nothing checks. `APLAN005` asks
    /// whether the evidence list is non-empty; this asks whether the task's own
    /// terms were met.
    ///
    /// **Measured before it was built.** Over this plan's 213 closed tasks, 196
    /// were fully ticked, 7 had none ticked, 10 state no criteria at all, and
    /// **none was partially ticked**. That zero is the informative number:
    /// nobody is making a per-criterion judgement and leaving one open, so this
    /// catches a forgotten step rather than overriding a decision. The 7 all
    /// closed on one day, under a workflow that used `close` exactly as
    /// designed.
    ///
    /// **Silent when the body states none.** Ten closed tasks here state no
    /// criteria, and a rule about a markdown convention must not demand the
    /// convention: it speaks only where the author wrote a checkbox and left it
    /// unticked. Absence is `APLAN005`'s business and this one's silence.
    ///
    /// **A warning, not an error.** An unticked box may be an oversight in
    /// bookkeeping rather than a false status, and which it is only the author
    /// knows -- the reading `APLAN023` already takes about a citation that may
    /// be honest. A repository whose policy is that every gate blocks sets
    /// `deny_warnings`.
    ///
    /// **Never ticked by the tool.** A `--tick` marking every criterion met
    /// because the task was closed would manufacture exactly the claim this tool
    /// exists to disbelieve, and is refused rather than deferred (`A1-224`).
    /// `p0000000000005` proposed `close --criteria-met` and it is refused for
    /// the same reason; what it actually wanted is `close --deny-warnings`, so
    /// the closure is stopped at the moment of the decision rather than in CI
    /// (`A1-266`).
    ///
    /// **Three outcomes, not two.** The fix used to offer *say what is left* or
    /// *supersede it*, and both read as unpick the closure. `p00020` took a
    /// third: hand the unmet criterion to the task that will do it, tick it as a
    /// decision, and leave the record of what was not achieved -- which is what
    /// a plan reaches for when the work is real and belongs elsewhere
    /// (`RD1-26` to `RD4-05`, `A1-266`).
    CriteriaUnanswered => CRITERIA_UNANSWERED = "APLAN038", Warning,
        "a `done` task has answered the acceptance criteria its own body states",
        points_at: None;

    /// A `done` task has not ticked a prompt this tool wrote as though it were a
    /// criterion.
    ///
    /// `aplan add` writes `- [ ] state what done means for this task, in this
    /// repository's terms`, and the import paths write `- [ ] restate what done
    /// means for this item, in this repository's terms`. Both are the
    /// instruction to write a criterion, not one, and a comment in this
    /// repository claimed for a long time that they were the same line -- so
    /// the first version of this rule knew only `add`'s, and an imported task
    /// closed with its prompt ticked made the same false claim unreported, on
    /// the path that creates tasks in bulk (`A1-271`'s audit). Unticked it is `APLAN038`'s business; **ticked**, the task records
    /// that the instruction was carried out, and the plan now holds a met
    /// criterion nobody wrote.
    ///
    /// **The one content this tool may read.** `A1-224` made `APLAN038` count
    /// checkboxes and never look inside them, because a rule about a markdown
    /// convention must not demand the convention. The scaffold is the exception
    /// that proves it: the sentence is `scaffold::ADD_BODY`, written by this
    /// tool, so recognising it is not a guess about somebody's prose. Matching
    /// is exact after trailing whitespace and never a prefix, so a real
    /// criterion that happens to begin the same way is silent.
    ///
    /// **Measured before it was built.** `p00020` closed four tasks over the
    /// placeholder left unticked, which `APLAN038` caught; `p0000014` authored
    /// 134 tasks in one `add --from`, every one carrying the line, where
    /// ticking is the shortcut that ends that job in a single pass (`A1-271`).
    ///
    /// A warning, for `APLAN038`'s reason: an adopter meets it on history they
    /// have already shipped, and `deny_warnings` is how a repository says every
    /// gate blocks.
    ScaffoldTicked => SCAFFOLD_TICKED = "APLAN039", Warning,
        "a `done` task has not ticked a prompt this tool wrote as though it were a criterion",
        points_at: None;

    /// A task does not declare the same `paths` entry twice.
    ///
    /// Not a correctness defect and the severity says so: a duplicate glob
    /// matches the same files, so `verify` resolves identically, `APLAN012`
    /// binds the same commits and no citation is weakened. It is a **signal**
    /// defect, and the signal is the one `paths` exists to give -- a duplicated
    /// entry is almost always a botched edit, and a bulk `paths` rewrite is
    /// exactly the occasion for one.
    ///
    /// `p00016` found two tasks carrying the same glob twice while consolidating
    /// a crate map from thirty crates to fifteen, which meant rewriting `paths`
    /// across 22 files. Both predated that rewrite. The audit that found them
    /// was written by hand because nothing else would have (`A1-267`).
    ///
    /// Two spellings of one path are one entry, because `A1-254` made them one
    /// path everywhere else -- read through `glob::spelling`, so this rule and
    /// the compiler that matches files cannot disagree about what "the same"
    /// means. Recognising that `src/**` *subsumes* `src/a.rs` is containment
    /// rather than duplication, a much larger claim, and deliberately not this.
    ///
    /// Distinct from `APLAN019`, which is about an entry matching no tracked
    /// file: this one matches exactly what its twin matches. Two different facts
    /// under one code is what the catalogue exists to prevent (`A1-088`).
    DuplicatePath => DUPLICATE_PATH = "APLAN040", Warning,
        "a task declares each `paths` entry once",
        points_at: Some("paths");

    /// The generated block in a repository's `AGENTS.md` still says what its
    /// policy says.
    ///
    /// `export-agents-md` writes the contract an agent reads before touching the
    /// plan, and this tool's own contract opens by warning about exactly what
    /// that is: *a document that carries status becomes a second copy of it, and
    /// the second copy drifts.* Here the tool is generating the copy.
    ///
    /// It drifted in the field. `p00002`'s block said *"a `done` task must cite
    /// `commit`, `file` or `test` evidence"* for a full day after
    /// `require_min_strength` had dropped `commit`, so the file told an agent a
    /// policy the tool would refuse. `--check` existed and would have caught it;
    /// nothing made anybody run it, and it was found on a hunch (`A1-273`).
    ///
    /// **Scoped by the markers, which is the repository asking.** A file with no
    /// block, or no file at all, is silent: an adopter who has never run
    /// `export-agents-md` is not being nagged about a feature they did not want.
    /// A file carrying *half* a marker pair is a finding rather than a silence,
    /// because otherwise the way to switch this rule off is to delete one comment
    /// line, and a gate that can be turned off invisibly is not one.
    ///
    /// A warning rather than an error, for the reason `APLAN018` is: a second
    /// copy going stale is a signal defect, the plan itself still holds, and the
    /// repository decides how loud that is.
    StaleAgentContract => STALE_AGENT_CONTRACT = "APLAN041", Warning,
        "the generated `AGENTS.md` block says what this repository's policy says",
        points_at: None;

    /// A `revisit_at` names a real instant, on a task that is parked.
    ///
    /// The field is a hand-off: it says when a parked task is worth looking at
    /// again, and `aplan due` is the only thing that reads it. It claims nothing
    /// about whether the wait is over -- `p00013`'s `DN0-29` waits on GitHub
    /// creating a `schedule` event, and the hour arriving proves only that the
    /// hour arrived (`A1-264`).
    ///
    /// Two ways one property fails. An instant this tool cannot parse is a
    /// hand-off no scheduler will act on, and one on a task that is not parked
    /// is inert: `due` reports parked work, `ready` already offers everything
    /// else, and a revisit on a `todo` task is a note to nobody.
    ///
    /// A warning rather than an error, because neither weakens a claim the plan
    /// makes. The field proves nothing and gates nothing; what it can do is
    /// quietly fail to reach the scheduler that was supposed to read it.
    RevisitUnusable => REVISIT_UNUSABLE = "APLAN042", Warning,
        "a `revisit_at` names a real instant, on a task that is parked",
        points_at: Some("revisit_at");

    /// A live task does not carry the scaffold's own prompt beside criteria it
    /// wrote (`A1-299`).
    ///
    /// `aplan add` writes a `## Acceptance criteria` heading and one prompt under
    /// it. There is no command that edits a body, so the way an author supplies
    /// the real criteria is by hand -- and the obvious hand-edit, appending a
    /// `## Why` and a `## Acceptance criteria` of their own, leaves the file with
    /// two of each and the prompt still in it. The sixth pilot did exactly that
    /// on its first day, and a human re-reading the file found it, not the tool.
    ///
    /// **The same exception `APLAN039` stands on**: the sentence is
    /// `scaffold::ADD_BODY`'s, written by this tool, so recognising it is not a
    /// guess about somebody's prose. Matched exactly, outside fenced code, so a
    /// task *quoting* the prompt -- which several in this repository's own plan
    /// do -- is silent.
    ///
    /// **Live tasks only, and not because the shape is acceptable on a closed
    /// one.** On a `done` task the same leftover is already reported: unticked by
    /// `APLAN038`, ticked by `APLAN039`. What was missing is the window between
    /// authoring and closing, which is where the author can still fix it in the
    /// edit they are already making.
    ///
    /// **Ticked or not**, for the same reason: a tick does not make the scaffold
    /// survive less, it makes the line claim the instruction was met. `APLAN039`
    /// covers the ticked case on a closed task, so nothing is reported twice.
    ///
    /// **Measured before it was built** (`A1-299`): across 2,768 task files in
    /// sixteen plans on this host, two carry the shape, one of them in this
    /// repository's own plan. Not a rule with nothing to check, and not one that
    /// would report a quarter of the corpus either.
    ///
    /// A warning, for `APLAN038`'s reason: an adopter meets it on tasks they have
    /// already written, and `deny_warnings` is how a repository says every gate
    /// blocks.
    ScaffoldLeftover => SCAFFOLD_LEFTOVER = "APLAN043", Warning,
        "a live task carries the scaffold's own prompt beside the criteria its author wrote",
        points_at: None;
    /// A closed task claims less of the released version than its level implies,
    /// and says nothing about why (`A1-312`, ADR-0034).
    ///
    /// A task's level already supplies a floor -- a unit of work `fix`, a cut
    /// `feature` -- so the ordinary task declares nothing and the plan still
    /// knows what it owes. Claiming *below* that floor is the author overriding
    /// the plan's own reading, which is a thing they are entitled to do and a
    /// thing a reviewer should be told about: a pilot log really does change
    /// nothing that ships.
    ///
    /// **What it is not.** Not a demand that every task declare an impact --
    /// that is unadoptable at three hundred tasks and produces the lazy
    /// declarations `semantic-release` is criticised for. And not a check that
    /// the claim is *true*: nothing hermetic can know whether a task broke an
    /// API, which is `verify --execute`'s job against a real checker.
    ///
    /// Off unless `version.require_impact` asks for it, like `APLAN009`'s
    /// estimate. A warning rather than an error, because an adopter meets it on
    /// tasks written before the key existed, and `deny_warnings` is how a
    /// repository says every gate blocks.
    VersionImpactUnexplained => VERSION_IMPACT_UNEXPLAINED = "APLAN044", Warning,
        "a closed task claims less of the released version than its level implies, without saying why",
        points_at: Some("version_impact");
    /// A *live* task's traced section has changed since anybody last touched the
    /// task, so its premise moved while it waited (`A1-301`, `p0000000000005`
    /// item 22).
    ///
    /// `APLAN018` watches a closed task's traced section against the stamp taken
    /// at closure. A live task has no stamp -- one is taken at `close` -- so
    /// nothing watched it at all, and a task can sit for months describing a
    /// document that has since said something else. The fleet audit found three
    /// live tasks tracking work that had already shipped, all passing `check`
    /// and `verify`, and needed a human reading code to see it.
    ///
    /// **Measured before it was built.** Across 2,271 task files in thirteen
    /// plans, every cheap structural signal for "this task is already done" has
    /// *no subjects at all*: no live task cites evidence, none carries a trace
    /// stamp, none has all its acceptance criteria ticked. This is the one signal
    /// with a real population, and it is narrow: of the live traced anchors that
    /// resolve at both ends, about a quarter have moved. Comparing whole
    /// documents instead would fire on 91% of them, which is a rule nobody reads.
    ///
    /// **Against the task's last commit, not its first.** Editing a task is
    /// evidence somebody re-read it, so touching the task clears this the way
    /// `--restamp` clears `APLAN018`. A task file that was never committed has no
    /// baseline and is left alone rather than guessed at.
    ///
    /// **Two things it cannot see, and says so rather than going quiet.** The
    /// traced document is read through `traces_root`, because a rule that looked
    /// at the repository root while the plan meant `docs/` would find nothing and
    /// report nothing -- a gate switched off without saying so. And a shallow
    /// clone cannot answer when a task was last touched at all: every path is
    /// added by the graft commit, so the baseline is `HEAD` and nothing can
    /// differ from it. Measured: a depth-1 clone of a repository where this
    /// fires reported nothing. It now reports the checkout instead, once, the
    /// way the commit resolver does (`A1-253`).
    ///
    /// **Its own code rather than widening `APLAN018`.** The subject differs (live
    /// against closed), the baseline differs (a commit against a stored
    /// fingerprint), and the remedy differs: `APLAN018` says re-review and
    /// re-stamp a claim, this says the ground your unstarted work rests on has
    /// moved and may already have been built. A warning, and `verify`-tier
    /// because it needs the repository (`A1-290` asks that a new code be argued
    /// for; this is the argument).
    StalePremise => STALE_PREMISE = "APLAN045", Warning,
        "a live task's traced section is unchanged since the task was last touched",
        points_at: Some("traces");
    /// A `done` task whose ratifier has not finished (`A1-306`, ADR-0036).
    ///
    /// `ratified_by` is the half of `blocked_by` that constrains *counting
    /// done* rather than *starting*: somebody must bless the result, and the
    /// work itself can be built, tested and gated before they do. `ready`
    /// therefore ignores it, which is the whole point -- the sixth pilot had
    /// eleven such edges drawn as `blocked_by`, five of the artefacts already
    /// existed and were gated in CI, and the plan called them blocked
    /// (pre-public adoption evidence).
    ///
    /// **This is where the gate went.** Re-pointing those edges to free `ready`
    /// risked losing the release gate they were also serving, and it survived
    /// only by accident, because the release happened to block on the cut
    /// holding the chain. An edge doing two jobs is exactly what this tool
    /// should make visible rather than hide, so the gate is stated here instead
    /// of inferred: the work may proceed, and it may not be called finished
    /// until the ratifier is.
    ///
    /// An *error*, and the same severity and shape as `APLAN007` for a producer
    /// edge, because a `done` task whose blessing has not happened is a status
    /// making a claim the plan contradicts.
    Unratified => UNRATIFIED = "APLAN046", Error,
        "a done task's ratifiers are done",
        points_at: Some("ratified_by");
    /// `APLAN047`: a `contains` edge is usable -- it names a milestone that
    /// exists, and the containment does not close a loop (`A1-333`, ADR-0042).
    ///
    /// Containment is the third thing `blocked_by` was asked to mean, and the
    /// first two already have their refusals: `APLAN003` for an id that names
    /// nothing, `APLAN006` for a dependency cycle. This one is theirs for the
    /// new edge, and it is separate rather than folded in because the remedy
    /// differs: a `contains` loop is a *modelling* mistake -- two checkpoints
    /// each claiming the other's work -- and the fix is to decide which
    /// contains which, not to break a dependency.
    ///
    /// **Its absence was measured, not theorised.** ADR-0042 claimed "the
    /// recursion terminates because `contains` is acyclic and refused
    /// otherwise", and on the day it was written nothing refused it: a
    /// two-milestone loop passed `check` cleanly. The rollup did not hang --
    /// a visited set stops that -- so the plan reported numbers nobody could
    /// reconcile instead of reporting the defect.
    ///
    /// An *error*, and not a warning, because a rollup over a loop has no
    /// defensible answer: whichever checkpoint is walked first swallows the
    /// other, and the number changes with iteration order.
    ContainsUnusable => CONTAINS_UNUSABLE = "APLAN047", Error,
        "a `contains` names a milestone that exists, and closes no loop",
        points_at: Some("contains");
    /// `APLAN048`: the artifact's version has moved as far as the tasks closed
    /// since the last release require (`A1-313`, ADR-0034, ADR-0038).
    ///
    /// This is the failure the whole version feature exists to prevent, at the
    /// only moment it can still be prevented. A breaking change that shipped in
    /// a patch release is not discovered by reading the tag; it is discovered by
    /// a consumer, after the fact, and no amount of process catches it then.
    ///
    /// **Both halves are read rather than believed**, which is why it is
    /// `verify` and not `check`: what closed since the last release comes from
    /// walking history to the nearest release tag (ADR-0038), and the version
    /// shipping today comes from a command the repository declares (ADR-0040).
    /// With no `version.current` there is nothing to compare and the rule is
    /// silent -- the same opt-in `url.check` and `test.list` have, and for the
    /// same reason: a key nobody set is not a gate nobody may switch off.
    ///
    /// **It fires on a version that moved, not on one that has not.** Between
    /// releases a tree ordinarily carries the number it last shipped, and
    /// reporting that as a defect would make this rule fire on every commit
    /// after the first closed task -- a gate nobody could leave on. What it
    /// catches is the bump that is too small: `0.3.0` to `0.3.1` with a
    /// `breaking` task in the window. A release that was never prepared at all
    /// is `ci/scripts/release.sh`'s refusal at tag time, where the tag is the
    /// thing making the claim.
    ///
    /// An *error*, because a version number is a compatibility promise to
    /// somebody who is not in the room.
    VersionBehind => VERSION_BEHIND = "APLAN048", Error,
        "the released version has moved as far as the closed tasks require",
        points_at: Some("version_impact");
    /// `APLAN049`: the artifact agrees with what the plan claims about
    /// compatibility (`A1-314`, ADR-0034).
    ///
    /// Every rung above this one takes the author's word. `APLAN044` checks the
    /// claim is well formed and `APLAN048` checks the number moved far enough
    /// for the claims made -- but a task that removed a public function and
    /// wrote `fix` passes both. That is the gap between "a field is filled in"
    /// and "the field is true", which is what this whole tool is about.
    ///
    /// **It runs the checker the repository declares and reports what it said.**
    /// `cargo semver-checks check-release` already exits non-zero on a breaking
    /// change and other ecosystems have equivalents; nothing here reimplements
    /// API diffing, exactly as nothing here reimplements a test runner. The
    /// question asked is the one such a tool answers natively -- *is this
    /// compatible with the last release* -- because a command that had to print
    /// `none|fix|feature|breaking` would be this project teaching an ecosystem
    /// its vocabulary (`A1-037`).
    ///
    /// **It fires in one direction only.** A checker that finds a break where
    /// the plan claims less is a finding; a checker that finds none where the
    /// plan claims `breaking` is silent, because ADR-0034 allows claiming more
    /// than a floor and because an API differ cannot see a behavioural break at
    /// all. Reporting an over-claim would punish the one habit that is always
    /// safe.
    ///
    /// A checker that could not be *launched* is a fault of the configuration,
    /// reported as itself, and no claim is called false on the strength of it --
    /// the distinction `A1-066` and `A1-300` both turn on.
    ///
    /// An *error*, and off unless `version.check` is declared: the same opt-in
    /// `url.check` and `test.list` have.
    ImpactContradicted => IMPACT_CONTRADICTED = "APLAN049", Error,
        "the artifact agrees with what the plan claims about compatibility",
        points_at: Some("version_impact");
    /// `APLAN050`: a recorded `version` is on a release, and is a version
    /// (`A1-339`, ADR-0038).
    ///
    /// A release node carries the version it produced, written when it closed,
    /// so a later run can ask what a release contained without re-deriving from
    /// a plan that has since moved (ADR-0038 decision 1).
    ///
    /// **A cut is not a delivery.** This repository has eight done cuts against
    /// three tags, which is what a cut being a planning bucket looks like, and a
    /// `version` on one would read as a claim that it shipped. The field says
    /// what a *release* produced or it says nothing.
    ///
    /// **And it has to be a version.** A value the scheme cannot read is a
    /// record nobody can compare against: the whole point of writing it down is
    /// that a later run reads it back, and `0.4` or `next` or an empty string
    /// would be a record that reads as an absence at the moment it is needed.
    ///
    /// An *error*, and hermetic: both halves are answerable from the plan alone.
    VersionRecordUnusable => VERSION_RECORD_UNUSABLE = "APLAN050", Error,
        "a recorded `version` is on a release, and is a version this tool can read",
        points_at: Some("version");
    /// `APLAN051`: a tag one edit from a word that changes what this tool does
    /// (`A1-212`, ADR-0014).
    ///
    /// Five bare words are load-bearing: `milestone` makes a task a cut,
    /// `release` makes it a checkpoint over cuts (ADR-0022), `deprecation` and
    /// `removal` drive the runway rules, and `cross-cutting` waives `APLAN022`.
    /// They live in the same namespace as an adopter's own tags and nothing
    /// declared them, so a typo did not misname anything -- it silently changed
    /// what the plan *is*.
    ///
    /// **Measured on two-task plans, not theorised** (`A1-212`). `deprecatoin`
    /// for `deprecation`: `aplan deprecations` answers "this plan declares no
    /// deprecation runway" and exits `0`. `milestne` for `milestone`: the cut
    /// dissolves, `roadmap` reports `0 cut(s)`, and `check` then advises adding
    /// an `effort_days` estimate to what it no longer knows is a checkpoint --
    /// the opposite of the rule `APLAN021` exists to state.
    ///
    /// Also the other direction: a `tag_families` entry that lists one of the
    /// five is declaring a vocabulary over a word the tool already acts on, and
    /// the tool's meaning is the one that applies. Reported once, against the
    /// configuration, rather than left as two rules disagreeing about the same
    /// word.
    ///
    /// A *warning*, for `APLAN026`'s reason: a tag one edit from a reserved word
    /// is as likely to be a word the repository means -- `releases` beside
    /// `release` -- as a mistake, and the repository sets `deny_warnings` if it
    /// wants the answer to block.
    ReservedTagNearMiss => RESERVED_TAG_NEAR_MISS = "APLAN051", Warning,
        "a tag is not one edit from a word that changes what this tool does",
        points_at: Some("tags");
    /// `APLAN052`: `cross-cutting` means "in no cut", so it is not on a task in
    /// one (`A1-212`).
    ///
    /// `APLAN022` reads "a member of exactly one cut, *or* tagged
    /// `cross-cutting`". The tag means "belongs to no cut", so on a task that
    /// *is* in one it claims something the task's own membership contradicts.
    ///
    /// **It used to mask, and that is fixed rather than only reported**
    /// (`A1-212`, audit). `APLAN022` took the tag as the answer whatever the
    /// count, so a task in *two* cuts -- an error, and a real defect -- passed
    /// as clean. The waiver now applies to the case the tag describes and no
    /// other, so that task is reported by `APLAN022` as the error it is, and by
    /// this rule for the claim that is wrong either way.
    ///
    /// **This is the half a rule can say with confidence.** On a task in no cut
    /// the same tag is genuinely ambiguous -- "belongs to no cut" and "concerns
    /// several components" are indistinguishable by looking -- and a rule that
    /// asks a question about forty-odd tasks is a rule people switch off. So
    /// that case is silent and this one is not, which is the asymmetry `A1-212`
    /// was filed to find.
    ///
    /// A *warning*: the remedy is a judgement between renaming the tag and
    /// removing it, and an adopter meets this on tasks they already wrote.
    CrossCuttingInACut => CROSS_CUTTING_IN_A_CUT = "APLAN052", Warning,
        "`cross-cutting` is not on a task that is a member of a cut",
        points_at: Some("tags");
    /// `APLAN053`: a task says work on it has started and the repository holds
    /// nothing it claims (`A1-359`).
    ///
    /// **This is the status the product's own argument could not otherwise
    /// defend.** `done` is believed because `verify` resolves what it cites;
    /// that is the whole premise. `in_progress` arrives as a claim with nothing
    /// to resolve against, and a stale claim is precisely the failure this tool
    /// exists to expose in other people's plans. `A1-224` refused
    /// `close --criteria-met` on the neighbouring ground that a claim is not
    /// proof, and storing `in_progress` unchecked is the same trade made the
    /// other way.
    ///
    /// So the status is kept and checked rather than removed: the premise is not
    /// "do not store claims", it is "do not take claims on trust". Nothing
    /// leaves the schema and nothing an adopter wrote stops loading.
    ///
    /// **Narrow by construction**, which is `APLAN052`'s argument applied here.
    /// The derived answer is "does the repository hold anything this task claims
    /// responsibility for", read from the index so a file that has been written
    /// and staged counts as work -- a task started five minutes ago that has
    /// created its files is silent. What it reports is the unambiguous case: a
    /// task declared under way whose declared `paths` match nothing that exists.
    /// It deliberately does *not* ask whether the right work happened, because
    /// a broad glob makes that question unanswerable and `aplan vacuity` is
    /// where the breadth of a `paths` declaration is reported.
    ///
    /// Silent where the task declares no `paths`: there is then nothing to judge
    /// the claim by, and a rule that fires on every task without `paths` would
    /// be reporting `APLAN023` a second time.
    ///
    /// A *warning*, for `A1-134`'s reason: a task may legitimately be under way
    /// before anything of it exists, so whether that blocks is the repository's
    /// call through `deny_warnings` rather than this rule's.
    UnstartedInProgress => UNSTARTED_IN_PROGRESS = "APLAN053", Warning,
        "a task that is `in_progress` has something in the repository to show for it",
        applies: &[Binding::InProgress], binds: &[Binding::Paths],
        points_at: Some("status");
    /// A mutable `file` evidence referent still has the contents that were
    /// reviewed when the citation was recorded (`A1-196`).
    ///
    /// `APLAN015` owns reference rot: a path, revision or anchor no longer
    /// resolves. This is the other failure mode, where the exact same reference
    /// still resolves and now names different proof. Unversioned `file`
    /// citations are stamped from committed blobs. An `@rev` that resolves as
    /// the object id it spells is pinned to an immutable tree and needs no
    /// stamp; a symbolic revision, including an all-hex branch or tag, can move
    /// and is stamped.
    ///
    /// Advisory and opt-in through `detect_drift`, matching `APLAN018`. A change
    /// does not prove the task is wrong; it proves the cited proof needs review.
    /// A task closed before stamping existed carries no fingerprint and remains
    /// unverifiable rather than being retrospectively declared invalid. Once a
    /// task carries any evidence stamp, a mutable citation missing its own stamp
    /// is reported: partial coverage is distinguishable from historical absence.
    EvidenceDrift => EVIDENCE_DRIFT = "APLAN054", Warning,
        "a closed task's mutable file evidence is covered by a current fingerprint",
        applies: &[Binding::Done, Binding::Evidence(crate::task::EvidenceKind::File)],
        binds: &[Binding::EvidenceFingerprints],
        points_at: Some("evidence");
    /// An author-entered `captured` date appears on machine-resolved evidence
    /// (`A1-197`).
    ///
    /// Only a `note` is the author's inline assertion, so only a note gains
    /// provenance from the date the author made it. A commit has immutable
    /// identity, a file has a content stamp, URL and test resolvers report the
    /// current run, and a future historical report needs its own subject and
    /// producer identity. Accepting a date on those kinds would make ignored
    /// metadata look like checked freshness.
    InapplicableCapture => INAPPLICABLE_CAPTURE = "APLAN055", Error,
        "captured dates are used only for authored note evidence",
        points_at: Some("evidence");
    /// `APLAN056`: a task's free-text owner belongs to the repository's
    /// declared owner vocabulary (`A1-190`).
    ///
    /// Without `owners`, every spelling stays legal and the rule is silent, so
    /// an existing repository does not acquire policy by upgrading. Once a
    /// repository declares the vocabulary, however, every owner lives in that
    /// one namespace: unlike tags, there is no second class of unrelated values
    /// that must remain outside it. An undeclared owner is therefore reported
    /// whether or not it is close enough to suggest a correction.
    ///
    /// A warning, matching `APLAN026`: an unfamiliar spelling may be a typo or
    /// a real person missing from the declaration. `deny_warnings` lets the
    /// repository make that judgement block its gate.
    OwnerVocabulary => OWNER_VOCABULARY = "APLAN056", Warning,
        "a task's owner belongs to the declared owner vocabulary",
        points_at: Some("owner");
    /// A capture file is one canonical, valid and cross-record-consistent V1
    /// inbox record (`A1-514`, ADR-0063).
    ///
    /// One rule owns the record boundary because its remedy is indivisible:
    /// repair the inbox before a read or mutation can select a trustworthy
    /// snapshot. Syntax, canonical identity, timestamps, lifecycle fields and
    /// source uniqueness are all properties of that same input. Captures do
    /// not become tasks, so none of the task-only rules can report them.
    CaptureInvalid => CAPTURE_INVALID = "APLAN057", Error,
        "every inbox entry is one canonical, valid and cross-record-consistent V1 capture",
        points_at: None;
}

/// A repository's verdict policy: what makes a report fail its build.
///
/// Severity is the rule's, single-valued, declared once on its `define_rules!`
/// row and read by `facts`, the docs and the catalogue. Tolerance is the
/// repository's, and lives here. Keeping them apart is what lets a plan
/// mid-migration see every finding without being stopped by one, which is the
/// case `APLAN022` could not serve: the check most useful *during* a migration
/// was the one a migration could not turn on until it was finished
/// (pre-public adoption evidence, item 3).
#[derive(Debug, Default, Clone, Serialize)]
pub struct Verdict {
    /// Whether a warning blocks.
    pub deny_warnings: bool,
    /// Rule codes this repository hears without being stopped by. Reported at
    /// their declared severity, and excluded from the verdict.
    pub advisory: Vec<String>,
}

impl Verdict {
    #[must_use]
    pub fn is_advisory(&self, code: &str) -> bool {
        self.advisory.iter().any(|c| c == code)
    }
}

/// Where in a file a finding sits.
///
/// **Line and column, not the byte span the HLD planned.** Section 7 asked for
/// a span "from day one" so a renderer could underline `/status` rather than
/// name it. Nothing renders an underline: all five consumers take a line and a
/// column and nothing else -- GitHub's `line=`/`col=`, SARIF's `startLine`/
/// `startColumn`, and the `path:line:` convention `check-docs` already prints.
/// A span would also have to come from somewhere, and `yaml_compat` hands
/// back no marks for a parse that *succeeded*; getting them means a second YAML
/// parser inside `aplan-core`, which is a large dependency to carry for a
/// rendering nobody performs. This is the part of the span every renderer would
/// have used, and a span can still be added under it later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    /// 1-based line, counted from the start of the file rather than from the
    /// start of the frontmatter, because that is what an editor and a CI
    /// annotation both mean by a line.
    pub line: usize,
    /// 1-based column of the field the finding is about.
    pub column: usize,
}

/// A single finding.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    /// Task this concerns, when it concerns exactly one.
    pub task: Option<String>,
    /// File the task was loaded from.
    pub file: Option<PathBuf>,
    /// Human-readable explanation.
    pub message: String,
    /// A directive an autonomous agent can act on without further context.
    pub agent_instruction: Option<String>,
    /// An exact edit the tool computed and can pass through its mutation gates.
    ///
    /// `None` means the remedy still requires judgement; it does not mean the
    /// prose instruction is absent.
    pub repair: Option<Repair>,
    /// Where in [`file`] the finding sits, when exactly one place answers.
    ///
    /// `None` is a real answer, not a missing one: a rule about the shape of
    /// the whole plan has no line, and one about a field whose text appears
    /// twice has no single line. Pointing at line 1 in either case would be a
    /// guess, and the loader already refuses to guess about which line an
    /// author meant (`source_literal`).
    ///
    /// [`file`]: Diagnostic::file
    pub location: Option<Location>,
    /// Captured evidence this finding carries, one line per element
    /// (`A1-365`).
    ///
    /// **Separate from [`message`] because the two are different kinds of
    /// text.** A message is prose, and every surface is free to fold it to its
    /// own width; this is output a command printed, where the line breaks carry
    /// meaning and folding would reflow an assertion into nonsense. Keeping
    /// them apart is also what lets each surface present it in the shape its
    /// consumer expects -- indented lines in a terminal, an array in JSON, a
    /// property in SARIF -- rather than every consumer parsing it back out of a
    /// sentence.
    ///
    /// Empty for every rule but `APLAN014`, which is why it is a `Vec` rather
    /// than an `Option`: the common case is "nothing to add", and `is_empty()`
    /// says that without a nested match.
    ///
    /// Whatever is in here has already been sanitized and bounded by whoever
    /// produced it. A renderer may still escape it for its own format, but it
    /// must not assume it is short.
    ///
    /// [`message`]: Diagnostic::message
    pub detail: Vec<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            task: None,
            file: None,
            message: message.into(),
            agent_instruction: None,
            repair: None,
            location: None,
            detail: Vec::new(),
        }
    }

    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Warning,
            task: None,
            file: None,
            message: message.into(),
            agent_instruction: None,
            repair: None,
            location: None,
            detail: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_task(mut self, id: impl Into<String>) -> Self {
        self.task = Some(id.into());
        self
    }

    #[must_use]
    pub fn with_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.file = Some(path.into());
        self
    }

    /// Place the finding at a line and column in its file.
    #[must_use]
    pub fn at(mut self, location: Location) -> Self {
        self.location = Some(location);
        self
    }

    #[must_use]
    pub fn with_agent_instruction(mut self, s: impl Into<String>) -> Self {
        self.agent_instruction = Some(s.into());
        self
    }

    /// Attach one exact edit this diagnostic has already computed.
    #[must_use]
    pub fn with_repair(mut self, repair: Repair) -> Self {
        self.repair = Some(repair);
        self
    }

    /// Attach captured evidence, one element per line (`A1-365`).
    ///
    /// The caller has already bounded and sanitized it; this only carries it.
    #[must_use]
    pub fn with_detail(mut self, lines: Vec<String>) -> Self {
        self.detail = lines;
        self
    }
}

/// The result of a validation run.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
    pub tasks_checked: usize,
}

impl Report {
    /// Every error, whatever the repository tolerates.
    ///
    /// Only [`is_clean`](Report::is_clean) reads it: the raw counts are not what
    /// a run reports, because a count that disagrees with the exit code beside it
    /// is the defect `A1-112` fixed. Use [`blocking`](Report::blocking) for
    /// anything a person or a consumer reads. The `warnings` twin was deleted
    /// when its last caller became policy-aware, rather than left available.
    pub fn errors(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    /// Exit code contract: `0` clean, `1` at least one error.
    pub fn is_clean(&self) -> bool {
        self.errors() == 0
    }

    /// Whether this report fails the build under the repository's full policy.
    ///
    /// The verdict, in one place, because it had two: the CLI computed it to pick
    /// an exit code and the MCP surface did not compute it at all, so a plan the
    /// CLI denied came back to an agent as `errors: 0, warnings: 1` with
    /// `isError: false`. The agent read the obvious field, concluded the plan
    /// passed, and handed CI a build that failed on the gate it thought it had
    /// just run (`A1-101`, pre-public adoption evidence item 18).
    ///
    /// Note what this is *not*: a third outcome. A repository that sets
    /// `deny_warnings` is saying a warning should block, not that it wants a new
    /// exit code, and `1` goes on meaning "at least one finding" (ADR-0007).
    ///
    /// It takes the whole [`Verdict`] rather than one flag, and the flag-only
    /// form was deleted rather than kept beside it: `close` read that form, so it
    /// refused citations `verify` accepted the moment a repository made a rule
    /// advisory, which is the disagreement `A1-109` exists to prevent. A
    /// predicate that reads half a policy invites the next instance of it.
    ///
    /// The severity stays the rule's own: `APLAN022` is an error here and in
    /// `facts` and in the catalogue whatever a repository configures, because a
    /// consumer asserting what a rule *says* must not be asserting something a
    /// config can change. What a repository configures is the **verdict**, which
    /// is where `deny_warnings` already lives, and this is the same knob turned
    /// the other way (`A1-112`).
    #[must_use]
    pub fn denied_by(&self, policy: &Verdict) -> bool {
        self.blocking(policy, Severity::Error) > 0
            || (policy.deny_warnings && self.blocking(policy, Severity::Warning) > 0)
    }

    /// How many findings of one severity count against the verdict.
    ///
    /// One implementation, because there were four: the human summary, the JSON
    /// report, the `denied:` note and the MCP payload each filtered the same way,
    /// and a count that disagrees with the exit code beside it is the defect
    /// `A1-112` fixed rather than a style problem.
    #[must_use]
    pub fn blocking(&self, policy: &Verdict, severity: Severity) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| !policy.is_advisory(d.code) && d.severity == severity)
            .count()
    }

    /// How many findings this policy heard but did not stop for.
    ///
    /// Reported, always: a build that passed because a rule was downgraded must
    /// not look like a build that had nothing to say.
    #[must_use]
    pub fn advisory_count(&self, policy: &Verdict) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| policy.is_advisory(d.code))
            .count()
    }
}

#[cfg(test)]
mod tests {

    /// The verdict, in every combination that decides it.
    ///
    /// `A1-101` put this in `aplan-core` so the CLI's exit code and the MCP
    /// payload cannot disagree, and then tested it only through the MCP layer.
    /// The crate that owns a predicate owns its truth table.
    #[test]
    fn denied_is_the_verdict_both_surfaces_read() {
        let deny = || Verdict {
            deny_warnings: true,
            advisory: Vec::new(),
        };
        let report = |errors: usize, warnings: usize| Report {
            tasks_checked: 1,
            diagnostics: (0..errors)
                .map(|_| Diagnostic::error(code::CYCLE, "e"))
                .chain((0..warnings).map(|_| Diagnostic::warning(code::MISSING_EFFORT, "w")))
                .collect(),
        };

        // An error denies whatever the policy says: the policy is about warnings.
        assert!(report(1, 0).denied_by(&Verdict::default()));
        assert!(report(1, 0).denied_by(&deny()));

        // A warning denies only where the repository asked for it.
        assert!(!report(0, 1).denied_by(&Verdict::default()));
        assert!(report(0, 1).denied_by(&deny()));

        // A clean plan is clean under either policy, which is the case a false
        // positive here would break for every adopter at once.
        assert!(!report(0, 0).denied_by(&Verdict::default()));
        assert!(!report(0, 0).denied_by(&deny()));

        // And `denied` never contradicts `is_clean` about errors.
        assert!(report(2, 3).denied_by(&Verdict::default()) && !report(2, 3).is_clean());

        // The third dimension, and the one a flag-only predicate could not see:
        // a downgraded rule stops denying under either warning policy, and the
        // rule beside it goes on denying (`A1-112`).
        let advisory = |deny_warnings| Verdict {
            deny_warnings,
            advisory: vec![code::CYCLE.to_string(), code::MISSING_EFFORT.to_string()],
        };
        assert!(!report(1, 0).denied_by(&advisory(false)));
        assert!(!report(1, 1).denied_by(&advisory(true)));
        let mixed = Verdict {
            deny_warnings: false,
            advisory: vec![code::MISSING_EFFORT.to_string()],
        };
        assert!(
            report(1, 1).denied_by(&mixed),
            "the error was not downgraded"
        );
    }
    use super::*;

    #[test]
    fn rule_codes_are_unique_and_wellformed() {
        let mut seen = std::collections::BTreeSet::new();
        for r in Rule::ALL {
            let c = r.code();
            assert!(seen.insert(c), "duplicate rule code {c}");
            let n = c.strip_prefix("APLAN").expect("code starts with APLAN");
            assert!(
                n.len() == 3 && n.bytes().all(|b| b.is_ascii_digit()),
                "code {c} is not APLAN + three digits"
            );
            assert!(!r.title().is_empty(), "{c} has an empty title");
        }
    }

    /// The `code::` constants are generated from the same table as [`Rule`], so
    /// they cannot drift; pin a couple of the mappings so a future refactor that
    /// *replaces* the macro still has to keep them equal.
    #[test]
    fn code_constants_agree_with_rules() {
        assert_eq!(Rule::Schema.code(), code::SCHEMA);
        assert_eq!(
            Rule::DoneWithoutEvidence.code(),
            code::DONE_WITHOUT_EVIDENCE
        );
        assert_eq!(Rule::DroppedBlocker.code(), code::DROPPED_BLOCKER);
        assert_eq!(Rule::CommitIrrelevant.code(), code::COMMIT_IRRELEVANT);
        assert_eq!(Rule::TestFailed.code(), code::TEST_FAILED);
        assert_eq!(Rule::WeakEvidence.code(), code::WEAK_EVIDENCE);
        assert_eq!(Rule::FileIrrelevant.code(), code::FILE_IRRELEVANT);
        // Every rule's `code()` is one of the module constants and vice versa is
        // structurally guaranteed by the macro; this pins the count so dropping
        // a row is a visible change.
        assert_eq!(Rule::ALL.len(), 57);
    }

    /// Severity is declared per rule in the macro. Pin the warnings so a
    /// future edit that silently promotes one to an error is caught here (the
    /// e2e audit separately asserts each firing site agrees with `severity()`).
    #[test]
    fn only_the_advisory_rules_are_warnings() {
        let warnings: Vec<&str> = Rule::ALL
            .iter()
            .filter(|r| r.severity() == Severity::Warning)
            .map(|r| r.code())
            .collect();
        assert_eq!(
            warnings,
            vec![
                "APLAN009", "APLAN016", "APLAN018", "APLAN019", "APLAN021", "APLAN023", "APLAN024",
                "APLAN025", "APLAN026", "APLAN027", "APLAN029", "APLAN035", "APLAN036", "APLAN037",
                "APLAN038", "APLAN039", "APLAN040", "APLAN041", "APLAN042", "APLAN043", "APLAN044",
                "APLAN045", "APLAN051", "APLAN052", "APLAN053", "APLAN054", "APLAN056"
            ]
        );
    }
}

#[cfg(test)]
mod points_at_tests {
    use super::*;

    /// The failure this guards against is silent and permanent. `points_at` is
    /// a string looked up in the frontmatter, so `blockedby` for `blocked_by`
    /// never matches, never errors, and quietly leaves every finding from that
    /// rule with no line -- indistinguishable from a rule that declared it had
    /// none. Checked against the schema derived from `Task`, which is the same
    /// authority the loader validates against, so the two cannot disagree.
    #[test]
    fn every_declared_field_is_a_real_frontmatter_field() {
        let schema = crate::load::derived_schema();
        let properties = schema["properties"]
            .as_object()
            .expect("the derived schema has properties");

        let mut wrong = Vec::new();
        let mut named = 0_usize;
        for rule in Rule::ALL {
            let Some(field) = rule.points_at() else {
                continue;
            };
            named += 1;
            if !properties.contains_key(field) {
                wrong.push(format!("{} points at `{field}`", rule.code()));
            }
        }

        assert!(
            named > 20,
            "only {named} rules name a field, so this audit is not reaching them"
        );
        assert!(
            wrong.is_empty(),
            "{} rule(s) point at a field a task does not have, so their findings will \
             silently never carry a line: {}\nThe schema has: {}",
            wrong.len(),
            wrong.join(", "),
            properties.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }

    /// `None` has to be a decision somebody made, and the rules that carry it
    /// are the ones with nothing on the task to point at: a plan-shaped rule,
    /// a `.aplan.yml`-shaped one, and the file that did not parse.
    #[test]
    fn the_rules_without_a_field_are_the_ones_that_cannot_have_one() {
        let without: Vec<&str> = Rule::ALL
            .iter()
            .filter(|r| r.points_at().is_none())
            .map(|r| r.code())
            .collect();
        assert_eq!(
            without,
            vec![
                // the file did not parse, so it has no fields
                "APLAN001",
                // several fields in one sentence: any single one is a guess
                "APLAN021", // fixed by editing a different task
                "APLAN022", // about a declared family, not about any one task
                "APLAN029", // about `.aplan.yml`, not about a task
                "APLAN036", "APLAN037",
                // the one rule that reads the body: its line is the first
                // unticked box, which no frontmatter field names (`A1-224`)
                "APLAN038",
                // and the one that reads a *line* of it, for the same reason
                // (`A1-271`)
                "APLAN039",
                // about a document outside the plan, so no task field names it
                // (`A1-273`)
                "APLAN041",
                // the third that reads the body, and the same answer: its line is
                // the leftover prompt, which no frontmatter field names
                // (`A1-299`)
                "APLAN043",
                // about a versioned inbox record rather than task frontmatter;
                // the diagnostic still carries the capture file path
                "APLAN057",
            ],
            "the set of rules with no location changed; if that is deliberate, say so here"
        );
    }

    #[test]
    fn from_code_agrees_with_code() {
        for rule in Rule::ALL {
            assert_eq!(
                Rule::from_code(rule.code()),
                Some(*rule),
                "{} did not round-trip",
                rule.code()
            );
        }
        assert_eq!(Rule::from_code("APLAN999"), None);
        assert_eq!(Rule::from_code(""), None);
    }
}
