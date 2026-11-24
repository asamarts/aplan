//! `close` refuses evidence `verify` would reject (`A1-109`).
//!
//! The first pilot closed a task citing three integration tests, was told
//! `closed`, and `verify` rejected all three seconds later: the citations used
//! `guider::name` where the runner lists them bare
//! (pre-public adoption evidence, item 10). **The tool contradicted itself about its
//! own central claim**, and the task sat `done` on evidence the project's own
//! gate rejects until a separate command was run.
//!
//! Driven through the built binary rather than through `resolve_proposed`, for
//! `A1-107`'s reason: a unit test on the resolver proves the resolver. If `close`
//! stopped calling it tomorrow, every such test would still pass. The runner here
//! is a shell script rather than Cargo, which also makes these the
//! non-Cargo-runner exercise `A1-034` asks for, in passing.
//!
//! Not a `trycmd` case: closing writes, and the corpus runs in its own fixture
//! directory with no sandbox, so a writing case edits the fixture it is pinned
//! against.

use std::path::Path;

use crate::common;

/// A git repository whose runner lists exactly one test, bare, and whose task
/// owns `tests/**`.
///
/// The listed name is the pilot's, so the fixture reproduces the mistake that was
/// actually made rather than a shorter one invented for the test.
fn fixture(dir: &Path) -> String {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("tests")).expect("mkdir tests");
    std::fs::write(
        dir.join(".aplan.yml"),
        "root: aplan\ntest:\n  list: [\"sh\", \"-c\", \
         \"echo a_settledone_that_overtakes_the_reply_is_not_lost\"]\n  \
         run: [\"sh\", \"-c\", \"true\", \"{test}\"]\n",
    )
    .expect("config");
    std::fs::write(
        dir.join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: a task\nphase: 1\nstatus: todo\nowner: t\n\
         effort_days: 1\npaths:\n- tests/**\n---\n\nbody\n",
    )
    .expect("task");
    std::fs::write(dir.join("tests/t.rs"), "fn main() {}\n").expect("test file");
    git(dir, &["init", "-q", "."]);
    git(dir, &["config", "user.email", "t@example.com"]);
    git(dir, &["config", "user.name", "t"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "initial"]);
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .expect("git rev-parse");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?} failed");
}

fn run(dir: &Path, args: &[&str]) -> (String, String, bool) {
    let out = common::output(dir, args);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

fn status_of(dir: &Path) -> String {
    let text = std::fs::read_to_string(dir.join("aplan/tasks/A-1.md")).expect("read");
    text.lines()
        .find(|l| l.starts_with("status:"))
        .expect("a status line")
        .to_string()
}

/// `APLAN019` got quieter and the close gate did not (`A1-138`).
///
/// The rule now fires only where a citation is judged against the task's paths,
/// which silences the 140 findings the fourth pilot met on `todo` tasks whose
/// globs name a tree a later cut creates. The moment a citation *appears* is the
/// moment the glob starts mattering, and this is that moment: closing a task
/// whose `paths` reach nothing is still refused, with the same message the pilot
/// quoted approvingly.
#[test]
fn closing_a_task_whose_paths_bind_nothing_is_still_refused() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let sha = fixture(dir.path());
    // A forward-looking glob, of exactly the shape the rule stopped reporting.
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: a task\nphase: 1\nstatus: todo\nowner: t\n\
         effort_days: 1\npaths:\n- crates/not-yet/**\n---\n\nbody\n",
    )
    .expect("task");

    // Silent while nothing is cited: this is what p00020 needed.
    let (_, err, ok) = run(dir.path(), &["verify"]);
    assert!(
        ok,
        "a forward-looking path is not a finding on its own: {err}"
    );

    // And refused the moment a citation would bind to it.
    let (out, err, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", &format!("commit:{sha}")],
    );
    let said = format!("{out}{err}");
    assert!(!ok, "the closure was accepted: {said}");
    assert!(said.contains("matches no tracked file"), "{said}");
    assert!(
        said.contains("binds to nothing through it"),
        "the message names what is unbound: {said}"
    );
    assert_eq!(status_of(dir.path()), "status: todo", "it wrote anyway");
}

#[test]
fn the_pilots_citation_is_refused_and_the_right_form_is_named() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (stdout, _, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:guider::a_settledone_that_overtakes_the_reply_is_not_lost",
        ],
    );

    assert!(!ok, "the close must fail; it printed: {stdout}");
    assert!(
        stdout.contains("APLAN013"),
        "the refusal must cite the rule that would have rejected it later: {stdout}"
    );
    // The near match is the whole difference between a typo and a dead end.
    assert!(
        stdout.contains("but `a_settledone_that_overtakes_the_reply_is_not_lost` is"),
        "the refusal must name what the runner does list: {stdout}"
    );
    assert!(
        stdout.contains("--no-resolve"),
        "a refusal must say how to proceed deliberately: {stdout}"
    );
    // The point of refusing before writing.
    assert_eq!(
        status_of(dir.path()),
        "status: todo",
        "a refused close must not have written anything"
    );
}

#[test]
fn the_citation_the_runner_lists_closes() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (stdout, stderr, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:a_settledone_that_overtakes_the_reply_is_not_lost",
        ],
    );

    assert!(ok, "a resolvable citation must close: {stdout}{stderr}");
    assert_eq!(status_of(dir.path()), "status: done");
    // A gate that refused everything would pass the test above and be useless.
    assert!(
        !stdout.contains("APLAN013"),
        "a good citation must not be reported: {stdout}"
    );
}

/// Proves `close.refuses_alternate_evidence_keys` through the public command:
/// alternate source spellings are existing fields, not permission to append a
/// second `evidence` key. Every refusal is usage, actionable and byte-exact.
#[test]
fn initial_close_refuses_alternate_evidence_keys_without_writing() {
    for (name, field) in [
        ("single quoted", "'evidence': []\n"),
        ("double quoted", "\"evidence\": []\n"),
        ("escaped double quoted", "\"\\u0065vidence\": []\n"),
        ("explicit", "? evidence\n: []\n"),
        ("tagged key", "!!str evidence: []\n"),
        ("anchored key", "&evidence_key evidence: []\n"),
        (
            "aliased key",
            "owner: &evidence_key evidence\n*evidence_key: []\n",
        ),
        (
            "nonempty double quoted",
            "\"evidence\":\n  - kind: file\n    ref: tests/t.rs\n",
        ),
        ("ordinary", "evidence: []\n"),
    ] {
        let dir = tempfile::Builder::new()
            .prefix("close-alternate-evidence-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        fixture(dir.path());
        let path = dir.path().join("aplan/tasks/A-1.md");
        let original = std::fs::read_to_string(&path).expect("task");
        let source = original
            .replace("owner: t\n", "")
            .replace("status: todo\n", &format!("status: todo\n{field}"));
        std::fs::write(&path, &source).expect("alternate evidence source");

        let out = common::run(
            dir.path(),
            &[
                "close",
                "A-1",
                "--evidence",
                "file:.aplan.yml",
                "--no-resolve",
            ],
        );
        assert_eq!(out.code, 2, "{name} reached the wrong exit: {out:?}");
        assert!(
            out.text().contains("live task")
                && out.text().contains("Review")
                && out.text().contains("reconcile or remove"),
            "{name} was not actionable: {out:?}"
        );
        assert!(!out.text().contains("bug in aplan"), "{name}: {out:?}");
        assert!(!out.text().contains("aplan amend"), "{name}: {out:?}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("task after refusal"),
            source,
            "{name} changed the task before refusing"
        );
    }
}

/// Proves the named behaviours `close.replaces_live_trace_fingerprints` and
/// `mutation.block_replacements_cross_yaml_trivia` through the real command,
/// with both proposed citations resolved before the rewrite.
#[test]
fn close_replaces_a_live_fingerprint_block_end_to_end() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let doc = "# Intro\n\nThe reviewed contract.\n";
    std::fs::write(dir.path().join("DOC.md"), doc).expect("doc");
    let config = std::fs::read_to_string(dir.path().join(".aplan.yml")).expect("config");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!("detect_drift: true\n{config}"),
    )
    .expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: a task\nphase: 1\nstatus: in_progress\n\
         trace_fingerprints:\n  DOC.md#old: stale-first\n\n\
         # Keep this author note.\n  DOC.md#intro: stale-second\nowner: t\neffort_days: 1\n\
         paths:\n- tests/**\n- DOC.md\ntraces:\n- DOC.md#intro\n---\n\nbody sentinel\n",
    )
    .expect("task");
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &["commit", "-q", "-m", "live task with a stale stamp"],
    );

    // Resolution still happens before the rewrite. Exercise that refusal on
    // the exact stale-mapping input, not merely on a simpler todo task, and
    // compare the whole file to prove the replacement path is atomic too.
    let before =
        std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("original task");
    let (stdout, stderr, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:this_runner_does_not_list_it",
        ],
    );
    assert!(!ok, "unresolved evidence closed the task: {stdout}{stderr}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("refused task"),
        before,
        "a refused close changed the stale-mapping input"
    );

    let (stdout, stderr, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "file:DOC.md",
            "--evidence",
            "test:a_settledone_that_overtakes_the_reply_is_not_lost",
        ],
    );
    assert!(ok, "the accepted live task did not close: {stdout}{stderr}");
    assert!(
        stdout.contains("1 trace fingerprint(s), 1 evidence fingerprint(s) stamped"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("unresolved"),
        "both citations should have resolved: {stdout}"
    );

    let written =
        std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("written task");
    assert_eq!(
        written.matches("trace_fingerprints:").count(),
        1,
        "{written}"
    );
    assert!(!written.contains("stale-first"), "{written}");
    assert!(!written.contains("stale-second"), "{written}");
    assert!(
        written.contains("\n# Keep this author note.\nowner: t\n"),
        "{written}"
    );
    assert!(
        written.find("trace_fingerprints:").unwrap() < written.find("owner:").unwrap(),
        "the block moved: {written}"
    );
    assert!(written.ends_with("\nbody sentinel\n"), "{written}");

    let (frontmatter, _) = aplan_core::load::split_frontmatter(&written).expect("frontmatter");
    let task: aplan_core::Task = yaml_compat::compat::from_str(frontmatter).expect("task parses");
    assert_eq!(task.status, aplan_core::Status::Done);
    assert_eq!(
        task.evidence
            .iter()
            .map(|e| (e.kind, e.reference.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (aplan_core::EvidenceKind::File, "DOC.md"),
            (
                aplan_core::EvidenceKind::Test,
                "a_settledone_that_overtakes_the_reply_is_not_lost"
            ),
        ]
    );
    assert_eq!(
        task.trace_fingerprints.get("DOC.md#intro"),
        aplan_core::anchor::stamp_in("DOC.md", doc, "intro").as_ref()
    );
    assert!(
        task.evidence_fingerprints.contains_key("file:DOC.md"),
        "the mutable file citation was not stamped: {:?}",
        task.evidence_fingerprints
    );
}

/// Proves `close.refuses_structural_fingerprint_mappings`: every command path
/// that replaces a derived baseline recognises a mapping shared by anchor and
/// alias as plan input, exits actionably, and leaves both the exact bytes and
/// parsed task untouched.
#[test]
fn fingerprint_rewrites_refuse_structural_aliases_without_writing() {
    let cases: &[(&str, &[&str])] = &[
        (
            "todo",
            &[
                "close",
                "A-1",
                "--evidence",
                "file:.aplan.yml",
                "--no-resolve",
            ],
        ),
        ("done", &["close", "A-1", "--restamp"]),
        (
            "done",
            &[
                "amend",
                "A-1",
                "--add-evidence",
                "test:new_proof",
                "--no-resolve",
            ],
        ),
    ];

    for (status, command) in cases {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        fixture(dir.path());
        let config = std::fs::read_to_string(dir.path().join(".aplan.yml")).expect("config");
        std::fs::write(
            dir.path().join(".aplan.yml"),
            format!("detect_drift: true\n{config}"),
        )
        .expect("enable drift detection");
        let evidence = if *status == "done" {
            "evidence:\n  - kind: file\n    ref: .aplan.yml\n"
        } else {
            ""
        };
        let source = format!(
            "---\nid: A-1\ntitle: a task\nphase: 1\nstatus: {status}\n{evidence}\
             trace_fingerprints: &stamps\n  file:.aplan.yml: stale\n\
             evidence_fingerprints: *stamps\neffort_days: 1\npaths:\n- tests/**\n---\n\n\
             body sentinel\n"
        );
        let path = dir.path().join("aplan/tasks/A-1.md");
        std::fs::write(&path, &source).expect("structural task");
        let (frontmatter, _) = aplan_core::load::split_frontmatter(&source).expect("frontmatter");
        let before: aplan_core::Task =
            yaml_compat::from_str(frontmatter).expect("shared mapping task loads");

        let out = common::run(dir.path(), command);
        assert_eq!(
            out.code,
            2,
            "{command:?} reached a non-actionable exit: {}",
            out.text()
        );
        let said = out.text();
        assert!(said.contains("fingerprints"), "{command:?}: {said}");
        assert!(
            said.contains("simplify that field by hand"),
            "{command:?}: {said}"
        );
        assert!(!said.contains("bug in aplan"), "{command:?}: {said}");

        let after_source = std::fs::read_to_string(&path).expect("task after refusal");
        assert_eq!(after_source, source, "{command:?} wrote despite refusing");
        let (frontmatter, _) =
            aplan_core::load::split_frontmatter(&after_source).expect("frontmatter after");
        let after: aplan_core::Task =
            yaml_compat::from_str(frontmatter).expect("unchanged task still loads");
        assert_eq!(after, before, "{command:?} changed the parsed task");
    }
}

/// Proves `close.refuses_structural_evidence_sequences`: every explicit
/// citation edit recognises a nested anchor used by another field before the
/// evidence block is reconstructed, reports plan input, and changes neither
/// the exact source nor the parsed task.
#[test]
fn evidence_rewrites_refuse_structural_aliases_without_writing() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let path = dir.path().join("aplan/tasks/A-1.md");
    let source = concat!(
        "---\n",
        "id: A-1\n",
        "title: a task\n",
        "phase: 1\n",
        "status: done\n",
        "effort_days: 1\n",
        "paths:\n",
        "  - tests/**\n",
        "evidence:\n",
        "  - kind: file\n",
        "    ref: &shared .aplan.yml\n",
        "  - kind: test\n",
        "    ref: existing_proof\n",
        "owner: *shared\n",
        "---\n\n",
        "body sentinel\n",
    );
    std::fs::write(&path, source).expect("structural evidence task");
    let (frontmatter, _) = aplan_core::load::split_frontmatter(source).expect("frontmatter");
    let before: aplan_core::Task =
        yaml_compat::from_str(frontmatter).expect("structural evidence task loads");

    let commands: &[&[&str]] = &[
        &[
            "amend",
            "A-1",
            "--add-evidence",
            "test:new_proof",
            "--no-resolve",
        ],
        &[
            "amend",
            "A-1",
            "--replace-evidence",
            "file:.aplan.yml",
            "test:new_proof",
            "--no-resolve",
        ],
        &["amend", "A-1", "--remove-evidence", "file:.aplan.yml"],
    ];
    for command in commands {
        let out = common::run(dir.path(), command);
        assert_eq!(out.code, 2, "{command:?}: {}", out.text());
        let said = out.text();
        assert!(said.contains("evidence"), "{command:?}: {said}");
        assert!(
            said.contains("simplify that field by hand"),
            "{command:?}: {said}"
        );
        assert!(!said.contains("bug in aplan"), "{command:?}: {said}");

        let after_source = std::fs::read_to_string(&path).expect("task after refusal");
        assert_eq!(after_source, source, "{command:?} wrote despite refusing");
        let (frontmatter, _) =
            aplan_core::load::split_frontmatter(&after_source).expect("frontmatter after");
        let after: aplan_core::Task =
            yaml_compat::from_str(frontmatter).expect("unchanged task still loads");
        assert_eq!(after, before, "{command:?} changed the parsed task");
    }
}

/// Proof for `close.preserves_fingerprint_mapping_boundary_trivia`: replacing
/// or clearing either derived map through the real command retains its authored
/// key boundary and note while changing no unrelated byte.
#[test]
fn fingerprint_rewrites_preserve_mapping_boundary_trivia() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let config = std::fs::read_to_string(dir.path().join(".aplan.yml")).expect("config");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!("detect_drift: true\n{config}"),
    )
    .expect("enable drift detection");
    std::fs::write(
        dir.path().join("DOC.md"),
        "# Intro\n\nThe reviewed contract.\n",
    )
    .expect("traced document");
    let task_path = dir.path().join("aplan/tasks/A-1.md");
    let source = concat!(
        "---\n",
        "id: A-1\n",
        "title: a task\n",
        "phase: 1\n",
        "status: done\n",
        "evidence:\n",
        "  - kind: file\n",
        "    ref: .aplan.yml\n",
        "traces:\n",
        "  - DOC.md#intro\n",
        "trace_fingerprints :   {DOC.md#intro: stale-trace}  # trace boundary\n",
        "evidence_fingerprints : # evidence boundary\n",
        "  file:.aplan.yml: stale-evidence\n",
        "effort_days: 1\n",
        "paths:\n",
        "  - tests/**\n",
        "---\n\n",
        "body sentinel\n",
    );
    std::fs::write(&task_path, source).expect("task with stale baselines");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "add baselines"]);

    let (stdout, stderr, ok) = run(dir.path(), &["close", "A-1", "--restamp"]);
    assert!(ok, "restamp failed: {stdout}{stderr}");
    let written = std::fs::read_to_string(&task_path).expect("restamped task");
    let (frontmatter, _) =
        aplan_core::load::split_frontmatter(&written).expect("restamped frontmatter");
    let task: aplan_core::Task = yaml_compat::from_str(frontmatter).expect("restamped task parses");
    let trace = task
        .trace_fingerprints
        .get("DOC.md#intro")
        .expect("trace baseline");
    let evidence = task
        .evidence_fingerprints
        .get("file:.aplan.yml")
        .expect("evidence baseline");
    let expected = format!(
        concat!(
            "---\n",
            "id: A-1\n",
            "title: a task\n",
            "phase: 1\n",
            "status: done\n",
            "evidence:\n",
            "  - kind: file\n",
            "    ref: .aplan.yml\n",
            "traces:\n",
            "  - DOC.md#intro\n",
            "trace_fingerprints :     # trace boundary\n",
            "  DOC.md#intro: {}\n",
            "evidence_fingerprints : # evidence boundary\n",
            "  file:.aplan.yml: {}\n",
            "effort_days: 1\n",
            "paths:\n",
            "  - tests/**\n",
            "---\n\n",
            "body sentinel\n",
        ),
        trace, evidence
    );
    assert_eq!(written, expected);

    let clear_dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("clear tempdir");
    fixture(clear_dir.path());
    let config =
        std::fs::read_to_string(clear_dir.path().join(".aplan.yml")).expect("clear config");
    std::fs::write(
        clear_dir.path().join(".aplan.yml"),
        format!("detect_drift: true\n{config}"),
    )
    .expect("enable drift detection");
    let clear_path = clear_dir.path().join("aplan/tasks/A-1.md");
    let clear_source = concat!(
        "---\n",
        "id: A-1\n",
        "title: a task\n",
        "phase: 1\n",
        "status: done\n",
        "evidence:\n",
        "  - kind: test\n",
        "    ref: a_settledone_that_overtakes_the_reply_is_not_lost\n",
        "trace_fingerprints : {DOC.md#old: orphaned} # trace boundary\n",
        "evidence_fingerprints : # evidence boundary\n",
        "  file:OLD.md: orphaned\n",
        "effort_days: 1\n",
        "paths:\n",
        "  - tests/**\n",
        "---\n\n",
        "body sentinel\n",
    );
    std::fs::write(&clear_path, clear_source).expect("task with orphaned baselines");
    git(clear_dir.path(), &["add", "-A"]);
    git(
        clear_dir.path(),
        &["commit", "-q", "-m", "add orphaned baselines"],
    );

    let (stdout, stderr, ok) = run(clear_dir.path(), &["close", "A-1", "--restamp"]);
    assert!(ok, "clearing restamp failed: {stdout}{stderr}");
    assert_eq!(
        std::fs::read_to_string(clear_path).expect("cleared task"),
        concat!(
            "---\n",
            "id: A-1\n",
            "title: a task\n",
            "phase: 1\n",
            "status: done\n",
            "evidence:\n",
            "  - kind: test\n",
            "    ref: a_settledone_that_overtakes_the_reply_is_not_lost\n",
            "# trace boundary\n",
            "# evidence boundary\n",
            "effort_days: 1\n",
            "paths:\n",
            "  - tests/**\n",
            "---\n\n",
            "body sentinel\n",
        )
    );
}

/// The whole repair loop through the real command and verifier. It also proves
/// `close.restamp_requires_drift_detection`: turning detection off cannot make
/// an operation called "restamp" silently erase the baselines it cannot refresh.
#[test]
fn close_restamp_refreshes_evidence_fingerprints_end_to_end() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let config = std::fs::read_to_string(dir.path().join(".aplan.yml")).expect("config");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!("detect_drift: true\n{config}"),
    )
    .expect("config");
    std::fs::write(dir.path().join("DOC.md"), "# Proof\n\nreviewed\n").expect("proof");
    let task_path = dir.path().join("aplan/tasks/A-1.md");
    let task = std::fs::read_to_string(&task_path).expect("task");
    std::fs::write(
        &task_path,
        task.replace("- tests/**\n", "- tests/**\n- DOC.md\n"),
    )
    .expect("task paths");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "add proof"]);

    let (stdout, stderr, ok) = run(dir.path(), &["close", "A-1", "--evidence", "file:DOC.md"]);
    assert!(ok, "close failed: {stdout}{stderr}");
    let closed = std::fs::read_to_string(&task_path).expect("closed task");
    let (frontmatter, _) = aplan_core::load::split_frontmatter(&closed).expect("frontmatter");
    let before: aplan_core::Task = yaml_compat::compat::from_str(frontmatter).expect("task");
    let before = before
        .evidence_fingerprints
        .get("file:DOC.md")
        .cloned()
        .expect("close stamped evidence");

    std::fs::write(dir.path().join("DOC.md"), "# Proof\n\nrewritten\n").expect("rewrite");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "rewrite proof"]);
    let (stdout, stderr, _) = run(dir.path(), &["verify"]);
    assert!(
        format!("{stdout}{stderr}").contains("APLAN054"),
        "the committed rewrite was not reported: {stdout}{stderr}"
    );
    let (shown, stderr, ok) = run(
        dir.path(),
        &["show", "A-1", "--format", "json", "--repo", "."],
    );
    assert!(ok, "show failed: {shown}{stderr}");
    assert!(
        shown.contains("APLAN054"),
        "show hid a finding verify attaches to the task: {shown}"
    );

    let (stdout, stderr, ok) = run(dir.path(), &["close", "A-1", "--restamp"]);
    assert!(ok, "restamp failed: {stdout}{stderr}");
    assert!(
        stdout.contains("committed file-evidence fingerprints: 1 changed, 0 unchanged"),
        "{stdout}"
    );
    let restamped = std::fs::read_to_string(&task_path).expect("restamped task");
    let (frontmatter, _) = aplan_core::load::split_frontmatter(&restamped).expect("frontmatter");
    let after: aplan_core::Task = yaml_compat::compat::from_str(frontmatter).expect("task");
    assert_ne!(
        after.evidence_fingerprints.get("file:DOC.md"),
        Some(&before),
        "restamp preserved the stale fingerprint"
    );

    let (stdout, stderr, _) = run(dir.path(), &["verify"]);
    assert!(
        !format!("{stdout}{stderr}").contains("APLAN054"),
        "restamp did not clear evidence drift: {stdout}{stderr}"
    );

    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!("detect_drift: false\n{config}"),
    )
    .expect("disable drift");
    let before_refusal = std::fs::read_to_string(&task_path).expect("task before refusal");
    let (stdout, stderr, ok) = run(dir.path(), &["close", "A-1", "--restamp"]);
    assert!(!ok && format!("{stdout}{stderr}").contains("detect_drift"));
    assert_eq!(
        std::fs::read_to_string(&task_path).expect("task after refusal"),
        before_refusal,
        "a disabled detector let restamp erase its existing baselines"
    );
}

/// Proof for `close.restamp_reports_baseline_changes`: a restamp distinguishes
/// the no-op, mixed and all-changed neighbours, and mutable file evidence is
/// always compared with committed content rather than a dirty worktree edit.
#[test]
fn restamp_reports_exact_changes_and_does_not_write_an_already_current_task() {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let config = std::fs::read_to_string(dir.path().join(".aplan.yml")).expect("config");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!("detect_drift: true\n{config}"),
    )
    .expect("config");
    for (name, body) in [
        ("TRACE-A.md", "# Contract A\n\ntrace a v1\n"),
        ("TRACE-B.md", "# Contract B\n\ntrace b v1\n"),
        ("EVIDENCE-A.md", "evidence a v1\n"),
        ("EVIDENCE-B.md", "evidence b v1\n"),
    ] {
        std::fs::write(dir.path().join(name), body).expect("document");
    }
    let task_path = dir.path().join("aplan/tasks/A-1.md");
    std::fs::write(
        &task_path,
        "---\nid: A-1\ntitle: a task\nphase: 1\nstatus: todo\nowner: t\n\
         effort_days: 1\npaths:\n- tests/**\n- TRACE-A.md\n- TRACE-B.md\n- EVIDENCE-A.md\n\
         - EVIDENCE-B.md\ntraces:\n- TRACE-A.md#contract-a\n- TRACE-B.md#contract-b\n\
         ---\n\nbody\n",
    )
    .expect("task");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "add reviewed inputs"]);

    let (stdout, stderr, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "file:EVIDENCE-A.md",
            "--evidence",
            "file:EVIDENCE-B.md",
        ],
    );
    assert!(ok, "close failed: {stdout}{stderr}");
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &["commit", "-q", "-m", "record initial baselines"],
    );

    // A dirty file-evidence referent is not a new baseline: HEAD still names
    // v1, so every trace and evidence fingerprint is already current.
    std::fs::write(
        dir.path().join("EVIDENCE-A.md"),
        "dirty evidence that is not committed\n",
    )
    .expect("dirty evidence");
    let before = std::fs::read_to_string(&task_path).expect("task before no-op");
    #[cfg(unix)]
    let before_inode = std::fs::metadata(&task_path).expect("metadata").ino();
    let (stdout, stderr, ok) = run(dir.path(), &["close", "A-1", "--restamp"]);
    assert!(ok, "no-op restamp failed: {stdout}{stderr}");
    assert!(
        stdout.contains("baselines already current; no task file written"),
        "{stdout}"
    );
    assert!(
        stdout.contains("trace fingerprints: 0 changed, 2 unchanged"),
        "{stdout}"
    );
    assert!(
        stdout.contains("committed file-evidence fingerprints: 0 changed, 2 unchanged"),
        "{stdout}"
    );
    assert_eq!(
        std::fs::read_to_string(&task_path).expect("task after no-op"),
        before,
        "an all-current restamp changed the task bytes"
    );
    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(&task_path).expect("metadata").ino(),
        before_inode,
        "an all-current restamp replaced the task file"
    );

    // Commit the dirty evidence change alongside one changed trace. One member
    // of each two-entry family now changes and one remains current.
    std::fs::write(
        dir.path().join("TRACE-A.md"),
        "# Contract A\n\ntrace a v2\n",
    )
    .expect("changed trace");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "change one of each"]);
    let (stdout, stderr, ok) = run(dir.path(), &["close", "A-1", "--restamp"]);
    assert!(ok, "mixed restamp failed: {stdout}{stderr}");
    assert!(
        stdout.contains("trace fingerprints: 1 changed, 1 unchanged"),
        "{stdout}"
    );
    assert!(
        stdout.contains("committed file-evidence fingerprints: 1 changed, 1 unchanged"),
        "{stdout}"
    );
    assert!(!stdout.contains("no task file written"), "{stdout}");

    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &["commit", "-q", "-m", "record mixed baselines"],
    );
    for (name, body) in [
        ("TRACE-A.md", "# Contract A\n\ntrace a v3\n"),
        ("TRACE-B.md", "# Contract B\n\ntrace b v2\n"),
        ("EVIDENCE-A.md", "evidence a v3\n"),
        ("EVIDENCE-B.md", "evidence b v2\n"),
    ] {
        std::fs::write(dir.path().join(name), body).expect("changed document");
    }
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "change every baseline"]);
    let (stdout, stderr, ok) = run(dir.path(), &["close", "A-1", "--restamp"]);
    assert!(ok, "all-changed restamp failed: {stdout}{stderr}");
    assert!(
        stdout.contains("trace fingerprints: 2 changed, 0 unchanged"),
        "{stdout}"
    );
    assert!(
        stdout.contains("committed file-evidence fingerprints: 2 changed, 0 unchanged"),
        "{stdout}"
    );
    let (stdout, stderr, ok) = run(dir.path(), &["verify"]);
    assert!(ok, "restamped baselines did not verify: {stdout}{stderr}");
}

#[test]
fn restamp_with_no_stampable_subject_does_not_invent_a_current_baseline() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let config = std::fs::read_to_string(dir.path().join(".aplan.yml")).expect("config");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!("detect_drift: true\n{config}"),
    )
    .expect("config");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "enable drift"]);
    let (stdout, stderr, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:a_settledone_that_overtakes_the_reply_is_not_lost",
        ],
    );
    assert!(ok, "close failed: {stdout}{stderr}");
    let task_path = dir.path().join("aplan/tasks/A-1.md");
    let before = std::fs::read_to_string(&task_path).expect("closed task");

    let (stdout, stderr, ok) = run(dir.path(), &["close", "A-1", "--restamp"]);
    assert!(ok, "empty restamp failed: {stdout}{stderr}");
    assert!(
        stdout.contains("no stampable fingerprint baselines; no task file written"),
        "{stdout}"
    );
    assert!(
        stdout.contains("trace fingerprints: 0 changed, 0 unchanged"),
        "{stdout}"
    );
    assert!(
        stdout.contains("committed file-evidence fingerprints: 0 changed, 0 unchanged"),
        "{stdout}"
    );
    assert!(!stdout.contains("already current"), "{stdout}");
    assert_eq!(
        std::fs::read_to_string(task_path).expect("task after restamp"),
        before
    );
}

#[test]
fn restamp_counts_and_removes_orphaned_baselines_once() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let config = std::fs::read_to_string(dir.path().join(".aplan.yml")).expect("config");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!("detect_drift: true\n{config}"),
    )
    .expect("config");
    let task_path = dir.path().join("aplan/tasks/A-1.md");
    std::fs::write(
        &task_path,
        "---\nid: A-1\ntitle: a task\nphase: 1\nstatus: done\nowner: t\n\
         effort_days: 1\npaths:\n- tests/**\nevidence:\n- kind: test\n  \
         ref: a_settledone_that_overtakes_the_reply_is_not_lost\n\
         trace_fingerprints:\n  DOC.md#contract: orphaned-trace\n\
         evidence_fingerprints:\n  file:EVIDENCE.md: orphaned-evidence\n---\n\nbody\n",
    )
    .expect("task with orphaned baselines");
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &["commit", "-q", "-m", "add orphaned baselines"],
    );

    let (stdout, stderr, ok) = run(dir.path(), &["close", "A-1", "--restamp"]);
    assert!(ok, "restamp failed: {stdout}{stderr}");
    assert_eq!(
        stdout,
        "restamped A-1: trace fingerprints: 1 changed, 0 unchanged; committed \
         file-evidence fingerprints: 1 changed, 0 unchanged\n"
    );
    let updated = std::fs::read_to_string(task_path).expect("restamped task");
    assert!(!updated.contains("trace_fingerprints:"), "{updated}");
    assert!(!updated.contains("evidence_fingerprints:"), "{updated}");
}

/// Proves the named behaviour `close.evidence_resolves_before_write` (`A1-211`).
#[test]
fn no_resolve_records_it_and_says_that_it_did() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (stdout, _, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:guider::a_settledone_that_overtakes_the_reply_is_not_lost",
            "--no-resolve",
        ],
    );

    assert!(ok, "the escape hatch must work: {stdout}");
    assert_eq!(status_of(dir.path()), "status: done");
    // A closure that skipped its checks must not read like one that passed.
    assert!(
        stdout.contains("--no-resolve") && stdout.contains("without being resolved"),
        "the escape hatch must say what it skipped: {stdout}"
    );
}

/// A dead sibling glob is reported by `APLAN019` even when another sibling
/// binds the task's citation (`A2-091`, superseding `A1-141`).
///
/// Under `deny_warnings: true`, `close` refuses what `verify` would fail so
/// dead `paths` entries cannot slip through behind a live sibling.
/// Proves the named behaviour `paths.unmatched_reports_every_dead_entry` (`A2-091`).
#[test]
fn a_dead_sibling_glob_is_reported_even_beside_a_bound_citation() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // `deny_warnings: true` turns the `APLAN019` warning into a refusal.
    let config = std::fs::read_to_string(dir.path().join(".aplan.yml")).expect("read");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!("deny_warnings: true\n{config}"),
    )
    .expect("config");
    // `tests/**` is live; `fixtures/**` is dead.
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: a task\nphase: 1\nstatus: todo\nowner: t\n\
         effort_days: 1\npaths:\n- tests/**\n- fixtures/**\n---\n\nbody\n",
    )
    .expect("task");

    let (out, err, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", "file:tests/t.rs"],
    );
    let said = format!("{out}{err}");
    assert!(
        !ok,
        "a dead sibling glob must be reported and refuse close under deny_warnings: {said}"
    );
    assert!(said.contains("APLAN019"), "{said}");
    assert!(said.contains("fixtures/**"), "{said}");
    assert_eq!(status_of(dir.path()), "status: todo");

    // Once the dead sibling is removed, the same close succeeds.
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: a task\nphase: 1\nstatus: todo\nowner: t\n\
         effort_days: 1\npaths:\n- tests/**\n---\n\nbody\n",
    )
    .expect("task");
    let (out, err, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", "file:tests/t.rs"],
    );
    assert!(ok, "close without dead sibling should pass: {out}{err}");
    assert_eq!(status_of(dir.path()), "status: done");
}

/// `A2-101` (`ADR-0071`): `aplan retarget-path` atomically rewrites matching
/// live tasks while preserving terminal tasks untouched and refusing explicit
/// terminal targets.
#[test]
#[allow(clippy::too_many_lines)]
fn retarget_path_updates_live_tasks_atomically_and_preserves_terminal_tasks() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // Create `src/old.rs`, commit it, close `A-DONE` against it, then rename to `src/new.rs`.
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/old.rs"), "pub fn work() {}\n").unwrap();
    git(dir.path(), &["add", "src/old.rs"]);
    git(dir.path(), &["commit", "-m", "add src/old.rs"]);
    let sha = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let sha = sha.trim();

    std::fs::write(
        dir.path().join("aplan/tasks/A-DONE.md"),
        format!(
            "---\nid: A-DONE\ntitle: done task\nphase: 1\nstatus: done\nowner: t\n\
             effort_days: 1\npaths:\n- src/old.rs\nevidence:\n- kind: commit\n  ref: {sha}\n---\n\nbody\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("aplan/tasks/A-LIVE1.md"),
        "---\nid: A-LIVE1\ntitle: live one\nphase: 1\nstatus: todo\nowner: t\n\
         effort_days: 1\npaths:\n- src/old.rs\n---\n\nbody\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("aplan/tasks/A-LIVE2.md"),
        "---\nid: A-LIVE2\ntitle: live two\nphase: 1\nstatus: todo\nowner: t\n\
         effort_days: 1\npaths:\n- src/old.rs\n---\n\nbody\n",
    )
    .unwrap();

    std::fs::rename(dir.path().join("src/old.rs"), dir.path().join("src/new.rs")).unwrap();
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &["commit", "-m", "rename src/old.rs to src/new.rs"],
    );

    // 1. Explicit `--id A-DONE` must be refused.
    let (out, err, ok) = run(
        dir.path(),
        &[
            "retarget-path",
            "src/old.rs",
            "src/new.rs",
            "--id",
            "A-DONE",
        ],
    );
    assert!(!ok, "terminal task retarget must be refused: {out}{err}");
    assert!(format!("{out}{err}").contains("terminal"), "{out}{err}");

    // 2. `--dry-run` previews live tasks and leaves disk untouched.
    let (out, err, ok) = run(
        dir.path(),
        &["retarget-path", "src/old.rs", "src/new.rs", "--dry-run"],
    );
    assert!(ok, "dry-run failed: {out}{err}");
    assert!(out.contains("A-LIVE1") && out.contains("A-LIVE2"), "{out}");
    assert!(out.contains("preserved 1 terminal"), "{out}");
    assert!(
        std::fs::read_to_string(dir.path().join("aplan/tasks/A-LIVE1.md"))
            .unwrap()
            .contains("src/old.rs")
    );

    // 3. Targeted `--id A-LIVE1` updates only `A-LIVE1`.
    let (out, err, ok) = run(
        dir.path(),
        &[
            "retarget-path",
            "src/old.rs",
            "src/new.rs",
            "--id",
            "A-LIVE1",
        ],
    );
    assert!(ok, "targeted retarget failed: {out}{err}");
    assert!(
        std::fs::read_to_string(dir.path().join("aplan/tasks/A-LIVE1.md"))
            .unwrap()
            .contains("src/new.rs")
    );
    assert!(
        std::fs::read_to_string(dir.path().join("aplan/tasks/A-LIVE2.md"))
            .unwrap()
            .contains("src/old.rs")
    );

    // 4. Whole-plan retarget updates remaining `A-LIVE2` and preserves `A-DONE`.
    let (out, err, ok) = run(dir.path(), &["retarget-path", "src/old.rs", "src/new.rs"]);
    assert!(ok, "whole-plan retarget failed: {out}{err}");
    assert!(
        std::fs::read_to_string(dir.path().join("aplan/tasks/A-LIVE2.md"))
            .unwrap()
            .contains("src/new.rs")
    );
    assert!(
        std::fs::read_to_string(dir.path().join("aplan/tasks/A-DONE.md"))
            .unwrap()
            .contains("src/old.rs"),
        "terminal task must keep its historical path"
    );

    // 5. Re-running when only `A-DONE` still holds `src/old.rs` must refuse.
    let (out, err, ok) = run(dir.path(), &["retarget-path", "src/old.rs", "src/new.rs"]);
    assert!(!ok, "only-terminal retarget must be refused: {out}{err}");
    assert!(format!("{out}{err}").contains("terminal"), "{out}{err}");

    // 6. Post-state gate: retargeting to a universal catch-all (`**`) is refused
    // under `--deny-warnings` (APLAN061), and allowed with `--no-deny-warnings`.
    let (out, err, ok) = run(
        dir.path(),
        &[
            "retarget-path",
            "src/new.rs",
            "**",
            "--id",
            "A-LIVE1",
            "--deny-warnings",
        ],
    );
    assert!(
        !ok,
        "retargeting to `**` under --deny-warnings must be refused: {out}{err}"
    );
    assert!(format!("{out}{err}").contains("APLAN061"), "{out}{err}");
    let (out, err, ok) = run(
        dir.path(),
        &[
            "retarget-path",
            "src/new.rs",
            "**",
            "--id",
            "A-LIVE1",
            "--dry-run",
            "--no-deny-warnings",
        ],
    );
    assert!(
        ok,
        "dry-run with --no-deny-warnings should succeed: {out}{err}"
    );
}

/// `A2-156`: `aplan check --range` validates every commit introduced by a
/// commit range without mutating the working tree, catching an intermediate
/// `APLAN018` commit behind a green restamp tip, supporting `--fail-fast`,
/// reusing proved-identical commits, and handling empty, root-inclusive,
/// unrelated, and invalid ranges explicitly.
///
/// Proves the named behaviour `check.range_validates_every_introduced_commit` (`A2-156`).
#[test]
#[allow(clippy::too_many_lines)]
fn check_range_catches_intermediate_aplan018_behind_green_tip_and_handles_edge_cases() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // Configure `deny_warnings: true` and `detect_drift: true`, plus a traced
    // doc and a closed stamped task.
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndeny_warnings: true\ndetect_drift: true\n",
    )
    .unwrap();
    let doc_v1 = "## Policy\n\nOriginal policy text.\n";
    std::fs::write(dir.path().join("SPEC.md"), doc_v1).unwrap();
    let stamp_v1 = aplan_core::stamp_in("SPEC.md", doc_v1, "policy").unwrap();
    let base_sha_init = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let base_sha_init = base_sha_init.trim().to_string();
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        format!(
            "---\nid: A-1\ntitle: stamped task\nphase: 1\nstatus: done\nowner: t\n\
             effort_days: 1\npaths:\n- tests/**\ntraces:\n- SPEC.md#policy\n\
             trace_fingerprints:\n  SPEC.md#policy: '{stamp_v1}'\n\
             evidence:\n- kind: commit\n  ref: {base_sha_init}\n---\n\nbody\n"
        ),
    )
    .unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-m", "base clean state"]);
    let base_sha = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let base_sha = base_sha.trim().to_string();

    // Commit 1 (intermediate, RED): edit SPEC.md#policy without restamping A-1.
    let doc_v2 = "## Policy\n\nChanged policy text without restamp.\n";
    std::fs::write(dir.path().join("SPEC.md"), doc_v2).unwrap();
    git(dir.path(), &["add", "SPEC.md"]);
    git(dir.path(), &["commit", "-m", "edit spec without restamp"]);

    // Commit 2 (tip, GREEN): restamp A-1 so tip passes ordinary `check`.
    let stamp_v2 = aplan_core::stamp_in("SPEC.md", doc_v2, "policy").unwrap();
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        format!(
            "---\nid: A-1\ntitle: stamped task\nphase: 1\nstatus: done\nowner: t\n\
             effort_days: 1\npaths:\n- tests/**\ntraces:\n- SPEC.md#policy\n\
             trace_fingerprints:\n  SPEC.md#policy: '{stamp_v2}'\n\
             evidence:\n- kind: commit\n  ref: {base_sha_init}\n---\n\nbody\n"
        ),
    )
    .unwrap();
    git(dir.path(), &["add", "aplan/tasks/A-1.md"]);
    git(dir.path(), &["commit", "-m", "restamp A-1"]);

    // Commit 3 (clean no-op code commit): touches unrelated file -> reuses Commit 2.
    std::fs::write(dir.path().join("tests/other.rs"), "// unrelated\n").unwrap();
    git(dir.path(), &["add", "tests/other.rs"]);
    git(dir.path(), &["commit", "-m", "unrelated test edit"]);

    // Ordinary tip-only `check` passes cleanly.
    let (out, err, ok) = run(dir.path(), &["check"]);
    assert!(ok, "ordinary tip-only check must stay green: {out}{err}");
    assert!(out.contains("ok:"), "{out}");

    // Range check catches intermediate APLAN018 behind the green tip (JSON).
    let range_spec = format!("{base_sha}..HEAD");
    let (out, err, ok) = run(
        dir.path(),
        &[
            "check",
            "--range",
            &range_spec,
            "--repo",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
        ],
    );
    assert!(
        !ok,
        "range check must fail on intermediate APLAN018: {out}{err}"
    );
    let json: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert_eq!(json["schema"], "aplan/range-check/v1");
    assert_eq!(json["range_clean"], false);
    assert_eq!(json["tip_clean"], true);
    assert_eq!(json["commits_total"], 3);
    assert_eq!(json["commits_audited"], 3);
    assert_eq!(json["commits_failed"], 1);
    assert_eq!(json["commits"][0]["clean"], false);
    assert_eq!(json["commits"][0]["diagnostics"][0]["code"], "APLAN018");
    assert_eq!(json["commits"][1]["clean"], true);
    assert_eq!(json["commits"][2]["clean"], true);
    assert!(
        json["commits"][2]["reused_from"].is_string(),
        "unrelated commit should reuse proved-identical result: {json:#}"
    );

    // `--fail-fast` stops at commit 1 (commits_audited == 1) while still reporting tip_clean.
    let (out, err, ok) = run(
        dir.path(),
        &[
            "check",
            "--range",
            &range_spec,
            "--fail-fast",
            "--format",
            "json",
        ],
    );
    assert!(!ok, "fail-fast range must fail: {out}{err}");
    let ff: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert_eq!(ff["stopped_early"], true);
    assert_eq!(ff["commits_audited"], 1);
    assert_eq!(ff["tip_clean"], true);

    // Clean multi-commit range (`HEAD~2..HEAD`: restamp + unrelated edit) passes.
    let (out, err, ok) = run(dir.path(), &["check", "--range", "HEAD~2..HEAD"]);
    assert!(ok, "clean range must pass: {out}{err}");
    assert!(out.contains("ok: range `HEAD~2..HEAD` clean"), "{out}");

    // Equal endpoints (`HEAD..HEAD`) -> empty range, ok.
    let (out, err, ok) = run(dir.path(), &["check", "--range", "HEAD..HEAD"]);
    assert!(ok, "empty range must succeed explicitly: {out}{err}");
    assert!(out.contains("empty"), "{out}");

    // Reversed range (`HEAD..{base_sha}`) -> explicit error refusal.
    let rev_spec = format!("HEAD..{base_sha}");
    let (out, err, ok) = run(dir.path(), &["check", "--range", &rev_spec]);
    assert!(!ok, "reversed range must be refused: {out}{err}");
    assert!(
        format!("{out}{err}").contains("not an ancestor"),
        "{out}{err}"
    );

    // Invalid revision -> explicit error refusal.
    let (out, err, ok) = run(dir.path(), &["check", "--range", "nope1234..HEAD"]);
    assert!(!ok, "invalid rev must be refused: {out}{err}");
}

/// The same refusal for the other kind that names an artefact (`A1-045`).
///
/// The fourth pilot closed a task about ADR authorship with a Containerfile, in
/// one command, and `check` and `verify` said `ok` afterwards. The commit
/// citation for the same work was refused by `APLAN012`; the file citation was
/// not, and its conventions prefer `file:` precisely because a large commit
/// names a Tuesday (pre-public adoption evidence item 25).
#[test]
fn a_file_outside_the_tasks_paths_is_refused_at_the_same_moment() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // The fixture's task owns `tests/**`; this file exists and is not its.
    std::fs::write(dir.path().join("elsewhere.md"), "hi\n").expect("write");
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &["commit", "-q", "-m", "a file the task does not own"],
    );

    let (out, err, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", "file:elsewhere.md"],
    );
    let said = format!("{out}{err}");
    assert!(!ok, "the pilot's closure was accepted: {said}");
    assert!(said.contains("APLAN031"), "{said}");
    assert!(said.contains("outside this task's"), "{said}");
    assert_eq!(status_of(dir.path()), "status: todo", "it wrote anyway");

    // And the citation the task does own closes it, so the refusal is about
    // relevance rather than about `file:` evidence.
    let (out, err, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", "file:tests/t.rs"],
    );
    assert!(ok, "{out}{err}");
    assert_eq!(status_of(dir.path()), "status: done");
}

#[test]
fn a_commit_that_touches_none_of_the_tasks_paths_is_refused() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(dir.path().join("NOTES.md"), "unrelated\n").expect("notes");
    git(dir.path(), &["add", "NOTES.md"]);
    git(dir.path(), &["commit", "-q", "-m", "unrelated work"]);
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git");
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();

    let (stdout, _, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", &format!("commit:{sha}")],
    );

    // `APLAN012` is the rung that made `paths` load-bearing; a close that ignored
    // it would certify a commit about something else.
    assert!(!ok, "an irrelevant commit must be refused: {stdout}");
    assert!(stdout.contains("APLAN012"), "{stdout}");
    assert_eq!(status_of(dir.path()), "status: todo");
}

#[test]
fn without_a_runner_a_test_citation_is_recorded_as_unchecked_not_as_passed() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // Take the runner away; `test` evidence now has no L1 at all.
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");

    let (stdout, _, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", "test:nothing_lists_this"],
    );

    assert!(
        ok,
        "a repository with no runner must still be able to close: {stdout}"
    );
    assert_eq!(status_of(dir.path()), "status: done");
    // The criterion this task turns on: silence about a check nothing ran reads
    // exactly like a check that passed, which is the defect one level up.
    assert!(
        stdout.contains("unresolved") && stdout.contains("test"),
        "it must say the citation was not checked: {stdout}"
    );
}

#[test]
fn an_already_closed_task_is_told_that_and_not_something_true_but_irrelevant() {
    // Found auditing `A1-109`. The gate ran before the structural refusals, so
    // closing an already-`done` task spawned the runner and answered "this
    // evidence would fail `aplan verify`" -- true, and not the reason. The reason
    // is that the task is closed, and a tool that reports a real problem in place
    // of the actual one sends the reader to fix the wrong thing.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let (_, _, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:a_settledone_that_overtakes_the_reply_is_not_lost",
        ],
    );
    assert!(ok, "the first close must succeed");

    let (stdout, stderr, ok) = run(dir.path(), &["close", "A-1", "--evidence", "test:ghost"]);
    let out = format!("{stdout}{stderr}");
    assert!(!ok, "closing a closed task must fail: {out}");
    assert!(
        out.contains("already `done`"),
        "it must give the structural reason: {out}"
    );
    assert!(
        !out.contains("APLAN013"),
        "and must not report the citation instead, which is true but not why: {out}"
    );
}

#[test]
fn a_refusal_does_not_claim_the_evidence_was_recorded() {
    // Also found auditing `A1-109`. The unchecked-kinds note printed before the
    // refusal, so a refused close said `url` evidence "was recorded unresolved"
    // while recording nothing. `A1-091`: a message may name only what happened.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (stdout, _, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:ghost",
            "--evidence",
            "url:https://example.com",
        ],
    );
    assert!(!ok, "the close must be refused: {stdout}");
    assert!(
        !stdout.contains("was recorded"),
        "a refusal must not claim anything was recorded: {stdout}"
    );
    assert_eq!(status_of(dir.path()), "status: todo");

    // The note is still owed when the closure goes ahead, or the fix would have
    // traded a false statement for a missing one.
    let (stdout, _, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:a_settledone_that_overtakes_the_reply_is_not_lost",
            "--evidence",
            "url:https://example.com",
        ],
    );
    assert!(ok, "{stdout}");
    assert!(
        stdout.contains("url evidence was recorded unresolved"),
        "a successful close must still name what nothing checked: {stdout}"
    );
}

#[test]
fn deny_warnings_decides_whether_a_ladder_warning_refuses() {
    // The design decision this task turns on: `close` refuses exactly what would
    // fail the build, by reusing `Report::denied`, rather than inventing a second
    // severity policy for closing. Proved both ways, because a gate that always
    // refused and a gate that never did would each pass one half of this.
    //
    // `APLAN025` is the warning used because it is on the `verify` ladder.
    // Hermetic warnings like `APLAN023` are deliberately *not* checked here: the
    // rule engine over a mutation's post-state is `A1-113`'s, and this task owns
    // the evidence ladder. Stated so the boundary is a decision rather than a gap
    // nobody noticed.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::create_dir_all(dir.path().join("other")).expect("mkdir");
    std::fs::write(dir.path().join("other/o.rs"), "fn main() {}\n").expect("other");
    // Written line by line: a wrapped string literal here once produced fifteen
    // spaces of indent for two keys and two for a third, which is invalid YAML,
    // and the test failed with an empty stdout rather than a wrong verdict.
    let config = |deny: bool| {
        [
            "root: aplan".to_string(),
            format!("deny_warnings: {deny}"),
            "test:".to_string(),
            "  list: [\"printf\", \"a_settledone_that_overtakes_the_reply_is_not_lost\\n\"]"
                .to_string(),
            "  run: [\"true\", \"{test}\"]".to_string(),
            "  covers: [\"other/**\"]".to_string(),
        ]
        .join("\n")
    };
    let cite = [
        "close",
        "A-1",
        "--evidence",
        "test:a_settledone_that_overtakes_the_reply_is_not_lost",
    ];

    std::fs::write(dir.path().join(".aplan.yml"), config(true)).expect("config");
    let (stdout, _, ok) = run(dir.path(), &cite);
    assert!(!ok, "with deny_warnings, a warning must refuse: {stdout}");
    assert!(stdout.contains("APLAN025"), "{stdout}");
    assert_eq!(status_of(dir.path()), "status: todo");

    std::fs::write(dir.path().join(".aplan.yml"), config(false)).expect("config");
    let (stdout, _, ok) = run(dir.path(), &cite);
    assert!(ok, "without it, the same warning must not refuse: {stdout}");
    assert!(
        stdout.contains("APLAN025"),
        "it must still be reported, not hidden: {stdout}"
    );
    assert_eq!(status_of(dir.path()), "status: done");
}

#[test]
fn a_plan_outside_a_repository_still_closes_and_says_what_was_skipped() {
    // `close` was hermetic before this task, and a plan kept outside git is a
    // real arrangement. Demanding a repository to close would be a regression
    // dressed as a gate.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: a task\nphase: 1\nstatus: todo\nowner: t\n\
         effort_days: 1\n---\n\nbody\n",
    )
    .expect("task");

    let (stdout, _, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", "commit:0123456789abcdef"],
    );

    assert!(ok, "no repository must not mean no closing: {stdout}");
    assert!(
        stdout.contains("unresolved"),
        "it must say the citation went unchecked: {stdout}"
    );
}

/// Proof for `close.focused_l3_is_explicit_scoped_and_stale_safe`.
///
/// The ordinary close first proves that the existing command remains L2-only.
/// The focused close then cites one identity twice and another runner once: the
/// log must contain each distinct selected identity exactly once, and must not
/// contain the historical sentinel even though it is cited by the same plan.
#[test]
fn focused_l3_runs_only_the_proposed_closures_tests() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("repository");
    let support = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("provider support");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(dir.path().join("tests")).expect("tests");
    std::fs::write(dir.path().join("tests/proof.txt"), "proof input\n").expect("proof");
    let log = support.path().join("executed.log");
    let primary_list = support.path().join("primary-list.sh");
    let secondary_list = support.path().join("secondary-list.sh");
    let unrelated_list = support.path().join("unrelated-list.sh");
    let runner = support.path().join("run.sh");
    std::fs::write(
        &primary_list,
        "#!/bin/sh\nprintf '%s\\n' ordinary::listed focus::primary focus::stale sentinel::historical\n",
    )
    .expect("primary list");
    std::fs::write(
        &secondary_list,
        "#!/bin/sh\nprintf '%s\\n' focus::secondary\n",
    )
    .expect("secondary list");
    std::fs::write(
        &unrelated_list,
        "#!/bin/sh\nprintf '%s\\n' unrelated::test\n",
    )
    .expect("unrelated list");
    std::fs::write(
        &runner,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$1\" >> '{}'\nif [ \"$1\" = focus::stale ]; then printf 'changed\\n' >> '{}'; fi\n[ \"$1\" != sentinel::historical ]\n",
            log.display(),
            dir.path().join("tests/proof.txt").display()
        ),
    )
    .expect("runner");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!(
            "root: aplan\nevidence:\n  require_min_strength: [test]\ntest:\n  - name: primary\n    \
             list: [sh, {}]\n    run: [sh, {}, '{{test}}']\n    run_selects: true\n    \
             covers: ['tests/**']\n  - name: secondary\n    list: [sh, {}]\n    run: [sh, \
             {}, '{{test}}']\n    run_selects: true\n    covers: ['tests/**']\n  - name: unrelated\n    \
             list: [sh, {}]\n    run: [/definitely/missing/unrelated-runner, '{{test}}']\n    \
             run_selects: true\n    covers: ['tests/**']\n",
            primary_list.display(),
            runner.display(),
            secondary_list.display(),
            runner.display(),
            unrelated_list.display()
        ),
    )
    .expect("config");
    let task = |id: &str, title: &str, status: &str, evidence: &str| {
        format!(
            "---\nid: {id}\ntitle: {title}\nphase: 1\nstatus: {status}\neffort_days: 1\n\
             paths: ['tests/**']\n{evidence}---\n\n## Acceptance criteria\n\n- [x] proved\n"
        )
    };
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        task("A-1", "focused", "todo", ""),
    )
    .expect("focused task");
    std::fs::write(
        dir.path().join("aplan/tasks/A-2.md"),
        task("A-2", "ordinary", "todo", ""),
    )
    .expect("ordinary task");
    std::fs::write(
        dir.path().join("aplan/tasks/A-3.md"),
        task(
            "A-3",
            "historical sentinel",
            "done",
            "evidence:\n  - kind: test\n    ref: sentinel::historical\n",
        ),
    )
    .expect("historical task");
    std::fs::write(
        dir.path().join("aplan/tasks/A-4.md"),
        task("A-4", "stale focused result", "todo", ""),
    )
    .expect("stale task");
    git(dir.path(), &["init", "-q", "."]);
    git(dir.path(), &["config", "user.email", "t@example.com"]);
    git(dir.path(), &["config", "user.name", "t"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "focused fixture"]);

    let (stdout, stderr, ok) = run(
        dir.path(),
        &["close", "A-2", "--evidence", "test:ordinary::listed"],
    );
    assert!(ok, "ordinary L2 close failed: {stdout}{stderr}");
    assert!(
        !log.exists(),
        "ordinary close crossed the execution boundary: {}",
        std::fs::read_to_string(&log).unwrap_or_default()
    );

    let (stdout, stderr, ok) = run(dir.path(), &["close", "A-1", "--execute"]);
    assert!(
        !ok,
        "focused close invented its own test scope: {stdout}{stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("requires at least one explicit"),
        "missing focused scope was not explained: {stdout}{stderr}"
    );
    assert!(
        !log.exists(),
        "scope validation started a configured runner"
    );

    let (stdout, stderr, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:focus::primary",
            "--execute",
            "--no-resolve",
        ],
    );
    assert!(
        !ok,
        "focused unresolved mode was accepted: {stdout}{stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("cannot be used with"),
        "the conflicting trust modes were not named: {stdout}{stderr}"
    );
    assert!(!log.exists(), "clap conflict started a configured runner");

    // `A2-106`: duplicate `--evidence` citations are refused before executing
    // any runner or writing the task.
    let (stdout, stderr, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:focus::primary",
            "--evidence",
            "test:focus::primary",
            "--evidence",
            "test:focus::secondary",
            "--execute",
        ],
    );
    assert!(
        !ok,
        "duplicate evidence citation must be refused by close (A2-106): {stdout}{stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("test:focus::primary"),
        "duplicate refusal must name kind and reference: {stdout}{stderr}"
    );
    assert!(
        !log.exists(),
        "duplicate refusal must happen before running tests"
    );

    let (stdout, stderr, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:focus::primary",
            "--evidence",
            "test:focus::secondary",
            "--execute",
        ],
    );
    assert!(ok, "focused L3 close failed: {stdout}{stderr}");
    assert!(
        stdout.contains("focused L3 executed and proved"),
        "{stdout}"
    );
    let mut executed = std::fs::read_to_string(&log)
        .expect("focused execution log")
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    executed.sort();
    assert_eq!(
        executed,
        ["focus::primary".to_string(), "focus::secondary".to_string()],
        "execution widened beyond distinct proposed citations"
    );
    assert_eq!(status_of(dir.path()), "status: done");

    let stale_task = dir.path().join("aplan/tasks/A-4.md");
    let before = std::fs::read(&stale_task).expect("stale task before execution");
    let (stdout, stderr, ok) = run(
        dir.path(),
        &[
            "close",
            "A-4",
            "--evidence",
            "test:focus::stale",
            "--execute",
        ],
    );
    assert!(!ok, "source movement published old proof: {stdout}{stderr}");
    assert!(
        stdout.contains("stale")
            && stdout.contains("nothing was published by `aplan`")
            && stdout.contains("configured test command are not rolled back"),
        "the stale L3 result was not distinguished: {stdout}{stderr}"
    );
    assert_eq!(
        std::fs::read(&stale_task).expect("stale task after refusal"),
        before,
        "stale focused proof changed task bytes"
    );
}

#[test]
fn focused_l3_reproduces_both_amorph_red_closures_as_atomic_refusals() {
    for (incident, stale_count, current_count) in [("AM-384", 402, 403), ("AM-364", 376, 380)] {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("repository");
        let support = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("provider support");
        std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
        std::fs::create_dir_all(dir.path().join("tests")).expect("tests");
        std::fs::write(
            dir.path().join("tests/count.txt"),
            format!("{current_count}\n"),
        )
        .expect("current count");
        let list = support.path().join("list.sh");
        let runner_script = support.path().join("run.sh");
        std::fs::write(
            &list,
            "#!/bin/sh\nprintf '%s\\n' checks::verify::28-the-record-counts-what-is-there\n",
        )
        .expect("list");
        std::fs::write(
            &runner_script,
            format!(
                "#!/bin/sh\n# {incident}: the recorded count was {stale_count}, current was {current_count}.\nexit 1\n"
            ),
        )
        .expect("red check");
        std::fs::write(
            dir.path().join(".aplan.yml"),
            format!(
                "root: aplan\nevidence:\n  require_min_strength: [test]\ntest:\n  name: p00002\n  list: \
                 [sh, {}]\n  run: [sh, {}, '{{test}}']\n  run_selects: true\n  covers: ['tests/**']\n",
                list.display(),
                runner_script.display()
            ),
        )
        .expect("config");
        let task_path = dir.path().join("aplan/tasks/A-1.md");
        std::fs::write(
            &task_path,
            format!(
                "---\nid: A-1\ntitle: reproduce {incident}\nphase: 1\nstatus: todo\neffort_days: 1\n\
                 paths: ['tests/**']\n---\n\n## Acceptance criteria\n\n- [x] count agrees\n"
            ),
        )
        .expect("task");
        git(dir.path(), &["init", "-q", "."]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "t"]);
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", "red p00002 fixture"]);
        let before = std::fs::read(&task_path).expect("task before");

        let (stdout, stderr, ok) = run(
            dir.path(),
            &[
                "close",
                "A-1",
                "--evidence",
                "test:checks::verify::28-the-record-counts-what-is-there",
                "--execute",
            ],
        );
        assert!(!ok, "{incident} red test closed: {stdout}{stderr}");
        assert!(
            format!("{stdout}{stderr}").contains("L3"),
            "{incident} did not distinguish execution from listing: {stdout}{stderr}"
        );
        assert_eq!(
            std::fs::read(&task_path).expect("task after refusal"),
            before,
            "{incident} refusal changed task bytes"
        );
    }
}

#[test]
fn focused_l3_refusal_does_not_overwrite_or_disguise_runner_effects() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("repository");
    let support = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("provider support");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
    let task_path = dir.path().join("aplan/tasks/A-1.md");
    std::fs::write(
        &task_path,
        "---\nid: A-1\ntitle: runner effects\nphase: 1\nstatus: todo\neffort_days: 1\npaths: ['tests/**']\n---\n\nbody\n",
    )
    .expect("task");
    let list = support.path().join("list.sh");
    let runner = support.path().join("run.sh");
    std::fs::write(&list, "#!/bin/sh\nprintf '%s\\n' suite::red\n").expect("list");
    std::fs::write(
        &runner,
        "#!/bin/sh\nprintf '\\nrunner-authored effect\\n' >> \"$1\"\nexit 1\n",
    )
    .expect("runner");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!(
            "root: aplan\ntest:\n  name: mutating\n  list: [sh, {}]\n  run: [sh, {}, {}, '{{test}}']\n  run_selects: true\n",
            list.display(),
            runner.display(),
            task_path.display()
        ),
    )
    .expect("config");
    git(dir.path(), &["init", "-q", "."]);
    git(dir.path(), &["config", "user.email", "t@example.com"]);
    git(dir.path(), &["config", "user.name", "t"]);
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &["commit", "-q", "-m", "mutating runner fixture"],
    );

    let (stdout, stderr, ok) = run(
        dir.path(),
        &["close", "A-1", "--evidence", "test:suite::red", "--execute"],
    );
    assert!(!ok, "a red runner closed the task: {stdout}{stderr}");
    assert!(
        stdout.contains("`aplan` published no plan bytes")
            && stdout.contains("configured test command are not rolled back"),
        "the refusal disguised the trusted command's effects: {stdout}{stderr}"
    );
    let after = std::fs::read_to_string(&task_path).expect("task after runner");
    assert!(
        after.ends_with("runner-authored effect\n"),
        "aplan overwrote the runner-authored change: {after}"
    );
    assert!(
        after.contains("status: todo"),
        "the refused close published: {after}"
    );
    assert!(
        !after.contains("evidence:"),
        "the refused close added evidence: {after}"
    );
}

/// Proof for `close.focused_l3_terminal_outcomes_name_external_effects`.
#[test]
fn focused_l3_preview_error_still_names_the_runner_effect_boundary() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("repository");
    let support = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("provider support");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(dir.path().join("tests")).expect("tests");
    std::fs::write(dir.path().join("tests/proof.txt"), "proof input\n").expect("proof");
    let task_path = dir.path().join("aplan/tasks/A-1.md");
    std::fs::write(
        &task_path,
        "---\nid: A-1\ntitle: moving provider\nphase: 1\nstatus: todo\neffort_days: 1\npaths: ['tests/**']\n---\n\nbody\n",
    )
    .expect("task");
    let list = support.path().join("list.sh");
    let runner = support.path().join("run.sh");
    std::fs::write(&list, "#!/bin/sh\nprintf '%s\\n' suite::green\n").expect("list");
    std::fs::write(
        &runner,
        "#!/bin/sh\nprintf '\\n# runner-authored effect\\n' >> \"$0\"\nexit 0\n",
    )
    .expect("runner");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!(
            "root: aplan\ntest:\n  name: moving\n  list: [sh, {}]\n  run: [sh, {}, '{{test}}']\n  run_selects: true\n  covers: ['tests/**']\n",
            list.display(),
            runner.display()
        ),
    )
    .expect("config");
    git(dir.path(), &["init", "-q", "."]);
    git(dir.path(), &["config", "user.email", "t@example.com"]);
    git(dir.path(), &["config", "user.name", "t"]);
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &["commit", "-q", "-m", "moving provider fixture"],
    );
    let before = std::fs::read(&task_path).expect("task before");

    let (stdout, stderr, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:suite::green",
            "--execute",
        ],
    );
    assert!(
        !ok,
        "changed provider published old proof: {stdout}{stderr}"
    );
    let output = format!("{stdout}{stderr}");
    assert!(
        output.contains("`aplan` published no plan bytes")
            && output.contains("configured test command")
            && output.contains("not rolled back"),
        "the preview error hid the runner-effect boundary: {output}"
    );
    assert_eq!(
        std::fs::read(&task_path).expect("task after"),
        before,
        "a stale provider result changed the task"
    );
    assert!(
        std::fs::read_to_string(&runner)
            .expect("runner after")
            .contains("runner-authored effect"),
        "the fixture did not cross the external-effect boundary"
    );
}

#[cfg(unix)]
#[test]
fn focused_l3_publication_error_still_names_the_runner_effect_boundary() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("repository");
    let support = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("provider support");
    let tasks = dir.path().join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("tasks");
    std::fs::create_dir_all(dir.path().join("tests")).expect("tests");
    std::fs::write(dir.path().join("tests/proof.txt"), "proof input\n").expect("proof");
    let task_path = tasks.join("A-1.md");
    std::fs::write(
        &task_path,
        "---\nid: A-1\ntitle: unwritable publication\nphase: 1\nstatus: todo\neffort_days: 1\npaths: ['tests/**']\n---\n\nbody\n",
    )
    .expect("task");
    let list = support.path().join("list.sh");
    let runner = support.path().join("run.sh");
    let executed = support.path().join("executed.log");
    std::fs::write(&list, "#!/bin/sh\nprintf '%s\\n' suite::green\n").expect("list");
    std::fs::write(
        &runner,
        format!(
            "#!/bin/sh\nprintf 'runner-authored effect\\n' >> '{}'\nexit 0\n",
            executed.display()
        ),
    )
    .expect("runner");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!(
            "root: aplan\ntest:\n  name: passing\n  list: [sh, {}]\n  run: [sh, {}, '{{test}}']\n  run_selects: true\n  covers: ['tests/**']\n",
            list.display(),
            runner.display()
        ),
    )
    .expect("config");
    git(dir.path(), &["init", "-q", "."]);
    git(dir.path(), &["config", "user.email", "t@example.com"]);
    git(dir.path(), &["config", "user.name", "t"]);
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &["commit", "-q", "-m", "publication failure fixture"],
    );
    let before = std::fs::read(&task_path).expect("task before");
    let original_mode = std::fs::metadata(&tasks)
        .expect("task directory")
        .permissions()
        .mode();
    std::fs::set_permissions(&tasks, std::fs::Permissions::from_mode(0o555))
        .expect("make publication fail");

    let (stdout, stderr, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            "test:suite::green",
            "--execute",
        ],
    );
    std::fs::set_permissions(&tasks, std::fs::Permissions::from_mode(original_mode))
        .expect("restore task directory");
    assert!(!ok, "unwritable task unexpectedly closed: {stdout}{stderr}");
    let output = format!("{stdout}{stderr}");
    assert!(
        output.contains("`aplan` published no plan bytes")
            && output.contains("configured test command")
            && output.contains("not rolled back"),
        "the publication error hid the runner-effect boundary: {output}"
    );
    assert_eq!(
        std::fs::read(&task_path).expect("task after"),
        before,
        "a failed publication changed the task"
    );
    assert!(
        executed.exists(),
        "the fixture did not run the configured command before publication"
    );
}

/// End-to-end CLI audit test for `A2-153`: `aplan verify` surfaces advisory
/// `candidate_projections` on a dirty working tree across JSON, agent, and
/// human formats without altering the HEAD verdict, and emits nothing extra on
/// a clean tree.
#[test]
fn verify_cli_reports_candidate_projections_on_dirty_tree_and_stays_silent_when_clean() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("repository");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(root.join("docs")).expect("docs");
    std::fs::create_dir_all(root.join("src")).expect("src");
    std::fs::write(root.join("src/lib.rs"), "pub fn ok() {}\n").expect("src");
    std::fs::write(
        root.join("docs/spec.md"),
        "# Spec\n\n## Section\n\nOriginal text.\n",
    )
    .expect("doc");
    std::fs::write(root.join(".aplan.yml"), "root: aplan\ndetect_drift: true\n").expect("cfg");
    std::fs::write(
        root.join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: done traced\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths: [src/lib.rs, docs/spec.md]\ntraces:\n  - docs/spec.md#section\n---\n\nbody\n",
    )
    .expect("A-1");
    std::fs::write(
        root.join("aplan/tasks/A-2.md"),
        "---\nid: A-2\ntitle: live traced\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths: [src/lib.rs]\ntraces:\n  - docs/spec.md#section\n---\n\nbody\n",
    )
    .expect("A-2");

    git(root, &["init", "-q", "."]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "initial"]);
    let sha = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .expect("git rev-parse")
            .stdout,
    )
    .expect("utf8 sha")
    .trim()
    .to_string();

    // Close A-1 with both commit and file evidence so it stamps
    // `evidence_fingerprints` (for APLAN054) and `trace_fingerprints`, then commit.
    let (out, err, ok) = run(
        root,
        &[
            "close",
            "A-1",
            "--evidence",
            &format!("commit:{sha}"),
            "--evidence",
            "file:docs/spec.md",
        ],
    );
    assert!(ok, "close failed: {out}{err}");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "close A-1"]);

    // Clean tree: no candidate_projections in json/agent/human.
    let (json_clean, err_clean, ok_clean) = run(root, &["verify", "--format", "json"]);
    assert!(ok_clean, "clean verify failed: {json_clean}{err_clean}");
    let clean_doc: serde_json::Value = serde_json::from_str(&json_clean).expect("clean json");
    assert!(clean_doc.get("candidate_projections").is_none());
    let (_, human_err_clean, ok_h_clean) = run(root, &["verify"]);
    assert!(ok_h_clean);
    assert!(!human_err_clean.contains("candidate projection"));

    // Dirty tree: edit docs/spec.md without committing.
    std::fs::write(
        root.join("docs/spec.md"),
        "# Spec\n\n## Section\n\nEdited uncommitted text.\n",
    )
    .expect("dirty doc");

    for fmt in ["json", "agent"] {
        let (out_d, err_d, ok_d) = run(root, &["verify", "--format", fmt]);
        assert!(
            ok_d,
            "dirty working tree must not alter HEAD verdict ({fmt}): {out_d}{err_d}"
        );
        let doc: serde_json::Value = serde_json::from_str(&out_d).expect("dirty json");
        let projs = doc["candidate_projections"]
            .as_array()
            .expect("candidate_projections array");
        assert!(
            projs
                .iter()
                .any(|p| p["code"] == "APLAN054" && p["task"] == "A-1" && p["state"] == "unstaged"),
            "expected APLAN054 projection for A-1 ({fmt}): {projs:?}"
        );
        assert!(
            projs
                .iter()
                .any(|p| p["code"] == "APLAN045" && p["task"] == "A-2" && p["state"] == "unstaged"),
            "expected APLAN045 projection for A-2 ({fmt}): {projs:?}"
        );
    }

    let (_, human_err_dirty, ok_h_dirty) = run(root, &["verify"]);
    assert!(ok_h_dirty);
    assert!(
        human_err_dirty.contains("candidate projection")
            && human_err_dirty.contains("APLAN054")
            && human_err_dirty.contains("APLAN045"),
        "human verify should report candidate projections on stderr:\n{human_err_dirty}"
    );
}

/// Audit test for `A2-156`: `aplan check --range` handles merge commits and
/// root-inclusive `..HEAD` ranges deterministically.
#[test]
fn check_range_handles_merge_commits_and_root_inclusive_range() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("repository");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("tasks");
    std::fs::write(root.join(".aplan.yml"), "root: aplan\ndetect_drift: true\n").expect("cfg");
    std::fs::write(
        root.join("aplan/tasks/M-1.md"),
        "---\nid: M-1\ntitle: base task\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
    )
    .expect("M-1");

    git(root, &["init", "-q", "."]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "root commit"]);

    git(root, &["checkout", "-q", "-b", "side"]);
    std::fs::write(
        root.join("aplan/tasks/M-2.md"),
        "---\nid: M-2\ntitle: side task\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
    )
    .expect("M-2");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "side commit"]);

    git(root, &["checkout", "-q", "-"]);
    std::fs::write(
        root.join("aplan/tasks/M-3.md"),
        "---\nid: M-3\ntitle: main task\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
    )
    .expect("M-3");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "main commit"]);
    git(
        root,
        &["merge", "-q", "--no-ff", "side", "-m", "merge side"],
    );

    let (out_all, err_all, ok_all) = run(root, &["check", "--range", "..HEAD", "--format", "json"]);
    assert!(ok_all, "..HEAD failed: {out_all}{err_all}");
    let doc_all: serde_json::Value = serde_json::from_str(&out_all).expect("json");
    assert_eq!(doc_all["range_clean"], true);
    assert_eq!(doc_all["commits_total"], 4);
    assert_eq!(doc_all["commits_audited"], 4);

    let (out_merge, err_merge, ok_merge) = run(
        root,
        &["check", "--range", "HEAD~1..HEAD", "--format", "json"],
    );
    assert!(ok_merge, "HEAD~1..HEAD failed: {out_merge}{err_merge}");
    let doc_merge: serde_json::Value = serde_json::from_str(&out_merge).expect("json");
    assert_eq!(doc_merge["range_clean"], true);
    assert!(doc_merge["commits_audited"].as_u64().unwrap_or(0) >= 2);
}

/// Proof for `close.combines_existing_live_evidence` (`A2-147`): closing a
/// live task (`todo` or `in_progress`) preserves existing citations, combines
/// them with newly supplied citations, allows closing an adequate existing set
/// with no redundant flag, refuses duplicates or invalid existing citations
/// before writing, and works across CLI and MCP.
#[test]
#[allow(clippy::too_many_lines)]
fn close_combines_existing_live_evidence_across_cli_app_and_mcp() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("repository");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(root.join("src")).expect("src");
    std::fs::write(root.join(".aplan.yml"), "root: aplan\n").expect("cfg");
    std::fs::write(root.join("src/a.rs"), "pub fn a() {}\n").expect("a");
    std::fs::write(root.join("src/b.rs"), "pub fn b() {}\n").expect("b");

    git(root, &["init", "-q", "."]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "first increment"]);
    let sha1 = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .expect("rev-parse")
            .stdout,
    )
    .expect("utf8")
    .trim()
    .to_string();

    std::fs::write(root.join("src/a.rs"), "pub fn a() { /* v2 */ }\n").expect("a2");
    std::fs::write(root.join("src/b.rs"), "pub fn b() { /* v2 */ }\n").expect("b2");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "second increment"]);
    let sha2 = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .expect("rev-parse")
            .stdout,
    )
    .expect("utf8")
    .trim()
    .to_string();

    // T-1: `in_progress` with 1 existing commit -> combine with sha2.
    std::fs::write(
        root.join("aplan/tasks/T-1.md"),
        format!(
            "---\nid: T-1\ntitle: combine\nphase: 1\nstatus: in_progress\neffort_days: 1\n\
             paths: [src/a.rs]\nevidence:\n  - kind: commit\n    ref: {sha1}\n---\n\nbody\n"
        ),
    )
    .expect("T-1");

    // Duplicate of existing citation must be refused before writing.
    let (dup_out, dup_err, dup_ok) = run(
        root,
        &["close", "T-1", "--evidence", &format!("commit:{sha1}")],
    );
    assert!(
        !dup_ok,
        "duplicate against existing must refuse: {dup_out}{dup_err}"
    );

    let (out1, err1, ok1) = run(
        root,
        &["close", "T-1", "--evidence", &format!("commit:{sha2}")],
    );
    assert!(ok1, "combining live evidence failed: {out1}{err1}");
    let t1_text = std::fs::read_to_string(root.join("aplan/tasks/T-1.md")).expect("read T-1");
    assert!(t1_text.contains("status: done"), "{t1_text}");
    assert!(
        t1_text.contains(&sha1) && t1_text.contains(&sha2),
        "{t1_text}"
    );

    // Already-done task still refuses and points to `amend`.
    let (done_out, done_err, done_ok) = run(
        root,
        &["close", "T-1", "--evidence", &format!("commit:{sha2}")],
    );
    assert!(!done_ok);
    assert!(
        format!("{done_out}{done_err}").contains("aplan amend T-1"),
        "already-done refusal should name amend: {done_out}{done_err}"
    );

    // T-2: `todo` with adequate existing evidence -> closes with NO `--evidence` flag.
    std::fs::write(
        root.join("aplan/tasks/T-2.md"),
        format!(
            "---\nid: T-2\ntitle: adequate existing\nphase: 1\nstatus: todo\neffort_days: 1\n\
             paths: [src/a.rs]\nevidence:\n  - kind: commit\n    ref: {sha1}\n  - kind: file\n    ref: src/a.rs\n---\n\nbody\n"
        ),
    )
    .expect("T-2");
    let (out2, err2, ok2) = run(root, &["close", "T-2"]);
    assert!(
        ok2,
        "closing adequate existing evidence failed: {out2}{err2}"
    );
    let t2_text = std::fs::read_to_string(root.join("aplan/tasks/T-2.md")).expect("read T-2");
    assert!(t2_text.contains("status: done"), "{t2_text}");
    assert!(
        !t2_text.contains(&sha2),
        "should not add redundant HEAD commit: {t2_text}"
    );

    // T-bad: live task with invalid existing citation -> refuses without writing.
    std::fs::write(
        root.join("aplan/tasks/T-bad.md"),
        "---\nid: T-bad\ntitle: bad existing\nphase: 1\nstatus: in_progress\neffort_days: 1\n\
         paths: [src/a.rs]\nevidence:\n  - kind: commit\n    ref: deadbeef0000\n---\n\nbody\n",
    )
    .expect("T-bad");
    let (bad_out, bad_err, bad_ok) = run(
        root,
        &["close", "T-bad", "--evidence", &format!("commit:{sha2}")],
    );
    assert!(
        !bad_ok,
        "invalid existing evidence must refuse: {bad_out}{bad_err}"
    );
    let bad_after = std::fs::read_to_string(root.join("aplan/tasks/T-bad.md")).expect("read");
    assert!(bad_after.contains("status: in_progress"), "{bad_after}");

    // T-mcp: MCP `close` combines existing live evidence with supplied citation.
    std::fs::write(
        root.join("aplan/tasks/T-mcp.md"),
        format!(
            "---\nid: T-mcp\ntitle: mcp combine\nphase: 1\nstatus: in_progress\neffort_days: 1\n\
             paths: [src/b.rs]\nevidence:\n  - kind: commit\n    ref: {sha1}\n---\n\nbody\n"
        ),
    )
    .expect("T-mcp");
    let policy = aplan_mcp::Policy {
        id_strategy: aplan_core::IdStrategy::Sequential,
        options: aplan_core::Options::default(),
        verdict: aplan_core::Verdict::default(),
        verify: aplan_evidence::VerifyOptions {
            detect_drift: false,
            version: None,
            traces_root: None,
            config_file: None,
            execute: false,
            test_runner: None,
            max_unrelated_files: None,
            observation_inputs: None,
            url_check: None,
        },
        repo_root: root.to_path_buf(),
    };
    let mcp_res = aplan_mcp::tools::close(
        &root.join("aplan"),
        "T-mcp",
        &[format!("commit:{sha2}")],
        &policy,
    )
    .expect("mcp close combines live evidence");
    assert_eq!(mcp_res["evidence"].as_array().expect("arr").len(), 2);
}

/// Regression test for `A2-183` (#56): `APLAN015` fix text for `HEAD`-relative
/// `file:` citations names `HEAD` and `aplan amend` (never "the cited rev"),
/// and `aplan close` warns when `HEAD` breaks an already-closed task's `file:`
/// citation.
#[test]
fn close_warns_when_commit_deletes_closed_task_file_evidence_and_aplan015_fix_is_truthful() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("repository");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(root.join("src")).expect("src");
    std::fs::write(root.join(".aplan.yml"), "root: aplan\n").expect("cfg");
    std::fs::write(root.join("src/old.rs"), "pub fn old() {}\n").expect("old");
    std::fs::write(
        root.join("aplan/tasks/F-1.md"),
        "---\nid: F-1\ntitle: cited file\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths: [src/old.rs]\n---\n\nbody\n",
    )
    .expect("F-1");
    std::fs::write(
        root.join("aplan/tasks/F-2.md"),
        "---\nid: F-2\ntitle: delete old\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths: [src/old.rs]\n---\n\nbody\n",
    )
    .expect("F-2");

    git(root, &["init", "-q", "."]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "initial"]);

    let (out1, err1, ok1) = run(root, &["close", "F-1", "--evidence", "file:src/old.rs"]);
    assert!(ok1, "close F-1 failed: {out1}{err1}");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "close F-1"]);

    // Delete src/old.rs in a later commit and close F-2.
    std::fs::remove_file(root.join("src/old.rs")).expect("rm");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "delete old.rs"]);
    let del_sha = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .expect("rev-parse")
            .stdout,
    )
    .expect("utf8")
    .trim()
    .to_string();

    let (out2, err2, ok2) = run(
        root,
        &["close", "F-2", "--evidence", &format!("commit:{del_sha}")],
    );
    assert!(ok2, "close F-2 failed: {out2}{err2}");
    assert!(
        out2.contains("F-1") && out2.contains("file:src/old.rs") && out2.contains("APLAN015"),
        "close should warn about broken closed file citation:\n{out2}"
    );

    let (v_out, _v_err, v_ok) = run(root, &["verify", "--format", "agent"]);
    assert!(!v_ok, "verify should fail with APLAN015");
    let doc: serde_json::Value = serde_json::from_str(&v_out).expect("json");
    let aplan015 = doc["diagnostics"]
        .as_array()
        .expect("diags")
        .iter()
        .find(|d| d["code"] == "APLAN015")
        .expect("APLAN015 finding");
    let fix = aplan015["agent_instruction"].as_str().expect("fix");
    assert!(
        fix.contains("HEAD") && fix.contains("aplan amend") && !fix.contains("cited rev"),
        "APLAN015 fix should name HEAD and aplan amend, not cited rev: {fix}"
    );
}

/// Regression test for `A2-157`: `aplan add --paths` rejects comma-separated
/// lists before writing any file, suggests repeated `--paths`, and accepts
/// repeated flags, brace globs (`{a,b}`), and escaped commas (`\,`).
#[test]
fn add_rejects_comma_separated_paths_and_supports_repeated_braces_and_escaped_comma() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("repository");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("tasks");
    std::fs::write(root.join(".aplan.yml"), "root: aplan\n").expect("cfg");

    let (bad_out, bad_err, bad_ok) = run(
        root,
        &[
            "add",
            "Comma near miss",
            "--id",
            "P-1",
            "--effort",
            "1",
            "--paths",
            "src/**,docs/**,tests/**",
        ],
    );
    assert!(
        !bad_ok,
        "comma-separated paths must refuse: {bad_out}{bad_err}"
    );
    let combined = format!("{bad_out}{bad_err}");
    assert!(
        combined.contains("--paths 'src/**' --paths 'docs/**' --paths 'tests/**'"),
        "refusal should show corrected repeated syntax:\n{combined}"
    );
    assert!(
        !root.join("aplan/tasks/P-1.md").exists(),
        "refused add must not write a task file"
    );

    let (ok_out, ok_err, ok) = run(
        root,
        &[
            "add",
            "Valid paths",
            "--id",
            "P-2",
            "--effort",
            "1",
            "--paths",
            "src/**",
            "--paths",
            "crates/{a,b}/**",
            "--paths",
            r"docs/notes\,v1.md",
        ],
    );
    assert!(
        ok,
        "valid repeated/brace/escaped paths failed: {ok_out}{ok_err}"
    );
    let content = std::fs::read_to_string(root.join("aplan/tasks/P-2.md")).expect("read P-2");
    let task = aplan_core::mutation::task_of(&content).expect("parse P-2");
    assert_eq!(
        task.paths,
        vec![
            "src/**".to_string(),
            "crates/{a,b}/**".to_string(),
            "docs/notes,v1.md".to_string()
        ]
    );
}

/// Regression test for `A2-161`: `verify` flags a working-tree hand-reopen of
/// a committed `done` task and names `aplan supersede`, and CLI help for
/// `supersede` and `amend --remove-evidence` covers withdrawn deliveries.
#[test]
fn verify_flags_working_tree_hand_reopen_and_help_names_supersede_and_amend() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("repository");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(root.join("src")).expect("src");
    std::fs::write(root.join(".aplan.yml"), "root: aplan\n").expect("cfg");
    std::fs::write(root.join("src/lib.rs"), "pub fn x() {}\n").expect("lib");
    std::fs::write(
        root.join("aplan/tasks/W-1.md"),
        "---\nid: W-1\ntitle: delivered\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths: [src/lib.rs]\n---\n\nbody\n",
    )
    .expect("W-1");

    git(root, &["init", "-q", "."]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "init"]);
    let sha = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .expect("rev-parse")
            .stdout,
    )
    .expect("utf8")
    .trim()
    .to_string();

    let (c_out, c_err, c_ok) = run(
        root,
        &["close", "W-1", "--evidence", &format!("commit:{sha}")],
    );
    assert!(c_ok, "close failed: {c_out}{c_err}");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "close W-1"]);

    // Hand-reopen W-1 in the working tree back to `todo`.
    std::fs::write(
        root.join("aplan/tasks/W-1.md"),
        "---\nid: W-1\ntitle: delivered\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths: [src/lib.rs]\n---\n\nbody\n",
    )
    .expect("reopen W-1");

    let (v_out, v_err, v_ok) = run(root, &["verify", "--format", "json"]);
    assert!(v_ok, "verify failed: {v_out}{v_err}");
    let doc: serde_json::Value = serde_json::from_str(&v_out).expect("json");
    let projs = doc["candidate_projections"]
        .as_array()
        .expect("candidate_projections");
    assert!(
        projs.iter().any(|p| {
            p["code"] == "reopen"
                && p["task"] == "W-1"
                && p["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("aplan supersede W-1"))
        }),
        "expected advisory reopen projection naming supersede: {projs:?}"
    );

    let (s_help, _, s_ok) = run(root, &["supersede", "--help"]);
    assert!(s_ok);
    assert!(
        s_help.contains("withdrawn") && s_help.contains("traces"),
        "supersede help should cover withdrawn deliveries with or without traces:\n{s_help}"
    );

    let (a_help, _, a_ok) = run(root, &["amend", "--help"]);
    assert!(a_ok);
    assert!(
        a_help.contains("removed with the work") && a_help.contains("aplan supersede"),
        "amend help should cover citations removed with reverted work:\n{a_help}"
    );
}
