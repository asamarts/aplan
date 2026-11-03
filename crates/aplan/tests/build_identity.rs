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

#[allow(dead_code)]
#[path = "../../aplan-attest/build_support.rs"]
mod attest_build_support;
use crate::common;

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

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run Git in build-identity fixture");
    assert!(
        output.status.success(),
        "git {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git_text(dir: &Path, args: &[&str]) -> String {
    String::from_utf8(git(dir, args).stdout)
        .expect("Git fixture output is UTF-8")
        .trim()
        .to_owned()
}

fn write_build_identity_fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("src")).expect("create build-identity source directory");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"identity-fixture\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .expect("write build-identity manifest");
    std::fs::write(
        dir.join("src/main.rs"),
        "fn main() { println!(\"{}\", env!(\"APLAN_SOURCE_REV\")); }\n",
    )
    .expect("write build-identity executable");
    let attest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../aplan-attest");
    std::fs::copy(attest.join("build.rs"), dir.join("build.rs"))
        .expect("copy production build script");
    std::fs::copy(
        attest.join("build_support.rs"),
        dir.join("build_support.rs"),
    )
    .expect("copy production build support");
}

fn init_build_identity_fixture(dir: &Path) {
    write_build_identity_fixture(dir);
    git(dir, &["init", "-q"]);
    git(
        dir,
        &["config", "user.email", "build-identity@example.invalid"],
    );
    git(dir, &["config", "user.name", "build identity fixture"]);
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "initial"]);
}

fn cargo_build(dir: &Path, target: &Path, git_dir: Option<&str>) -> std::process::Output {
    let git_environment = git_dir.map_or_else(Vec::new, |value| vec![("GIT_DIR", value)]);
    cargo_build_with_git_environment(dir, target, &git_environment)
}

fn cargo_build_with_git_environment(
    dir: &Path,
    target: &Path,
    git_environment: &[(&str, &str)],
) -> std::process::Output {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = std::process::Command::new(cargo);
    command
        .args(["build", "--offline", "--verbose", "--color", "never"])
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", target)
        // Reproduce the authoritative runner's inherited presentation policy.
        // The explicit command-line mode above must win at the boundary whose
        // output this test parses (`A1-641`).
        .env("CARGO_TERM_COLOR", "always");
    // Make the fixture independent of the shell that launched the outer test;
    // each case then adds exactly the hostile Git context it means to exercise.
    for (name, _) in std::env::vars_os().filter(|(name, _)| {
        name.to_string_lossy()
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("GIT_"))
    }) {
        command.env_remove(name);
    }
    for (name, value) in git_environment {
        command.env(name, value);
    }
    command.output().expect("run Cargo freshness fixture")
}

fn assert_build_succeeds(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "Cargo fixture failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_fresh(output: &std::process::Output) {
    assert_build_succeeds(output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.as_bytes().contains(&0x1b),
        "Cargo freshness diagnostics retained terminal escapes:\n{stderr}"
    );
    assert!(
        stderr.contains("Fresh identity-fixture"),
        "Cargo did not report the unchanged package fresh:\n{stderr}"
    );
    assert!(
        !stderr.contains("Dirty identity-fixture")
            && !stderr.contains("Compiling identity-fixture"),
        "Cargo rebuilt an unchanged package:\n{stderr}"
    );
}

fn built_revision(target: &Path) -> String {
    let output = std::process::Command::new(target.join("debug/identity-fixture"))
        .output()
        .expect("run build-identity fixture");
    assert!(output.status.success(), "fixture executable runs");
    String::from_utf8(output.stdout)
        .expect("fixture revision is UTF-8")
        .trim()
        .to_owned()
}

fn assert_exact_existing_watchers(repo: &Path, expected_ref: Option<&str>) {
    let paths = attest_build_support::git_watch_paths(repo);
    assert!(!paths.is_empty(), "a Git checkout must watch its HEAD");
    assert!(
        paths
            .iter()
            .all(|path| path.try_exists().is_ok_and(|exists| exists)),
        "every Cargo watcher must exist: {paths:?}"
    );
    let common = Path::new(&git_text(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ))
    .to_path_buf();
    let refs = std::fs::canonicalize(common.join("refs/heads")).expect("refs/heads directory");
    assert!(
        !paths.iter().any(|path| path == &refs),
        "the whole shared branch directory must never be watched: {paths:?}"
    );
    if let Some(expected_ref) = expected_ref {
        let expected =
            std::fs::canonicalize(common.join(expected_ref)).expect("expected loose branch ref");
        assert!(
            paths.contains(&expected),
            "exact branch ref is absent: {paths:?}"
        );
    }
}

/// `A1-629`: the production build script drives Cargo's real freshness logic,
/// not merely a test double of the path selector.
/// Proves `facts.source_identity_tracks_active_checkout` (`A1-638`).
#[test]
fn build_identity_tracks_only_the_checked_out_ref_and_stays_fresh() {
    let root = scratch();
    let repo = root.path().join("repo");
    std::fs::create_dir(&repo).expect("create main fixture checkout");
    init_build_identity_fixture(&repo);
    let target = root.path().join("main-target");

    let first = cargo_build(&repo, &target, None);
    assert_build_succeeds(&first);
    assert_eq!(
        built_revision(&target),
        git_text(&repo, &["rev-parse", "HEAD"])
    );
    assert_fresh(&cargo_build(&repo, &target, None));

    let current_ref = git_text(&repo, &["symbolic-ref", "HEAD"]);
    assert_exact_existing_watchers(&repo, Some(&current_ref));

    // Updating another branch changes a file beside the selected ref, but not
    // any input this checkout declared to Cargo.
    let tree = git_text(&repo, &["rev-parse", "HEAD^{tree}"]);
    let other = git_text(&repo, &["commit-tree", &tree, "-m", "other branch"]);
    git(&repo, &["update-ref", "refs/heads/other", &other]);
    assert_fresh(&cargo_build(&repo, &target, None));

    git(
        &repo,
        &["commit", "--allow-empty", "-q", "-m", "current branch"],
    );
    let changed = cargo_build(&repo, &target, None);
    assert_build_succeeds(&changed);
    assert_eq!(
        built_revision(&target),
        git_text(&repo, &["rev-parse", "HEAD"])
    );
    assert_fresh(&cargo_build(&repo, &target, None));

    // Packing deletes the watched loose ref. Cargo reruns once, then the exact
    // reflog and packed-refs are both existing stable inputs.
    git(&repo, &["pack-refs", "--all", "--prune"]);
    assert_build_succeeds(&cargo_build(&repo, &target, None));
    assert_eq!(
        built_revision(&target),
        git_text(&repo, &["rev-parse", "HEAD"])
    );
    assert_exact_existing_watchers(&repo, None);
    assert_fresh(&cargo_build(&repo, &target, None));

    git(
        &repo,
        &["commit", "--allow-empty", "-q", "-m", "unpack current ref"],
    );
    assert_build_succeeds(&cargo_build(&repo, &target, None));
    assert_eq!(
        built_revision(&target),
        git_text(&repo, &["rev-parse", "HEAD"])
    );

    git(&repo, &["checkout", "-q", "other"]);
    assert_build_succeeds(&cargo_build(&repo, &target, None));
    assert_eq!(built_revision(&target), other);

    git(&repo, &["checkout", "-q", "--detach"]);
    assert_eq!(
        attest_build_support::git_watch_paths(&repo).len(),
        1,
        "a detached checkout needs only its worktree-specific HEAD"
    );
    git(
        &repo,
        &["commit", "--allow-empty", "-q", "-m", "detached commit"],
    );
    assert_build_succeeds(&cargo_build(&repo, &target, None));
    assert_eq!(
        built_revision(&target),
        git_text(&repo, &["rev-parse", "HEAD"])
    );
}

/// `A1-638`: a packed branch without reflogs uses one existing directory as a
/// transition watcher, then returns to the exact loose ref after the commit.
/// Proves `facts.source_identity_tracks_active_checkout`.
#[test]
fn packed_ref_without_reflogs_still_refreshes_the_source_identity() {
    let root = scratch();
    let repo = root.path().join("repo");
    std::fs::create_dir(&repo).expect("create no-reflog fixture checkout");
    init_build_identity_fixture(&repo);
    let target = root.path().join("target");

    assert_build_succeeds(&cargo_build(&repo, &target, None));
    let before = built_revision(&target);
    git(&repo, &["config", "core.logAllRefUpdates", "false"]);
    let git_dir = Path::new(&git_text(
        &repo,
        &["rev-parse", "--path-format=absolute", "--git-dir"],
    ))
    .to_path_buf();
    if git_dir.join("logs").exists() {
        std::fs::remove_dir_all(git_dir.join("logs")).expect("remove fixture reflogs");
    }
    git(&repo, &["pack-refs", "--all", "--prune"]);

    // Deleting the previously watched loose ref reruns the build script once.
    assert_build_succeeds(&cargo_build(&repo, &target, None));
    let ref_directory = std::fs::canonicalize(git_dir.join("refs/heads"))
        .expect("canonical no-reflog refs directory");
    let packed_watchers = attest_build_support::git_watch_paths(&repo);
    assert!(
        packed_watchers.contains(&ref_directory),
        "no exact ref or reflog exists, so the nearest ref directory must cover its creation: \
         {packed_watchers:?}"
    );
    assert_fresh(&cargo_build(&repo, &target, None));

    git(
        &repo,
        &[
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "unpack without reflog",
        ],
    );
    assert_build_succeeds(&cargo_build(&repo, &target, None));
    let after = git_text(&repo, &["rev-parse", "HEAD"]);
    assert_ne!(after, before, "the fixture commit must have moved");
    assert_eq!(built_revision(&target), after);

    let current_ref = git_text(&repo, &["symbolic-ref", "HEAD"]);
    assert_exact_existing_watchers(&repo, Some(&current_ref));
    assert_fresh(&cargo_build(&repo, &target, None));
}

/// `A1-638`: inherited Git discovery and configuration variables cannot
/// redirect a build-time query away from its source directory.
/// Proves `facts.source_identity_tracks_active_checkout`.
#[test]
fn source_identity_ignores_all_inherited_git_context() {
    let root = scratch();
    let repo = root.path().join("repo");
    let package = repo.join("package");
    std::fs::create_dir_all(&package).expect("create nested identity fixture");
    write_build_identity_fixture(&package);
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &["config", "user.email", "build-identity@example.invalid"],
    );
    git(&repo, &["config", "user.name", "build identity fixture"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "nested package"]);

    let ceiling = repo.to_str().expect("fixture path is UTF-8");
    let hidden = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(&package)
        .env("GIT_CEILING_DIRECTORIES", ceiling)
        .output()
        .expect("run negative-control Git query");
    assert!(
        !hidden.status.success(),
        "the inherited ceiling must hide the parent repository from an unsanitized Git query"
    );

    let target = root.path().join("target");
    let hostile = [
        ("GIT_CEILING_DIRECTORIES", ceiling),
        ("GIT_CONFIG_COUNT", "not-a-number"),
        ("GIT_DIR", "relative-hook-git-dir"),
    ];
    let first = cargo_build_with_git_environment(&package, &target, &hostile);
    assert_build_succeeds(&first);
    assert_eq!(
        built_revision(&target),
        git_text(&repo, &["rev-parse", "HEAD"]),
        "the package path, not inherited Git context, selects the source checkout"
    );
    assert_fresh(&cargo_build_with_git_environment(
        &package, &target, &hostile,
    ));
}

/// `A1-629`: linked-worktree metadata and hook-local Git variables select the
/// source checkout rather than creating missing main-checkout paths.
/// Proves `facts.source_identity_tracks_active_checkout` (`A1-638`).
#[test]
fn linked_worktree_and_non_git_builds_have_stable_existing_inputs() {
    let root = scratch();
    let repo = root.path().join("repo");
    let linked = root.path().join("linked");
    std::fs::create_dir(&repo).expect("create linked-worktree source");
    init_build_identity_fixture(&repo);
    let linked_text = linked.to_str().expect("fixture path is UTF-8");
    git(
        &repo,
        &["worktree", "add", "-q", "-b", "linked", linked_text, "HEAD"],
    );

    let linked_target = root.path().join("linked-target");
    let first = cargo_build(&linked, &linked_target, Some("relative-hook-git-dir"));
    assert_build_succeeds(&first);
    assert_eq!(
        built_revision(&linked_target),
        git_text(&linked, &["rev-parse", "HEAD"])
    );
    assert_fresh(&cargo_build(
        &linked,
        &linked_target,
        Some("relative-hook-git-dir"),
    ));
    assert_exact_existing_watchers(&linked, Some("refs/heads/linked"));

    git(
        &linked,
        &["commit", "--allow-empty", "-q", "-m", "linked commit"],
    );
    assert_build_succeeds(&cargo_build(
        &linked,
        &linked_target,
        Some("relative-hook-git-dir"),
    ));
    assert_eq!(
        built_revision(&linked_target),
        git_text(&linked, &["rev-parse", "HEAD"])
    );

    let linked_tree = git_text(&linked, &["rev-parse", "HEAD^{tree}"]);
    let linked_parent = git_text(&linked, &["rev-parse", "HEAD"]);
    let linked_other = git_text(
        &linked,
        &[
            "commit-tree",
            &linked_tree,
            "-p",
            &linked_parent,
            "-m",
            "linked other branch",
        ],
    );
    git(
        &linked,
        &["update-ref", "refs/heads/linked-other", &linked_other],
    );
    assert_fresh(&cargo_build(
        &linked,
        &linked_target,
        Some("relative-hook-git-dir"),
    ));
    git(&linked, &["pack-refs", "--all", "--prune"]);
    assert_build_succeeds(&cargo_build(
        &linked,
        &linked_target,
        Some("relative-hook-git-dir"),
    ));
    assert_eq!(built_revision(&linked_target), linked_parent);
    assert_fresh(&cargo_build(
        &linked,
        &linked_target,
        Some("relative-hook-git-dir"),
    ));

    git(&linked, &["checkout", "-q", "linked-other"]);
    assert_build_succeeds(&cargo_build(
        &linked,
        &linked_target,
        Some("relative-hook-git-dir"),
    ));
    assert_eq!(built_revision(&linked_target), linked_other);

    let outside = root.path().join("not-a-repository");
    std::fs::create_dir(&outside).expect("create non-Git source");
    write_build_identity_fixture(&outside);
    let outside_target = root.path().join("outside-target");
    assert_build_succeeds(&cargo_build(
        &outside,
        &outside_target,
        Some("relative-hook-git-dir"),
    ));
    assert_eq!(built_revision(&outside_target), "");
    assert_fresh(&cargo_build(
        &outside,
        &outside_target,
        Some("relative-hook-git-dir"),
    ));
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

    let bytes = std::fs::read(common::binary()).expect("read the binary under test");
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

/// **Asking for identity does not alter the capability document**, which is why
/// identity is a separate document rather than four more keys in that one.
///
/// This comparison must use the same build on both sides. A supported headless
/// build deliberately has fewer capabilities than the committed standard-build
/// document, so comparing it with `facts.json` tests the feature set rather than
/// whether asking for identity changed anything.
#[test]
fn asking_for_the_identity_does_not_change_the_facts_document() {
    let dir = scratch();
    let before = common::run(dir.path(), &["facts"]);
    assert_eq!(before.code, 0, "{}", before.text());

    let identified = identity(dir.path());

    let after = common::run(dir.path(), &["facts"]);
    assert_eq!(after.code, 0, "{}", after.text());
    assert_eq!(
        before.stdout, after.stdout,
        "asking which build this is changed what it says it can do"
    );
    assert_eq!(
        identified["facts_sha256"].as_str().unwrap_or_default(),
        aplan_attest::sha256_hex(after.stdout.as_bytes()),
        "identity must still name the unchanged capability document"
    );
    assert!(
        !after.stdout.contains("binary_sha256"),
        "the capability document must carry no per-build digest"
    );
}

/// **The standard build's `facts.json` is committed byte for byte.**
///
/// The feature condition is part of the claim: ADR-0048 requires a headless
/// build to omit the TUI capability and behavior names, while the checked-in
/// document describes the standard, default-feature product.
#[cfg(feature = "tui")]
#[test]
fn the_default_feature_capability_document_is_committed() {
    let dir = scratch();
    let facts = common::run(dir.path(), &["facts"]);
    assert_eq!(facts.code, 0, "{}", facts.text());
    let committed =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../facts.json"))
            .expect("facts.json");
    assert_eq!(
        facts.stdout, committed,
        "the capability document is the one that is committed and gated"
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

    for field in ["version", "facts_sha256", "binary_sha256"] {
        assert_eq!(
            tool[field], doc[field],
            "`attest` and `facts --identity` disagree about `{field}`:\n  attest:   {}\n               identity: {}",
            tool[field], doc[field]
        );
    }

    // `attest` retains the v1 source string, while the newer identity document
    // omits an unknown source. Both mean "this build could not name a commit".
    // A build from a tarball or a git hook can exercise this supported case.
    if tool["source"] == "" {
        assert!(
            doc.get("source").is_none(),
            "an unknown source must be absent from facts --identity: {doc}"
        );
    } else {
        assert_eq!(tool["source"], doc["source"]);
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
