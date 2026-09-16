//! An explicit, trusted configuration boundary (`A1-155`).
//!
//! `.aplan.yml` names the programs a `test` or `url` citation runs, and it was
//! discovered by walking up from the plan root or the repository. That is right
//! for your own repository and wrong when a trusted binary reviews somebody
//! else's checkout: the fifth pilot supplied its own binary and working
//! directory, pointed `--root` and `--repo` at a candidate tree, and watched the
//! candidate's config select the program that ran (pre-public adoption evidence).
//!
//! These tests are the hostile fixture: a candidate config whose runner writes a
//! marker, and a trusted config that does not. The marker file is the assertion
//! -- if it exists, the boundary leaked.

use std::path::{Path, PathBuf};
use std::process::Command;

mod common;
use common::aplan;

#[test]
fn discovery_still_reads_the_tree_being_looked_at() {
    // The behaviour this task bounds rather than removes: with no selection,
    // discovery works as it always has, which is what a person in their own
    // repository wants. Recorded so the fix cannot be mistaken for a change to
    // the default.
    let (dir, marker) = hostile(None);
    let (out, code) = aplan(&dir, &["--root", "aplan", "verify"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        marker.exists(),
        "discovery reads the candidate's config: {out}"
    );
}

/// Proves the named behaviour `config.explicitly_selectable` (`A1-211`).
#[test]
fn a_named_config_is_the_only_one_consulted() {
    let (dir, marker) = hostile(None);
    let trusted = dir.join("trusted.yml");
    std::fs::write(&trusted, "root: aplan\n").expect("trusted config");

    let trusted_arg = trusted.display().to_string();
    let (out, code) = aplan(
        &dir,
        &["--root", "aplan", "--config", &trusted_arg, "verify"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        !marker.exists(),
        "the candidate's config selected a program anyway: {out}"
    );
}

/// Proves the named behaviour `config.explicitly_selectable` (`A1-211`).
#[test]
fn no_config_runs_nothing_and_says_so() {
    let (dir, marker) = hostile(None);
    let (out, code) = aplan(&dir, &["--root", "aplan", "--no-config", "verify"]);
    assert_eq!(code, 0, "{out}");
    assert!(!marker.exists(), "{out}");
    // Deliberate is not the same as silent: the run says the repository's own
    // policy was not applied and that `test`/`url` citations went unchecked
    // rather than passing.
    assert!(out.contains("--no-config"), "{out}");
    assert!(out.contains("was not applied"), "{out}");
    assert!(out.contains("unchecked rather than passing"), "{out}");
}

/// A typo must not degrade to the tree under review.
/// Proves the named behaviour `config.explicitly_selectable` (`A1-211`).
#[test]
fn a_named_config_that_is_missing_is_an_error_not_a_fallback() {
    let (dir, marker) = hostile(None);
    let absent = dir.join("no-such-file.yml").display().to_string();
    let (out, code) = aplan(&dir, &["--root", "aplan", "--config", &absent, "verify"]);
    assert_eq!(code, 2, "a missing selection is a usage error: {out}");
    assert!(!marker.exists(), "and nothing ran: {out}");
    // The message names the file the caller chose, not `.aplan.yml`, which is
    // the file they had deliberately not used.
    assert!(out.contains("--config"), "{out}");
    assert!(out.contains("no-such-file.yml"), "{out}");
}

/// Every command that can run a configured program, and each one proves it can.
///
/// The first version of this test listed seven commands and asserted the marker
/// was absent after each -- but six of them (`check`, `ready`, `summary`,
/// `roadmap`, `stale`, `tasks`) never resolve evidence, so they cannot run a
/// program under *any* configuration and their assertions passed for free. A
/// test that cannot fail is worse than no test, because it is counted
/// (`A1-155`, found by its audit; the same shape `A1-020`'s audit found).
///
/// So each command here is run twice: once under discovery, which must leak,
/// and once under the selection, which must not.
#[test]
fn every_command_that_can_run_a_program_is_stopped_by_the_selection() {
    for args in [
        vec!["verify"],
        vec!["show", "C-01"],
        vec!["close", "C-02", "--evidence", "test:it_ran"],
        vec!["attest"],
    ] {
        let (dir, marker) = hostile(None);
        let mut discovery = vec!["--root", "aplan"];
        discovery.extend(args.iter().copied());
        let (out, _) = aplan(&dir, &discovery);
        assert!(
            marker.exists(),
            "{args:?} cannot run a program even under discovery, so asserting it does not is \
             asserting nothing: {out}"
        );

        let (dir, marker) = hostile(None);
        let trusted = dir.join("trusted.yml");
        std::fs::write(&trusted, "root: aplan\n").expect("trusted config");
        let trusted_arg = trusted.display().to_string();
        let mut selected = vec!["--root", "aplan", "--config", &trusted_arg];
        selected.extend(args.iter().copied());
        let (out, _) = aplan(&dir, &selected);
        assert!(
            !marker.exists(),
            "{args:?} let the candidate's config select a program: {out}"
        );
    }
}

/// The commands that read configuration without resolving evidence still honour
/// the selection, which is checked by what the *policy* does rather than by a
/// marker they could never write.
#[test]
fn a_read_view_uses_the_selected_policy_not_the_candidates() {
    let (dir, _) = hostile(None);
    // The candidate's config denies warnings; the trusted one does not.
    let candidate = std::fs::read_to_string(dir.join(".aplan.yml")).expect("read");
    std::fs::write(
        dir.join(".aplan.yml"),
        format!("{candidate}deny_warnings: true\nrequire_effort: true\n"),
    )
    .expect("write");
    // A task with no estimate: `APLAN009` warns, and the candidate's policy would
    // make that fatal.
    std::fs::write(
        dir.join("aplan/tasks/C-03.md"),
        "---\nid: C-03\ntitle: no estimate\nphase: 1\nstatus: todo\n---\n\nbody\n",
    )
    .expect("task");

    let (out, code) = aplan(&dir, &["--root", "aplan", "check"]);
    assert_eq!(code, 1, "the candidate's `deny_warnings` blocks: {out}");

    let trusted = dir.join("trusted.yml");
    std::fs::write(&trusted, "root: aplan\n").expect("trusted config");
    let trusted_arg = trusted.display().to_string();
    let (out, code) = aplan(
        &dir,
        &["--root", "aplan", "--config", &trusted_arg, "check"],
    );
    assert_eq!(code, 0, "the selected policy is the one applied: {out}");
}

/// A refused configuration is announced by *every* command, including the ones
/// that write: `close` under `--no-config` records a citation nothing checked,
/// and the first version said so only in `check` and `verify` (`A1-155`, its
/// audit).
#[test]
fn refusing_configuration_is_announced_wherever_it_is_read() {
    for args in [
        vec!["check"],
        vec!["verify"],
        vec!["close", "C-02", "--evidence", "test:it_ran"],
        vec!["ready"],
    ] {
        let (dir, _) = hostile(None);
        let mut full = vec!["--root", "aplan", "--no-config"];
        full.extend(args.iter().copied());
        let (out, _) = aplan(&dir, &full);
        assert!(
            out.contains("--no-config: no configuration was read"),
            "{args:?} read a refused config and said nothing: {out}"
        );
        assert_eq!(
            out.matches("--no-config: no configuration was read")
                .count(),
            1,
            "{args:?} said it more than once: {out}"
        );
    }
}

/// Relative values in a selected config resolve against *its* directory, so a
/// trusted policy can live outside the tree it judges.
#[test]
fn a_selected_configs_relative_values_resolve_from_its_own_directory() {
    let (dir, _) = hostile(None);
    let outside = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    // The trusted config names its own traces root, relative to itself.
    std::fs::create_dir_all(outside.path().join("docs")).expect("mkdir");
    std::fs::write(
        outside.path().join("docs/spec.md"),
        "# Spec\n\n## One\n\nx\n",
    )
    .expect("doc");
    std::fs::write(
        outside.path().join("trusted.yml"),
        "root: aplan\ntraces_root: .\n",
    )
    .expect("config");

    let selected = outside.path().join("trusted.yml").display().to_string();
    let (out, code) = aplan(&dir, &["--root", "aplan", "--config", &selected, "check"]);
    assert!(code <= 1, "{out}");
    // The proof that the *selected* file's directory was used: a task tracing
    // `docs/spec.md#one` resolves against the trusted tree, where that document
    // exists, rather than against the candidate, where it does not.
    std::fs::write(
        dir.join("aplan/tasks/C-02.md"),
        "---\nid: C-02\ntitle: traced\nphase: 1\nstatus: todo\neffort_days: 1\n\
         traces:\n- docs/spec.md#one\n---\n\nbody\n",
    )
    .expect("task");
    let (out, code) = aplan(&dir, &["--root", "aplan", "--config", &selected, "check"]);
    assert!(
        code <= 1,
        "the trace resolved against the trusted tree: {out}"
    );
    assert!(!out.contains("APLAN008"), "{out}");
}

/// The agent surface is the other half of "every CLI or MCP path", and it also
/// had to learn that a named config says where *values* resolve and nothing
/// about where the plan's git history is (`A1-155`, found by its audit: with a
/// trusted config outside the tree, the server resolved evidence against the
/// config's directory and answered "not a git repository").
#[test]
fn the_mcp_surface_honours_the_selection_and_still_finds_the_repository() {
    let (dir, marker) = hostile(Some("MCP-MARKER"));
    let outside = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::write(outside.path().join("trusted.yml"), "root: aplan\n").expect("config");
    let selected = outside.path().join("trusted.yml").display().to_string();

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
    let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"aplan_verify","arguments":{}}}"#;
    let requests = format!("{init}\n{call}\n");

    let out = mcp(
        &dir,
        &["--root", "aplan", "--config", &selected, "mcp"],
        &requests,
    );
    assert!(
        !marker.exists(),
        "the candidate's config ran a program: {out}"
    );
    assert!(
        !out.contains("not a git repository"),
        "the plan's own repository is where evidence resolves: {out}"
    );
    // The tool's answer is a JSON document inside a JSON string, so the quotes
    // arrive escaped; the field name is what matters.
    assert!(out.contains("tasks_checked"), "and it answered: {out}");
}

/// Structural: no command may reach around the selection.
///
/// The boundary is only as good as its narrowest gap, and the gap would be one
/// `cmd_*` calling the discovering loader directly. `config_at` is the only
/// reader, and this holds that line the way `aplan-e2e` holds the hermetic one.
#[test]
fn no_call_site_bypasses_the_selection() {
    let source = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
        .expect("read the CLI source");

    let offenders: Vec<(usize, &str)> = source
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("aplan_config::load_config_located"))
        // The doc comment on `CONFIG_SOURCE` names it deliberately.
        .filter(|(_, line)| !line.trim_start().starts_with("///"))
        .map(|(n, line)| (n + 1, line.trim()))
        .collect();
    assert!(
        offenders.is_empty(),
        "these call sites bypass `--config`/`--no-config`; route them through `config_at`:\n  {}",
        offenders
            .iter()
            .map(|(n, l)| format!("line {n}: {l}"))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// A candidate checkout whose `.aplan.yml` runs a program, and the marker that
/// program writes. Returns the checkout and the marker path.
fn hostile(marker_name: Option<&str>) -> (PathBuf, PathBuf) {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    // Leaked deliberately: the test asserts on a path inside it after the
    // command has run, and `TempDir` would remove it at the end of the scope.
    let dir = dir.keep();
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    let marker = dir.join(marker_name.unwrap_or("MARKER"));

    std::fs::write(
        dir.join(".aplan.yml"),
        format!(
            "test:\n  - name: candidate\n    list: [sh, -c, \"touch {}; echo it_ran\"]\n    \
             run: [sh, -c, \"echo\", \"{{test}}\"]\n",
            marker.display()
        ),
    )
    .expect("candidate config");
    std::fs::write(
        dir.join("aplan/tasks/C-01.md"),
        "---\nid: C-01\ntitle: candidate task\nphase: 1\nstatus: done\neffort_days: 1\n\
         evidence:\n  - kind: test\n    ref: it_ran\n---\n\nbody\n",
    )
    .expect("task");
    // A second, open task: `close` resolves the citation it is given, which is
    // one of the ways a candidate's config gets to run a program.
    std::fs::write(
        dir.join("aplan/tasks/C-02.md"),
        "---\nid: C-02\ntitle: still open\nphase: 1\nstatus: todo\neffort_days: 1\n\
         ---\n\nbody\n",
    )
    .expect("task");

    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.email", "t@example.com"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "candidate"]);
    (dir, marker)
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("running git");
    assert!(out.status.success(), "git {args:?} failed");
}

/// Drive the MCP server over stdio with a canned request stream.
fn mcp(dir: &Path, args: &[&str], requests: &str) -> String {
    use std::io::Write as _;
    let mut child = common::command(dir, args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawning the server");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(requests.as_bytes())
        .expect("write");
    let out = child.wait_with_output().expect("the server exits");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}
