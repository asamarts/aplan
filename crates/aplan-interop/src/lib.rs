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
//!    ([`report::Report`]); silent omission is forbidden.
//! 4. An importer never asserts a `done` it cannot prove: a source's
//!    "implemented" is a claim, so it imports as `todo` plus a stage tag unless
//!    the caller opts in with [`ImplementedAs::Done`].
//! 5. Import writes new task files and nothing else, unless asked.
//!
//! Each source shape is a module implementing [`ProposalSource`], so a second
//! flavor is an addition rather than a refactor, and the policy above has a
//! single enforcement point.

pub mod blueprint;
pub mod checklist;
pub mod cut;
pub mod dependency_manifest;
pub mod docs;
pub mod emit;
pub mod export;
pub mod identity;
pub mod ids;
pub mod kep;
pub mod manifest;
pub mod report;
pub mod roadmap_json;
pub mod spec_kit;

pub use report::{Notice, NoticeKind, Report};

use aplan_core::Task;

/// Which directions one shape travels, and when it travels only one, why.
///
/// The reason lives in the table rather than in the code that refuses, so
/// "unsupported" cannot be the whole answer an adopter gets: a shape this build
/// declines to write names the one field a generated document would have had to
/// invent, and what to do instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// `import --format <name>` reads it and `export --format <name>` writes it.
    BothWays,
    /// Read only, because writing it honestly is not possible from a plan.
    ReadOnly {
        /// The one field a written document would have to invent.
        field: &'static str,
        /// Why the plan cannot supply it.
        why: &'static str,
        /// The supported way to get the same result.
        instead: &'static str,
    },
}

/// One shape `--format` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flavor {
    /// The name as it appears in `--format`.
    pub name: &'static str,
    /// Which way it goes.
    pub direction: Direction,
}

/// Every shape `--format` names, in the order the help lists them.
///
/// One table for both halves. `import` validates its `--format` against it and
/// `export` dispatches on it, so a shape cannot be readable on one side and
/// unknown to the other, and a new flavor is one row plus its module rather than
/// a list to remember to update in three places.
pub const FLAVORS: &[Flavor] = &[
    Flavor {
        name: "kep",
        direction: Direction::ReadOnly {
            field: "approvers",
            why: "a `kep.yaml` names the people who signed a proposal off, and a task records at \
                  most one `owner`",
            instead: "Keep the KEP as the source and re-import it. A sign-off you want in the \
                      plan is `url` evidence on the task.",
        },
    },
    Flavor {
        name: "checklist",
        direction: Direction::BothWays,
    },
    Flavor {
        name: "roadmap-json",
        direction: Direction::ReadOnly {
            field: "artifact",
            why: "that document describes a repository as well as its statuses, so a file \
                  generated from the plan alone would validate here and break there",
            instead: "Run `aplan export-roadmap-json roadmap.json`, which regenerates the half \
                      that drifts and carries every other byte of the file through.",
        },
    },
    Flavor {
        name: "spec-kit",
        direction: Direction::ReadOnly {
            field: "id",
            why: "its ids are a letter and digits, `T012`, which a plan's ids are not, so every \
                  row would be renumbered and stop naming the task it came from",
            instead: "Run `aplan export --format checklist`, which writes the same rows and \
                      invents no ids.",
        },
    },
    Flavor {
        name: "openspec",
        direction: Direction::ReadOnly {
            field: "id",
            why: "its `1.1` numbering is a section hierarchy the task model does not hold, so \
                  the numbers would be invented and would move whenever a task was added",
            instead: "Run `aplan export --format checklist`, which writes the same rows and \
                      invents no ids.",
        },
    },
];

/// Every flavor name, comma-separated, for a message that lists them.
#[must_use]
pub fn flavor_names() -> Vec<&'static str> {
    FLAVORS.iter().map(|f| f.name).collect()
}

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
    /// Destination reporting coordinate for every emitted task. Proposal
    /// repositories have no equivalent aplan field: a source concept also
    /// called "phase" remains source provenance rather than being mistaken for
    /// this value. It creates no membership, gate, or execution order
    /// The report states which value was assigned.
    pub phase: u32,
    /// How to treat a source's "implemented".
    pub implemented_as: ImplementedAs,
    /// Prefix for ids a source cannot supply, e.g. `T` giving `T-001`.
    ///
    /// A KEP carries its own number, so this is unused there. A checklist has
    /// nothing to use as an id, and a task nobody can refer to is not a task, so
    /// the importer generates them and the report says it did.
    pub id_prefix: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            phase: 1,
            implemented_as: ImplementedAs::default(),
            id_prefix: "T".to_string(),
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
    /// The body to write for each task, parallel to `tasks`.
    ///
    /// Per task rather than one template, because what is worth recording differs
    /// by source: a KEP's provenance is the same sentence every time, while a
    /// checklist item's is the section it sat under and the prose above it, which
    /// is the one thing importing a list of one-liners would otherwise discard.
    pub bodies: Vec<String>,
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
    ///
    /// `detail` says what the document *does* contain and what to do about it.
    /// Without it the refusal was three words and an exit code, which told the
    /// fifth pilot's 1,887-line programme plan nothing about why 451 rows of it
    /// were declined or what would work instead (`A1-129`).
    #[error("{source_name}: no importable proposal found{}", detail.as_deref().unwrap_or(""))]
    Empty {
        /// The source as the user named it.
        source_name: String,
        /// What the document contains, and the supported next step.
        detail: Option<String>,
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

    struct PhaseCase {
        name: &'static str,
        parse: fn(&Options) -> Import,
    }

    fn kep(options: &Options) -> Import {
        kep::Kep
            .parse(
                "kep.yaml",
                "title: Work\nkep-number: 7\nstatus: provisional\n",
                options,
            )
            .expect("KEP parses")
    }

    fn checklist(options: &Options) -> Import {
        checklist::Checklist
            .parse("ROADMAP.md", "## Work\n\n- [ ] Do it\n", options)
            .expect("checklist parses")
    }

    fn roadmap_json(options: &Options) -> Import {
        roadmap_json::RoadmapJson
            .parse(
                "roadmap.json",
                r#"{"phases":[{"id":"source-1","title":"Source phase"}]}"#,
                options,
            )
            .expect("roadmap JSON parses")
    }

    fn spec_kit(options: &Options) -> Import {
        spec_kit::SpecKit
            .parse("tasks.md", "## Phase 1\n\n- [ ] T001 Do it\n", options)
            .expect("spec-kit parses")
    }

    fn openspec(options: &Options) -> Import {
        spec_kit::OpenSpec
            .parse("tasks.md", "## 1. Work\n\n- [ ] 1.1 Do it\n", options)
            .expect("OpenSpec parses")
    }

    /// The phase decision's importer inventory is an audit, not a prose list.
    /// Each registry member drives its real parser, so a sixth flavor cannot
    /// arrive without deciding what destination phase its tasks receive.
    #[test]
    fn every_import_flavor_assigns_the_destination_phase() {
        let cases = [
            PhaseCase {
                name: "kep",
                parse: kep,
            },
            PhaseCase {
                name: "checklist",
                parse: checklist,
            },
            PhaseCase {
                name: "roadmap-json",
                parse: roadmap_json,
            },
            PhaseCase {
                name: "spec-kit",
                parse: spec_kit,
            },
            PhaseCase {
                name: "openspec",
                parse: openspec,
            },
        ];
        let registered: Vec<&str> = FLAVORS.iter().map(|flavor| flavor.name).collect();
        let audited: Vec<&str> = cases.iter().map(|case| case.name).collect();
        assert_eq!(
            audited, registered,
            "every registered flavor needs an answer"
        );

        let options = Options {
            phase: 7,
            ..Options::default()
        };
        for case in cases {
            let imported = (case.parse)(&options);
            assert!(!imported.tasks.is_empty(), "{} drove no tasks", case.name);
            assert!(
                imported.tasks.iter().all(|task| task.phase == 7),
                "{} did not assign destination phase 7: {:?}",
                case.name,
                imported
                    .tasks
                    .iter()
                    .map(|task| (&task.id, task.phase))
                    .collect::<Vec<_>>()
            );
        }
    }

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
