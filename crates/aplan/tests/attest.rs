//! `aplan attest`, end to end (`A1-040`).
//!
//! The four claims the acceptance criteria make, each driven through the built
//! binary against a real git repository: the document is stamped with the commit
//! and records what resolved, a plan that does not verify is refused, the note
//! attachment is optional and the plain document stands without it, and two runs
//! at one commit agree byte for byte.

use std::path::Path;
use std::process::Command;

mod common;
use common::aplan;

#[test]
fn the_document_is_stamped_with_the_commit_and_records_what_resolved() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let sha = fixture(dir.path());

    let (out, code) = aplan(dir.path(), &["attest"]);
    assert_eq!(code, 0, "{out}");
    let doc: serde_json::Value = serde_json::from_str(&out).expect("the document is JSON");

    assert_eq!(doc["schema"], "aplan/attestation/v1");
    let (head, _) = git(dir.path(), &["rev-parse", "HEAD"]);
    assert_eq!(
        doc["commit"],
        head.trim(),
        "bound to the tree it was true of, which is HEAD rather than what a task happened to cite"
    );
    // Who verified, in terms a reader can check rather than trust (`A1-154`).
    let tool = &doc["tool"];
    assert_eq!(
        tool["version"],
        env!("CARGO_PKG_VERSION"),
        "a reader needs to know what `verified` meant: {doc}"
    );
    for field in ["facts_sha256", "binary_sha256"] {
        let digest = tool[field].as_str().unwrap_or_default();
        assert_eq!(digest.len(), 64, "{field} is a sha256: {doc}");
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()), "{doc}");
    }
    // The one an auditor checks first: the digest of the binary that ran, as
    // `sha256sum` spells it.
    let exe = std::fs::read(common::binary()).expect("read the binary");
    assert_eq!(
        tool["binary_sha256"],
        aplan_attest::sha256_hex(&exe),
        "the document names the binary that produced it: {doc}"
    );
    // `source` is a bare commit or nothing: the first version appended
    // `git describe --dirty`, and a marker a build script cannot keep true has
    // no place in a provenance record (`A1-154`, found by its audit).
    let source = tool["source"].as_str().expect("a string, empty or a sha");
    assert!(
        source.is_empty() || (source.len() == 40 && source.chars().all(|c| c.is_ascii_hexdigit())),
        "source must be a bare commit sha or empty, not {source:?}"
    );

    // And the capability contract is what `aplan facts` prints, so
    // `aplan facts | sha256sum` reproduces it.
    let (facts, _) = aplan(dir.path(), &["facts"]);
    assert_eq!(
        tool["facts_sha256"],
        aplan_attest::sha256_hex(facts.as_bytes()),
        "reproducible with the tools a reader has: {doc}"
    );
    // Which claim this is: a runner listed the test, or the test was run.
    assert_eq!(doc["tests_executed"], false, "{doc}");

    let tasks = doc["tasks"].as_array().expect("tasks");
    assert_eq!(tasks.len(), 1, "only a claim can be attested: {doc}");
    assert_eq!(tasks[0]["id"], "A-01");

    // The commit citation is recorded as the full sha it resolved to, so a
    // reader re-derives it rather than trusting this document.
    let evidence = tasks[0]["evidence"].as_array().expect("evidence");
    let commit = evidence
        .iter()
        .find(|e| e["kind"] == "commit")
        .expect("the commit citation");
    assert_eq!(commit["resolved"], sha);
    assert_ne!(
        commit["reference"], commit["resolved"],
        "the short ref was resolved, which is the point"
    );

    // A `file` citation carries the blob sha the file had at that commit.
    let file = evidence
        .iter()
        .find(|e| e["kind"] == "file")
        .expect("the file citation");
    let blob = file["resolved"].as_str().expect("a blob sha");
    let (git_blob, _) = git(dir.path(), &["rev-parse", "HEAD:src/lib.rs"]);
    assert_eq!(blob, git_blob.trim(), "git re-derives it");
}

/// An attestation of an unproven plan is worse than none.
#[test]
fn a_plan_that_does_not_verify_is_refused() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // A citation to a commit that does not exist: `verify` reports it, so this
    // must refuse rather than record a document that says nothing went wrong.
    std::fs::write(
        dir.path().join("aplan/tasks/A-02.md"),
        "---\nid: A-02\ntitle: Closed on a commit nobody has\nphase: 1\nstatus: done\n\
         effort_days: 1\npaths:\n- src/**\nevidence:\n  - kind: commit\n    \
         ref: 0000000000000000000000000000000000000000\n---\n\nbody\n",
    )
    .expect("task");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-m", "a claim nothing backs"]);

    let (out, code) = aplan(dir.path(), &["attest"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("does not verify"), "{out}");
    assert!(out.contains("aplan verify"), "the answer is named: {out}");
    assert!(!out.contains("aplan/attestation/v1"), "no document: {out}");
}

/// A dirty tree names a state nobody else can reproduce.
#[test]
fn an_uncommitted_change_is_refused() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(dir.path().join("src/lib.rs"), "// edited, not committed\n").expect("edit");

    let (out, code) = aplan(dir.path(), &["attest"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("does not match the commit"), "{out}");
    assert!(out.contains("1 tracked path(s) changed"), "{out}");
}

/// An *untracked* task file is the same failure wearing a different hat, and it
/// is the one that broke the guarantee: it changed the document while the commit
/// stayed the same, so two people at one commit got different attestations and
/// the document recorded a task nobody else had (`A1-040`, found by its audit).
#[test]
fn an_untracked_file_is_refused_because_it_is_not_in_the_tree() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (before, code) = aplan(dir.path(), &["attest"]);
    assert_eq!(code, 0, "{before}");

    // A task that would verify perfectly well: the only thing wrong with it is
    // that git has never seen the file, which is the whole point.
    let (sha, _) = git(dir.path(), &["rev-parse", "HEAD"]);
    std::fs::write(
        dir.path().join("aplan/tasks/A-99.md"),
        format!(
            "---\nid: A-99\ntitle: Done, in a file git has never seen\nphase: 1\nstatus: done\n\
             effort_days: 1\npaths:\n- src/**\nevidence:\n  - kind: commit\n    ref: '{}'\n\
             ---\n\nbody\n",
            sha.trim()
        ),
    )
    .expect("task");

    let (out, code) = aplan(dir.path(), &["attest"]);
    assert_ne!(code, 0, "an untracked task file must be refused: {out}");
    assert!(out.contains("1 untracked path(s)"), "{out}");
    assert!(!out.contains("A-99"), "and nothing is recorded: {out}");
    // The tree is judged before any citation is resolved, so the reader is told
    // the actionable thing rather than a finding about a task the tree does not
    // have (`A1-040`, its audit).
    assert!(
        !out.contains("does not verify"),
        "the tree is the first answer, not the plan: {out}"
    );

    // Ignored files are what the repository has already said are not part of it.
    std::fs::remove_file(dir.path().join("aplan/tasks/A-99.md")).expect("rm");
    std::fs::write(dir.path().join("scratch.tmp"), "noise\n").expect("write");
    std::fs::write(dir.path().join(".gitignore"), "scratch.tmp\n").expect("ignore");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "ignore scratch"]);
    let (after, code) = aplan(dir.path(), &["attest"]);
    assert_eq!(code, 0, "an ignored file is not a dirty tree: {after}");
}

/// A `commit` citation says whether the attested commit can reach it.
#[test]
fn a_citation_on_an_unmerged_branch_is_recorded_as_unreachable() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    git(dir.path(), &["checkout", "-q", "-b", "side"]);
    std::fs::write(dir.path().join("src/side.rs"), "// side work\n").expect("write");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "nobody merged this"]);
    let (side, _) = git(dir.path(), &["rev-parse", "HEAD"]);
    let side = side.trim().to_string();
    git(dir.path(), &["checkout", "-q", "-"]);

    std::fs::write(
        dir.path().join("aplan/tasks/A-03.md"),
        format!(
            "---\nid: A-03\ntitle: Closed on an unmerged branch\nphase: 1\nstatus: done\n\
             effort_days: 1\npaths:\n- src/**\nevidence:\n  - kind: commit\n    ref: '{side}'\n\
             ---\n\nbody\n"
        ),
    )
    .expect("task");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "cite the side commit"]);

    // `verify` is happy: the cited commit's diff touched the declared paths.
    let (verified, code) = aplan(dir.path(), &["verify"]);
    assert_eq!(code, 0, "{verified}");

    let (out, code) = aplan(dir.path(), &["attest"]);
    assert_eq!(code, 0, "{out}");
    let doc: serde_json::Value = serde_json::from_str(&out).expect("json");
    let tasks = doc["tasks"].as_array().expect("tasks");
    let find = |id: &str| {
        tasks
            .iter()
            .find(|t| t["id"] == id)
            .expect("the task")
            .clone()
    };
    let merged = find("A-01");
    assert_eq!(merged["evidence"][0]["reachable"], true);
    let unmerged = find("A-03");
    assert_eq!(
        unmerged["evidence"][0]["reachable"], false,
        "a reader checking out this commit will not find that work: {out}"
    );
    assert_eq!(
        unmerged["evidence"][0]["resolved"], side,
        "recorded rather than refused: the citation is not a lie"
    );
}

/// Deterministic for a given commit: no timestamp, sorted keys, stable order.
#[test]
fn two_runs_at_one_commit_agree_byte_for_byte() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (first, _) = aplan(dir.path(), &["attest"]);
    let (second, _) = aplan(dir.path(), &["attest"]);
    assert_eq!(first, second, "an attestation is a function of the commit");
    // The property behind the equality: nothing in the document is a clock.
    let doc: serde_json::Value = serde_json::from_str(&first).expect("json");
    let keys: Vec<&String> = doc.as_object().expect("object").keys().collect();
    assert!(
        keys.iter()
            .all(|k| !k.contains("date") && !k.contains("time")),
        "a timestamp would make it a function of when it ran: {keys:?}"
    );
}

/// Signing is optional, and what ships is attachment rather than signature:
/// tamper-evident because git hashes it. The unsigned document is the useful
/// thing on day one, which is why the two are separate.
#[test]
fn the_note_is_optional_and_the_document_stands_without_it() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // Without the flag, nothing is written to the repository at all.
    let (plain, code) = aplan(dir.path(), &["attest"]);
    assert_eq!(code, 0);
    let (notes, _) = git(dir.path(), &["notes", "--ref=aplan-attest", "list"]);
    assert!(notes.trim().is_empty(), "nothing attached: {notes}");

    let (out, code) = aplan(dir.path(), &["attest", "--attach-note", "aplan-attest"]);
    assert_eq!(code, 0, "{out}");
    let (shown, _) = git(dir.path(), &["notes", "--ref=aplan-attest", "show"]);
    assert_eq!(
        shown.trim(),
        plain.trim(),
        "the note carries the same document, not a second spelling of it"
    );

    // A note already there is somebody's record, and overwriting it is not this
    // command's decision.
    let (again, code) = aplan(dir.path(), &["attest", "--attach-note", "aplan-attest"]);
    assert_ne!(code, 0, "a second attach must refuse: {again}");
}

/// A repository with a commit, a plan, and one closed task whose evidence
/// resolves. Returns the commit sha the task cites.
/// A file that does not load is named, not counted. The count alone
/// ("1 task file(s) failed to load") is what a CI log gave this author while a
/// fixture was quietly unloadable, and it does not say which file or why
/// (`A1-158`).
#[test]
fn a_file_that_does_not_load_is_named_in_the_refusal() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/A-BAD.md"),
        "---\nid: A-BAD\ntitle: t\nphase: 1\nstatus: done\neffort_days: 1\n\
         evidence:\n  - kind: commit\n    ref: 12345678\n---\n\nbody\n",
    )
    .expect("task");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "an unloadable task"]);

    let (out, code) = aplan(dir.path(), &["attest"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("task file(s) failed to load"), "{out}");
    assert!(
        out.contains("YAML read `12345678` as a number"),
        "the refusal must say what is wrong with the file, not only that one is: {out}"
    );
    assert!(out.contains("aplan check"), "{out}");
}

/// The document says the tests were executed *and* what that was worth
/// (`A1-178`).
///
/// `tests_executed` says the commands were run. `A1-167` measured a runner whose
/// filter matched no test -- everything skipped, status `0`, read as a pass --
/// so `verify --execute` reported `APLAN037` on the line above while the
/// attestation carried the strongest claim this tool can make about a run that
/// did not happen. This is that contradiction, reproduced and then closed.
#[test]
fn an_execution_that_proved_nothing_is_recorded_beside_the_claim_that_it_ran() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // A runner in the shape `A1-167` measured: the filter matches a test's own
    // title, and the listing prints a file-qualified name that matches none.
    let selector = "hit=0; for t in adds zero; do case \"$t\" in *\"$1\"*) hit=1 ;; esac; done; \
                    [ $hit = 1 ] && echo 'Tests  1 passed (2)' || echo 'Tests  2 skipped (2)'";
    std::fs::write(
        dir.path().join("runner.sh"),
        format!("#!/bin/sh\n{selector}\nexit 0\n"),
    )
    .expect("runner");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            dir.path().join("runner.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .expect("chmod");
    }
    let blind = "test:\n  name: blind\n  list: [\"printf\", \"tests/math.test.js > adds\\n\"]\n  \
                 run: [\"./runner.sh\", \"{test}\"]\n";
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!("root: aplan\n{blind}"),
    )
    .expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/A-02.md"),
        "---\nid: A-02\ntitle: proved by a test that never ran\nphase: 1\nstatus: done\n\
         effort_days: 1\npaths:\n- src/**\nevidence:\n  - kind: test\n    \
         ref: \"tests/math.test.js > adds\"\n---\n\nbody\n",
    )
    .expect("task");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "a blind runner"]);

    // `verify` says it.
    let (out, _) = aplan(dir.path(), &["verify", "--execute"]);
    assert!(out.contains("APLAN037"), "the fixture must be blind: {out}");

    // And so does the document, which is the point: its reader was not there.
    let (doc, code) = aplan(dir.path(), &["attest", "--execute"]);
    assert_eq!(code, 0, "a warning does not stop an attestation: {doc}");
    let parsed: serde_json::Value = serde_json::from_str(&doc).expect("json");
    assert_eq!(parsed["tests_executed"], true, "{doc}");
    assert_eq!(
        parsed["runners_that_proved_nothing"], 1,
        "the claim is qualified where it is made: {doc}"
    );

    // Twice at one commit, still byte for byte: the qualification is a fact
    // about the plan and the tool, not about when either was asked.
    let (again, _) = aplan(dir.path(), &["attest", "--execute"]);
    assert_eq!(doc, again, "an attestation is a function of the commit");
}

/// A document with nothing to qualify is the document it was before the field
/// existed (`A1-178`).
///
/// Absent rather than `0`, so a repository whose runners select properly gets
/// byte-identical output -- which the determinism this command promises
/// requires, and which keeps the field's presence the signal.
#[test]
fn a_run_that_proved_something_carries_no_qualification_at_all() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let (doc, code) = aplan(dir.path(), &["attest"]);
    assert_eq!(code, 0, "{doc}");
    assert!(
        !doc.contains("runners_that_proved_nothing"),
        "nothing to qualify, so nothing said: {doc}"
    );
    let parsed: serde_json::Value = serde_json::from_str(&doc).expect("json");
    assert_eq!(parsed["tests_executed"], false, "{doc}");
}

fn fixture(dir: &Path) -> String {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("src")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.join("src/lib.rs"), "// the work\n").expect("src");
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "t@example.com"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "the work"]);
    let (sha, _) = git(dir, &["rev-parse", "HEAD"]);
    let sha = sha.trim().to_string();

    std::fs::write(
        dir.join("aplan/tasks/A-01.md"),
        format!(
            "---\nid: A-01\ntitle: Done, and provable\nphase: 1\nstatus: done\neffort_days: 1\n\
             paths:\n- src/**\nevidence:\n  - kind: commit\n    ref: '{}'\n  \
             - kind: file\n    ref: src/lib.rs\n---\n\nbody\n",
            // Quoted: about one short sha in forty-three is all digits, and
            // YAML reads that as an integer. Unquoted, these seven tests failed
            // together in CI on one unlucky commit and passed everywhere else
            // (`A1-158`).
            &sha[..8]
        ),
    )
    .expect("task");
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "the plan"]);
    sha
}

fn git(dir: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("running git");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}
