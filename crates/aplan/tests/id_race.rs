//! Concurrent writers minting sequential ids (`A1-230`).
//!
//! The defect, measured before the fix: eight agents running `aplan add` at
//! once against the same plan, and **three of them succeeded**. Each one read
//! the plan, saw the highest id, derived the next one, and wrote it; seven lost
//! the file to whoever got there first. At twenty-four agents it was two. A
//! fleet of agents sharing a repository is the case `README.md` opens with, and
//! "most of your agents are refused" is not a plan tool working.
//!
//! What makes this different from `A1-226` next door is where the collision is.
//! There, two writers updated the *same* milestone and one update was lost
//! silently. Here nothing is lost -- `emit` refuses to overwrite, which is why
//! the defect was five refusals rather than five vanished tasks -- and the cost
//! is throughput.
//!
//! **The invariant these tests hold is not "all eight succeed".** A refusal is
//! a correct outcome, and a machine under enough load can still produce one.
//! What must never happen is two agents writing the same file: distinct tasks
//! landing on one id, where whoever wrote second would silently own the first
//! one's file. That assertion holds at three successes and at eight, and it
//! fails the moment an id is handed out twice.

use std::collections::BTreeSet;
use std::path::Path;

mod common;

/// A plan with three tasks in a sequence, and the default (sequential)
/// strategy: every concurrent `add` will derive `T0003`.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    for i in 0..3 {
        let id = format!("T{i:04}");
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: seed {i}\nphase: 0\nstatus: todo\nowner: t\n\
                 effort_days: 1\n---\n\n## Why\n\nx\n"
            ),
        )
        .expect("seed task");
    }
}

/// One writer's outcome: the id it was told it wrote, if it was told anything.
struct Wrote {
    writer: usize,
    id: Option<String>,
}

/// Run `writers` concurrent `add`, each with a title naming itself.
fn race(dir: &Path, writers: usize) -> Vec<Wrote> {
    let handles: Vec<_> = (0..writers)
        .map(|writer| {
            let dir = dir.to_path_buf();
            std::thread::spawn(move || {
                let out = common::run(&dir, &["add", &format!("Writer {writer}"), "--effort", "1"]);
                let id = (out.code == 0)
                    .then(|| {
                        out.text()
                            .lines()
                            .find_map(|l| l.strip_prefix("wrote "))
                            .and_then(|l| l.split_whitespace().next())
                            .map(str::to_string)
                    })
                    .flatten();
                Wrote { writer, id }
            })
        })
        .collect();
    handles
        .into_iter()
        .map(|h| h.join().expect("writer thread"))
        .collect()
}

/// **The eight-agent reproduction, as the invariant it must hold** (`A1-230`).
///
/// Every writer that was told `wrote <id>` owns that id alone, and the file
/// under it carries that writer's own title. Two agents on one id would show up
/// here as a duplicate in the set, or as a file whose title belongs to a
/// different writer than the one that was told it wrote it.
///
/// Proof of `ids.sequential_survives_concurrent_writers`: the half a consumer
/// pinning that name is relying on for *correctness*, as against the
/// throughput half below.
#[test]
fn no_two_concurrent_writers_are_given_the_same_id() {
    let tmp = tempfile::Builder::new()
        .prefix("id-race-eight")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let dir = tmp.path();
    fixture(dir);

    let results = race(dir, 8);
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    for Wrote { writer, id } in &results {
        let Some(id) = id else { continue };
        assert!(
            claimed.insert(id.clone()),
            "writer {writer} and an earlier writer were both told they wrote `{id}`, \
             so one of them owns a file holding the other's task"
        );
        let body = std::fs::read_to_string(dir.join(format!("aplan/tasks/{id}.md")))
            .unwrap_or_else(|e| panic!("writer {writer} was told it wrote `{id}`: {e}"));
        assert!(
            body.contains(&format!("Writer {writer}")),
            "writer {writer} was told it wrote `{id}`, but that file holds somebody \
             else's task:\n{body}"
        );
    }

    // The counts have to agree too: a success that left no file, or a file
    // nobody was told about, is the same defect seen from the other side.
    let on_disk = std::fs::read_dir(dir.join("aplan/tasks"))
        .expect("read plan dir")
        .count();
    assert_eq!(
        on_disk,
        3 + claimed.len(),
        "{} writers reported success and the plan holds {on_disk} files over three seeds",
        claimed.len()
    );

    let (said, code) = common::aplan(dir, &["check"]);
    assert_eq!(code, 0, "the plan does not load after the race:\n{said}");
}

/// **Contention is survived rather than merely detected** (`A1-230`).
///
/// The invariant above held before the fix, when five of eight agents were
/// refused. This is the part that changed: a writer that loses the id mints
/// again from the plan as it now is, so contention costs milliseconds instead
/// of a failed command. Measured over five runs of each: 8 of 8 every time with
/// the slowest writer taking about 40ms, and 24 of 24 every time with the
/// slowest about 160ms, against a five-second deadline. So the headroom is
/// roughly thirty times the worst case observed, and about fifteen times what
/// the same measurement gave on a loaded machine.
///
/// If this ever fails while the test above passes, the honest reading is load
/// and not breakage -- but it has not, and a regression that reinstates the
/// defect fails it on the first run.
///
/// Proof of `ids.sequential_survives_concurrent_writers`: the throughput half,
/// and the only one that distinguishes this build from one that merely refuses
/// safely.
#[test]
fn eight_concurrent_writers_all_land() {
    let tmp = tempfile::Builder::new()
        .prefix("id-race-throughput")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let dir = tmp.path();
    fixture(dir);

    let results = race(dir, 8);
    let landed: Vec<usize> = results
        .iter()
        .filter(|w| w.id.is_some())
        .map(|w| w.writer)
        .collect();
    assert_eq!(
        landed.len(),
        8,
        "only {} of 8 concurrent writers landed (writers {landed:?}); before the \
         retry it was three",
        landed.len()
    );
}

/// **An id the caller named is refused at once** (`A1-230`).
///
/// Retrying re-mints, which is the remedy for an id this command chose and
/// never for one the caller supplied: `--id T0000` against an existing `T0000`
/// is the same collision five seconds later. So a supplied collision must come
/// back immediately, and it must not be dressed up as somebody else's race.
///
/// Proof of `ids.sequential_survives_concurrent_writers`: its boundary. The
/// promise is about an id this tool minted, and a consumer pinning the name is
/// entitled to know it did not also make `--id` slow to refuse.
#[test]
fn a_supplied_id_that_is_taken_is_refused_without_waiting() {
    let tmp = tempfile::Builder::new()
        .prefix("id-race-supplied")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let dir = tmp.path();
    fixture(dir);

    let started = std::time::Instant::now();
    let (said, code) = common::aplan(dir, &["add", "Mine", "--id", "T0000", "--effort", "1"]);
    let took = started.elapsed();
    assert_ne!(code, 0, "a taken id was accepted:\n{said}");
    assert!(
        took < std::time::Duration::from_secs(4),
        "a supplied collision took {}ms, so it is being retried against the \
         five-second deadline:\n{said}",
        took.as_millis()
    );
    assert!(
        !said.contains("another writer"),
        "a supplied collision was reported as a race:\n{said}"
    );
}

/// **A writer that lost an id leaves no trace of the one it lost** (`A1-230`).
///
/// The retry re-reads and mints again, which is only safe if the failed attempt
/// wrote nothing. `add --cut` is where that could go wrong: it writes the task
/// file and then edits the milestone, so an attempt that got the order the
/// other way round would leave the lost id in `blocked_by` with no file behind
/// it. Eight writers against one cut, and the membership must be exactly the
/// eight that landed -- `aplan check` is the independent witness, because
/// `APLAN003` refuses a `blocked_by` naming a task that does not exist.
///
/// Proof of `ids.sequential_survives_concurrent_writers`: that a lost attempt
/// wrote nothing, which is what makes re-minting sound rather than merely
/// convenient.
#[test]
fn a_lost_id_leaves_nothing_behind_in_the_cut() {
    let tmp = tempfile::Builder::new()
        .prefix("id-race-cut")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let dir = tmp.path();
    fixture(dir);
    std::fs::write(
        dir.join("aplan/tasks/M-c1.md"),
        "---\nid: M-c1\ntitle: Cut one\nphase: 0\nstatus: todo\ntags: [milestone]\n\
         blocked_by: []\n---\n\n## Why\n\nx\n",
    )
    .expect("milestone");

    let handles: Vec<_> = (0..8)
        .map(|writer| {
            let dir = dir.to_path_buf();
            std::thread::spawn(move || {
                let out = common::run(
                    &dir,
                    &[
                        "add",
                        &format!("Writer {writer}"),
                        "--cut",
                        "M-c1",
                        "--effort",
                        "1",
                    ],
                );
                (out.code == 0)
                    .then(|| {
                        out.text()
                            .lines()
                            .find_map(|l| l.strip_prefix("wrote "))
                            .and_then(|l| l.split_whitespace().next())
                            .map(str::to_string)
                    })
                    .flatten()
            })
        })
        .collect();
    let landed: BTreeSet<String> = handles
        .into_iter()
        .filter_map(|h| h.join().expect("writer thread"))
        .collect();

    let milestone = std::fs::read_to_string(dir.join("aplan/tasks/M-c1.md")).expect("milestone");
    let members: BTreeSet<String> = milestone
        .lines()
        .find(|l| l.starts_with("blocked_by:"))
        .map(|l| {
            l.trim_start_matches("blocked_by:")
                .trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        members, landed,
        "the cut's membership is not the set of writers that landed, so a lost \
         attempt left something behind:\n{milestone}"
    );

    let (said, code) = common::aplan(dir, &["check"]);
    assert_eq!(
        code, 0,
        "the plan does not load after eight writers raced into one cut:\n{said}"
    );
}

/// A plan of `n` closed tasks, each with an outcome a successor can take over.
fn closed_fixture(dir: &Path, n: usize) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    for i in 0..n {
        let id = format!("T{i:04}");
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: pred {i}\nphase: 0\nstatus: done\nowner: t\n\
                 effort_days: 1\nevidence:\n  - kind: note\n    ref: shipped\n    \
                 captured: 2026-10-01\n---\n\n## Why\n\nx\n"
            ),
        )
        .expect("seed task");
    }
}

/// **`supersede` survives contention too, through the other writer** (`A1-230`,
/// audit).
///
/// This is not the same code path as `add` and it was not covered. `add` writes
/// one new file through `emit`, which reports a collision as
/// `EmitError::Exists`; `supersede` writes the successor *and* the re-stamped
/// predecessor in one batch through `save`, which reports it as
/// `SaveError::Exists` behind a `SaveFailed`. `lost_the_id` has a separate
/// branch for each, so a break in one is invisible to every test of the other
/// -- and the behaviour this build advertises covers all three minting
/// commands.
///
/// Proof of `ids.sequential_survives_concurrent_writers`: its batch-writer
/// half.
#[test]
fn eight_concurrent_supersedes_all_land() {
    let tmp = tempfile::Builder::new()
        .prefix("id-race-supersede")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let dir = tmp.path();
    closed_fixture(dir, 8);

    let handles: Vec<_> = (0..8)
        .map(|writer| {
            let dir = dir.to_path_buf();
            std::thread::spawn(move || {
                let out = common::run(
                    &dir,
                    &[
                        "supersede",
                        &format!("T{writer:04}"),
                        &format!("Successor {writer}"),
                        "--effort",
                        "1",
                    ],
                );
                // `supersede` reports `superseded <pred>: filed <successor>`,
                // which is a different line from `add`'s `wrote <id>`.
                (out.code == 0).then(|| {
                    out.text()
                        .lines()
                        .find_map(|l| l.rsplit_once(": filed "))
                        .and_then(|(_, id)| id.split_whitespace().next())
                        .map(str::to_string)
                })
            })
        })
        .collect();
    let results: Vec<Option<Option<String>>> = handles
        .into_iter()
        .map(|h| h.join().expect("writer thread"))
        .collect();
    let landed: BTreeSet<String> = results.iter().flatten().flatten().cloned().collect();
    assert_eq!(
        landed.len(),
        8,
        "only {} of 8 concurrent supersedes landed; each one mints a successor id \
         from the same plan",
        landed.len()
    );

    // Eight predecessors and eight successors, and a plan that still loads: a
    // successor written over another's file would leave fewer than sixteen.
    let on_disk = std::fs::read_dir(dir.join("aplan/tasks"))
        .expect("read plan dir")
        .count();
    assert_eq!(on_disk, 16, "the plan holds {on_disk} files, not sixteen");
    let (said, code) = common::aplan(dir, &["check"]);
    assert_eq!(code, 0, "the plan does not load after the race:\n{said}");
}

/// **`add --from` survives contention, and re-plans the whole batch** (`A1-230`,
/// audit).
///
/// The hardest of the three, and the one most worth pinning. A manifest is
/// all-or-nothing: an entry may join a cut an earlier entry creates, so losing
/// one id means re-reading and planning the *batch* again rather than re-minting
/// one entry. Eight writers of two entries each is sixteen ids contending, and
/// what must come out is sixteen new tasks and a plan that loads.
///
/// Proof of `ids.sequential_survives_concurrent_writers`: the batch is re-planned
/// rather than partly applied.
#[test]
fn eight_concurrent_manifests_all_land_whole() {
    let tmp = tempfile::Builder::new()
        .prefix("id-race-manifest")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let dir = tmp.path();
    fixture(dir);

    for writer in 0..8 {
        std::fs::write(
            dir.join(format!("m{writer}.yml")),
            format!(
                "- title: Manifest {writer} first\n  effort: 1\n\
                 - title: Manifest {writer} second\n  effort: 1\n"
            ),
        )
        .expect("manifest");
    }

    let handles: Vec<_> = (0..8)
        .map(|writer| {
            let dir = dir.to_path_buf();
            std::thread::spawn(move || {
                let manifest = dir.join(format!("m{writer}.yml"));
                let out = common::run(
                    &dir,
                    &["add", "--from", manifest.to_str().expect("utf-8 path")],
                );
                (out.code == 0, out.text())
            })
        })
        .collect();
    let results: Vec<(bool, String)> = handles
        .into_iter()
        .map(|h| h.join().expect("writer thread"))
        .collect();
    let landed = results.iter().filter(|(ok, _)| *ok).count();
    assert_eq!(
        landed,
        8,
        "only {landed} of 8 concurrent manifests landed:\n{}",
        results
            .iter()
            .filter(|(ok, _)| !*ok)
            .map(|(_, said)| said.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );

    // Three seeds and sixteen new tasks. A batch applied twice, or half
    // applied, shows up here before `check` ever sees it.
    let on_disk = std::fs::read_dir(dir.join("aplan/tasks"))
        .expect("read plan dir")
        .count();
    assert_eq!(
        on_disk, 19,
        "the plan holds {on_disk} files, not three seeds and sixteen new tasks"
    );
    // Every title exactly once: a re-planned batch that wrote an entry twice
    // under two ids would pass the count above only by losing another.
    for writer in 0..8 {
        for half in ["first", "second"] {
            let needle = format!("Manifest {writer} {half}");
            let hits = std::fs::read_dir(dir.join("aplan/tasks"))
                .expect("read plan dir")
                .filter_map(Result::ok)
                .filter(|e| {
                    std::fs::read_to_string(e.path()).is_ok_and(|body| body.contains(&needle))
                })
                .count();
            assert_eq!(hits, 1, "`{needle}` is in {hits} task files, not one");
        }
    }
    let (said, code) = common::aplan(dir, &["check"]);
    assert_eq!(code, 0, "the plan does not load after the race:\n{said}");
}
