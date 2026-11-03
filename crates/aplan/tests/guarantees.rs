//! **A guarantee this tool makes in one command, it makes in all of them**
//! (`A1-356`).
//!
//! Five defects in one increment had one shape: a promise kept by one command
//! and not by the command beside it. Each was reachable by an ordinary
//! invocation, none was found by a rule, and in three of the five the correct
//! behaviour was written down in a comment next to the code that had it. The
//! knowledge was not missing. It was not carried.
//!
//! **Why no existing audit catches it.** Every census here is per *surface*:
//! `inert_flags.rs` proves each flag drives something, `behaviour_proof.rs`
//! proves each named behaviour has a test, `coverage.rs` proves each rule has a
//! fires-and-silent pair. All of them ask whether a thing that exists is
//! exercised. A guarantee absent from a command is not there to exercise, so it
//! is invisible to all three.
//!
//! **The shape of this file.** A guarantee names the commands it binds and a
//! probe for each. The probe is an invocation and what must come back, so a
//! command that stops keeping the promise fails here by name. Adding a command
//! to a guarantee is how the promise is carried; leaving it out is a decision
//! somebody has to write down in `mutation_policy.rs`, which refuses a command
//! that is classified nowhere.

use std::path::Path;

use crate::common;

/// What a probe must see. Deliberately coarse: this file asks whether the
/// promise is kept, not how the message is worded, which is
/// `consumer_facing.rs`' question.
enum Expect {
    /// Refused, with this in what it said.
    Refused(&'static str),
    /// Accepted, and the plan it left behind still checks.
    Accepted,
}

struct Probe {
    /// The command this binds, as `aplan <name>` spells it.
    command: &'static str,
    args: &'static [&'static str],
    expect: Expect,
}

struct Guarantee {
    name: &'static str,
    /// Why it exists, in the words of whichever command had it first.
    why: &'static str,
    /// Built fresh for each probe, so one probe cannot shift the next.
    fixture: fn(&Path),
    probes: &'static [Probe],
}

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(path, body).expect("write");
}

fn task(dir: &Path, id: &str, extra: &str) {
    write(
        dir,
        &format!("aplan/tasks/{id}.md"),
        &format!(
            "---\nid: {id}\ntitle: task {id}\nphase: 1\nstatus: todo\neffort_days: 1\n\
             {extra}---\n\n## Why\n\nBecause.\n"
        ),
    );
}

fn closed(dir: &Path, id: &str, extra: &str) {
    write(
        dir,
        &format!("aplan/tasks/{id}.md"),
        &format!(
            "---\nid: {id}\ntitle: task {id}\nphase: 1\nstatus: done\neffort_days: 1\n\
             {extra}evidence:\n  - kind: note\n    ref: reviewed\n    captured: 2026-08-14\n\
             ---\n\n## Why\n\nBecause.\n"
        ),
    );
}

/// A cut holding two tasks.
fn a_cut_with_work(dir: &Path) {
    write(dir, ".aplan.yml", "root: aplan\n");
    write(
        dir,
        "aplan/tasks/M-a.md",
        "---\nid: M-a\ntitle: a cut\nphase: 1\nstatus: todo\ntags:\n  - milestone\n\
         blocked_by:\n  - W-1\n  - W-2\n---\n\n## Why\n\nA cut.\n",
    );
    task(dir, "W-1", "");
    task(dir, "W-2", "");
}

/// A plan that requires a cut membership, with one closed task in the cut.
fn membership_required(dir: &Path) {
    write(
        dir,
        ".aplan.yml",
        "root: aplan\nrequire_cut_membership: true\n",
    );
    write(
        dir,
        "aplan/tasks/M-a.md",
        "---\nid: M-a\ntitle: a cut\nphase: 1\nstatus: todo\ntags:\n  - milestone\n\
         blocked_by:\n  - W-1\n---\n\n## Why\n\nA cut.\n",
    );
    closed(dir, "W-1", "");
}

/// One live task and one closed one, nothing required.
fn a_plain_plan(dir: &Path) {
    write(dir, ".aplan.yml", "root: aplan\n");
    write(dir, "NOTES.md", "# Notes\n\n- [ ] something to import\n");
    task(dir, "W-1", "tags:\n  - keep\n");
    closed(dir, "W-2", "tags:\n  - keep\n");
}

/// A settled cut holding finished work that cites a dated note: the two shapes
/// the "guard against adding" defects were found in.
fn a_settled_cut_holding_finished_work(dir: &Path) {
    write(dir, ".aplan.yml", "root: aplan\n");
    write(
        dir,
        "aplan/tasks/M-done.md",
        "---\nid: M-done\ntitle: a finished cut\nphase: 1\nstatus: done\ntags:\n\
         \x20 - milestone\nblocked_by:\n  - W-1\n  - W-9\nevidence:\n  - kind: file\n\
         \x20   ref: .aplan.yml\n---\n\n## Why\n\nDone.\n",
    );
    write(
        dir,
        "aplan/tasks/M-live.md",
        "---\nid: M-live\ntitle: a live cut\nphase: 1\nstatus: todo\ntags:\n\
         \x20 - milestone\nblocked_by:\n  - W-2\n---\n\n## Why\n\nLive.\n",
    );
    write(
        dir,
        "aplan/tasks/W-1.md",
        "---\nid: W-1\ntitle: finished work\nphase: 1\nstatus: done\neffort_days: 1\n\
         paths:\n- \"*.yml\"\nevidence:\n  - kind: note\n    ref: reviewed\n\
         \x20   captured: 2026-08-14\n  - kind: file\n    ref: .aplan.yml\n---\n\n\
         ## Why\n\nBecause.\n",
    );
    write(
        dir,
        "aplan/tasks/W-9.md",
        "---\nid: W-9\ntitle: other finished work\nphase: 1\nstatus: done\n\
         effort_days: 1\npaths:\n- \"*.yml\"\nevidence:\n  - kind: file\n\
         \x20   ref: .aplan.yml\n---\n\n## Why\n\nBecause.\n",
    );
    write(
        dir,
        "aplan/tasks/W-2.md",
        "---\nid: W-2\ntitle: live work\nphase: 1\nstatus: todo\neffort_days: 1\n\
         ---\n\n## Why\n\nBecause.\n",
    );
}

const GUARANTEES: &[Guarantee] = &[
    Guarantee {
        name: "work is not taken out of a cut without being put somewhere",
        why: "`dep rm` says it: a cut's `blocked_by` is which tasks are in it, and a \
              sequenced cut emptied of work keeps its predecessor, so `APLAN021` stays \
              silent. Nothing reports the loss, so the command has to refuse it",
        fixture: a_cut_with_work,
        probes: &[
            Probe {
                command: "dep",
                args: &["dep", "rm", "M-a", "--after", "W-1"],
                expect: Expect::Refused("is a cut"),
            },
            Probe {
                command: "tag",
                args: &["tag", "rm", "M-a", "milestone"],
                expect: Expect::Refused("is a cut holding"),
            },
        ],
    },
    Guarantee {
        name: "a placement `--cut` makes is part of the plan the gate judges",
        why: "`add` says it: the milestone edit `--cut` performs is part of the \
              post-state, so a placement that would break the plan is refused there \
              rather than reported on the next `check`. Applied afterwards instead, it \
              makes the gate refuse a placement the caller has just made",
        fixture: membership_required,
        probes: &[
            Probe {
                command: "add",
                args: &["add", "New work", "--cut", "M-a", "--effort", "1"],
                expect: Expect::Accepted,
            },
            Probe {
                command: "supersede",
                args: &[
                    "supersede",
                    "W-1",
                    "A successor",
                    "--cut",
                    "M-a",
                    "--effort",
                    "1",
                ],
                expect: Expect::Accepted,
            },
        ],
    },
    Guarantee {
        name: "a value the caller supplied is never reported as this tool's bug",
        why: "`close`'s round-trip guard exists to catch a defect in the writer. A \
              value a caller typed reaching it means the tool asked the reader to file \
              a bug about their own input (`A1-250`), so every writing path serialises \
              what it writes rather than pasting it",
        fixture: a_plain_plan,
        probes: &[
            Probe {
                command: "dep",
                args: &["dep", "add", "W-1", "--after", "  spaced"],
                expect: Expect::Refused("APLAN003"),
            },
            Probe {
                command: "tag",
                args: &["tag", "add", "W-1", "a, b"],
                expect: Expect::Accepted,
            },
            Probe {
                command: "add",
                args: &["add", "Tagged", "--tag", "*star", "--effort", "1"],
                expect: Expect::Accepted,
            },
            Probe {
                command: "supersede",
                args: &["supersede", "W-2", "A successor", "--tag", "a, b"],
                expect: Expect::Accepted,
            },
            Probe {
                command: "import",
                args: &[
                    "import",
                    "NOTES.md",
                    "--format",
                    "checklist",
                    "--tag",
                    "a, b",
                ],
                expect: Expect::Accepted,
            },
        ],
    },
    Guarantee {
        name: "a guard against adding does not refuse a removal",
        why: "a removal names something already in the file and records nothing, so a rule \
              about what may be *written* has nothing to say about it. Two commands shared \
              one check with their opposite and refused the removal in the adding rule's \
              words: `amend` (`A1-361`) and `mv` (`A1-362`)",
        fixture: a_settled_cut_holding_finished_work,
        probes: &[
            Probe {
                command: "mv",
                args: &["mv", "W-1", "--cut", "M-live"],
                expect: Expect::Accepted,
            },
            Probe {
                command: "amend",
                args: &["amend", "W-1", "--remove-evidence", "note:reviewed"],
                expect: Expect::Accepted,
            },
        ],
    },
    Guarantee {
        name: "a tag is never written twice",
        why: "`tag rename` says it: `[cli, cli]` is this tool writing a document its \
              own loader reads as one tag and a reviewer reads as a mistake",
        fixture: a_plain_plan,
        probes: &[
            Probe {
                command: "add",
                args: &[
                    "add", "Doubled", "--tag", "x", "--tag", "x", "--effort", "1",
                ],
                expect: Expect::Accepted,
            },
            Probe {
                command: "tag",
                args: &["tag", "add", "W-1", "keep"],
                expect: Expect::Accepted,
            },
            Probe {
                command: "supersede",
                args: &[
                    "supersede",
                    "W-2",
                    "A successor",
                    "--tag",
                    "y",
                    "--tag",
                    "y",
                ],
                expect: Expect::Accepted,
            },
        ],
    },
    Guarantee {
        name: "a tag is a word, so a blank one is refused",
        why: "`tag add` has said it since that command existed. Written instead, it \
              is `tags: ['']`: a tag no rule reports and nobody meant",
        fixture: a_plain_plan,
        probes: &[
            Probe {
                command: "add",
                args: &["add", "Blank", "--tag", "", "--effort", "1"],
                expect: Expect::Refused("a tag is a word"),
            },
            Probe {
                command: "tag",
                args: &["tag", "add", "W-1", ""],
                expect: Expect::Refused("a tag is a word"),
            },
            Probe {
                command: "supersede",
                args: &["supersede", "W-2", "A successor", "--tag", ""],
                expect: Expect::Refused("a tag is a word"),
            },
            Probe {
                command: "import",
                args: &["import", "NOTES.md", "--format", "checklist", "--tag", ""],
                expect: Expect::Refused("a tag is a word"),
            },
        ],
    },
];

fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("guarantees-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir")
}

/// **Every probe sees the promise kept.**
#[test]
fn every_command_bound_by_a_guarantee_keeps_it() {
    let mut broken: Vec<String> = Vec::new();
    for g in GUARANTEES {
        for probe in g.probes {
            let dir = scratch();
            (g.fixture)(dir.path());
            let out = common::run(dir.path(), probe.args);
            let text = out.text();
            match probe.expect {
                Expect::Refused(says) => {
                    if out.code == 0 {
                        broken.push(format!(
                            "`{}` does not keep \"{}\": `aplan {}` was accepted\n      {text}",
                            probe.command,
                            g.name,
                            probe.args.join(" ")
                        ));
                    } else if !text.contains(says) {
                        broken.push(format!(
                            "`{}` refused `aplan {}` for some other reason than \"{}\": \
                             expected `{says}`\n      {text}",
                            probe.command,
                            probe.args.join(" "),
                            g.name
                        ));
                    }
                }
                Expect::Accepted => {
                    if out.code != 0 {
                        broken.push(format!(
                            "`{}` does not keep \"{}\": `aplan {}` was refused\n      {text}",
                            probe.command,
                            g.name,
                            probe.args.join(" ")
                        ));
                        continue;
                    }
                    let after = common::run(dir.path(), &["check"]);
                    if after.code != 0 {
                        broken.push(format!(
                            "`{}` kept \"{}\" but left a plan that does not check: \
                             `aplan {}`\n      {}",
                            probe.command,
                            g.name,
                            probe.args.join(" "),
                            after.text()
                        ));
                    }
                }
            }
        }
    }
    assert!(
        broken.is_empty(),
        "{} guarantee(s) not carried to a command that makes the same promise \
         (`A1-356`):\n  - {}",
        broken.len(),
        broken.join("\n  - ")
    );
}

/// **Nothing this tool writes is reported as its own bug** (`A1-250`).
///
/// Exit `3` means the run proved nothing and asks the reader to file a report.
/// A caller's own argument reaching it is the defect, whatever the argument was,
/// so this asks the question of every probe above rather than of one command.
#[test]
fn no_probe_is_answered_with_this_tools_own_bug() {
    let mut blamed = Vec::new();
    for g in GUARANTEES {
        for probe in g.probes {
            let dir = scratch();
            (g.fixture)(dir.path());
            let out = common::run(dir.path(), probe.args);
            if out.code == 3 || out.text().contains("bug in aplan") {
                blamed.push(format!(
                    "aplan {}\n      {}",
                    probe.args.join(" "),
                    out.text()
                ));
            }
        }
    }
    assert!(
        blamed.is_empty(),
        "{} invocation(s) answered with `aplan`'s own defect:\n  - {}",
        blamed.len(),
        blamed.join("\n  - ")
    );
}

/// **A guarantee names more than one command, or it is not one.**
///
/// A promise kept by a single command is that command's behaviour and belongs
/// in its own test. This file is for the ones that bind several, because that is
/// where the carrying fails.
#[test]
fn every_guarantee_binds_at_least_two_commands_and_says_why() {
    for g in GUARANTEES {
        let commands: std::collections::BTreeSet<&str> =
            g.probes.iter().map(|p| p.command).collect();
        assert!(
            commands.len() >= 2,
            "\"{}\" binds only {:?}, so it is one command's behaviour rather than a \
             guarantee this file can carry",
            g.name,
            commands
        );
        assert!(
            g.why.len() > 60,
            "\"{}\" is asserted with no reason worth reading: {:?}",
            g.name,
            g.why
        );
    }
}
