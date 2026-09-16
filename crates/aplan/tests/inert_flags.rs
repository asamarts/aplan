//! A flag that changes nothing passes every test that names it (`A1-209`).
//!
//! `A1-208` proves a flag was *passed*. That is the weaker half of what
//! `AGENTS.md` asks of an audit, and it names the failure directly: the clause
//! that is easy to skip is giving the entry something to *drive*. A flag parsed
//! and then ignored satisfies an invocation audit perfectly, and so does one
//! whose handling was deleted in a refactor.
//!
//! **The criterion, checkable without knowing what a flag means**: two runs
//! differing only in that flag must differ in output or exit code. It is the
//! same fires-and-silent shape the rule corpus already uses, and it is what
//! `aplan vacuity` asks of a rule -- a rule that ran on nothing is not a passing
//! rule.
//!
//! **The enumeration is clap's and the values are not.** A flag's meaning is not
//! derivable, so each one needs a value chosen by somebody; what must not be
//! hand-kept is the *list*. Every per-command flag in the tree is either probed
//! below or held with a reason, and a flag in neither fails this file -- which
//! is the exhaustive-audit shape `AGENTS.md` names, applied to the CLI.

use std::path::Path;

mod common;

/// What to pass to exercise one flag, beside the arguments its command needs
/// anyway.
struct Probe {
    command: &'static str,
    flag: &'static str,
    /// `None` for a flag that takes no value.
    value: Option<&'static str>,
    /// Arguments the command needs before the flag is reached at all. A command
    /// refused for a missing argument never reaches its flags, and would pass
    /// this audit twice over.
    extra: &'static [&'static str],
    /// Which plan to run against. Most flags want the one that exercises the
    /// most rules; a few want one where nothing is wrong.
    fixture: Plan,
}

/// The two plans a probe can ask for.
#[derive(Clone, Copy, PartialEq)]
enum Plan {
    /// Two of everything, and two deliberate findings: a task with no estimate
    /// and a trace that only resolves once the root moves. Most flags need
    /// something to be wrong before they can change it.
    Rich,
    /// The same plan with those two removed, for the commands that refuse to
    /// run at all on a plan that does not verify. `attest` is the whole reason
    /// this exists: "an attestation of an unproven plan is worse than none".
    Verifying,
    /// No error, but a warning: the only state where `--deny-warnings` has
    /// something to promote. With an error present the summary already says
    /// `failed` and the `denied:` line is suppressed, so the flag reads inert
    /// against the rich plan and is not.
    Warned,
    /// [`Plan::Warned`] with `deny_warnings: true`, which is the only state
    /// where `--no-deny-warnings` has anything to undo. A flag that only
    /// matters against a config needs that config, or the audit calls it inert
    /// and is wrong.
    Strict,
}

impl Plan {
    /// The tasks a variant leaves out, and why each is left out.
    ///
    /// `D-6` traces a file that only resolves once the root moves, which is an
    /// error; `D-7` cites a commit that does not exist, which is another; `D-9`
    /// is live without an estimate, which is a warning. A variant is these three
    /// dials.
    fn without(self) -> &'static [&'static str] {
        match self {
            Plan::Rich => &[],
            Plan::Verifying => &["D-5", "D-6", "D-7", "D-9"],
            Plan::Warned | Plan::Strict => &["D-6", "D-7"],
        }
    }
}

/// A flag no pair could be built for, and why.
///
/// A held entry states who owns the gap. An audit that accepts "this one is
/// fine" for a growing list stops being an audit, so the reason is checked for
/// being a reason and the count is what a reader watches.
struct Held {
    command: &'static str,
    flag: &'static str,
    why: &'static str,
}

/// Every per-command long flag clap declares, as `(command, flag)`.
///
/// Globals are not here: `--root`, `--config` and `--no-config` are one
/// behaviour on thirty-two commands, and requiring a pair per command would be
/// thirty-two tests of one code path (`A1-208`).
fn declared() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut root = aplan::cli_command();
    root.build();
    let mut stack: Vec<(String, clap::Command)> = root
        .get_subcommands()
        .map(|c| (c.get_name().to_string(), c.clone()))
        .collect();
    while let Some((path, cmd)) = stack.pop() {
        for arg in cmd.get_arguments() {
            let Some(long) = arg.get_long() else { continue };
            if arg.is_global_set() || matches!(long, "help" | "version") {
                continue;
            }
            // The verdict pair is one behaviour on every command that writes,
            // for the same reason the globals above are excluded: it reaches
            // `refuse_bad_mutation_unless` and nothing else, so a probe per
            // command would be eight tests of one code path (`A1-208`'s
            // argument, `A1-266`'s flags). It is probed once, on `close`, and
            // `every_mutating_command_can_be_told_the_policy` in
            // `mutation_policy.rs` is what fails if a command stops declaring
            // it -- including a rename that still compiles.
            if matches!(long, "deny-warnings" | "no-deny-warnings")
                && !matches!(path.as_str(), "check" | "verify" | "close")
            {
                continue;
            }
            out.push((path.clone(), format!("--{long}")));
        }
        for sub in cmd.get_subcommands() {
            stack.push((format!("{path} {}", sub.get_name()), sub.clone()));
        }
    }
    out.sort();
    out
}

/// Where this file's fixtures live.
///
/// **Not the system temporary directory.** Every probe copies a plan and a git
/// repository, and on a machine where `/tmp` is a RAM-backed tmpfs shared by
/// everything else, a few hundred of those is inode pressure somebody else pays
/// for -- which is how this test first announced itself. `CARGO_TARGET_TMPDIR`
/// is the directory cargo provides for integration tests to write in, under
/// `target/`, and it is on disk (`A1-209`).
fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("inert-flags-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir")
}

/// A plan with two of everything, in a repository, so most commands have
/// something to say and most flags something to narrow.
fn fixture(dir: &Path, plan: Plan) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("src/one")).expect("mkdir");
    let config = if plan == Plan::Strict {
        "root: aplan\ndeny_warnings: true\n"
    } else {
        "root: aplan\n"
    };
    std::fs::write(dir.join(".aplan.yml"), config).expect("config");
    std::fs::write(dir.join("src/one/a.rs"), "fn main() {}\n").expect("src");
    // What `add --from` reads. An explicit id, so the probe does not depend on
    // whatever the minting convention would produce here (`A1-241`).
    std::fs::write(
        dir.join("bulk.yml"),
        "- title: From a manifest\n  id: D-90\n  effort: 1\n",
    )
    .expect("manifest");
    std::fs::write(
        dir.join("DOC.md"),
        "# Doc\n\n## The rail\n\nThe rail is fixed at three points.\n",
    )
    .expect("doc");
    // A traced section reachable only from `docs/`, so `--traces-root` has
    // somewhere to move to and `--no-traces` something to silence.
    std::fs::create_dir_all(dir.join("docs")).expect("mkdir");
    std::fs::write(
        dir.join("docs/design.md"),
        "# Design\n\n## The spar\n\nFixed at five points.\n",
    )
    .expect("design");
    std::fs::write(
        dir.join("PLAN.md"),
        "# Plan\n\n- [ ] Wire it\n- [x] Land it\n",
    )
    .expect("import source");
    let files = [
        (
            "D-1",
            "---\nid: D-1\ntitle: in cut a, kim, cli, todo\nphase: 0\nstatus: todo\nowner: kim\n\
             effort_days: 1\nblocked_by: [D-5]\npaths:\n- src/one/**\ntags: [cli]\n---\n\nbody\n",
        ),
        (
            "D-2",
            "---\nid: D-2\ntitle: in cut a, ada, docs, in progress\nphase: 0\n\
             status: in_progress\nowner: ada\neffort_days: 2\nblocked_by: [D-1, D-4]\n\
             paths:\n- src/two/**\ntags: [docs]\n---\n\nbody\n",
        ),
        (
            "D-3",
            "---\nid: D-3\ntitle: in cut b, ada, cli, todo\nphase: 0\nstatus: todo\nowner: ada\n\
             effort_days: 3\npaths:\n- src/one/**\ntags: [cli]\n---\n\nbody\n",
        ),
        (
            "D-4",
            "---\nid: D-4\ntitle: closed, traced, kim\nphase: 0\nstatus: done\nowner: kim\n\
             effort_days: 1\npaths:\n- src/one/**\ntags: [docs, deprecation]\n\
             traces:\n- DOC.md#the-rail\nevidence:\n  - kind: file\n    ref: src/one/a.rs\n\
             ---\n\nbody\n",
        ),
        (
            // No estimate, so `check` warns: `--no-require-effort` has something
            // to silence and `--deny-warnings` something to promote.
            "D-5",
            "---\nid: D-5\ntitle: no estimate, docs, closed on a commit\nphase: 0\n\
             status: done\nowner: ada\ntags: [docs]\n\
             evidence:\n  - kind: note\n    ref: reviewed\n    captured: 2026-08-14\n\
             ---\n\nbody\n",
        ),
        (
            // Traces a section by a path that only resolves once the root moves.
            "D-6",
            "---\nid: D-6\ntitle: traces from a moved root\nphase: 0\nstatus: todo\n\
             owner: kim\neffort_days: 1\ntags: [cli]\ntraces:\n- design.md#the-spar\n\
             ---\n\nbody\n",
        ),
        (
            // Live and unestimated, so `APLAN009` warns: `--no-require-effort`
            // has something to silence and `--deny-warnings` something to
            // promote. A *closed* task without an estimate does not warn, which
            // is why the one below could not serve.
            "D-9",
            "---\nid: D-9\ntitle: live, no estimate\nphase: 0\nstatus: todo\nowner: kim\n\
             tags: [cli]\n---\n\nbody\n",
        ),
        (
            // Parked with a revisit already in the past, which is the one shape
            // `due` reports at all: without it every flag on that command reads
            // as inert, because two empty answers are equal (`A1-264`).
            "D-11",
            "---\nid: D-11\ntitle: parked, revisit long past\nphase: 0\nstatus: blocked\n\
             owner: kim\neffort_days: 1\nrevisit_at: 2020-01-01T00:00:00Z\ntags: [cli]\n\
             ---\n\nbody\n",
        ),
        (
            // Waits on a *cut*, which is the one shape `why --all` reads
            // differently from the default: with no edge into a milestone the
            // bounded walk and the complete one give the same answer and the
            // flag looks inert. Its own task rather than an edge added to an
            // existing one, because making `D-9` wait on `M-b` changed what
            // `close D-9` answers and took two unrelated probes with it -- which
            // this audit caught, and is the reason it exists (`A1-263`).
            "D-10",
            "---\nid: D-10\ntitle: waits on a whole cut\nphase: 0\nstatus: todo\n\
             owner: kim\neffort_days: 1\nblocked_by: [M-b]\ntags: [cli]\n---\n\nbody\n",
        ),
        (
            // Closed on a commit with no `paths`, so `APLAN012` has a subject it
            // is blind about -- and one carrying a tag, so a `--tag` selection
            // changes what `vacuity` counts.
            "D-7",
            "---\nid: D-7\ntitle: blind subject, cli\nphase: 0\nstatus: done\nowner: kim\n\
             effort_days: 1\ntags: [cli]\n\
             evidence:\n  - kind: commit\n    ref: \"1111111\"\n---\n\nbody\n",
        ),
        (
            "M-a",
            "---\nid: M-a\ntitle: cut a\nphase: 0\nstatus: todo\ntags: [milestone, a]\n\
             blocked_by: [{members}]\n---\n\nbody\n",
        ),
        (
            "M-b",
            "---\nid: M-b\ntitle: cut b\nphase: 0\nstatus: todo\ntags: [milestone, b]\n\
             blocked_by: [D-3]\n---\n\nbody\n",
        ),
    ];
    let out = plan.without();
    let members: Vec<&str> = ["D-1", "D-2", "D-4", "D-5", "D-6", "D-7", "D-9"]
        .into_iter()
        .filter(|id| !out.contains(id))
        .collect();
    for (id, body) in files {
        if out.contains(&id) {
            continue;
        }
        // Nothing may still name a task the variant left out: a `blocked_by`
        // pointing at one is `APLAN003`, an error, which is the one thing these
        // variants exist to control.
        let mut body = body.replace("{members}", &members.join(", "));
        for gone in out {
            body = body.replace(&format!("blocked_by: [{gone}]\n"), "");
        }
        std::fs::write(dir.join(format!("aplan/tasks/{id}.md")), body).expect("task");
    }

    commit_all(dir);
}

fn commit_all(dir: &Path) {
    let repo = git2::Repository::init(dir).expect("git init");
    let mut index = repo.index().expect("index");
    index
        .add_all(["."].iter(), git2::IndexAddOption::DEFAULT, None)
        .expect("add");
    index.write().expect("write index");
    let tree = repo
        .find_tree(index.write_tree().expect("tree"))
        .expect("tree");
    let sig = git2::Signature::now("t", "t@e.com").expect("sig");
    repo.commit(Some("HEAD"), &sig, &sig, "in", &tree, &[])
        .expect("commit");
}

/// Copy the fixture, so a writing command cannot make the next run's answer
/// depend on this one's.
fn fresh(under: &Path, name: &str, plan: Plan) -> std::path::PathBuf {
    let dir = under.join(name);
    std::fs::create_dir_all(&dir).expect("mkdir");
    copy_tree(&fixture_root(plan), &dir);
    dir
}

fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).expect("read").flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().expect("kind").is_dir() {
            std::fs::create_dir_all(&target).expect("mkdir");
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// A repository that is not the plan's own, so `--repo` has somewhere else to
/// look: `.` is the default, and a probe passing the default proves nothing.
///
/// Beside the fixture rather than inside it, because a repository nested in
/// another is not a path the outer one can add.
fn elsewhere() -> std::path::PathBuf {
    use std::sync::OnceLock;
    static REPO: OnceLock<tempfile::TempDir> = OnceLock::new();
    let dir = REPO.get_or_init(|| {
        let dir = scratch();
        std::fs::write(dir.path().join("other.txt"), "x\n").expect("file");
        commit_all(dir.path());
        dir
    });
    dir.path().to_path_buf()
}

/// One prepared fixture, built once and copied per run.
fn fixture_root(plan: Plan) -> std::path::PathBuf {
    use std::sync::OnceLock;
    static RICH: OnceLock<tempfile::TempDir> = OnceLock::new();
    static CLEAN: OnceLock<tempfile::TempDir> = OnceLock::new();
    static WARNED: OnceLock<tempfile::TempDir> = OnceLock::new();
    static STRICT: OnceLock<tempfile::TempDir> = OnceLock::new();
    // One cell per variant, and matched exhaustively: an `else` here sent two
    // variants to the plan they were defined not to be, and the probes that
    // needed them reported four flags inert (`A1-209`).
    let cell = match plan {
        Plan::Rich => &RICH,
        Plan::Verifying => &CLEAN,
        Plan::Warned => &WARNED,
        Plan::Strict => &STRICT,
    };
    let dir = cell.get_or_init(|| {
        let dir = scratch();
        fixture(dir.path(), plan);
        dir
    });
    dir.path().to_path_buf()
}

/// Whether a command can change the plan.
///
/// Named rather than inferred, and erring towards copying: a command wrongly
/// called read-only would edit the shared fixture and every probe after it
/// would be answering about a different plan. The list is short and clap's tree
/// does not say which of its commands write.
fn writes(command: &str) -> bool {
    matches!(
        command.split(' ').next().unwrap_or(command),
        "add"
            | "amend"
            | "attest"
            | "close"
            | "dep"
            | "drop"
            | "export"
            | "export-agents-md"
            | "export-roadmap-json"
            | "import"
            | "init"
            | "mv"
            | "supersede"
    )
}

/// The answer a run gives: everything a reader would notice.
fn answer(dir: &Path, args: &[&str]) -> String {
    let out = common::run(dir, args);
    let raw = format!("{}|{}|{}", out.code, out.stdout, out.stderr);
    // **The directory is not part of the answer.** A pair run in two copies of
    // the fixture prints two different absolute paths, so any command that
    // names a file "differed" whatever the flag did -- and this audit passed on
    // a hundred and twenty flags it had proved nothing about. Found by making
    // the read-only pairs share one directory, which took the accident away
    // (`A1-209`).
    let mut normalised = raw.replace(&dir.display().to_string(), "<dir>");
    if let Ok(real) = std::fs::canonicalize(dir) {
        normalised = normalised.replace(&real.display().to_string(), "<dir>");
    }
    // **And what the run left behind.** `add --owner ada` prints the same note
    // either way; what it changes is the file. A criterion reading only stdout
    // calls that flag inert, which would have been this audit reporting a defect
    // in five flags that work (`A1-209`).
    format!("{normalised}|{}", plan_state(dir))
}

/// Every file the run could have touched, and its contents.
///
/// The whole working tree, not just `aplan/tasks`: `add --out` puts a task
/// somewhere else by definition, and `export-agents-md --write` edits a file
/// that is not a task at all. A digest of the plan directory alone would call
/// both of those inert (`A1-209`, audit).
///
/// `.git` is skipped: it changes whenever anything commits, which would make
/// every pair differ for a reason that is not the flag.
fn plan_state(dir: &Path) -> String {
    let mut out = Vec::new();
    collect_state(dir, dir, &mut out);
    out.sort();
    out.join("\n--\n")
}

fn collect_state(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_name() == ".git" {
            continue;
        }
        if entry.file_type().is_ok_and(|k| k.is_dir()) {
            collect_state(root, &path, out);
        } else {
            let name = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            let body = std::fs::read_to_string(&path).unwrap_or_default();
            out.push(format!("{name}\n{body}"));
        }
    }
}

const fn p(
    command: &'static str,
    flag: &'static str,
    value: Option<&'static str>,
    extra: &'static [&'static str],
) -> Probe {
    Probe {
        command,
        flag,
        value,
        extra,
        fixture: Plan::Rich,
    }
}

/// A probe against the plan that warns and does not err.
const fn warned(
    command: &'static str,
    flag: &'static str,
    value: Option<&'static str>,
    extra: &'static [&'static str],
) -> Probe {
    Probe {
        fixture: Plan::Warned,
        ..p(command, flag, value, extra)
    }
}

/// A probe against the plan whose config denies warnings.
const fn strict(
    command: &'static str,
    flag: &'static str,
    value: Option<&'static str>,
    extra: &'static [&'static str],
) -> Probe {
    Probe {
        fixture: Plan::Strict,
        ..p(command, flag, value, extra)
    }
}

/// A probe against the plan that verifies.
const fn v(
    command: &'static str,
    flag: &'static str,
    value: Option<&'static str>,
    extra: &'static [&'static str],
) -> Probe {
    Probe {
        fixture: Plan::Verifying,
        ..p(command, flag, value, extra)
    }
}

/// One probe per per-command flag, and the enumeration below refuses to be
/// missing any.
///
/// The values are chosen, because a flag's meaning is not derivable. What is not
/// chosen is the *list*: `declared()` walks clap, and a flag absent from both
/// this table and [`HELD`] fails.
const PROBES: &[Probe] = &[
    p("dep add", "--after", Some("D-1"), &["D-6"]),
    p("dep add", "--prefer", None, &["D-6", "--after", "D-1"]),
    p("dep rm", "--after", Some("D-1"), &["D-2"]),
    p("dep rm", "--prefer", None, &["D-2", "--after", "D-1"]),
    // `--from` is the one `add` flag with no pair that differs only by the
    // flag: it *replaces* the title, which clap refuses to accept beside it. So
    // the base run is `add` with nothing, and the contrast is that the flag is
    // what makes the command an invocation at all -- the runs differ because of
    // it, which is what this audit asks (`A1-241`).
    p("add", "--from", Some("bulk.yml"), &[]),
    p("add", "--id", Some("X-9"), &["Newtask"]),
    p("add", "--phase", Some("3"), &["Newtask"]),
    p("add", "--blocked-by", Some("D-1"), &["Newtask"]),
    p("add", "--tag", Some("docs"), &["Newtask"]),
    p("add", "--effort", Some("4"), &["Newtask"]),
    p("add", "--owner", Some("ada"), &["Newtask"]),
    p("add", "--paths", Some("src/**"), &["Newtask"]),
    p("add", "--traces", Some("DOC.md#the-rail"), &["Newtask"]),
    p("add", "--cut", Some("M-b"), &["Newtask"]),
    p("add", "--out", Some("aplan"), &["Newtask"]),
    p("add", "--dry-run", None, &["Newtask"]),
    p(
        "amend",
        "--replace-evidence",
        Some("file:src/one/a.rs file:PLAN.md"),
        &["D-4"],
    ),
    p("amend", "--add-evidence", Some("file:PLAN.md"), &["D-4"]),
    p(
        "amend",
        "--remove-evidence",
        Some("file:src/one/a.rs"),
        &["D-4"],
    ),
    p(
        "amend",
        "--repo",
        Some("{elsewhere}"),
        &["D-4", "--add-evidence", "file:PLAN.md"],
    ),
    p(
        "amend",
        "--no-resolve",
        None,
        &["D-4", "--add-evidence", "commit:1234567"],
    ),
    v("attest", "--repo", Some("{elsewhere}"), &[]),
    v("attest", "--attach-note", Some("reviewed"), &[]),
    v("attest", "--execute", None, &[]),
    p("check", "--cut", Some("M-b"), &[]),
    p("check", "--tag", Some("docs"), &[]),
    p("check", "--owner", Some("ada"), &[]),
    p("check", "--status", Some("in_progress"), &[]),
    p("check", "--format", Some("json"), &[]),
    p("check", "--no-require-effort", None, &[]),
    p("check", "--traces-root", Some("docs"), &[]),
    p("check", "--no-traces", None, &[]),
    warned("check", "--deny-warnings", None, &[]),
    strict("check", "--no-deny-warnings", None, &[]),
    p("check-docs", "--format", Some("json"), &["DOC.md"]),
    p("close", "--evidence", Some("file:PLAN.md"), &["D-1"]),
    p("close", "--repo", Some("{elsewhere}"), &["D-1"]),
    p("close", "--restamp", None, &["D-1"]),
    p("close", "--no-resolve", None, &["D-1"]),
    // `A1-266`: probed here and excluded elsewhere, because the pair reaches
    // one function. `D-9` is live with no estimate, so closing it is a mutation
    // whose delta carries a warning -- the only state either flag can act on,
    // since `refuse_bad_mutation` judges what a write *adds* and a finding the
    // plan already had is not one.
    warned("close", "--deny-warnings", None, &["D-9"]),
    strict("close", "--no-deny-warnings", None, &["D-9"]),
    p("critical-path", "--format", Some("json"), &[]),
    p("deprecations", "--format", Some("json"), &[]),
    p("drop", "--cut", None, &["D-1"]),
    // `--format checklist` is the default, so probing it would compare a run
    // against itself. A shape this build declines to write is what makes the
    // flag observable.
    p("export", "--format", Some("kep"), &[]),
    p("export", "--out", Some("OUT.md"), &[]),
    p("export", "--check", Some("DOC.md"), &[]),
    p("export-agents-md", "--write", Some("AGENTS.md"), &[]),
    p("export-agents-md", "--check", Some("AGENTS.md"), &[]),
    p("export-roadmap-json", "--check", None, &[]),
    p("facts", "--require", Some("traces.table_rows"), &[]),
    // No value: it replaces the capability document with the build's identity,
    // which is a different answer to a different question (`A1-256`).
    p("facts", "--identity", None, &[]),
    p("graph", "--cut", Some("M-b"), &[]),
    p("graph", "--tag", Some("docs"), &[]),
    p("graph", "--owner", Some("ada"), &[]),
    p("graph", "--status", Some("in_progress"), &[]),
    p("graph", "--reach", Some("downstream:D-1"), &[]),
    p("graph", "--membership", None, &[]),
    p("graph", "--format", Some("json"), &[]),
    p(
        "import",
        "--format",
        Some("json"),
        &["PLAN.md", "--format", "checklist"],
    ),
    p(
        "import",
        "--out",
        Some("aplan"),
        &["PLAN.md", "--format", "checklist"],
    ),
    p(
        "import",
        "--dry-run",
        None,
        &["PLAN.md", "--format", "checklist"],
    ),
    p(
        "import",
        "--phase",
        Some("3"),
        &["PLAN.md", "--format", "checklist"],
    ),
    p(
        "import",
        "--implemented-as",
        Some("done"),
        &["PLAN.md", "--format", "checklist"],
    ),
    p(
        "import",
        "--as-of",
        Some("2026-01-01"),
        &["PLAN.md", "--format", "checklist"],
    ),
    p(
        "import",
        "--cut",
        Some("M-b"),
        &["PLAN.md", "--format", "checklist"],
    ),
    p(
        "import",
        "--id-prefix",
        Some("IM"),
        &["PLAN.md", "--format", "checklist"],
    ),
    p("mv", "--cut", Some("M-b"), &["D-1"]),
    p("mv", "--phase", Some("3"), &["D-1"]),
    p("paths", "--sample", Some("2"), &[]),
    p("paths", "--repo", Some("{elsewhere}"), &[]),
    p("paths", "--format", Some("json"), &[]),
    p("paths", "--cut", Some("M-b"), &[]),
    p("paths", "--tag", Some("docs"), &[]),
    p("paths", "--owner", Some("ada"), &[]),
    p("paths", "--status", Some("in_progress"), &[]),
    p("ready", "--explain", None, &[]),
    p("ready", "--format", Some("json"), &[]),
    p("ready", "--cut", Some("M-b"), &[]),
    p("ready", "--tag", Some("docs"), &[]),
    p("ready", "--owner", Some("ada"), &[]),
    p("ready", "--status", Some("in_progress"), &[]),
    p("roadmap", "--format", Some("json"), &[]),
    p("roadmap", "--regenerate-with", Some("x"), &[]),
    p("show", "--format", Some("json"), &["D-1"]),
    p("show", "--body", None, &["D-1"]),
    p("show", "--repo", Some("{elsewhere}"), &["D-4"]),
    p("show", "--no-resolve", None, &["D-1"]),
    // `due` reports `D-11`, which is parked with a revisit in the past, so each
    // of these changes the answer rather than comparing two empty ones.
    p("due", "--as-of", Some("2019-01-01T00:00:00Z"), &[]),
    p("due", "--format", Some("json"), &[]),
    p("due", "--cut", Some("M-b"), &[]),
    p("due", "--tag", Some("docs"), &[]),
    p("due", "--owner", Some("ada"), &[]),
    p("due", "--status", Some("in_progress"), &[]),
    p("stale", "--older-than", Some("0"), &[]),
    p("stale", "--as-of", Some("2026-01-01"), &[]),
    p("stale", "--repo", Some("{elsewhere}"), &[]),
    p("stale", "--format", Some("json"), &[]),
    p("stale", "--cut", Some("M-b"), &[]),
    p("stale", "--tag", Some("docs"), &[]),
    p("stale", "--owner", Some("ada"), &[]),
    p("stale", "--status", Some("in_progress"), &[]),
    p("summary", "--format", Some("json"), &[]),
    p("summary", "--cut", Some("M-b"), &[]),
    p("summary", "--tag", Some("docs"), &[]),
    p("summary", "--owner", Some("ada"), &[]),
    p("summary", "--status", Some("in_progress"), &[]),
    p("supersede", "--cut", Some("M-b"), &["D-4", "Successor"]),
    p("supersede", "--effort", Some("4"), &["D-4", "Successor"]),
    p("supersede", "--owner", Some("ada"), &["D-4", "Successor"]),
    p("tasks", "--format", Some("json"), &[]),
    p("tasks", "--cut", Some("M-b"), &[]),
    p("tasks", "--tag", Some("docs"), &[]),
    p("tasks", "--owner", Some("ada"), &[]),
    p("tasks", "--status", Some("in_progress"), &[]),
    p("tree", "--depth", Some("1"), &["D-2"]),
    p("tree", "--live", None, &["D-2"]),
    p("vacuity", "--format", Some("json"), &[]),
    p("vacuity", "--cut", Some("M-b"), &[]),
    p("vacuity", "--tag", Some("docs"), &[]),
    p("vacuity", "--owner", Some("ada"), &[]),
    p("vacuity", "--status", Some("in_progress"), &[]),
    p("verify", "--cut", Some("M-b"), &[]),
    p("verify", "--tag", Some("docs"), &[]),
    p("verify", "--owner", Some("ada"), &[]),
    p("verify", "--status", Some("in_progress"), &[]),
    p("verify", "--format", Some("json"), &[]),
    p("verify", "--repo", Some("{elsewhere}"), &[]),
    p("verify", "--no-require-effort", None, &[]),
    p("verify", "--traces-root", Some("docs"), &[]),
    p("verify", "--no-traces", None, &[]),
    p("verify", "--execute", None, &[]),
    warned("verify", "--deny-warnings", None, &[]),
    strict("verify", "--no-deny-warnings", None, &[]),
    p("why", "--format", Some("json"), &["D-1"]),
    // `D-9` waits on the cut `M-b`, so the bounded answer names the cut and the
    // complete one walks into its membership (`A1-263`).
    p("why", "--all", None, &["D-10"]),
];

/// Flags no pair could be built for, each with the reason.
const HELD: &[Held] = &[];

/// **Every per-command flag has a probe or a held reason.**
///
/// The half that makes the audit exhaustive: a flag added tomorrow is covered
/// without anyone remembering, because clap's tree is the enumeration and this
/// list is only the values.
#[test]
fn every_declared_flag_is_probed_or_held() {
    let probed: std::collections::BTreeSet<(String, String)> = PROBES
        .iter()
        .map(|p| (p.command.to_string(), p.flag.to_string()))
        .collect();
    let held: std::collections::BTreeSet<(String, String)> = HELD
        .iter()
        .map(|h| (h.command.to_string(), h.flag.to_string()))
        .collect();

    let declared = declared();
    assert!(
        declared.len() > 100,
        "the walk found {} flag(s), so it has stopped matching",
        declared.len()
    );
    let missing: Vec<String> = declared
        .iter()
        .filter(|f| !probed.contains(f) && !held.contains(f))
        .map(|(c, f)| format!("aplan {c} {f}"))
        .collect();
    assert!(
        missing.is_empty(),
        "{} flag(s) with neither a probe nor a reason:\n  {}",
        missing.len(),
        missing.join("\n  ")
    );

    let declared_set: std::collections::BTreeSet<(String, String)> = declared.into_iter().collect();
    let stale: Vec<String> = probed
        .union(&held)
        .filter(|f| !declared_set.contains(f))
        .map(|(c, f)| format!("aplan {c} {f}"))
        .collect();
    assert!(
        stale.is_empty(),
        "{} entry(s) for a flag the tool no longer has:\n  {}",
        stale.len(),
        stale.join("\n  ")
    );
    for h in HELD {
        assert!(
            h.why.len() > 30,
            "`aplan {} {}` is held with no reason worth reading: {:?}",
            h.command,
            h.flag,
            h.why
        );
    }
}

/// **Every probed flag changes the answer.**
///
/// Two runs differing only in the flag, each against its own copy of the
/// fixture so a writing command cannot make the next answer depend on this one.
#[test]
fn every_flag_changes_something() {
    let under = scratch();
    // What the shared plans hold before anything runs, so a command wrongly
    // called read-only cannot change them unnoticed.
    let untouched: Vec<(Plan, String)> = [Plan::Rich, Plan::Verifying, Plan::Warned, Plan::Strict]
        .into_iter()
        .map(|plan| (plan, plan_state(&fixture_root(plan))))
        .collect();
    // In parallel, because the cost is process spawns and fixture copies rather
    // than anything this test computes. Sequentially it added 39 seconds to a
    // 60-second suite, which is the difference the task names between a gate
    // people keep and one they mute (`A1-209`).
    let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let chunk = PROBES.len().div_ceil(threads);
    let inert: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = PROBES
            .chunks(chunk)
            .enumerate()
            .map(|(part, probes)| {
                let under = under.path();
                scope.spawn(move || inert_among(under, part, probes))
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("a probe thread"))
            .collect()
    });
    assert!(
        inert.is_empty(),
        "{} flag(s) that changed nothing, so nothing here would notice their handling \
         being deleted:\n  {}",
        inert.len(),
        inert.join("\n  ")
    );

    // **And the shared plans are still what they were.** A command wrongly
    // called read-only edits the fixture every later probe reads, so the pairs
    // after it compare against a plan nobody wrote -- silently, and differently
    // depending on which thread got there first. `writes()` is a hand-kept list
    // and this is what stops it being a hand-kept list nobody checks
    // (`A1-209`, audit).
    for (plan, before) in untouched {
        assert_eq!(
            plan_state(&fixture_root(plan)),
            before,
            "a command this file calls read-only edited the shared plan"
        );
    }
}

/// The inert flags among one slice of the probes.
fn inert_among(under: &Path, part: usize, probes: &[Probe]) -> Vec<String> {
    let mut inert = Vec::new();
    let mut remembered: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for (i, probe) in probes.iter().enumerate() {
        let n = format!("{part}-{i}");
        // Split, because a nested command is two words and one argv element
        // would be a subcommand nothing has.
        let mut base: Vec<&str> = probe.command.split(' ').collect();
        base.extend(probe.extra.iter().copied());

        // `{elsewhere}` stands for a repository that is not this plan's, whose
        // path is only known at run time.
        let substituted: Vec<String> = probe
            .value
            .into_iter()
            .flat_map(|v| v.split(' '))
            .map(|word| word.replace("{elsewhere}", &elsewhere().display().to_string()))
            .collect();
        let mut with = base.clone();
        with.push(probe.flag);
        with.extend(substituted.iter().map(String::as_str));

        // A command that cannot write runs in the prepared plan itself. Copying
        // for it costs a git repository per run and buys nothing: two hundred
        // and forty copies is most of this file's time, and the ones that
        // matter are the forty that mutate (`A1-209`).
        // The run *without* the flag is the same for every flag of a command,
        // and `check` alone has ten: computed once and remembered, which is
        // ninety fewer processes than asking the same question ten times
        // (`A1-209`).
        let key = format!("{:?}|{}", probe.fixture as u8, base.join(" "));
        let without = if let Some(seen) = remembered.get(&key) {
            seen.clone()
        } else {
            let fresh_answer = if writes(probe.command) {
                answer(&fresh(under, &format!("a{n}"), probe.fixture), &base)
            } else {
                answer(&fixture_root(probe.fixture), &base)
            };
            remembered.insert(key, fresh_answer.clone());
            fresh_answer
        };
        let flagged = if writes(probe.command) {
            answer(&fresh(under, &format!("b{n}"), probe.fixture), &with)
        } else {
            answer(&fixture_root(probe.fixture), &with)
        };
        if without == flagged {
            // The answer itself, because "these are the same" without saying
            // what they are is a finding a reader cannot act on.
            inert.push(format!(
                "`aplan {}` and `aplan {}` both answer: {}",
                base.join(" "),
                with.join(" "),
                flagged
                    .replace('\n', " / ")
                    .chars()
                    .take(160)
                    .collect::<String>()
            ));
        }
    }
    inert
}
