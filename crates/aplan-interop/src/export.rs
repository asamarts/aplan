//! Writing a plan back out into somebody else's shape.
//!
//! The other half of [`ProposalSource`](crate::ProposalSource), and it obeys the
//! same policy read backwards. ADR-0012 made an import declare what the *task
//! model* could not hold; an export declares what the *target shape* cannot,
//! because the drift is the same in either direction. A generated document that
//! quietly leaves out a task's evidence is one somebody will read as the whole
//! plan, which is exactly the divergence this tool exists to detect.
//!
//! **Not every shape earns a way back, and the refusal is the point.** Of the
//! five [`FLAVORS`] `import --format` reads, one is written
//! here. The other four are declined by name, each stating the one field it
//! could not carry and what to do instead, because a shape approximated is a
//! document that looks like its format and is not one.

use aplan_core::Task;

use crate::report::Report;
use crate::{Direction, FLAVORS, checklist::Checklist};

/// Knobs a caller sets for one export run.
///
/// A struct rather than a bare argument so a second knob is an addition rather
/// than a signature change, which is the shape [`Options`](crate::Options)
/// already has on the import side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOptions {
    /// How the reader named the document being written, used to label every
    /// notice. `standard output` when the bytes are not going to a file.
    pub destination: String,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            destination: "standard output".to_string(),
        }
    }
}

/// The result of writing one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    /// The bytes to write, ending in a newline.
    pub contents: String,
    /// What the shape could not carry, and where the reader has to look.
    pub report: Report,
}

/// Why a document could not be written at all.
///
/// A *field the shape cannot hold* is not an error: it is a [`Report`] notice,
/// because the export still succeeded. This type is for a request that cannot
/// produce an honest document.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// This build reads the shape and does not write it.
    ///
    /// The field is named because "unsupported" tells an adopter nothing about
    /// whether to wait for it or work around it, and because naming it is what
    /// makes the refusal checkable rather than a matter of taste.
    #[error(
        "`{flavor}` is a shape this reads and does not write: a generated one could not carry \
         `{field}`, because {why}. {instead}"
    )]
    CannotCarry {
        /// The flavor as the user spelled it.
        flavor: String,
        /// The one field a written document would have to invent.
        field: String,
        /// Why the plan cannot supply it.
        why: String,
        /// The supported way to get the same result.
        instead: String,
    },
    /// `--format` named a shape this build does not know.
    #[error("unknown format `{flavor}` (this writes: {writes})")]
    UnknownFlavor {
        /// The flavor as the user spelled it.
        flavor: String,
        /// The names that would have worked, comma-separated.
        writes: String,
    },
    /// The plan holds nothing this shape can carry.
    ///
    /// `detail` says what the plan *does* hold and what to do about it, for the
    /// reason the import side learned: a refusal that is three words and an exit
    /// code tells the reader nothing about why.
    #[error("nothing to write{}", detail.as_deref().unwrap_or(""))]
    Empty {
        /// What the plan contains, and the supported next step.
        detail: Option<String>,
    },
}

/// One shape `aplan` can write.
///
/// Implementors live beside the importer for the same shape, so the two halves
/// of one format share a module and a change to the mapping is visible to both.
pub trait PlanTarget {
    /// The flavor name as it appears in `--format`.
    fn flavor(&self) -> &'static str;

    /// Render `tasks` into one document plus the conversion report.
    ///
    /// `tasks` is the whole plan, milestones included: a shape that groups work
    /// needs the grouping, and deriving it here rather than taking it
    /// pre-chewed keeps the one membership rule
    /// ([`edge_kind`](aplan_core::edge_kind)) as the only answer to what a cut
    /// holds.
    fn render(&self, tasks: &[&Task], options: &ExportOptions) -> Result<Export, ExportError>;
}

/// The names this build writes, in [`FLAVORS`] order.
#[must_use]
pub fn writable() -> Vec<&'static str> {
    FLAVORS
        .iter()
        .filter(|f| f.direction == Direction::BothWays)
        .map(|f| f.name)
        .collect()
}

/// Write one document, or say why this shape has no honest way back.
///
/// One `PlanTarget` per flavor, chosen here and nowhere else, so adding a shape
/// is an arm rather than a branch through the write path -- the shape
/// `read_source` has on the import side.
pub fn write(
    flavor: &str,
    tasks: &[&Task],
    options: &ExportOptions,
) -> Result<Export, ExportError> {
    let Some(known) = FLAVORS.iter().find(|f| f.name == flavor) else {
        return Err(ExportError::UnknownFlavor {
            flavor: flavor.to_string(),
            writes: writable().join(", "),
        });
    };
    match known.direction {
        Direction::BothWays => match flavor {
            "checklist" => Checklist.render(tasks, options),
            // Unreachable while `FLAVORS` and this arm agree, and
            // `every_writable_flavor_has_a_target` is what keeps them agreeing:
            // a flavor marked `BothWays` with no arm here fails that test rather
            // than reaching a reader.
            _ => Err(ExportError::UnknownFlavor {
                flavor: flavor.to_string(),
                writes: writable().join(", "),
            }),
        },
        Direction::ReadOnly {
            field,
            why,
            instead,
        } => Err(ExportError::CannotCarry {
            flavor: flavor.to_string(),
            field: field.to_string(),
            why: why.to_string(),
            instead: instead.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The census: every shape marked writable really is written.**
    ///
    /// Walks `FLAVORS` rather than a hand-written list, the discipline
    /// `NoticeKind::ALL` and `Rule::ALL` already use here: a flavor promoted to
    /// `BothWays` without an arm in `write` fails this instead of telling a
    /// reader its format is unknown.
    #[test]
    fn every_writable_flavor_has_a_target() {
        let plan = [task("T-001", "a task")];
        let refs: Vec<&Task> = plan.iter().collect();
        for name in writable() {
            let out = write(name, &refs, &ExportOptions::default());
            assert!(
                !matches!(out, Err(ExportError::UnknownFlavor { .. })),
                "`{name}` is listed as writable and `write` does not know it"
            );
        }
    }

    /// The other direction: a shape that is not written says which field stopped
    /// it, so the refusal is a fact a reader can act on rather than a policy.
    #[test]
    fn every_read_only_flavor_names_the_field_it_could_not_carry() {
        let plan = [task("T-001", "a task")];
        let refs: Vec<&Task> = plan.iter().collect();
        for flavor in FLAVORS {
            let Direction::ReadOnly { field, .. } = flavor.direction else {
                continue;
            };
            let err = write(flavor.name, &refs, &ExportOptions::default())
                .expect_err("a read-only shape must refuse");
            let text = err.to_string();
            assert!(
                text.contains(field),
                "`{}` refuses without naming `{field}`: {text}",
                flavor.name
            );
            assert!(
                matches!(err, ExportError::CannotCarry { .. }),
                "`{}` refuses with the wrong error: {text}",
                flavor.name
            );
        }
    }

    #[test]
    fn an_unknown_format_lists_the_ones_that_would_have_worked() {
        let err = write("yaml", &[], &ExportOptions::default())
            .expect_err("an unknown format is refused");
        let text = err.to_string();
        assert!(text.contains("checklist"), "{text}");
    }

    pub(super) fn task(id: &str, title: &str) -> Task {
        Task {
            id: id.to_string(),
            title: title.to_string(),
            phase: 1,
            status: aplan_core::Status::Todo,
            owner: None,
            effort_days: None,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
        }
    }
}
