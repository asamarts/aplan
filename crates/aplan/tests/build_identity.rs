//! Which build is this (`A1-256`).
//!
//! Three pilots could not tell. `p00025` item 9 found a release and a build from
//! main both printing `aplan 0.2.0`, where the release could not run the gate;
//! `p00013` item 6 found two `0.3.0-dev` builds four days apart. Both were
//! answered with "pin by capability", and `aplan facts --require` is that.
//!
//! The ninth pilot is the case that answer cannot reach. `p00002` authored and
//! validated a 64-task plan against a five-week-old binary, and two of the six
//! fixes it was writing about had changed *diagnostics it was quoting*. A change
//! to a sentence earns no rule, command, config key, format, evidence kind or
//! behaviour, so `facts` is byte-identical across it and there is nothing to
//! require (pre-public adoption evidence, second pass).
//!
//! The tool already knew: `attest` records the build commit and the binary's
//! digest. It refuses a dirty tree and a plan that does not verify, both for
//! good reasons, so it cannot answer the question in passing. This can.

use std::path::Path;

mod common;

fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("build-identity-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir")
}

fn identity(dir: &Path) -> serde_json::Value {
    let out = common::run(dir, &["facts", "--identity"]);
    assert_eq!(out.code, 0, "{}", out.text());
    serde_json::from_str(&out.stdout).expect("the identity is json")
}

/// **It answers where nothing else will.**
///
/// No plan and no repository: an adopter asking "is this the build whose output
/// I am reading" is often standing in neither. The dirty-tree case, which is the
/// one the pilot was actually in, is its own test below -- this one's directory
/// has no repository, so `attest` refuses here for a different reason
/// (`A1-256`, audit).
/// Proves the named behaviour `facts.reports_which_build` (`A1-256`).
#[test]
fn the_identity_needs_no_plan_and_no_repository() {
    let dir = scratch();
    let doc = identity(dir.path());

    assert_eq!(doc["schema"], "aplan/tool-identity/v1", "{doc}");
    assert!(
        !doc["version"].as_str().unwrap_or_default().is_empty(),
        "{doc}"
    );
    assert!(
        !doc["facts_sha256"].as_str().unwrap_or_default().is_empty(),
        "{doc}"
    );

    // And `attest`, in the same directory, refuses -- which is the contrast that
    // makes this command worth having rather than a second way to say one thing.
    let attested = common::run(dir.path(), &["attest"]);
    assert_ne!(
        attested.code,
        0,
        "if `attest` answered here there would be nothing to add: {}",
        attested.text()
    );
}

/// **`binary_sha256` is the digest of this executable**, which is what makes it
/// the field that tells two builds apart when nothing else does: it is computed
/// from the artifact, so it cannot be stale.
/// Proves the named behaviour `facts.reports_which_build` (`A1-256`).
#[test]
fn the_binary_digest_is_this_binarys_own() {
    let dir = scratch();
    let doc = identity(dir.path());

    let bytes = std::fs::read(env!("CARGO_BIN_EXE_aplan")).expect("read the binary under test");
    let expected = aplan_attest::sha256_hex(&bytes);
    assert_eq!(
        doc["binary_sha256"].as_str().unwrap_or_default(),
        expected,
        "the digest must name the artifact that printed it: {doc}"
    );
}

/// **`facts_sha256` is the capability contract**, and it is the digest of what
/// `aplan facts` prints, so an auditor reproduces it with `aplan facts |
/// sha256sum` rather than trusting this document.
///
/// It is also the field that *cannot* answer this task's question: two builds
/// differing only in wording share it, which is why `binary_sha256` exists
/// beside it.
#[test]
fn the_facts_digest_is_the_digest_of_the_facts_document() {
    let dir = scratch();
    let doc = identity(dir.path());

    let facts = common::run(dir.path(), &["facts"]);
    assert_eq!(facts.code, 0, "{}", facts.text());
    assert_eq!(
        doc["facts_sha256"].as_str().unwrap_or_default(),
        aplan_attest::sha256_hex(facts.stdout.as_bytes()),
        "an auditor reproduces this with the tool they already have: {doc}"
    );
}

/// **A field with nothing to say is absent, not empty.**
///
/// A binary built from a tarball has no commit to name. An empty string would
/// read as an answer, and a reader scripting against the document would have to
/// know that one of the values means "unknown".
#[test]
fn no_field_is_present_and_empty() {
    let dir = scratch();
    let doc = identity(dir.path());

    let empties: Vec<&String> = doc
        .as_object()
        .expect("an object")
        .iter()
        .filter(|(_, v)| v.as_str().is_some_and(str::is_empty))
        .map(|(k, _)| k)
        .collect();
    assert!(
        empties.is_empty(),
        "these say nothing and should have been left out: {empties:?} in {doc}"
    );
}

/// **It does not answer the other question at the same time.**
///
/// `--require` asks what a build can do and `--identity` asks which build it is.
/// Answering both in one document would put a digest that changes on every build
/// into the output an adopter's gate greps for a verdict.
#[test]
fn identity_and_require_are_separate_questions() {
    let dir = scratch();
    let out = common::run(
        dir.path(),
        &["facts", "--identity", "--require", "traces.table_rows"],
    );
    assert_eq!(out.code, 2, "{}", out.text());
    assert!(out.text().contains("cannot be used with"), "{}", out.text());
}

/// **The committed `facts.json` is unaffected**, which is why the identity is a
/// separate document rather than four more keys in that one.
///
/// `facts.json` is gated byte for byte against what the build emits, and
/// `binary_sha256` differs on every build: putting identity in it would make the
/// drift gate unpassable, and the gate is what keeps the counts honest.
#[test]
fn asking_for_the_identity_does_not_change_the_facts_document() {
    let dir = scratch();
    let facts = common::run(dir.path(), &["facts"]);
    let committed =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../facts.json"))
            .expect("facts.json");
    assert_eq!(
        facts.stdout, committed,
        "the capability document is the one that is committed and gated"
    );
    assert!(
        !committed.contains("binary_sha256"),
        "and it carries no per-build digest"
    );
}

/// A repository with a plan that verifies, and a clean tree: the one state
/// `attest` will speak in.
fn repo_that_attests() -> tempfile::TempDir {
    let dir = scratch();
    let run = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}");
    };
    std::fs::create_dir_all(dir.path().join("aplan")).expect("plan");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("aplan/T-1.md"),
        "---\nid: T-1\ntitle: a task\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\n\
         ## Why\n\nx\n",
    )
    .expect("task");
    run(&["init", "-q"]);
    run(&["config", "user.email", "t@t"]);
    run(&["config", "user.name", "t"]);
    run(&["add", "-A"]);
    run(&["commit", "-qm", "init"]);
    dir
}

/// **Both surfaces report one identity.**
///
/// `attest` embeds it in a provenance document and this prints it alone. Two
/// commands describing one build differently is what `A1-104` exists to stop,
/// and the digest's whole value is that an auditor can compare numbers recorded
/// by different runs (`A1-154`). Compared end to end rather than trusted from
/// the shared function they both call.
#[test]
fn attest_and_identity_agree_about_this_build() {
    let dir = repo_that_attests();

    let attested = common::run(dir.path(), &["attest"]);
    assert_eq!(attested.code, 0, "{}", attested.text());
    let attested: serde_json::Value =
        serde_json::from_str(&attested.stdout).expect("the attestation is json");
    let tool = &attested["tool"];
    let doc = identity(dir.path());

    for field in ["version", "source", "facts_sha256", "binary_sha256"] {
        assert_eq!(
            tool[field], doc[field],
            "`attest` and `facts --identity` disagree about `{field}`:\n  attest:   {}\n               identity: {}",
            tool[field], doc[field]
        );
    }
}

/// **A dirty tree is answered here and refused there**, which is the situation
/// the task was filed for.
///
/// The ninth pilot spent five weeks editing and rebuilding, so its tree was
/// dirty most of the time, and `attest` is unavailable in exactly that state --
/// for a good reason, since an attestation names a state somebody else must be
/// able to reproduce. Asking which build is running carries no such promise.
///
/// The first version of this file tested "no clean tree" in a directory with no
/// repository at all, where `attest` refuses for a different reason entirely
/// (`A1-256`, audit).
/// Proves the named behaviour `facts.reports_which_build` (`A1-256`).
#[test]
fn a_dirty_tree_is_answered_here_and_refused_by_attest() {
    let dir = repo_that_attests();
    std::fs::write(
        dir.path().join("aplan/T-1.md"),
        "---\nid: T-1\ntitle: edited\nphase: 1\nstatus: todo\neffort_days: 2\n---\n\n## Why\n\nx\n",
    )
    .expect("an ordinary edit");

    let refused = common::run(dir.path(), &["attest"]);
    assert_ne!(refused.code, 0, "{}", refused.text());
    assert!(
        refused.text().contains("does not match the commit"),
        "refused for the reason that makes this command necessary: {}",
        refused.text()
    );

    let doc = identity(dir.path());
    assert!(
        !doc["binary_sha256"].as_str().unwrap_or_default().is_empty(),
        "and the identity is still available: {doc}"
    );
}

/// **The two questions are refused in either order**, since a reader who typed
/// them the other way round has the same confusion.
#[test]
fn the_conflict_is_symmetric() {
    let dir = scratch();
    for args in [
        vec!["facts", "--identity", "--require", "traces.table_rows"],
        vec!["facts", "--require", "traces.table_rows", "--identity"],
    ] {
        let out = common::run(dir.path(), &args);
        assert_eq!(out.code, 2, "{args:?}: {}", out.text());
        assert!(
            out.text().contains("cannot be used with"),
            "{args:?}: {}",
            out.text()
        );
    }
}
