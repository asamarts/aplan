//! **Every config switch a rule reads is one `vacuity` knows about** (`A1-269`).
//!
//! Which *rules* a switch stops is measured rather than listed, so a rule that
//! starts reading one joins on the commit that makes it do so. The switches
//! themselves are hand-written, and nothing made a new one join -- so a field
//! added to `Options` and read by a rule would be a rule `vacuity` reports as
//! having run when it did not, which is the defect that task was filed about,
//! one level up.
//!
//! This reads the rules' own source for the fields they consult and requires
//! each to be a switch or to be named here with the reason it is not one.

/// A field of `Options` that rules read but which is not an on/off switch.
const NOT_A_SWITCH: &[(&str, &str)] = &[
    (
        "traces_root",
        "where traced documents are read from, not whether they are: `APLAN008` runs either way \
         and resolves against a different directory",
    ),
    (
        "traces_repository",
        "the boundary `APLAN034` compares a resolved trace against, not whether that rule runs",
    ),
    (
        "traces_external",
        "which outside roots are declared, which changes what `APLAN034` accepts rather than \
         whether it speaks",
    ),
    (
        "evidence_for",
        "the one accessor `APLAN017` now reads, covering `require_min_strength`, `exempt_tags` \
         and `evidence_scopes` together. None of them is a switch: they decide which requirement \
         a subject is held to, not whether the rule runs, and the `subjects` column measures who \
         it could speak about. `exempt_tags` had its own entry here until scoping generalised it \
         and no rule read it directly any more (`A1-183`)",
    ),
    (
        "tag_families",
        "a vocabulary rather than a switch. Turning it `on` means inventing families, and a \
         family this tool made up would make `APLAN026`'s near-miss report nonsense",
    ),
    (
        "owners",
        "a vocabulary rather than a switch. Turning it `on` by copying the owners already in \
         the plan would declare every typo canonical, while inventing a roster would make \
         `APLAN056` report people the repository may legitimately use",
    ),
    (
        "config_file",
        "where the configuration was read from, which no rule's predicate consults",
    ),
    (
        "agents_block",
        "not a `.aplan.yml` key: it carries a block the *caller* rendered, and `APLAN041`'s \
         subject is a file outside the plan. Turning it `on` means inventing a contract to \
         compare against, and an invented one would report every repository's `AGENTS.md` as \
         stale -- the same reason `tag_families` cannot be flipped",
    ),
];

#[test]
fn every_options_field_a_rule_reads_is_a_switch_or_is_explained() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/validate.rs");
    let text = std::fs::read_to_string(&src).expect("validate.rs");
    let production = text
        .split_once("\n#[cfg(test)]")
        .map_or(text.as_str(), |(before, _)| before);

    // Fields the rules actually consult, read off `opts.<field>`.
    let mut read: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (_, rest) in production
        .match_indices("opts.")
        .map(|(i, _)| (i, &production[i + 5..]))
    {
        let field: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !field.is_empty() {
            read.insert(field);
        }
    }
    assert!(
        read.len() >= 5,
        "only {} field(s) found, so this audit is not reading the rules: {read:?}",
        read.len()
    );

    // The switches `vacuity` turns on, by the tail of the `.aplan.yml` key --
    // `evidence.require_min_strength` is the field `require_min_strength`.
    let vac = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vacuity.rs"),
    )
    .expect("vacuity.rs");
    let block = vac
        .split_once("const SWITCHES")
        .and_then(|(_, rest)| rest.split_once("];"))
        .map(|(block, _)| block)
        .expect("the switch table");
    let switches: std::collections::BTreeSet<String> = block
        .split('"')
        .skip(1)
        .step_by(2)
        .map(|k| k.rsplit('.').next().unwrap_or_default().to_string())
        .collect();
    assert!(
        switches.len() >= 3,
        "only {} switch(es) parsed, so this audit is not reading the table: {switches:?}",
        switches.len()
    );

    let mut unexplained = Vec::new();
    for field in &read {
        if switches.contains(field) || NOT_A_SWITCH.iter().any(|(f, _)| f == field) {
            continue;
        }
        unexplained.push(field.clone());
    }
    assert!(
        unexplained.is_empty(),
        "a rule reads `.aplan.yml` through a field `vacuity` neither turns on nor accounts for, \
         so a rule it stops is reported as having run (`A1-269`). Add it to `SWITCHES`, or to \
         `NOT_A_SWITCH` with the reason it cannot be one:\n  {}",
        unexplained.join("\n  ")
    );

    // And every exemption must still name a field the rules read, or the list
    // becomes a set of excuses for options that no longer exist.
    for (field, why) in NOT_A_SWITCH {
        assert!(
            read.contains(*field),
            "`{field}` is explained here ({why}) and no rule reads it any more"
        );
    }
}
