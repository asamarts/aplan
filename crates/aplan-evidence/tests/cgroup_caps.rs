//! The kernel's half of `test.max_memory` and `test.max_processes` (`A1-310`).
//!
//! Everything *around* the cage is unit-tested in `resolve.rs` against ordinary
//! directories, because that half has to be covered on any machine. These two
//! are the other half: proof that a real cgroup really stops a real command,
//! and that `aplan verify --execute` then says the citation was *stopped* rather
//! than that it failed. They need a cgroup this process may write to, so they
//! skip where there is none -- which includes this project's own CI, a rootless
//! container with `/sys/fs/cgroup` mounted read-only. That is exactly the
//! `NotDelegated` platform ADR-0033 decided to name rather than pretend about,
//! so the skip is the feature's own answer and not a hole in it.
//!
//! They live outside `src/` deliberately. A test that cannot run everywhere
//! would otherwise drag the crate's coverage floor down by however much of the
//! machine it happens to be standing on.

use aplan_core::task::{Evidence, EvidenceKind, Status, Task};
use aplan_core::{Plan, code, load::LoadedTask};
use aplan_evidence::{TestRunner, TestRunners, VerifyOptions, verify};

/// A command that allocates until the kernel stops it -- and stops itself if the
/// kernel does not.
///
/// **Bounded on purpose.** The first version doubled a shell variable in a loop
/// with no end, which is harmless while the cage holds it and eats the machine
/// the moment the cage fails to apply: a test failure became a host outage, and
/// this session watched background work killed for memory while it ran. This
/// reaches about 225 MB in under a second and exits, so a cage that did not
/// apply fails the assertion rather than the box.
///
/// `awk` rather than a shell loop because it allocates in small steps, and
/// rather than `python3` because the CI image has one and not the other. A
/// program with only a `BEGIN` block reads no input, so the argument carrying
/// the test name or url is ignored.
const HOG: &str = "BEGIN{for(i=0;i<200000;i++) a[i]=sprintf(\"%1000s\",\"\")}";

/// A `url` block that caps the checker.
fn capped_checker(max_memory: Option<&str>) -> aplan_config::UrlCheck {
    aplan_config::UrlCheck {
        // Allocates until the kernel stops it, in POSIX shell: no python, no
        // package the CI image would have to grow.
        check: vec![
            "awk".into(),
            HOG.into(),
            aplan_config::URL_PLACEHOLDER.into(),
        ],
        timeout_seconds: Some(60),
        max_memory: max_memory.map(ToString::to_string),
        max_processes: None,
        wrap: None,
    }
}

/// Whether this machine has a cgroup this process may create children in.
///
/// Asked the way the code under test asks it: build a cage through the public
/// path and see whether the limit came back unenforced.
fn cgroups_are_available() -> bool {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("a temp dir");
    let plan = plan_of(done_task("T", vec![test_ev("quiet")]));
    let opts = capped(
        &["printf", "quiet\n"],
        &["true", aplan_config::PLACEHOLDER],
        Some("64M"),
        None,
    );
    git2::Repository::init(dir.path()).expect("a repository");
    let (_, spend) = aplan_evidence::verify_measured(&plan, dir.path(), &opts).expect("it ran");
    !spend.is_empty() && spend.iter().all(|(_, c)| c.unenforced.is_none())
}

fn capped(
    list: &[&str],
    run: &[&str],
    max_memory: Option<&str>,
    max_processes: Option<u32>,
) -> VerifyOptions {
    VerifyOptions {
        config_file: None,
        execute: true,
        test_runner: Some(TestRunners::One(TestRunner {
            max_memory: max_memory.map(ToString::to_string),
            max_processes,
            wrap: None,
            expect_tests: None,
            skipped: None,
            run_selects: None,
            timeout_seconds: None,
            covers: None,
            covers_by_test: None,
            name: None,
            dir: None,
            list: list.iter().map(|s| (*s).to_string()).collect(),
            run: run.iter().map(|s| (*s).to_string()).collect(),
        })),
        max_unrelated_files: None,
        url_check: None,
    }
}

fn done_task(id: &str, evidence: Vec<Evidence>) -> Task {
    Task {
        id: id.to_string(),
        title: id.to_string(),
        phase: 0,
        status: Status::Done,
        owner: None,
        effort_days: None,
        blocked_by: Vec::new(),
        prefer_after: Vec::new(),
        evidence,
        paths: Vec::new(),
        traces: Vec::new(),
        trace_fingerprints: std::collections::BTreeMap::new(),
        tags: Vec::new(),
        revisit_at: None,
        version_impact: None,
        version_impact_reason: None,
    }
}

fn test_ev(name: &str) -> Evidence {
    Evidence {
        kind: EvidenceKind::Test,
        reference: name.to_string(),
        captured: None,
    }
}

fn plan_of_all(tasks: Vec<Task>) -> Plan {
    Plan {
        tasks: tasks
            .into_iter()
            .map(|task| LoadedTask {
                path: std::path::PathBuf::from(format!("plan/tasks/{}.md", task.id)),
                task,
                fields: aplan_core::load::FieldLines::new(),
                criteria: aplan_core::load::Criteria::default(),
            })
            .collect(),
        load_diagnostics: Vec::new(),
        unloadable: std::collections::BTreeMap::new(),
    }
}

fn plan_of(task: Task) -> Plan {
    Plan {
        tasks: vec![LoadedTask {
            task,
            path: std::path::PathBuf::from("plan/tasks/x.md"),
            fields: aplan_core::load::FieldLines::new(),
            criteria: aplan_core::load::Criteria::default(),
        }],
        load_diagnostics: Vec::new(),
        unloadable: std::collections::BTreeMap::new(),
    }
}

/// A cited test whose command really exceeds `max_memory` is *stopped*.
///
/// The fixture is a POSIX shell doubling a string until it is killed: no
/// python, no `dd`, nothing the CI image would have to grow a package for. The
/// assertion that matters is the wording -- a stopped command neither passed
/// nor failed, and reporting it as a failure is the misreading `A1-300` exists
/// to prevent.
#[test]
fn a_cited_test_whose_command_exceeds_its_memory_cap_is_stopped_not_failed() {
    if !cgroups_are_available() {
        return;
    }
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("a temp dir");
    git2::Repository::init(dir.path()).expect("a repository");
    let report = verify(
        &plan_of(done_task("T-1", vec![test_ev("hog")])),
        dir.path(),
        &capped(
            &["printf", "hog\n"],
            &["awk", HOG, aplan_config::PLACEHOLDER],
            Some("64M"),
            None,
        ),
    )
    .expect("it ran");

    let found: Vec<&str> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == code::TEST_FAILED)
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(found.len(), 1, "{:?}", report.diagnostics);
    assert!(
        found[0].contains("was stopped") && found[0].contains("neither passed nor failed"),
        "a stopped command is not a failing test: {}",
        found[0]
    );
    assert!(
        found[0].contains("`max_memory` (64M)"),
        "it names the cap the adopter wrote: {}",
        found[0]
    );
}

/// A `max_processes` breach is caught even though the command exits happily.
///
/// This is the case an exit status cannot tell you about: `pids.max` makes
/// `fork()` return `EAGAIN`, the shell prints its own complaint and carries on,
/// and the script below still exits **0**. Reading `pids.events` is the only
/// thing that separates it from a command that simply worked -- the reason
/// `A1-308`'s audit insisted a breach be read and never inferred.
#[test]
fn a_cited_test_that_exceeds_its_process_cap_is_caught_though_the_command_exits_zero() {
    if !cgroups_are_available() {
        return;
    }
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("a temp dir");
    git2::Repository::init(dir.path()).expect("a repository");
    let report = verify(
        &plan_of(done_task("T-1", vec![test_ev("forky")])),
        dir.path(),
        &capped(
            &["printf", "forky\n"],
            &[
                "sh",
                "-c",
                "i=0; while [ $i -lt 40 ]; do /bin/sleep 1 & i=$((i+1)); done; wait; exit 0",
                aplan_config::PLACEHOLDER,
            ],
            None,
            Some(4),
        ),
    )
    .expect("it ran");

    let found: Vec<&str> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == code::TEST_FAILED)
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(found.len(), 1, "{:?}", report.diagnostics);
    assert!(
        found[0].contains("`max_processes` (4)"),
        "it names the cap the adopter wrote: {}",
        found[0]
    );
}

/// A `url.check` stopped at its cap is the configuration's fault, and no url is
/// called dead on the strength of a checker that never answered.
///
/// ADR-0033 covers every spawned command, not the test runner's alone. This is
/// the half that was decided and not built: `url.check` ran uncapped until
/// `A1-310`'s audit (`A1-310`).
#[test]
fn a_url_checker_stopped_at_its_cap_is_a_configuration_fault() {
    if !cgroups_are_available() {
        return;
    }
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("a temp dir");
    git2::Repository::init(dir.path()).expect("a repository");
    let task = done_task(
        "T-1",
        vec![Evidence {
            kind: EvidenceKind::Url,
            reference: "https://example.invalid/page".to_string(),
            captured: None,
        }],
    );
    let report = verify(
        &plan_of(task),
        dir.path(),
        &VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: None,
            max_unrelated_files: None,
            url_check: Some(capped_checker(Some("64M"))),
        },
    )
    .expect("it ran");

    let found: Vec<&str> = report
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .filter(|m| m.contains("was stopped"))
        .collect();
    assert_eq!(found.len(), 1, "{:?}", report.diagnostics);
    assert!(
        found[0].contains("`url.max_memory`"),
        "it names the key the adopter wrote: {}",
        found[0]
    );
    // Reported once, against the configuration rather than against any task:
    // a cap the adopter set is not a dead link.
    let about: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == code::URL_UNREACHABLE)
        .collect();
    assert_eq!(about.len(), 1, "{:?}", report.diagnostics);
    assert!(
        about[0].task.is_none(),
        "it is the configuration's fault, not the task's: {:?}",
        about[0]
    );
    assert!(
        !about[0].message.contains("could not be run"),
        "it ran and was stopped, which is not the same thing: {}",
        about[0].message
    );
}

/// A verdict reached without its probe is not remembered (`A1-243`, audit).
///
/// `APLAN037` is decided by a sentinel run, which `run_sentinel` deliberately
/// retries rather than caches when it cannot be run. Memoizing the *test's*
/// verdict alongside it undid that: the first citation judged `alpha` while the
/// probe was unavailable, cached "it proved something", and every later citation
/// reused it -- so a real false green went unreported where it had been caught
/// before.
///
/// The fixture makes only the sentinel breach, and only on its first attempt: a
/// cgroup is the one lever that can fail one spawn of a command and not the
/// next, which is why this lives here and skips where there is none.
#[test]
fn a_pass_judged_without_its_probe_is_not_remembered() {
    if !cgroups_are_available() {
        return;
    }
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("a temp dir");
    git2::Repository::init(dir.path()).expect("a repository");
    let counter = dir.path().join("probe-attempts").display().to_string();
    let script = format!(
        "case \"$0\" in \
           *aplan-probe*) n=$(cat '{counter}' 2>/dev/null || echo 0); \
             echo $((n+1)) > '{counter}'; \
             if [ \"$n\" = 0 ]; then awk '{HOG}'; fi; \
             exit 0;; \
           *) exit 0;; \
         esac"
    );
    let report = verify(
        &plan_of_all(vec![
            done_task("T-1", vec![test_ev("alpha")]),
            done_task("T-2", vec![test_ev("alpha")]),
        ]),
        dir.path(),
        &capped(
            &["printf", "alpha\n"],
            &["sh", "-c", &script, aplan_config::PLACEHOLDER],
            Some("64M"),
            None,
        ),
    )
    .expect("it ran");

    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == code::RUN_PROVES_NOTHING),
        "the second citation must re-run rather than reuse a verdict reached \
         without the probe: {:?}",
        report.diagnostics
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("probe-attempts"))
            .expect("the probe ran")
            .trim(),
        "2",
        "the sentinel was retried, which is what `run_sentinel` promises"
    );
}

/// A stopped command is every citing task's finding, and still one run.
///
/// The intersection of the two features: `A1-310` says a command stopped at its
/// cap is reported per citation as *stopped*, and `A1-243` says it runs once.
/// Both, or the second would silence all but the first task.
#[test]
fn a_stopped_command_is_reported_for_every_citation_and_run_once() {
    if !cgroups_are_available() {
        return;
    }
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("a temp dir");
    git2::Repository::init(dir.path()).expect("a repository");
    let counter = dir.path().join("runs").display().to_string();
    let script = format!(
        "n=$(cat '{counter}' 2>/dev/null || echo 0); echo $((n+1)) > '{counter}'; \
         exec awk '{HOG}'"
    );
    let report = verify(
        &plan_of_all(vec![
            done_task("T-1", vec![test_ev("hog")]),
            done_task("T-2", vec![test_ev("hog")]),
        ]),
        dir.path(),
        &capped(
            &["printf", "hog\n"],
            &["sh", "-c", &script, aplan_config::PLACEHOLDER],
            Some("64M"),
            None,
        ),
    )
    .expect("it ran");

    let stopped: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == code::TEST_FAILED && d.message.contains("was stopped"))
        .collect();
    assert_eq!(
        stopped.len(),
        2,
        "both citations are told, not just the one that ran it: {:?}",
        report.diagnostics
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("runs"))
            .expect("it ran")
            .trim(),
        "1",
        "and the command ran once"
    );
}
