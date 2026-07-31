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

#[test]
fn cli_corpus() {
    trycmd::TestCases::new()
        .default_bin_name("aplan")
        .case("tests/cmd/*.trycmd");
}
