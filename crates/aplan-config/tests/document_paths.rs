//! The public, production-compiled document resolver (`A1-203`).
//!
//! Unit tests in `lib.rs` pin validation at its source. These integration tests
//! deliberately call the library as a consumer does: coverage then measures
//! the production build rather than the `cfg(test)` copy of the module.

use std::path::{Path, PathBuf};

use aplan_config::Config;

fn config(yaml: &str) -> Config {
    let config: Config = yaml_compat::from_str(yaml).expect("configuration parses");
    config.validate_docs().expect("declaration is valid");
    config
}

fn relative(base: &Path, paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .map(|path| path.strip_prefix(base).unwrap_or(path).to_path_buf())
        .collect()
}

#[test]
fn literals_globs_and_membership_form_one_stable_set() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("docs/nested")).expect("directories");
    for path in ["docs/a.md", "docs/b.md", "docs/nested/c.md", "PLAN.md"] {
        std::fs::write(dir.path().join(path), path).expect("document");
    }

    let documents = config(
        "docs:\n  paths: ['docs/*.md', 'docs/**/*.md', PLAN.md]\n  membership:\n    - doc: PLAN.md\n      declared_by: table\n",
    )
    .docs
    .expect("docs")
    .document_paths(dir.path())
    .expect("declared documents resolve");

    assert_eq!(
        relative(dir.path(), &documents),
        ["PLAN.md", "docs/a.md", "docs/b.md", "docs/nested/c.md"].map(PathBuf::from)
    );
}

#[test]
fn every_declared_entry_has_to_bind_to_a_file() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir(dir.path().join("docs")).expect("directory");

    for (yaml, wanted) in [
        ("docs:\n  paths: [missing.md]\n", "not a file"),
        ("docs:\n  paths: ['docs/*.md']\n", "matched no files"),
        ("docs:\n  paths: [docs]\n", "not a file"),
        (
            "docs:\n  membership:\n    - doc: missing.md\n      declared_by: table\n",
            "`docs.membership`",
        ),
    ] {
        let error = config(yaml)
            .docs
            .expect("docs")
            .document_paths(dir.path())
            .expect_err("a declaration over no file must be refused")
            .to_string();
        assert!(error.contains(wanted), "expected {wanted:?} in {error:?}");
    }
}

#[test]
fn a_single_star_stays_bounded_and_an_absolute_glob_still_resolves() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("nested/deeper")).expect("directories");
    std::fs::write(dir.path().join("root.md"), "root").expect("root document");
    std::fs::write(dir.path().join("nested/child.md"), "child").expect("child document");
    std::fs::write(dir.path().join("nested/deeper/grandchild.md"), "grandchild")
        .expect("grandchild document");

    let root_only = config("docs:\n  paths: ['*.md']\n")
        .docs
        .expect("docs")
        .document_paths(dir.path())
        .expect("root glob");
    assert_eq!(relative(dir.path(), &root_only), [PathBuf::from("root.md")]);

    let absolute = format!("docs:\n  paths: ['{}/**/*.md']\n", dir.path().display());
    let all = config(&absolute)
        .docs
        .expect("docs")
        .document_paths(Path::new("/unused"))
        .expect("absolute glob");
    assert_eq!(all.len(), 3);
}

#[test]
fn absolute_literals_and_unvalidated_globs_take_their_distinct_paths() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let document = dir.path().join("document.md");
    std::fs::write(&document, "document").expect("document");

    let absolute = format!(
        "docs:\n  paths: ['{}']\n  membership:\n    - doc: '{}'\n      declared_by: table\n",
        document.display(),
        document.display()
    );
    let documents = config(&absolute)
        .docs
        .expect("docs")
        .document_paths(Path::new("/unused"))
        .expect("absolute declarations");
    assert_eq!(documents, [document]);

    let unchecked: Config =
        yaml_compat::from_str("docs:\n  paths: ['[']\n").expect("configuration parses");
    let error = unchecked
        .docs
        .expect("docs")
        .document_paths(dir.path())
        .expect_err("the resolver must remain safe when a caller skipped validation")
        .to_string();
    assert!(
        error.contains("error parsing glob '['") && error.contains("unclosed character class"),
        "unexpected error: {error}"
    );
}

#[cfg(unix)]
#[test]
fn fixed_prefix_symlinks_work_but_aliases_do_not_duplicate_a_file() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir(dir.path().join("real-docs")).expect("directory");
    std::fs::write(dir.path().join("real-docs/real.md"), "document").expect("document");
    symlink("real-docs", dir.path().join("linked-docs")).expect("directory symlink");
    symlink("real-docs/real.md", dir.path().join("alias.md")).expect("file symlink");

    let through_prefix = config("docs:\n  paths: ['linked-docs/*.md']\n")
        .docs
        .expect("docs")
        .document_paths(dir.path())
        .expect("fixed symlink prefix");
    assert_eq!(
        relative(dir.path(), &through_prefix),
        [PathBuf::from("linked-docs/real.md")]
    );

    let deduplicated = config("docs:\n  paths: [real-docs/real.md, alias.md]\n")
        .docs
        .expect("docs")
        .document_paths(dir.path())
        .expect("aliases");
    assert_eq!(
        relative(dir.path(), &deduplicated),
        [PathBuf::from("alias.md")],
        "the same physical file is reported once by its stable least spelling"
    );
}

#[cfg(unix)]
#[test]
fn recursive_globs_take_file_symlinks_but_never_descend_directory_symlinks() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir(dir.path().join("real-dir")).expect("directory");
    std::fs::write(dir.path().join("real.md"), "document").expect("document");
    std::fs::write(dir.path().join("real-dir/nested.md"), "nested").expect("document");
    symlink("real.md", dir.path().join("alias.md")).expect("file symlink");
    symlink("real-dir", dir.path().join("alias-dir")).expect("directory symlink");
    symlink("absent.md", dir.path().join("dangling.md")).expect("dangling symlink");

    let documents = config("docs:\n  paths: ['**/*.md']\n")
        .docs
        .expect("docs")
        .document_paths(dir.path())
        .expect("recursive glob");
    assert_eq!(
        relative(dir.path(), &documents),
        [
            PathBuf::from("alias.md"),
            PathBuf::from("real-dir/nested.md")
        ],
        "a file alias is eligible and deduplicated, while directory and dangling symlinks stop"
    );

    let error = config("docs:\n  paths: ['missing-dir/*.md']\n")
        .docs
        .expect("docs")
        .document_paths(dir.path())
        .expect_err("a missing fixed prefix must not become a vacuous success")
        .to_string();
    assert!(
        error.contains("matched no files"),
        "unexpected error: {error}"
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_directory_refuses_the_declaration_with_its_pattern() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let unreadable = dir.path().join("docs/private");
    std::fs::create_dir_all(&unreadable).expect("directory");
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000))
        .expect("remove directory permissions");

    let result = config("docs:\n  paths: ['docs/**/*.md']\n")
        .docs
        .expect("docs")
        .document_paths(dir.path());

    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700))
        .expect("restore directory permissions for cleanup");
    let error = result
        .expect_err("an unreadable reachable directory must refuse the whole declaration")
        .to_string();
    assert!(
        error.contains("could not expand pattern `docs/**/*.md`")
            && error.contains("Permission denied"),
        "unexpected error: {error}"
    );
}
