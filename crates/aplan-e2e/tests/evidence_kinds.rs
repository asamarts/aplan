//! Every evidence kind answers resolution, reference rot, referent freshness,
//! capture semantics, repair, locality and dilution, or says why a capability
//! does not apply (`A1-115`, `A1-192`, `A1-197`).
//!
//! **The exhaustive audit, named.** This repository keeps arriving at one idea
//! from different directions: enumerate a surface from the type system, require an
//! entry per member, and give the entry something to *drive* so it cannot be a
//! label. `crates/aplan-e2e/tests/coverage.rs` walks `Rule::ALL` and fails naming
//! the rule with no scenario. `crates/aplan/tests/mcp_parity.rs` walks the MCP
//! tools and says it best: "The table is the audit [...] There is no state in
//! which a tool is classified and quietly unchecked." Both were invented
//! independently. This is the third, over `EvidenceKind::ALL`, and `AGENTS.md`
//! names the pattern so the fourth surface does not have to rediscover it.
//!
//! **What the audit is for here.** A citation is a reference and references rot;
//! some citations also expose a path footprint that can be bound to the task.
//! Until this file, nothing required a kind to answer either question, and the
//! gap was invisible in the obvious way: `note` -- one of the five kinds -- was
//! missing entirely from the hand-written matrix in `A1-115`, three of that
//! matrix's cells were stale, and URL locality was only an absent implementation.
//! The rows now come from the capability model the verification engine reads;
//! this file audits that model instead of maintaining a second one.
//!
//! **Why the codes are checked against `Rule::ALL`.** Naming `APLAN011` in a
//! column proves nothing if the string is never resolved; that is the "enum that
//! appeared only in the table and drove nothing" the parity audit had to remove
//! from its own first version. Every code here is looked up, and every repair
//! command is looked up in `aplan facts`, so an entry that names something this
//! build does not have fails.

use std::fmt::Write as _;

use aplan_core::task::{
    EvidenceCapture, EvidenceDilution, EvidenceFreshness, EvidenceLocality,
    EvidenceReferenceRepair, EvidenceReferenceRot, EvidenceResolution,
};
use aplan_core::{Evidence, EvidenceKind, Plan, Rule, Status, Task, unresolved_by_check};
use aplan_evidence::{VerifyOptions, unresolved_by_verify};

/// Every `EvidenceKind` has exactly one row, so a new kind cannot arrive unchecked.
#[test]
fn every_evidence_kind_is_audited() {
    assert_eq!(
        matrix().lines().skip(2).count(),
        EvidenceKind::ALL.len(),
        "the capability model must render exactly one row per evidence kind"
    );
}

/// Every rule a row names is a rule this build has.
///
/// The half that stops a row being a label. `A1-115`'s hand-written matrix named
/// `APLAN016` and `APLAN019` as unbuilt long after both shipped, which nothing
/// could catch while the codes were prose.
#[test]
fn every_rot_detector_named_is_a_rule_this_build_has() {
    let mut wrong = Vec::new();
    let mut detectors = 0;
    for kind in EvidenceKind::ALL {
        let EvidenceReferenceRot::DetectedBy(code) = kind.capabilities().reference_rot else {
            continue;
        };
        detectors += 1;
        if !Rule::ALL.iter().any(|r| r.code() == code) {
            wrong.push(format!(
                "`{}` names `{code}`, which is not a rule",
                kind.as_str()
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(
        detectors >= 4,
        "only {detectors} kinds declare a detector, so this audit is not looking"
    );
}

/// A kind with no detector has to say why none can exist, so "none" is a
/// position rather than an omission.
#[test]
fn a_kind_without_a_detector_explains_why_it_cannot_have_one() {
    for kind in EvidenceKind::ALL {
        let capabilities = kind.capabilities();
        let EvidenceReferenceRot::NotApplicable { reason } = capabilities.reference_rot else {
            continue;
        };
        assert!(
            matches!(
                capabilities.reference_repair,
                EvidenceReferenceRepair::NotApplicable
            ),
            "`{}` has no detector but claims a repair",
            kind.as_str()
        );
        assert!(
            reason.len() > 40,
            "`{}` has no detector and no explanation",
            kind.as_str()
        );
    }
}

/// A kind with no locality has to explain why repository paths cannot locate
/// it. In particular this records `url`'s answer instead of treating the
/// missing fourth implementation as an accidental omission.
#[test]
fn a_kind_without_locality_explains_why_it_cannot_have_one() {
    for kind in EvidenceKind::ALL {
        let EvidenceLocality::NotApplicable { reason } = kind.capabilities().locality else {
            continue;
        };
        assert!(
            reason.len() > 40,
            "`{}` has no locality and no explanation",
            kind.as_str()
        );
    }
}

/// Every path-local kind names a real rule that reports a mismatch.
#[test]
fn every_locality_detector_named_is_a_rule_this_build_has() {
    let mut checked = 0;
    for kind in EvidenceKind::ALL {
        let Some(code) = kind.capabilities().locality.detector() else {
            continue;
        };
        checked += 1;
        assert!(
            Rule::ALL.iter().any(|rule| rule.code() == code),
            "`{}` names locality detector `{code}`, which is not a rule",
            kind.as_str()
        );
    }
    assert_eq!(checked, 3, "commit, file and test must all be audited");
}

/// Dilution is answered for every kind, including an explanation when it does
/// not apply. An unexplained `None` would recreate the gap this task closes.
#[test]
fn every_kind_decides_whether_dilution_applies() {
    let mut measured = 0;
    for kind in EvidenceKind::ALL {
        match kind.capabilities().dilution {
            EvidenceDilution::Measured { detector, .. } => {
                measured += 1;
                assert!(
                    Rule::ALL.iter().any(|rule| rule.code() == detector),
                    "`{}` names dilution detector `{detector}`, which is not a rule",
                    kind.as_str()
                );
            }
            EvidenceDilution::NotApplicable { reason } => assert!(
                reason.len() > 40,
                "`{}` has no dilution measure and no explanation",
                kind.as_str()
            ),
        }
    }
    assert_eq!(
        measured, 1,
        "only changed-path evidence has an observed footprint to measure"
    );
}

/// Every kind answers the distinct referent-freshness question. In particular,
/// a live URL check is not mislabelled as proof that the page's meaning stayed
/// the same, and a passing opaque test name is not mislabelled as a stable test
/// body.
#[test]
fn every_kind_decides_whether_its_referent_can_change() {
    let mut fingerprinted = Vec::new();
    let mut immutable = Vec::new();
    let mut undetectable = Vec::new();
    let mut inline = Vec::new();
    for kind in EvidenceKind::ALL {
        match kind.capabilities().freshness {
            EvidenceFreshness::Fingerprinted { detector } => {
                assert!(
                    Rule::ALL.iter().any(|rule| rule.code() == detector),
                    "`{}` names freshness detector `{detector}`, which is not a rule",
                    kind.as_str()
                );
                fingerprinted.push(kind.as_str());
            }
            EvidenceFreshness::Immutable { reason } => {
                assert!(
                    reason.len() > 40,
                    "`{}` does not explain immutability",
                    kind.as_str()
                );
                immutable.push(kind.as_str());
            }
            EvidenceFreshness::NotDetectable { reason } => {
                assert!(
                    reason.len() > 40,
                    "`{}` does not explain why freshness is unobservable",
                    kind.as_str()
                );
                undetectable.push(kind.as_str());
            }
            EvidenceFreshness::NotApplicable { reason } => {
                assert!(
                    reason.len() > 40,
                    "`{}` does not explain why freshness is inapplicable",
                    kind.as_str()
                );
                inline.push(kind.as_str());
            }
        }
    }
    assert_eq!(fingerprinted, ["file"]);
    assert_eq!(immutable, ["commit"]);
    assert_eq!(undetectable, ["url", "test"]);
    assert_eq!(inline, ["note"]);
}

/// `captured` is not a generic timestamp. It is authored provenance for the
/// inline assertion in a note; accepting it on a resolver-backed kind would
/// make an unchecked date look like a resolver observation.
#[test]
fn every_kind_decides_whether_captured_has_semantics() {
    let mut authored = Vec::new();
    let mut rejected = Vec::new();
    for kind in EvidenceKind::ALL {
        let capture = kind.capabilities().capture;
        assert!(
            Rule::ALL
                .iter()
                .any(|rule| rule.code() == capture.detector()),
            "`{}` names capture detector `{}`, which is not a rule",
            kind.as_str(),
            capture.detector()
        );
        match capture {
            EvidenceCapture::AuthoredDate { detector } => {
                assert_eq!(detector, "APLAN010");
                authored.push(kind.as_str());
            }
            EvidenceCapture::NotApplicable { detector, reason } => {
                assert_eq!(detector, "APLAN055");
                assert!(
                    reason.len() > 60,
                    "`{}` rejects capture metadata without explaining its real provenance: \
                     {reason}",
                    kind.as_str()
                );
                rejected.push(kind.as_str());
            }
        }
    }
    assert_eq!(authored, ["note"]);
    assert_eq!(rejected, ["commit", "url", "file", "test"]);
}

/// Every kind's resolution, detector and repair tell one coherent story.
///
/// The half `A1-115` asked for and the first version of this file omitted: it
/// checked the rot detector and never the resolver, while ticking a criterion
/// that named both (found by this task's audit).
#[test]
fn every_kind_declares_how_it_resolves() {
    // A kind that resolves against nothing is the one shape that may have no
    // detector, because there is nothing to detect. The two must agree.
    for kind in EvidenceKind::ALL {
        let capabilities = kind.capabilities();
        let unresolvable = matches!(capabilities.resolution, EvidenceResolution::None);
        assert_eq!(
            unresolvable,
            matches!(
                capabilities.reference_rot,
                EvidenceReferenceRot::NotApplicable { .. }
            ),
            "`{}` cannot be resolvable-with-no-detector or unresolvable-with-one",
            kind.as_str()
        );
        assert_eq!(
            matches!(
                capabilities.reference_rot,
                EvidenceReferenceRot::NotApplicable { .. }
            ),
            matches!(
                capabilities.reference_repair,
                EvidenceReferenceRepair::NotApplicable
            ),
            "`{}` cannot have a detector without a repair or a repair without a detector",
            kind.as_str()
        );
    }
}

/// A kind this audit calls resolvable has a resolver registered for it.
///
/// Drive the real unopened-citation report rather than parsing the resolver
/// registry's source. Git kinds are always covered, configured kinds are not
/// covered under default options, and a kind with nothing to resolve is not an
/// unopened citation in the first place.
#[test]
fn resolution_capabilities_drive_both_unopened_reports() {
    let evidence = EvidenceKind::ALL
        .iter()
        .map(|kind| Evidence {
            kind: *kind,
            reference: kind.as_str().to_string(),
            captured: (*kind == EvidenceKind::Note).then(|| "2026-10-08".to_string()),
        })
        .collect();
    let task = Task {
        id: "T-1".to_string(),
        title: "all evidence kinds".to_string(),
        phase: 0,
        status: Status::Done,
        owner: None,
        effort_days: None,
        blocked_by: Vec::new(),
        prefer_after: Vec::new(),
        ratified_by: Vec::new(),
        contains: Vec::new(),
        evidence,
        paths: Vec::new(),
        traces: Vec::new(),
        trace_fingerprints: std::collections::BTreeMap::new(),
        evidence_fingerprints: std::collections::BTreeMap::new(),
        tags: Vec::new(),
        revisit_at: None,
        version_impact: None,
        version_impact_reason: None,
        version: None,
    };
    let plan = Plan {
        tasks: vec![aplan_core::load::LoadedTask {
            task,
            fields: aplan_core::load::FieldLines::new(),
            criteria: aplan_core::load::Criteria::default(),
            path: "T-1.md".into(),
        }],
        ..Plan::default()
    };
    let structurally_unresolved = unresolved_by_check(&plan);
    assert_eq!(structurally_unresolved.citations, 4);
    assert_eq!(structurally_unresolved.tasks, 1);

    let unopened = unresolved_by_verify(
        &plan,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
        &VerifyOptions::default(),
    );
    let expected = vec![EvidenceKind::Url, EvidenceKind::Test];
    assert_eq!(unopened.kinds, expected);
    assert_eq!(unopened.citations, expected.len());
    assert_eq!(unopened.tasks, usize::from(!expected.is_empty()));
}

/// Every repair the capability model names is a command this build ships.
///
/// Reading the generated facts document is deliberate: it is the command
/// surface a consumer receives, and its own drift gate derives it from clap.
#[test]
fn every_reference_repair_is_a_command_this_build_has() {
    let facts = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../facts.json"),
    )
    .expect("generated facts.json");
    let facts: serde_json::Value = serde_json::from_str(&facts).expect("facts.json is JSON");
    let commands = facts["commands"]
        .as_array()
        .expect("facts.json commands are an array");
    let mut checked = 0;
    for kind in EvidenceKind::ALL {
        let command = match kind.capabilities().reference_repair {
            EvidenceReferenceRepair::Amend => "amend",
            EvidenceReferenceRepair::NotApplicable => continue,
        };
        checked += 1;
        assert!(
            commands.iter().any(|fact| fact["name"] == command),
            "`{}` names repair command `{command}`, which facts.json does not advertise",
            kind.as_str()
        );
    }
    assert!(
        checked >= 4,
        "only {checked} repairs were checked, so this audit is not looking"
    );
}

/// The three instances `AGENTS.md` names are three files that exist.
///
/// The pattern's own registry, checked. Naming it in prose and letting the paths
/// rot would be the table-beside-the-code failure this file was written to end,
/// committed in the act of describing it (found by this task's audit).
#[test]
fn every_instance_the_contract_names_is_a_file() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let agents = std::fs::read_to_string(root.join("AGENTS.md")).expect("AGENTS.md");
    let section = agents
        .split("### When a surface can grow a member, the table is the audit")
        .nth(1)
        .expect("the pattern is named in AGENTS.md")
        .split("\n### ")
        .next()
        .unwrap_or_default();
    let paths: Vec<&str> = section
        .lines()
        .filter(|l| l.starts_with("| `crates/"))
        .filter_map(|l| l.split('`').nth(1))
        .collect();
    assert!(
        paths.len() >= 3,
        "only {} instances listed, so this check is not looking",
        paths.len()
    );
    for p in paths {
        assert!(
            root.join(p).is_file(),
            "`AGENTS.md` names `{p}`, which is not a file"
        );
    }
}

/// The matrix, rendered from the production model rather than maintained beside it.
///
/// `A1-115` carried this as a hand-written table and it drifted in three cells
/// before the task was even started. There is one copy now and it is this one.
fn matrix() -> String {
    let mut out = String::from(
        "| citation | rot detector | repair | freshness | capture | locality | dilution |\n\
         |---|---|---|---|---|---|---|\n",
    );
    for kind in EvidenceKind::ALL {
        let capabilities = kind.capabilities();
        let rot = match capabilities.reference_rot {
            EvidenceReferenceRot::DetectedBy(code) => format!("`{code}`"),
            EvidenceReferenceRot::NotApplicable { .. } => "not applicable".to_string(),
        };
        let repair = match capabilities.reference_repair {
            EvidenceReferenceRepair::Amend => "`aplan amend`",
            EvidenceReferenceRepair::NotApplicable => "not applicable",
        };
        let freshness = match capabilities.freshness {
            EvidenceFreshness::Immutable { .. } => "immutable".to_string(),
            EvidenceFreshness::Fingerprinted { detector } => format!("`{detector}`"),
            EvidenceFreshness::NotDetectable { .. } => "mutable, not detectable".to_string(),
            EvidenceFreshness::NotApplicable { .. } => "not applicable".to_string(),
        };
        let capture = match capabilities.capture {
            EvidenceCapture::AuthoredDate { detector } => {
                format!("authored date (`{detector}`)")
            }
            EvidenceCapture::NotApplicable { detector, .. } => {
                format!("rejected (`{detector}`)")
            }
        };
        let locality = capabilities
            .locality
            .detector()
            .map_or("not applicable".to_string(), |code| format!("`{code}`"));
        let dilution = capabilities
            .dilution
            .detector()
            .map_or("not applicable".to_string(), |code| format!("`{code}`"));
        let _ = writeln!(
            out,
            "| `{}` | {rot} | {repair} | {freshness} | {capture} | {locality} | {dilution} |",
            kind.as_str()
        );
    }
    out
}

/// The rendered matrix names every kind, so the failure message is the answer.
#[test]
fn the_rendered_matrix_covers_every_kind() {
    let m = matrix();
    for kind in EvidenceKind::ALL {
        let k = kind.as_str();
        assert!(m.contains(&format!("| `{k}` |")), "{k} missing from:\n{m}");
    }
}
