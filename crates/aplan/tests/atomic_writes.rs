//! A move is one change, however many files it touches (`A1-234`).
//!
//! `mv` placed the task in the destination cut and then took it out of the
//! source. A failure between the two left the task in *both*, and `APLAN022`
//! reports that only where the repository asked for `require_cut_membership` --
//! so the ordinary outcome was a plan somebody had to repair by hand, with
//! nothing saying so.
//!
//! That was rare while "a file that changed under us" meant a permission problem
//! or a person editing by hand. `A1-226` made the same branch reachable by
//! **ordinary contention**: another writer holding the lock for the two-second
//! wait, or having written the file since this command read it. Under a fleet
//! that is an ordinary Tuesday.

use std::path::Path;

mod common;

/// Two cuts, one holding two tasks so moving one out does not empty it -- an
/// empty cut is refused before any write, which would test the refusal rather
/// than the write.
fn two_cuts(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    for n in 1..=2 {
        std::fs::write(
            dir.join(format!("aplan/tasks/T-00{n}.md")),
            format!(
                "---\nid: T-00{n}\ntitle: work {n}\nphase: 0\nstatus: todo\nowner: k\n\
                 effort_days: 1\n---\n\nbody\n"
            ),
        )
        .expect("task");
    }
    std::fs::write(
        dir.join("aplan/tasks/M-a.md"),
        "---\nid: M-a\ntitle: cut a\nphase: 0\nstatus: todo\ntags: [milestone, a]\n\
         blocked_by: [T-001, T-002]\n---\n\nbody\n",
    )
    .expect("milestone");
    std::fs::write(
        dir.join("aplan/tasks/M-b.md"),
        "---\nid: M-b\ntitle: cut b\nphase: 0\nstatus: todo\ntags: [milestone, b]\n\
         blocked_by: [T-002]\n---\n\nbody\n",
    )
    .expect("milestone");
}

fn holds(dir: &Path, milestone: &str, id: &str) -> bool {
    std::fs::read_to_string(dir.join(format!("aplan/tasks/{milestone}.md")))
        .expect("read")
        .contains(id)
}

/// **A move across a locked source changes nothing at all.**
///
/// The reproduction the task names: lock one milestone, move across it, and the
/// plan must be unchanged rather than half-changed. Before this the destination
/// was already written by the time the source failed, and the message had to
/// describe a task in two cuts.
#[test]
fn a_move_across_a_locked_cut_leaves_the_plan_whole() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    two_cuts(dir.path());

    // The lock `aplan_core::save` takes, held by somebody else. Fresh, so it is
    // a live writer rather than a stale one to be broken.
    let lock = dir.path().join("aplan/tasks/.M-a.md.lock");
    std::fs::write(&lock, "").expect("lock");

    let out = common::run(dir.path(), &["mv", "T-001", "--cut", "M-b"]);
    assert_eq!(out.code, 2, "the move was not refused: {out:?}");
    assert!(
        out.text().contains("nothing was written"),
        "the refusal must say the plan is untouched: {}",
        out.text()
    );
    assert!(
        !holds(dir.path(), "M-b", "T-001"),
        "the destination was written while the source could not be"
    );
    assert!(
        holds(dir.path(), "M-a", "T-001"),
        "the source lost the task anyway"
    );

    // And with the lock gone the same move completes, so the refusal above is
    // about contention and not about the move being impossible.
    std::fs::remove_file(&lock).expect("unlock");
    let out = common::run(dir.path(), &["mv", "T-001", "--cut", "M-b"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(holds(dir.path(), "M-b", "T-001") && !holds(dir.path(), "M-a", "T-001"));
}

/// The same for `drop --cut`, which the task asked be checked for the shape.
///
/// It had a worse one: every file went through `std::fs::write`, so there was no
/// comparison (a concurrent editor's change discarded in silence, which is
/// exactly what `A1-226` measured), no lock, and no atomic rename -- and an
/// interrupted write on a *milestone* truncates it, orphaning every member of
/// the cut at once.
#[test]
fn dropping_a_cut_against_a_locked_member_changes_nothing() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    two_cuts(dir.path());

    let lock = dir.path().join("aplan/tasks/.T-002.md.lock");
    std::fs::write(&lock, "").expect("lock");

    let out = common::run(dir.path(), &["drop", "M-a", "--cut"]);
    assert_eq!(out.code, 2, "the drop was not refused: {out:?}");
    assert!(out.text().contains("nothing was written"), "{}", out.text());
    for id in ["T-001", "T-002", "M-a"] {
        let body =
            std::fs::read_to_string(dir.path().join(format!("aplan/tasks/{id}.md"))).expect("read");
        assert!(
            !body.contains("dropped"),
            "`{id}` was cancelled while a sibling could not be: {body}"
        );
    }
}

/// **A move that changes a cut and a phase is one change too.**
///
/// The cut writes went through the batch and the phase writes were raw
/// `std::fs::write` calls after it, so `mv --cut X --phase N` could land the cut
/// and half the phases -- a partial move inside the command whose help says
/// "editing both milestones or neither" (`A1-234`, audit).
#[test]
fn a_move_that_also_changes_phase_is_all_or_nothing() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    two_cuts(dir.path());

    // The *task* file locked, which only the phase edit touches. The cut writes
    // would have succeeded before this.
    let lock = dir.path().join("aplan/tasks/.T-001.md.lock");
    std::fs::write(&lock, "").expect("lock");

    let out = common::run(dir.path(), &["mv", "T-001", "--cut", "M-b", "--phase", "3"]);
    assert_eq!(out.code, 2, "the move was not refused: {out:?}");
    assert!(out.text().contains("nothing was written"), "{}", out.text());
    assert!(
        !holds(dir.path(), "M-b", "T-001"),
        "the cut was written while the phase could not be"
    );
    let task = std::fs::read_to_string(dir.path().join("aplan/tasks/T-001.md")).expect("read");
    assert!(task.contains("phase: 0"), "the phase moved anyway: {task}");

    std::fs::remove_file(&lock).expect("unlock");
    let out = common::run(dir.path(), &["mv", "T-001", "--cut", "M-b", "--phase", "3"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(holds(dir.path(), "M-b", "T-001"));
    let task = std::fs::read_to_string(dir.path().join("aplan/tasks/T-001.md")).expect("read");
    assert!(task.contains("phase: 3"), "{task}");
}

/// A plan of two tasks in a git repository, with `T-001` in whatever state the
/// case under test needs: `close` wants it live, `amend` and `supersede` want it
/// `done` and already citing something.
fn one_task_repo(dir: &Path, done: bool) -> String {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("src")).expect("src");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.join("src/a.rs"), "fn main() {}\n").expect("src file");
    std::fs::write(
        dir.join("aplan/tasks/T-002.md"),
        "---\nid: T-002\ntitle: work 2\nphase: 0\nstatus: todo\nowner: k\n\
         effort_days: 1\npaths:\n- src/**\n---\n\n## Acceptance criteria\n\n- [x] done\n",
    )
    .expect("task");
    for args in [
        &["init", "-q", "."][..],
        &["config", "user.email", "t@example.com"],
        &["config", "user.name", "t"],
    ] {
        run_git(dir, args);
    }
    // `T-001` last and separately, so a `done` one can cite the commit that
    // touched the path it owns rather than a commit that predates the file.
    let head = |dir: &Path| -> String {
        let out = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .expect("git rev-parse");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    run_git(dir, &["add", "-A"]);
    run_git(dir, &["commit", "-q", "-m", "work"]);
    let sha = head(dir);
    let evidence = if done {
        format!("evidence:\n  - kind: commit\n    ref: {sha}\n")
    } else {
        String::new()
    };
    let status = if done { "done" } else { "in_progress" };
    std::fs::write(
        dir.join("aplan/tasks/T-001.md"),
        format!(
            "---\nid: T-001\ntitle: work 1\nphase: 0\nstatus: {status}\nowner: k\n\
             effort_days: 1\npaths:\n- src/**\n{evidence}---\n\n\
             ## Acceptance criteria\n\n- [x] done\n"
        ),
    )
    .expect("task");
    run_git(dir, &["add", "-A"]);
    run_git(dir, &["commit", "-q", "-m", "the task"]);
    sha
}

fn run_git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?} failed");
}

/// **Every command that edits a task file refuses a locked one** (`A1-246`).
///
/// `A1-226` measured the lost update at twenty-four agents and built
/// `aplan_core::save` to stop it: compare, lock, rename. That writer was reached
/// from the cut-membership module and nowhere else, so `close`, `amend`, `dep`
/// and `supersede` still called `std::fs::write` -- no comparison, so a
/// concurrent editor's change was discarded in silence; no lock; and an
/// interrupted write left a partial task file.
///
/// One case per command, driven from a table so a command added later without a
/// row is visible as an absence rather than passing by omission. Each asserts
/// the refusal *and* that the file is untouched, because a write that fails
/// after truncating is the failure this is about, and then that the same command
/// succeeds once the lock is gone.
#[test]
fn every_command_that_edits_a_task_refuses_a_locked_file() {
    // (command, argv, task must be `done` first, what success produces)
    //
    // The observable differs by command, and saying so is the point.
    // `supersede`'s edit to the predecessor is a re-stamp, which on a task
    // tracing nothing rewrites identical bytes -- so its visible product is the
    // successor file, and asserting on `T-001` would pass whether or not the
    // command ran at all.
    enum Produces {
        /// `T-001` itself is rewritten.
        AChangedTask,
        /// A new task file appears at this path.
        ANewFile(&'static str),
    }
    let cases: &[(&str, &[&str], bool, Produces)] = &[
        (
            "close",
            &["close", "T-001", "--evidence", "commit:SHA"],
            false,
            Produces::AChangedTask,
        ),
        (
            "dep",
            &["dep", "add", "T-001", "--after", "T-002"],
            false,
            Produces::AChangedTask,
        ),
        (
            "amend",
            &["amend", "T-001", "--add-evidence", "file:src/a.rs"],
            true,
            Produces::AChangedTask,
        ),
        (
            "supersede",
            &["supersede", "T-001", "the next one"],
            true,
            Produces::ANewFile("aplan/tasks/T-003.md"),
        ),
    ];
    for (name, args, done, produces) in cases {
        let dir = tempfile::Builder::new()
            .prefix("locked-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let sha = one_task_repo(dir.path(), *done);
        let target = dir.path().join("aplan/tasks/T-001.md");
        let before = std::fs::read_to_string(&target).expect("read");

        let filled: Vec<String> = args.iter().map(|a| a.replace("SHA", &sha)).collect();
        let argv: Vec<&str> = filled.iter().map(String::as_str).collect();

        // The lock `aplan_core::save` takes, held by somebody else.
        let lock = dir.path().join("aplan/tasks/.T-001.md.lock");
        std::fs::write(&lock, "").expect("lock");

        let out = common::run(dir.path(), &argv);
        assert_eq!(out.code, 2, "`{name}` was not refused: {out:?}");
        assert!(
            out.text().contains("nothing was written"),
            "`{name}` must say the plan is untouched: {}",
            out.text()
        );
        assert_eq!(
            std::fs::read_to_string(&target).expect("read"),
            before,
            "`{name}` changed the file it said it had not written"
        );
        if let Produces::ANewFile(rel) = produces {
            assert!(
                !dir.path().join(rel).exists(),
                "`{name}` said nothing was written and left {rel} behind"
            );
        }

        // And the same command completes once the lock is gone, so the refusal
        // is about contention rather than about the command being impossible.
        std::fs::remove_file(&lock).expect("unlock");
        let out = common::run(dir.path(), &argv);
        assert_eq!(out.code, 0, "`{name}` failed without the lock: {out:?}");
        match produces {
            Produces::AChangedTask => assert_ne!(
                std::fs::read_to_string(&target).expect("read"),
                before,
                "`{name}` reported success and wrote nothing"
            ),
            Produces::ANewFile(rel) => assert!(
                dir.path().join(rel).exists(),
                "`{name}` reported success and produced no {rel}"
            ),
        }
    }
}

/// **Nothing writes a plan file except through `aplan_core::save`** (`A1-246`).
///
/// `A1-226` fixed one call site and the rule did not spread: five commands grew
/// or kept a raw write, and `A1-234` found a sixth a month later. A scan is what
/// would have said so on the commit that added the next one.
///
/// The two exemptions are artefact writers, not plan writers: they own a file
/// this tool generates for somebody else, where there is no concurrent author to
/// lose and the whole file is the unit.
///
/// **What this does not reach**, stated rather than implied: it scans this
/// crate's `src` only. `aplan-mcp` writes task files too, still with
/// `std::fs::write`, and is the surface `A1-226` actually measured the defect on
/// -- twenty-four agents, and an agent reaches this tool through MCP. Tracked as
/// `A1-270`, whose first criterion is that this scan grows to cover it and fails
/// on that crate today. `aplan-interop`'s creates are safe by construction
/// (`create_new`), which is why they are not in that task.
#[test]
fn every_plan_write_goes_through_save() {
    // **Every crate that writes a plan file, not just this one** (`A1-270`).
    // Scoping this to the binary crate is exactly why `aplan-mcp` kept the
    // defect through `A1-246`: the scan could not see the surface `A1-226`
    // measured the defect on.
    //
    // `aplan-interop` is not here and the reason is in its code rather than in
    // this list: `emit::write_task` opens with `create_new(true)`, which cannot
    // clobber because it fails if the file exists, and its cut edits go through
    // `save` already.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // **Every crate, discovered rather than listed.** A hand-kept list is the
    // weakness that let `aplan-mcp` keep this defect through `A1-246`: the scan
    // named the crates somebody had thought of. Walking the workspace means a
    // crate that grows its first plan write is covered on the commit that adds
    // it, which is the whole point of the audit (`A1-270`'s audit).
    let crates: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("crates"))
        .expect("crates")
        .flatten()
        .map(|e| e.path().join("src"))
        .filter(|p| p.is_dir())
        .chain(std::iter::once(root.join("xtask/src")))
        .collect();
    // **One, and the list is the argument.** Two artefact writers were exempt
    // here and both were wrong: `roadmap.json` is *merged* rather than rendered
    // (`A1-033`) and `AGENTS.md` keeps the repository's own prose around a
    // generated block, so both had a human co-author and both did
    // read-modify-write. They are routed through `save` now. An exemption whose
    // reason does not survive being written down was not an exemption
    // (`A1-246`'s audit).
    //
    // The one that survives it is a different shape, which is why it reads
    // differently below.
    let exempt = [
        (
            "write_plan",
            "`aplan-e2e`'s scale generator (`A1-229`). It creates a whole plan from nothing \
             inside a directory it just made, so there is no prior content to preserve and no \
             concurrent author to lose -- the same reason `write_or_check` gives. Routing 24,000 \
             fresh files through a compare-lock-rename would also make the benchmark measure \
             `save` rather than the `check` it exists to time",
        ),
        (
            "write_or_check",
            "xtask's generated artefacts -- `ROADMAP.status.md`, `schemas/v1/*.json`, the rule          catalogue. Whole-file renders whose contract is the opposite of a merge: a hand edit          is caught by `--check` and reported as drift, not content to preserve. There is no          concurrent author to lose, which is the reason the two `export-*` writers could not          give (`A1-270`'s audit)",
        ),
        (
            "cgroup_knob_is_writable",
            "the same cgroup control files, opened to prove the kernel would take a write \
             before a command depends on it (`A1-310`). It writes nothing, and never touches a \
             plan file",
        ),
        (
            "write_cgroup_knob",
            "a cgroup control file (`A1-310`). These are kernel pseudo-files: the write is the \
             operation, there is nothing to rename onto, and a partial write is rejected by the \
             kernel rather than torn. It never touches a plan file",
        ),
    ];
    let mut raw = Vec::new();
    let mut scanned = 0usize;
    for entry in crates
        .iter()
        .flat_map(|d| std::fs::read_dir(d).expect("crate src").flatten())
    {
        let path = entry.path();
        if path.extension().is_some_and(|x| x == "rs") {
            scanned += 1;
            let text = std::fs::read_to_string(&path).expect("read");
            let mut current = String::new();
            // Production only. A fixture writing its own temp files is not the
            // hazard, and `mod tests` is where most of them are -- the first
            // pass at this scan reported four of them in `test_policy`, which
            // is a helper it had misattributed anyway.
            let production = text
                .split_once("\n#[cfg(test)]")
                .map_or(text.as_str(), |(before, _)| before);
            for (n, line) in production.lines().enumerate() {
                if let Some(rest) = line
                    .strip_prefix("fn ")
                    .or_else(|| line.strip_prefix("pub fn "))
                {
                    current = rest.split('(').next().unwrap_or_default().to_string();
                }
                // The call, not a mention of it: these files discuss the defect
                // in prose, and a comment naming it is the opposite of one.
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") || trimmed.starts_with("///") {
                    continue;
                }
                // Every way to put bytes in a file, not just the one this
                // defect happened to use. `fs::write` is what the six call
                // sites were; `File::create` and an `OpenOptions` that creates
                // or truncates are the same hazard spelled differently, and a
                // gate that only knew the first would pass the day somebody
                // wrote the second (`A1-246`'s audit).
                //
                // A bare `.write(true).open()` is not a write: three of those
                // are permission *probes*, which open without modifying so the
                // refusal names the file before the work rather than after.
                let opens_to_write = line.contains("OpenOptions")
                    && production
                        .lines()
                        .skip(n)
                        .take(6)
                        .any(|l| l.contains(".create(") || l.contains(".truncate("));
                let writes =
                    line.contains("fs::write(") || line.contains("File::create(") || opens_to_write;
                if writes && !exempt.iter().any(|(f, _)| *f == current) {
                    raw.push(format!(
                        "{}:{} in `{current}`",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        n + 1
                    ));
                }
            }
        }
    }
    assert!(
        raw.is_empty(),
        "a plan file is written without the compare-lock-rename `aplan_core::save` gives \
         (`A1-226`, `A1-246`). Route it through `save_one` or `save_batch`, or name it exempt \
         here with the reason it owns its file:\n  {}",
        raw.join("\n  ")
    );
    // A scan that reached no files would pass for the wrong reason, which is
    // the shape this whole file exists to refuse.
    assert!(
        scanned >= 20,
        "only {scanned} file(s) scanned, so this audit is not reaching the workspace"
    );
    // The exemptions must still name real functions, or this audit narrows
    // itself every time one is renamed. Empty today, and kept so the next
    // exemption has to say why in a place somebody reviews.
    let all: String = crates
        .iter()
        .flat_map(|d| std::fs::read_dir(d).expect("crate src").flatten())
        .filter(|e| e.path().extension().is_some_and(|x| x == "rs"))
        .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
        .collect();
    for (f, why) in exempt {
        assert!(
            all.contains(&format!("fn {f}(")),
            "`{f}` is exempt ({why}) and no longer exists"
        );
    }
}

/// **The lost update itself, not just the lock** (`A1-246`).
///
/// The test above holds the lock, which proves the command reaches
/// `aplan_core::save`. It does not prove the command hands `save` the *right*
/// `expected`: a call site passing the text it was about to write, or an empty
/// string, would lock correctly and still clobber. That is the defect `A1-226`
/// measured -- a concurrent editor's change discarded in silence -- and this is
/// the test for it.
///
/// The concurrency is deterministic rather than raced. `close` resolves its
/// citation before writing, and resolving a `test` citation runs the configured
/// runner's `list` command; so a runner that edits the task file puts a real
/// third-party change exactly between the read and the write, every time.
#[test]
fn a_concurrent_edit_is_refused_rather_than_discarded() {
    let dir = tempfile::Builder::new()
        .prefix("lost-update-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    one_task_repo(dir.path(), false);

    let tamper = dir.path().join("tamper.sh");
    std::fs::write(
        &tamper,
        "#!/bin/sh\nprintf '\\n<!-- somebody else was here -->\\n' >> aplan/tasks/T-001.md\n\
         echo the-name\n",
    )
    .expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&tamper, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ntest:\n  list: [\"./tamper.sh\"]\n  run: [\"./tamper.sh\", \"{test}\"]\n",
    )
    .expect("config");

    let out = common::run(
        dir.path(),
        &["close", "T-001", "--evidence", "test:the-name"],
    );
    assert_eq!(out.code, 2, "the closure was not refused: {out:?}");
    assert!(
        out.text().contains("changed while this was being prepared"),
        "and must say why, so the author knows to re-read rather than retry blind: {}",
        out.text()
    );

    let after = std::fs::read_to_string(dir.path().join("aplan/tasks/T-001.md")).expect("read");
    assert!(
        after.contains("somebody else was here"),
        "the other writer's change was discarded, which is the whole defect"
    );
    assert!(
        !after.contains("status: done"),
        "and the closure landed anyway: {after}"
    );
}

/// **`supersede --cut` places the successor, and places it atomically.**
///
/// The cut edit joined the batch in `A1-246`, so it is a third file that must
/// land with the other two or not at all. Nothing else covers `--cut` on this
/// command, and the placement was rewritten from `write_task_placed` to
/// `cut::plan_placement` in the same change.
#[test]
fn supersede_places_the_successor_in_a_cut_or_writes_nothing() {
    let dir = tempfile::Builder::new()
        .prefix("supersede-cut-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    one_task_repo(dir.path(), true);
    std::fs::write(
        dir.path().join("aplan/tasks/M-a.md"),
        "---\nid: M-a\ntitle: a cut\nphase: 0\nstatus: todo\ntags: [milestone]\n\
         blocked_by: [T-002]\n---\n\nbody\n",
    )
    .expect("milestone");

    // Locked predecessor: the successor must not appear and the cut must not
    // gain a member.
    let lock = dir.path().join("aplan/tasks/.T-001.md.lock");
    std::fs::write(&lock, "").expect("lock");
    let out = common::run(
        dir.path(),
        &["supersede", "T-001", "the next one", "--cut", "M-a"],
    );
    assert_eq!(out.code, 2, "not refused: {out:?}");
    assert!(
        !dir.path().join("aplan/tasks/T-003.md").exists(),
        "the successor was filed while nothing was supposed to be written"
    );
    let cut = std::fs::read_to_string(dir.path().join("aplan/tasks/M-a.md")).expect("read");
    assert!(
        !cut.contains("T-003"),
        "the cut gained a member for a task that does not exist: {cut}"
    );

    // And with the lock gone, all three land.
    std::fs::remove_file(&lock).expect("unlock");
    let out = common::run(
        dir.path(),
        &["supersede", "T-001", "the next one", "--cut", "M-a"],
    );
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        dir.path().join("aplan/tasks/T-003.md").exists(),
        "no successor was filed"
    );
    let cut = std::fs::read_to_string(dir.path().join("aplan/tasks/M-a.md")).expect("read");
    assert!(cut.contains("T-003"), "the cut did not gain it: {cut}");
}

/// **Both surfaces describe the same failure in the same words** (`A1-270`).
///
/// `A1-131`'s rule is that an agent cannot close through MCP what the command
/// refuses, and "cannot" includes being told the same thing when the same
/// condition stops it. Two independent `match` arms would not stay true to that,
/// so `aplan_core::save::advice` is the one place the sentence is written and
/// this pins that both callers reach it.
///
/// What differs is the retry, and only the retry: a person re-runs an
/// invocation, an agent re-runs a tool.
#[test]
fn the_command_and_the_tool_say_the_same_thing_about_a_failed_write() {
    let path = std::path::Path::new("/plan/tasks/T-1.md");
    for err in [
        aplan_core::save::SaveError::Changed {
            path: path.to_path_buf(),
        },
        aplan_core::save::SaveError::Busy {
            path: path.to_path_buf(),
        },
    ] {
        let command = aplan_core::save::advice(&err, "aplan close");
        let tool = aplan_core::save::advice(&err, "aplan_close");
        assert_eq!(
            command.replace("aplan close", "<retry>"),
            tool.replace("aplan_close", "<retry>"),
            "the two surfaces disagree about {err:?}"
        );
        assert!(
            command.contains("nothing was written"),
            "and both must say the plan is untouched: {command}"
        );
    }

    // `Exists` names the file rather than a retry, because a second attempt
    // collides again. Both surfaces inherit that, which is the point of the
    // sentence living in one place.
    let exists = aplan_core::save::advice(
        &aplan_core::save::SaveError::Exists {
            path: path.to_path_buf(),
        },
        "aplan close",
    );
    assert!(
        !exists.contains("again") && exists.contains("the id is taken"),
        "a collision must not be advertised as retryable: {exists}"
    );
}
