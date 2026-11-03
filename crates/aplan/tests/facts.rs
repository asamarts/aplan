//! The `facts.json` drift gate (HLD Section 16: "generate the counts, never
//! hand-write them", after a README claimed six invariants over a table of ten).
//!
//! `facts.json` is generated from the code by `aplan::facts()`. This test pins the
//! committed file to that output, so adding a rule, a format, a command, or an
//! evidence kind without regenerating turns CI red. It runs in the normal
//! default-feature `build + test` job, so no extra CI wiring is needed. The
//! supported headless build deliberately emits a smaller capability document
//! and therefore cannot be compared with the standard build's committed file.

use crate::common;

#[cfg(feature = "tui")]
#[test]
fn facts_json_is_up_to_date() {
    let generated = format!("{}\n", aplan::facts());
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../facts.json");
    // Fail with a clear cause if the file is missing, rather than an empty string
    // that the assertion below would report as a huge, misleading "stale" diff.
    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    assert_eq!(
        on_disk, generated,
        "facts.json is stale. Regenerate it with `cargo run -p aplan -- facts > facts.json` \
         and commit the result."
    );
}

/// A name this build advertises is a name this build keeps (`A1-142`).
///
/// `--require` is a promise, and the gate above only proves the promise is
/// *printed*. The two behaviours below were added by `A1-142`'s own audit, which
/// found them shipped and unnamed; a name added after the fact is worth nothing
/// unless something fails when the behaviour goes away again. So this runs the
/// binary and asks it to demonstrate each sentence, on a plan built for the
/// purpose, rather than trusting the array.
///
/// Deliberately a pair of observations rather than one: the advertisement and
/// the behaviour, checked together, so deleting either half fails here.
#[test]
fn each_named_behaviour_is_one_this_build_demonstrates() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan dir");
    let write = |id: &str, body: &str| {
        std::fs::write(root.join(format!("aplan/tasks/{id}.md")), body).expect("task");
    };
    // A cut whose single member is done: the boundary `ready` used to answer with
    // silence, which an agent reads as "there is no work left" (`A1-105`).
    write(
        "M-s0",
        "---\nid: M-s0\ntitle: First cut\nphase: 1\nstatus: todo\ntags:\n- milestone\n\
         blocked_by:\n- A-001\n---\n\n## Why\n\nx\n",
    );
    write(
        "A-001",
        "---\nid: A-001\ntitle: Done work\nphase: 1\nstatus: done\neffort_days: 1\n\
         tags:\n- untestable\nevidence:\n  - kind: note\n    ref: it was done\n---\n\n\
         ## Why it is `untestable`\n\nA fixture.\n",
    );

    let run = |args: &[&str]| {
        let out = common::output(root, args);
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        (text, out.status.code().unwrap_or(-1))
    };

    // `ready.reports_a_closeable_cut`: advertised, and demonstrated.
    let (_, code) = run(&["facts", "--require", "ready.reports_a_closeable_cut"]);
    assert_eq!(code, 0, "the build does not advertise the name");
    let (out, _) = run(&["ready"]);
    // The sentence, not the id: the id would appear in any listing, and the
    // behaviour is that the silence is *explained*.
    assert!(
        out.contains("members are all done") && out.contains("M-s0"),
        "`ready` went silent at a cut boundary, which is the behaviour the name \
         promises is gone:\n{out}"
    );

    // `mutation.refused_when_check_would_reject`: the same pair. `dep add` would
    // make the plan cyclic, which `check` rejects (`APLAN004`), so the write is
    // refused and the file is unchanged.
    let (_, code) = run(&[
        "facts",
        "--require",
        "mutation.refused_when_check_would_reject",
    ]);
    assert_eq!(code, 0, "the build does not advertise the name");
    write(
        "A-002",
        "---\nid: A-002\ntitle: Later work\nphase: 1\nstatus: todo\neffort_days: 1\n\
         blocked_by:\n- A-001\n---\n\n## Why\n\nx\n",
    );
    let before = std::fs::read_to_string(root.join("aplan/tasks/A-001.md")).expect("read");
    let (out, code) = run(&["dep", "add", "A-001", "--after", "A-002", "--root", "aplan"]);
    assert_ne!(
        code, 0,
        "a mutation `check` would reject was written:\n{out}"
    );
    // Which refusal, and not a usage error: `dep` exits 2 for a rejected mutation
    // and clap exits 2 for a mistyped argument, so the code alone would let this
    // test pass on a command line the tool never understood -- which it did,
    // until the audit of this very test ran it by hand.
    assert!(
        out.contains("refusing") && out.contains("APLAN004"),
        "the command failed for some other reason than the rules:\n{out}"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("aplan/tasks/A-001.md")).expect("read"),
        before,
        "the refusal wrote the mutation anyway"
    );
}
