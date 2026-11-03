//! A summary may roll up by a declared tag family (`A1-186`).
//!
//! The hard case is overlapping membership. `APLAN026` validates which tags
//! belong to the family; it does not say a task carries only one. Group rows
//! therefore may overlap, while the total remains a distinct count of plan
//! tasks and must never be obtained by adding those rows.

use std::path::Path;

use crate::common;

fn repo(config: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(dir.path().join(".aplan.yml"), config).expect("config");
    dir
}

fn task(root: &Path, id: &str, status: &str, effort: f64, tags: &[&str], blocked: &[&str]) {
    let tags = tags.join(", ");
    let blocked_by = if blocked.is_empty() {
        String::new()
    } else {
        format!("blocked_by: [{}]\n", blocked.join(", "))
    };
    std::fs::write(
        root.join(format!("aplan/tasks/{id}.md")),
        format!(
            "---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: {status}\neffort_days: {effort}\n\
             {blocked_by}tags: [{tags}]\n---\n\n## Why\n\nFixture.\n"
        ),
    )
    .expect("task");
}

fn raw_task(root: &Path, id: &str, tag_yaml: Option<&str>) {
    let tags = tag_yaml.map_or_else(String::new, |tag| format!("tags: [{tag}]\n"));
    std::fs::write(
        root.join(format!("aplan/tasks/{id}.md")),
        format!(
            "---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: todo\neffort_days: 1\n\
             {tags}---\n\n## Why\n\nFixture.\n"
        ),
    )
    .expect("task");
}

fn json(dir: &Path) -> serde_json::Value {
    let (out, code) = common::aplan(dir, &["summary", "--format", "json"]);
    assert_eq!(code, 0, "{out}");
    serde_json::from_str(&out).expect("summary json")
}

#[test]
fn a_declared_family_groups_summary_end_to_end_without_inflating_totals() {
    let dir = repo(
        "root: aplan\ntag_families:\n  kind: [kind-build, kind-review]\n\
         summary:\n  group_by: kind\n",
    );
    task(
        dir.path(),
        "S-1",
        "done",
        2.0,
        &["kind-build", "kind-review", "untestable"],
        &[],
    );
    task(dir.path(), "S-2", "todo", 3.0, &["kind-review"], &[]);
    task(dir.path(), "S-3", "todo", 4.0, &[], &["S-2"]);
    task(dir.path(), "S-4", "dropped", 5.0, &["kind-build"], &[]);

    let doc = json(dir.path());
    assert_eq!(doc["schema"], "aplan/summary-grouped/v1");
    assert_eq!(doc["group_by"], "kind");
    assert_eq!(doc["groups_overlap"], true);
    let groups = doc["groups"].as_array().expect("groups");
    assert_eq!(groups.len(), 3, "{doc}");
    assert_eq!(groups[0]["group"], "kind-build");
    assert_eq!(groups[0]["tasks"], 1);
    assert_eq!(groups[0]["done"], 1);
    assert_eq!(groups[0]["ready"], 0);
    assert_eq!(groups[0]["dropped"], 1);
    assert_eq!(groups[1]["group"], "kind-review");
    assert_eq!(groups[1]["tasks"], 2);
    assert_eq!(groups[1]["ready"], 1);
    assert!(groups[2]["group"].is_null(), "{doc}");
    assert_eq!(groups[2]["tasks"], 1);
    assert_eq!(groups[2]["ready"], 0);

    // S-1 appears in both named groups. It remains one task and 2d in the
    // unique total, so adding group rows is visibly not the total contract.
    assert_eq!(doc["total_tasks"], 3);
    assert_eq!(doc["total_done"], 1);
    assert_eq!(doc["total_effort_days"], 9.0);
    assert_eq!(doc["total_dropped"], 1);
    assert_eq!(doc["total_dropped_effort_days"], 5.0);
    assert_eq!(doc["ready"], 1);

    // Selection narrows what is counted, without recomputing readiness inside
    // the subset or summing overlapping rows back into the total.
    let (selected, code) = common::aplan(
        dir.path(),
        &["summary", "--format", "json", "--status", "todo"],
    );
    assert_eq!(code, 0, "{selected}");
    let selected: serde_json::Value = serde_json::from_str(&selected).expect("selected summary");
    assert_eq!(selected["total_tasks"], 2);
    assert_eq!(selected["total_done"], 0);
    assert_eq!(selected["ready"], 1);
    assert_eq!(selected["groups_overlap"], false);

    let (human, code) = common::aplan(dir.path(), &["summary"]);
    assert_eq!(code, 0, "{human}");
    assert!(human.contains("group"), "{human}");
    assert!(human.contains("kind-build"), "{human}");
    assert!(human.contains("kind-review"), "{human}");
    assert!(human.contains("(no group)"), "{human}");
    assert!(
        human.contains("group rows overlap; totals count each task once"),
        "{human}"
    );
}

#[test]
fn roadmap_and_summary_choose_their_grouping_independently() {
    let dir = repo(
        "root: aplan\ntag_families:\n  epic: [epic-a]\n  kind: [kind-build]\n\
         roadmap:\n  group_by: epic\nsummary:\n  group_by: kind\n",
    );
    task(
        dir.path(),
        "S-1",
        "todo",
        1.0,
        &["epic-a", "kind-build"],
        &[],
    );

    let doc = json(dir.path());
    assert_eq!(doc["group_by"], "kind");
    assert_eq!(doc["groups"][0]["group"], "kind-build");
    assert_ne!(doc["groups"][0]["group"], "epic-a");
}

#[test]
fn a_mapping_family_keeps_declaration_order_and_omits_empty_members() {
    let dir = repo(
        "root: aplan\ntag_families:\n  kind:\n    members: [kind-empty, kind-review, \
         kind-build]\n    require: optional\nsummary:\n  group_by: kind\n",
    );
    task(dir.path(), "S-1", "todo", 1.0, &["kind-build"], &[]);
    task(dir.path(), "S-2", "todo", 1.0, &["kind-review"], &[]);
    task(dir.path(), "S-3", "todo", 1.0, &[], &[]);

    let doc = json(dir.path());
    let labels: Vec<_> = doc["groups"]
        .as_array()
        .expect("groups")
        .iter()
        .map(|group| group["group"].clone())
        .collect();
    assert_eq!(
        labels,
        [
            serde_json::json!("kind-review"),
            serde_json::json!("kind-build"),
            serde_json::Value::Null,
        ],
        "{doc}"
    );
}

#[test]
fn an_undeclared_summary_family_is_refused_by_name() {
    let dir = repo("root: aplan\nsummary:\n  group_by: typo\n");
    let (out, code) = common::aplan(dir.path(), &["summary"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("`summary.group_by` names `typo`"), "{out}");
    assert!(out.contains("`tag_families` does not declare"), "{out}");
}

/// Proves `config.grouping_error_names_its_view`.
#[test]
fn an_undeclared_grouping_family_names_the_view_not_a_test_runner() {
    let summary = repo("root: aplan\nsummary:\n  group_by: typo\n");
    let roadmap = repo("root: aplan\nroadmap:\n  group_by: typo\n");
    for (dir, key) in [
        (&summary, "summary.group_by"),
        (&roadmap, "roadmap.group_by"),
    ] {
        let (out, code) = common::aplan(dir.path(), &["summary"]);
        assert_eq!(code, 2, "{out}");
        assert!(out.contains(key), "{out}");
        assert!(out.contains("invalid grouping in `.aplan.yml`"), "{out}");
        assert!(!out.contains("invalid test runner"), "{out}");
    }
}

#[test]
fn a_plan_without_summary_grouping_keeps_the_v2_shape() {
    let dir = repo("root: aplan\ntag_families:\n  kind: [kind-build]\n");
    task(dir.path(), "S-1", "todo", 1.0, &["kind-build"], &[]);
    let doc = json(dir.path());
    assert_eq!(doc["schema"], "aplan/summary/v2");
    assert!(doc.get("phases").is_some(), "{doc}");
    assert!(doc.get("groups").is_none(), "{doc}");
    assert!(doc.get("group_by").is_none(), "{doc}");
}

#[test]
fn a_family_member_cannot_break_the_human_table() {
    let dir = repo(
        "root: aplan\ntag_families:\n  kind: [\"kind\\nrow\"]\n\
         summary:\n  group_by: kind\n",
    );
    std::fs::write(
        dir.path().join("aplan/tasks/S-1.md"),
        "---\nid: S-1\ntitle: S-1\nphase: 1\nstatus: todo\neffort_days: 1\n\
         tags: [\"kind\\nrow\"]\n---\n\n## Why\n\nFixture.\n",
    )
    .expect("task");

    let (out, code) = common::aplan(dir.path(), &["summary"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("kind\nrow"), "a label split the row:\n{out}");
    assert!(out.contains("\"kind\\u{a}row\""), "{out}");
}

/// Proves `summary.grouped_human_is_unambiguous`.
#[test]
fn distinct_hostile_family_members_remain_distinguishable_in_human_summary() {
    let dir = repo(
        "root: aplan\ntag_families:\n  \"kind\\naxis\": [\"kind\\nrow\", \"kind row\", \
         \"(no group)\", total]\nsummary:\n  group_by: \"kind\\naxis\"\n",
    );
    raw_task(dir.path(), "S-1", Some("\"kind\\nrow\""));
    raw_task(dir.path(), "S-2", Some("\"kind row\""));
    raw_task(dir.path(), "S-3", Some("\"(no group)\""));
    raw_task(dir.path(), "S-4", Some("total"));
    raw_task(dir.path(), "S-5", None);

    let (out, code) = common::aplan(dir.path(), &["summary"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("grouped by \"kind\\u{a}axis\""), "{out}");
    for prefix in [
        "\"kind\\u{a}row\"",
        "\"kind row\"",
        "\"(no group)\"",
        "\"total\"",
        "(no group)",
        "total",
    ] {
        assert!(
            out.lines().any(|line| line.starts_with(prefix)),
            "missing distinct row `{prefix}`:\n{out}"
        );
    }

    // A wide member and an ASCII member with the same terminal width must
    // leave the same suffix. Comparing byte columns would make these differ.
    let aligned = repo(
        "root: aplan\ntag_families:\n  kind: [abcd, \"\\u65e5\\u672c\"]\n\
         summary:\n  group_by: kind\n",
    );
    raw_task(aligned.path(), "S-1", Some("abcd"));
    raw_task(aligned.path(), "S-2", Some("\"\\u65e5\\u672c\""));
    let (out, code) = common::aplan(aligned.path(), &["summary"]);
    assert_eq!(code, 0, "{out}");
    let ascii = out
        .lines()
        .find_map(|line| line.strip_prefix("\"abcd\""))
        .expect("ASCII member row");
    let wide_label = format!("\"{}{}\"", '\u{65e5}', '\u{672c}');
    let wide = out
        .lines()
        .find_map(|line| line.strip_prefix(&wide_label))
        .expect("wide member row");
    assert_eq!(ascii, wide, "numeric columns moved:\n{out}");

    // The machine contract carries raw values and is deliberately unchanged.
    let doc = json(dir.path());
    assert_eq!(doc["group_by"], "kind\naxis");
    assert_eq!(doc["groups"][0]["group"], "kind\nrow");
    assert_eq!(doc["groups"][4]["group"], serde_json::Value::Null);
}

/// Proves `config.tag_family_error_names_its_key`.
#[test]
fn an_invalid_tag_family_is_not_reported_as_a_test_runner() {
    for (config, detail) in [
        (
            "root: aplan\ntag_families:\n  kind: []\nsummary:\n  group_by: kind\n",
            "`tag_families.kind` is empty",
        ),
        (
            "root: aplan\ntag_families:\n  kind: [kind-a, kind-a]\nsummary:\n  group_by: kind\n",
            "`tag_families.kind` lists `kind-a` twice",
        ),
    ] {
        let dir = repo(config);
        let (out, code) = common::aplan(dir.path(), &["summary"]);
        assert_eq!(code, 2, "{out}");
        assert!(
            out.contains("invalid `tag_families` in `.aplan.yml`"),
            "{out}"
        );
        assert!(out.contains(detail), "{out}");
        assert!(!out.contains("invalid test runner"), "{out}");
    }
}
