//! Helpers shared by the integration tests in this crate.
//!
//! A `tests/common/mod.rs` is compiled into each test binary that declares it
//! rather than becoming a target of its own. It exists so a helper used by two
//! suites has one implementation: `argv_of` is used by the README-invocation gate
//! in `docs.rs` (`A1-092`) and by the diagnostic-invocation gate in
//! `consumer_facing.rs` (`A1-089`), and two copies of a parser is precisely the
//! shape of defect those gates were written to catch.
//!
//! [`run`] is the other one, and the reason is the same at a larger scale: forty
//! eight files spawned the binary and thirty-six of them defined a helper to do
//! it, twenty-five byte-identical (`A1-207`). What made that worth fixing beyond
//! the duplication is that there was **nowhere to stand**: `A1-208`, `A1-209` and
//! `A1-211` all need to know what the suite actually invoked, and for a Rust
//! test the only honest source is the spawn itself. Measuring it from the
//! sources was tried and failed twice -- counting command names counts the word
//! `check` in prose, and attributing flags by position misses `--config`, which
//! `trusted_config.rs` passes *before* the subcommand.
//!
//! **This is one of the suite's two halves.** `cli.rs` hands
//! `tests/cmd/*.trycmd` to `trycmd`, which spawns the binary itself: 126 further
//! invocations that pass through nothing here and are invisible to the
//! recording. They need no observing, being declared in the corpus files
//! verbatim, but an audit of the CLI surface has to read both and say so
//! (`A1-207`, audit).
//!
//! Every item here is `allow(dead_code)`: this module is compiled into each test
//! binary that declares it, so a file using `run` and not `argv_of` would
//! otherwise warn about the one it did not need.
#![allow(dead_code)]

use std::io::Write as _;
use std::path::Path;
use std::process::Command;

/// The environment variable naming the file invocations are recorded to.
///
/// Absent by default, so an ordinary `cargo test` spawns exactly as it did
/// before this module existed.
pub const RECORD_TO: &str = "APLAN_TEST_INVOCATIONS";

/// What one run of the binary produced.
#[derive(Debug, Clone)]
pub struct Run {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

impl Run {
    /// Stdout followed by stderr.
    ///
    /// The shape twenty-five callers wrote by hand, and the right default for a
    /// test asking "did it say this": a refusal's reason goes to stderr while
    /// its report goes to stdout, so a test that read only one of them would
    /// pass on a message printed to the other.
    #[must_use]
    pub fn text(&self) -> String {
        let mut text = self.stdout.clone();
        text.push_str(&self.stderr);
        text
    }

    /// Whether the run exited clean (`0`). Not `success()`: `1` means findings,
    /// which is a normal outcome for `check` (ADR-0007).
    #[must_use]
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

/// A recorded [`Command`] for the binary, for the callers that have to shape the
/// process itself.
///
/// An MCP test talks to a long-lived child over piped stdin and stdout, and
/// `help_wraps.rs` sets `COLUMNS`; neither can be expressed as "spawn and give
/// me the output". Handing those callers a `Command` rather than exempting them
/// is what lets the gate below be absolute: no integration test names the binary
/// itself, so there is no spawn the recording cannot see.
///
/// The invocation is recorded here, when the arguments are known. A caller that
/// adds more with `.arg()` afterwards records less than it ran -- which is why
/// the argument vector belongs in this call.
pub fn command(dir: &Path, args: &[&str]) -> Command {
    record(args);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_aplan"));
    cmd.args(args).current_dir(dir);
    cmd
}

/// Run one `git` command for a fixture, fail closed, and never leave the
/// fixture (`A1-257`).
///
/// Two guards, because either alone has been watched fail:
///
/// **The exit status is checked.** `Command::output().expect(..)` proves only
/// that `git` *started*. When a fixture's `git init` failed under a full `/tmp`,
/// the `git add` and `git commit` after it ran anyway -- and having no
/// repository of their own, they found the checkout the fixture was sitting in.
/// A linked worktree's reflog recorded two fixture commits, its index was
/// reduced to the fixture's four files, and all 268 real tasks read as
/// untracked.
///
/// **And the walk is stopped at the fixture.** `GIT_CEILING_DIRECTORIES` means
/// even a caller who ignores a status, or a `git` run before the check, cannot
/// reach an ancestor: it fails with "not a git repository" instead. That matters
/// more since fixtures moved to `<target>/tmp`, which is *inside* the checkout
/// unless `CARGO_TARGET_DIR` says otherwise -- `/tmp` at least had no repository
/// above it.
pub fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        // The fixture *root*, not this directory: git refuses to `chdir` up
        // into a listed path, so naming the fixture itself would still let the
        // walk pass through its parent. Naming `<target>/tmp` stops every
        // fixture at the tree they all share, and the checkout is above it.
        .env(
            "GIT_CEILING_DIRECTORIES",
            aplan_core::fixture::fixture_root(),
        )
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} in {} failed, and a fixture that carries on from here runs \
         the rest of its setup against whatever repository it can find: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// Spawn the binary and hand back what the process produced, unmodified.
///
/// The primitive the rest of this module is built on, exposed because a caller
/// that already works in terms of [`std::process::Output`] should not have to be
/// rewritten to be recorded. Every spawn in the suite goes through here or
/// through something that calls it, which is what makes the recording complete
/// rather than merely widespread.
pub fn output(dir: &Path, args: &[&str]) -> std::process::Output {
    command(dir, args).output().expect("running aplan")
}

/// Spawn the binary. **The one place an integration test starts a process.**
///
/// It spawns, records, and returns what the caller already got. It deliberately
/// asserts nothing: a helper that asserts is a helper every caller fights, and
/// the callers that do want an assertion keep their own thin wrapper over this
/// one.
pub fn run(dir: &Path, args: &[&str]) -> Run {
    let out = output(dir, args);
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code().unwrap_or(-1),
    }
}

/// [`run`], as the pair twenty-five files spelled out: combined output and exit
/// code.
pub fn aplan(dir: &Path, args: &[&str]) -> (String, i32) {
    let out = run(dir, args);
    (out.text(), out.code)
}

/// Append one invocation to the recording file, when one is asked for.
///
/// One `write_all` of a whole line to a file opened `O_APPEND`, because the test
/// binaries run in parallel threads and across processes: two appends of a
/// single short buffer do not interleave, two `write!`s of the same line can.
/// JSON rather than a joined string, because an argument may contain a space --
/// `aplan add "Ship it"` is in the corpus.
///
/// A recording that cannot be written is silently skipped. This is measurement,
/// not a gate: failing a hundred tests because a path was wrong would tell the
/// reader nothing about the code under test.
fn record(args: &[&str]) {
    if let Some(path) = SINK.lock().ok().and_then(|s| s.clone()) {
        record_to(&path, args);
        return;
    }
    let Ok(path) = std::env::var(RECORD_TO) else {
        return;
    };
    record_to(Path::new(&path), args);
}

/// Where this test binary records, when something other than the environment
/// decides.
static SINK: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// Record into `path` for the rest of this test binary.
///
/// It exists for one test: the one proving that [`run`] reaches [`record_to`] at
/// all. Without it the wiring is untested -- `record_to` can be perfect and
/// never called, which is a hole of exactly the kind this module was built to
/// close. Setting the environment variable instead is not open to a test here,
/// because `set_var` is `unsafe` and this workspace forbids unsafe code.
///
/// Process-wide, so its one caller lives in a test file of its own.
pub fn record_into(path: std::path::PathBuf) {
    if let Ok(mut sink) = SINK.lock() {
        *sink = Some(path);
    }
}

/// The half that writes, separated from the half that reads the environment so
/// it can be tested without a test setting a variable for every other thread in
/// its process.
pub fn record_to(path: &Path, args: &[&str]) {
    let Ok(mut line) = serde_json::to_string(args) else {
        return;
    };
    line.push('\n');
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Split a documented invocation into argv, honouring the single and double
/// quotes the examples use (`aplan add "Ship it" --paths 'src/**'`) and dropping
/// a trailing `# comment`.
pub fn argv_of(line: &str) -> Vec<String> {
    let line = match line.find(" #") {
        Some(i) => &line[..i],
        None => line,
    };
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, ' ') => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (None, _) => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

/// Run `args` with stdout attached to a pty, recorded like any other spawn.
///
/// Two files had their own copy, and they had drifted apart in the way that
/// matters: one hard-fails when `script(1)` is missing under `CI` -- because a
/// width gate that skips reports green having asserted nothing -- and the other
/// printed "skipping" and returned. The stricter reading is the one that
/// survives (`A1-207`).
/// Run `args` with stdout attached to a pty of `cols` columns, and return what
/// the terminal would have shown, with styling stripped.
///
/// Returns `None` when `script(1)` is unavailable, so the suite still runs on a
/// machine without util-linux rather than failing for the wrong reason.
pub fn on_a_terminal(dir: &Path, cols: usize, args: &[&str]) -> Option<String> {
    on_a_terminal_raw(dir, cols, &[], args).map(|text| strip(&text))
}

/// [`on_a_terminal`] with the styling left in, and an environment.
///
/// The stripping variant above cannot answer "did this emit colour", which is
/// the whole question `A1-286` asks of a terminal: it removes the evidence
/// before the caller sees it. `env` is here for the same reason -- `NO_COLOR`
/// and `CLICOLOR_FORCE` are half the contract and are only observable at a
/// terminal, where the default is otherwise to colour.
///
/// Carriage returns are still dropped: a pty writes them and no assertion in
/// this workspace is about them.
pub fn on_a_terminal_raw(
    dir: &Path,
    cols: usize,
    env: &[(&str, &str)],
    args: &[&str],
) -> Option<String> {
    on_a_terminal_full(dir, cols, env, args, false)
}

/// [`on_a_terminal_raw`], with stderr sent to `/dev/null` instead of the pty.
///
/// **The one configuration that distinguishes the two streams.** `script(1)`
/// attaches a pty to stdout *and* stderr, so `fold::out_width()` and
/// `fold::err_width()` return the same number and a renderer folding against the
/// wrong one is invisible -- which is how four commands came to fold their
/// stdout against stderr and pass every width test (`A1-291`). Here stderr has
/// no terminal, so `err_width()` answers "do not fold" and the mistake shows.
pub fn on_a_terminal_stderr_piped(dir: &Path, cols: usize, args: &[&str]) -> Option<String> {
    on_a_terminal_full(dir, cols, &[], args, true).map(|text| strip(&text))
}

fn on_a_terminal_full(
    dir: &Path,
    cols: usize,
    env: &[(&str, &str)],
    args: &[&str],
    stderr_piped: bool,
) -> Option<String> {
    let binary = env!("CARGO_BIN_EXE_aplan");
    let quoted: Vec<String> = args.iter().map(|a| format!("'{a}'")).collect();
    let redirect = if stderr_piped { " 2>/dev/null" } else { "" };
    let inner = format!(
        "stty rows 24 cols {cols}; {binary} {}{redirect}",
        quoted.join(" ")
    );

    let mut command = Command::new("script");
    command
        .args(["-qc", &inner, "/dev/null"])
        .current_dir(dir)
        .envs(env.iter().copied());
    let out = command.output();
    let out = match out {
        Ok(out) => out,
        // A gate that can skip is not a gate. On a contributor's machine
        // without util-linux, skipping beats failing for the wrong reason; in
        // CI it would mean this file reports green having asserted nothing, so
        // there it is a hard failure.
        Err(e) => {
            assert!(
                std::env::var_os("CI").is_none(),
                "script(1) is missing in CI, so the width gate would pass vacuously: {e}"
            );
            return None;
        }
    };
    // After the spawn, not before it. `script(1)` may be missing, and a record
    // saying the binary was invoked when it never started is worse than no
    // record at all -- an audit reading it would count coverage that did not
    // happen (`A1-207`, audit).
    record(args);
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).replace('\r', ""))
}

/// Remove the ANSI a terminal consumes, and the carriage returns a pty adds.
fn strip(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {}
            '\u{1b}' => {
                if chars.next() == Some('[') {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
            }
            c => out.push(c),
        }
    }
    out
}
