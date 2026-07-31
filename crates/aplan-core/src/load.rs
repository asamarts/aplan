//! Loading tasks from markdown files with YAML frontmatter.
//!
//! Frontmatter is validated against the schema *derived from* [`crate::task::Task`]
//! before deserialisation, so a malformed field produces a schema diagnostic
//! rather than an opaque serde error.

use std::path::{Path, PathBuf};

use crate::diagnostic::{Diagnostic, code};
use crate::task::Task;

/// A task plus the file it came from.
#[derive(Debug, Clone)]
pub struct LoadedTask {
    pub task: Task,
    pub path: PathBuf,
}

/// Everything a validation run needs.
#[derive(Debug, Default)]
pub struct Plan {
    pub tasks: Vec<LoadedTask>,
    /// Diagnostics raised during loading (schema, parse).
    pub load_diagnostics: Vec<Diagnostic>,
}

/// Split `---\n<yaml>\n---\n<body>` into its frontmatter and body.
///
/// Returns `None` when the source does not open with a `---\n` / `---\r\n`
/// fence, or opens with one that is never closed. The caller ([`load_plan`])
/// pre-filters non-fence files as "not a task", so a `None` it sees always
/// means an unterminated fence.
pub fn split_frontmatter(source: &str) -> Option<(&str, &str)> {
    // Tolerate one or more leading UTF-8 BOMs (Windows editors, PowerShell
    // redirection). Without this a BOM-prefixed task file is treated as "not a
    // task" and its invariants, including `done`-needs-evidence, never run.
    let source = source.trim_start_matches('\u{feff}');
    let rest = source
        .strip_prefix("---\n")
        .or_else(|| source.strip_prefix("---\r\n"))?;
    // Find a line that is exactly `---`.
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" {
            let fm = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return Some((fm, body));
        }
        offset += line.len();
    }
    None
}

/// The JSON Schema derived from the Rust types. This is the same value the
/// `gen-schema` xtask writes to `schemas/v1/task.json`.
pub fn derived_schema() -> serde_json::Value {
    let schema = schemars::schema_for!(Task);
    serde_json::to_value(schema).expect("schema is serialisable")
}

/// Load and schema-check a single task file.
fn load_one(path: &Path, validator: &jsonschema::Validator) -> Result<LoadedTask, Vec<Diagnostic>> {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            return Err(vec![
                Diagnostic::error(code::SCHEMA, format!("cannot read: {e}"))
                    .with_file(path)
                    .with_agent_instruction(
                        "Make the file readable, or remove it from the plan directory.".to_string(),
                    ),
            ]);
        }
    };

    let Some((frontmatter, _body)) = split_frontmatter(&source) else {
        // The walker only routes files that open with a `---` fence to
        // `load_one`, so a `None` split means the fence was opened but never
        // closed.
        return Err(vec![
            Diagnostic::error(
                code::SCHEMA,
                "frontmatter fence (`---`) opened but never closed",
            )
            .with_file(path)
            .with_agent_instruction(
                "Close the YAML frontmatter with a `---` line, or remove the opening fence."
                    .to_string(),
            ),
        ]);
    };

    // YAML -> JSON so we can schema-check before typed deserialisation.
    let value: serde_json::Value = match serde_yaml_ng::from_str(frontmatter) {
        Ok(v) => v,
        Err(e) => {
            return Err(vec![
                Diagnostic::error(code::SCHEMA, format!("invalid YAML frontmatter: {e}"))
                    .with_file(path)
                    .with_agent_instruction("Fix the YAML syntax in the frontmatter.".to_string()),
            ]);
        }
    };

    let schema_errors: Vec<Diagnostic> = validator
        .iter_errors(&value)
        .map(|e| {
            let at = e.instance_path();
            Diagnostic::error(code::SCHEMA, format!("{e} (at `{at}`)"))
                .with_file(path)
                .with_agent_instruction(format!(
                    "Fix the frontmatter field at `{at}` to satisfy schemas/v1/task.json."
                ))
        })
        .collect();
    if !schema_errors.is_empty() {
        return Err(schema_errors);
    }

    // Deserialise straight from the YAML (not from the schema-checked `Value`):
    // the `serde_json::Value` step silently keeps the last of any duplicate keys,
    // so `id: A` twice would pass unnoticed. Serde rejects a duplicate field, the
    // way `.aplan.yml` loading already does.
    match serde_yaml_ng::from_str::<Task>(frontmatter) {
        Ok(task) => Ok(LoadedTask {
            task,
            path: path.to_path_buf(),
        }),
        Err(e) => Err(vec![
            Diagnostic::error(
                code::SCHEMA,
                format!("frontmatter does not deserialise: {e}"),
            )
            .with_file(path)
            .with_agent_instruction(
                "Correct the frontmatter to match the task schema (`aplan schema`).".to_string(),
            ),
        ]),
    }
}

/// Walk `root` for `*.md` files and load every one that has frontmatter.
///
/// A plan validator must *see every task file*, so the walk deliberately does
/// not honour `.gitignore`/`.ignore`, nor skip dotfiles, nor care about the
/// case of the `.md` extension: a gitignored, hidden, or `TASK.MD` file is still
/// a task, and silently dropping one is the exact failure this tool exists to
/// catch. It skips only well-known non-plan directories (VCS metadata, build
/// output) so pointing `--root` at a repository root does not descend into
/// `.git` or `target`; the default root is the plan directory, which contains
/// none of these, so that skip is belt-and-suspenders.
///
/// Files whose first line is not a bare `---` fence are skipped silently; a
/// candidate that fails to load (unterminated fence, invalid YAML, schema
/// violation) produces a diagnostic; an unreadable file is reported, never
/// silently dropped.
pub fn load_plan(root: &Path) -> Plan {
    let schema = derived_schema();
    let validator = match jsonschema::validator_for(&schema) {
        Ok(v) => v,
        Err(e) => {
            return Plan {
                tasks: Vec::new(),
                load_diagnostics: vec![
                    Diagnostic::error(
                        code::SCHEMA,
                        format!("internal: derived schema is not a valid JSON Schema: {e}"),
                    )
                    .with_agent_instruction(
                        "This is an internal error in aplan; please file a bug.".to_string(),
                    ),
                ],
            };
        }
    };

    let mut plan = Plan::default();

    // Sort the walk so load order, and therefore the order of any tie-broken
    // diagnostics, is identical across filesystems and CI runners, not the
    // OS `readdir` order.
    for entry in ignore::WalkBuilder::new(root)
        .hidden(false)
        .parents(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        // Prune VCS metadata (never holds tasks) at any depth, but prune build
        // output (`target`/`node_modules`) ONLY as an immediate child of the root,
        // where it is the `--root .`-at-a-repo-root case. A plan subdirectory that
        // happens to be named `target` (e.g. `plan/tasks/target/x.md`) must still
        // be walked: silently dropping a task is the exact failure this tool
        // exists to catch.
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true; // never prune the root itself
            }
            let name = e.file_name().to_str();
            if matches!(name, Some(".git" | ".jj" | ".hg" | ".svn")) {
                return false;
            }
            !(e.depth() == 1 && matches!(name, Some("target" | "node_modules")))
        })
        .sort_by_file_path(Path::cmp)
        .build()
        .flatten()
    {
        let path = entry.path();
        // Case-insensitive: `.MD`/`.Md` (a Windows editor, a careless rename) is
        // still a task file, not something to silently drop.
        if !path.is_file()
            || path
                .extension()
                .is_none_or(|e| !e.eq_ignore_ascii_case("md"))
        {
            continue;
        }
        // Pre-filter: a task candidate is a file whose first line is a bare
        // `---` fence (after any leading BOMs). A `----` rule or a `---foo`
        // line is not a fence and is skipped silently; an unreadable file is
        // *reported*, not skipped, silently dropping a file is the failure
        // this tool exists to catch.
        let head = match std::fs::read_to_string(path) {
            Ok(h) => h,
            Err(e) => {
                plan.load_diagnostics.push(
                    Diagnostic::error(code::SCHEMA, format!("cannot read: {e}"))
                        .with_file(path)
                        .with_agent_instruction(
                            "Make the file readable, or remove it from the plan directory."
                                .to_string(),
                        ),
                );
                continue;
            }
        };
        let opens_fence = head
            .trim_start_matches('\u{feff}')
            .lines()
            .next()
            .is_some_and(|l| l.trim_end_matches('\r') == "---");
        if !opens_fence {
            continue;
        }
        match load_one(path, &validator) {
            Ok(t) => plan.tasks.push(t),
            Err(mut d) => plan.load_diagnostics.append(&mut d),
        }
    }

    plan.tasks.sort_by(|a, b| a.task.id.cmp(&b.task.id));
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_frontmatter() {
        let src = "---\nid: X\n---\nbody here\n";
        let (fm, body) = split_frontmatter(src).expect("has frontmatter");
        assert_eq!(fm, "id: X\n");
        assert_eq!(body, "body here\n");
    }

    #[test]
    fn no_fence_is_none() {
        assert!(split_frontmatter("# just a doc\n").is_none());
    }

    #[test]
    fn unterminated_fence_is_none() {
        assert!(split_frontmatter("---\nid: X\nnever closed\n").is_none());
    }

    #[test]
    fn derived_schema_is_valid_json_schema() {
        let schema = derived_schema();
        assert!(jsonschema::validator_for(&schema).is_ok());
    }

    #[test]
    fn splits_frontmatter_with_bom() {
        let src = "\u{feff}---\nid: X\n---\nbody\n";
        let (fm, body) = split_frontmatter(src).expect("BOM tolerated");
        assert_eq!(fm, "id: X\n");
        assert_eq!(body, "body\n");
    }

    /// Regression: a BOM-prefixed task (common from Windows editors) was
    /// silently skipped, so a `done` with no evidence slipped through.
    #[test]
    fn bom_prefixed_task_is_loaded_not_skipped() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("t.md"),
            "\u{feff}---\nid: T\ntitle: t\nphase: 0\nstatus: todo\n---\nx\n",
        )
        .expect("write");
        let plan = load_plan(dir.path());
        assert_eq!(plan.tasks.len(), 1, "BOM file must not be skipped");
        assert!(
            plan.load_diagnostics.is_empty(),
            "{:?}",
            plan.load_diagnostics
        );
    }

    /// Regression: the walk was unsorted, so diagnostics that tie on
    /// (code, task) came out in filesystem order, divergent across runners.
    #[test]
    fn load_order_is_deterministic() {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["c.md", "a.md", "b.md"] {
            std::fs::write(
                dir.path().join(name),
                "---\nid: X\ntitle: t\nphase: 0\nstatus: bogus\n---\nx\n",
            )
            .expect("write");
        }
        let plan = load_plan(dir.path());
        let files: Vec<_> = plan
            .load_diagnostics
            .iter()
            .filter_map(|d| d.file.as_ref())
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        assert_eq!(files, vec!["a.md", "b.md", "c.md"]);
    }

    #[test]
    fn strips_multiple_leading_boms() {
        let src = "\u{feff}\u{feff}---\nid: X\n---\nbody\n";
        let (fm, _) = split_frontmatter(src).expect("double BOM tolerated");
        assert_eq!(fm, "id: X\n");
    }

    /// Regression: a `----` rule or a `---foo` line is not a `---` fence and
    /// must be skipped silently, not turned into a false `APLAN001`.
    #[test]
    fn non_fence_markdown_is_skipped_not_errored() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("notes.md"), "----\n\nnotes\n").expect("write");
        std::fs::write(dir.path().join("other.md"), "---foo\nbar\n").expect("write");
        let plan = load_plan(dir.path());
        assert_eq!(plan.tasks.len(), 0);
        assert!(
            plan.load_diagnostics.is_empty(),
            "non-fence markdown must be skipped, not errored: {:?}",
            plan.load_diagnostics
        );
    }

    /// Regression: a file that opens with a `---` fence but never closes it is
    /// reported with an accurate message (not "no fence at start of file").
    #[test]
    fn unterminated_fence_reports_accurately() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("t.md"),
            "---\n# heading, no close\n\nprose\n",
        )
        .expect("write");
        let plan = load_plan(dir.path());
        assert_eq!(plan.tasks.len(), 0);
        let d = plan
            .load_diagnostics
            .iter()
            .find(|d| d.code == code::SCHEMA)
            .expect("a diagnostic");
        assert!(
            d.message.contains("never closed"),
            "message should name the unterminated fence, got: {}",
            d.message
        );
    }

    /// Regression: an unreadable task file must be reported, not silently
    /// dropped (silent skip is the failure the tool exists to catch).
    #[cfg(unix)]
    #[test]
    fn unreadable_task_is_reported_not_skipped() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("u.md");
        std::fs::write(&p, "---\nid: U\ntitle: t\nphase: 0\nstatus: done\n---\nx\n")
            .expect("write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        let plan = load_plan(dir.path());
        let dropped = plan.load_diagnostics.is_empty() && plan.tasks.is_empty();
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644));
        assert!(
            !dropped,
            "an unreadable task file must not be silently dropped"
        );
    }

    /// Regression: the VCS/build-output prune must not drop a task in a *nested*
    /// directory that happens to be named `target`/`node_modules`. Only VCS
    /// metadata (any depth) and a root-level build dir are skipped.
    #[test]
    fn a_task_in_a_nested_build_named_dir_is_not_dropped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let task = "---\nid: {ID}\ntitle: t\nphase: 0\nstatus: done\n---\nx\n";
        for (rel, id) in [
            ("tasks/target/deep.md", "NEST1"),
            ("a/node_modules/x.md", "NEST2"),
            ("normal.md", "NORMAL"),
        ] {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, task.replace("{ID}", id)).unwrap();
        }
        let plan = load_plan(dir.path());
        let ids: Vec<&str> = plan.tasks.iter().map(|t| t.task.id.as_str()).collect();
        for id in ["NEST1", "NEST2", "NORMAL"] {
            assert!(ids.contains(&id), "{id} was dropped: {ids:?}");
        }
    }

    /// A duplicate top-level key in task frontmatter must be rejected, not
    /// silently deduped to the last value (as the `Value` step used to do).
    #[test]
    fn duplicate_frontmatter_keys_are_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("t.md"),
            "---\nid: A\nid: B\ntitle: t\nphase: 0\nstatus: todo\n---\nx\n",
        )
        .expect("write");
        let plan = load_plan(dir.path());
        assert!(plan.tasks.is_empty(), "a duplicate key must not load");
        assert!(
            plan.load_diagnostics
                .iter()
                .any(|d| d.message.contains("duplicate")),
            "expected a duplicate-field error: {:?}",
            plan.load_diagnostics
        );
    }

    /// A negative `effort_days` is meaningless and would corrupt a rollup; the
    /// schema's `minimum: 0` rejects it at load (APLAN001).
    #[test]
    fn a_negative_effort_is_a_schema_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("t.md"),
            "---\nid: T\ntitle: t\nphase: 0\nstatus: todo\neffort_days: -5\n---\nx\n",
        )
        .expect("write");
        let plan = load_plan(dir.path());
        assert!(plan.tasks.is_empty(), "a negative effort must not load");
        assert!(
            plan.load_diagnostics.iter().any(|d| d.code == code::SCHEMA),
            "expected a schema diagnostic: {:?}",
            plan.load_diagnostics
        );
    }

    /// Regression: `.gitignore`/`.ignore` rules, hidden dotfiles, and an
    /// uppercase `.MD` extension must NOT hide a task from the gate. A silently
    /// dropped `done`-without-evidence task is the exact failure aplan exists to
    /// catch, so the walk sees every `.md` regardless of ignore rules or case.
    #[test]
    fn ignore_rules_case_and_dotfiles_do_not_hide_tasks() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A `.ignore` (honoured with no git repo, unlike `.gitignore`) that would
        // match a task file:
        std::fs::write(dir.path().join(".ignore"), "secret*.md\n").expect("write");
        let task = "---\nid: {ID}\ntitle: t\nphase: 0\nstatus: done\nevidence:\n  - kind: commit\n    ref: abc1234\n---\nx\n";
        std::fs::write(dir.path().join("secret-a.md"), task.replace("{ID}", "IGN")).expect("write");
        std::fs::write(dir.path().join("UPPER.MD"), task.replace("{ID}", "UP")).expect("write");
        std::fs::write(dir.path().join(".hidden.md"), task.replace("{ID}", "HID")).expect("write");
        let plan = load_plan(dir.path());
        let ids: Vec<&str> = plan.tasks.iter().map(|t| t.task.id.as_str()).collect();
        assert!(
            ids.contains(&"IGN"),
            "ignored task hidden from the gate: {ids:?}"
        );
        assert!(ids.contains(&"UP"), "uppercase `.MD` task hidden: {ids:?}");
        assert!(ids.contains(&"HID"), "hidden dotfile task dropped: {ids:?}");
    }

    /// The `agent` output format promises *every* diagnostic carries an
    /// `agent_instruction`; the parse/load diagnostics must honour that too.
    #[test]
    fn parse_error_diagnostics_carry_an_agent_instruction() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("bad.md"), "---\nid: [unclosed\n---\nx\n").expect("write");
        let plan = load_plan(dir.path());
        let d = plan
            .load_diagnostics
            .iter()
            .find(|d| d.code == code::SCHEMA)
            .expect("a diagnostic");
        assert!(
            d.agent_instruction.is_some(),
            "load diagnostics must carry an agent_instruction: {d:?}"
        );
    }
}
