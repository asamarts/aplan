//! `aplan init` writes where it is told (`A1-208`).
//!
//! The corpus pins the guard -- that it refuses to overwrite a policy the
//! repository already has -- and nothing else could live there: a writing
//! command run inside a `.trycmd` fixture edits the fixture, so the case would
//! pass once and then be comparing against its own output.
//!
//! What was left uncovered is the argument. `[DIR]` is optional, so it is a
//! second behaviour behind the same command name, and the surface audit counted
//! neither kind of positional until it was taught to. Nothing in the suite ever
//! passed one.

mod common;

/// **The directory named is the one written to**, and the current one is left
/// alone.
///
/// Both halves matter. A command that wrote to both would satisfy "the file is
/// there", and a scaffolder that quietly drops a policy file into whatever
/// directory you happen to be standing in is the kind of thing an adopter finds
/// out about from `git status`.
#[test]
fn the_directory_named_is_the_one_scaffolded() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let here = dir.path().join("here");
    let there = dir.path().join("there");
    std::fs::create_dir_all(&here).expect("here");
    std::fs::create_dir_all(&there).expect("there");

    let out = common::run(&here, &["init", there.to_str().expect("utf-8")]);
    assert_eq!(out.code, 0, "{out:?}");

    assert!(
        there.join(".aplan.yml").is_file(),
        "`init <dir>` did not write into the directory it was given: {}",
        out.text()
    );
    assert!(
        !here.join(".aplan.yml").exists(),
        "`init <dir>` also wrote into the current directory: {}",
        out.text()
    );

    // And the scaffold is one the tool accepts, which is the only claim worth
    // making about its content here: `aplan-config` owns what the keys mean.
    let check = common::run(&there, &["check"]);
    assert!(
        !check.text().contains("could not"),
        "the scaffolded policy does not load: {}",
        check.text()
    );
}

/// Without the argument it writes beside you, which is the default the help
/// documents and the behaviour every other test depends on.
#[test]
fn without_the_argument_it_writes_into_the_current_directory() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let out = common::run(dir.path(), &["init"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        dir.path().join(".aplan.yml").is_file(),
        "`init` wrote nothing where it stood: {}",
        out.text()
    );
}
