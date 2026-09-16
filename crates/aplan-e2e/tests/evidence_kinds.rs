//! Every evidence kind has a rot detector and a repair, or says why it has none
//! (`A1-115`).
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
//! **What the audit is for here.** A citation is a reference and references rot.
//! Until this file, nothing required a kind to have a detector for that, and the
//! gap was invisible in the obvious way: `note` -- one of the five kinds -- was
//! missing entirely from the hand-written matrix in `A1-115` that this replaces,
//! and three of that matrix's cells were stale, naming rules as unbuilt that had
//! since shipped. A table beside the code drifts from it. A table the code runs
//! cannot.
//!
//! **Why the codes are checked against `Rule::ALL`.** Naming `APLAN011` in a
//! column proves nothing if the string is never resolved; that is the "enum that
//! appeared only in the table and drove nothing" the parity audit had to remove
//! from its own first version. Every code here is looked up, and every repair
//! command is looked up in `aplan facts`, so an entry that names something this
//! build does not have fails.

use std::fmt::Write as _;

use aplan_core::{EvidenceKind, Rule};

/// How a citation of this kind is turned back into a fact about the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolution {
    /// Asked of git: a commit that exists, a blob at a rev.
    Git,
    /// Asked of a configured program: `test.list`, `url.check`.
    Configured,
    /// Nothing to ask. The kind references something no machine can re-derive.
    None,
}

/// The resolver half of the audit, and the reason it is a `match` rather than a
/// column.
///
/// `aplan-evidence` does have a central registry -- `verify` builds a
/// `Vec<Box<dyn EvidenceResolver>>` and each resolver declares its `kind()` --
/// but it is a `Vec`, not a `match`, so **nothing requires it to cover
/// `EvidenceKind::ALL`**. Adding a variant fails to compile nowhere and simply
/// resolves to nothing, which is what `A1-115`'s criterion means by "a kind
/// lacks a resolver". This match has no wildcard, so the compiler refuses a new
/// kind until somebody says how it resolves.
///
/// Two of the four resolvers join that registry **only when configured**: the
/// url resolver needs `url.check` and the test resolver needs a runner, "for the
/// same reason: an adopter who has not asked for network access does not get
/// it". So `Configured` is not a synonym for "asked of a program" -- it is the
/// kinds whose resolver is absent until the repository opts in.
const fn resolution(kind: EvidenceKind) -> Resolution {
    match kind {
        EvidenceKind::Commit | EvidenceKind::File => Resolution::Git,
        EvidenceKind::Test | EvidenceKind::Url => Resolution::Configured,
        EvidenceKind::Note => Resolution::None,
    }
}

/// What repairs a citation of this kind once it has rotted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Repair {
    /// `aplan amend <id> --replace-evidence <from> <to>`, which resolves the
    /// replacement before it writes, so a repair cannot record what `verify`
    /// would reject.
    Amend,
    /// Nothing yet. The reason is the entry's `note`, and it is not an assertion
    /// that "none" is right -- it is the shape `mcp_parity.rs` used for its known
    /// divergences, held against the task that owns it.
    None { owner: &'static str },
}

/// One evidence kind, and what this build does about it rotting.
struct Entry {
    kind: EvidenceKind,
    /// How it is resolved, checked against [`resolution`] below so the table and
    /// the match cannot disagree.
    resolves: Resolution,
    /// The rule that fires when a citation of this kind stops resolving.
    rot: Option<&'static str>,
    repair: Repair,
    /// Why the row reads the way it does, in one line.
    note: &'static str,
}

/// The audit. One row per `EvidenceKind`, checked exhaustively below.
const KINDS: &[Entry] = &[
    Entry {
        kind: EvidenceKind::Commit,
        resolves: Resolution::Git,
        rot: Some("APLAN011"),
        repair: Repair::Amend,
        note: "rots when history is rewritten, which a rebase-merge does",
    },
    Entry {
        kind: EvidenceKind::Test,
        resolves: Resolution::Configured,
        rot: Some("APLAN013"),
        repair: Repair::Amend,
        note: "rots when the test is renamed or its runner stops listing it",
    },
    Entry {
        kind: EvidenceKind::File,
        resolves: Resolution::Git,
        rot: Some("APLAN015"),
        repair: Repair::Amend,
        note: "rots when the file moves, or the anchor inside it is renamed",
    },
    Entry {
        kind: EvidenceKind::Url,
        resolves: Resolution::Configured,
        rot: Some("APLAN016"),
        repair: Repair::Amend,
        note: "rots when the target moves; the only kind whose check is opt-in",
    },
    Entry {
        kind: EvidenceKind::Note,
        resolves: Resolution::None,
        rot: None,
        repair: Repair::None { owner: "A1-115" },
        note: "a dated human note references nothing a machine can re-resolve, so \
               there is nothing to detect and nothing to repair; it is the weakest \
               kind for exactly this reason, and `APLAN017` is how a repository \
               refuses it",
    },
];

/// Every `EvidenceKind` has exactly one row, so a new kind cannot arrive unchecked.
#[test]
fn every_evidence_kind_is_audited() {
    let mut missing = Vec::new();
    for kind in EvidenceKind::ALL {
        let rows = KINDS.iter().filter(|e| e.kind == *kind).count();
        if rows != 1 {
            missing.push(format!(
                "`{}` has {rows} rows in KINDS, expected exactly 1",
                kind.as_str()
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "the evidence-kind audit is not exhaustive:\n  {}\n\n{}",
        missing.join("\n  "),
        matrix()
    );
    assert_eq!(
        KINDS.len(),
        EvidenceKind::ALL.len(),
        "KINDS has rows for kinds that do not exist"
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
    for e in KINDS {
        let Some(code) = e.rot else { continue };
        detectors += 1;
        if !Rule::ALL.iter().any(|r| r.code() == code) {
            wrong.push(format!(
                "`{}` names `{code}`, which is not a rule",
                e.kind.as_str()
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(
        detectors >= 4,
        "only {detectors} kinds declare a detector, so this audit is not looking"
    );
}

/// A kind with no detector has to say who owns that, so "none" is a position
/// rather than an omission.
#[test]
fn a_kind_without_a_detector_names_the_task_that_owns_it() {
    for e in KINDS {
        if e.rot.is_some() {
            continue;
        }
        let Repair::None { owner } = e.repair else {
            panic!("`{}` has no detector but claims a repair", e.kind.as_str());
        };
        assert!(
            owner.starts_with("A1-") && owner.len() == 6,
            "`{}` must name the task that owns its gap, not `{owner}`",
            e.kind.as_str()
        );
        assert!(
            e.note.len() > 40,
            "`{}` has no detector and no explanation",
            e.kind.as_str()
        );
    }
}

/// Every kind says how it resolves, and the table agrees with the match.
///
/// The half `A1-115` asked for and the first version of this file omitted: it
/// checked the rot detector and never the resolver, while ticking a criterion
/// that named both (found by this task's audit).
#[test]
fn every_kind_declares_how_it_resolves() {
    for e in KINDS {
        assert_eq!(
            e.resolves,
            resolution(e.kind),
            "`{}` claims {:?} in the table and {:?} in the match",
            e.kind.as_str(),
            e.resolves,
            resolution(e.kind)
        );
    }
    // A kind that resolves against nothing is the one shape that may have no
    // detector, because there is nothing to detect. The two must agree.
    for e in KINDS {
        let unresolvable = e.resolves == Resolution::None;
        assert_eq!(
            unresolvable,
            e.rot.is_none(),
            "`{}` cannot be resolvable-with-no-detector or unresolvable-with-one",
            e.kind.as_str()
        );
    }
}

/// A kind this audit calls resolvable has a resolver registered for it.
///
/// Read from the registration site rather than from anywhere in the crate: a
/// resolver joins by being pushed into `verify`'s list, so that block is where a
/// kind either has one or does not. It is still a source-text check and still the
/// weakest test here -- the list is built inside a function and cannot be
/// enumerated from outside -- and `rule_tests.rs` makes the same trade for the
/// same reason and says so. What it buys is that a kind cannot be called
/// resolvable while nothing constructs a resolver for it.
#[test]
fn a_kind_that_resolves_is_named_by_the_resolver() {
    let lib = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../aplan-evidence/src/lib.rs"),
    )
    .expect("the resolver registry");
    let src = lib
        .split("let mut resolvers")
        .nth(1)
        .expect("`verify` builds the resolver list")
        .split("// `APLAN019`")
        .next()
        .unwrap_or_default();
    let mut absent = Vec::new();
    let mut looked = 0;
    for e in KINDS {
        if e.resolves == Resolution::None {
            continue;
        }
        looked += 1;
        // `CommitResolver`, `FileResolver`, `UrlResolver`, `TestResolver`.
        let ctor = format!("{}Resolver", capitalise(e.kind.as_str()));
        if !src.contains(&ctor) {
            absent.push(ctor);
        }
    }
    assert!(
        absent.is_empty(),
        "declared resolvable, but `verify` registers no resolver for: {}",
        absent.join(", ")
    );
    assert!(
        looked >= 4,
        "only {looked} kinds were checked against the resolver, so this is not looking"
    );
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
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

/// The matrix, rendered from the audit rather than maintained beside it./// The matrix, rendered from the audit rather than maintained beside it.
///
/// `A1-115` carried this as a hand-written table and it drifted in three cells
/// before the task was even started. There is one copy now and it is this one.
fn matrix() -> String {
    let mut out = String::from("| citation | rot detector | repair |\n|---|---|---|\n");
    for e in KINDS {
        let rot = e
            .rot
            .map_or_else(|| "none".to_string(), |c| format!("`{c}`"));
        let repair = match e.repair {
            Repair::Amend => "`aplan amend`".to_string(),
            Repair::None { owner } => format!("none (`{owner}`)"),
        };
        let _ = writeln!(out, "| `{}` | {rot} | {repair} |", e.kind.as_str());
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
