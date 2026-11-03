//! `aplan version` with no id: the number the next release must carry
//! (`A1-313`, ADR-0038, ADR-0040, ADR-0041).
//!
//! Driven through the built binary rather than through the arithmetic, for
//! `A1-107`'s reason: the unit tests beside `required` prove the sum, and if the
//! command stopped calling it tomorrow every one of them would still pass. What
//! is under test here is the seam -- the declared command actually spawned, the
//! window actually walked, the refusals actually reached.
//!
//! Not a `trycmd` case: every one of these needs a git repository with a tag in
//! it, and the corpus runs its fixtures un-initialised.

use std::path::Path;

use crate::common;

/// A repository that released `v0.3.0` and has since closed `W-1`, declaring
/// `declares` as the version it ships today.
fn fixture(dir: &Path, declares: &str) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(
        dir.join(".aplan.yml"),
        "root: aplan\nversion:\n  current:\n    command: [sh, ./version.sh]\n",
    )
    .expect("config");
    std::fs::write(dir.join("version.sh"), format!("echo {declares}\n")).expect("version.sh");
    std::fs::write(
        dir.join("aplan/tasks/W-1.md"),
        "---\nid: W-1\ntitle: a task\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
    )
    .expect("task");
    common::git(dir, &["init", "-q", "."]);
    common::git(dir, &["config", "user.email", "t@example.invalid"]);
    common::git(dir, &["config", "user.name", "t"]);
    common::git(dir, &["add", "-A"]);
    common::git(dir, &["commit", "-q", "-m", "before the release"]);
    common::git(dir, &["tag", "v0.3.0"]);
}

/// Close `W-1` with the impact it claims, and commit that.
fn close(dir: &Path, impact: &str) {
    std::fs::write(
        dir.join("aplan/tasks/W-1.md"),
        format!(
            "---\nid: W-1\ntitle: a task\nphase: 1\nstatus: done\neffort_days: 1\n\
             version_impact: {impact}\nevidence:\n  - kind: note\n    ref: shipped\n---\n\nbody\n"
        ),
    )
    .expect("task");
    common::git(dir, &["add", "-A"]);
    common::git(dir, &["commit", "-q", "-m", "the work"]);
}

fn tmp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir")
}

/// **stdout is a version and nothing else**, because both demonstrated
/// consumers are one line of shell from a bare number (`A1-319`) and the window
/// would break them if it shared the stream.
#[test]
fn the_number_is_on_stdout_and_the_window_is_on_stderr() {
    let dir = tmp();
    fixture(dir.path(), "0.3.0");
    close(dir.path(), "breaking");
    let out = common::run(dir.path(), &["version"]);
    assert_eq!(out.code, 0, "{}", out.text());
    assert_eq!(
        out.stdout, "0.4.0\n",
        "a break on top of 0.3.0 is 0.4.0, and stdout carries nothing else"
    );
    assert!(
        out.stderr.contains("v0.3.0") && out.stderr.contains("W-1"),
        "the window and the claim that raised it are named: {}",
        out.stderr
    );
}

/// **A plan whose `root` is declared is read at the tag too** (`A1-313`,
/// audit).
///
/// The defect this pins was invisible in this repository and wrong for almost
/// every adopter. `.aplan.yml` here declares no `root`, so the default relative
/// one is used and the loader's paths are relative; declaring one makes them
/// absolute, because discovery canonicalises the config's directory before
/// joining. The walk stripped the caller's `--repo` -- `.` by default -- which
/// cannot relativise an absolute path, so **no task file was found at the tag
/// and every `done` task counted as newly closed**, silently, with a confident
/// number on top of it.
#[test]
fn a_declared_plan_root_still_reads_the_statuses_at_the_tag() {
    let dir = tmp();
    // The shape that failed: a declared `root`, and a task that was already
    // `done` when the release was tagged.
    std::fs::create_dir_all(dir.path().join("docs/plan/tasks")).expect("mkdir");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: docs/plan/tasks\nversion:\n  current:\n    command: [sh, ./version.sh]\n",
    )
    .expect("config");
    std::fs::write(dir.path().join("version.sh"), "echo 0.3.0\n").expect("version.sh");
    std::fs::write(
        dir.path().join("docs/plan/tasks/W-1.md"),
        "---\nid: W-1\ntitle: already shipped\nphase: 1\nstatus: done\neffort_days: 1\n\
         evidence:\n  - kind: note\n    ref: shipped\n---\n\nbody\n",
    )
    .expect("task");
    common::git(dir.path(), &["init", "-q", "."]);
    common::git(dir.path(), &["config", "user.email", "t@example.invalid"]);
    common::git(dir.path(), &["config", "user.name", "t"]);
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-q", "-m", "before the release"]);
    common::git(dir.path(), &["tag", "v0.3.0"]);
    std::fs::write(dir.path().join("other.txt"), "x\n").expect("file");
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-q", "-m", "something else"]);

    let out = common::run(dir.path(), &["version", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    assert_eq!(
        v["window"]["closed"],
        0,
        "W-1 was already done at the tag: {}",
        out.text()
    );
    assert_eq!(v["owes"], "none");
    assert_eq!(v["version"], "0.3.0", "nothing closed, so nothing moves");
}

/// **The highest version on a commit, not the alphabetically last** (`A1-313`,
/// audit).
///
/// Sorting names puts `v0.9.0` after `v0.10.0`, so a repository that reached a
/// double-digit minor and tagged twice on one commit measured from the older
/// release and answered a version it had already shipped.
#[test]
fn two_release_tags_on_one_commit_take_the_higher_version() {
    let dir = tmp();
    fixture(dir.path(), "0.10.0");
    common::git(dir.path(), &["tag", "v0.10.0"]);
    close(dir.path(), "fix");
    let out = common::run(dir.path(), &["version", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    assert_eq!(v["window"]["since"], "v0.10.0", "{}", out.text());
    assert_eq!(v["version"], "0.10.1");
}

/// **The answer is stable for a plan that has not changed** (`A1-313`,
/// criterion 1).
///
/// Every input is a set or a walk that could have been ordered by the
/// filesystem: the tasks the window holds, the tags on one commit, the member
/// that raised the impact. A number that moved between two runs of the same
/// tree would be unusable as a release gate, and it is exactly the kind of
/// instability nothing else here would notice.
#[test]
fn the_same_tree_answers_the_same_number_every_time() {
    let dir = tmp();
    fixture(dir.path(), "0.3.0");
    close(dir.path(), "feature");
    let first = common::run(dir.path(), &["version"]);
    for _ in 0..4 {
        let again = common::run(dir.path(), &["version"]);
        assert_eq!(again.stdout, first.stdout, "the number moved");
        assert_eq!(again.stderr, first.stderr, "the window moved");
    }
}

/// The staged number is a floor the plan raises, never one it replaces: a tree
/// preparing `0.4.0` and owing a break is `0.4.0`, not `0.5.0`.
#[test]
fn a_staged_pre_release_is_the_release_being_prepared() {
    let dir = tmp();
    fixture(dir.path(), "0.4.0-dev");
    close(dir.path(), "breaking");
    let out = common::run(dir.path(), &["version"]);
    assert_eq!(out.stdout, "0.4.0\n", "{}", out.text());
}

/// The machine form carries the window as a field, which is the same honesty in
/// the format that has nowhere to put a note (ADR-0041).
#[test]
fn the_json_form_carries_the_window() {
    let dir = tmp();
    fixture(dir.path(), "0.3.0");
    close(dir.path(), "feature");
    let out = common::run(dir.path(), &["version", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    assert_eq!(v["version"], "0.3.1");
    assert_eq!(v["current"], "0.3.0");
    assert_eq!(v["owes"], "feature");
    assert_eq!(v["raised_by"], "W-1");
    assert_eq!(v["window"]["kind"], "since_tag");
    assert_eq!(v["window"]["since"], "v0.3.0");
    assert_eq!(v["window"]["closed"], 1);
    assert_eq!(v["schema"], "aplan/version/v1");
}

/// **A plan that models its releases gets its own statement of what ships**,
/// rather than an inference from what happens to have closed (ADR-0038).
#[test]
fn an_open_release_node_is_the_window_instead_of_the_tag() {
    let dir = tmp();
    fixture(dir.path(), "0.3.0");
    std::fs::write(
        dir.path().join("aplan/tasks/M-r.md"),
        "---\nid: M-r\ntitle: the next release\nphase: 1\nstatus: todo\n\
         tags:\n  - milestone\n  - release\nblocked_by:\n  - W-1\n---\n\nbody\n",
    )
    .expect("release");
    close(dir.path(), "breaking");
    let out = common::run(dir.path(), &["version"]);
    assert_eq!(out.stdout, "0.4.0\n", "{}", out.text());
    assert!(
        out.stderr.contains("M-r"),
        "it must say which window it used, and this one is a node: {}",
        out.stderr
    );
}

/// **A release cannot ship a break it does not claim** (`A1-348`).
///
/// A release's members are cuts and a `cross-cutting` task is in none of them,
/// so the hierarchy walk never reaches it: this plan answered `fix` over a
/// declared `breaking` and would have shipped `1.0.1` carrying an incompatible
/// change. Three of this repository's own thirteen breaking tasks are
/// cross-cutting, so the shape is ordinary rather than contrived.
#[test]
fn a_release_node_accounts_for_the_work_no_cut_claims() {
    let dir = tmp();
    fixture(dir.path(), "1.0.0");
    common::git(dir.path(), &["tag", "v1.0.0"]);
    // A release over a cut, and a break that belongs to neither.
    std::fs::write(
        dir.path().join("aplan/tasks/M-c.md"),
        "---\nid: M-c\ntitle: the cut\nphase: 1\nstatus: todo\n\
         tags:\n  - milestone\nblocked_by:\n  - W-1\n---\n\nbody\n",
    )
    .expect("cut");
    std::fs::write(
        dir.path().join("aplan/tasks/M-r.md"),
        "---\nid: M-r\ntitle: the release\nphase: 1\nstatus: todo\n\
         tags:\n  - milestone\n  - release\nblocked_by:\n  - M-c\n---\n\nbody\n",
    )
    .expect("release");
    std::fs::write(
        dir.path().join("aplan/tasks/W-2.md"),
        "---\nid: W-2\ntitle: a cross-cutting break\nphase: 1\nstatus: done\n\
         effort_days: 1\nversion_impact: breaking\ntags:\n  - cross-cutting\n\
         evidence:\n  - kind: note\n    ref: shipped\n    captured: 2026-09-24\n---\n\nbody\n",
    )
    .expect("break");
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-q", "-m", "the break"]);

    let out = common::run(dir.path(), &["version", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    assert_eq!(v["window"]["kind"], "release", "{}", out.text());
    assert_eq!(
        v["version"],
        "2.0.0",
        "a break on top of 1.0.0 is 2.0.0, whoever claims it: {}",
        out.text()
    );
    assert_eq!(v["owes"], "breaking");
    assert_eq!(v["raised_by"], "W-2");
    assert_eq!(v["window"]["unclaimed"], 1);

    // **The hermetic form still answers about the members**, which is what it is
    // for, and says what it cannot see rather than leaving it to be discovered.
    let node = common::run(dir.path(), &["version", "M-r"]);
    assert_eq!(
        node.stdout,
        "feature\n",
        "the cut floor, and not the break it cannot see: {}",
        node.text()
    );
    assert!(
        node.stderr.contains("claimed by no cut"),
        "the named form must say what it is not counting: {}",
        node.stderr
    );
    // And a *cut* says nothing: it genuinely does not contain the task, so its
    // own answer is right and there is nothing to warn about.
    let cut = common::run(dir.path(), &["version", "M-c"]);
    assert!(cut.stderr.is_empty(), "{}", cut.stderr);
}

/// **The number and the gate measure the same window** (`A1-348`, audit).
///
/// They did not. This command preferred the release node the plan models and
/// `APLAN048` always took the tag window, so a plan whose release holds an
/// unfinished `breaking` task had `aplan version` answer `2.0.0` while `verify`
/// -- the gate an adopter wires into CI -- reported `ok` on a tree declaring
/// `1.0.1`. `ci/scripts/release.sh` follows the command, so the release would
/// have been refused at tag time on a plan CI had been green about all along:
/// the discovered-late failure this whole feature exists to prevent, one layer
/// in.
#[test]
fn the_derived_number_and_the_rule_measure_the_same_window() {
    let dir = tmp();
    fixture(dir.path(), "1.0.0");
    // A break that is *planned* rather than closed: in the release's cut, so the
    // node sees it, and closed by nothing, so the tag window does not.
    std::fs::write(
        dir.path().join("aplan/tasks/W-1.md"),
        "---\nid: W-1\ntitle: a planned break\nphase: 1\nstatus: todo\neffort_days: 1\n\
         version_impact: breaking\n---\n\nbody\n",
    )
    .expect("task");
    std::fs::write(
        dir.path().join("aplan/tasks/M-c.md"),
        "---\nid: M-c\ntitle: the cut\nphase: 1\nstatus: todo\n\
         tags:\n  - milestone\nblocked_by:\n  - W-1\n---\n\nbody\n",
    )
    .expect("cut");
    std::fs::write(
        dir.path().join("aplan/tasks/M-r.md"),
        "---\nid: M-r\ntitle: the release\nphase: 1\nstatus: todo\n\
         tags:\n  - milestone\n  - release\nblocked_by:\n  - M-c\n---\n\nbody\n",
    )
    .expect("release");
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-q", "-m", "the plan"]);
    common::git(dir.path(), &["tag", "v1.0.0"]);
    std::fs::write(dir.path().join("version.sh"), "echo 1.0.1\n").expect("bump");
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-q", "-m", "bump to 1.0.1"]);

    let number = common::run(dir.path(), &["version"]);
    assert_eq!(number.stdout, "2.0.0\n", "{}", number.text());

    let gate = common::run(dir.path(), &["verify"]);
    let said = gate.text();
    assert!(
        said.contains("APLAN048") && said.contains("2.0.0"),
        "the gate must reach the number the command derives: {said}"
    );
    // **And it names the claim, not the level above it.** A release whose cut
    // owes `breaking` used to report "`M-c` is `breaking`", which reads as
    // though the cut had declared it and sends the reader one walk short.
    assert!(
        said.contains("`W-1` is `breaking`"),
        "the finding must point at the task that said so: {said}"
    );
}

/// Two open releases: the gate says so rather than measuring the wrong window.
#[test]
fn two_open_releases_are_a_finding_rather_than_a_silent_fallback() {
    let dir = tmp();
    fixture(dir.path(), "1.0.1");
    for id in ["M-r1", "M-r2"] {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: a release\nphase: 1\nstatus: todo\n\
                 tags:\n  - milestone\n  - release\nblocked_by:\n  - W-1\n---\n\nbody\n"
            ),
        )
        .expect("release");
    }
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-q", "-m", "two releases"]);
    let gate = common::run(dir.path(), &["verify"]);
    let said = gate.text();
    assert!(
        said.contains("APLAN048") && said.contains("M-r1") && said.contains("M-r2"),
        "the rule must say why it could not judge, not fall back to another window: {said}"
    );
}

/// A break that shipped in an earlier release does not raise this one forever.
///
/// The reason the fix is a union with the *window* rather than a wider walk:
/// sweeping every unclaimed task in the plan would make one break permanent.
#[test]
fn a_cross_cutting_break_that_already_shipped_is_not_swept_up_again() {
    let dir = tmp();
    fixture(dir.path(), "1.0.0");
    std::fs::write(
        dir.path().join("aplan/tasks/W-2.md"),
        "---\nid: W-2\ntitle: a break that already shipped\nphase: 1\nstatus: done\n\
         effort_days: 1\nversion_impact: breaking\ntags:\n  - cross-cutting\n\
         evidence:\n  - kind: note\n    ref: shipped\n    captured: 2026-09-24\n---\n\nbody\n",
    )
    .expect("break");
    std::fs::write(
        dir.path().join("aplan/tasks/M-r.md"),
        "---\nid: M-r\ntitle: the next release\nphase: 1\nstatus: todo\n\
         tags:\n  - milestone\n  - release\nblocked_by:\n  - W-1\n---\n\nbody\n",
    )
    .expect("release");
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-q", "-m", "the break"]);
    // Tagged *after* the break, so it is behind the window rather than in it.
    common::git(dir.path(), &["tag", "v1.0.0"]);
    close(dir.path(), "fix");

    let out = common::run(dir.path(), &["version", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    assert_eq!(v["window"]["unclaimed"], 0, "{}", out.text());
    assert_eq!(
        v["version"],
        "1.0.1",
        "the break is behind the tag, so it is somebody else's release: {}",
        out.text()
    );
}

/// **A cancelled task in a cut is not work the release fails to account for**
/// (`A1-348`, audit).
///
/// The walk skips dropped members, so one sitting inside a cut never enters the
/// reached set -- and the note counted it as unclaimed, which is the walk's own
/// exclusion read backwards.
#[test]
fn a_dropped_task_inside_a_cut_is_not_counted_as_unclaimed() {
    let dir = tmp();
    fixture(dir.path(), "1.0.0");
    std::fs::write(
        dir.path().join("aplan/tasks/W-9.md"),
        "---\nid: W-9\ntitle: cancelled, and in the cut\nphase: 1\nstatus: dropped\n\
         effort_days: 1\n---\n\nbody\n",
    )
    .expect("dropped");
    std::fs::write(
        dir.path().join("aplan/tasks/M-c.md"),
        "---\nid: M-c\ntitle: the cut\nphase: 1\nstatus: todo\n\
         tags:\n  - milestone\nblocked_by:\n  - W-1\n  - W-9\n---\n\nbody\n",
    )
    .expect("cut");
    std::fs::write(
        dir.path().join("aplan/tasks/M-r.md"),
        "---\nid: M-r\ntitle: the release\nphase: 1\nstatus: todo\n\
         tags:\n  - milestone\n  - release\nblocked_by:\n  - M-c\n---\n\nbody\n",
    )
    .expect("release");
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-q", "-m", "a cancelled member"]);

    let out = common::run(dir.path(), &["version", "M-r", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    assert!(
        v["unclaimed_by_any_cut"].is_null(),
        "every task this plan will ship is in a cut: {}",
        out.stdout
    );
}

/// Which release ships next is the plan's question, not this tool's, and a guess
/// here would be a number.
#[test]
fn two_open_releases_are_refused_by_name() {
    let dir = tmp();
    fixture(dir.path(), "0.3.0");
    for id in ["M-r1", "M-r2"] {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: a release\nphase: 1\nstatus: todo\n\
                 tags:\n  - milestone\n  - release\nblocked_by:\n  - W-1\n---\n\nbody\n"
            ),
        )
        .expect("release");
    }
    close(dir.path(), "fix");
    let out = common::run(dir.path(), &["version"]);
    assert_ne!(out.code, 0, "it answered anyway: {}", out.text());
    let said = out.text();
    assert!(
        said.contains("M-r1") && said.contains("M-r2"),
        "both are named: {said}"
    );
}

/// **No declared command is a refusal that names the key**, never a guess and
/// never a silent skip -- and the form that needs nothing outside the plan still
/// answers, which is what "the declaration rung only" means.
#[test]
fn without_a_declared_command_it_refuses_and_says_what_to_declare() {
    let dir = tmp();
    fixture(dir.path(), "0.3.0");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    close(dir.path(), "breaking");
    let out = common::run(dir.path(), &["version"]);
    assert_ne!(out.code, 0, "it answered anyway: {}", out.text());
    assert!(
        out.text().contains("version.current.command"),
        "the refusal names the key to add: {}",
        out.text()
    );
    // And the hermetic form is unaffected, because what a node owes needs
    // nothing outside the plan.
    let owed = common::run(dir.path(), &["version", "W-1"]);
    assert_eq!(owed.stdout, "breaking\n", "{}", owed.text());
}

/// A command that will not run is a fault of the configuration, reported as
/// itself. No number may be invented from it (`A1-066`, `A1-300`).
#[test]
fn a_command_that_cannot_run_is_a_refusal_naming_the_key() {
    let dir = tmp();
    fixture(dir.path(), "0.3.0");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\nversion:\n  current:\n    command: [no-such-program-anywhere-4f2a]\n",
    )
    .expect("config");
    close(dir.path(), "fix");
    let out = common::run(dir.path(), &["version"]);
    assert_ne!(out.code, 0, "it answered anyway: {}", out.text());
    assert!(
        out.text().contains("version.current"),
        "it blames the configuration: {}",
        out.text()
    );
}

/// A scheme this does not understand gets no answer rather than a guess, at the
/// command's own edge (ADR-0034).
#[test]
fn a_calver_version_is_refused_rather_than_mangled() {
    let dir = tmp();
    fixture(dir.path(), "2026.09.1");
    close(dir.path(), "fix");
    let out = common::run(dir.path(), &["version"]);
    assert_ne!(out.code, 0, "it answered anyway: {}", out.text());
    assert!(
        out.text().contains("2026.09.1"),
        "the refusal quotes the version it could not read: {}",
        out.text()
    );
}
