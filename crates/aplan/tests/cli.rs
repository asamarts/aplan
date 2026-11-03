//! The CLI black-box corpus.
//!
//! `aplan-core` and `aplan-output` are well covered by unit tests; the binary
//! that wires them together was not covered at all, and that is exactly where a
//! review found every bug: a missing `--root` reported a clean plan, `ready`
//! announced explicitly parked tasks, and three commands answered questions from
//! a plan that had failed to load. Unit tests could not have caught any of them.
//!
//! Each `tests/cmd/<name>.trycmd` runs a sequence of commands against the
//! sandboxed plan in `tests/cmd/<name>.in/`, pinning stdout, stderr, and, the
//! contract the README makes, the **exit code**: `0` clean, `1` violations,
//! `2` usage or internal failure.
//!
//! The source fixtures live inside this checkout. `trycmd` uses those adjacent
//! `.in` directories directly when there is no `.out` directory, so libgit2
//! would otherwise walk upward and combine a fixture plan with this checkout's
//! private claim store. The corpus is copied under a unique temporary repository
//! before it runs. The enclosing repository deliberately holds an incompatible
//! claim: that makes the boundary a regression proof rather than an assumption.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[test]
fn cli_corpus() {
    let sandbox = tempfile::Builder::new()
        .prefix("cli-corpus-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create the CLI corpus sandbox");
    let outer = sandbox.path();
    git2::Repository::init(outer).expect("initialize the enclosing repository");

    let outer_plan_root = outer.join("outer-plan");
    std::fs::create_dir_all(outer_plan_root.join("tasks")).expect("create outer plan");
    std::fs::write(
        outer_plan_root.join("tasks/OUTER-1.md"),
        "---\nid: OUTER-1\ntitle: Enclosing work\nphase: 0\nstatus: todo\n\
         effort_days: 1\n---\n\n## Why\n\nThis claim must not enter the CLI fixtures.\n",
    )
    .expect("write outer task");
    let outer_plan = aplan_core::load::load_plan(&outer_plan_root);
    let identity = aplan_interop::identity::ResolvedIdentity {
        name: "outer-holder".to_string(),
        source: aplan_interop::identity::IdentitySource::Argument,
    };
    let outer_claim =
        aplan_app::claims::claim(&outer_plan_root, &outer_plan, "OUTER-1", &identity, None, 1)
            .expect("hold an enclosing claim");

    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cmd");
    let source_before = tree_bytes(&source);
    let corpus = outer.join("cmd");
    copy_tree(&source, &corpus);
    let corpus_before = tree_bytes(&corpus);
    assert_eq!(
        corpus_before, source_before,
        "the corpus copy changed bytes"
    );

    // Prove the adverse setup first. Without the nested repository below, the
    // fixture really does discover the enclosing claim and fail closed because
    // its plan cannot see OUTER-1. A test that seeded an inert outer file could
    // stay green even if the isolation stopped doing anything.
    let fixture_root = corpus.join("ready.in/aplan");
    let fixture_plan = aplan_core::load::load_plan(&fixture_root);
    let leaked = aplan_app::claims::snapshot(&fixture_root, &fixture_plan)
        .expect_err("the unisolated fixture must see the adverse outer claim");
    assert!(
        matches!(
            leaked,
            aplan_app::claims::ClaimError::Orphaned { ref ids } if ids.contains("OUTER-1")
        ),
        "the adverse setup failed for an unexpected reason: {leaked}"
    );

    git2::Repository::init(&corpus).expect("isolate the copied corpus in a nested repository");
    let isolated = aplan_app::claims::snapshot(&fixture_root, &fixture_plan)
        .expect("the isolated fixture has an available, empty claim scope");
    assert!(isolated.scope.available, "the nested scope is unavailable");
    assert_eq!(
        isolated.scope.kind.as_deref(),
        Some("git_common_directory"),
        "the nested scope is not backed by its Git common directory"
    );
    assert_ne!(
        isolated.scope.id, outer_claim.scope.id,
        "the nested and enclosing repositories resolved to one claim scope"
    );
    assert!(
        isolated.claims.is_empty(),
        "the test-scoped repository inherited enclosing claims"
    );

    let outer_claim_path = outer.join(".git/aplan/claims-v1.json");
    let outer_claim_before = std::fs::read(&outer_claim_path).expect("read the outer claim");
    let fixture_claim_path = corpus.join(".git/aplan/claims-v1.json");
    let fixture_claim_before = optional_bytes(&fixture_claim_path);

    let pattern = corpus.join("*.trycmd").display().to_string();
    let cases = trycmd::TestCases::new();
    cases.default_bin_name("aplan").case(pattern);
    drop(cases);

    assert_eq!(
        std::fs::read(&outer_claim_path).expect("read the preserved outer claim"),
        outer_claim_before,
        "the corpus changed real enclosing coordination state"
    );
    assert_eq!(
        optional_bytes(&fixture_claim_path),
        fixture_claim_before,
        "the corpus left coordination residue in its test-scoped repository"
    );
    assert_eq!(
        tree_bytes(&source),
        source_before,
        "the corpus changed its committed source fixtures"
    );
    assert_eq!(
        tree_bytes_without_root_git(&corpus),
        corpus_before,
        "a command changed its copied fixture or left non-Git residue"
    );
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).expect("create copied fixture directory");
    let mut entries: Vec<_> = std::fs::read_dir(source)
        .expect("read fixture directory")
        .map(|entry| entry.expect("read fixture entry"))
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let file_type = entry.file_type().expect("read fixture entry type");
        if file_type.is_dir() {
            copy_tree(&from, &to);
        } else if file_type.is_file() {
            std::fs::copy(&from, &to).expect("copy fixture file");
        } else {
            panic!("unsupported fixture entry `{}`", from.display());
        }
    }
}

fn tree_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    tree_bytes_inner(root, false)
}

fn tree_bytes_without_root_git(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    tree_bytes_inner(root, true)
}

fn tree_bytes_inner(root: &Path, skip_root_git: bool) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(
        root: &Path,
        directory: &Path,
        skip_root_git: bool,
        files: &mut BTreeMap<PathBuf, Vec<u8>>,
    ) {
        let mut entries: Vec<_> = std::fs::read_dir(directory)
            .expect("read fixture tree")
            .map(|entry| entry.expect("read fixture tree entry"))
            .collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let file_type = entry.file_type().expect("read fixture tree entry type");
            if file_type.is_dir() {
                if skip_root_git && directory == root && entry.file_name() == ".git" {
                    continue;
                }
                visit(root, &path, skip_root_git, files);
            } else if file_type.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .expect("fixture entry stays beneath its root")
                    .to_path_buf();
                files.insert(relative, std::fs::read(&path).expect("read fixture file"));
            } else {
                panic!("unsupported fixture entry `{}`", path.display());
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(root, root, skip_root_git, &mut files);
    files
}

fn optional_bytes(path: &Path) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("read `{}`: {error}", path.display()),
    }
}
