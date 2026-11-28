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
        "scoped_evidence_for",
        "the one accessor `APLAN017` reads for scoped evidence policy overrides \
         (`policy[].evidence.require_min_strength` and `exempt_tags`), paired with \
         `require_min_strength` as the whole-plan fallback (`A1-183`, `A2-135`)",
    ),
    (
        "evidence_scopes",
        "a list of scoped policy rules rather than an on/off switch. `APLAN058` checks each \
         declared `policy[].when` selector against the plan's milestones, tags and tag families, \
         and `APLAN059` checks each `policy[].evidence.require_min_strength` against configured \
         resolvers; turning it `on` means inventing policy rules the repository never wrote",
    ),
    (
        "configured_resolvers",
        "the set of configured evidence resolvers declared in `.aplan.yml` (`test`, `url.check`), \
         not an on/off switch. `APLAN059` warns when a policy requires a kind whose resolver is \
         absent, while `vacuity` reports resolver-gated verification rules through \
         `resolver_side_off` using the same keys",
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
        "id_families",
        "a per-cut vocabulary mapping rather than an on/off switch. Inventing a cut-to-prefix \
         mapping for a plan that treats ids as opaque would fabricate `APLAN064` mismatches",
    ),
    (
        "max_effort_days",
        "a repository-declared numeric review-size limit rather than an on/off switch. Turning \
         it `on` would mean inventing a universal task-size limit the repository never declared, \
         fabricating `APLAN067` findings across plans that did not opt in (`A2-149`)",
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

    // The switches declared in the authoritative rule catalogue (`A2-135`),
    // paired with the exact `Options` field each switch reads.
    let mut switches = std::collections::BTreeSet::new();
    let mut keys = std::collections::BTreeSet::new();
    for &switch in aplan_core::ValidationSwitch::ALL {
        assert!(
            switches.insert(switch.options_field().to_string()),
            "duplicate ValidationSwitch options_field `{}`",
            switch.options_field()
        );
        assert!(
            keys.insert(switch.config_key()),
            "duplicate ValidationSwitch config_key `{}`",
            switch.config_key()
        );
        assert!(
            !switch.rules().is_empty(),
            "ValidationSwitch `{}` governs no rule in Rule::ALL",
            switch.config_key()
        );
    }
    assert!(
        switches.len() >= 3,
        "only {} switch(es) declared in ValidationSwitch::ALL: {switches:?}",
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
