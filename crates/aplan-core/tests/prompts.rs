//! **Every prompt this tool writes is declared in one place** (`A1-271`).
//!
//! `APLAN039` refuses a prompt handed back as an answer, and it can only refuse
//! the ones it knows. A writer with its own copy of the sentence is invisible to
//! it -- and that was not hypothetical: `checklist` and `roadmap_json` each held
//! a verbatim duplicate of `add`'s line, and `emit`'s import body held a
//! *different* sentence the rule had never been told about, while a comment
//! claimed the two were the same. So a task created by `import`, the path that
//! makes tasks in bulk, could be closed with its prompt ticked and nothing said
//! so.
//!
//! An integration test rather than a unit one because it reads another crate's
//! source: it is an audit of the workspace, not of a function.

/// A criterion prompt written into a body somewhere `aplan_core::scaffold` does
/// not declare.
#[test]
fn every_prompt_this_tool_writes_is_declared_in_scaffold() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../aplan-interop/src");
    let declared = aplan_core::scaffold::criterion_prompts();
    let mut stray = Vec::new();
    let mut scanned = 0usize;

    for entry in std::fs::read_dir(&dir)
        .expect("aplan-interop/src")
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read");
        // Production only: the test modules build fixtures that quote prompts
        // on purpose.
        let production = text
            .split_once("\n#[cfg(test)]")
            .map_or(text.as_str(), |(before, _)| before);
        scanned += 1;
        for (n, line) in production.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            // A checkbox *written into a body*, not one a message mentions. The
            // importers describe their own input format in errors -- "no
            // `- [ ] T001 ...` items were found" -- and that is a code span, so
            // the character before the marker tells the two apart.
            let Some(at) = trimmed.find("- [ ] ") else {
                continue;
            };
            if at > 0 && trimmed.as_bytes()[at - 1] == b'`' {
                continue;
            }
            let sentence =
                trimmed[at + "- [ ] ".len()..].trim_end_matches(['\\', '"', 'n', ',', ' ']);
            // Interpolated *at the marker*, so a constant is what it writes and
            // this has nothing to judge. Checked at the start rather than
            // anywhere in the line: a hardcoded prompt that merely shares a line
            // with some other `{}` is exactly what this is looking for, and a
            // laxer test let one through when this was first written.
            if sentence.starts_with('{') || sentence.len() < 20 {
                continue;
            }
            if !declared.contains(&sentence) {
                stray.push(format!(
                    "{}:{}: {sentence}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    n + 1
                ));
            }
        }
    }

    assert!(
        scanned >= 5,
        "only {scanned} file(s) scanned, so this audit is not reaching the crate that writes bodies"
    );
    assert!(
        stray.is_empty(),
        "a criterion prompt is written somewhere `aplan_core::scaffold` does not declare, so \
         `APLAN039` cannot see it. Move it into `criterion_prompts`:\n  {}",
        stray.join("\n  ")
    );
}
