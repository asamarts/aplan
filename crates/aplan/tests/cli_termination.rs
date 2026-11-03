//! Process-level configured-command termination coverage (`A1-592`).

#![cfg(target_os = "linux")]

use std::os::unix::process::ExitStatusExt as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::common;

fn fixture() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("termination fixture");
    let root = fixture.path();
    std::fs::create_dir_all(root.join("plan/tasks")).expect("plan directory");
    std::fs::create_dir(root.join("src")).expect("source directory");
    let status = Command::new("mkfifo")
        .arg(root.join("provider.block"))
        .status()
        .expect("create provider fifo");
    assert!(status.success(), "mkfifo failed: {status}");
    std::fs::write(root.join("src/lib.rs"), "pub fn proof() {}\n").expect("source");
    std::fs::write(
        root.join("plan/tasks/T-001.md"),
        "---\nid: T-001\ntitle: Signal-owned evidence\nphase: 1\nstatus: done\n\
         effort_days: 1\npaths:\n- src/**\nevidence:\n- kind: test\n  ref: suite::proof\n\
         ---\n\n## Why\n\nExercise the configured runner.\n",
    )
    .expect("task");

    let runner = root.join("list.sh");
    std::fs::write(
        &runner,
        "#!/bin/sh\n\
         set -eu\n\
         trap '' HUP INT QUIT TERM\n\
         printf '%s\\n' \"$$\" > provider.pid\n\
         printf 'suite::proof\\n'\n\
         /bin/cat provider.block &\n\
         printf '%s\\n' \"$!\" > provider.descendant.pid\n\
         : > provider.started\n\
         wait\n",
    )
    .expect("blocking provider");
    let mut permissions = runner.metadata().expect("provider metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&runner, permissions).expect("provider executable");
    let wrapper = root.join("list-wrapper.sh");
    std::fs::write(
        &wrapper,
        "#!/bin/sh\n\
         printf '%s\\n' \"$$\" > provider.wrapper.pid\n\
         ./list.sh 2> provider.stderr\n\
         status=$?\n\
         printf '%s\\n' \"$status\" > provider.exit\n\
         exit \"$status\"\n",
    )
    .expect("provider status wrapper");
    let mut permissions = wrapper.metadata().expect("wrapper metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&wrapper, permissions).expect("wrapper executable");
    std::fs::write(
        root.join(".aplan.yml"),
        "root: plan\ntest:\n  list: [./list-wrapper.sh]\n  run: ['true', '{test}']\n",
    )
    .expect("configuration");

    common::git(root, &["init", "-q", "."]);
    common::git(root, &["config", "user.email", "test@example.com"]);
    common::git(root, &["config", "user.name", "test"]);
    common::git(root, &["add", "-A"]);
    common::git(root, &["commit", "-q", "-m", "fixture"]);
    fixture
}

const PROVIDER_READY_POLL_ATTEMPTS: usize = 6_000;
const PROVIDER_READY_POLL_PAUSE: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProcessIdentity {
    pid: u32,
    parent_pid: u32,
    start_time: u64,
    state: char,
}

fn process_identity(pid: u32) -> Result<ProcessIdentity, String> {
    let path = format!("/proc/{pid}/stat");
    let stat = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot inspect process {pid} through {path}: {error}"))?;
    // The command name is parenthesized and may itself contain spaces or `)`, so
    // split at the final delimiter before the fixed-position fields.
    let (_, fields) = stat
        .rsplit_once(") ")
        .ok_or_else(|| format!("process {pid} has a malformed {path}"))?;
    let fields = fields.split_whitespace().collect::<Vec<_>>();
    let state = fields
        .first()
        .and_then(|value| value.chars().next())
        .ok_or_else(|| format!("process {pid} has no state in {path}"))?;
    let parent_pid = fields
        .get(1)
        .ok_or_else(|| format!("process {pid} has no parent in {path}"))?
        .parse::<u32>()
        .map_err(|error| format!("process {pid} has an invalid parent in {path}: {error}"))?;
    let start_time = fields
        .get(19)
        .ok_or_else(|| format!("process {pid} has no start time in {path}"))?
        .parse::<u64>()
        .map_err(|error| format!("process {pid} has an invalid start time in {path}: {error}"))?;
    Ok(ProcessIdentity {
        pid,
        parent_pid,
        start_time,
        state,
    })
}

fn published_child_process(path: &Path, parent_pid: u32) -> Result<ProcessIdentity, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|error| format!("{} cannot be read: {error}", path.display()))?;
    let pid = contents.trim().parse::<u32>().map_err(|error| {
        format!(
            "{} does not contain a complete PID: {error}",
            path.display()
        )
    })?;
    let identity = process_identity(pid).map_err(|error| {
        format!(
            "{} names a process that is not live: {error}",
            path.display()
        )
    })?;
    if matches!(identity.state, 'Z' | 'X') {
        return Err(format!(
            "{} names PID {pid}, which is in non-live state {}",
            path.display(),
            identity.state
        ));
    }
    if identity.parent_pid != parent_pid {
        return Err(format!(
            "{} names PID {pid}, whose parent is {} rather than expected process {parent_pid}",
            path.display(),
            identity.parent_pid
        ));
    }
    Ok(identity)
}

fn published_provider_failure(root: &Path) -> Option<String> {
    let path = root.join("provider.exit");
    let contents = std::fs::read_to_string(&path).ok()?;
    let status = contents.trim().parse::<i32>().ok()?;
    let stderr = std::fs::read_to_string(root.join("provider.stderr"))
        .unwrap_or_else(|error| format!("<provider stderr unavailable: {error}>"));
    Some(format!(
        "provider exited before readiness with status {status}\nstderr:\n{stderr}"
    ))
}

fn wait_for_provider_ready(
    mut child: Child,
    root: &Path,
) -> Result<(Child, ProcessIdentity, ProcessIdentity, ProcessIdentity), String> {
    let started = root.join("provider.started");
    let wrapper_path = root.join("provider.wrapper.pid");
    let provider_path = root.join("provider.pid");
    let descendant_path = root.join("provider.descendant.pid");
    let mut last_observation = "the readiness marker is absent".to_string();

    for _ in 0..PROVIDER_READY_POLL_ATTEMPTS {
        if started.is_file() {
            match published_child_process(&wrapper_path, child.id()).and_then(|wrapper| {
                published_child_process(&provider_path, wrapper.pid).and_then(|provider| {
                    published_child_process(&descendant_path, provider.pid)
                        .map(|descendant| (wrapper, provider, descendant))
                })
            }) {
                Ok((wrapper, provider, descendant)) => {
                    return Ok((child, wrapper, provider, descendant));
                }
                Err(error) => last_observation = error,
            }
        } else if started.exists() {
            last_observation = format!("{} is not a regular file", started.display());
        }

        if let Some(error) = published_provider_failure(root) {
            if child
                .try_wait()
                .map_err(|failure| format!("{error}\nfailed to inspect aplan: {failure}"))?
                .is_none()
            {
                common::send_signal(child.id(), "TERM");
            }
            let cleanup = child
                .wait_with_output()
                .map_err(|cleanup| format!("{error}\nfailed to reap aplan: {cleanup}"))?;
            return Err(format!(
                "{error}\naplan cleanup status: {}\nstdout:\n{}\nstderr:\n{}",
                cleanup.status,
                String::from_utf8_lossy(&cleanup.stdout),
                String::from_utf8_lossy(&cleanup.stderr)
            ));
        }

        if child
            .try_wait()
            .map_err(|error| format!("checking aplan before provider readiness: {error}"))?
            .is_some()
        {
            let output = child
                .wait_with_output()
                .map_err(|error| format!("collecting aplan's early exit: {error}"))?;
            return Err(format!(
                "aplan exited before provider readiness with {} ({last_observation})\n\
                 stdout:\n{}\nstderr:\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        std::thread::sleep(PROVIDER_READY_POLL_PAUSE);
    }

    if child
        .try_wait()
        .map_err(|error| format!("checking aplan after provider readiness exhaustion: {error}"))?
        .is_none()
    {
        common::send_signal(child.id(), "TERM");
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("stopping aplan after provider readiness exhaustion: {error}"))?;
    Err(format!(
        "provider readiness was not published after {PROVIDER_READY_POLL_ATTEMPTS} scheduling \
         opportunities ({last_observation}); cleanup produced {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn wait_for_process_exit(label: &str, identity: ProcessIdentity) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while process_identity(identity.pid)
        .is_ok_and(|current| current.start_time == identity.start_time)
    {
        assert!(
            Instant::now() < deadline,
            "configured {label} {} survived CLI termination",
            identity.pid
        );
        std::thread::yield_now();
    }
}

/// Proof for `cli.configured_commands_follow_termination_signals` and
/// `progress.cli_renderer_is_identity_safe_and_terminal_safe`: every
/// catchable termination signal reaches an ordinary `show` while its configured
/// runner has printed into a pipe and then become uncooperative. The runner
/// group disappears and the parent observes the original signal, not exit 0 or
/// a synthesized `128 + signal` status.
#[test]
fn noninteractive_signals_stop_configured_groups_and_preserve_the_signal() {
    use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};

    for signal in [SIGINT, SIGTERM, SIGHUP, SIGQUIT] {
        let fixture = fixture();
        let root = fixture.path();
        let mut process = common::command(
            root,
            &[
                "--color",
                "never",
                "--progress",
                "always",
                "--root",
                "plan",
                "show",
                "T-001",
                "--refresh-evidence",
            ],
        );
        process
            .env("PATH", "")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = process.spawn().expect("spawn aplan show");
        let aplan_pid = child.id();
        let (child, wrapper_pid, provider_pid, descendant_pid) =
            wait_for_provider_ready(child, root).unwrap_or_else(|error| {
                panic!("configured provider did not become ready: {error}")
            });

        // Cross the progress grace threshold while the provider is known to be
        // blocked. Redirected stderr selects the portable newline renderer, so
        // terminating the process cannot leave a cursor mode or partial row.
        std::thread::sleep(Duration::from_millis(250));

        let sent = Instant::now();
        common::send_signal(aplan_pid, &signal.to_string());
        let output = child.wait_with_output().expect("wait for aplan");
        assert_eq!(
            output.status.signal(),
            Some(signal),
            "signal {signal} became {status:?}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            status = output.status,
        );
        assert!(
            sent.elapsed() < Duration::from_secs(10),
            "signal {signal} waited for the uncooperative provider"
        );
        wait_for_process_exit("wrapper", wrapper_pid);
        wait_for_process_exit("provider", provider_pid);
        wait_for_process_exit("descendant", descendant_pid);
        assert!(
            !output.stdout.contains(&0x1b) && !output.stderr.contains(&0x1b),
            "non-interactive signal handling emitted terminal controls"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("progress: "),
            "signal {signal} did not exercise visible progress"
        );
    }
}

#[test]
fn readiness_requires_complete_live_pid_publication() {
    let fixture = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("PID publication fixture");
    let path = fixture.path().join("provider.pid");

    std::fs::write(&path, "").expect("empty publication");
    assert!(
        published_child_process(&path, std::process::id())
            .unwrap_err()
            .contains("complete PID")
    );
    std::fs::write(&path, "not-a-pid\n").expect("malformed publication");
    assert!(
        published_child_process(&path, std::process::id())
            .unwrap_err()
            .contains("complete PID")
    );
    std::fs::write(&path, format!("{}\n", std::process::id())).expect("live publication");
    assert!(
        published_child_process(&path, u32::MAX)
            .unwrap_err()
            .contains("rather than expected process"),
        "an unrelated live PID must not satisfy readiness"
    );

    let mut child = Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .expect("live child process");
    std::fs::write(&path, format!("{}\n", child.id())).expect("child publication");
    let published = published_child_process(&path, std::process::id())
        .expect("a complete live child publication");
    assert_eq!(published.pid, child.id());
    child.kill().expect("stop live child");
    child.wait().expect("reap live child");

    let mut zombie = Command::new("/bin/true").spawn().expect("zombie child");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if process_identity(zombie.id()).is_ok_and(|identity| identity.state == 'Z') {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "child did not reach zombie state before it was reaped"
        );
        std::thread::yield_now();
    }
    std::fs::write(&path, format!("{}\n", zombie.id())).expect("zombie publication");
    assert!(
        published_child_process(&path, std::process::id())
            .unwrap_err()
            .contains("non-live state Z")
    );
    zombie.wait().expect("reap zombie child");

    std::fs::write(&path, format!("{}\n", u32::MAX)).expect("dead publication");
    assert!(
        published_child_process(&path, std::process::id())
            .unwrap_err()
            .contains("not live")
    );
}

fn startup_error(fixture: &tempfile::TempDir) -> String {
    let root = fixture.path();
    let mut process = common::command(
        root,
        &[
            "--color",
            "never",
            "--root",
            "plan",
            "show",
            "T-001",
            "--refresh-evidence",
        ],
    );
    process
        .env("PATH", "")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = process.spawn().expect("spawn failing aplan show");
    match wait_for_provider_ready(child, root) {
        Ok((child, _, _, _)) => {
            common::send_signal(child.id(), "TERM");
            let _ = child.wait_with_output();
            panic!("provider unexpectedly reached readiness");
        }
        Err(error) => error,
    }
}

#[test]
fn provider_startup_failures_report_actual_status_and_output() {
    use std::os::unix::fs::PermissionsExt as _;

    let exited = fixture();
    std::fs::write(
        exited.path().join("list.sh"),
        "#!/bin/sh\necho provider-exited-before-readiness >&2\nexit 23\n",
    )
    .expect("exiting provider");
    let error = startup_error(&exited);
    assert!(
        error.contains("provider exited before readiness with status 23"),
        "{error}"
    );
    assert!(
        error.contains("provider-exited-before-readiness"),
        "{error}"
    );

    let no_channel = fixture();
    std::fs::create_dir(no_channel.path().join("provider.started"))
        .expect("block readiness publication");
    let error = startup_error(&no_channel);
    assert!(
        error.contains("provider exited before readiness with status"),
        "{error}"
    );
    assert!(error.contains("provider.started"), "{error}");

    let cannot_execute = fixture();
    let runner = cannot_execute.path().join("list.sh");
    let mut permissions = runner.metadata().expect("provider metadata").permissions();
    permissions.set_mode(0o644);
    std::fs::set_permissions(&runner, permissions).expect("make provider non-executable");
    let error = startup_error(&cannot_execute);
    assert!(
        error.contains("provider exited before readiness with status 126"),
        "{error}"
    );
    assert!(
        error.contains("Permission denied") || error.contains("permission denied"),
        "{error}"
    );
}
