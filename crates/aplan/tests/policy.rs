//! This repository's own evidence policy, held to by a test (`A1-123`).
//!
//! `.aplan.yml` is the enforcement: `require_min_strength: [test]` refuses a
//! closure whose only proof is a commit, because `APLAN012` binds a cited commit
//! to a task by path overlap alone and that is the weakest of the three kinds.
//! Measured before it was turned on: 53 of 108 closed tasks cited no test, 24 of
//! them owning code, and the tests existed in every case -- the plan simply did
//! not point at them.
//!
//! Two things a config cannot say, which is why this file exists.
//!
//! The **exemption is a claim**, and this project does not take claims on trust:
//! `untestable` means "no test would fail if this task's work were reverted", and
//! a task carrying it must say which case it is, in its body, where a reader will
//! meet it. A tag alone is an assertion nobody can check.
//!
//! And a policy that can be relaxed silently is not a policy. Deleting the line
//! from `.aplan.yml` would make every commit-only closure legal again with
//! nothing failing, which is exactly the state this task ended.

use std::path::{Path, PathBuf};

use crate::common;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// The frontmatter and the body, split on the fence *lines*.
///
/// Not on the first `---` anywhere in the text: a `traces` anchor for a title
/// containing `--require` slugs to `so---require`, and splitting there cut a task
/// file in half and hid its evidence from this gate (`A1-123`).
fn split_task(text: &str) -> (String, String) {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (String::new(), text.to_string());
    }
    let mut fm = Vec::new();
    for line in lines.by_ref() {
        if line.trim() == "---" {
            break;
        }
        fm.push(line);
    }
    (fm.join("\n"), lines.collect::<Vec<_>>().join("\n"))
}

/// The `tags:` of a frontmatter, in either shape this plan uses.
///
/// Parsed rather than matched as a substring, which is the defect this file was
/// written to prevent and then had: `fm.contains("untestable")` was true of
/// `A1-123` because it *cites* `every_untestable_task_says_why_in_its_body`, so
/// the gate read an evidence citation as an exemption claim (`A1-123`).
fn tags_of(fm: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_block = false;
    for line in fm.lines() {
        if let Some(rest) = line.strip_prefix("tags:") {
            let rest = rest.trim();
            if let Some(inline) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                out.extend(inline.split(',').map(|t| t.trim().to_string()));
            } else if rest.is_empty() {
                in_block = true;
            }
            continue;
        }
        if in_block {
            match line
                .strip_prefix("- ")
                .or_else(|| line.strip_prefix("  - "))
            {
                Some(tag) => out.push(tag.trim().to_string()),
                None => in_block = false,
            }
        }
    }
    out
}

/// Every `done` task in this plan, as `(id, frontmatter, body)`.
fn closed_tasks() -> Vec<(String, String, String)> {
    let dir = repo_root().join("aplan/tasks");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("plan dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("task file");
        let (fm, body) = split_task(&text);
        if !fm.contains("status: done") {
            continue;
        }
        let id = path
            .file_stem()
            .expect("stem")
            .to_string_lossy()
            .into_owned();
        out.push((id, fm, body));
    }
    out.sort();
    out
}

/// The rule an author meets, stated where the enforcement lives.
#[test]
fn this_repositorys_policy_requires_a_test_and_says_so() {
    let config = std::fs::read_to_string(repo_root().join(".aplan.yml")).expect(".aplan.yml");
    assert!(
        config.contains("require_min_strength: [test]"),
        "a commit-only closure is legal again, and nothing else would have said so"
    );
    assert!(
        config.contains("exempt_tags: [untestable, milestone]"),
        "the exemption is part of the policy, not an escape hatch that drifts"
    );

    // `APLAN025` asks whether a cited test belongs to the code a task owns, and it
    // is silent for a runner that declares no `covers`. This repository shipped
    // that rule and left every runner silent, so a task owning `aplan-core` could
    // cite an `aplan-mcp` unit test and nothing objected (`A1-123`). The two
    // integration crates are silent on purpose -- their tests drive the binary and
    // assert facts about every crate -- and every library crate claims its own.
    for crate_name in [
        "aplan-core",
        "aplan-config",
        "aplan-evidence",
        "aplan-output",
        "aplan-interop",
        "aplan-mcp",
        "xtask",
    ] {
        let claim = format!("covers: [{crate_name}/**]");
        let claim_crate = format!("covers: [crates/{crate_name}/**]");
        assert!(
            config.contains(&claim) || config.contains(&claim_crate),
            "the `{crate_name}` runner claims no locality, so `APLAN025` is vacuous for it"
        );
    }

    // `AGENTS.md` is where an author reads the testing contract, so the rule has
    // to be there too: a rule enforced only by a refusal is one an author meets
    // by being refused (`A1-123`).
    let agents = std::fs::read_to_string(repo_root().join("AGENTS.md")).expect("AGENTS.md");
    assert!(
        agents.contains("untestable"),
        "the exemption is undocumented"
    );
    assert!(
        agents.contains("require_min_strength"),
        "the rule an author has to satisfy is not stated where they would read it"
    );
}

/// One `## ` section of a document, with its fenced blocks removed.
///
/// **The stripping is what makes the assertions below real.** `[commit, test,
/// file]` is in the configuration example as well as in the prose, so a check
/// over the raw file passed on the code block alone and would have gone on
/// passing with every sentence deleted (`A1-283`, audit).
fn section_prose(doc: &str, heading: &str) -> String {
    let start = doc
        .find(&format!("\n## {heading}\n"))
        .unwrap_or_else(|| panic!("no `## {heading}` section"));
    let rest = &doc[start + 1..];
    let end = rest[1..].find("\n## ").map_or(rest.len(), |i| i + 1);
    let mut out = String::new();
    let mut fenced = false;
    for line in rest[..end].lines() {
        if line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The setting a plan with nothing to cite yet starts at.
///
/// Typed here rather than derived, because it is not what this repository runs:
/// it is the wide end of the arc, and the adopter guide has to name both.
const WIDE: &str = "[commit, test, file]";

/// **The progression an adopter is told to follow names the end this repository
/// actually stands at, and names both ends in one breath** (`A1-283`).
///
/// The two ends of `require_min_strength` look like disagreement if nobody says
/// so: the configuration example shows the wide setting and this repository runs
/// `[test]`, and the pilot that noticed wrote the pairing down in its own log and
/// filed nothing. So the adopter guide carries the arc now.
///
/// Three things make this a check rather than a spell-check. The values are read
/// out of `.aplan.yml` rather than typed here, so widening the policy without
/// rewriting the paragraph fails. The fenced examples are stripped, so the wide
/// end has to be in a sentence. And both ends have to be in the *same paragraph*,
/// because two mentions in different places are the state this task was filed
/// about.
#[test]
fn the_adopter_guide_names_the_narrow_end_this_repository_stands_at() {
    let config = std::fs::read_to_string(repo_root().join(".aplan.yml")).expect(".aplan.yml");
    let declared = |key: &str| {
        config
            .lines()
            .find_map(|l| l.trim().strip_prefix(&format!("{key}: ")))
            .unwrap_or_else(|| panic!(".aplan.yml declares no `{key}`"))
            .trim()
            .to_string()
    };
    let narrow = declared("require_min_strength");
    let exempt = declared("exempt_tags");

    let upgrading =
        std::fs::read_to_string(repo_root().join("docs/upgrading.md")).expect("docs/upgrading.md");
    let prose = section_prose(
        &upgrading,
        "What is new since a build made before 2026-08-06",
    );
    assert!(
        !prose.contains("```") && prose.len() > 2_000,
        "the section walk is not reading the upgrade guidance: {} bytes",
        prose.len()
    );

    // The paragraph that names what this repository runs. Blank-line delimited,
    // which is what a paragraph is in this document.
    let arc = prose
        .split("\n\n")
        .find(|p| {
            p.contains("evidence.require_min_strength") && p.contains(&format!("is `{narrow}`"))
        })
        .unwrap_or_else(|| {
            panic!(
                "no paragraph in the upgrade guidance names `require_min_strength: {narrow}`, \
                 which is what this repository runs, so the progression it describes is about \
                 somebody else's policy"
            )
        });
    assert!(
        arc.contains(&format!("evidence.exempt_tags` is `{exempt}")),
        "the paragraph names the strength this repository runs and not the exemption beside \
         it, and the two are one decision:\n{arc}"
    );
    assert!(
        arc.contains(WIDE),
        "the wide end a greenfield plan starts at is not in the same paragraph as the narrow \
         end, so the two read as alternatives rather than as stages:\n{arc}"
    );
    assert!(
        arc.contains("`test:`"),
        "the paragraph says which value to start at and not what earns a narrower one: a \
         runner `aplan verify` can list is the thing that earns it:\n{arc}"
    );
}

/// The section walk reads one section's prose and stops at the next heading.
#[test]
fn the_section_walk_stops_at_the_next_heading_and_drops_the_fences() {
    let doc = "# Title\n\n## One\n\nprose one\n\n```yaml\nfenced: true\n```\n\nmore one\n\n\
               ## Two\n\nprose two\n";
    let one = section_prose(doc, "One");
    assert!(
        one.contains("prose one") && one.contains("more one"),
        "{one}"
    );
    assert!(!one.contains("fenced"), "the fence survived: {one}");
    assert!(
        !one.contains("prose two"),
        "the walk ran past its heading: {one}"
    );
}

/// This repository takes its own advice about the warning tier (`A1-276`).
///
/// `README.md` tells an adopter wiring the gate into CI that an automatic gate
/// with no `deny_warnings` has switched the warning tier off, and this repository
/// stood in that exact position while saying so: `.aplan.yml` switches
/// `detect_drift` on, `structural.sh` runs `aplan check` on every push, and a
/// drifted traced section printed `ok: 287 tasks, 1 warning(s)` and exited `0`.
///
/// Asserted here rather than left to the file, because the absence of a key is
/// invisible: deleting the line would restore the gap in silence, and the only
/// symptom would be a build that goes on passing.
#[test]
fn the_gate_this_repository_runs_on_itself_has_no_advisory_tier() {
    let config = std::fs::read_to_string(repo_root().join(".aplan.yml")).expect(".aplan.yml");
    assert!(
        config.contains("deny_warnings: true"),
        "seventeen of the forty rules warn, and this repository's gate is automatic"
    );

    // And it reached the contract, which is the half a config key can fail at:
    // an author reads `AGENTS.md`, not `.aplan.yml`.
    let agents = std::fs::read_to_string(repo_root().join("AGENTS.md")).expect("AGENTS.md");
    assert!(
        agents.contains("**Every warning blocks**"),
        "the policy is set and the generated contract still describes the old one"
    );
}

/// Whether a task claims the exemption without saying which case it is.
///
/// A predicate rather than an inline check, so it can be tested against a task
/// that gets it wrong. Over live files it is a guard that has never fired, and a
/// guard nothing exercises is one that can be deleted without a failure -- which
/// is how a gate stops being a gate (`A1-123`).
fn exemption_is_unexplained(fm: &str, body: &str) -> bool {
    tags_of(fm).iter().any(|t| t == "untestable") && !body.contains("## Why it is `untestable`")
}

/// The predicate itself, against a task that claims the exemption and says
/// nothing, and one that claims it properly.
#[test]
fn an_exemption_with_no_reason_is_what_this_gate_looks_for() {
    let fm = "id: T-1\nstatus: done\ntags:\n- cli\n- untestable\n";
    assert!(exemption_is_unexplained(fm, "\n## Why\n\nSomething.\n"));
    assert!(!exemption_is_unexplained(
        fm,
        "\n## Why it is `untestable`\n\nIt is prose.\n"
    ));
    // And a task that does not claim it is not asked for the section.
    assert!(!exemption_is_unexplained("tags:\n- cli\n", "\n## Why\n"));
    // The substring trap: citing a test whose name contains the tag is not a
    // claim to the exemption.
    let citing =
        "tags:\n- cli\nevidence:\n  - kind: test\n    ref: every_untestable_task_says_why\n";
    assert!(!exemption_is_unexplained(citing, "\n## Why\n"));
}

/// The exemption is a claim, so it carries its reason where a reader meets it.
#[test]
fn every_untestable_task_says_why_in_its_body() {
    let mut silent = Vec::new();
    let mut untagged = Vec::new();
    for (id, fm, body) in closed_tasks() {
        let exempt = tags_of(&fm).iter().any(|t| t == "untestable");
        let cites_test = fm.contains("kind: test");
        if exemption_is_unexplained(&fm, &body) {
            silent.push(id.clone());
        }
        let milestone = tags_of(&fm).iter().any(|t| t == "milestone");
        // The other direction: a closed task with neither proof nor a stated
        // exemption is what `APLAN017` refuses, and this fails first with a list
        // rather than leaving the reader to read a verdict line.
        if !exempt && !cites_test && !milestone {
            untagged.push(id);
        }
    }
    assert!(
        silent.is_empty(),
        "tagged `untestable` and does not say why: {silent:?}"
    );
    assert!(
        untagged.is_empty(),
        "closed with no test citation and no stated exemption: {untagged:?}"
    );
}

/// A tag nobody uses is a vocabulary, not a policy: if the exemption ever falls
/// out of use, this says so rather than leaving a dead word in the config.
fn is_explicit_exemption_free_epoch(tasks: &[String]) -> bool {
    !tasks.is_empty() && tasks.iter().all(|name| name.starts_with("A2-"))
}

#[test]
fn the_exemption_free_epoch_is_only_a2() {
    assert!(is_explicit_exemption_free_epoch(&[
        "A2-001.md".to_string(),
        "A2-004.md".to_string(),
    ]));
    assert!(!is_explicit_exemption_free_epoch(&[]));
    assert!(!is_explicit_exemption_free_epoch(&[
        "A2-001.md".to_string(),
        "A3-001.md".to_string(),
    ]));
}

#[test]
fn the_exemption_is_used_and_stays_a_minority() {
    let closed = closed_tasks();
    let exempt: Vec<&String> = closed
        .iter()
        .filter(|(_, fm, _)| tags_of(fm).iter().any(|t| t == "untestable"))
        .map(|(id, _, _)| id)
        .collect();
    if exempt.is_empty() {
        let tasks = std::fs::read_dir(repo_root().join("aplan/tasks"))
            .expect("public plan task directory")
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|value| value == "md"))
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(
            is_explicit_exemption_free_epoch(&tasks),
            "an exemption-free plan is valid here only at the explicit A2 public-plan epoch"
        );
        return;
    }
    // Measured when this landed: 14 of 108. A majority claiming it would mean the
    // rule had been routed around rather than adopted, which is the failure this
    // whole task exists to prevent.
    assert!(
        exempt.len() * 2 < closed.len(),
        "{} of {} closed tasks claim the exemption; the rule is being routed around",
        exempt.len(),
        closed.len()
    );
}

/// The policy is a *refusal*, not a line in a file, and this is what proves it
/// (`A1-123`).
///
/// `this_repositorys_policy_requires_a_test_and_says_so` asserts the
/// configuration; a configuration nobody runs proves nothing. This runs the
/// binary against a plan carrying the same policy and asks it to close a task on
/// a commit alone, which is the closure that was legal here for 108 tasks.
#[test]
fn a_commit_alone_no_longer_closes_a_task_under_this_policy() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan dir");
    std::fs::write(
        root.join(".aplan.yml"),
        "evidence:\n  require_min_strength: [test]\n  exempt_tags: [untestable, milestone]\n",
    )
    .expect("config");
    let task = |id: &str, tags: &str| {
        std::fs::write(
            root.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: A task\nphase: 1\nstatus: todo\neffort_days: 1\n{tags}\
                 ---\n\n## Why\n\nx\n"
            ),
        )
        .expect("task");
    };
    task("T-1", "");
    task("T-2", "tags:\n- untestable\n");
    // A repository, so a `file` citation resolves: it was an accepted kind under
    // the old policy and is the sharpest case, because nothing about it is
    // unresolvable -- it is simply not proof that the work was done.
    for args in [
        vec!["init", "-q"],
        vec!["add", "-A"],
        vec![
            "-c",
            "user.email=a@b.c",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
        ],
    ] {
        std::process::Command::new("git")
            .args(&args)
            .current_dir(root)
            .output()
            .expect("git");
    }

    let run = |args: &[&str]| {
        let out = common::output(root, args);
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        (text, out.status.code().unwrap_or(-1))
    };

    let (out, code) = run(&[
        "close",
        "T-1",
        "--evidence",
        "file:aplan/tasks/T-1.md",
        "--root",
        "aplan",
    ]);
    assert_ne!(code, 0, "a closure with no test was accepted:\n{out}");
    assert!(out.contains("APLAN017"), "{out}");
    assert!(
        std::fs::read_to_string(root.join("aplan/tasks/T-1.md"))
            .expect("read")
            .contains("status: todo"),
        "the refusal wrote the closure anyway"
    );

    // And the exemption is what lets the same closure through, which is the only
    // way past the rule.
    let (out, code) = run(&[
        "close",
        "T-2",
        "--evidence",
        "file:aplan/tasks/T-2.md",
        "--root",
        "aplan",
    ]);
    assert_eq!(code, 0, "the exemption did not exempt:\n{out}");
}
