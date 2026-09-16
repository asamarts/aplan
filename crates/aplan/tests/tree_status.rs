//! `tree` selects by status, because depth is not the axis (`A1-217`).
//!
//! `tree` is the one view of the graph a person could already read, and its only
//! control was depth. Depth has no usable middle: on this plan the step from one
//! level to two was eight lines to a hundred and forty-nine, and 133 of those
//! were `done`. A `done` task cannot block anything -- that is exactly what
//! `ready` computes -- so every one of those lines was a constraint already
//! satisfied, printed to somebody asking what was in their way.

use std::path::Path;

mod common;

/// A chain of finished work under one live task, plus a dropped blocker, which
/// is the case a status filter must *not* hide.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    let done = |id: &str, blocked: &str| {
        format!(
            "---\nid: {id}\ntitle: finished {id}\nphase: 0\nstatus: done\nowner: t\n\
             effort_days: 1\n{blocked}evidence:\n  - kind: note\n    ref: seeded\n    \
             captured: 2026-01-01\n---\n\nbody\n"
        )
    };
    let files = [
        ("T-001", done("T-001", "")),
        ("T-002", done("T-002", "blocked_by: [T-001]\n")),
        ("T-003", done("T-003", "blocked_by: [T-002]\n")),
        (
            "T-004",
            "---\nid: T-004\ntitle: dropped, and still in the way\nphase: 0\nstatus: dropped\n\
             owner: t\neffort_days: 1\n---\n\nbody\n"
                .to_string(),
        ),
        (
            "T-005",
            "---\nid: T-005\ntitle: the live one a reader asks about\nphase: 0\nstatus: todo\n\
             owner: t\neffort_days: 1\nblocked_by: [T-003, T-004]\n---\n\nbody\n"
                .to_string(),
        ),
    ];
    for (id, body) in files {
        std::fs::write(dir.join(format!("aplan/tasks/{id}.md")), body).expect("task");
    }
}

fn run(dir: &Path, args: &[&str]) -> String {
    let mut all: Vec<&str> = vec!["tree"];
    all.extend(args);
    let out = common::output(dir, &all);
    assert!(
        out.status.success(),
        "`aplan {}` failed: {}",
        all.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The ancestry a reader is shown is the part that can still block.
#[test]
fn live_hides_the_satisfied_ancestry() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let all = run(dir.path(), &["T-005"]);
    for id in ["T-001", "T-002", "T-003"] {
        assert!(all.contains(id), "the default no longer shows {id}:\n{all}");
    }

    let live = run(dir.path(), &["T-005", "--live"]);
    for id in ["T-001", "T-002", "T-003"] {
        assert!(
            !live.contains(id),
            "`--live` still shows finished {id}:\n{live}"
        );
    }
    assert!(
        live.lines().count() < all.lines().count(),
        "`--live` printed no fewer lines:\n{live}"
    );
}

/// `dropped` does not satisfy a dependency: a dropped blocker means its
/// dependents can never legitimately start, so it is still in the way and must
/// survive the filter. Hiding it would be the filter answering a different
/// question from the one it was given.
#[test]
fn a_dropped_blocker_is_not_hidden_by_live() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let live = run(dir.path(), &["T-005", "--live"]);
    assert!(
        live.contains("T-004"),
        "a dropped blocker was hidden as though it were satisfied:\n{live}"
    );
}

/// What was withheld is named, and the number is the truth.
#[test]
fn what_the_filter_withheld_is_named_and_counted() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let live = run(dir.path(), &["T-005", "--live"]);
    assert!(
        live.contains("3 satisfied tasks not shown, past --live"),
        "the count is wrong or missing; T-001..T-003 are the three:\n{live}"
    );

    // And the number is a difference, not a guess: the ids the default shows
    // and this one does not.
    let all = run(dir.path(), &["T-005"]);
    let hidden = ["T-001", "T-002", "T-003"]
        .iter()
        .filter(|id| all.contains(**id) && !live.contains(**id))
        .count();
    assert_eq!(hidden, 3, "the fixture stopped exercising this");
}

/// Both filters at once, which is where a tally would have started lying.
///
/// A satisfied task below a `--depth` cut is withheld by both, and a count kept
/// during the walk would either double it or argue with the depth sentence about
/// whose it was. Each sentence is true of the view actually printed.
#[test]
fn both_filters_together_say_true_things() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let both = run(dir.path(), &["T-005", "--depth", "1", "--live"]);
    let depth_only = run(dir.path(), &["T-005", "--depth", "1"]);

    // The depth-limited default cuts below level one and says so.
    assert!(
        depth_only.contains("past --depth"),
        "the fixture no longer reaches a depth cut:\n{depth_only}"
    );

    // With the filter on, the branch that would have been cut is gone before
    // the cut applies, so claiming a depth cut would be false.
    assert!(
        !both.contains("past --depth"),
        "it reports a depth cut that did not happen in the view it printed:\n{both}"
    );
    assert!(
        both.contains("satisfied task"),
        "it does not say the filter withheld anything:\n{both}"
    );

    // No line claims two different totals for one view.
    let counts: Vec<&str> = both
        .lines()
        .filter(|l| l.contains("not shown") || l.contains("past --depth"))
        .collect();
    assert_eq!(counts.len(), 1, "two elision sentences at once: {counts:?}");
}

/// The default did not move, which is what `A1-217`'s third criterion asks: a
/// script reading `tree` sees what it saw before, and the runway for changing
/// that is declared in the plan rather than taken here.
#[test]
fn the_default_is_unchanged() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let out = run(dir.path(), &["T-005"]);
    for id in ["T-001", "T-002", "T-003", "T-004"] {
        assert!(out.contains(id), "the default stopped showing {id}:\n{out}");
    }
    assert!(
        !out.contains("past --live"),
        "the default mentions a filter it did not apply:\n{out}"
    );
}

/// A satisfied task is walked *through*, not skipped over.
///
/// The first version suppressed a hidden task's whole subtree, so live work
/// downstream of finished work vanished -- and was then counted as "satisfied",
/// which it was not. On this repository's own plan `tree A0-001 --live` printed
/// nothing under `releases` and claimed 117 satisfied tasks, 46 of which were
/// live.
#[test]
fn live_work_below_finished_work_is_still_shown() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // T-001 (done) -> T-002 (done) -> T-003 (done) -> T-005 (todo).
    let releases = run(dir.path(), &["T-001", "--live"]);
    let below = releases
        .split_once("releases:")
        .map(|(_, r)| r.to_string())
        .unwrap_or_default();

    assert!(
        below.contains("T-005"),
        "live work below finished work was suppressed with it:\n{releases}"
    );
    for done in ["T-001", "T-002", "T-003"] {
        assert!(
            !below.contains(&format!("{done}  done")),
            "{done} is finished and was printed anyway:\n{releases}"
        );
    }
}

/// Everything the filter counts as satisfied is satisfied.
///
/// The count and the hiding are one decision, so a task hidden for another
/// reason would be reported under a word that does not describe it.
#[test]
fn the_count_names_only_tasks_that_are_actually_satisfied() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    for id in ["T-001", "T-005"] {
        let all = run(dir.path(), &[id]);
        let live = run(dir.path(), &[id, "--live"]);

        let ids = |text: &str| -> std::collections::BTreeSet<String> {
            text.lines()
                .filter_map(|l| l.split_whitespace().next())
                .filter(|w| w.starts_with("T-"))
                .map(str::to_string)
                .collect()
        };
        let hidden: Vec<String> = ids(&all).difference(&ids(&live)).cloned().collect();

        // Every hidden id is one of the three finished tasks; the dropped one
        // and the live one are never hidden.
        for h in &hidden {
            assert!(
                ["T-001", "T-002", "T-003"].contains(&h.as_str()),
                "`{h}` was hidden by `--live` and is not satisfied:\n{live}"
            );
        }
        // And nothing appears that the default did not show.
        assert!(
            ids(&live).difference(&ids(&all)).count() == 0,
            "`--live` invented a row the default does not have:\n{live}"
        );
    }
}

/// Both elisions at once, each counting a different thing.
///
/// A live chain with one finished task in the middle, cut at depth two. The
/// finished task is transparent: it is not printed, it does not consume a level,
/// and the live work under it moves up into view. So the depth sentence counts
/// what is past the cut and the status sentence counts what is satisfied, and
/// neither claims the other's tasks.
#[test]
fn both_elisions_appear_and_count_different_things() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");

    let write = |id: &str, body: String| {
        std::fs::write(dir.path().join(format!("aplan/tasks/{id}.md")), body).expect("task");
    };
    write(
        "L-1",
        "---\nid: L-1\ntitle: root live\nphase: 0\nstatus: todo\nowner: t\n\
         effort_days: 1\n---\n\nbody\n"
            .to_string(),
    );
    write(
        "D-1",
        "---\nid: D-1\ntitle: finished middle\nphase: 0\nstatus: done\nowner: t\n\
         effort_days: 1\nblocked_by: [L-1]\nevidence:\n  - kind: note\n    ref: seeded\n    \
         captured: 2026-01-01\n---\n\nbody\n"
            .to_string(),
    );
    for (id, prev) in [("L-2", "D-1"), ("L-3", "L-2"), ("L-4", "L-3")] {
        write(
            id,
            format!(
                "---\nid: {id}\ntitle: live {id}\nphase: 0\nstatus: todo\nowner: t\n\
                 effort_days: 1\nblocked_by: [{prev}]\n---\n\nbody\n"
            ),
        );
    }

    let both = run(dir.path(), &["L-1", "--depth", "2", "--live"]);

    // The finished task is transparent, so the live work under it moves up.
    assert!(
        !both.contains("D-1"),
        "the finished task was printed:\n{both}"
    );
    assert!(both.contains("L-2") && both.contains("L-3"), "{both}");

    // Two sentences, each about its own tasks: D-1 is satisfied, L-4 is past
    // the cut, and neither count includes the other's.
    assert!(
        both.contains("1 satisfied task not shown, past --live"),
        "the status elision is wrong or missing:\n{both}"
    );
    assert!(
        both.contains("1 more task below, past --depth"),
        "the depth elision is wrong or missing:\n{both}"
    );

    // And without the filter the finished task *does* consume a level, so less
    // of the chain is visible: the filter buys a level rather than costing one.
    let depth_only = run(dir.path(), &["L-1", "--depth", "2"]);
    assert!(depth_only.contains("D-1"), "{depth_only}");
    assert!(
        !depth_only.contains("L-3"),
        "the fixture no longer shows the filter buying a level:\n{depth_only}"
    );
    assert!(
        depth_only.contains("2 more tasks below, past --depth"),
        "{depth_only}"
    );
}
