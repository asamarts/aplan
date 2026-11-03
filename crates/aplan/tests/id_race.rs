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
//! **The invariant these tests hold is not "all eight succeed".** A bounded,
//! actionable refusal is a correct outcome, and a machine under enough load
//! can still produce one. What must never happen is two agents writing the
//! same file, a partial batch, an orphaned membership, or an unloadable plan.
//! Those assertions hold at one success and at eight, and fail the moment a
//! writer reports work that did not land whole.

use std::collections::BTreeSet;
use std::path::Path;

use crate::common;

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
    code: i32,
    said: String,
}

/// Run `writers` concurrent `add`, each with a title naming itself.
fn race(dir: &Path, writers: usize) -> Vec<Wrote> {
    let handles: Vec<_> = (0..writers)
        .map(|writer| {
            let dir = dir.to_path_buf();
            std::thread::spawn(move || {
                let out = common::run(&dir, &["add", &format!("Writer {writer}"), "--effort", "1"]);
                let said = out.text();
                let id = (out.code == 0)
                    .then(|| {
                        said.lines()
                            .find_map(|l| l.strip_prefix("wrote "))
                            .and_then(|l| l.split_whitespace().next())
                            .map(str::to_string)
                    })
                    .flatten();
                Wrote {
                    writer,
                    id,
                    code: out.code,
                    said,
                }
            })
        })
        .collect();
    handles
        .into_iter()
        .map(|h| h.join().expect("writer thread"))
        .collect()
}

fn assert_actionable_refusal(operation: &str, said: &str) {
    let normalized = said.to_ascii_lowercase();
    for needle in ["nothing was written", "run the command again"] {
        assert!(
            normalized.contains(needle),
            "a refused concurrent {operation} omitted `{needle}`:\n{said}"
        );
    }
}

fn title_hits(dir: &Path, title: &str) -> usize {
    let expected = format!("title: {title}");
    std::fs::read_dir(dir.join("aplan/tasks"))
        .expect("read plan dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            std::fs::read_to_string(entry.path())
                .is_ok_and(|body| body.lines().any(|line| line == expected))
        })
        .count()
}

fn has_title(body: &str, title: &str) -> bool {
    let expected = format!("title: {title}");
    body.lines().any(|line| line == expected)
}

fn milestone_members(dir: &Path, id: &str) -> (String, BTreeSet<String>) {
    let milestone = std::fs::read_to_string(dir.join(format!("aplan/tasks/{id}.md")))
        .unwrap_or_else(|error| panic!("read milestone `{id}`: {error}"));
    let members = milestone
        .lines()
        .find(|line| line.starts_with("blocked_by:"))
        .map(|line| {
            line.trim_start_matches("blocked_by:")
                .trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|member| member.trim().to_string())
                .filter(|member| !member.is_empty())
                .collect()
        })
        .unwrap_or_default();
    (milestone, members)
}

/// **The eight-agent reproduction, as the invariant it must hold** (`A1-230`,
/// `A1-599`).
///
/// Every writer that was told `wrote <id>` owns that id alone, and the file
/// under it carries that writer's own title. Two agents on one id would show up
/// here as a duplicate in the set, or as a file whose title belongs to a
/// different writer than the one that was told it wrote it.
///
/// Proof of `ids.sequential_survives_concurrent_writers`: a real process race
/// in which every outcome is either a complete uniquely owned task or the
/// production bounded refusal. Scheduler speed is not a correctness verdict.
#[test]
fn concurrent_adds_are_unique_complete_or_actionably_refused() {
    let tmp = tempfile::Builder::new()
        .prefix("id-race-eight")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let dir = tmp.path();
    fixture(dir);

    let results = race(dir, 8);
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    for Wrote {
        writer,
        id,
        code,
        said,
    } in &results
    {
        if *code != 0 {
            assert!(id.is_none(), "a refused writer reported an id: {said}");
            assert_actionable_refusal("add", said);
            assert_eq!(
                title_hits(dir, &format!("Writer {writer}")),
                0,
                "a refused writer left a task behind"
            );
            continue;
        }
        let id = id.as_ref().unwrap_or_else(|| {
            panic!("writer {writer} exited zero without reporting an id:\n{said}")
        });
        assert!(
            claimed.insert(id.clone()),
            "writer {writer} and an earlier writer were both told they wrote `{id}`, \
             so one of them owns a file holding the other's task"
        );
        let body = std::fs::read_to_string(dir.join(format!("aplan/tasks/{id}.md")))
            .unwrap_or_else(|e| panic!("writer {writer} was told it wrote `{id}`: {e}"));
        assert!(
            has_title(&body, &format!("Writer {writer}")),
            "writer {writer} was told it wrote `{id}`, but that file holds somebody \
             else's task:\n{body}"
        );
        assert_eq!(
            title_hits(dir, &format!("Writer {writer}")),
            1,
            "writer {writer}'s task did not land exactly once"
        );
    }
    assert!(!claimed.is_empty(), "no concurrent add made progress");

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

/// **An id the caller named is not reported as a race** (`A1-230`, `A1-599`).
///
/// Retrying re-mints, which is the remedy for an id this command chose and
/// never for one the caller supplied: `--id T0000` against an existing `T0000`
/// is the same collision five seconds later. The controlled unit test proves
/// it makes one attempt without waiting; this process-level check proves the
/// user sees the ordinary collision rather than somebody else's race.
///
/// Proof of `ids.sequential_survives_concurrent_writers`: its boundary. The
/// promise is about an id this tool minted, and a consumer pinning the name is
/// entitled to know it did not also make `--id` slow to refuse.
#[test]
fn a_supplied_id_that_is_taken_is_not_reported_as_a_race() {
    let tmp = tempfile::Builder::new()
        .prefix("id-race-supplied")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let dir = tmp.path();
    fixture(dir);

    let (said, code) = common::aplan(dir, &["add", "Mine", "--id", "T0000", "--effort", "1"]);
    assert_ne!(code, 0, "a taken id was accepted:\n{said}");
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
                let said = out.text();
                let id = (out.code == 0)
                    .then(|| {
                        said.lines()
                            .find_map(|l| l.strip_prefix("wrote "))
                            .and_then(|l| l.split_whitespace().next())
                            .map(str::to_string)
                    })
                    .flatten();
                (writer, out.code, id, said)
            })
        })
        .collect();
    let results: Vec<(usize, i32, Option<String>, String)> = handles
        .into_iter()
        .map(|h| h.join().expect("writer thread"))
        .collect();
    let mut landed = BTreeSet::new();
    for (writer, code, id, said) in &results {
        let title = format!("Writer {writer}");
        if *code == 0 {
            let id = id.as_ref().unwrap_or_else(|| {
                panic!("cut writer {writer} exited zero without an id:\n{said}")
            });
            assert!(
                landed.insert(id.clone()),
                "two cut writers reported task `{id}`"
            );
            let body = std::fs::read_to_string(dir.join(format!("aplan/tasks/{id}.md")))
                .unwrap_or_else(|e| panic!("cut writer {writer} reported `{id}`: {e}"));
            assert!(
                body.contains(&title),
                "cut writer {writer} reported `{id}`, but it owns another body:\n{body}"
            );
            assert_eq!(
                title_hits(dir, &title),
                1,
                "cut writer {writer}'s task did not land exactly once"
            );
        } else {
            assert!(id.is_none(), "a refused cut writer reported an id: {said}");
            assert_actionable_refusal("add --cut", said);
            assert_eq!(
                title_hits(dir, &title),
                0,
                "refused cut writer {writer} left a task behind"
            );
        }
    }
    assert!(!landed.is_empty(), "no concurrent cut add made progress");

    let (milestone, members) = milestone_members(dir, "M-c1");
    assert_eq!(
        members, landed,
        "the cut's membership is not the set of writers that landed, so a lost \
         attempt left something behind:\n{milestone}"
    );
    let on_disk = std::fs::read_dir(dir.join("aplan/tasks"))
        .expect("read plan dir")
        .count();
    assert_eq!(
        on_disk,
        4 + landed.len(),
        "a refused cut writer left a task file or a successful one wrote extra files"
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
    std::fs::create_dir_all(dir.join("docs")).expect("docs");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\ndetect_drift: true\n").expect("config");
    std::fs::write(
        dir.join("docs/design.md"),
        "# Design\n\n## Shared\n\nThe first design.\n",
    )
    .expect("design");
    for i in 0..n {
        let id = format!("T{i:04}");
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: pred {i}\nphase: 0\nstatus: done\nowner: t\n\
                 effort_days: 1\npaths: [docs/**]\ntraces: [docs/design.md#shared]\nevidence:\n  - kind: note\n    ref: shipped\n    \
                 captured: 2026-10-01\n---\n\n## Why\n\nx\n"
            ),
        )
        .expect("seed task");
        let (said, code) = common::aplan(dir, &["close", &id, "--restamp"]);
        assert_eq!(code, 0, "could not seed `{id}` trace stamp:\n{said}");
    }
    std::fs::write(
        dir.join("docs/design.md"),
        "# Design\n\n## Shared\n\nThe second design requires successor work.\n",
    )
    .expect("drift design");
}

/// **`supersede` stays whole under contention, through the other writer**
/// (`A1-230`, `A1-599`).
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
/// Proof of `ids.sequential_survives_concurrent_writers`: each real process
/// either publishes one uniquely owned successor or reports the production
/// bounded refusal, and the plan remains loadable either way.
#[test]
fn concurrent_supersedes_are_complete_or_actionably_refused() {
    let tmp = tempfile::Builder::new()
        .prefix("id-race-supersede")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let dir = tmp.path();
    closed_fixture(dir, 8);
    let predecessors_before = (0..8)
        .map(|writer| {
            std::fs::read_to_string(dir.join(format!("aplan/tasks/T{writer:04}.md")))
                .expect("predecessor before race")
        })
        .collect::<Vec<_>>();

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
                let said = out.text();
                let id = (out.code == 0).then(|| {
                    said.lines()
                        .find_map(|l| l.rsplit_once(": filed "))
                        .and_then(|(_, id)| id.split_whitespace().next())
                        .map(str::to_string)
                });
                (writer, out.code, id.flatten(), said)
            })
        })
        .collect();
    let results: Vec<(usize, i32, Option<String>, String)> = handles
        .into_iter()
        .map(|h| h.join().expect("writer thread"))
        .collect();
    let mut landed = BTreeSet::new();
    for (writer, code, id, said) in &results {
        let title = format!("Successor {writer}");
        if *code == 0 {
            let id = id.as_ref().unwrap_or_else(|| {
                panic!("supersede {writer} exited zero without reporting an id:\n{said}")
            });
            assert!(
                landed.insert(id.clone()),
                "two supersedes reported successor `{id}`"
            );
            let body = std::fs::read_to_string(dir.join(format!("aplan/tasks/{id}.md")))
                .unwrap_or_else(|e| panic!("supersede {writer} reported `{id}`: {e}"));
            assert!(
                has_title(&body, &title),
                "supersede {writer} reported `{id}`, but it owns another body:\n{body}"
            );
            assert_eq!(
                title_hits(dir, &title),
                1,
                "successful supersede {writer} did not land exactly once"
            );
        } else {
            assert!(id.is_none(), "a refused supersede reported an id: {said}");
            assert_actionable_refusal("supersede", said);
            assert_eq!(
                title_hits(dir, &title),
                0,
                "refused supersede {writer} left a successor behind"
            );
        }
        let predecessor = std::fs::read_to_string(dir.join(format!("aplan/tasks/T{writer:04}.md")))
            .expect("predecessor after race");
        if *code == 0 {
            assert_ne!(
                predecessor, predecessors_before[*writer],
                "successful supersede {writer} did not publish its predecessor restamp"
            );
        } else {
            assert_eq!(
                predecessor, predecessors_before[*writer],
                "refused supersede {writer} changed its predecessor"
            );
        }
    }
    assert!(!landed.is_empty(), "no concurrent supersede made progress");

    // Eight predecessors plus exactly the successors reported as complete.
    // A batch that partly applied or overwrote another's file changes this
    // count, even on a loaded host where a writer may refuse at its deadline.
    let on_disk = std::fs::read_dir(dir.join("aplan/tasks"))
        .expect("read plan dir")
        .count();
    assert_eq!(
        on_disk,
        8 + landed.len(),
        "the plan holds {on_disk} files for {} successful supersedes",
        landed.len()
    );
    let (said, code) = common::aplan(dir, &["check"]);
    assert_eq!(code, 0, "the plan does not load after the race:\n{said}");
}

/// **`add --from` stays all-or-nothing and re-plans the whole batch**
/// (`A1-230`, `A1-599`).
///
/// The hardest of the three, and the one most worth pinning. A manifest is
/// all-or-nothing: an entry may join a cut an earlier entry creates, so losing
/// one id means re-reading and planning the *batch* again rather than re-minting
/// one entry. Eight writers of two entries each is sixteen ids contending.
/// Every process must either add both titles exactly once or add neither and
/// return the bounded actionable refusal; the final plan must load either way.
///
/// Proof of `ids.sequential_survives_concurrent_writers`: the batch is
/// re-planned rather than partly applied, without making host throughput part
/// of the correctness verdict.
#[test]
fn concurrent_manifests_are_whole_exact_once_or_actionably_refused() {
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
                (writer, out.code, out.text())
            })
        })
        .collect();
    let results: Vec<(usize, i32, String)> = handles
        .into_iter()
        .map(|h| h.join().expect("writer thread"))
        .collect();
    let mut landed = 0usize;
    let mut ids = BTreeSet::new();
    for (writer, code, said) in &results {
        let reported = said
            .lines()
            .filter_map(|line| line.strip_prefix("wrote "))
            .filter_map(|line| line.split_whitespace().next())
            .collect::<Vec<_>>();
        if *code == 0 {
            landed += 1;
            assert_eq!(
                reported.len(),
                2,
                "manifest {writer} exited zero without reporting two tasks:\n{said}"
            );
            for id in &reported {
                assert!(
                    ids.insert(id.to_string()),
                    "two manifest entries reported task `{id}`"
                );
            }
        } else {
            assert!(
                reported.is_empty(),
                "refused manifest {writer} reported written tasks:\n{said}"
            );
            assert_actionable_refusal("add --from", said);
        }

        for half in ["first", "second"] {
            let title = format!("Manifest {writer} {half}");
            assert_eq!(
                title_hits(dir, &title),
                usize::from(*code == 0),
                "manifest {writer} did not publish all-or-nothing: `{title}`"
            );
            if *code == 0 {
                assert!(
                    reported.iter().any(|id| {
                        std::fs::read_to_string(dir.join(format!("aplan/tasks/{id}.md")))
                            .is_ok_and(|body| body.contains(&title))
                    }),
                    "manifest {writer} reported two ids, but neither owns `{title}`:\n{said}"
                );
            }
        }
    }
    assert!(landed > 0, "no concurrent manifest made progress");
    assert_eq!(ids.len(), landed * 2, "manifest ids were not distinct");

    // Three seeds plus two tasks for every successful batch. A batch applied
    // twice or half applied shows up here before `check` ever sees it.
    let on_disk = std::fs::read_dir(dir.join("aplan/tasks"))
        .expect("read plan dir")
        .count();
    assert_eq!(
        on_disk,
        3 + landed * 2,
        "the plan holds {on_disk} files for {landed} successful manifests"
    );
    let (said, code) = common::aplan(dir, &["check"]);
    assert_eq!(code, 0, "the plan does not load after the race:\n{said}");
}
