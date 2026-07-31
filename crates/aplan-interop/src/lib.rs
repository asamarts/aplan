//! Importers: read other tools' plans and proposal repositories into `aplan`
//! tasks.
//!
//! The policy every importer here obeys is ADR-0012, *an importer is lossy, and
//! says so*:
//!
//! 1. No new [`Status`](aplan_core::Status) variant for pipeline stages. A
//!    `todo` task already *is* accepted-but-not-implemented.
//! 2. A proposal's stage is a tag, never a status.
//! 3. Every field the task model does not hold produces a notice
//!    ([`Report`](report::Report)); silent omission is forbidden.
//! 4. An importer never asserts a `done` it cannot prove: a source's
//!    "implemented" is a claim, so it imports as `todo` plus a stage tag unless
//!    the caller opts in with [`ImplementedAs::Done`].
//! 5. Import writes new task files and nothing else, unless asked.
//!
//! Each source shape is a module implementing [`ProposalSource`], so a second
//! flavor is an addition rather than a refactor, and the policy above has a
//! single enforcement point.

pub mod cut;
pub mod emit;
pub mod kep;
pub mod report;

pub use report::{Notice, NoticeKind, Report};

use aplan_core::Task;

/// How an imported "implemented" claim is represented (ADR-0012 clause 4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ImplementedAs {
    /// The default and the honest one: a source that says "implemented" yields
    /// a `todo` task plus its stage tag, because the importer cannot prove the
    /// claim. Output satisfies any configured evidence strength.
    #[default]
    Todo,
    /// Opt-in. Yields `done` backed only by a dated `note`, the weakest kind, and
    /// records a [`NoticeKind::Provenance`] warning. This fails `APLAN017`
    /// wherever `require_min_strength` excludes `note`, by design.
    ///
    /// The date is carried here rather than derived, for two reasons that the
    /// codebase already settled. `APLAN010` requires a `note`'s `captured` date
    /// to be a real ISO-8601 `YYYY-MM-DD`, and `aplan close` refuses to author
    /// note evidence at all ("add it by hand rather than via `close`") because
    /// the tool does not invent provenance; there is deliberately no date
    /// dependency in the workspace. Requiring the caller to state the date keeps
    /// this crate pure and its output byte-identical across runs (HLD section
    /// 14), and makes a dateless `done` import unrepresentable rather than a
    /// runtime error.
    Done {
        /// ISO-8601 `YYYY-MM-DD` recorded as the note's `captured` date.
        as_of: String,
    },
}

/// Knobs a caller sets for one import run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// `phase` for every emitted task. Proposal repositories have no equivalent
    /// field, and `phase` is for rollups and ordering rather than gating, so a
    /// default is safe; the report states which value was used.
    pub phase: u32,
    /// How to treat a source's "implemented".
    pub implemented_as: ImplementedAs,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            phase: 1,
            implemented_as: ImplementedAs::default(),
        }
    }
}

/// The result of reading one source document.
///
/// Not `Eq`: [`Task`] carries a floating-point `effort_days`, so equality here
/// is `PartialEq` like the model it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    /// Tasks ready to be emitted, in source order.
    pub tasks: Vec<Task>,
    /// What was lost, weakened, or left for the user to place.
    pub report: Report,
}

/// Why a source could not be read at all.
///
/// A *dropped field* is not an error: it is a [`Report`] notice, because the
/// import still succeeded. This type is for input that cannot yield tasks.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// The document is not valid for this flavor.
    #[error("{source_name}: {message}")]
    Malformed {
        /// The source as the user named it.
        source_name: String,
        /// What was wrong.
        message: String,
    },
    /// The document parsed but carries no importable item.
    #[error("{source_name}: no importable proposal found")]
    Empty {
        /// The source as the user named it.
        source_name: String,
    },
}

/// One source shape `aplan` can read.
///
/// Implementors live in their own module (`kep`, and later `backlog_md`,
/// `task_master`, `spec_kit`, `beads`), so adding a flavor never edits an
/// existing one.
pub trait ProposalSource {
    /// The flavor name as it appears in `--format`.
    fn flavor(&self) -> &'static str;

    /// Read one source document into tasks plus the conversion report.
    ///
    /// `source_name` is how the user referred to the document; it labels every
    /// notice so a multi-file import stays attributable.
    fn parse(
        &self,
        source_name: &str,
        contents: &str,
        options: &Options,
    ) -> Result<Import, ImportError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_never_asserts_an_unprovable_done() {
        // ADR-0012 clause 4: the safe mapping is the default, so a caller that
        // passes no options cannot silently launder a source's claim.
        assert_eq!(Options::default().implemented_as, ImplementedAs::Todo);
    }

    #[test]
    fn the_default_phase_is_one() {
        assert_eq!(Options::default().phase, 1);
    }
}
