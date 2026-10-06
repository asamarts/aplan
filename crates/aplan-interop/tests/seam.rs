//! The importer seam, exercised from outside the crate.
//!
//! An in-test flavor implements [`ProposalSource`] exactly as `kep` and every
//! later flavor will, which proves the public surface is usable by an external
//! consumer: if a type needed to implement or call the seam were not exported,
//! this file would not compile.

use aplan_core::{Status, Task};
use aplan_interop::{
    ImplementedAs, Import, ImportError, Notice, NoticeKind, Options, ProposalSource, Report,
};

/// A minimal flavor: one line of input becomes one task, and the second field
/// is a value the task model does not hold, so it is reported rather than lost.
#[derive(Debug)]
struct LineSource;

impl ProposalSource for LineSource {
    fn flavor(&self) -> &'static str {
        "line"
    }

    fn parse(
        &self,
        source_name: &str,
        contents: &str,
        options: &Options,
    ) -> Result<Import, ImportError> {
        let mut tasks = Vec::new();
        let mut report = Report::new();

        for line in contents.lines().filter(|l| !l.trim().is_empty()) {
            let mut parts = line.split(',');
            let id = parts.next().unwrap_or_default().trim().to_string();
            if id.is_empty() {
                return Err(ImportError::Malformed {
                    source_name: source_name.to_string(),
                    message: "a record needs an id".to_string(),
                });
            }
            let claimed_done = parts.next().is_some_and(|s| s.trim() == "implemented");

            // ADR-0012 clause 4: the source's claim is a claim. Only an explicit
            // opt-in turns it into `done`, and then it is flagged.
            let status =
                if claimed_done && matches!(options.implemented_as, ImplementedAs::Done { .. }) {
                    report.push(Notice::new(
                        source_name,
                        NoticeKind::Provenance,
                        format!("{id} imported `done` on note-level provenance only"),
                    ));
                    Status::Done
                } else {
                    if claimed_done {
                        report.push(Notice::new(
                            source_name,
                            NoticeKind::Provenance,
                            format!("{id} claims implemented; imported as `todo`"),
                        ));
                    }
                    Status::Todo
                };

            report.dropped(source_name, "reviewers");

            tasks.push(Task {
                id: id.clone(),
                title: format!("imported {id}"),
                phase: options.phase,
                status,
                owner: None,
                effort_days: None,
                blocked_by: Vec::new(),
                prefer_after: Vec::new(),
                ratified_by: Vec::new(),
                contains: Vec::new(),
                evidence: Vec::new(),
                paths: Vec::new(),
                traces: Vec::new(),
                trace_fingerprints: std::collections::BTreeMap::new(),
                tags: vec!["line:imported".to_string()],
                revisit_at: None,
                version_impact: None,
                version_impact_reason: None,
                version: None,
            });
        }

        if tasks.is_empty() {
            return Err(ImportError::Empty {
                source_name: source_name.to_string(),
                detail: None,
            });
        }
        // An out-of-tree flavor supplies a body per task, the same as the
        // built-in ones; the seam does not assume a shared template.
        let bodies = tasks
            .iter()
            .map(|t| format!("## Why\n\nImported: {}\n", t.id))
            .collect();
        Ok(Import {
            tasks,
            report,
            bodies,
        })
    }
}

#[test]
fn an_external_flavor_can_implement_the_seam() {
    let out = LineSource
        .parse(
            "in.csv",
            "A-1,implementable\nA-2,implemented\n",
            &Options::default(),
        )
        .expect("parses");

    assert_eq!(LineSource.flavor(), "line");
    assert_eq!(out.tasks.len(), 2);
    assert_eq!(out.tasks[0].id, "A-1");
    assert_eq!(out.tasks[0].phase, 1, "default phase");
}

#[test]
fn the_default_refuses_to_assert_an_unprovable_done() {
    // The whole point of ADR-0012 clause 4, checked through the public seam.
    let out = LineSource
        .parse("in.csv", "A-2,implemented\n", &Options::default())
        .expect("parses");

    assert_eq!(
        out.tasks[0].status,
        Status::Todo,
        "a source's `implemented` must not become `done` by default"
    );
    assert_eq!(out.report.count(NoticeKind::Provenance), 1);
    assert!(
        out.report.notices()[0]
            .message
            .contains("imported as `todo`")
            || out.report.notices()[1]
                .message
                .contains("imported as `todo`"),
        "the report must say what it did: {}",
        out.report
    );
}

#[test]
fn opting_in_yields_done_and_flags_the_provenance() {
    let options = Options {
        implemented_as: ImplementedAs::Done {
            as_of: "2026-07-30".to_string(),
        },
        ..Options::default()
    };
    let out = LineSource
        .parse("in.csv", "A-2,implemented\n", &options)
        .expect("parses");

    assert_eq!(out.tasks[0].status, Status::Done);
    assert!(
        out.report
            .notices()
            .iter()
            .any(|n| n.kind == NoticeKind::Provenance && n.message.contains("note-level")),
        "opting in must warn that the provenance is weak"
    );
}

#[test]
fn every_dropped_field_is_reported_never_silent() {
    let out = LineSource
        .parse("in.csv", "A-1,implementable\n", &Options::default())
        .expect("parses");

    assert_eq!(
        out.report.count(NoticeKind::DroppedField),
        1,
        "the unmodelled field must produce a notice (ADR-0012 clause 3)"
    );
    assert!(out.report.to_string().contains("reviewers"));
}

#[test]
fn a_phase_override_reaches_every_task() {
    let options = Options {
        phase: 4,
        ..Options::default()
    };
    let out = LineSource
        .parse("in.csv", "A-1,x\nA-2,y\n", &options)
        .expect("parses");
    assert!(out.tasks.iter().all(|t| t.phase == 4));
}

#[test]
fn unreadable_input_is_an_error_not_an_empty_success() {
    // A dropped field is a notice; input that cannot yield tasks is an error.
    let err = LineSource
        .parse("in.csv", "\n\n", &Options::default())
        .expect_err("no records means no import");
    assert!(matches!(err, ImportError::Empty { .. }), "{err:?}");
    assert!(err.to_string().contains("in.csv"), "{err}");

    let err = LineSource
        .parse("in.csv", " ,implemented\n", &Options::default())
        .expect_err("a record without an id is malformed");
    assert!(matches!(err, ImportError::Malformed { .. }), "{err:?}");
}
