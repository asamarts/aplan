//! End-to-end tests for CLI terminal Markdown rendering, `--raw-body`, and
//! the executable CLI Markdown-bearing output site census (`A2-115`).

use aplan_output::markdown::{CLI_MARKDOWN_SITES, MarkdownOutputDisposition};

use crate::common;

fn fixture_with_markdown_body() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("markdown-cli-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let tasks = dir.path().join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("tasks dir");
    std::fs::create_dir_all(dir.path().join("src")).expect("src dir");
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn demo() {}\n").expect("src");

    // Task with rich Markdown body (headings, emphasis, table, fenced code, link, image,
    // raw HTML, and hostile CSI/OSC/bidi sequences) AND no trailing newline in raw bytes.
    let body = "## Why\n\n\
                Summary with *italic*, **bold**, `code`, and [spec](https://example.invalid/s).\n\n\
                ![arch](https://example.invalid/arch.png)\n\n\
                ```rust\n\
                let x = 1;\n\
                ```\n\n\
                | Task | Effort | Owner |\n\
                |---|---:|---|\n\
                | A-1 | 2.0d | alice |\n\n\
                Hostile \x1b[31mred\x1b[0m \x1b]8;;https://evil.invalid\x07click\x1b]8;;\x07 \
                safe\u{202e}gnirts <script>alert(1)</script>";
    std::fs::write(
        tasks.join("A-1.md"),
        format!(
            "---\n\
             id: A-1\n\
             title: Markdown body task\n\
             phase: 1\n\
             status: todo\n\
             effort_days: 2.0\n\
             paths: [src/**]\n\
             tags: [cross-cutting]\n\
             ---\n\n\
             {body}"
        ),
    )
    .expect("task A-1");

    // Task with empty body for `--raw-body` empty-output rule.
    std::fs::write(
        tasks.join("A-2.md"),
        "---\n\
         id: A-2\n\
         title: Empty body task\n\
         phase: 1\n\
         status: todo\n\
         effort_days: 1.0\n\
         paths: [src/**]\n\
         tags: [cross-cutting]\n\
         ---\n",
    )
    .expect("task A-2");

    dir
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Proof for `show.renders_markdown_body`: human `show --body` renders
/// Markdown through the semantic renderer (bold headings, aligned tables,
/// fenced code labels, narrow/ASCII fallback, sanitization, and color
/// invariance), while `--raw-body` emits verbatim stored bytes under the
/// final-newline rule and refuses incompatible flags, and JSON keeps the exact
/// body string.
#[test]
#[allow(clippy::too_many_lines)]
fn show_body_renders_markdown_and_raw_body_preserves_exact_bytes() {
    let dir = fixture_with_markdown_body();

    // 1. Human `show --body` with `--color never` (redirected/plain) and `--color always`.
    let plain = common::run(
        dir.path(),
        &["show", "A-1", "--body", "--no-resolve", "--color", "never"],
    );
    assert!(plain.ok(), "plain show --body failed: {plain:?}");
    // Rendered features that raw display would never emit:
    assert!(plain.stdout.contains("[code: rust]"), "{}", plain.stdout);
    assert!(
        plain.stdout.contains("A-1  \u{2502}   2.0d \u{2502} alice"),
        "aligned Unicode table missing:\n{}",
        plain.stdout
    );
    assert!(
        plain
            .stdout
            .contains("[image: arch (https://example.invalid/arch.png)]"),
        "{}",
        plain.stdout
    );
    assert!(
        plain.stdout.contains("spec (https://example.invalid/s)"),
        "{}",
        plain.stdout
    );
    // Malicious controls & bidi overrides are neutralized; raw HTML text is preserved literally.
    assert!(!plain.stdout.contains('\x1b'), "raw ESC leaked");
    assert!(!plain.stdout.contains('\u{202e}'), "bidi RLO leaked");
    assert!(
        plain
            .stdout
            .replace('\n', "")
            .contains("<script>alert(1)</script>"),
        "{}",
        plain.stdout
    );

    let colored = common::run(
        dir.path(),
        &["show", "A-1", "--body", "--no-resolve", "--color", "always"],
    );
    assert!(colored.ok(), "colored show --body failed: {colored:?}");
    assert!(
        colored.stdout.contains("\x1b[1m## Why"),
        "expected bold heading in colored output:\n{}",
        colored.stdout
    );
    assert_eq!(
        strip_ansi(&colored.stdout),
        plain.stdout,
        "stripping ANSI from --color always must match --color never byte-for-byte"
    );

    // 2. `--ascii` and narrow `--width 18` trigger ASCII borders and vertical table card fallback.
    let narrow = common::run(
        dir.path(),
        &[
            "show",
            "A-1",
            "--body",
            "--no-resolve",
            "--ascii",
            "--width",
            "18",
            "--color",
            "never",
        ],
    );
    assert!(narrow.ok(), "narrow ascii show failed: {narrow:?}");
    assert!(narrow.stdout.contains("| let x = 1;"), "{}", narrow.stdout);
    assert!(narrow.stdout.contains("Task: A-1"), "{}", narrow.stdout);
    assert!(narrow.stdout.contains("Effort: 2.0d"), "{}", narrow.stdout);
    assert!(narrow.stdout.contains("Owner: alice"), "{}", narrow.stdout);

    // 3. `--raw-body` emits ONLY the exact stored bytes + one trailing newline (when missing),
    // and zero bytes for an empty body.
    let raw = common::run(
        dir.path(),
        &[
            "show",
            "A-1",
            "--raw-body",
            "--no-resolve",
            "--color",
            "always",
        ],
    );
    assert!(raw.ok(), "raw-body failed: {raw:?}");
    assert!(raw.stdout.starts_with("\n## Why\n\n"), "{:?}", raw.stdout);
    assert!(raw.stdout.ends_with('\n'));
    assert!(raw.stdout.contains("```rust\nlet x = 1;\n```"));
    assert!(!raw.stdout.contains("[code: rust]"));
    assert!(
        raw.stdout.contains("\x1b[31mred\x1b[0m"),
        "raw-body must preserve verbatim stored bytes"
    );

    let empty_raw = common::run(dir.path(), &["show", "A-2", "--raw-body", "--no-resolve"]);
    assert!(empty_raw.ok(), "empty raw-body failed: {empty_raw:?}");
    assert!(
        empty_raw.stdout.is_empty(),
        "empty task body must emit 0 bytes: {:?}",
        empty_raw.stdout
    );

    // 4. `--raw-body` refuses incompatible presentation flags and `--format json`.
    for bad in [
        &["show", "A-1", "--raw-body", "--body"][..],
        &["show", "A-1", "--raw-body", "--ascii"][..],
        &["show", "A-1", "--raw-body", "--width", "40"][..],
        &["show", "A-1", "--raw-body", "--format", "json"][..],
    ] {
        let refused = common::run(dir.path(), bad);
        assert!(
            !refused.ok(),
            "expected refusal for `aplan {}`, got: {refused:?}",
            bad.join(" ")
        );
    }

    // 5. `--body --format json` carries the exact unrendered body string.
    let json = common::run(
        dir.path(),
        &["show", "A-1", "--body", "--format", "json", "--no-resolve"],
    );
    assert!(json.ok(), "json show failed: {json:?}");
    let parsed: serde_json::Value = serde_json::from_str(&json.stdout).expect("valid json");
    let json_body = parsed["body"].as_str().expect("body string");
    assert!(json_body.contains("```rust\nlet x = 1;\n```"));
    assert!(!json_body.contains("[code: rust]"));
}

/// Executable census over `CLI_MARKDOWN_SITES` (`A2-115`): drives every
/// declared CLI Markdown-bearing site and proves its rendered, raw-source, or
/// byte-exact artifact disposition.
#[test]
fn cli_markdown_output_site_census_drives_every_assigned_disposition() {
    let dir = fixture_with_markdown_body();

    // Seed one capture so `inbox show` has a real record to drive.
    let cap = common::run(
        dir.path(),
        &[
            "capture",
            "Inbox markdown note",
            "--body",
            "## Capture heading\n\n*raw body*",
            "--format",
            "json",
        ],
    );
    assert!(cap.ok(), "capture failed: {cap:?}");
    let cap_json: serde_json::Value = serde_json::from_str(&cap.stdout).expect("capture json");
    let cap_id = cap_json["capture"]["id"]
        .as_str()
        .expect("capture id")
        .to_string();

    for site in CLI_MARKDOWN_SITES {
        match (site.command, site.surface, site.disposition) {
            ("show", "show --body (human)", MarkdownOutputDisposition::Rendered) => {
                let res = common::run(dir.path(), &["show", "A-1", "--body", "--no-resolve"]);
                assert!(res.ok());
                assert!(res.stdout.contains("[code: rust]"));
            }
            ("show", "show --raw-body", MarkdownOutputDisposition::RawSource) => {
                let res = common::run(dir.path(), &["show", "A-1", "--raw-body", "--no-resolve"]);
                assert!(res.ok());
                assert!(res.stdout.contains("```rust"));
            }
            ("show", "show --body --format json", MarkdownOutputDisposition::RawSource) => {
                let res = common::run(
                    dir.path(),
                    &["show", "A-1", "--body", "--format", "json", "--no-resolve"],
                );
                assert!(res.ok());
                let v: serde_json::Value = serde_json::from_str(&res.stdout).expect("json");
                assert!(v["body"].as_str().is_some_and(|b| b.contains("```rust")));
            }
            ("inbox", "inbox show", MarkdownOutputDisposition::RawSource) => {
                let res = common::run(dir.path(), &["inbox", "show", &cap_id]);
                assert!(res.ok(), "inbox show failed: {res:?}");
                assert!(res.stdout.contains("## Capture heading"));
                assert!(res.stdout.contains("*raw body*"));
            }
            ("roadmap", "roadmap --format markdown", MarkdownOutputDisposition::Artifact) => {
                let res = common::run(dir.path(), &["roadmap", "--format", "markdown"]);
                assert!(res.ok());
                assert!(res.stdout.contains("<!-- generated by"));
                assert!(!res.stdout.contains('\x1b'));
            }
            (
                "deprecations",
                "deprecations --format markdown",
                MarkdownOutputDisposition::Artifact,
            ) => {
                let res = common::run(dir.path(), &["deprecations", "--format", "markdown"]);
                assert!(res.ok());
                assert!(!res.stdout.contains('\x1b'));
            }
            ("export-agents-md", "export-agents-md", MarkdownOutputDisposition::Artifact) => {
                let res = common::run(dir.path(), &["export-agents-md"]);
                assert!(res.ok());
                assert!(res.stdout.contains("<!-- aplan:begin"));
                assert!(!res.stdout.contains('\x1b'));
            }
            ("export", "export --format checklist", MarkdownOutputDisposition::Artifact) => {
                let res = common::run(dir.path(), &["export", "--format", "checklist"]);
                assert!(res.ok());
                assert!(
                    res.stdout.contains("- [ ] Markdown body task"),
                    "{}",
                    res.stdout
                );
                assert!(!res.stdout.contains('\x1b'));
            }
            other => panic!("unhandled CLI_MARKDOWN_SITES entry: {other:?}"),
        }
    }
}
