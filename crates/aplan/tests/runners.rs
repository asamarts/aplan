//! `test` evidence against runners that are not cargo (`A1-034`).
//!
//! Every claim that this tool is language agnostic was an inference from the
//! shape of `.aplan.yml`: `test.list` and `test.run` are argv templates, so
//! anything *should* work. For a tool whose thesis is that an unverified claim
//! is not a proof, that was an uncomfortable place to leave it.
//!
//! **The claim was checked against the real programs before this file existed.**
//! `pytest 9.1.1` and `vitest 4.1.11`, each driving a real `aplan verify` and
//! `aplan verify --execute` over a real plan. What those runs found is recorded
//! in `A1-034`'s body; the bytes below are theirs, copied from the terminal, and
//! this file is the regression half.
//!
//! **Why the runners here are `printf` and `sh` rather than the real tools.**
//! `ci/Containerfile` installs a Rust toolchain, git and a C toolchain, and says
//! in as many words that it carries no Node, because aplan's own gates are pure
//! cargo (the pre-public repository-boundary decision). It installs no Python either. Adding two language runtimes
//! to that image to gate somebody else's runner is a large price for a small
//! test, and `sh` and `printf` are there already. What can be
//! made hermetic is the part that is actually this tool's: the *shapes* a runner
//! answers in, and what aplan does with them. A test that ran real pytest would
//! prove pytest works, which was never in doubt.

use std::path::Path;
use std::process::Command;

mod common;
use common::aplan;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("running git")
        .status
        .success();
    assert!(ok, "git {args:?} failed");
}

/// A repository with one `done` task citing `test_ref`, and `.aplan.yml` as given.
fn repo(config: &str, test_ref: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::create_dir_all(dir.path().join("tests")).expect("tests dir");
    std::fs::write(dir.path().join("tests/suite"), "x").expect("file");
    std::fs::write(
        dir.path().join("aplan/tasks/T-001.md"),
        format!(
            "---\nid: T-001\ntitle: proved by a test\nphase: 1\nstatus: done\n\
             paths:\n  - 'tests/**'\nevidence:\n  - kind: test\n    ref: \"{test_ref}\"\n---\n"
        ),
    )
    .expect("task");
    std::fs::write(dir.path().join(".aplan.yml"), config).expect("config");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "t@t"]);
    git(dir.path(), &["config", "user.name", "t"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "init"]);
    dir
}

/// `pytest --collect-only -q`, byte for byte, from pytest 9.1.1.
///
/// Three properties this shape has and cargo's does not: a node id carries `::`
/// and a path, a parametrised id carries **spaces, commas and brackets**, and the
/// listing ends with a blank line and a summary sentence.
const PYTEST_LISTING: &str = "tests/test_math.py::test_add\n\
     tests/test_math.py::test_subtract\n\
     tests/test_math.py::TestEdges::test_zero\n\
     tests/test_strings.py::test_parametrised[a b]\n\
     tests/test_strings.py::test_parametrised[c,d]\n\
     \n\
     5 tests collected in 0.01s\n";

/// `vitest list`, byte for byte, from vitest 4.1.11. No summary, and the name is
/// a ` > `-joined path through file, suite and test.
const VITEST_LISTING: &str = "tests/math.test.js > adds\n\
     tests/math.test.js > edges > handles zero\n\
     tests/math.test.js > edges > a name, with a comma\n";

/// A `test.list` that prints `text` and nothing else.
fn lister(text: &str) -> String {
    format!("[\"printf\", \"%s\", {}]", serde_json::json!(text))
}

/// A plain `verify` asks the runner what it lists, without `--execute`
/// (`A1-176`).
///
/// `--execute` is documented as the flag that runs your tests, which is easy to
/// read as "without it, nothing is run". Resolving a `test` citation means
/// asking each runner what it lists, so `test.list` is invoked either way -- and
/// for a cargo runner that *compiles*, which is what decides where an adopter's
/// plan gate can live and what it costs. The pilot designed a standalone CI job
/// around the opposite belief.
///
/// This is the guard on that paragraph in `docs/upgrading.md`: the claim is
/// about behaviour, so it is pinned by behaviour rather than by a promise not to
/// edit the file.
#[test]
fn a_plain_verify_asks_the_runner_what_it_lists() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::create_dir_all(dir.path().join("tests")).expect("tests dir");
    std::fs::write(dir.path().join("tests/suite"), "x").expect("file");
    std::fs::write(
        dir.path().join("aplan/tasks/T-001.md"),
        "---{n}id: T-001{n}title: proved by a test{n}phase: 1{n}status: done{n}\
         paths:{n}  - 'tests/**'{n}evidence:{n}  - kind: test{n}    ref: known{n}---{n}"
            .replace("{n}", "\n"),
    )
    .expect("task");
    // The listing writes a marker, so "it was asked" is a fact on disk rather
    // than an inference from the answer.
    let marker = dir.path().join("asked.log");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        format!(
            "test:{n}  name: cargo-shaped{n}  list: [\"sh\", \"-c\", \"echo x >> {} ; printf 'known{bs}{bs}n'\"]{n}  run: [\"true\", \"{{test}}\"]{n}",
            marker.display(),
            n = "\n",
            bs = "{BS}",
        )
        .replace("{BS}", "\\"),
    )
    .expect("config");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "t@t"]);
    git(dir.path(), &["config", "user.name", "t"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "init"]);

    let (out, code) = aplan(dir.path(), &["verify"]);
    assert_eq!(code, 0, "{out}");
    let asked = std::fs::read_to_string(&marker).unwrap_or_default();
    assert_eq!(
        asked.lines().count(),
        1,
        "`test.list` runs on a plain `verify`, once: {out}"
    );

    // And a listing that *fails* is reported on a plain `verify` too, which is
    // how a build error reaches an adopter who never passed `--execute`.
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "test:\n  name: cargo-shaped\n  list: [\"sh\", \"-c\", \"echo 'error[E0308]: mismatched types' >&2; exit 101\"]\n  run: [\"true\", \"{test}\"]\n",
    )
    .expect("config");
    let (out, code) = aplan(dir.path(), &["verify"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("APLAN013"), "{out}");
    assert!(
        out.contains("could not be asked what it lists") && out.contains("exited 101"),
        "the build error reaches the reader without `--execute`: {out}"
    );
}

/// A pytest-shaped node id resolves, and the parametrised one -- spaces, commas
/// and brackets in the name -- resolves too.
#[test]
fn a_pytest_node_id_is_a_name_this_tool_can_resolve() {
    for name in [
        "tests/test_math.py::test_add",
        "tests/test_math.py::TestEdges::test_zero",
        "tests/test_strings.py::test_parametrised[a b]",
        "tests/test_strings.py::test_parametrised[c,d]",
    ] {
        let config = format!(
            "test:\n  name: pytest\n  list: {}\n  run: [\"true\", \"{{test}}\"]\n",
            lister(PYTEST_LISTING)
        );
        let dir = repo(&config, name);
        let (out, code) = aplan(dir.path(), &["verify"]);
        assert_eq!(code, 0, "`{name}` did not resolve:\n{out}");
    }
}

/// A vitest name is a ` > `-joined path, and resolves as written.
#[test]
fn a_vitest_name_is_a_name_this_tool_can_resolve() {
    for name in [
        "tests/math.test.js > adds",
        "tests/math.test.js > edges > a name, with a comma",
    ] {
        let config = format!(
            "test:\n  name: vitest\n  list: {}\n  run: [\"true\", \"{{test}}\"]\n",
            lister(VITEST_LISTING)
        );
        let dir = repo(&config, name);
        let (out, code) = aplan(dir.path(), &["verify"]);
        assert_eq!(code, 0, "`{name}` did not resolve:\n{out}");
    }
}

/// The citation reaches the runner as **one** argument, whatever is in it.
///
/// The property `.aplan.yml` already enforces on the template -- `{test}` is a
/// whole argument, never spliced into a larger string -- is only worth anything
/// if the value arrives that way too. A pytest parametrised id is the case that
/// tells: it holds a space and a comma, so a runner receiving it as two
/// arguments would be selecting a test nobody cited.
///
/// **Both halves, because one of them proves nothing on its own.** A runner that
/// demands one argument exits `0`, and so does a run that never happened -- the
/// audit of this task found exactly that, by dropping `--execute` and watching
/// the assertion still pass. The second fixture demands *two* arguments and so
/// must fail; together they say the command ran and got what it should.
#[test]
fn a_name_holding_a_space_arrives_as_one_argument() {
    let name = "tests/test_strings.py::test_parametrised[a b]";
    for (argc, code, needle) in [(1, 0, "evidence resolved"), (2, 1, "APLAN014")] {
        let config = format!(
            "test:\n  name: pytest\n  list: {}\n  \
             run: [\"sh\", \"-c\", \"[ $# -eq {argc} ] && [ \\\"$1\\\" = \\\"{name}\\\" ]\", \"sh\", \"{{test}}\"]\n",
            lister(PYTEST_LISTING)
        );
        let dir = repo(&config, name);
        let (out, got) = aplan(dir.path(), &["verify", "--execute"]);
        assert_eq!(
            got, code,
            "expected {argc} argument(s) to be {code}:\n{out}"
        );
        assert!(out.contains(needle), "{out}");
    }
}

/// `--execute` against a pytest-shaped runner, both ways round.
#[test]
fn a_pytest_shaped_run_is_executed_and_its_failure_is_reported() {
    let name = "tests/test_math.py::test_add";
    for (run, code, needle) in [
        ("[\"true\", \"{test}\"]", 0, "evidence resolved"),
        ("[\"false\", \"{test}\"]", 1, "APLAN014"),
    ] {
        let config = format!(
            "test:\n  name: pytest\n  list: {}\n  run: {run}\n",
            lister(PYTEST_LISTING)
        );
        let dir = repo(&config, name);
        let (out, got) = aplan(dir.path(), &["verify", "--execute"]);
        assert_eq!(got, code, "{out}");
        assert!(out.contains(needle), "{out}");
    }
}

/// **A runner that collects nothing looks like a runner with one test.**
///
/// `pytest --collect-only -q` prints `no tests collected in 0.00s` and exits 0
/// when it finds nothing, so the listing is one line and this tool reads it as a
/// test name. A misconfigured runner -- wrong `dir`, wrong path, tests not found
/// -- is then indistinguishable from a working one, and every citation against it
/// is reported as a name the runner does not list, which sends the reader to fix
/// the citation rather than the configuration.
///
/// Pinned rather than fixed, and `A1-064` owns it: it is that task's subject
/// exactly, arriving from a second runner. What this adds to that task is the
/// sting -- pytest gives no way to suppress the line, and a fix that asks whether
/// a runner listed *nothing* will not fire here, because this runner listed one
/// thing. Asserting the current behaviour means the fix has a test that must
/// change rather than one that quietly still passes.
#[test]
fn a_runner_that_collected_nothing_still_lists_its_own_summary() {
    let config = format!(
        "test:\n  name: pytest\n  list: {}\n  run: [\"true\", \"{{test}}\"]\n",
        lister("no tests collected in 0.00s\n")
    );
    // The summary sentence is a name this tool will resolve a citation against,
    // which is what "the listing has one entry in it" means from outside.
    let named = repo(&config, "no tests collected in 0.00s");
    let (out, code) = aplan(named.path(), &["verify"]);
    assert_eq!(
        code, 0,
        "the summary line did not resolve as a test:\n{out}"
    );

    // And a real citation is blamed for the runner having found nothing, which
    // is the same message a runner that listed truly nothing produces.
    let real = repo(&config, "tests/test_math.py::test_add");
    let (out, code) = aplan(real.path(), &["verify"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("is not one the configured runner lists"),
        "the citation is blamed, not the configuration:\n{out}"
    );
}

/// **`--execute` reports `ok` for a test that never ran.**
///
/// The assumption underneath the ladder's top rung: the name a runner *lists* is
/// the name that *selects* it. cargo and pytest honour it; vitest does not --
/// `vitest list` prints `file > suite > test` and `vitest run -t` matches the
/// test's own title, so the listed name selects nothing, every test is skipped,
/// and the exit status is 0. Measured against vitest 4.1.11: a deliberately
/// failing test, cited and executed, verified clean.
///
/// The runner below is that mechanism rather than a stand-in for it. It matches
/// its argument against the *titles* the way `-t` does, and the title `adds`
/// fails; so a citation naming the title runs a failing test and is reported,
/// and the same test cited by the name the listing prints selects nothing and is
/// reported as passing. A fixture that merely exited `0` would assert the same
/// thing while demonstrating none of it -- the audit of this task replaced one.
///
/// Not fixable by exit status: `cargo test -- --exact <no such test>` also exits
/// 0, so a runner that exits 0 on a miss is not evidence of anything -- it is
/// what a correctly configured cargo runner does. `A1-167` asks the runner what
/// a *miss* looks like instead, and this test now pins that: the run is still
/// reported as passing, because the tool cannot make a skipped test fail, and
/// `APLAN037` says the pass proves nothing.
#[test]
fn a_run_that_selects_nothing_is_reported_as_proving_nothing() {
    // `vitest run -t <pattern>`, in nine lines: the pattern is matched against
    // each test's own title, and `adds` is the failing one.
    // Faithful to what vitest prints, not only to what it exits: a miss says
    // `Tests  3 skipped (3)` and a hit says `1 passed`. That difference is the
    // only thing separating them, which is the whole subject of `APLAN037`.
    let selector = "hit=0; for t in 'adds' 'handles zero' 'a name, with a comma'; do \
                    case \\\"$t\\\" in *\\\"$1\\\"*) hit=1; \
                    [ \\\"$t\\\" = adds ] && { echo 'Tests  1 failed (3)'; exit 1; } ;; \
                    esac; done; \
                    [ $hit = 1 ] && echo 'Tests  1 passed (3)' || echo 'Tests  3 skipped (3)'; \
                    exit 0";
    let config = format!(
        "test:\n  name: vitest\n  list: {}\n  run: [\"sh\", \"-c\", \"{selector}\", \"sh\", \"{{test}}\"]\n",
        lister(VITEST_LISTING)
    );

    // First, that the runner can fail at all. A listing of the bare titles is the
    // workaround available to an adopter -- it is what `-t` selects by -- and
    // under it the failing test is cited, run, and reported.
    let titles = format!(
        "test:\n  name: vitest\n  list: {}\n  run: [\"sh\", \"-c\", \"{selector}\", \"sh\", \"{{test}}\"]\n",
        lister("adds\nhandles zero\na name, with a comma\n")
    );
    let by_title = repo(&titles, "adds");
    let (out, code) = aplan(by_title.path(), &["verify", "--execute"]);
    assert_eq!(
        code, 1,
        "the runner never fails, so it proves nothing:\n{out}"
    );
    assert!(out.contains("APLAN014"), "{out}");

    // Then the same failing test, cited by the name the listing prints. Nothing
    // is selected and the exit status is 0, so `APLAN014` cannot see it -- and
    // the run is reported as proving nothing rather than as a pass.
    let by_listed_name = repo(&config, "tests/math.test.js > adds");
    let (out, code) = aplan(by_listed_name.path(), &["verify", "--execute"]);
    assert_eq!(
        code, 0,
        "a warning does not fail the build by itself:\n{out}"
    );
    assert!(
        out.contains("APLAN037"),
        "the false green must be reported:\n{out}"
    );
    assert!(
        out.contains("did not prove the test ran"),
        "and say what it means:\n{out}"
    );

    // The claim the whole rule rests on: the *same* runner, cited by the name it
    // really selects, is silent. Without this the rule could be firing on
    // `--execute` itself rather than on the pairing.
    let by_title = repo(&titles, "handles zero");
    let (out, code) = aplan(by_title.path(), &["verify", "--execute"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !out.contains("APLAN037"),
        "a test that really ran must not be reported:\n{out}"
    );
}
