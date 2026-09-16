//! The wiring: a spawn through `common` reaches the recorder (`A1-207`).
//!
//! One test, in a file of its own, because it redirects the recording for the
//! whole test binary. Everything else about the recorder is unit-tested in
//! `spawn_helper.rs`; what only an end-to-end run can show is that [`common::run`]
//! calls it at all -- a recorder that formats a line perfectly and is never
//! reached is the shape of defect this module exists to remove.

use std::path::PathBuf;

mod common;

#[test]
fn a_spawn_through_the_helper_reaches_the_recorder() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");

    let log: PathBuf = dir.path().join("invocations.jsonl");

    // Before anything is asked for, nothing is written. The sibling test in
    // `spawn_helper.rs` can only show that the fixture gained no files; this
    // shows that the very path a recording *would* go to stays absent, which is
    // the property "silent unless asked" actually means (`A1-207`, audit).
    common::run(dir.path(), &["summary"]);
    assert!(
        !log.exists(),
        "a run wrote a recording before one was asked for"
    );

    common::record_into(log.clone());

    common::run(dir.path(), &["summary"]);
    common::output(dir.path(), &["check", "--format", "json"]);
    common::command(dir.path(), &["ready"])
        .output()
        .expect("running aplan");

    let text = std::fs::read_to_string(&log).expect("the spawns were recorded");
    let seen: Vec<Vec<String>> = text
        .lines()
        .map(|l| serde_json::from_str(l).expect("each line is JSON"))
        .collect();
    assert_eq!(
        seen,
        vec![
            vec!["summary".to_string()],
            vec![
                "check".to_string(),
                "--format".to_string(),
                "json".to_string()
            ],
            vec!["ready".to_string()],
        ],
        "every entry point records, in the order it ran: {text:?}"
    );
}
