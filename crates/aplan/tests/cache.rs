use crate::common;

fn task(title: &str) -> String {
    format!(
        "---\nid: A-1\ntitle: {title}\nphase: 1\nstatus: todo\neffort_days: 1\npaths:\n- src/**\n---\n\n## Acceptance criteria\n\n- [ ] works\n"
    )
}

fn fixture() -> tempfile::TempDir {
    let directory = tempfile::Builder::new()
        .prefix("aplan-cache-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    std::fs::create_dir_all(directory.path().join("aplan/tasks")).expect("plan directory");
    std::fs::write(directory.path().join("aplan/tasks/A-1.md"), task("first")).expect("task");
    std::fs::write(directory.path().join("aplan/README.md"), "# prose\n").expect("prose");
    directory
}

fn run(directory: &std::path::Path, cache: &std::path::Path, mode: &str) -> common::Run {
    let output = common::command(
        directory,
        &[
            "--timings",
            "--cache",
            mode,
            "--no-config",
            "--root",
            "aplan",
            "show",
            "A-1",
            "--no-resolve",
            "--format",
            "json",
        ],
    )
    .env("APLAN_CACHE_DIR", cache)
    .output()
    .expect("run aplan");
    common::Run {
        stdout: String::from_utf8(output.stdout).expect("UTF-8 stdout"),
        stderr: String::from_utf8(output.stderr).expect("UTF-8 stderr"),
        code: output.status.code().expect("ordinary exit"),
    }
}

fn run_check(directory: &std::path::Path, cache: &std::path::Path, root: &str) -> common::Run {
    let output = common::command(
        directory,
        &[
            "--cache",
            "auto",
            "--no-config",
            "--root",
            root,
            "check",
            "--format",
            "json",
        ],
    )
    .env("APLAN_CACHE_DIR", cache)
    .output()
    .expect("run aplan check");
    common::Run {
        stdout: String::from_utf8(output.stdout).expect("UTF-8 stdout"),
        stderr: String::from_utf8(output.stderr).expect("UTF-8 stderr"),
        code: output.status.code().expect("ordinary exit"),
    }
}

fn run_layered_graph(directory: &std::path::Path, cache: &std::path::Path) -> common::Run {
    let output = common::command(
        directory,
        &[
            "--timings",
            "--cache",
            "auto",
            "--no-config",
            "--root",
            "aplan",
            "graph",
            "--format",
            "text",
            "--layout",
            "layered",
            "--detail",
            "compact",
            "--orientation",
            "top-to-bottom",
            "--width",
            "120",
            "--ascii",
        ],
    )
    .env("APLAN_CACHE_DIR", cache)
    .output()
    .expect("run layered graph");
    common::Run {
        stdout: String::from_utf8(output.stdout).expect("UTF-8 stdout"),
        stderr: String::from_utf8(output.stderr).expect("UTF-8 stderr"),
        code: output.status.code().expect("ordinary exit"),
    }
}

fn timing(run: &common::Run) -> &str {
    run.stderr
        .lines()
        .find(|line| line.starts_with("aplan: timings "))
        .expect("timing diagnostic")
}

fn diagnostics(run: &common::Run) -> String {
    run.stderr
        .lines()
        .filter(|line| !line.starts_with("aplan: timings "))
        .fold(String::new(), |mut output, line| {
            output.push_str(line);
            output.push('\n');
            output
        })
}

fn cache_bytes(cache: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let directory = cache.join("aplan/plan-v1");
    let mut values = std::fs::read_dir(directory)
        .expect("cache directory")
        .map(|entry| entry.expect("cache entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            let bytes = std::fs::read(&path).expect("cache generation");
            (path, bytes)
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| left.0.cmp(&right.0));
    values
}

/// A persistent hit is an optimization of the same operation, including the
/// strict source-only escape hatch. This is the end-to-end proof for A1-589.
#[test]
fn cache_hit_off_and_refresh_preserve_the_complete_cli_result() {
    let directory = fixture();
    let cache = directory.path().join("host-cache");

    let cold = run(directory.path(), &cache, "auto");
    let hit = run(directory.path(), &cache, "auto");
    assert_eq!(cold.code, 0, "{}", cold.text());
    assert_eq!(hit.code, cold.code);
    assert_eq!(hit.stdout, cold.stdout);
    assert_eq!(diagnostics(&hit), diagnostics(&cold));
    assert!(timing(&cold).contains("cache_hits=0"), "{}", timing(&cold));
    for expected in [
        "bytes_read=0",
        "tasks_parsed=0",
        "cache_hits=1",
        "cache_reused=2",
        "cache_parsed=0",
        "trace_reads=0",
    ] {
        assert!(
            timing(&hit).contains(expected),
            "missing `{expected}` in {}",
            timing(&hit)
        );
    }

    let before_off = cache_bytes(&cache);
    let off = run(directory.path(), &cache, "off");
    assert_eq!(off.code, cold.code);
    assert_eq!(off.stdout, cold.stdout);
    assert_eq!(diagnostics(&off), diagnostics(&cold));
    assert_eq!(cache_bytes(&cache), before_off, "--cache=off wrote state");
    for expected in ["cache_hits=0", "cache_reused=0", "cache_parsed=2"] {
        assert!(timing(&off).contains(expected), "{}", timing(&off));
    }

    let refresh = run(directory.path(), &cache, "refresh");
    assert_eq!(refresh.code, cold.code);
    assert_eq!(refresh.stdout, cold.stdout);
    assert_eq!(diagnostics(&refresh), diagnostics(&cold));
    for expected in ["cache_hits=0", "cache_reused=0", "cache_parsed=2"] {
        assert!(timing(&refresh).contains(expected), "{}", timing(&refresh));
    }
}

/// A semantic read that never computes the structural report must still
/// publish its parsed artifacts. This is the end-to-end performance proof for
/// the bounded A1-686 graph journey: a graph invocation warms itself rather
/// than depending on an unrelated `check` first.
#[test]
fn layered_graph_publishes_the_cache_that_its_next_process_reuses() {
    let directory = fixture();
    let cache = directory.path().join("host-cache");

    let cold = run_layered_graph(directory.path(), &cache);
    let hit = run_layered_graph(directory.path(), &cache);
    assert_eq!(cold.code, 0, "{}", cold.text());
    assert_eq!(hit.code, cold.code, "{}", hit.text());
    assert_eq!(hit.stdout, cold.stdout);
    assert_eq!(diagnostics(&hit), diagnostics(&cold));
    assert!(timing(&cold).contains("cache_hits=0"), "{}", timing(&cold));
    assert!(
        !timing(&cold).contains("cache_bytes_written=0"),
        "{}",
        timing(&cold)
    );
    for expected in ["cache_hits=1", "cache_reused=2", "cache_parsed=0"] {
        assert!(
            timing(&hit).contains(expected),
            "missing `{expected}` in {}",
            timing(&hit)
        );
    }
}

/// Metadata reuse must not turn an in-place, same-length edit into a stale
/// answer. Unix change identity catches this even when size does not change;
/// platforms without that identity conservatively hash and reach the same
/// result.
#[test]
fn a_same_size_edit_reparses_only_the_changed_candidate() {
    let directory = fixture();
    let cache = directory.path().join("host-cache");
    let initial = run(directory.path(), &cache, "auto");
    assert_eq!(initial.code, 0, "{}", initial.text());

    std::fs::write(directory.path().join("aplan/tasks/A-1.md"), task("other"))
        .expect("same-size edit");
    let edited = run(directory.path(), &cache, "auto");
    assert_eq!(edited.code, 0, "{}", edited.text());
    assert!(edited.stdout.contains("other"), "{}", edited.stdout);
    assert!(!edited.stdout.contains("first"), "{}", edited.stdout);
    for expected in ["cache_hits=1", "cache_reused=1", "cache_parsed=1"] {
        assert!(
            timing(&edited).contains(expected),
            "missing `{expected}` in {}",
            timing(&edited)
        );
    }
}

/// Artifact reuse is independent of how a caller spelled the root, but a
/// persisted diagnostic already contains that invocation's path spelling. The
/// structural row must therefore be recomputed rather than leaking a relative
/// path into an absolute-root result.
#[test]
fn cached_reports_preserve_the_current_root_spelling_end_to_end() {
    let directory = fixture();
    let cache = directory.path().join("host-cache");
    let task_path = directory.path().join("aplan/tasks/A-1.md");
    std::fs::write(&task_path, task("first").replace("effort_days: 1\n", ""))
        .expect("task without effort");

    let relative = run_check(directory.path(), &cache, "aplan");
    assert_eq!(relative.code, 0, "{}", relative.text());
    let relative: serde_json::Value =
        serde_json::from_str(&relative.stdout).expect("relative JSON");
    assert_eq!(relative["diagnostics"][0]["file"], "aplan/tasks/A-1.md");

    let absolute_root = directory.path().join("aplan");
    let absolute_root = absolute_root.to_str().expect("UTF-8 fixture path");
    let absolute = run_check(directory.path(), &cache, absolute_root);
    assert_eq!(absolute.code, 0, "{}", absolute.text());
    let absolute: serde_json::Value =
        serde_json::from_str(&absolute.stdout).expect("absolute JSON");
    assert_eq!(
        absolute["diagnostics"][0]["file"],
        task_path.to_str().expect("UTF-8 task path")
    );
}
