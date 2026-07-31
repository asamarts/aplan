//! `kep.yaml` import, checked against the upstream template's real field set.
//!
//! The fixture below carries all seventeen top-level fields the Kubernetes KEP
//! template defines, so the loss accounting in ADR-0012 clause 3 is verified
//! here rather than asserted in prose: six fields are held, eleven are reported
//! as dropped.

use aplan_core::{EvidenceKind, Status};
use aplan_interop::{ImplementedAs, NoticeKind, Options, ProposalSource, kep::Kep};

/// Every field the upstream KEP template defines.
const FULL: &str = r"
title: Graduate the widget API to stable
kep-number: 3063
authors:
  - '@alice'
  - '@bob'
owning-sig: sig-node
participating-sigs:
  - sig-api-machinery
status: implementable
creation-date: 2024-01-15
reviewers:
  - '@carol'
approvers:
  - '@dave'
see-also:
  - KEP-1234
replaces:
  - KEP-0001
stage: beta
latest-milestone: v1.31
milestone:
  alpha: v1.29
  beta: v1.31
  stable: v1.33
feature-gates:
  - name: WidgetAPI
    components:
      - kube-apiserver
disable-supported: true
metrics:
  - widget_requests_total
";

fn parse(src: &str, options: &Options) -> aplan_interop::Import {
    Kep.parse("kep.yaml", src, options).expect("parses")
}

#[test]
fn the_held_fields_land_on_the_task() {
    let out = parse(FULL, &Options::default());
    let t = &out.tasks[0];

    assert_eq!(t.id, "KEP-3063", "kep-number becomes the id");
    assert_eq!(t.title, "Graduate the widget API to stable");
    assert_eq!(
        t.owner.as_deref(),
        Some("alice"),
        "first author, @ stripped"
    );
    assert_eq!(t.phase, 1, "default phase");
}

#[test]
fn stage_and_sig_become_tags_never_a_status() {
    let out = parse(FULL, &Options::default());
    let tags = &out.tasks[0].tags;

    assert!(tags.contains(&"kep:implementable".to_string()), "{tags:?}");
    assert!(tags.contains(&"kep:stage:beta".to_string()), "{tags:?}");
    assert!(tags.contains(&"sig:node".to_string()), "{tags:?}");
}

#[test]
fn exactly_the_unmodelled_fields_are_reported_as_dropped() {
    // ADR-0012 clause 3, verified against the real template rather than trusted:
    // the eleven fields the task model cannot hold each produce one notice.
    let out = parse(FULL, &Options::default());
    let dropped: Vec<&str> = out
        .report
        .notices()
        .iter()
        .filter(|n| n.kind == NoticeKind::DroppedField)
        .map(|n| n.message.as_str())
        .collect();

    for field in [
        "participating-sigs",
        "creation-date",
        "reviewers",
        "approvers",
        "see-also",
        "replaces",
        "latest-milestone",
        "milestone",
        "feature-gates",
        "disable-supported",
        "metrics",
    ] {
        assert!(
            dropped.iter().any(|m| m.contains(field)),
            "`{field}` must be reported as dropped, got {dropped:?}"
        );
    }
    // The extra notice is the multi-author truncation, which is also a loss.
    assert!(
        dropped.iter().any(|m| m.contains("authors")),
        "a second author is lost and must be reported: {dropped:?}"
    );

    // A field the model DOES hold must never be reported as lost: a false alarm
    // in a loss report is as corrosive as a silent omission. This is what would
    // break if `#[serde(flatten)]` ever double-captured a named field.
    for held in ["title", "kep-number", "owning-sig", "stage"] {
        assert!(
            !dropped.iter().any(|m| m.contains(held)),
            "`{held}` is held by the task model and must not be reported dropped: {dropped:?}"
        );
    }

    // Exactly the eleven unmodelled fields, plus the author truncation. An
    // unexpected twelfth notice means the accounting drifted from ADR-0012.
    assert_eq!(
        dropped.len(),
        12,
        "expected 11 dropped fields + 1 author truncation, got {dropped:?}"
    );
}

#[test]
fn status_matching_is_case_insensitive() {
    let out = parse(
        "title: T\nkep-number: 7\nstatus: Implemented\n",
        &Options::default(),
    );
    assert_eq!(out.tasks[0].status, Status::Todo);
    assert!(
        out.tasks[0].tags.contains(&"kep:implemented".to_string()),
        "the tag is normalised: {:?}",
        out.tasks[0].tags
    );
    assert!(
        !out.report
            .notices()
            .iter()
            .any(|n| n.message.contains("unrecognised")),
        "a capitalised known status is not unrecognised: {}",
        out.report
    );
}

#[test]
fn an_upstream_field_we_never_heard_of_is_still_reported() {
    // The point of `#[serde(flatten)]`: loss accounting cannot go stale.
    let out = parse(
        "title: T\nkep-number: 1\nstatus: provisional\nfuture-field: surprising\n",
        &Options::default(),
    );
    assert!(
        out.report
            .notices()
            .iter()
            .any(|n| n.kind == NoticeKind::DroppedField && n.message.contains("future-field")),
        "{}",
        out.report
    );
}

#[test]
fn an_empty_field_is_not_reported_as_lost() {
    let out = parse(
        "title: T\nkep-number: 1\nstatus: provisional\nsee-also:\nreviewers: []\n",
        &Options::default(),
    );
    assert_eq!(
        out.report.count(NoticeKind::DroppedField),
        0,
        "nothing was lost, so nothing should be reported: {}",
        out.report
    );
}

#[test]
fn every_documented_status_maps_as_adr_0012_says() {
    for (kep_status, expected) in [
        ("provisional", Status::Todo),
        ("implementable", Status::Todo),
        ("implemented", Status::Todo), // the default: a claim is not proof
        ("deferred", Status::Todo),
        ("rejected", Status::Dropped),
        ("withdrawn", Status::Dropped),
        ("replaced", Status::Dropped),
    ] {
        let src = format!("title: T\nkep-number: 7\nstatus: {kep_status}\n");
        let out = parse(&src, &Options::default());
        assert_eq!(
            out.tasks[0].status, expected,
            "`{kep_status}` must import as {expected:?}"
        );
        assert!(
            out.tasks[0].tags.contains(&format!("kep:{kep_status}")),
            "the stage must survive as a tag"
        );
    }
}

#[test]
fn implemented_stays_todo_by_default_and_says_why() {
    let out = parse(
        "title: T\nkep-number: 7\nstatus: implemented\n",
        &Options::default(),
    );
    assert_eq!(out.tasks[0].status, Status::Todo);
    assert!(
        out.tasks[0].evidence.is_empty(),
        "no evidence may be invented"
    );
    assert!(
        out.report.notices().iter().any(|n| {
            n.kind == NoticeKind::Provenance
                && n.message.contains("claims `implemented`")
                && n.message.contains("aplan close")
        }),
        "the report must explain the downgrade and name the fix: {}",
        out.report
    );
}

#[test]
fn opting_in_yields_done_with_a_dated_note_and_a_warning() {
    let options = Options {
        implemented_as: ImplementedAs::Done {
            as_of: "2026-07-30".to_string(),
        },
        ..Options::default()
    };
    let out = parse("title: T\nkep-number: 7\nstatus: implemented\n", &options);

    assert_eq!(out.tasks[0].status, Status::Done);
    let e = &out.tasks[0].evidence[0];
    assert_eq!(e.kind, EvidenceKind::Note, "the weakest kind, deliberately");
    assert_eq!(
        e.captured.as_deref(),
        Some("2026-07-30"),
        "APLAN010 needs a real date, and the caller supplies it"
    );
    assert!(
        out.report
            .notices()
            .iter()
            .any(|n| n.kind == NoticeKind::Provenance && n.message.contains("note-level")),
        "{}",
        out.report
    );
}

#[test]
fn a_non_implemented_kep_never_gains_evidence_even_when_opted_in() {
    let options = Options {
        implemented_as: ImplementedAs::Done {
            as_of: "2026-07-30".to_string(),
        },
        ..Options::default()
    };
    let out = parse("title: T\nkep-number: 7\nstatus: implementable\n", &options);
    assert_eq!(out.tasks[0].status, Status::Todo);
    assert!(out.tasks[0].evidence.is_empty());
}

#[test]
fn an_unrecognised_status_imports_as_todo_and_says_so() {
    let out = parse(
        "title: T\nkep-number: 7\nstatus: marinating\n",
        &Options::default(),
    );
    assert_eq!(out.tasks[0].status, Status::Todo);
    assert!(out.tasks[0].tags.contains(&"kep:marinating".to_string()));
    assert!(
        out.report
            .notices()
            .iter()
            .any(|n| n.message.contains("unrecognised status")),
        "{}",
        out.report
    );
}

#[test]
fn a_quoted_kep_number_is_accepted() {
    let out = parse(
        "title: T\nkep-number: '0042'\nstatus: provisional\n",
        &Options::default(),
    );
    assert_eq!(out.tasks[0].id, "KEP-0042");
}

#[test]
fn a_phase_override_reaches_the_task() {
    let options = Options {
        phase: 3,
        ..Options::default()
    };
    let out = parse("title: T\nkep-number: 7\nstatus: provisional\n", &options);
    assert_eq!(out.tasks[0].phase, 3);
}

#[test]
fn unusable_input_is_an_error_not_a_silent_empty_import() {
    let o = Options::default();
    assert!(
        Kep.parse("kep.yaml", "   \n", &o).is_err(),
        "empty document"
    );
    assert!(
        Kep.parse("kep.yaml", "title: T\nstatus: provisional\n", &o)
            .is_err(),
        "no kep-number means no id"
    );
    assert!(
        Kep.parse("kep.yaml", "kep-number: 7\nstatus: provisional\n", &o)
            .is_err(),
        "no title"
    );
    assert!(
        Kep.parse("kep.yaml", "\t not: [valid: yaml\n", &o).is_err(),
        "malformed yaml"
    );
}

#[test]
fn the_same_input_always_produces_the_same_report() {
    // Determinism (HLD section 14): `extra` is a BTreeMap, so dropped-field
    // order is sorted and stable rather than hash-seeded.
    let a = parse(FULL, &Options::default());
    let b = parse(FULL, &Options::default());
    assert_eq!(a.report.to_string(), b.report.to_string());
    assert_eq!(a.tasks, b.tasks);
}
