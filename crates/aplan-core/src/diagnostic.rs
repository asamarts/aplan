//! Diagnostics. Every rule has a stable code so CI output, agent output, and
//! human output all refer to the same thing.

use std::fmt;
use std::path::PathBuf;

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

/// Define every invariant once, as an enum *variant*, a stable `code::`
/// constant, and a one-line title, from a single table, so the three can never
/// drift apart. Adding a rule is one row here.
macro_rules! define_rules {
    ($(
        $(#[$meta:meta])*
        $variant:ident => $const_name:ident = $code:literal, $severity:ident, $title:literal
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
    Schema => SCHEMA = "APLAN001", Error, "frontmatter parses and satisfies the schema";
    /// Two tasks share an `id`.
    DuplicateId => DUPLICATE_ID = "APLAN002", Error, "task ids are unique";
    /// `blocked_by` names a task that does not exist.
    UnknownDep => UNKNOWN_DEP = "APLAN003", Error, "blocked_by references a task that exists";
    /// The dependency graph contains a cycle.
    Cycle => CYCLE = "APLAN004", Error, "the dependency graph is acyclic";
    /// `status: done` with an empty `evidence` list.
    DoneWithoutEvidence => DONE_WITHOUT_EVIDENCE = "APLAN005", Error,
        "done carries at least one piece of evidence";
    /// A task lists itself in `blocked_by`.
    SelfDep => SELF_DEP = "APLAN006", Error, "no task blocks itself";
    /// A `done` task depends on a task that is not `done`.
    DoneBeforeBlocker => DONE_BEFORE_BLOCKER = "APLAN007", Error,
        "a done task does not depend on an unfinished one";
    /// A `traces` anchor does not resolve to a real file (and heading).
    DanglingTrace => DANGLING_TRACE = "APLAN008", Error, "traces anchors resolve to real headings";
    /// An active task carries no effort estimate.
    MissingEffort => MISSING_EFFORT = "APLAN009", Warning, "active tasks carry an effort estimate";
    /// Evidence of kind `note` without a `captured` date.
    UndatedNote => UNDATED_NOTE = "APLAN010", Error, "a note used as evidence carries a date";
    /// `commit` evidence names a SHA that does not resolve to a commit in this
    /// repository (L1 "Resolved"). Raised by `aplan verify`, never by `check`.
    CommitUnresolved => COMMIT_UNRESOLVED = "APLAN011", Error,
        "commit evidence resolves to a commit in this repository";
    /// `commit` evidence resolves, but its diff touches nothing in the task's
    /// `paths` (L2 "Relevant"), the citation is real but not bound to the task.
    CommitIrrelevant => COMMIT_IRRELEVANT = "APLAN012", Error,
        "commit evidence touches at least one of the task's paths";
    /// `test` evidence names a test that the configured runner does not list
    /// (L1 "Resolved"). Needs a `test` runner in `.aplan.yml`; without one,
    /// `test` evidence stays at L0, like `url` reachability.
    TestUnresolved => TEST_UNRESOLVED = "APLAN013", Error,
        "test evidence names a test the runner can find";
    /// `test` evidence names a test that the runner ran and it failed (L3
    /// "Executed"). Only under `aplan verify --execute`, which is off by default.
    TestFailed => TEST_FAILED = "APLAN014", Error, "test evidence names a test that passes";
    /// `file` evidence names a path that does not exist at the cited rev (or at
    /// `HEAD` when none is cited) (L1 "Resolved").
    FileUnresolved => FILE_UNRESOLVED = "APLAN015", Error, "file evidence resolves at the cited rev";
    /// A `done` task's evidence is weaker than the configured minimum: it has
    /// evidence, but none of an accepted kind, and no exempting tag. Off unless
    /// `.aplan.yml` sets `evidence.require_min_strength`. (`APLAN016`, url
    /// reachability, is reserved; it needs the network path, still off.)
    WeakEvidence => WEAK_EVIDENCE = "APLAN017", Error, "done evidence meets the configured minimum strength";
    /// A `done` task's traced document section has changed since the task was
    /// closed: its `trace_fingerprints` no longer match the live content. A
    /// *warning* (advisory), and off unless `.aplan.yml` sets `detect_drift`; a
    /// task with no stamp is unverifiable, not in violation (zero false positives).
    /// Cleared by re-reviewing and re-stamping (`aplan close --restamp`).
    TraceDrift => TRACE_DRIFT = "APLAN018", Warning, "a closed task's traced section is unchanged since closure";
    /// A live task is blocked by a `dropped` one, so it can never legitimately
    /// start: the plan is deadlocked.
    ///
    /// `APLAN011`-`APLAN019` are reserved for the planned evidence ladder
    /// (ROADMAP §10), which is why this rule, shipped earlier, takes `020`. A
    /// code is a contract with consumers; it is not renumbered for tidiness.
    DroppedBlocker => DROPPED_BLOCKER = "APLAN020", Error, "a live task is not blocked by a dropped one";
    /// A task tagged `milestone` does not look like one: it has no members
    /// (`blocked_by` is empty, so the cut scopes no work), or it carries
    /// `effort_days` or `paths` (a milestone is a checkpoint, not work, and owns
    /// no files; ADR-0008). A *warning*, because the `milestone` tag silently
    /// excludes a task from every work-view (`ready`, `critical-path`, `summary`,
    /// `APLAN009`); this is how a real task mis-tagged `milestone`, or an empty
    /// cut, stops being a silent reclassification. Membership *totality* is the
    /// separate `APLAN022`.
    MilestoneHygiene => MILESTONE_HYGIENE = "APLAN021", Warning,
        "a milestone has members and carries no effort or paths";
    /// A non-milestone task belongs to no cut, or to more than one: it is in the
    /// `blocked_by` of zero, or of several, milestone meta-tasks (ADR-0008), and is
    /// not tagged `cross-cutting`. Off unless `.aplan.yml` sets
    /// `require_cut_membership`. Enforcing it makes a milestone's `done` prove the
    /// cut is *exhaustively* complete, not merely that its listed members are, and
    /// closes the gap `APLAN021` (empty/mis-tagged cuts) does not: a real task in
    /// no cut. `cross-cutting` is the escape hatch for a task that deliberately
    /// belongs to none (a quality gate spanning every cut).
    MembershipTotality => MEMBERSHIP_TOTALITY = "APLAN022", Error,
        "every task is a member of exactly one cut, or is tagged cross-cutting";
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

    #[must_use]
    pub fn with_agent_instruction(mut self, s: impl Into<String>) -> Self {
        self.agent_instruction = Some(s.into());
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
    pub fn errors(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    pub fn warnings(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count()
    }

    /// Exit code contract: `0` clean, `1` at least one error.
    pub fn is_clean(&self) -> bool {
        self.errors() == 0
    }
}

#[cfg(test)]
mod tests {
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
        // Every rule's `code()` is one of the module constants and vice versa is
        // structurally guaranteed by the macro; this pins the count so dropping
        // a row is a visible change.
        assert_eq!(Rule::ALL.len(), 20);
    }

    /// Severity is declared per rule in the macro. Pin the two warnings so a
    /// future edit that silently promotes one to an error is caught here (the
    /// e2e audit separately asserts each firing site agrees with `severity()`).
    #[test]
    fn only_the_advisory_rules_are_warnings() {
        let warnings: Vec<&str> = Rule::ALL
            .iter()
            .filter(|r| r.severity() == Severity::Warning)
            .map(|r| r.code())
            .collect();
        assert_eq!(warnings, vec!["APLAN009", "APLAN018", "APLAN021"]);
    }
}
