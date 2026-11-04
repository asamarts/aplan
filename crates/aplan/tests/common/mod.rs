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

pub mod census;
pub mod sources;

use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Send one signal without relying on `PATH` or an external `kill` executable.
///
/// A minimal POSIX environment may provide `kill` only as a shell builtin.
/// Signal and pid are positional parameters,
/// not interpolated shell source, so the helper cannot turn either into code.
#[cfg(unix)]
pub fn send_signal(pid: u32, signal: &str) {
    let status = Command::new("/bin/sh")
        .args([
            "-c",
            "kill \"-$1\" \"$2\"",
            "aplan-test-signal",
            signal,
            &pid.to_string(),
        ])
        .env("PATH", "")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("POSIX shell runs");
    assert!(
        status.success(),
        "kill -{signal} {pid} failed through /bin/sh: {status}"
    );
}

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

/// One immutable copy of the binary an integration-test process exercises.
///
/// Cargo gives every integration test the same path in `target/debug`, and a
/// rebuild may atomically replace that path while the workspace suite is still
/// running. Holding the path therefore does not hold the artifact. The copy is
/// kept for the process lifetime so two commands in one test cannot silently
/// execute builds from opposite sides of a concurrent commit (`A1-242`).
pub struct BinarySnapshot {
    path: PathBuf,
    #[cfg(not(unix))]
    _directory: tempfile::TempDir,
}

/// Old binary snapshots outlive any test process that could still name them.
const KEEP_BINARY_SNAPSHOTS_FOR: std::time::Duration = std::time::Duration::from_secs(60 * 60);

impl BinarySnapshot {
    /// Hold the inode at `source` under one cache path shared by this build.
    ///
    /// A hard link is the snapshot: Cargo may replace its path, but it cannot
    /// change the inode already linked here. Keying the cache by device and
    /// inode makes every integration-test process reuse the same link instead
    /// of allocating another roughly 200 MB instrumented executable. The
    /// cache is inside Cargo's target tree, so coverage cleanup and `cargo
    /// clean` remove it; an age sweep bounds ordinary rebuilds (`A1-399`).
    #[must_use]
    pub fn capture(source: &Path) -> Self {
        #[cfg(unix)]
        {
            Self::capture_unix(source)
        }
        #[cfg(not(unix))]
        {
            Self::capture_portable(source)
        }
    }

    #[cfg(unix)]
    fn capture_unix(source: &Path) -> Self {
        let root = source
            .parent()
            .expect("the binary has a parent directory")
            .join(".aplan-test-binaries");
        std::fs::create_dir_all(&root).expect("create the binary snapshot cache");
        let staging = tempfile::Builder::new()
            .prefix("aplan-binary-stage-")
            .tempdir_in(&root)
            .expect("create the binary snapshot staging directory");
        let name = source.file_name().expect("the binary has a file name");
        let staged = staging.path().join(name);
        std::fs::hard_link(source, &staged).expect("hard-link the binary under test");

        let metadata = staged
            .metadata()
            .expect("read the binary snapshot metadata");
        let dir = root.join(format!(
            "aplan-binary-{}-{}",
            metadata.dev(),
            metadata.ino()
        ));
        std::fs::create_dir_all(&dir).expect("create the stable binary snapshot directory");
        let path = dir.join(name);
        match std::fs::hard_link(&staged, &path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = path
                    .metadata()
                    .expect("read the shared binary snapshot metadata");
                assert_eq!(
                    (existing.dev(), existing.ino()),
                    (metadata.dev(), metadata.ino()),
                    "the cache path for one binary inode names another inode"
                );
            }
            Err(error) => panic!("cache the binary snapshot: {error}"),
        }
        clear_old_binary_snapshots(&root, &dir);
        Self { path }
    }

    /// Windows does not expose Unix device/inode keys, and an executable may
    /// not be hard-linked uniformly across its supported filesystems. One
    /// process-private copy still holds the exact artifact for the lifetime of
    /// this integration harness; the portability matrix has one consolidated
    /// harness, so it neither duplicates the binary nor races Cargo replacing
    /// the source path (`A1-448`).
    #[cfg(not(unix))]
    fn capture_portable(source: &Path) -> Self {
        let root = source
            .parent()
            .expect("the binary has a parent directory")
            .join(".aplan-test-binaries");
        std::fs::create_dir_all(&root).expect("create the binary snapshot cache");
        let directory = tempfile::Builder::new()
            .prefix("aplan-binary-portable-")
            .tempdir_in(&root)
            .expect("create the portable binary snapshot directory");
        let path = directory
            .path()
            .join(source.file_name().expect("the binary has a file name"));
        std::fs::copy(source, &path).expect("copy the binary under test");
        clear_old_binary_snapshots(&root, directory.path());
        Self {
            path,
            _directory: directory,
        }
    }

    /// The immutable executable path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Retire builds old enough that no integration-test process can still need
/// one. A new build may start while an old suite is running -- the race this
/// cache exists for -- so freshness, not merely being a different inode, is
/// the safety boundary.
fn clear_old_binary_snapshots(root: &Path, current: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let cutoff = std::time::SystemTime::now() - KEEP_BINARY_SNAPSHOTS_FOR;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_snapshot = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("aplan-binary-"));
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified < cutoff);
        if path != current && is_snapshot && old {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

/// Prefer a runner's relocatable executable path over Cargo's build-time path.
///
/// Nextest rewrites both runtime variables when it extracts an archive. The
/// compile-time value remains the fallback for test harnesses that do not
/// provide either runtime contract (`A1-616`).
fn select_binary_source(
    nextest: Option<&std::ffi::OsStr>,
    cargo: Option<&std::ffi::OsStr>,
    compiled: &Path,
) -> PathBuf {
    nextest
        .filter(|path| !path.is_empty())
        .or(cargo)
        .filter(|path| !path.is_empty())
        .map_or_else(|| compiled.to_owned(), PathBuf::from)
}

/// The binary this integration-test process exercises, resolved once.
#[must_use]
pub fn binary() -> &'static Path {
    static BINARY: OnceLock<BinarySnapshot> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            let nextest = std::env::var_os("NEXTEST_BIN_EXE_aplan");
            let cargo = std::env::var_os("CARGO_BIN_EXE_aplan");
            let source = select_binary_source(
                nextest.as_deref(),
                cargo.as_deref(),
                Path::new(env!("CARGO_BIN_EXE_aplan")),
            );
            BinarySnapshot::capture(&source)
        })
        .path()
}

#[test]
fn the_binary_snapshot_prefers_the_runners_relocatable_path() {
    use std::ffi::OsStr;

    let compiled = Path::new("/build-host/target/debug/aplan");
    assert_eq!(
        select_binary_source(
            Some(OsStr::new("/archive/target/debug/aplan")),
            Some(OsStr::new("/cargo/target/debug/aplan")),
            compiled,
        ),
        Path::new("/archive/target/debug/aplan")
    );
    assert_eq!(
        select_binary_source(
            None,
            Some(OsStr::new("/cargo/target/debug/aplan")),
            compiled,
        ),
        Path::new("/cargo/target/debug/aplan")
    );
    assert_eq!(select_binary_source(None, None, compiled), compiled);
    assert_eq!(
        select_binary_source(Some(OsStr::new("")), Some(OsStr::new("")), compiled),
        compiled
    );
}

/// A complete copy of this repository's plan outside its Git coordination scope.
///
/// A repository-self test wants the plan bytes committed beside it, but not the
/// mutable claim store shared by every linked worktree. A detached worktree can
/// otherwise see a claim for a task introduced later and correctly refuse that
/// orphan, making the test depend on unrelated work happening at the same time.
/// Copying rather than linking gives the test one source snapshot even if an
/// editor replaces or rewrites a task while the suite is running.
pub struct WorkspacePlanFixture {
    directory: tempfile::TempDir,
    source_commit: git2::Oid,
}

impl WorkspacePlanFixture {
    /// Directory from which the CLI discovers the copied `aplan/` root.
    #[must_use]
    pub fn directory(&self) -> &Path {
        self.directory.path()
    }

    /// The copied plan root.
    #[must_use]
    pub fn plan_root(&self) -> PathBuf {
        self.directory.path().join("aplan")
    }

    /// The one immutable tree from which every fixture byte was read.
    #[must_use]
    pub fn source_commit(&self) -> git2::Oid {
        self.source_commit
    }
}

fn copy_committed_tree(repository: &git2::Repository, tree: &git2::Tree<'_>, destination: &Path) {
    std::fs::create_dir_all(destination).expect("create committed plan fixture directory");
    for entry in tree {
        let name = std::str::from_utf8(entry.name_bytes()).unwrap_or_else(|_| {
            panic!(
                "committed plan tree entry {} has a non-UTF-8 name",
                entry.id()
            )
        });
        assert!(
            !name.is_empty()
                && name != "."
                && name != ".."
                && !name.contains('/')
                && !name.contains('\\'),
            "committed plan tree entry {} has unsafe name {name:?}",
            entry.id()
        );
        let path = destination.join(name);
        match (entry.kind(), entry.filemode()) {
            (Some(git2::ObjectType::Tree), 0o040_000) => {
                let child = repository.find_tree(entry.id()).unwrap_or_else(|error| {
                    panic!("read committed plan tree {}: {error}", entry.id())
                });
                copy_committed_tree(repository, &child, &path);
            }
            (Some(git2::ObjectType::Blob), 0o100_644 | 0o100_755) => {
                let blob = repository.find_blob(entry.id()).unwrap_or_else(|error| {
                    panic!("read committed plan blob {}: {error}", entry.id())
                });
                std::fs::write(&path, blob.content()).unwrap_or_else(|error| {
                    panic!("write committed plan fixture {}: {error}", path.display())
                });
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    let mode = entry.filemode();
                    let permissions = std::fs::Permissions::from_mode(mode.cast_unsigned() & 0o777);
                    std::fs::set_permissions(&path, permissions).unwrap_or_else(|error| {
                        panic!(
                            "set committed plan fixture mode on {}: {error}",
                            path.display()
                        )
                    });
                }
            }
            (kind, mode) => panic!(
                "committed plan entry `{}` ({}) has unsupported kind {kind:?} and mode {mode:o}",
                path.display(),
                entry.id()
            ),
        }
    }
}

/// Snapshot one committed plan tree after calling `after_resolve` with its id.
///
/// The hook exists for the regression that moves `HEAD` after resolution. The
/// production helper below supplies a no-op, while both paths continue reading
/// immutable Git objects from the already-resolved commit.
#[must_use]
pub fn committed_plan_fixture_from(
    repository_root: &Path,
    after_resolve: impl FnOnce(git2::Oid),
) -> WorkspacePlanFixture {
    let repository = git2::Repository::discover(repository_root).unwrap_or_else(|error| {
        panic!(
            "discover repository containing committed plan at {}: {error}",
            repository_root.display()
        )
    });
    let commit = repository
        .head()
        .and_then(|head| head.peel_to_commit())
        .unwrap_or_else(|error| {
            panic!(
                "resolve one committed HEAD for plan fixture at {}: {error}",
                repository_root.display()
            )
        });
    let source_commit = commit.id();
    let root = commit.tree().unwrap_or_else(|error| {
        panic!("read tree for plan fixture commit {source_commit}: {error}")
    });
    let plan_entry = root
        .get_name("aplan")
        .unwrap_or_else(|| panic!("commit {source_commit} has no `aplan/` tree"));
    assert_eq!(
        plan_entry.kind(),
        Some(git2::ObjectType::Tree),
        "commit {source_commit} contains `aplan` as {:?}, not a tree",
        plan_entry.kind()
    );
    let plan = repository
        .find_tree(plan_entry.id())
        .unwrap_or_else(|error| {
            panic!(
                "read `aplan/` tree {} from commit {source_commit}: {error}",
                plan_entry.id()
            )
        });

    after_resolve(source_commit);

    let directory = tempfile::Builder::new()
        .prefix("workspace-plan-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create committed plan fixture");
    let destination = directory.path().join("aplan");
    copy_committed_tree(&repository, &plan, &destination);

    let claims = aplan_app::claims::snapshot(&destination, &aplan_core::Plan::default())
        .expect("inspect committed plan fixture coordination");
    assert!(
        !claims.scope.available,
        "committed plan fixture must be outside a Git common directory"
    );
    WorkspacePlanFixture {
        directory,
        source_commit,
    }
}

/// Snapshot this repository's committed plan into a standalone fixture.
#[must_use]
pub fn workspace_plan_fixture() -> WorkspacePlanFixture {
    committed_plan_fixture_from(&sources::workspace_root(), |_| {})
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
    let mut cmd = Command::new(binary());
    cmd.args(args).current_dir(dir);
    colour_baseline(&mut cmd);
    cmd
}

/// Start the binary in a real PTY, through the same immutable and recorded
/// boundary as every pipe-backed integration test.
///
/// The environment is deliberately empty except for the terminal identity
/// supplied by `termlens`. A PTY journey must pass `--no-config` when it does
/// not intend to exercise repository configuration, rather than inheriting a
/// developer's environment and turning it into an undeclared test input.
// The workspace gate runs every real-terminal journey beside CPU- and process-heavy suites.
// This bounds transport and teardown under that contention; narrower product deadlines stay local.
const TUI_TRANSPORT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub fn tui_terminal(
    dir: &Path,
    columns: u16,
    rows: u16,
    args: &[&str],
) -> termlens::Result<termlens::Terminal> {
    tui_terminal_with_environment(dir, columns, rows, args, &[])
}

/// Start the binary in a real PTY with an explicit portable environment.
///
/// This is the common recorded process boundary for journeys that exercise an
/// editor or provider subprocess. It remains deliberately narrower than an
/// arbitrary builder: the environment is the only additional input (`A1-448`).
pub fn tui_terminal_with_environment(
    dir: &Path,
    columns: u16,
    rows: u16,
    args: &[&str],
    environment: &[(String, String)],
) -> termlens::Result<termlens::Terminal> {
    record(args);
    termlens::Terminal::builder()
        .size(columns, rows)
        .env_clear()
        .envs(environment.iter().map(|(key, value)| (key, value)))
        .timeout(TUI_TRANSPORT_TIMEOUT)
        .args(args)
        .current_dir(dir)
        .spawn(binary())
}

/// Start the TUI with an isolated disposable cache root.
///
/// Performance tests need to distinguish a forced cache refresh from a warm
/// cache hit without reading or mutating the account running the test. The
/// ordinary PTY helper intentionally clears the environment; this narrower
/// variant restores only `XDG_CACHE_HOME` (`A1-447`).
pub fn tui_terminal_with_cache(
    dir: &Path,
    cache: &Path,
    columns: u16,
    rows: u16,
    args: &[&str],
) -> termlens::Result<termlens::Terminal> {
    record(args);
    termlens::Terminal::builder()
        .size(columns, rows)
        .env_clear()
        .env("XDG_CACHE_HOME", cache)
        .timeout(TUI_TRANSPORT_TIMEOUT)
        .args(args)
        .current_dir(dir)
        .spawn(binary())
}

/// Start the TUI after recording the PTY's exact termios baseline and changing
/// one otherwise non-default flag. The shell `exec`s aplan, so the pid exposed
/// by termlens remains the process that receives lifecycle signals.
#[cfg(unix)]
pub fn tui_terminal_with_termios_baseline(
    dir: &Path,
    columns: u16,
    rows: u16,
    args: &[&str],
    baseline: &Path,
) -> termlens::Result<termlens::Terminal> {
    record(args);
    let mut shell_args = vec![
        "-c".to_string(),
        "stty -echoctl; stty -g > \"$1\"; shift; exec \"$@\"".to_string(),
        "aplan-tui-wrapper".to_string(),
        baseline.display().to_string(),
        binary().display().to_string(),
    ];
    shell_args.extend(args.iter().map(|arg| (*arg).to_string()));
    termlens::Terminal::builder()
        .size(columns, rows)
        .env_clear()
        .timeout(TUI_TRANSPORT_TIMEOUT)
        .args(shell_args)
        .current_dir(dir)
        .spawn("/bin/sh")
}

/// Run one non-interactive command behind util-linux `script`, recording its
/// exact termios state immediately before and after the binary. The command's
/// stdin is closed so long-lived stdio servers receive EOF rather than owning
/// the test PTY indefinitely.
#[cfg(unix)]
pub fn on_terminal_with_state(dir: &Path, args: &[&str]) -> std::process::Output {
    record(args);
    let mut invocation = shell_word(&binary().display().to_string());
    for arg in args {
        invocation.push(' ');
        invocation.push_str(&shell_word(arg));
    }
    let command = format!(
        "stty -echoctl cols 120 rows 40; \
         before=$(stty -g 2>/dev/null) || before=STTY_FAILED; \
         {invocation} </dev/null; code=$?; \
         after=$(stty -g 2>/dev/null) || after=STTY_FAILED; \
         printf '\\n__APLAN_TERMIOS__|%s|%s|%s\\n' \"$before\" \"$after\" \"$code\""
    );
    Command::new("script")
        .args(["-qfec", &command, "/dev/null"])
        .current_dir(dir)
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE")
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .output()
        .expect("run aplan in a terminal")
}

#[cfg(unix)]
fn shell_word(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// Give colour-sensitive fixtures one known parent environment (`A1-346`).
///
/// A contributor's `NO_COLOR` is a valid preference for commands they run. It
/// is not an input to an integration test unless that test supplies it: the
/// colour suite asserts both sides of the environment contract and its default
/// terminal cases otherwise change meaning under the caller's shell profile.
/// `TERM=dumb` matters independently of `NO_COLOR`, so merely removing the
/// three policy variables is not enough.
///
/// Callers deliberately retain the last word. [`command`] returns the prepared
/// value for a test to extend, and the pty launcher applies its explicit `env`
/// after this function.
fn colour_baseline(command: &mut Command) {
    command
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE")
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor");
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
    if let Some(path) = SINK.with(|sink| sink.borrow().clone()) {
        record_to(&path, args);
        return;
    }
    let Ok(path) = std::env::var(RECORD_TO) else {
        return;
    };
    record_to(Path::new(&path), args);
}

thread_local! {
    /// Where this test thread records, when something other than the environment
    /// decides. A thread-local scope cannot capture concurrent tests in the merged
    /// integration harness (`A1-630`).
    static SINK: std::cell::RefCell<Option<std::path::PathBuf>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Restores the calling test thread's prior recording destination on drop.
#[derive(Debug)]
#[must_use]
pub struct RecordingScope {
    previous: Option<std::path::PathBuf>,
    _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Drop for RecordingScope {
    fn drop(&mut self) {
        SINK.with(|sink| {
            sink.replace(self.previous.take());
        });
    }
}

/// Record into `path` on this test thread until the returned scope is dropped.
///
/// It exists for one test: the one proving that [`run`] reaches [`record_to`] at
/// all. Without it the wiring is untested -- `record_to` can be perfect and
/// never called, which is a hole of exactly the kind this module was built to
/// close. Setting the environment variable instead is not open to a test here,
/// because `set_var` is `unsafe` and this workspace forbids unsafe code.
///
/// Thread-local and scoped so the merged integration harness can run unrelated
/// tests concurrently without redirecting their invocations (`A1-630`).
pub fn record_into(path: std::path::PathBuf) -> RecordingScope {
    let previous = SINK.with(|sink| sink.replace(Some(path)));
    RecordingScope {
        previous,
        _not_send: std::marker::PhantomData,
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
    on_a_terminal_raw(dir, cols, &[], args).map(|text| terminal_text(&text))
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
    on_a_terminal_full(dir, cols, &[], args, true).map(|text| terminal_text(&text))
}

fn on_a_terminal_full(
    dir: &Path,
    cols: usize,
    env: &[(&str, &str)],
    args: &[&str],
    stderr_piped: bool,
) -> Option<String> {
    let binary = binary().display();
    let quoted: Vec<String> = args.iter().map(|a| format!("'{a}'")).collect();
    let redirect = if stderr_piped { " 2>/dev/null" } else { "" };
    let inner = format!(
        "stty rows 24 cols {cols}; {binary} {}{redirect}",
        quoted.join(" ")
    );

    let mut command = Command::new("script");
    colour_baseline(&mut command);
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

/// Reduce terminal presentation bytes to the text left on screen.
///
/// Most controls in ordinary command output are SGR styling and disappear.
/// Attended progress additionally owns `CSI 2 K`: merely deleting that escape
/// would concatenate the transient row with the result that replaced it and
/// make width/readability tests judge text a person never sees. Apply that one
/// erase operation to the current output row before returning the plain text.
pub fn terminal_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {}
            '\u{1b}' => {
                if chars.next() == Some('[') {
                    let mut control = String::new();
                    for c in chars.by_ref() {
                        control.push(c);
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                    if control == "2K" {
                        let row = out.rfind('\n').map_or(0, |newline| newline + 1);
                        out.truncate(row);
                    }
                }
            }
            c => out.push(c),
        }
    }
    out
}
