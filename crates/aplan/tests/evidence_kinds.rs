//! **A kind this tool advertises is a kind some command can maintain**
//! (`A1-361`).
//!
//! `note` was advertised in `facts`, accepted by the loader, resolved by
//! `verify`, and reachable by no command at all: `close` refused to record one
//! and `amend` refused to take one away, so for one kind in five, hand-editing
//! was the only route in *and* out. That is what `amend` exists to remove.
//!
//! This is the census that would have caught it, in the shape
//! `mutation_policy.rs` uses for commands: every kind the tool advertises says
//! whether a command can write it, and one that cannot says why. A kind added
//! later is refused until somebody answers.
//!
//! **Enumerated from `EvidenceKind::ALL`**, which is what `facts.json` reads, so
//! the census and the advertisement cannot drift apart. And judged by running
//! the commands, because a list of kinds proves nothing about what the binary
//! will take.

use std::path::Path;

mod common;

/// Kinds `close` can record, as `kind:ref`.
const WRITABLE: &[(&str, &str)] = &[
    ("commit", "commit:deadbee"),
    ("test", "test:some_test"),
    ("file", "file:.aplan.yml"),
    ("url", "url:https://example.com"),
];

/// Kinds no command can record, and why that is a decision rather than a gap.
const NOT_WRITABLE: &[(&str, &str)] = &[(
    "note",
    "carries a `captured` date, and `kind:ref` has no room for one. Whether that \
     should be a syntax or stay a refusal is open on `A1-361`; what is settled is \
     that it must still be removable, which the census below asserts",
)];

fn tmp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("evidence-kinds-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir")
}

/// One live task with `paths`, so closing it is about the evidence and not about
/// `APLAN023`.
fn a_live_task(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.join("aplan/tasks/W-1.md"),
        "---\nid: W-1\ntitle: live work\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths:\n- \"*.yml\"\n---\n\n## Why\n\nBecause.\n",
    )
    .expect("task");
}

/// A closed task citing every kind there is, each one dated where its kind needs
/// a date.
fn a_task_citing_every_kind(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    let mut evidence = String::from("evidence:\n");
    for kind in aplan_core::EvidenceKind::ALL {
        let _ = std::fmt::Write::write_fmt(
            &mut evidence,
            format_args!(
                "  - kind: {}\n    ref: {}\n",
                kind.as_str(),
                reference_for(*kind)
            ),
        );
        if *kind == aplan_core::EvidenceKind::Note {
            evidence.push_str("    captured: 2026-08-14\n");
        }
    }
    std::fs::write(
        dir.join("aplan/tasks/W-1.md"),
        format!(
            "---\nid: W-1\ntitle: cites every kind\nphase: 1\nstatus: done\n\
             effort_days: 1\npaths:\n- \"*.yml\"\n{evidence}---\n\n## Why\n\nBecause.\n"
        ),
    )
    .expect("task");
}

/// A reference of the shape each kind takes. Not resolvable, deliberately:
/// whether a citation *resolves* is `verify`'s question, and this file asks
/// whether the citation can be maintained at all.
fn reference_for(kind: aplan_core::EvidenceKind) -> &'static str {
    match kind {
        aplan_core::EvidenceKind::Commit => "deadbee",
        aplan_core::EvidenceKind::Test => "some_test",
        aplan_core::EvidenceKind::File => ".aplan.yml",
        aplan_core::EvidenceKind::Url => "https://example.com",
        aplan_core::EvidenceKind::Note => "reviewed",
    }
}

/// **Every kind says whether a command can write it, and a new one is refused
/// until it does.**
#[test]
fn every_advertised_kind_is_classified() {
    let advertised: std::collections::BTreeSet<&str> = aplan_core::EvidenceKind::ALL
        .iter()
        .map(|k| k.as_str())
        .collect();
    let answered: std::collections::BTreeSet<&str> = WRITABLE
        .iter()
        .map(|(k, _)| *k)
        .chain(NOT_WRITABLE.iter().map(|(k, _)| *k))
        .collect();

    let unclassified: Vec<&&str> = advertised.difference(&answered).collect();
    assert!(
        unclassified.is_empty(),
        "an evidence kind this tool advertises, which says neither that a command can write \
         it nor why not (`A1-361`): {unclassified:?}"
    );
    let stale: Vec<&&str> = answered.difference(&advertised).collect();
    assert!(
        stale.is_empty(),
        "classified, but not a kind any more: {stale:?}"
    );
    for (kind, why) in NOT_WRITABLE {
        assert!(
            why.len() > 40,
            "`{kind}` is unwritable with no reason worth reading: {why:?}"
        );
    }
}

/// **Every kind said to be writable really is**, judged by running `close`.
#[test]
fn every_writable_kind_can_be_recorded() {
    let mut refused = Vec::new();
    for (kind, citation) in WRITABLE {
        let dir = tmp();
        a_live_task(dir.path());
        let out = common::run(dir.path(), &["close", "W-1", "--evidence", citation]);
        if out.code != 0 {
            refused.push(format!("`{kind}` via `{citation}`: {}", out.text()));
            continue;
        }
        // Exit zero is not the claim: the claim is that the citation is in the
        // file. A command that succeeded and wrote nothing would pass an
        // exit-code assertion, which is the shape of vacuous test this file
        // exists to avoid.
        let body = std::fs::read_to_string(dir.path().join("aplan/tasks/W-1.md")).expect("task");
        let (_, reference) = citation.split_once(':').expect("kind:ref");
        if !body.contains(&format!("kind: {kind}")) || !body.contains(reference) {
            refused.push(format!(
                "`{citation}` was accepted and is not in the file:\n{body}"
            ));
        }
    }
    assert!(
        refused.is_empty(),
        "{} kind(s) classified writable that `close` refuses:\n  {}",
        refused.len(),
        refused.join("\n  ")
    );
}

/// **Every kind can be taken away, writable or not** (`A1-361`).
///
/// This is the promise the defect broke. A kind `close` will not record still
/// reaches a plan -- by hand, by `import`, by an older build -- and a citation
/// the tool cannot remove is one the tool cannot maintain.
#[test]
fn every_advertised_kind_can_be_removed() {
    let mut stuck = Vec::new();
    for kind in aplan_core::EvidenceKind::ALL {
        let dir = tmp();
        a_task_citing_every_kind(dir.path());
        let citation = format!("{}:{}", kind.as_str(), reference_for(*kind));
        let out = common::run(
            dir.path(),
            &["amend", "W-1", "--remove-evidence", &citation],
        );
        if out.code != 0 {
            stuck.push(format!("`{citation}`: {}", out.text()));
            continue;
        }
        let body = std::fs::read_to_string(dir.path().join("aplan/tasks/W-1.md")).expect("task");
        if body.contains(&format!("ref: {}", reference_for(*kind))) {
            stuck.push(format!(
                "`{citation}` was reported removed and is still there"
            ));
        }
    }
    assert!(
        stuck.is_empty(),
        "{} kind(s) this tool advertises and cannot take away:\n  {}",
        stuck.len(),
        stuck.join("\n  ")
    );
}
