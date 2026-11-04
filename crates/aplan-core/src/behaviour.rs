//! Named behaviours a consumer can assert (`A1-099`).
//!
//! `facts` describes the *surface*: the rules, commands, config keys, formats and
//! evidence kinds a build has. A change that widens what an existing name accepts
//! leaves no trace there. ADR-0015 taught `APLAN008` to resolve a table row, and
//! `--require APLAN008` passes on every build since the rule existed, which is
//! long before the behaviour did, so the fourth pilot depended on a behaviour it
//! could not assert (pre-public adoption evidence, item 15).
//!
//! It was tolerable there only because that failure is loud: an older binary
//! rejects the row anchor outright. A widening the other way, an older binary
//! silently accepting an input and doing something broader with it, would be a
//! blocker, and the same blind spot would have hidden it.
//!
//! **When a change earns a name here.** When it changes what an *existing* name
//! accepts or means, so no new rule code, command, config key, format or evidence
//! kind appears. A change that adds one of those is already assertable and must
//! not be listed twice. That rule is in `AGENTS.md`, where the next author will
//! meet it.
//!
//! **A name here is a claim, and this project does not take claims on trust**
//! (`A1-211`). Each entry names the tests that prove it, and
//! `crates/aplan/tests/behaviour_proof.rs` refuses one that names a test this
//! workspace does not have. Without that, `--require` answered from the list
//! alone: fourteen of the first eighteen names appeared in no test source at
//! all. Mostly that was a missing *connection* rather than a missing test --
//! the tests existed, beside the code, saying nothing about the promise they
//! were holding up -- but not always, and the difference is invisible until
//! someone looks: `ready.order_is_derived` turned out to be pinned by nothing,
//! and `ready` could revert to id order with the suite green (`A1-211`). A
//! behaviour that stops working goes on being asserted by name unless something
//! fails when it breaks, which is the one failure `--require` is pinned to
//! catch.

/// A behaviour a build either has or does not, addressable by name.
#[derive(Debug, Clone, Copy)]
pub struct Behaviour {
    /// The name `--require` accepts. Dotted, and never an `APLANxxx` code: a rule
    /// code is a stable consumer contract and a dotted variant of one would read
    /// as a rule.
    pub name: &'static str,
    /// What a build having it can do, in one line.
    pub what: &'static str,
    /// The tests that prove this build has it, by function name (`A1-211`).
    ///
    /// `what` is a sentence, and a sentence goes on being true in the source
    /// long after the code stopped honouring it. `--require` answers from this
    /// list's *existence*, so a behaviour that quietly stopped working would
    /// still be asserted by name. Naming the proofs here makes the field the
    /// compiler's business: a new behaviour cannot be added without saying what
    /// would fail if it broke, and [`crate::behaviour`]'s audit
    /// (`crates/aplan/tests/behaviour_proof.rs`) refuses a name that is not a
    /// test in this workspace.
    ///
    /// A behaviour with more than one half names a test for each: `close`
    /// resolving before it writes is a promise about two surfaces, and one test
    /// can only speak for one of them.
    pub proof: &'static [&'static str],
}

/// Every named behaviour this build has.
pub const ALL: &[Behaviour] = &[
    // `A1-515`. `APLAN057` and its capture schema already existed, but an older
    // build accepts a body made only of whitespace as durable content. The
    // rule code cannot distinguish that weaker predicate.
    Behaviour {
        name: "capture.body_is_non_vacuous",
        what: "capture bodies from profiles, commands and loaded records are absent or contain visible content, never blank or invisible placeholders",
        proof: &["capture_body_is_non_vacuous"],
    },
    // `A1-515`. The strict inbox loader already existed. The guarded writer's
    // same-filesystem staging name is not repository data, and treating its
    // microsecond lifetime or crash-left artifact as a malformed capture makes
    // a valid concurrent command fail nondeterministically.
    Behaviour {
        name: "capture.writer_staging_is_not_a_record",
        what: "the strict capture loader ignores only the guarded writer's exact canonical staging-file pattern while continuing to reject every other non-record inbox member",
        proof: &["capture_writer_staging_is_not_a_record"],
    },
    // `A1-515`. Top-level command facts name `inbox`, but stop before its
    // nested command tree, so a build without these lifecycle operations needs
    // a separately assertable name.
    Behaviour {
        name: "capture.inbox_lifecycle_commands",
        what: "the inbox command provides focused show, open-only amendment, discard and discarded-only restore over one guarded capture snapshot",
        proof: &["capture_inbox::capture_cli_create_list_show_amend_discard_restore_and_dry_run"],
    },
    // `A1-516`. Top-level command facts still stop at `inbox`; this names the
    // nested operation and the stronger guarded publication contract it adds.
    Behaviour {
        name: "capture.promotion_is_atomic",
        what: "inbox promotion creates one ordinary validated task and freezes its capture link in one retry-safe guarded transaction whose late failure rolls back every write",
        proof: &["capture_inbox::capture_promotion_is_validated_atomic_and_idempotent"],
    },
    // `A1-518`. Native capture forms and the guarded lifecycle operations
    // already existed, but the plan input identity deliberately excludes the
    // inbox. An older build could therefore seed editable defaults from one
    // retained record version and preview them against a later version.
    Behaviour {
        name: "tui.capture_forms_are_record_bound",
        what: "every native TUI form populated from a retained capture carries that record's immutable fingerprint and refuses preview if the capture changes or the visible id is redirected before review",
        proof: &["selected_capture_forms_refuse_a_record_changed_after_the_form_was_seeded"],
    },
    // `A1-531`. Claim, release and every claim-reading command already existed,
    // but older builds stored the active holder in durable task `owner` and
    // protected only one worktree's copy of that file.
    Behaviour {
        name: "claim.uses_worktree_shared_ephemeral_state",
        what: "`claim` records time, origin and identity-source attribution in private local \
               state shared by every linked worktree, while durable task `owner` remains stable",
        proof: &["claim::exactly_one_cross_worktree_claimant_is_told_it_won"],
    },
    Behaviour {
        name: "claim.concurrent_disjoint_updates_compose",
        what: "concurrent claims or releases for different tasks retry guarded whole-store \
               contention so every nonconflicting update is retained",
        proof: &["concurrent_claims_and_releases_on_distinct_tasks_all_compose"],
    },
    // `A2-013`. The guarded writer and both private application caches already
    // existed, but Windows can transiently deny an atomic replacement while a
    // concurrent reader still has the destination open. Older builds surfaced
    // that ordinary sharing race as a permanent mutation failure.
    Behaviour {
        name: "storage.atomic_replacement_is_portable",
        what: "guarded mutations and private cache refreshes retry transient Windows sharing \
               denials while retaining the same atomic replacement and stale-input boundaries",
        proof: &[
            "concurrent_claims_and_releases_on_distinct_tasks_all_compose",
            "one_same_size_edit_updates_the_persisted_generation",
            "observation_publication_replaces_an_older_generation",
        ],
    },
    // `A2-013`. Rekey already refused a selected configuration outside the
    // repository, but Windows canonicalized an existing repository boundary
    // with an extended-length prefix while leaving a missing child absolute.
    // That made a missing in-repository configuration look external.
    Behaviour {
        name: "authoring.rekey_repository_boundary_is_portable",
        what: "rekey applies one canonical repository boundary to existing and missing selected \
               configuration paths on every platform before reading or rewriting policy",
        proof: &["rekey_policy_failures_are_typed_before_publication"],
    },
    // `A2-013`. Provider material identity already resolved a configured bare
    // executable through PATH, but unlike Windows process launch it did not
    // try the implicit `.exe` suffix, so ordinary Git providers were refused.
    Behaviour {
        name: "evidence.provider_executable_resolution_is_portable",
        what: "provider material identity resolves configured executables with the same native \
               spelling accepted by process launch, including Windows' implicit `.exe` suffix",
        proof: &[
            "provider_executable_resolution_follows_native_command_lookup",
            "material_provider_executable_resolution_follows_native_lookup",
        ],
    },
    Behaviour {
        name: "coordination.is_not_a_plan_verdict_mutation",
        what: "`claim` and `release` mutate only local ephemeral coordination state, so they do \
               not accept plan-verdict overrides or imply that the plan passed a gate",
        proof: &["mutation_policy::local_coordination_commands_refuse_plan_verdict_overrides"],
    },
    Behaviour {
        name: "release.recovers_orphaned_local_claims",
        what: "`release` can explicitly clear an ephemeral claim whose task is absent, while \
               corrupt, unsafe or other orphaned claim state is never treated as free work",
        proof: &[
            "corrupt_and_orphaned_state_never_looks_free_but_orphans_can_be_released",
            "claim::a_branch_that_cannot_see_a_claim_fails_closed_until_explicit_release",
        ],
    },
    Behaviour {
        name: "owner.remains_stable_responsibility",
        what: "task `owner` is durable responsibility and claim or release never rewrites it",
        proof: &["claim::read_surfaces_share_ephemeral_claims_and_keep_owner_stable"],
    },
    Behaviour {
        name: "owner.refuses_vacuous_values",
        what: "task creation and supersession refuse an owner made only of whitespace or \
               invisible formatting characters rather than writing responsibility nobody can see",
        proof: &["claim::a_blank_name_is_refused_rather_than_written_or_ignored"],
    },
    Behaviour {
        name: "selection.unclaimed_uses_ephemeral_claims",
        what: "`--unclaimed` selects the absence of an observed ephemeral claim, independently \
               of durable `owner`, and refuses when local coordination is unavailable",
        proof: &[
            "claim::read_surfaces_share_ephemeral_claims_and_keep_owner_stable",
            "claim::unclaimed_refuses_when_local_coordination_cannot_be_observed",
        ],
    },
    Behaviour {
        name: "ready.exposes_ephemeral_claims",
        what: "`ready` and `aplan_ready` expose stable owner plus one local claim snapshot with \
               holder, time, origin and explicit coordination scope",
        proof: &["claim::read_surfaces_share_ephemeral_claims_and_keep_owner_stable"],
    },
    Behaviour {
        name: "show.exposes_ephemeral_claims",
        what: "`show` and `aplan_show` report stable owner and ephemeral claim as distinct facts, \
               including claim age, origin and coordination scope",
        proof: &["claim::read_surfaces_share_ephemeral_claims_and_keep_owner_stable"],
    },
    Behaviour {
        name: "stale.exposes_ephemeral_claims",
        what: "`stale` reports stable owner plus claim age and origin without a claim refreshing \
               the task-file clock used to find neglected work",
        proof: &["claim::read_surfaces_share_ephemeral_claims_and_keep_owner_stable"],
    },
    Behaviour {
        name: "tasks.exposes_ephemeral_claims",
        what: "`tasks --format json` carries the same explicit coordination scope and complete \
               per-task claim records as the other read surfaces",
        proof: &["claim::read_surfaces_share_ephemeral_claims_and_keep_owner_stable"],
    },
    Behaviour {
        name: "tui.distinguishes_owner_and_ephemeral_claim",
        what: "the TUI task list and inspector display the active local claim separately from \
               durable task owner, including age, source, origin and unavailable coordination",
        proof: &["complete_task_inspector_renders_every_section_and_relationship_state"],
    },
    Behaviour {
        name: "start.reports_ephemeral_holder",
        what: "`start` reports the ephemeral holder from local claim state rather than treating \
               durable `owner` as the actor working now",
        proof: &["claim::starting_and_stopping_move_the_status_and_nothing_else"],
    },
    Behaviour {
        name: "rekey.refuses_claimed_task",
        what: "`rekey` refuses an id carrying a local claim until it is explicitly released, so \
               the old claim cannot become orphaned while the new id looks free",
        proof: &["claim::rekey_refuses_a_claim_that_would_become_an_orphan"],
    },
    Behaviour {
        name: "claim.warns_on_placeholder_identity",
        what: "`claim` retains deterministic identity precedence but warns when the winning \
               unauthenticated label looks like a machine or test placeholder",
        proof: &["claim::a_placeholder_identity_is_actionably_warned"],
    },
    Behaviour {
        name: "claim.warns_on_cross_origin_identity_collision",
        what: "a holder label already observed from another local worktree origin produces an \
               actionable warning that separate actors need distinct labels",
        proof: &["claim::exactly_one_cross_worktree_claimant_is_told_it_won"],
    },
    Behaviour {
        name: "claim.origin_fallback_skips_invalid_host_values",
        what: "claim-origin attribution uses the first valid configured host label rather than \
               letting a blank, overlong or control-bearing primary value suppress a valid fallback",
        proof: &["host_label_selection_skips_invalid_candidates_before_falling_back"],
    },
    // `A1-589`. Application input identities already existed in CLI, MCP and
    // TUI responses, but were hashes of Rust's developer-facing `Debug` text.
    // No command or format name distinguishes that unstable representation
    // from the typed canonical identity used by the persistent store.
    Behaviour {
        name: "input_identity.uses_typed_canonical_bytes",
        what: "application input identities encode validation policy, verdict, configuration, \
               paths, document state and explicit absence as deterministic typed bytes rather \
               than Rust Debug output or map iteration order",
        proof: &["input_identity_uses_typed_canonical_policy_bytes"],
    },
    // `A1-572`. `APLAN011` already meant "resolved", but a Git object shared
    // with an abandoned linked worktree satisfied it even when no history
    // rooted at the verified tree contained the work. The rule code alone
    // cannot distinguish that weaker build from this one.
    Behaviour {
        name: "evidence.commit_requires_head_reachability",
        what: "APLAN011 accepts a commit citation only when its immutable SHA resolves to HEAD \
               or one of HEAD's ancestors; APLAN012 and secondary commit rules never use an \
               unreachable object, while incomplete shallow ancestry remains a checkout fault",
        proof: &[
            "commit_evidence_requires_head_reachability_before_relevance",
            "unreachable_commit_cannot_acquit_diluted_reachable_evidence",
            "unreachable_ordinary_commit_cannot_acquit_reachable_merge_evidence",
            "shallow_history_does_not_turn_unknown_ancestry_into_task_blame",
            "ladder::only_commits_reachable_from_head_can_be_commit_evidence",
            "mcp_close_claims::mcp_close_refuses_commit_outside_head_history",
        ],
    },
    // `A1-562`. `check-docs` already claimed that a reference has a word
    // boundary at both ends, but its right-hand test omitted underscore and
    // could silently accept a longer token by truncating it to a known id.
    Behaviour {
        name: "check_docs.identifier_boundaries_are_symmetric",
        what: "`check-docs` applies one Unicode-aware word-continuation predicate at both ends \
               of ordinary, dotted, suffix-bearing and near-prefix task references",
        proof: &[
            "identifier_word_boundaries_are_symmetric_across_every_family",
            "docs::check_docs_does_not_truncate_ids_before_an_underscore",
        ],
    },
    // `A1-561`. Prefix-typo detection already existed, but its derived
    // vocabulary did not preserve the primary scanner's longest-first family
    // order. Neither the command nor the original behavior name distinguishes
    // a build that can misread `A11-999` as the invented shorter id `A1-1`.
    Behaviour {
        name: "check_docs.near_prefixes_prefer_longest_family",
        what: "`check-docs` matches overlapping one-digit-typo families longest-first, so the \
               finding retains the complete referenced id and recommends the exact source family",
        proof: &["overlapping_near_prefixes_are_matched_longest_first"],
    },
    // `A1-557`. The `check-docs` command already existed, but its plan-learned
    // scanner stopped a reference at the numeric core even when the same plan
    // used hyphenated suffixes in that family. The command name cannot tell an
    // adopter whether `M-p0-design` resolves or is misreported as `M-p0`.
    Behaviour {
        name: "check_docs.complete_multi_hyphen_ids",
        what: "`check-docs` learns when a numeric id family carries hyphen-separated suffixes, \
               resolves known complete ids and reports an unknown sibling without truncating it",
        proof: &[
            "complete_multi_hyphen_ids_are_resolved_and_reported_without_truncation",
            "docs::check_docs_resolves_complete_multi_hyphen_ids_end_to_end",
        ],
    },
    // `A1-389` added action results to the existing `agent` format. Its refusal
    // used only that format's first surface description, so it said catalogue
    // and action commands rendered findings. The command and format names alone
    // cannot distinguish a build whose corrective guidance is accurate.
    Behaviour {
        name: "formats.refusal_names_every_rendered_surface",
        what: "a rejected known format describes every kind of output it can render and names the commands that accept it, rather than attributing its first surface to all of them",
        proof: &["format_refusal_names_every_rendered_surface"],
    },
    // `A1-427`. The `tui` command already existed, so neither the command nor
    // a format name distinguishes the foundation shell from a build whose
    // task view applies the shared selector and ready order interactively.
    Behaviour {
        name: "tui.task_browser_preserves_selector_semantics",
        what: "the TUI task browser searches ids and titles, applies every shared selector \
               dimension without changing plan semantics, exposes deterministic ready ranking, \
               and preserves task identity across narrowing and refresh",
        proof: &[
            "task_browser_preserves_selector_semantics_and_ready_rank",
            "task_browser_search_filters_sort_and_match_highlighting_are_operable",
            "refresh_preserves_selection_by_id_and_reports_removal",
        ],
    },
    // `A1-546`, found by the adversarial audit of `A1-427`. The task browser
    // already accepted search input and preserved hidden task identity, but an
    // enhanced-keyboard repeat vanished inside the modal and a refresh or
    // hidden identity could leave a cursor addressing no visible row.
    Behaviour {
        name: "tui.task_browser_state_is_refresh_and_repeat_safe",
        what: "the TUI task browser applies enhanced-keyboard repeats to search editing without \
               repeating modal activation, clamps a filter cursor after refresh, and anchors a \
               hidden task identity at the visible viewport start",
        proof: &[
            "task_search_accepts_enhanced_keyboard_repeat_events",
            "refresh_clamps_the_filter_cursor_to_the_refreshed_vocabulary",
            "a_hidden_selection_anchors_rendering_and_hit_testing_at_the_visible_start",
        ],
    },
    // `A1-428`. `show` already existed but omitted persisted fingerprints,
    // relationship kinds and version claims. Its command and format names
    // cannot distinguish a build whose purportedly complete projection drops
    // those fields.
    Behaviour {
        name: "show.carries_complete_task_projection",
        what: "the canonical typed `show` projection carries every persisted relationship, \
               fingerprint and version claim, distinguishes missing from unloadable neighbours, \
               and can include a body from the same identified input",
        proof: &[
            "show::the_typed_show_projection_carries_every_persisted_task_field",
            "task_inspectors_are_complete_show_projections_of_the_identified_input",
            "duplicate_ids_use_one_consistent_task_and_body_projection",
        ],
    },
    // `A1-590`. `show`, `aplan_show` and interactive verification all existed,
    // but ordinary task inspection synchronously started every configured test
    // catalogue and could therefore hang before emitting output. No command or
    // evidence-kind name distinguishes a provider-free read from that build.
    Behaviour {
        name: "show.evidence_observations_are_nonblocking",
        what: "ordinary CLI and MCP task inspection never starts a configured provider and exposes typed per-citation and aggregate observation freshness; explicit bounded refresh recomputes the current plan against complete raw catalogues, reports exact runner progress, publishes only stable complete observations, and rejects stale late results",
        proof: &[
            "evidence_observation::show_default_never_starts_configured_providers_and_requires_current_explicitly",
            "evidence_observation::refresh_reuses_raw_catalogues_rederives_plan_changes_and_marks_input_changes_stale",
            "show_carries_machine_applicable_repairs_to_mcp",
            "bounded_catalogue_is_event_driven_deterministic_and_exactly_once",
            "verification_executor_rejects_an_edit_that_races_provider_work",
            "provider_progress_precedes_the_typed_completion",
            "verification_progress_uses_a_privacy_safe_provider_ordinal_without_a_fake_percentage",
        ],
    },
    // `A1-658`, found by the adversarial audit of A1-643. Scheduled catalogue
    // discovery already preserved row order, but its isolated worker resolver
    // rendered a later unnamed runner's fault and cost as `runner 1`. The
    // schedule/config keys do not distinguish that misattributing build.
    Behaviour {
        name: "evidence.scheduled_runner_identity_is_configured",
        what: "bounded provider discovery attributes catalogue rows, diagnostics and cost subjects to the same configured runner even when several runners have no name or directory label",
        proof: &["bounded_catalogue_preserves_the_configured_identity_of_an_unnamed_failure"],
    },
    // `A1-604`, found while making MCP carry the selected repository's full
    // evidence policy. `show` already accepted a configured trace root, but
    // discarded it before APLAN045 read historical document bytes, silently
    // omitting the finding whenever traces lived below the repository root.
    Behaviour {
        name: "evidence.stale_premise_uses_configured_roots",
        what: "APLAN045 makes configured absolute plan paths repository-relative and resolves \
               historical traced-document bytes through the configured trace root on `verify`, \
               `show` and `aplan_show`",
        proof: &[
            "show::verify_and_show_resolve_stale_premises_through_configured_roots",
            "mcp::an_explicit_root_selects_an_independent_repository_over_stdio",
        ],
    },
    // The TUI command also predates this inspector, and a build with only its
    // foundation shell advertises the same command. Kept in the feature-gated
    // namespace so a headless build cannot promise UI code it did not compile.
    Behaviour {
        name: "tui.task_inspector_uses_show_projection",
        what: "the TUI task inspector searches task bodies, separates summary, body, relationships, \
               paths, traces, evidence and findings from canonical `show`, wraps and scrolls inert \
               text, preserves context through section and task links, and discards stale inspector \
               coordinates when its snapshot refreshes",
        proof: &[
            "task_browser_search_returns_unicode_safe_case_insensitive_spans",
            "inspector_sections_and_relationships_preserve_back_navigation",
            "refresh_reconciles_inspector_state_with_the_new_snapshot",
            "complete_task_inspector_renders_every_section_and_relationship_state",
            "inspector_page_scrolling_clamps_and_can_leave_selection_following",
        ],
    },
    // `A1-429`. The TUI command and task inspector predate the joined work
    // flow. Neither name distinguishes a build that merely lists ready rows
    // from one that carries the rank, bounded/complete why semantics and an
    // explicit due clock through one identified projection.
    Behaviour {
        name: "tui.work_discovery_is_one_typed_flow",
        what: "the TUI joins ready rank factors and declared preferences, cut-bounded or complete \
               why answers, and revisit due state at an explicit clock in one typed task \
               inspector; due never claims an external condition is satisfied, and every loaded \
               blocker or dependent is navigable with an identity-preserving back stack",
        proof: &[
            "ready_rank_factors_and_preferences_are_explicit",
            "why_traversal_labels_folds_and_undefined_blockers",
            "revisit_and_status_verdicts_are_exact_at_the_boundary",
            "work_discovery_controls_and_links_share_one_inspector_flow",
            "work_discovery_commands_are_contextual_and_rendered",
            "tui_journey::work_discovery_runs_end_to_end_in_a_real_terminal",
        ],
    },
    // `A1-581`, found by the adversarial audit of `A1-429`. The original
    // behaviour name cannot distinguish a build that parses whitespace as a
    // blank clock but persists it as an explicit clock, or that attributes an
    // unavailable ready row to the preferred task instead of the subject.
    Behaviour {
        name: "tui.work_discovery_state_is_semantically_exact",
        what: "the TUI canonicalizes a blank work clock so it follows refreshed snapshot time, \
               keeps an explicit clock pinned, attributes an inactive preference to the exact \
               unavailable side, and proves folded why and revisit controls against real data",
        proof: &[
            "blank_work_clock_tracks_refresh_while_explicit_clock_stays_pinned",
            "work_discovery_controls_and_links_share_one_inspector_flow",
            "work_discovery_commands_are_contextual_and_rendered",
            "tui_journey::work_discovery_runs_end_to_end_in_a_real_terminal",
        ],
    },
    // `A1-426`. The TUI command and Overview route both predate the programme
    // health projection, so neither existing name distinguishes placeholder
    // counts from a build whose values and drill-down identities are one
    // application result.
    Behaviour {
        name: "tui.overview_is_one_typed_projection",
        what: "the TUI Overview renders plan, build and refresh identity plus cut, effort, ready, \
               due, finding and critical-path values from one typed projection, preserves honest \
               empty and partial states, and drills each aggregate into the exact identities that \
               produced it",
        proof: &[
            "overview_projects_counts_membership_due_and_findings_once",
            "overview_names_empty_all_done_no_effort_and_partial_load_states",
            "overview_aggregates_drill_into_their_exact_projection_members",
            "overview_freshness_never_calls_a_failed_refresh_current",
            "overview_renders_every_typed_number_and_honest_edge_state",
            "overview_findings_drill_is_exact_bounded_and_subject_aware",
        ],
    },
    // `A1-547`, found by the adversarial audit of `A1-426`. The Overview
    // already printed due and future revisit counts under the typed-projection
    // behavior, but retained identities only for the due half. An older build
    // therefore advertises the original behavior while a drill silently omits
    // every task counted as `LATER`.
    Behaviour {
        name: "tui.overview_revisit_membership_is_exact",
        what: "the TUI Overview retains distinct timestamped identities for due and future \
               revisits, and opening the composite row re-projects every task counted by both \
               sides from the current snapshot",
        proof: &[
            "overview_projects_counts_membership_due_and_findings_once",
            "overview_revisit_row_preserves_due_and_future_membership_after_refresh",
        ],
    },
    // `A1-430`. The Diagnostics route and `check`, `check-docs` and `explain`
    // command names all predate this workbench. An older build can advertise
    // every one while showing only lossy structural rows with no document
    // scope, instruction, contribution, complete-run state or stable identity.
    Behaviour {
        name: "tui.diagnostic_workbench_retains_complete_runs",
        what: "the TUI retains structural and configured-document diagnostics as distinct complete runs with independent verdicts, exact ordered findings and stable identities; filtering and grouping preserve canonical order and cannot turn a partial or denied run into a clean verdict, while detail carries subject, location, instruction, contribution and safe source context",
        proof: &[
            "diagnostic_workbench_preserves_scope_verdict_location_instruction_and_vacuity",
            "diagnostic_workbench_filters_without_reordering_or_recomputing_verdict",
            "diagnostic_refresh_preserves_identity_and_announces_delta",
            "diagnostic_renderer_keeps_whole_verdict_and_shows_actionable_detail",
            "narrowed_diagnostics_never_render_as_clean",
            "the_tui_context_loads_and_refreshes_configured_document_diagnostics",
            "exact_rule_lookup_and_task_navigation_return_to_the_finding",
        ],
    },
    // `A1-549`, found by the adversarial audit of `A1-430`. The workbench
    // retained its complete denied result internally, but an empty narrowed
    // list still rendered the clean-run sentence. The broader behavior above
    // was already advertised by that build, so this correction needs its own
    // assertable name.
    Behaviour {
        name: "tui.narrowed_diagnostic_state_is_truthful",
        what: "an active diagnostic search or filter is always disclosed, an empty narrowed view names the retained whole-run verdict instead of claiming the run is clean, and narrowing and one-run relaxation remain visible before truncatable metadata",
        proof: &["narrowed_diagnostics_never_render_as_clean"],
    },
    // `A1-430`. Existing structural check flags are not enumerated by facts,
    // and an older TUI has no way to prove whether a relaxed interactive run
    // became a hidden session default. The request-owned override is the
    // behavior staged consumers need to assert.
    Behaviour {
        name: "tui.check_overrides_are_one_run",
        what: "the TUI advanced check sheet exposes effective effort, trace and warning policy, carries explicit overrides on one effect request, labels a relaxed result, and restores repository policy for the next ordinary refresh",
        proof: &[
            "diagnostic_workbench_preserves_scope_verdict_location_instruction_and_vacuity",
            "advanced_check_overrides_are_explicit_and_one_run",
        ],
    },
    // `A1-431`. The `tui` and `verify` command names both predate this
    // integration. An older build can advertise each while offering no typed
    // interactive evidence ledger, no pre-run trust sheet and no responsive
    // bounded execution path.
    Behaviour {
        name: "tui.verification_is_typed_and_non_blocking",
        what: "the TUI reviews identified repository, policy and provider inputs before running verification on its bounded worker; execute is explicit, complete findings and per-citation strength, outcome, detail and shared cost stay typed, exact repairs remain inspect-only, and queued, running, completed, refused, timed-out, output-limited, unavailable and stale states cannot impersonate one another",
        proof: &[
            "verification_context_is_identified_and_uses_the_typed_engine",
            "observed_verification_reports_each_citations_actual_resolver_state",
            "a_shallow_checkout_preserves_the_highest_rung_it_actually_reached",
            "a_test_list_that_never_returns_is_abandoned_and_reported",
            "an_output_limited_test_listing_is_distinct_in_the_citation_ledger",
            "verification_run_sheet_and_results_keep_every_state_explicit",
            "navigation_remains_available_while_an_effect_is_blocked",
            "cancelling_a_queued_effect_prevents_it_from_starting",
            "stale_completion_is_visible_and_cannot_replace_the_snapshot",
            "terminal_modes_restore_before_shutdown_waits_for_a_worker",
        ],
    },
    // `A1-552`, found by the adversarial audit of `A1-431`. The run sheet
    // already carried a digest and stale answers were discarded, but the
    // worker reloaded configuration and could launch a replacement provider
    // before the reducer learned that the answer was stale. The older build
    // advertises the broad verification behavior above while crossing the
    // trust boundary it says was reviewed, so the correction needs a name.
    Behaviour {
        name: "tui.verification_execution_is_preview_bound",
        what: "interactive verification reloads and compares the reviewed input before any configured provider can run, reports a mismatch as typed stale work, and derives the evidence repository solely from the identified application context",
        proof: &[
            "tui_verification_refuses_changed_provider_before_execution",
            "verification_context_is_identified_and_uses_the_typed_engine",
            "a_failed_effect_keeps_the_last_complete_model",
        ],
    },
    // `A1-441`. Close, amend, restamp and verify --fix already existed, as did
    // the TUI and its generic preview form. Older builds can therefore
    // advertise every surface name while still resolving evidence on the input
    // thread or committing a preview after its repository or selected policy
    // changed.
    Behaviour {
        name: "tui.evidence_mutation_previews_bind_identified_inputs",
        what: "interactive close, amend, restamp and verification repair work is staged as one typed guarded ChangeSet whose evidence ledger and policy are inspectable, and publication re-observes its identified plan, repository HEAD/status and selected configuration inputs",
        proof: &[
            "evidence_mutation_preview_refuses_repository_and_configuration_only_races",
            "every_evidence_kind_drives_input_help_resolution_and_preview_results",
            "verification_fix_is_one_guarded_batch_with_no_repair_and_stale_controls",
        ],
    },
    // `A1-663`, found by the third closure audit of v0.12. Generic plan forms
    // already discarded an old response after editing, but the claim form's
    // separately typed work preview bypassed that token check. Its candidate
    // also repeated an outer action beside the preview's own action, allowing
    // a corrupt handoff to disagree with itself.
    Behaviour {
        name: "tui.work_preview_responses_are_request_bound",
        what: "an interactive work preview derives its action and review projection from the immutable typed request and can replace a form only when its request token, selected task and identified plan input still match the unchanged form",
        proof: &["work_previews_are_bound_to_form_request_task_and_input"],
    },
    // `A1-663`. A dismissed running prepare/preview left its effect live, but
    // opening another mutation installed a new Loading layer before the
    // one-mutation guard refused its request. A late response for the old
    // operation could then populate that unrelated layer.
    Behaviour {
        name: "tui.mutation_layers_do_not_adopt_prior_effects",
        what: "after a running mutation layer is dismissed, another mutation cannot open until the prior effect finishes, and that late result is discarded without creating or populating a new layer",
        proof: &["a_running_mutation_must_finish_before_another_layer_can_open"],
    },
    // `A1-663`, found by the same source-boundary audit. Manifest and import
    // promised to reject streams and devices, but opened a path before asking
    // what it was, so a FIFO without a writer could occupy the sole mutation
    // worker forever instead of reaching the refusal.
    Behaviour {
        name: "tui.authoring_capture_refuses_non_regular_before_blocking",
        what: "interactive manifest and import capture accepts only a bounded regular file and refuses a stream or device before opening it can block the mutation worker, including a regular-file-to-FIFO replacement race on Unix",
        proof: &["captured_sources_refuse_non_regular_files_before_open_can_block"],
    },
    // `A1-662`, found by the second closure audit of v0.12. The typed
    // configured-test client and evidence forms already existed, but only a
    // test called the client: the palette treated the forms themselves as the
    // `test` command's interactive answer. That passed the discovery audit
    // while no terminal action could list or select a configured test.
    Behaviour {
        name: "tui.configured_test_selector_is_reachable",
        what: "the TUI command palette starts configured-test discovery on the bounded worker, retains partial and ambiguous resolver advice, searches the typed answer locally, and inserts only an unambiguous exact reference into the selected task's close or amend form",
        proof: &[
            "configured_test_selector_runs_on_the_worker_and_confirms_its_input",
            "configured_test_selector_is_reachable_and_inserts_the_exact_reference",
            "configured_test_palette_action_has_a_searchable_selectable_modal",
            "tui_journey::configured_test_selector_reaches_a_seeded_close_form_in_a_real_terminal",
        ],
    },
    // `A1-662`, found by the same reachability audit. Dependency and tag
    // catalogues were typed and well tested, but only the tests called them;
    // the live forms showed free-text fields with help that claimed candidate
    // context existed elsewhere.
    Behaviour {
        name: "tui.planning_forms_render_typed_choice_context",
        what: "interactive dependency and preference forms render the complete typed endpoint catalogue with status, cut membership and current-edge state while excluding self; tag forms render declared family scope, cardinality, members, selected values and observed arbitrary tags before preview",
        proof: &[
            "catalogues_and_review_rows_expose_context_and_readiness_without_cli_text",
            "tui_journey::mutations_claim_start_edit_dependencies_add_and_close_in_a_real_terminal",
        ],
    },
    // `A1-442`. The generic form and guarded publisher already existed, but an
    // older interactive client can flatten a late multi-file failure into one
    // error, discard submitted scope after confirmation, or call a successful
    // writer return committed before a fresh snapshot observes the result.
    Behaviour {
        name: "tui.broad_mutations_are_recoverable",
        what: "broad interactive mutations enumerate every affected task and file, bind exact plan and selected-configuration bytes, require a deliberately re-focused confirmation, retain form input and per-path unchanged/complete/diverged observations after failure, and report success only after a fresh snapshot contains the reviewed after-state",
        proof: &[
            "mutation_snapshot_identity_is_not_only_repository_head",
            "snapshot_reconciliation_observes_exact_plan_relocation_and_config_bytes",
            "every_broad_operation_family_lists_its_complete_scope",
            "broad_confirmation_and_partial_failure_name_every_path_and_recovery_step",
            "unchanged_complete_and_diverged_failures_render_distinct_recovery",
            "applied_publication_is_not_success_until_reload_reconciles_exact_bytes",
        ],
    },
    // `A1-443`. The TUI and every mutation command already existed, and the
    // workflow modules already exposed typed forms. Older builds can advertise
    // all of those names while offering no production path from a terminal key
    // to the guarded publisher, and while a newly added CLI mode can be absent
    // from the interactive audit.
    Behaviour {
        name: "tui.mutation_surface_has_cross_adapter_parity",
        what: "the interactive mutation catalogue is a closed-world union of typed workflows; \
               task/plan CLI modes are mapped or visibly bounded, representative coordination, planning, \
               authoring and evidence writes cross a real PTY review boundary, and shared CLI, \
               MCP and TUI requests retain exact post-state and refusal parity",
        proof: &[
            "tui_mutation_surface::every_native_mutation_drives_parity_and_each_contract_state",
            "tui_mutation_surface::deleting_one_driver_makes_the_closed_world_audit_fail",
            "tui_mutation_surface::every_mutating_cli_mode_is_native_or_has_an_explicit_boundary",
            "tui_journey::mutations_claim_start_edit_dependencies_add_and_close_in_a_real_terminal",
            "tui_work_action_parity::cli_and_tui_claim_start_stop_release_leave_the_same_state",
            "tui_planning_parity::cli_and_typed_tui_planning_operations_publish_the_same_plan_bytes",
            "tui_authoring_parity::every_authoring_action_matches_the_complete_cli_post_state",
            "tui_evidence_parity::every_task_local_evidence_action_matches_the_complete_cli_post_state",
            "tui_evidence_parity::verification_fix_matches_the_complete_cli_post_state",
            "mcp_parity::every_tool_agrees_with_the_command_it_mirrors",
        ],
    },
    // `A1-432`. The TUI command and Graph destination both predate the
    // explorer. An older build advertises both while rendering a task-list
    // placeholder with no semantic lanes, typed scope, hit identities or
    // viewport, so the existing names cannot distinguish this capability.
    Behaviour {
        name: "tui.graph_explorer_uses_semantic_scene",
        what: "the TUI graph explorer composes shared selectors with core reach, tree and critical-path scopes over the canonical typed GraphView, paints only visible ranges of the CLI's semantic scene, preserves exact omissions and identities, and supports non-colour selection, relationship following, hit testing, panning and recentering",
        proof: &[
            "graph_workspace_composes_every_scope_selector_and_membership_policy",
            "graph_workspace_keeps_selectors_exclusions_and_failures_typed",
            "graph_search_filters_and_combined_roles_are_semantic",
            "graph_explorer_uses_semantic_scene_end_to_end",
            "graph_viewport_is_operable_and_identity_safe_in_ascii_and_small_terminals",
            "manual_graph_pan_begins_at_the_followed_viewport",
            "graph_shapes::interactive_projection_matches_cli_json_and_text_for_shared_graph_shapes",
            "tui_journey::graph_explorer_runs_end_to_end_in_a_real_terminal",
        ],
    },
    // `A1-689`, found by the adversarial audit of the Layered text default.
    // The explorer already carried exact scope and grouping accounting, but
    // its context line printed neither. In particular, `unreachable` appeared
    // beside `m dep` even though that traversal is defined over containment.
    Behaviour {
        name: "tui.graph_context_discloses_scope_and_grouping_omissions",
        what: "the TUI graph context names the relation class its active structural scope actually walked and reports exact omitted cut-membership-edge and grouping-only-task counts at every supported width",
        proof: &["graph_context_discloses_scope_edges_and_grouping_omissions"],
    },
    // `A1-668`. The complete semantic scene and interactive explorer already
    // existed, but neither name distinguishes a build which can only refuse an
    // oversized scene from one which returns an explicit, fully accounted and
    // traversable semantic window.
    Behaviour {
        name: "tui.graph_windows_are_bounded_and_accounted",
        what: "the application projects selector/scope/membership results into deterministic \
               anchor-preserving node, edge and lane budgets; exact omissions, cyclic/empty/failed \
               states and paged incoming, outgoing and disconnected-component continuations stay \
               typed, stable and reachable without changing complete CLI graph output",
        proof: &[
            "bounded_graph_windows_account_for_every_fact_and_preserve_anchor_identity",
            "high_degree_continuations_page_without_repeating_or_losing_a_relationship",
            "every_five_node_dag_is_fully_reachable_through_bounded_continuations",
            "cycles_membership_and_disconnected_components_remain_typed_and_reachable",
            "ten_thousand_node_projection_has_bounded_output_and_deterministic_work",
        ],
    },
    // `A1-677`. The task-only graph projection and bounded window predate the
    // multiscale explorer. Older builds can therefore advertise both names
    // while having no phase bands, milestone aggregates, exact quotient-edge
    // provenance or reversible source-bound semantic transitions.
    Behaviour {
        name: "tui.graph_explorer_projects_multiscale_semantics",
        what: "the application projects plan, phase, milestone and task levels from one admitted canonical task graph; phase remains grouping, milestone edges retain every underlying dependency and quotient feedback stays distinct from invalid task cycles, while raw selection, exact accounting, bounded task continuations and reversible transitions remain source-bound",
        proof: &["graph_explorer_projects_multiscale_semantics"],
    },
    // `A1-668`, found by the post-delivery adversarial audit. Numeric sibling
    // ranges are meaningful only within the immutable projection which
    // emitted them; accepting one after refresh can silently retarget it to a
    // different relationship with the same ordinal position.
    Behaviour {
        name: "tui.graph_window_cursors_are_source_bound",
        what: "every graph continuation cursor carries the immutable input identity which emitted it, and a cursor presented to a refreshed or otherwise different projection is refused as invalid rather than reinterpreted by ordinal position",
        proof: &["continuation_cursors_refuse_a_different_input_identity"],
    },
    // `A1-669`. The bounded application answer already existed, while the
    // production TUI still attempted the complete scene first and displayed a
    // refusal paragraph when it exceeded the fixed width. The engine name
    // alone therefore cannot assert that an interactive client adopts it.
    Behaviour {
        name: "tui.graph_explorer_windows_oversized_inputs",
        what: "the TUI opens an oversized graph as an explicitly labelled, fully accounted semantic window; keyboard and pointer controls traverse and reverse source-bound continuations, relationship navigation recentres omitted identities, neighbourhood depth composes with structural scope, and resizing changes only presentation",
        proof: &[
            "oversized_graph_windows_are_traversable_reversible_and_source_bound",
            "graph_window_controls_preserve_scope_and_expose_membership_without_colour",
            "oversized_graph_renders_and_activates_semantic_window_controls",
            "tui_journey::graph_manual_pan_and_resize_run_end_to_end_in_a_real_terminal",
        ],
    },
    // `A1-433`. `tui`, `roadmap`, `summary` and the analysis commands already
    // existed, but a foundation-only TUI had no aggregate workspace. None of
    // those surface names tells an adopter whether report rows preserve CLI
    // values, selectors and provenance or merely render lookalike prose.
    Behaviour {
        name: "tui.report_hub_preserves_cli_report_semantics",
        what: "the TUI report hub retains typed roadmap, summary, version, paths, vacuity, stale \
               and deprecation values, states computation scope explicitly, and drills a row's \
               frozen contributors without recomputing the report",
        proof: &[
            "report_hub::report_hub_typed_values_match_cli_json",
            "report_hub_navigation_dates_and_drill_down_are_operable",
            "report_hub::report_hub_distinguishes_unavailable_invalid_and_zero_measurements",
            "report_hub_renders_scope_provenance_and_typed_states",
            "tui_journey::report_hub_runs_end_to_end_in_a_real_terminal",
        ],
    },
    // `A1-571`, found by the adversarial audit of `A1-433`. The typed report
    // values were correct, but the `/` action contradicted the design by
    // editing a Stale-only parameter from every report, and configured argv
    // was flattened into an ambiguous space-joined string. The existing hub
    // name cannot tell a staged consumer whether discovery and source identity
    // are literal.
    Behaviour {
        name: "tui.report_discovery_is_literal_and_parameter_safe",
        what: "the TUI searches report names and typed result rows without changing report scope, edits the stale as-of date through a distinct contextual action, and displays configured version-source argv without erasing argument boundaries",
        proof: &[
            "report_hub_navigation_dates_and_drill_down_are_operable",
            "report_hub_renders_scope_provenance_and_typed_states",
            "tui_journey::report_hub_runs_end_to_end_in_a_real_terminal",
        ],
    },
    // `A1-434`. A repository-backed report can change when HEAD or Git status
    // moves even if plan/config bytes do not. Naming only "refresh exists"
    // would let a build claim support while racing two repository revisions or
    // discarding the reader's stable navigation state.
    Behaviour {
        name: "tui.external_refresh_is_identity_safe",
        what: "the TUI identifies plan, configuration, HEAD and exact Git status inputs, probes them off the input thread, atomically rejects a refresh raced by a newer identity, and preserves identity-based navigation while the previous complete snapshot remains visible",
        proof: &[
            "repository_identity_distinguishes_head_and_dirty_path_transitions",
            "repository_identity_fails_closed_when_status_is_unavailable",
            "external_probe_refreshes_only_after_identity_moves",
            "identity_transition_messages_name_configuration_repository_and_load_state",
            "probe_failures_and_wrong_outputs_remain_explicit_without_retry_noise",
            "unloadable_and_racing_external_edits_keep_the_complete_snapshot",
            "snapshot_executor_probe_returns_only_the_identified_input",
            "refresh_executor_rejects_an_edit_that_races_snapshot_construction",
            "stale_completion_is_visible_and_cannot_replace_the_snapshot",
            "refresh_reconciles_inspector_state_with_the_new_snapshot",
            "tui_journey::external_plan_edit_refreshes_in_a_real_terminal",
            "tui_journey::focus_gain_probes_an_external_edit_in_a_real_terminal",
        ],
    },
    // `A1-573`, found by the second adversarial audit of `A1-434`. Merely
    // checking whether the terminal is active when work completes lets a
    // pre-loss probe or refresh certify a session that regained focus before
    // the old answer arrived.
    Behaviour {
        name: "tui.focus_epoch_invalidates_external_results",
        what: "a probe or refresh requested before terminal focus is lost cannot certify a later active focus epoch, and regaining focus obtains a current-epoch observation",
        proof: &[
            "focus_epoch_invalidates_pre_loss_probe_and_refresh_results",
            "tui_journey::focus_gain_probes_an_external_edit_in_a_real_terminal",
        ],
    },
    // `A1-577`, found by the adversarial audit of the read-only TUI increment.
    // Process suspension restored terminal modes but did not enter the focus
    // freshness boundary, so the first resumed frame could certify bytes that
    // changed while the process was stopped.
    Behaviour {
        name: "tui.suspension_invalidates_snapshot_freshness",
        what: "process suspension advances the external-observation epoch before stopping, resume paints retained bytes as unconfirmed and obtains a current-epoch observation as soon as worker capacity is available",
        proof: &[
            "focus_regain_retries_its_probe_as_soon_as_a_busy_worker_is_available",
            "tui_journey::suspend_restores_exact_termios_and_resume_repaints",
        ],
    },
    // `A1-578`, found while auditing the suspension freshness correction. A
    // continue arriving between screen teardown and the default stop handler
    // was consumed while the stale suspend result still stopped the process.
    Behaviour {
        name: "tui.continue_cancels_pending_suspend_handoff",
        what: "a continue observed after terminal teardown cancels a pending suspend decision before the default stop handler can strand the process, without changing quit or termination results",
        proof: &["continue_after_screen_exit_cancels_only_a_pending_suspend"],
    },
    // `A1-579`, found while auditing the two suspension corrections. SIGSTOP
    // cannot be handled before stopping, so its later continue was previously
    // treated as a repaint request without invalidating retained bytes.
    Behaviour {
        name: "tui.continue_establishes_freshness_boundary",
        what: "every continue that resumes interactive work establishes an external-observation boundary, including a standalone continue after an uncatchable stop and a continue that cancels the post-teardown suspend handoff",
        proof: &[
            "repeated_focus_loss_is_idempotent_until_focus_returns",
            "tui_journey::standalone_continue_after_sigstop_refreshes_the_snapshot",
        ],
    },
    // `A1-574`, found by the second adversarial audit of `A1-434`. Advancing
    // the periodic deadline while the sole worker was occupied postponed the
    // next probe and left an unobserved snapshot labelled current.
    Behaviour {
        name: "tui.overdue_external_probe_is_fail_closed",
        what: "a periodic identity probe that cannot start because the bounded worker is occupied makes snapshot freshness unconfirmed and remains due until capacity starts it",
        proof: &[
            "an_overdue_external_probe_waits_for_capacity_without_claiming_current",
            "tui_journey::overdue_probe_recovers_after_a_long_provider_in_a_real_terminal",
        ],
    },
    // `A1-634`, found when the P00024 pilot-log push exercised the complete
    // workspace gate. An idle quit could start an overdue identity probe before
    // observing the reducer's quit state, turning new work into exit latency.
    Behaviour {
        name: "tui.quit_starts_no_new_work",
        what: "after input requests quit, termination and suspension retain precedence while continue is inert; the TUI exits before draining a completion that could start queued work, starting a probe, drawing another frame or polling for more input",
        proof: &[
            "quit_does_not_start_an_overdue_external_probe",
            "quit_ignores_continue_without_starting_a_resume_probe",
            "quit_drops_queued_work_before_draining_a_completion",
            "quit_preserves_termination_and_suspension_precedence",
        ],
    },
    // `A1-575`, found by the second adversarial audit of `A1-434`. The
    // production probe previously used the refresh loader, constructing and
    // discarding a verification run sheet that its measurement omitted.
    Behaviour {
        name: "tui.production_external_probe_avoids_refresh_work",
        what: "periodic and focus-gain probes load the complete production input identity without constructing refresh-only verification or report projections, while refresh confirmation uses the same identity observation",
        proof: &[
            "snapshot_executor_probe_returns_only_the_identified_input",
            "the_tui_refresh_reloads_repository_inputs",
        ],
    },
    // `A1-542`, found by the adversarial audit of `A1-455`. The semantic
    // scene and both graph painters already existed, but a fan-in whose
    // destination lane was not the leftmost active lane extended its join to
    // column zero. That painted a false junction through unrelated routes and
    // exposed the same false edge identities to hit testing.
    Behaviour {
        name: "graph.merge_starts_at_destination_lane",
        what: "a fan-in merge begins at its destination lane, preserving unrelated routes to its \
               left as vertical cells with only their own edge identities",
        proof: &["merge_routes_do_not_extend_left_of_their_destination_lane"],
    },
    // `A1-456`. The `graph` command and `text` format both predate focused
    // roles, exact status labels, stream-resolved colour and projection of the
    // critical route. Their existing names cannot distinguish the austere lane
    // drawing from a build with this human presentation contract.
    Behaviour {
        name: "graph.text_focus_and_critical_overlay",
        what: "`graph --format text` retains the canonical semantic scene while adding exact \
               statuses, a focus with visible direct dependency neighbours, the deterministic \
               whole-plan critical route and exact edge cues through documented non-colour \
               badges and stream-resolved terminal styling; presentation never changes scope or \
               machine formats",
        proof: &[
            "critical_route_keeps_the_exact_milestone_steps_the_report_hides",
            "focused_graph_roles_statuses_and_critical_edges_are_non_colour_semantics",
            "graph_shapes::cli_text_graph_focus_and_critical_overlay_are_exact_and_pipe_stable",
            "graph_shapes::graph_focus_refuses_unknown_excluded_and_membership_only_tasks",
            "color::graph_text_obeys_every_colour_mode_without_changing_its_plain_bytes",
        ],
    },
    // `A1-565`, found by the adversarial audit of `A1-456`. The first
    // critical overlay counted matching scene edge records, so a public view
    // with repeated or differently typed equal endpoints could report two of
    // one exact route steps as visible. An omitted matching edge was counted
    // even though it had no connector cells.
    Behaviour {
        name: "graph.critical_visibility_counts_step_identities",
        what: "the CLI text graph counts visible critical-route step identities rather than matching scene edge records, so repeated equal endpoints count once and omitted edges count zero",
        proof: &["critical_route_visibility_counts_step_identities_not_scene_records"],
    },
    // `A1-194`. `summary` existed before it could choose an immutable input,
    // and command names in `facts` do not enumerate flags. An older build
    // answers only from a checkout and cannot be distinguished by the existing
    // command or format names.
    Behaviour {
        name: "summary.revision_selects_committed_inputs",
        what: "`summary --at` reads the plan and its discovered configuration from one resolved \
               commit without a checkout, while an explicit configuration remains selected",
        proof: &[
            "revision::summary_at_a_revision_reads_its_plan_and_discovered_view_config",
            "revision::an_explicit_config_remains_the_selected_policy_for_a_historical_plan",
        ],
    },
    // `A1-498`, found by the adversarial audit of `A1-194`. An explicit
    // historical root was first assigned to the nearest repository in today's
    // filesystem, so a nested marker created later could select another object
    // database. A root that was itself a gitlink was then described as absent,
    // as were a root link to the top tree and a broken root link (despite the
    // latter being invalid input rather than absence).
    Behaviour {
        name: "summary.revision_uses_historical_repository_boundary",
        what: "`summary --at` assigns an explicit in-repository root to the invocation's \
               historical tree, unaffected by nested repositories created later, and refuses a \
               plan-root gitlink or broken root link as incomplete input while accepting a root \
               link to the top tree; an external repository, including one reached through an \
               alias, requires the caller to select or decline current policy explicitly",
        proof: &[
            "revision::todays_nested_repository_cannot_capture_an_outer_historical_root",
            "revision::a_submodule_as_the_plan_root_is_refused_as_incomplete_input",
            "revision::a_committed_plan_root_symlink_can_resolve_to_the_repository_root",
            "revision::a_broken_committed_plan_root_symlink_is_named_as_invalid_input",
            "revision::an_external_historical_root_selects_its_repository_after_policy_is_explicit",
            "revision::an_external_repository_symlink_alias_remains_addressable",
            "revision::an_external_root_refuses_a_discovered_caller_policy_unless_selected_or_declined",
        ],
    },
    // `A1-496`, found by the adversarial audit of `A1-201`. Attestation
    // generation and note attachment used to resolve `HEAD` independently, so
    // a concurrent commit could receive a document about the preceding tree.
    Behaviour {
        name: "attest.note_targets_attested_commit",
        what: "an attached attestation is written to the exact commit recorded in its document, \
               even if HEAD advances before the note write",
        proof: &["a_note_targets_the_attested_commit_even_if_head_moves"],
    },
    // `A1-473`, found by the adversarial audit of `A1-185`. `url.check`
    // promised one request per URL but cached only ordinary exits, treated a
    // signal as a dead link, and let a later checker fault erase an earlier
    // cached answer. No rule, command, config key, format or evidence kind
    // changed; the existing verifier now gives a stable, narrower answer.
    Behaviour {
        name: "verify.url_checker_outcomes_are_stable",
        what: "each url's answered, refused or timed-out outcome is checked once and reused for \
               every citation; a checker crash is one configuration fault and cannot erase an \
               earlier answer",
        proof: &[
            "a_url_checker_fault_does_not_erase_an_earlier_cached_answer",
            "a_url_check_that_never_returns_is_abandoned_and_reported",
        ],
    },
    // `A1-470`, found by the adversarial audit of `A1-187`. Phase remained a
    // required reporting coordinate, but the selector and scoped policy kept
    // the four-dimensional shape written while that decision was pending.
    Behaviour {
        name: "selection.phase_coordinate",
        what: "the repeatable phase coordinate narrows every selecting command and may scope \
               policy through the same selector without changing graph or readiness semantics",
        proof: &[
            "selection::every_dimension_narrows_on_every_selecting_command",
            "scoped_policy::a_scope_can_select_a_phase",
        ],
    },
    // `A1-471`, also found by that audit. Checklist export already named phase
    // loss when several values existed, but silently discarded the one value in
    // the common single-phase case. The format name did not distinguish those
    // builds, so the stronger loss report needs a capability name.
    Behaviour {
        name: "export.checklist_names_single_phase_loss",
        what: "checklist export reports phase loss for every non-empty export and gives a \
               single-phase plan the exact import flag that preserves its value",
        proof: &["export::a_single_phase_is_named_as_export_loss"],
    },
    // `A1-186`. `roadmap.group_by` already refused an undeclared family, but
    // reported it as an invalid test runner. The same predicate now serves two
    // views, so carrying the view's key with the verdict prevents the shared
    // validation from naming the wrong part of the configuration.
    Behaviour {
        name: "config.grouping_error_names_its_view",
        what: "an undeclared family in `roadmap.group_by` or `summary.group_by` is refused as \
               invalid grouping and names the exact view key, never the test runner",
        proof: &["summary_group::an_undeclared_grouping_family_names_the_view_not_a_test_runner"],
    },
    // `A1-468`, found by the adversarial audit of `A1-186`. That task gave an
    // undeclared group its own error but left the vocabulary it depends on
    // returning `BadRunner`; a blank, empty or duplicate family still sent a
    // reader to a test block the file might not have.
    Behaviour {
        name: "config.tag_family_error_names_its_key",
        what: "a structurally invalid `tag_families` declaration is refused as that exact \
               configuration block, never as an invalid test runner",
        proof: &["summary_group::an_invalid_tag_family_is_not_reported_as_a_test_runner"],
    },
    // `A1-467`, the human-output finding from the same audit. The grouped JSON
    // already preserved the family and raw member. The terminal table flattened
    // distinct members to the same text, omitted the family, and padded by byte
    // length, so an older build can present a materially different answer under
    // the same command and format names.
    Behaviour {
        name: "summary.grouped_human_is_unambiguous",
        what: "a grouped human summary names its family, renders every member distinctly from \
               synthetic rows, and aligns values by terminal display width",
        proof: &[
            "summary_group::distinct_hostile_family_members_remain_distinguishable_in_human_summary",
        ],
    },
    // `A1-199`. `verify` already reported the two evidence failures this
    // applies, and `amend` already accepted the replacement. What changed is
    // that the diagnostic now carries the exact edit as data and `verify
    // --fix` can apply the batch. `--require verify`, `APLAN013` or `APLAN015`
    // all pass on a build that offers only prose, so the widening needs its own
    // name.
    Behaviour {
        name: "diagnostics.machine_applicable_repairs",
        what: "diagnostics carry exact evidence replacements as typed data, and `verify --fix` \
               applies all valid ones as one guarded mutation before verifying the result",
        proof: &[
            "amend::computed_repairs_are_reviewable_and_applied_as_one_guarded_batch",
            "amend::a_computed_repair_the_evidence_gate_refuses_writes_none_of_the_batch",
        ],
    },
    // `A1-458`, the audit of `A1-199`. One distinct suggested spelling is not
    // one resolvable test when two configured runners both list it. Older
    // builds publish an operation that their own evidence gate then refuses.
    Behaviour {
        name: "diagnostics.repair_requires_unique_resolver",
        what: "a typed test-name repair is published only when exactly one configured runner \
               resolves its replacement; a shared spelling remains prose advice",
        proof: &["amend::a_shared_near_match_is_not_published_as_a_typed_repair"],
    },
    // `A1-478`. `APLAN013` already refused an unresolved or ambiguous test
    // name, but its advice pointed only at the protocol key `test.list`. An
    // older build therefore passes `--require APLAN013` while sending the
    // reader to reconstruct configured runner execution by hand.
    Behaviour {
        name: "evidence.test_unresolved_points_to_catalogue",
        what: "an unresolved or ambiguous test citation points at `aplan test list`, which \
               exposes the resolver's exact configured names and runner identities",
        proof: &["test_catalogue::close_points_at_configured_test_discovery"],
    },
    // `A1-459`, the audit of `A1-199`. Configured evidence resolvers run before
    // the repair mutation and can change a sibling task. Older builds re-read
    // only repaired files, gated a stale plan, wrote, and noticed the sibling's
    // finding during the post-write verification.
    Behaviour {
        name: "verify_fix.post_runner_plan_is_gated",
        what: "`verify --fix` reloads the whole plan after configured runners finish, refuses \
               their newly invalid sibling state before writing, and preserves valid sibling edits",
        proof: &[
            "amend::a_computed_repair_gates_the_sibling_state_changed_by_its_runner",
            "amend::a_computed_repair_preserves_a_valid_sibling_edit_from_its_runner",
        ],
    },
    // `A1-460`, the audit of `A1-199`. The same external resolver can change
    // the selected policy that defines whether its proposed replacement
    // resolves. Older builds reloaded the config only in post-verification.
    Behaviour {
        name: "verify_fix.post_runner_config_is_gated",
        what: "`verify --fix` reloads the selected configuration after external resolvers finish \
               and validates every replacement under the policy its post-verification will use",
        proof: &["amend::a_computed_repair_reloads_configuration_changed_by_its_runner"],
    },
    // `A1-462`, the audit of `A1-199`. The ordinary mutation wrapper
    // discovers from the plan root, while `verify` discovers from `--repo`.
    // Calling both made one command use two policies when those trees differed.
    Behaviour {
        name: "verify_fix.one_post_runner_config_source",
        what: "`verify --fix` uses the post-run configuration discovered from its evidence \
               repository for replacement resolution, mutation gating, and post-verification",
        proof: &[
            "amend::a_computed_repair_keeps_the_repositorys_configuration_through_its_mutation_gate",
        ],
    },
    // `A1-240`, ADR-0022. A widening with no new command, no new flag and no new
    // rule -- the kind `--require` exists for, and the kind `p00020`'s item 23
    // predicted would stop being named: the derived `cut` changed meaning on a
    // plan with releases, and nothing about the surface says which answer a
    // build gives. An older binary reports the release that reaches a task and
    // reports it confidently, so a consumer pinning a vendored `aplan` cannot
    // tell the two apart by any name that already existed.
    Behaviour {
        name: "rollup.cut_excludes_releases",
        what: "a task's reported `cut` is the cut it is in, never a release that closes it",
        proof: &[
            "a_release_is_never_the_cut_a_task_is_in",
            "selection::a_release_is_refused_as_a_cut_on_every_command_that_selects",
        ],
    },
    // `A1-463`, the audit of `A1-199`. Replacement validation invokes the
    // configured lister again, so a snapshot taken after the original report
    // is still too early when that external command changes the repository.
    Behaviour {
        name: "verify_fix.revalidation_side_effects_are_gated",
        what: "`verify --fix` builds its candidate from the plan left by every replacement \
               resolver and refuses a configuration that changes during that validation",
        proof: &[
            "amend::a_computed_repair_gates_a_sibling_changed_during_revalidation",
            "amend::a_computed_repair_refuses_a_sibling_made_unloadable_during_revalidation",
            "amend::a_computed_repair_refuses_configuration_changed_during_revalidation",
        ],
    },
    // ADR-0015. The decision number stays in this comment: `what` is printed by
    // `aplan facts` into a consumer's terminal, where this project's ADRs do not
    // resolve (`A1-044`). Their own gate caught it here.
    Behaviour {
        name: "traces.table_rows",
        what: "a `traces` anchor may name a table row by its first cell",
        proof: &[
            "a_table_row_cell_follows_the_same_rule_as_a_heading",
            "traces::an_edit_to_a_traced_row_re_flags_exactly_its_owner",
        ],
    },
    // `A1-112`. The `advisory` config key is listed under `config_keys` and is not
    // repeated here. This is the separable half: the summary word and the reported
    // counts became the *verdict's* rather than the raw severities', which changes
    // what a run with `deny_warnings` and no `advisory` prints. A build predating
    // it exits 1 beneath a line reading `ok`, so a consumer reading the summary
    // cannot tell the two apart by any name that already existed.
    Behaviour {
        name: "verdict.summary_agrees_with_exit_code",
        what: "the `ok`/`failed` word and the reported counts are what blocked, not every finding",
        proof: &[
            "advisory::a_denied_warning_does_not_summarise_as_ok",
            "advisory::the_summary_agrees_with_the_exit_code_and_names_what_was_downgraded",
        ],
    },
    // `A1-125`. `ready` answered in id order and now answers in a derived rank
    // order, on both surfaces. No rule, command, config key, format or evidence
    // kind appeared: `--require ready` and `--require aplan_ready` pass on a build
    // that sorts alphabetically, and a consumer reading the first line as "what
    // to do next" cannot tell the two apart. `--explain` is a flag and `rank` is a
    // field, neither of which `facts` carries.
    //
    // Named one commit late, which is the direction AGENTS.md calls the safe one:
    // a build between the change and this name supports the behaviour and is
    // still refused. The rule says "the same commit", and this is the audit
    // finding rather than an exception to it.
    Behaviour {
        name: "ready.order_is_derived",
        what: "`ready` answers in a derived rank (what a task releases, then the critical path, \
               then the chain of work behind it, then effort) rather than in id order, and \
               `--explain` says which",
        proof: &[
            "ready_is_ordered_by_what_the_plan_records_not_by_id",
            "the_order_does_not_depend_on_how_the_plan_was_written",
            "machine_readable::the_explain_reasons_are_fields_not_a_sentence",
            "ready_is_ranked_here_too_and_carries_its_reason",
            "a_longer_chain_behind_a_task_ranks_it_higher",
        ],
    },
    // `A1-138` narrowed `APLAN019` to fire only where the task cites a `commit`,
    // `file` or `test`. No rule code, command, config key, format or evidence
    // kind appeared, so `--require APLAN019` passes on the build that reported
    // 140 findings across 71 `todo` tasks and on the build that reports none.
    //
    // The fourth pilot named this as the *third* instance of the same blind spot
    // -- row anchors, MCP close stamping, and this -- and mitigated it on its own
    // side by pinning the binary's digest, while noting that "naming a behaviour
    // when the behaviour changes is the only thing that keeps `--require`
    // meaningful, and it has been skipped more often than it has been done"
    // (pre-public adoption evidence, twelfth pass). It was skipped here too, by the
    // change that closed their blocker, in the session that audited another task
    // for the same omission. Named late, which under-approximates: a build
    // between `A1-138` and this has the behaviour and is still refused.
    Behaviour {
        name: "paths.unmatched_needs_evidence",
        what: "`APLAN019` reports a dead `paths` glob only where the task cites a `commit`, \
               `file` or `test`, so a plan that names trees a later cut creates is silent",
        proof: &[
            "aplan019_is_silent_when_nothing_binds_to_the_paths",
            "aplan019_is_silent_where_nothing_is_bound_to_the_paths",
        ],
    },
    // `A1-141`. The scope moved again, so a second name rather than a rewritten
    // one: `paths.unmatched_needs_evidence` stays true of this build, and a
    // consumer that pinned it is not refused by a build that also has this.
    //
    // What changed: a dead glob beside a live one the task's evidence binds
    // through is silent. `--require paths.unmatched_needs_evidence` passes on the
    // build that refused the fourth pilot's bound closure and on the build that
    // accepts it, which is the blind spot `A1-142` exists to gate rather than to
    // keep meeting (pre-public adoption evidence item 26).
    Behaviour {
        name: "paths.unmatched_ignores_a_bound_sibling",
        what: "`APLAN019` is silent about a dead `paths` glob when the task's cited commit or \
               file binds through another one, so a stale declaration does not refuse a close",
        proof: &[
            "aplan019_is_silent_when_the_evidence_binds_through_another_glob",
            "close_gate::a_dead_sibling_glob_does_not_refuse_a_bound_citation",
        ],
    },
    // `A1-491`. The rule already ignored vacuous evidence when deciding whether
    // it applied, but reconstructed its diagnostic subject with `trim`. An
    // invisible reference beside a substantive one was consequently named as
    // evidence even though the predicate had rejected it. The diagnostic now
    // uses the same canonical predicate as the rule scope.
    Behaviour {
        name: "paths.unmatched_ignores_vacuous_evidence",
        what: "`APLAN019` names only substantive commit, file and test references in its \
               diagnostic, using the same invisible-reference predicate that scopes the rule",
        proof: &["aplan019_names_what_is_unbound_and_says_it_as_english"],
    },
    // `A1-491`. A zero-width test reference counted as independent proof for
    // `APLAN024` because the resolver used `trim`, while the core validation
    // predicate correctly treated it as no citation. Evidence-presence checks
    // now share the predicate, so an invisible value cannot acquit a substantive
    // citation from an evidence-quality rule.
    Behaviour {
        name: "evidence.vacuous_test_does_not_acquit_dilution",
        what: "`APLAN024` does not accept a whitespace-only or invisible test reference as \
               independent proof that acquits diluted commit evidence",
        proof: &["a_vacuous_test_does_not_acquit_diluted_commit_evidence"],
    },
    // `A1-644`. The v1 batch protocol and `passed`/`failed`/`skipped` outcome
    // names already existed. Older builds could not preserve retry, flake,
    // cancellation and framework-terminal semantics, so a new command or
    // format name does not distinguish the widened existing protocol.
    Behaviour {
        name: "evidence.batch_outcomes_preserve_terminal_semantics",
        what: "the existing test-batch result protocol keeps expected failure, retry, flake, \
               cancellation, timeout, resource, framework and incomplete outcomes distinct, \
               and a retry-created green cannot be encoded as an unconditional pass",
        proof: &[
            "batch_retry_and_terminal_outcomes_remain_distinct",
            "batch_retry_and_terminal_outcomes_do_not_satisfy_l3",
        ],
    },
    // `A1-644`. `facts.commands` currently enumerates top-level clap commands,
    // so the new `test receipt` leaf needs its own capability name.
    Behaviour {
        name: "evidence.portable_provider_receipts",
        what: "`test receipt` exports one complete provider execution as a canonical, plan-independent statement whose current material and externally authenticated producer can be validated before it is used as evidence",
        proof: &[
            "test_catalogue::receipt_command_exports_a_complete_plan_independent_provider_statement",
            "one_canonical_statement_round_trips_without_a_plan_identity",
        ],
    },
    // `A1-645`. `verify` and test evidence already existed, while the new
    // current-only source changes what L3 can consume and what the same command
    // means. A top-level command or evidence-kind fact cannot distinguish it.
    Behaviour {
        name: "evidence.receipt_backed_whole_plan_resolution",
        what: "`verify --test-receipts` authenticates one complete current observation per configured provider, recomputes the exact current whole-plan L1/L2/L3 verdict, and never starts a test provider or silently falls back to direct execution",
        proof: &[
            "test_catalogue::verify_imports_current_receipts_without_starting_the_provider",
            "receipt_backed_resolution_starts_no_provider_and_fans_out_current_results",
            "rejected_receipt_rows_are_typed_findings_without_fallback",
            "receipt_and_direct_no_match_sentinel_have_diagnostic_parity",
        ],
    },
    // `A1-645`. Verification output already existed on four surfaces, so the
    // new independent source/scope/completeness dimensions are a semantic
    // widening rather than a new format or command.
    Behaviour {
        name: "evidence.verification_reports_provider_provenance",
        what: "human, JSON, MCP and TUI verification results identify observation scope, provider source, freshness and completeness without conflating receipt, direct and deferred work",
        proof: &[
            "test_catalogue::verify_imports_current_receipts_without_starting_the_provider",
            "verify_says_which_kinds_nothing_is_configured_to_check",
            "verification_run_sheet_and_results_keep_every_state_explicit",
        ],
    },
    // `A1-646`. `verify`, provider receipts and the batch protocol already
    // existed. This mode changes how the same whole-plan verification command
    // can safely refresh those observations, so none of their existing facts
    // distinguish a build that still has to execute every provider identity.
    Behaviour {
        name: "evidence.incremental_receipt_refresh",
        what: "`verify --refresh-test-receipts` derives invalidation from an explicit Git boundary, executes only identities covered by an exhaustive input-dependency declaration, widens every ambiguous case, atomically publishes a complete current receipt set, and re-derives the whole-plan verdict",
        proof: &[
            "test_catalogue::incremental_refresh_executes_only_invalidated_tests_and_rederives_the_whole_plan",
        ],
    },
    // `A1-105`. `ready` printed nothing at a cut boundary whose members were all
    // `done`, and now names the cut as closeable on both surfaces. `--require
    // ready` and `--require aplan_ready` pass on the build that went silent, and
    // silence is the one answer an agent cannot tell from "there is no work":
    // that is how the fourth pilot spent a boundary stranded (item 21).
    //
    // Found by `A1-142`'s own audit, together with the entry below. Both are
    // *commands*, and the three reminders `A1-142` placed covered rules, MCP
    // tools and the scenario corpus -- the same blind spot one level up. The
    // command surface carries one now.
    Behaviour {
        name: "ready.reports_a_closeable_cut",
        what: "`ready` names a cut whose members are all done rather than answering with silence",
        proof: &[
            "a_cut_whose_members_are_done_is_closeable",
            "a_closeable_cut_says_how_much_closing_it_would_free",
        ],
    },
    // `A1-113`. Every mutating command and tool now runs the rules over the plan
    // it is about to write and refuses a mutation that makes `check` worse, with
    // `--no-resolve` the only opt-out. No command, tool, config key, format or
    // evidence kind appeared: `--require add` and `--require aplan_close` pass on
    // the build that writes the broken plan and on the build that refuses it, and
    // the failure is silent in the worst direction -- an older binary accepts the
    // input and does something broader with it, which is the case the paragraph
    // at the top of this file calls a blocker.
    Behaviour {
        name: "mutation.refused_when_check_would_reject",
        what: "a command or tool that writes runs the rules against the plan it would produce, \
               and refuses a mutation that introduces a finding",
        proof: &[
            "mutation_gate::commands_other_than_add_refuse_a_post_state_check_would_reject",
            "tag_families::add_refuses_before_writing_what_check_would_reject_after",
        ],
    },
    // `A1-420`. `dep` read the target immediately before its guarded write,
    // but derived the new relationship list from the older plan snapshot. An
    // intervening valid edge was therefore present in the compare-and-swap's
    // expected bytes and still disappeared from the bytes it published.
    Behaviour {
        name: "dep.rebases_on_current_target",
        what: "a dependency edit derives its relationship list and mutation verdict from the \
               current task bytes, preserving a valid edge added after its read snapshot",
        proof: &["a_dependency_edit_preserves_an_intervening_valid_edge"],
    },
    // `A1-655`. The command already kept a flow sequence inline, but every
    // cardinality change re-emitted its members and rebuilt a block sequence in
    // canonical indentation. That made an edge edit rewrite quotes, comments
    // and every retained importer-style line without changing their meaning.
    Behaviour {
        name: "dep.preserves_scalar_sequence_source_deltas",
        what: "dependency and preference edge edits retain the exact source of surviving scalar \
               sequence members and surrounding trivia, and refuse a structure they cannot edit \
               unambiguously instead of rebuilding it",
        proof: &["dep::dependency_edits_preserve_source_sequence_members_and_comments"],
    },
    // `A1-715`. Tags shared the same scalar-list writer but selected its
    // replacement mode, so ordinary add/remove rebuilt retained members and a
    // structural sequence could reach the round-trip guard only after an
    // anchor had been orphaned. The plan input now receives the refusal rather
    // than an internal error, and plan-wide rename preserves each file's own
    // source form.
    Behaviour {
        name: "tag.preserves_scalar_sequence_source_deltas",
        what: "tag add, remove and plan-wide rename retain the exact source of surviving scalar \
               sequence members and surrounding trivia, and refuse a structural sequence they \
               cannot edit unambiguously without changing another field",
        proof: &["tag_cmd::tag_edits_preserve_source_sequences_and_refuse_structural_aliases"],
    },
    // `A1-537`, found by the adversarial audit of `A1-420`. The application
    // context accepted a parsed plan and then reopened its files for the input
    // digest. An edit in between paired an old result with a new digest, while
    // Markdown that was not yet a task was absent from the identity entirely.
    Behaviour {
        name: "application.input_identity_matches_loaded_inputs",
        what: "an application result's input identity is derived from the exact candidate bytes \
               parsed for that result, including Markdown candidates that are not yet tasks",
        proof: &[
            "an_identified_load_keeps_every_candidate_and_the_bytes_it_parsed",
            "a_result_identity_names_the_bytes_that_produced_it",
            "a_non_task_markdown_candidate_is_part_of_the_input_identity",
        ],
    },
    // `A1-066`. `verify` used to stop on a defect it met in one task's input or
    // in one configured command -- a `paths` glob that would not compile, a test
    // runner that would not list, a `test.run` that would not launch under
    // `--execute`, a `url.check` naming a program that is not installed -- and
    // report nothing about anything else, exiting `2`. Each is a finding now
    // (`APLAN032`, `APLAN013`, `APLAN016`) and the rest of the plan is judged.
    //
    // The boundary is deliberate and stops here: a repository that cannot be
    // opened or read still ends the run, because nothing can be resolved against
    // it and every silence would then be meaningless.
    //
    // `APLAN032` is a new rule and assertable on its own; this name is here for
    // the half that is not. `--require verify` and `--require APLAN013` pass on
    // the build that goes quiet about every other task, and a consumer whose CI
    // reads "0 findings" from an aborted run cannot tell the two apart by any
    // name that already existed.
    Behaviour {
        name: "verify.one_tasks_defect_does_not_stop_the_run",
        what: "a defect in one task's input, or in one configured command, is a finding rather \
               than a reason to abandon the run and report nothing about the rest",
        proof: &[
            "ladder::a_tasks_bad_glob_does_not_hide_another_tasks_finding",
            "a_runner_that_lists_nothing_is_reported_against_the_configuration",
        ],
    },
    // `A1-121`. Every configured command this tool spawns is now bounded:
    // `test.list` at 600 seconds, `url.check` at 30, both overridable. Before
    // it, a command that never returned made `verify` wait for it, which in CI
    // is a job that hangs rather than fails.
    //
    // `test.timeout_seconds` and `url.timeout_seconds` are new config keys and
    // assertable on their own, but the *default* moved for every repository that
    // sets neither, which is the half no key proves: `--require verify` passes on
    // the build that waits forever and on the build that gives up and reports.
    // Named for that half, and named because the audit asked (`A1-142`).
    Behaviour {
        name: "verify.configured_commands_are_bounded",
        what: "a configured `test.list` or `url.check` that does not return is killed at its \
               limit and reported, rather than waited for",
        proof: &[
            "a_test_list_that_never_returns_is_abandoned_and_reported",
            "a_url_check_that_never_returns_is_abandoned_and_reported",
        ],
    },
    // `A1-592`. Configured commands already led their own process groups and
    // the TUI knew how to stop registered groups, but every non-interactive
    // command left the terminal-delivered signal attached only to its parent.
    // A build could therefore advertise bounded configured commands while an
    // interrupted `show`, `verify` or mutation orphaned the runner.
    Behaviour {
        name: "cli.configured_commands_follow_termination_signals",
        what: "on Unix, every non-interactive CLI path closes configured-command startup, stops all active process groups and preserves SIGINT, SIGTERM, SIGHUP or SIGQUIT as the caller-visible outcome",
        proof: &[
            "termination_cancels_active_and_future_configured_commands",
            "cli_termination::noninteractive_signals_stop_configured_groups_and_preserve_the_signal",
        ],
    },
    // `A1-150`, ADR-0025. `ready` gained an input no earlier build had: a task's
    // declared preference for following another. A plan that declares none ranks
    // exactly as before, so the field is invisible until used -- which is also
    // why the name is needed. `prefer_after` is a task field rather than a config
    // key or a command, so nothing in `facts` changes shape, and `--require
    // ready` and `--require ready.order_is_derived` both pass on a build that
    // ignores the field while a consumer's plan quietly declares one.
    Behaviour {
        name: "ready.honours_a_declared_preference",
        what: "`ready` offers a task after the offered tasks its `prefer_after` names, without \
               ever blocking it",
        proof: &[
            "a_preference_orders_the_offer_without_blocking_it",
            "dep::a_preference_orders_the_offer_and_blocks_nothing",
        ],
    },
    // `A1-134`. `APLAN021` asked whether a milestone's `blocked_by` was empty
    // and now asks whether it holds any live work. No rule code, command, config
    // key, format or evidence kind appeared: `--require APLAN021` passes on the
    // build that was silent about a cut emptied of work and on the build that
    // reports it, and the plans that exhibit it are exactly the ones using
    // ADR-0013's cut sequencing, whose `blocked_by` is never empty.
    Behaviour {
        name: "milestone.members_are_live_work",
        what: "`APLAN021` counts a cut's members as live work, so a cut left holding only other \
               cuts, or only cancelled tasks, is reported rather than silent",
        proof: &[
            "a_cut_emptied_of_work_is_reported_and_says_which_case_it_is",
            "mutate::a_cut_emptied_of_work_is_reported_rather_than_refused",
        ],
    },
    // `A1-139`. Every command that takes a `--cut` now accepts the milestone id
    // *or* the label the cut reports itself under, which only `graph --cut` did.
    // No command, config key, format or evidence kind appeared: `--require add`
    // and `--require mv` pass on the build that refuses `--cut s1` with a message
    // about an id the reader never typed, and the surface an adopter reads first
    // (`roadmap`) prints the label.
    Behaviour {
        name: "cut.named_by_id_or_label",
        what: "a `--cut` argument may be the milestone id or the label the cut reports itself \
               under, on every command that takes one",
        proof: &[
            "mutate::every_cut_taking_command_accepts_the_label",
            "cut::a_cut_is_named_by_its_id_or_by_its_label",
        ],
    },
    // `A1-251`. `supersede` gained `--tag`, which is assertable on its own, so
    // the flag is not what is named here. What a name is owed is the *default*:
    // a successor now inherits the predecessor's tags, minus the words that
    // place a task in a runway, and its `--cut` is part of the plan the mutation
    // gate judges rather than something applied afterwards. Neither is visible
    // to `--require supersede`, which passes on the build where the command
    // exists and refuses every repair in a plan with a mandatory family.
    Behaviour {
        name: "supersede.successor_is_placed_and_classified",
        what: "a successor inherits the predecessor's subject tags and is judged in the cut \
               `--cut` names, so `supersede` itself can repair a plan that requires a tag \
               family and a cut membership",
        proof: &[
            "supersede::a_successor_satisfies_a_mandatory_family_and_the_cut_it_is_given",
            "supersede::an_explicit_tag_replaces_what_would_have_been_inherited",
            "supersede::a_runway_role_is_not_inherited_but_the_subject_is",
            "supersede::cross_cutting_is_not_inherited_because_it_is_a_waiver",
        ],
    },
    // `A1-343`. `supersede` already inherited paths, traces and a generated
    // body. The new flags are parse-visible, but omission versus explicit empty
    // changes what the existing command means and therefore needs a name.
    Behaviour {
        name: "supersede.controls_inherited_successor_content",
        what: "`supersede` inherits paths, traces and its generated body only when their controls are omitted; explicit replacements, clears and a supplied body publish atomically",
        proof: &[
            "supersede::successor_scope_and_body_can_be_replaced_or_cleared_atomically",
            "add_and_supersede_forms_decode_every_new_creation_control",
        ],
    },
    // `A1-357`. `--tag` is a flag, and a flag is not advertised in `facts`, so a
    // consumer cannot tell the build that takes it from the build that does not:
    // `--require import` passes on both, and on the older one every import into a
    // plan with a declared vocabulary fails `APLAN030` with no way to prevent it.
    // The notice half is the same story -- what a run *says* is not a rule, a
    // command or a format.
    Behaviour {
        name: "import.carries_the_plans_vocabulary",
        what: "`import` gives every task it writes the tags the invocation names, and says \
               which declared `tag_families` member the import would otherwise leave \
               missing, so a plan with a mandatory vocabulary can be imported into",
        proof: &[
            "import_vocabulary::an_import_names_the_family_it_cannot_satisfy_and_takes_the_answer",
            "import_vocabulary::a_plan_with_no_family_is_told_nothing_new",
            "import_vocabulary::the_tag_reaches_every_imported_task_and_is_written_once",
        ],
    },
    // `A1-146`, ADR-0022. A milestone tagged `release` is a checkpoint over
    // cuts: `APLAN021` stops calling it an emptied cut, and `roadmap` rolls up
    // the work of the cuts it closes instead of reporting `0/0`. No rule code,
    // command, config key, format or evidence kind appeared -- the marker is a
    // tag -- so `--require APLAN021` passes on the build that warns about every
    // release, and a plan carrying releases renders differently on the two.
    Behaviour {
        name: "milestone.release_holds_cuts",
        what: "a milestone tagged `release` holds the cuts it closes: `APLAN021` accepts it and \
               `roadmap` rolls up their work rather than reporting none",
        proof: &[
            "a_release_holds_cuts_and_an_emptied_cut_still_reports",
            "a_release_borrows_the_effort_of_its_cuts",
        ],
    },
    // `A1-147`, ADR-0023. `effort_days_per_week` is a new config key and
    // assertable on its own, so the *duration* half is not listed here. This is
    // the half no key proves: both rollups now report the effort behind their
    // counts whether or not a throughput is declared -- a cut's own effort on
    // every `roadmap` surface, a phase's done effort in `summary --format json`
    // -- and a consumer reading the markdown table by column position, or
    // summing the JSON, cannot tell the build that carries it from the build
    // that does not. `--require roadmap` and `--require summary` pass on both.
    //
    // Renamed by this task's own audit, before any release carried it: the first
    // name said `roadmap`, and `summary` had quietly grown the same half. One
    // change earns one name (this file's rule), so the name widened rather than
    // gaining a second entry beside it.
    Behaviour {
        name: "rollup.carries_effort",
        what: "`roadmap` and `summary` report the effort behind their counts, and divide it into \
               weeks where `effort_days_per_week` is declared",
        proof: &[
            "a_cut_carries_the_effort_of_its_live_members",
            "a_derived_duration_is_on_every_surface",
        ],
    },
    // `A1-155`, pre-public adoption evidence. `--config` and `--no-config` are flags,
    // and `facts` carries commands and config keys rather than flags, so nothing
    // in it changes shape: `--require check` and `--require verify` pass on the
    // build where a reviewed checkout still selects the programs that run. A
    // consumer wiring a trusted-binary review needs to assert the boundary
    // exists before relying on it, and this is the name for that.
    Behaviour {
        name: "config.explicitly_selectable",
        what: "`--config <file>` reads exactly that file and discovers none, `--no-config` reads \
               none at all, and a named file that is missing is an error rather than a fallback",
        proof: &[
            "trusted_config::a_named_config_is_the_only_one_consulted",
            "trusted_config::no_config_runs_nothing_and_says_so",
            "trusted_config::a_named_config_that_is_missing_is_an_error_not_a_fallback",
        ],
    },
    // `A1-237`. `APLAN034` already existed, but a selected config outside the
    // tree silently changed what it called "this repository" to the config's
    // directory. `--require APLAN034` and `--require config.explicitly_selectable`
    // both pass on that build, so neither lets a review runner assert the fixed
    // trust boundary before depending on it.
    Behaviour {
        name: "traces.boundary_is_plan_repository",
        what: "`APLAN034` judges a trace against the plan's repository even when `--config` \
               selects policy elsewhere; config-relative values and diagnostic locations stay \
               with the selected file",
        proof: &[
            "a_selected_config_does_not_become_the_trace_repository",
            "trusted_config::an_external_selected_config_keeps_in_repository_traces_internal",
            "trusted_config::a_nested_plan_root_keeps_the_whole_git_repository_inside_the_trace_boundary",
            "trusted_config::an_external_trace_still_needs_a_config_relative_declaration",
            "trusted_config::the_mcp_surface_keeps_in_repository_traces_inside_the_plan_repository",
            "trusted_config::selected_config_diagnostics_name_the_file_that_was_read",
        ],
    },
    // `A1-109` widened `close` on both surfaces: proposed evidence is resolved
    // before either writes a `done` claim. The command and the tool already had
    // their names, so `--require close` and `--require aplan_close` pass on a
    // build with the silent failure that widening repaired, and a version string
    // is not a capability. This is the case `A1-099` made a general rule, met by
    // the change that most needed it (`A1-132`).
    Behaviour {
        name: "close.evidence_resolves_before_write",
        what: "`close` resolves a proposed citation before it writes, on both surfaces; only \
               `--no-resolve` opts out, and it says what it skipped",
        proof: &[
            "mcp_close_claims::one_unresolvable_citation_refuses_the_whole_closure_on_both_surfaces",
            "close_gate::no_resolve_records_it_and_says_that_it_did",
        ],
    },
    // `A1-705`. `close` already existed and resolved configured test names at
    // L2, but two adopter tasks reached `done` while the cited test was red.
    // The new flag changes the existing command's trust and proof semantics,
    // so the command name alone cannot let a consumer require the widening.
    Behaviour {
        name: "close.focused_l3_is_explicit_scoped_and_stale_safe",
        what: "`close --execute` explicitly runs only the proposed closure's distinct test \
               identities, requires typed L3 passes, and publishes the reviewed task only while \
               plan, repository, configuration and selected provider material remain current; \
               ordinary close and agent tools remain L2-only",
        proof: &["close_gate::focused_l3_runs_only_the_proposed_closures_tests"],
    },
    // `A1-708`. Focused L3 already refused stale and failed publication, but
    // preview-preparation and writer-error exits omitted the external-effect
    // warning. The existing command therefore had terminal outcomes which
    // made the same explicit execution boundary mean something weaker.
    Behaviour {
        name: "close.focused_l3_terminal_outcomes_name_external_effects",
        what: "every focused L3 outcome after configured test code starts distinguishes aplan's \
               guarded plan publication from command effects it cannot roll back, on CLI and \
               interactive surfaces",
        proof: &[
            "close_gate::focused_l3_preview_error_still_names_the_runner_effect_boundary",
            "focused_close_execution_is_explicit_typed_and_preview_bounded",
        ],
    },
    // `A1-358`. A live task with one valid fingerprint mapping is an accepted
    // input, but `close` used to append a fresh mapping and let its round-trip
    // guard discover the duplicate. The guard correctly prevented a write, yet
    // reported an internal bug and left no command-owned way forward. No rule,
    // command, flag, format or evidence kind appeared: the existing command now
    // supports an input it previously failed on, so the widening needs a name.
    Behaviour {
        name: "close.replaces_live_trace_fingerprints",
        what: "`close` replaces an accepted live task's existing trace fingerprint mapping in \
               place with freshly resolved stamps instead of appending a duplicate key",
        proof: &[
            "close_replaces_a_live_tasks_existing_fingerprints_with_fresh_stamps",
            "close_gate::close_replaces_a_live_fingerprint_block_end_to_end",
        ],
    },
    // `A1-716`. Fingerprint replacement already round-tripped its result, but
    // deserialisation had erased whether the source mapping carried an anchor,
    // alias or tag. Reconstructing one half of a shared mapping then orphaned
    // the other half and reported a plan-supplied source form as an internal
    // writer defect. The commands already existed, so the refusal boundary is
    // invisible without a behaviour name.
    Behaviour {
        name: "close.refuses_structural_fingerprint_mappings",
        what: "close, restamp and evidence repair replace ordinary block or one-line flow \
               fingerprint maps, while structural mappings refuse actionably before any task \
               bytes are written",
        proof: &["close_gate::fingerprint_rewrites_refuse_structural_aliases_without_writing"],
    },
    // `A1-717`. Citation edits reconstructed the complete evidence block after
    // deserialisation had erased anchors, aliases and explicit tags. Removing
    // a nested anchor still used by another field made an accepted task fail
    // the writer's own round-trip check. The commands already existed, so this
    // source-locality boundary needs its own assertable name.
    Behaviour {
        name: "close.refuses_structural_evidence_sequences",
        what: "explicit and computed citation edits replace only an ordinary source-local block \
               sequence of citation mappings; structural evidence refuses actionably before any \
               task bytes or batch prefix are written",
        proof: &[
            "close_gate::evidence_rewrites_refuse_structural_aliases_without_writing",
            "amend::computed_repairs_refuse_structural_evidence_as_one_unwritten_batch",
        ],
    },
    // `A1-717`, audit. The core had correctly defined citation identity as
    // `kind:ref`, but the CLI separately derived the proposed evidence vector
    // with full-record equality. A dated source citation therefore looked
    // unchanged and the newly written replacement skipped resolution.
    Behaviour {
        name: "amend.policy_reads_rewritten_evidence_candidate",
        what: "evidence amendment derives resolution, mutation-policy and freshness inputs by \
               parsing the exact candidate produced by the core rewrite, so captured metadata \
               cannot make a newly written citation look like an empty delta",
        proof: &["amend::a_dated_note_citation_can_be_removed_and_replaced"],
    },
    // `A1-722`, A1-717 audit. The structural recogniser made reconstruction
    // safe but still canonicalised every accepted citation mapping, deleting
    // comments and source style outside the one semantic edit. The narrower
    // write boundary is invisible through the existing command name.
    Behaviour {
        name: "amend.preserves_evidence_sequence_source_deltas",
        what: "evidence add, remove and replace retain every unaddressed ordinary citation byte, \
               including mapping indentation, key spacing, quote style, comments and newline \
               convention, while structural sequences keep their exact no-write refusal",
        proof: &["amend::ordinary_evidence_amendments_are_exact_source_deltas"],
    },
    // `A1-719`. The ordinary source path introduced beside that refusal still
    // rebuilt the mapping's top-level key line, so an accepted comment and
    // literal colon spacing disappeared even though neither was a derived
    // baseline. No command or format is new; the existing replacement has a
    // narrower write boundary that consumers need to distinguish.
    Behaviour {
        name: "close.preserves_fingerprint_mapping_boundary_trivia",
        what: "fingerprint replacement preserves an accepted ordinary mapping's literal key, \
               colon spacing and boundary comment, retaining the comment standalone when the \
               final derived baseline is cleared",
        proof: &["close_gate::fingerprint_rewrites_preserve_mapping_boundary_trivia"],
    },
    // `A1-720`. Portfolio extraction retained the typed rewrite source inside
    // the application crate but the CLI's `?` conversion flattened it before
    // exit routing. The message called the failure internal while the process
    // returned usage, so a staged consumer needs an explicit corrected build.
    Behaviour {
        name: "portfolio.preserves_rewrite_error_category",
        what: "park and unpark preserve typed rewrite failures through the CLI adapter, so plan \
               input refusals exit as usage and a failed writer self-check exits as aplan's \
               published internal error",
        proof: &["the_portfolio_adapter_preserves_every_application_error_category"],
    },
    // `A1-721`. Optional scalar clear treated every non-literal key spelling as
    // absence. A quoted revisit key therefore survived `unpark`, and only the
    // writer's read-back noticed after the status candidate had been built.
    Behaviour {
        name: "portfolio.refuses_alternate_optional_scalar_keys",
        what: "park and unpark edit ordinary revisit scalar lines exactly, while quoted or \
               otherwise non-local optional-scalar keys refuse actionably before any target in \
               the portfolio batch is written",
        proof: &[
            "mutate::unpark_refuses_alternate_revisit_keys_atomically_and_clears_ordinary_source_exactly",
        ],
    },
    // `A1-723`. Initial close decided that evidence was absent by searching
    // only for a literal `evidence:` source line. A quoted or explicit YAML key
    // loaded as the same field, then received a duplicate ordinary key and
    // reached the writer's internal read-back failure.
    Behaviour {
        name: "close.refuses_alternate_evidence_keys",
        what: "initial close treats every top-level key the YAML mapping resolves to `evidence` \
               as an existing field, refusing a live task actionably and without constructing or \
               writing a duplicate-key candidate",
        proof: &["close_gate::initial_close_refuses_alternate_evidence_keys_without_writing"],
    },
    // `A1-724`. A null release record deserialises to `None`, but its authored
    // key still owns that source location. Initial close previously appended a
    // second key when it derived a version and reached its read-back guard.
    Behaviour {
        name: "close.refuses_empty_version_fields",
        what: "release close distinguishes an absent version field from every ordinary or \
               alternate top-level key carrying a null value, refusing the present empty field \
               actionably and without constructing or writing a duplicate-key candidate",
        proof: &["version_record::empty_version_fields_are_refused_without_writing"],
    },
    // `A1-725`. The line-local scalar writers previously compared only the
    // token on the key's first physical line. YAML folds a following indented
    // line into a plain scalar, so a valid id or optional value could reach the
    // read-back guard only after the editor had built a different value.
    Behaviour {
        name: "mutation.refuses_multiline_plain_scalar_rewrites",
        what: "line-local task scalar edits compare the complete parsed field with the inline \
               source token, refusing multiline plain ids and optional values actionably before \
               any task or batch bytes are written",
        proof: &[
            "rekey::a_multiline_plain_source_id_refuses_without_writing",
            "mutate::portfolio_refuses_multiline_plain_revisit_values_atomically",
        ],
    },
    // `A1-196`, adversarial audit. `close --restamp` used to report success
    // with `detect_drift: false` while replacing every trace and evidence
    // baseline with an empty mapping. No command, flag, rule or config key is
    // new: the existing operation now refuses the state in which its promise is
    // impossible, so consumers need a name that distinguishes the safe answer.
    Behaviour {
        name: "close.restamp_requires_drift_detection",
        what: "`close --restamp` refuses while `detect_drift` is disabled instead of claiming \
               success while deleting trace and evidence baselines it did not refresh",
        proof: &["close_gate::close_restamp_refreshes_evidence_fingerprints_end_to_end"],
    },
    // `A1-582`, after the same wording caused failed repair round trips in two
    // pilots. The command used to report every baseline it examined as
    // "stamped", even when none changed, and replaced the task file on that
    // no-op. No command, flag, rule or format is new; the existing operation's
    // report and write boundary changed, so that contract needs its own name.
    Behaviour {
        name: "close.restamp_reports_baseline_changes",
        what: "`close --restamp` separately reports changed and already-current trace and \
               committed file-evidence baselines, does not replace the task file when every \
               baseline in its current task snapshot is current, and refuses a task changed \
               since its plan load rather than misreporting a no-op",
        proof: &[
            "close_gate::restamp_reports_exact_changes_and_does_not_write_an_already_current_task",
            "a_no_op_restamp_refuses_a_task_changed_since_plan_load",
        ],
    },
    // `A1-408`, found while auditing `A1-358`. YAML trivia does not terminate
    // an indented mapping, but the surgical block editor treated a blank line
    // or column-zero comment as the next top-level value. A later old stamp was
    // left behind, so the round-trip guard refused an otherwise valid input.
    // The existing replacement name predates and cannot assert this widening.
    Behaviour {
        name: "mutation.block_replacements_cross_yaml_trivia",
        what: "surgical mapping and sequence replacements cross blank lines and comments without \
               leaving an old entry behind or moving the trivia and following field",
        proof: &[
            "block_replacements_cross_yaml_trivia",
            "close_replaces_every_old_fingerprint_across_yaml_trivia",
            "close_gate::close_replaces_a_live_fingerprint_block_end_to_end",
        ],
    },
    // `A1-151`'s audit. `paths` is a command, so `--require paths` passes on the
    // build that shipped it -- including on a repository with no commits, where
    // it printed a full table of `0%` and a median of `0%` for a history it had
    // not read. That is the same name meaning two different things, which is
    // what this list is for. A consumer scripting on the JSON needs to know
    // `median_percent` can be absent and that `sampled` is what says so.
    Behaviour {
        name: "paths.no_sample_is_not_a_zero",
        what: "`aplan paths` reports that nothing was sampled rather than a share of 0%, and a \
               task whose `paths` do not compile is left unmeasured rather than counted as zero",
        proof: &[
            "paths::a_repository_with_no_history_reports_no_measurement",
            "paths::a_path_that_does_not_compile_is_not_reported_as_zero",
        ],
    },
    // `A1-253`. `verify` gained no rule, command, config key, format or evidence
    // kind: what changed is what an existing name *reports*. A resolver joins the
    // run only when `.aplan.yml` declares it, so a `test` citation under no runner
    // was recorded and never opened, and the run said `evidence resolved` over it
    // with a JSON document carrying no field that told the two apart. An older
    // binary answers the same question with the same words and a different
    // meaning, which is precisely the widening `--require` exists for.
    Behaviour {
        name: "verify.reports_what_nothing_checked",
        what: "`verify` reports the citations its configuration gave it no resolver for, \
               rather than counting them as resolved",
        proof: &[
            "unopened::the_summary_says_how_many_citations_nothing_opened",
            "unopened::the_json_report_carries_the_count_in_both_directions",
            "verify_says_which_kinds_nothing_is_configured_to_check",
        ],
    },
    // `A1-241`. A flag, which `facts` does not carry, on a command it already
    // lists: `--require add` passes on a build that can only add one task at a
    // time, and a consumer generating a plan cannot tell the two apart. The
    // precedent is `config.explicitly_selectable`, which named `--config` and
    // `--no-config` for the same reason.
    Behaviour {
        name: "add.reads_a_task_list",
        what: "`aplan add --from <file>` adds every task a file names as one change, each entry \
               resolved against the ones before it",
        proof: &[
            "add_from::a_later_entry_sees_what_an_earlier_one_created",
            "add_from::a_batch_that_cannot_finish_writes_none_of_it",
        ],
    },
    // `A1-343`. The command and manifest already existed, so accepting the
    // model's live state, scheduling, impact and relationship fields is not
    // assertable from either surface name alone.
    Behaviour {
        name: "add.accepts_creation_time_model_fields",
        what: "single and manifest task creation author live state, scheduling, version impact and every declared relationship in one validated write, including relationships to later manifest entries",
        proof: &[
            "add::single_add_authors_live_state_and_every_creation_relationship",
            "add_from::manifest_creation_fields_and_terminal_ledger_are_atomic",
            "add_tool_preserves_every_live_creation_time_model_field",
            "add_and_supersede_forms_decode_every_new_creation_control",
        ],
    },
    // `A1-556`. The command names were already public, but neither repeated
    // existing-task rewrites nor reverse creation edges could be asserted by a
    // consumer. The widened transaction boundary therefore needs one fact.
    Behaviour {
        name: "dep.atomic_batch_rewrites",
        what: "dependency additions and removals are judged as one final graph and guarded publication, while add-from and blueprint reverse blocks edges to existing or same-batch tasks publish with creation",
        proof: &[
            "dep::batch_dependency_rewrites_apply_one_final_acyclic_graph_atomically",
            "add_from::reverse_blocks_edges_join_creation_transaction",
            "blueprint_application::blueprint_application_is_atomic_and_remints_under_contention",
        ],
    },
    // `A1-343`. A done cut used to be categorically closed to `add --from`,
    // forcing historical corrections through direct frontmatter edits that
    // skipped close-time resolution. The command and evidence kinds are old;
    // this evidence-resolving exception is the widened meaning.
    Behaviour {
        name: "add_from.resolves_terminal_ledger_entries_atomically",
        what: "a manifest may add proved done history to a done cut only after every citation resolves and trace/evidence baselines are captured, with any failure leaving the complete batch and cut unchanged",
        proof: &["add_from::manifest_creation_fields_and_terminal_ledger_are_atomic"],
    },
    // `A1-343`, audited beside `A1-385`. Creation values that consist only of
    // zero-width characters are absent to a reader but survived ordinary
    // `trim()` checks, recreating the metadata defect the preceding audit had
    // just closed.
    Behaviour {
        name: "authoring.refuses_visually_empty_creation_values",
        what: "single and manifest authoring refuse visually empty ids, titles, metadata, relationships and bodies before any file is written",
        proof: &[
            "add::creation_refuses_every_visually_empty_model_value",
            "add_from::manifest_refuses_every_visually_empty_model_value_atomically",
            "an_id_that_is_not_a_file_name_is_refused",
        ],
    },
    // `A1-386`. The blueprint format already existed, but a build that only
    // parses its parameter declarations cannot distinguish itself through
    // `--require` from one that binds them totally and safely.
    Behaviour {
        name: "blueprint.parameters_bind_totally_after_parsing",
        what: "blueprint parameters bind every parameterisable typed string slot after YAML parsing, refuse missing, undeclared or unused names, and treat supplied values as opaque non-recursive text",
        proof: &["blueprint_binding::binding_is_total_field_aware_and_non_recursive"],
    },
    // `A1-387`. The blueprint format and its relationship-shaped manifest
    // fields already existed, but an older build can only preserve strings
    // such as `local:design`; it cannot mint the sibling ids, resolve the graph
    // or prove that the complete result passes the ordinary mutation gate.
    Behaviour {
        name: "blueprint.references_resolve_atomically",
        what: "blueprint relationships use explicit `local:` and `task:` namespaces, resolve every sibling key to an id minted for one current plan snapshot, and publish the complete graph into one existing cut only when its post-state passes",
        proof: &[
            "blueprint_application::blueprint_references_are_namespaced_and_total",
            "blueprint_application::blueprint_graph_refusals_are_specific_and_write_nothing",
            "blueprint_application::blueprint_application_is_atomic_and_remints_under_contention",
        ],
    },
    // `A1-393`. `export-agents-md` and the blueprint command both predate the
    // generated guidance. Their surface names cannot distinguish a build whose
    // agent contract teaches the explicit-file preview and trust boundary from
    // one which leaves an agent unaware that blueprints exist.
    Behaviour {
        name: "agent_contract.guides_explicit_blueprint_application",
        what: "the generated agent contract says when a recurring plan shape warrants a blueprint, gives the explicit-file dry-run command, and requires review of its source identity and complete predicted graph before application",
        proof: &["the_agent_contract_guides_explicit_blueprint_application"],
    },
    // `A1-369`. `APLAN021` remains a warning because temporarily emptying a cut
    // is part of moving its work, but an explicit work field on a newly added
    // checkpoint is not an intermediate state. `--require add` and `--require
    // APLAN021` both pass on the build that writes the invalid milestone and on
    // the build that refuses it, so the changed meaning needs its own name.
    Behaviour {
        name: "add.refuses_milestone_work_fields",
        what: "`aplan add` and `add --from` refuse a milestone carrying `effort` or `paths`, and \
               write nothing, regardless of the warning policy",
        proof: &[
            "add::a_milestone_with_work_fields_is_refused_before_plain_add_writes",
            "add_from::a_manifest_milestone_with_work_fields_refuses_the_whole_batch",
        ],
    },
    // `A1-406`. The first A1-369 implementation classified a refusal by a new
    // diagnostic's subject rather than the task this add created. Adding work
    // to an existing malformed cut changed APLAN021's message and was therefore
    // mistaken for adding the cut's pre-existing work field. The old and fixed
    // builds both advertise the rule and the broader refusal behaviour, so this
    // accepted input needs its own name.
    Behaviour {
        name: "add.milestone_work_refusal_is_new_input_only",
        what: "the unconditional milestone-work-field refusal applies only to a milestone this \
               add creates; improving an existing malformed cut remains under warning policy",
        proof: &[
            "add::an_improving_plain_add_to_an_existing_malformed_milestone_is_not_hard_refused",
            "add_from::an_improving_manifest_add_to_an_existing_malformed_milestone_is_not_hard_refused",
            "mcp::mcp_add_can_improve_an_existing_malformed_milestone",
        ],
    },
    // `A1-704`. `add` already created tasks and milestones, but no supported
    // invocation could make a new milestone around work which already belonged
    // to another cut: every sequence exposed either an empty destination or
    // double membership. The command and agent surfaces retain their old names,
    // so a consumer needs a behaviour name to require the atomic widening.
    Behaviour {
        name: "add.creates_cut_around_existing_members",
        what: "`add --member`, native application/TUI add and `aplan_add` create an explicitly identified milestone around existing live work, remove every old membership and publish all files as one validated guarded batch",
        proof: &[
            "new_cut::a_new_cut_moves_existing_amorph_work_atomically",
            "new_cut_preview_names_both_membership_directions_and_stale_input_publishes_nothing",
            "a_new_cut_move_returns_versioned_files_and_both_membership_directions",
            "add_form_moves_an_existing_member_through_the_native_typed_preview",
        ],
    },
    // `A1-706`, auditing `A1-704`. The two identity fields had to reach
    // `aplan_add` so member mode could name its new cut, but the flat request
    // also made ordinary add accept them. The schema exposed that widening and
    // the implementation intentionally keeps it, so it needs a separately
    // require-able name and a proof outside member mode.
    Behaviour {
        name: "mcp.add_accepts_explicit_identity",
        what: "ordinary `aplan_add` accepts an explicit task id and reporting phase, uses both \
               verbatim and mints or derives them only when the caller omits them",
        proof: &["mcp::mcp_add_accepts_explicit_identity_for_an_ordinary_task"],
    },
    // `A1-407`. The agent surface exposed the same fields as `add` but retained
    // only the generic mutation gate, so warning policy let it write exactly the
    // invalid milestone A1-369 made both CLI forms refuse. The shared add gate
    // fixes that existing tool; no command, rule or input field is new.
    Behaviour {
        name: "mcp.add_refuses_milestone_work_fields",
        what: "`aplan_add` refuses a new milestone carrying `effort_days` or `paths` before \
               writing, under every warning policy, like both CLI add forms",
        proof: &["mcp::mcp_add_refuses_milestone_work_fields_before_writing"],
    },
    // `A1-604`. Every plan tool already accepted `root`, so both the tool name
    // and its input field exist in older builds. Those builds changed only the
    // Markdown scan directory and silently retained the launch repository's
    // config, traces, providers and Git boundary. The same request now selects
    // or refuses one complete context, which needs a capability name of its own.
    Behaviour {
        name: "mcp.explicit_root_resolves_one_repository_context",
        what: "an explicit MCP root denotes either a repository carrying `.aplan.yml` or its \
               configured plan directory, resolves plan, policy, traces, evidence providers and \
               Git together, exposes bounded provenance and refuses a partial or ambiguous root \
               without falling back to the server's launch repository",
        proof: &[
            "explicit_root_selects_plan_policy_and_mutation_as_one_context",
            "explicit_root_refuses_partial_and_missing_contexts_without_fallback",
            "mcp::an_explicit_root_selects_an_independent_repository_over_stdio",
        ],
    },
    // `A1-604`, found while extracting the configuration projection shared by
    // the CLI and MCP. The server passed its repository fallback as an explicit
    // trace override, so both `traces_root` and `no_traces` were unreachable.
    Behaviour {
        name: "mcp.launch_context_honours_trace_selection",
        what: "an MCP server's launch context honours configured `traces_root` and `no_traces` \
               before applying its repository fallback",
        proof: &["mcp::mcp_launch_context_honours_configured_trace_selection"],
    },
    // `A1-254`. A widening with no new rule, command, config key, format or
    // evidence kind: `file:./src/a.rs` was `APLAN015` and now resolves, and
    // `paths: ["./src/**"]` matched nothing and now owns the tree. A consumer
    // pinning `--require APLAN015` cannot tell a build that accepts them from one
    // that does not. `traces` has normalised since it existed, which is what made
    // the disagreement invisible: one field of three behaved, so the two that did
    // not looked like the author's mistake.
    Behaviour {
        name: "evidence.file_path_is_normalised",
        what: "a path is read the same way wherever a plan writes one: a `file` citation, a \
               `paths` glob and a `traces` target all resolve however it is spelled (`./x`, \
               `a/../x`)",
        proof: &[
            "path_spelling::a_path_spelled_any_way_resolves_in_both_fields",
            "path_spelling::an_oddly_spelled_citation_is_judged_against_paths_the_same_way",
        ],
    },
    // `A1-261`. A widening `A1-159` shipped and named nothing, so `facts` was
    // silent about it and no `--require` could assert it. It is the rung a
    // prose deliverable stands on -- a decision, a design record, a reviewed
    // report -- and it binds twice where `commit` binds once: the blob is in the
    // tree at the cited rev, and `APLAN031` requires it inside the task's own
    // `paths`.
    //
    // Named because three pilots met the question of how such work closes and
    // none found this: `p00020` and `p0000000000005` each took an exempt tag,
    // and `p00002` -- which found the answer, one rung higher, by making its
    // design document executable -- could not have pinned the build that has it.
    // 41 `file` citations across two plans carry no anchor at all, which is what
    // an undiscoverable feature looks like from the outside.
    Behaviour {
        name: "evidence.file_names_a_section",
        what: "a `file` citation may name the section that proves it \
               (`file:docs/report.md#result`), resolved against that document's own headings \
               and table rows at the cited rev, so a written decision is citable as evidence \
               rather than only as a file that exists",
        proof: &[
            "a_file_citation_may_name_the_section_that_proves_it",
            "the_section_is_read_at_the_rev_and_not_from_the_working_tree",
            "relevance_is_judged_on_the_path_and_not_on_the_anchor",
        ],
    },
    // `A1-256`. A flag, which `facts` does not carry, on a command it lists: a
    // consumer pinning `--require facts` cannot tell a build that can name itself
    // from one that cannot. The precedent is `config.explicitly_selectable`,
    // which named `--config` and `--no-config` for the same reason -- and this
    // one exists because `--require` is precisely what could *not* answer the
    // question three pilots asked.
    Behaviour {
        name: "facts.reports_which_build",
        what: "`aplan facts --identity` names this build -- its version, the commit it was \
               built from, and the digests of its capability document and its own image -- \
               without needing a plan, a repository or a clean tree",
        proof: &[
            "build_identity::the_identity_needs_no_plan_and_no_repository",
            "build_identity::a_dirty_tree_is_answered_here_and_refused_by_attest",
            "build_identity::the_binary_digest_is_this_binarys_own",
        ],
    },
    // `A1-638`. `facts --identity` and the source field both predate this
    // correction. Older build scripts can inherit Git discovery or
    // configuration controls from their caller and silently report no source;
    // they can also retain a packed branch's old commit when reflogs are
    // disabled. No existing surface name distinguishes those builds.
    Behaviour {
        name: "facts.source_identity_tracks_active_checkout",
        what: "the build-time source identity ignores inherited `GIT_` context and follows the \
               active checkout through loose refs, packed refs, disabled reflogs and linked \
               worktrees without making ordinary unrelated branches build inputs",
        proof: &[
            "build_identity::build_identity_tracks_only_the_checked_out_ref_and_stays_fresh",
            "build_identity::linked_worktree_and_non_git_builds_have_stable_existing_inputs",
            "build_identity::packed_ref_without_reflogs_still_refreshes_the_source_identity",
            "build_identity::source_identity_ignores_all_inherited_git_context",
        ],
    },
    // `A1-421`. The `tui` command is deliberately present in both builds: the
    // headless one keeps an enablement stub, so the command inventory cannot
    // answer whether the interactive implementation exists. This names the
    // feature-sensitive answer and lets an installer require it without trying
    // to launch a terminal UI from automation.
    Behaviour {
        name: "facts.reports_build_capabilities",
        what: "`aplan facts` reports whether optional build capability `build.tui` is enabled, \
               and `facts --require` accepts it only in a build that compiled the interface",
        proof: &[
            "tui::a_build_capability_is_advertised_only_when_enabled",
            "tui::the_default_build_advertises_the_tui_capability",
        ],
    },
    // `A1-538`. `aplan tui` already promised to restore the cursor, but the
    // first implementation gave both its mode guard and Ratatui ownership.
    // A real PTY therefore received two hide/show pairs while the fake mode
    // controller observed only one. This names the corrected terminal behavior
    // so a staged binary can distinguish it from the superficially safe one.
    Behaviour {
        name: "tui.cursor_restoration_is_singular",
        what: "`aplan tui` leaves cursor visibility to the Ratatui terminal, so a foundation frame and normal exit emit one hide and one restoring show rather than two competing pairs",
        proof: &["the_renderer_is_the_only_cursor_owner"],
    },
    // `A1-422`. The command already existed, but its first implementation drew
    // one frame and blocked in `read`, with no semantic reducer, refresh or
    // stale-result boundary. The command inventory and `build.tui` capability
    // are identical on both builds, so consumers need a behaviour name for the
    // event-driven foundation.
    Behaviour {
        name: "tui.event_loop_is_generation_safe",
        what: "`aplan tui` decodes terminal input into semantic actions, redraws only after state changes, and runs one identified effect with one bounded queued successor while stale or failed results cannot replace the last complete snapshot",
        proof: &[
            "event_loop_is_dirty_driven_and_effects_are_generation_safe",
            "terminal_events_decode_without_updating_the_model",
            "crossterm_poll_and_read_have_one_owner",
            "one_worker_runs_one_waits_and_a_third_is_refused",
            "navigation_remains_available_while_an_effect_is_blocked",
            "shutdown_joins_running_work_and_drops_work_that_never_started",
            "terminal_modes_restore_before_shutdown_waits_for_a_worker",
            "the_tui_refresh_reloads_repository_inputs",
        ],
    },
    // `A1-539`. `catch_unwind` runs after the process panic hook, so merely
    // turning a worker panic into an effect failure still wrote raw diagnostics
    // through an active alternate screen. The worker boundary now marks its
    // thread for a one-time hook dispatcher while preserving the host's hook
    // everywhere else.
    Behaviour {
        name: "tui.caught_worker_panics_do_not_write_terminal",
        what: "a panic caught at the TUI effect-worker boundary becomes a typed failure without writing through the active terminal, while non-worker panics still reach the host's previous panic hook",
        proof: &["a_worker_panic_is_a_typed_completion"],
    },
    // `A1-425`. Explicit shutdown already joined the sole active worker, but
    // an unwind from the terminal body bypassed that call and `JoinHandle`'s
    // default drop detached repository work. The runtime now gives ordinary
    // return and unwinding the same bounded-work cleanup guarantee.
    Behaviour {
        name: "tui.worker_cleanup_is_unwind_safe",
        what: "dropping the TUI worker runtime, including during unwinding, cancels queued work and joins the sole running repository or process operation instead of detaching it",
        proof: &["dropping_the_runtime_joins_running_work_and_drops_work_that_never_started"],
    },
    // `A1-548`. The original session guard restored modes only when Rust
    // control flow returned through it. Catchable process signals and job
    // control bypassed that path, while the ordinary CLI surface had no
    // executable terminal-state census. This name distinguishes the complete
    // process-lifecycle boundary from the earlier unwind-only guarantee.
    Behaviour {
        name: "tui.terminal_lifecycle_handles_signals",
        what: "every non-interactive command preserves its caller's terminal and emits only declared SGR controls; the TUI restores exact terminal state, stops active configured-command groups and re-raises catchable termination signals without waiting for the worker, while suspend restores before stopping and continue reacquires and repaints",
        proof: &[
            "terminal_integrity::every_non_tui_command_preserves_terminal_state_and_emits_only_sgr",
            "tui_journey::terminating_signals_restore_the_terminal_and_keep_the_exit_reason",
            "tui_journey::suspend_restores_exact_termios_and_resume_repaints",
            "tui_journey::termination_does_not_wait_for_an_active_provider",
            "a_signal_during_each_partial_acquisition_restores_the_complete_attempt",
            "coalesced_continue_cannot_leave_the_process_suspended_forever",
        ],
    },
    // `A1-448`. The Unix signal lifecycle above says nothing about builds on
    // the other supported operating systems, and an in-memory renderer cannot
    // prove which modes a real PTY observed during resize, focus or teardown.
    Behaviour {
        name: "tui.terminal_portability_is_explicit",
        what: "a dedicated platform-neutral real-PTY module exercises mode ownership, focus, \
               resize and minimum-size recovery plus ordinary and interrupt teardown, while the \
               support matrix keeps unavailable platform observations distinct from passes",
        proof: &[
            "tui_portability::shared_real_pty_lifecycle_handles_focus_resize_and_tiny_terminals",
        ],
    },
    // `A1-448`. A1-440 admitted the bounded private draft type but deliberately
    // did not connect it to the live TUI, so the `tui` command name cannot tell
    // a staged consumer whether an editor is launched under raw screen modes or
    // whether repository changes made during the handoff are ever observed.
    Behaviour {
        name: "tui.external_editor_handoff_is_safe_and_identity_bound",
        what: "the Add form launches VISUAL or EDITOR directly only after restoring terminal \
               modes, accepts one private bounded Markdown draft through ordinary preview, and \
               reacquires the interface only after requesting a current input observation",
        proof: &[
            "tui_portability::external_editor_handoff_restores_modes_and_refreshes_identity",
            "external_editor_body_is_bound_to_the_add_preview_request",
        ],
    },
    // `A1-560`, found by the adversarial audit of `A1-548`. The terminal was
    // already restored before graceful shutdown joined a worker, but that
    // blocking join stopped consuming the installed signal handler's queue.
    // An older build therefore advertises the broad lifecycle behavior above
    // while swallowing termination delivered in this last process state.
    Behaviour {
        name: "tui.termination_preempts_worker_shutdown",
        what: "after restoring the terminal for a graceful quit, the TUI continues polling \
               lifecycle signals while it waits for its worker, so catchable termination kills \
               configured-command groups and preserves the signal exit instead of hanging",
        proof: &[
            "shutdown_polling_never_joins_a_running_worker",
            "a_lifecycle_event_arriving_only_during_worker_shutdown_is_consumed",
            "tui_journey::termination_after_terminal_restoration_preempts_worker_shutdown",
        ],
    },
    // `A1-423`. The foundation renderer grew from one task list into a
    // responsive routed shell. This changes the meaning of the existing `tui`
    // command without adding a command, rule or configuration key, so staged
    // consumers need a behaviour name for the keyboard, catalogue and cell
    // geometry contract.
    Behaviour {
        name: "tui.shell_is_responsive_and_contextual",
        what: "the TUI provides keyboard-complete routed wide, normal and narrow shells whose dispatch, footer, help and palette share one availability-aware command catalogue and whose text, cursor and hit targets use terminal-cell geometry",
        proof: &[
            "the_responsive_shell_uses_one_contextual_command_catalogue",
            "fixed_size_buffers_pin_every_layout_focus_and_minimum_refusal",
            "grapheme_cursor_and_hit_targets_share_cell_geometry",
        ],
    },
    // `A1-444`. The original contextual shell catalogue covered navigation
    // controls only: mutations were appended in a second palette list, full
    // help omitted them, and intentionally non-interactive commands existed
    // only in a test table. The `tui` command name and the earlier shell
    // behaviour therefore cannot tell a staged consumer whether the complete
    // product surface is discoverable or whether a new command silently fell
    // through the gap.
    Behaviour {
        name: "tui.action_discovery_is_closed_and_contextual",
        what: "palette, full help and contextual footer consume one availability-aware action catalogue; every command published by facts maps to a driven native surface, guided route, inert exact CLI alternative or the TUI entry point, and every CLI leaf and semantic option has an executable disposition",
        proof: &[
            "every_product_command_resolves_to_one_discoverable_action_or_boundary",
            "tui_read_surface::command_disposition_is_closed_and_every_read_member_runs",
            "tui_read_surface::every_read_option_has_a_disposition_and_behavioral_dimensions_run",
            "tui_read_surface::every_command_leaf_and_option_has_one_interactive_disposition",
            "tui_journey::action_discovery_guides_boundaries_and_native_routes_in_a_real_terminal",
        ],
    },
    // `A1-553`, found while driving A1-548's active-provider path through the
    // verification modal. The catalogue recognized arrows and j/k, but modal
    // dispatch applied their main-list actions. Tab happened to take the
    // modal-aware path, making advertised keys disagree by input family.
    Behaviour {
        name: "tui.modal_navigation_is_consistent",
        what: "arrow, j/k, Tab and Shift-Tab navigation all move and wrap the active modal selection instead of changing the obscured main view",
        proof: &[
            "every_modal_movement_key_moves_the_modal_selection",
            "tui_journey::termination_does_not_wait_for_an_active_provider",
        ],
    },
    // `A1-540`, found by adversarially reviewing `A1-423`. The first
    // responsive shell could move selection below the painted rows, map a
    // border click to a task, let wheel input mutate a hidden narrow list, and
    // retain unbounded zero-width palette input. The existing shell behaviour
    // name cannot distinguish that build from this corrected interaction and
    // resource boundary.
    Behaviour {
        name: "tui.selection_is_visible_and_pointer_safe",
        what: "the responsive TUI keeps selected task rows visible, derives painting and pointer targets from one bounded viewport, and preserves an escapable, size-bounded input surface across narrow and below-minimum layouts",
        proof: &[
            "selection_viewport_and_pointer_geometry_stay_in_lockstep",
            "minimum_refusal_and_palette_inputs_remain_bounded_and_escapable",
        ],
    },
    // `A1-541`, found by a second adversarial review of `A1-423`. The shell's
    // task table was bounded, but route explanations, modal command rows and
    // detail fields could still rely on the widget to clip or wrap content.
    // Wheel input also reused wrapping keyboard navigation. Those are stronger
    // forms of existing shell behavior and need a build-level name of their own.
    Behaviour {
        name: "tui.shell_content_is_explicitly_bounded",
        what: "every responsive TUI surface fits hostile text to its real content rectangle with an explicit continuation marker, pointer targets stay inside painted cells, and wheel navigation stops at task-list boundaries",
        proof: &["every_responsive_surface_fits_content_at_layout_boundaries"],
    },
    // `A1-445`. The original shell named dark, light and monochrome palettes,
    // but always launched dark/Unicode, had no reduced-capability policy and
    // animated every foreground operation. The existing shell behaviour does
    // not distinguish that build from this complete appearance contract.
    Behaviour {
        name: "tui.visual_language_is_semantic_and_deterministic",
        what: "the TUI resolves explicit dark, light, low-color and monochrome themes against NO_COLOR and reported color capacity without guessing a background; semantic roles, exact compact visualizations, ASCII equivalents and reduced-motion progress survive every responsive class",
        proof: &[
            "appearance_policy_is_deterministic_across_terminal_capabilities",
            "tui_journey::appearance_controls_apply_from_the_first_real_terminal_frame",
            "reduced_motion_keeps_progress_state_without_animation",
            "tui_cli_appearance_options_reach_the_terminal_adapter",
            "visual_language_review_matrix_preserves_hierarchy_and_exact_values",
        ],
    },
    // `A1-446`. The responsive shell, modal-navigation and visual-language
    // behaviours predate complete input lifecycle handling. Older builds can
    // accept unadvertised modified arrows, let key repeat request or cancel a
    // form action, route wheel input through the topmost layer, or advertise a
    // key from a mapping separate from the one dispatch uses. No command,
    // option or format name distinguishes that unsafe interaction boundary.
    Behaviour {
        name: "tui.input_accessibility_is_safe_and_discoverable",
        what: "the TUI derives dispatch, quick help, complete help and palette key labels from one lifecycle-aware binding catalogue; keyboard-complete layers restore exact focus, and clamped pointer accelerators cannot click or wheel through an overlay or activate on drag, release or modifiers",
        proof: &[
            "input_accessibility_uses_one_key_lifecycle_and_restores_focus",
            "pointer_accelerators_are_layered_clamped_and_nondestructive",
        ],
    },
    // `A1-672`. The input-accessibility behaviour above made overlays own
    // pointer input, but every press inside a modal or mutation layer was
    // deliberately inert. The existing `tui` command and accessibility name
    // cannot distinguish that fail-closed interim from complete semantic
    // overlay activation.
    Behaviour {
        name: "tui.overlay_pointer_activation_is_semantic_and_guarded",
        what: "visible modal rows, labelled overlay controls and mutation fields expose typed pointer targets derived from their painted geometry, while confirmation retains freshness, broad-change and pending-work guards",
        proof: &[
            "overlay_pointer_targets_share_painted_geometry_and_preserve_guards",
            "pointer_confirmation_preserves_fresh_press_and_broad_change_arming",
            "tui_journey::overlay_pointer_controls_run_end_to_end_in_a_real_terminal",
        ],
    },
    // `A1-666`. Older builds resolved configuration, plan, repository and
    // projection inputs before the terminal owned a frame, so `tui` could look
    // hung even though the command itself already existed.
    Behaviour {
        name: "tui.startup_feedback_precedes_plan_io",
        what: "an eligible interactive launch paints a responsive in-frame loading state before configuration or plan reads begin, consumes one generation-safe phase stream, and restores the terminal before reporting failure or joining cancelled work",
        proof: &[
            "first_loading_frame_precedes_cancellable_worker_and_quit_restores_modes",
            "dropping_a_startup_runtime_requests_cancellation_before_joining",
            "startup_executor_projects_after_host_loading_and_publishes_exact_build",
            "startup_failure_exits_through_the_post_restoration_error_path",
            "startup_progress_is_generation_safe_and_adopts_one_snapshot",
            "startup_crossing_a_focus_epoch_schedules_confirmation_after_adoption",
            "the_tui_refresh_reloads_repository_inputs",
            "tui_journey::loading_frame_precedes_blocked_configuration_and_transitions_in_place",
            "tui_journey::blocked_startup_quit_restores_the_terminal_before_joining",
        ],
    },
    // `A1-670`. The task inspector and its sections already existed, but
    // older builds flattened identity, state, coordination, placement, counts,
    // warnings and links into indistinguishable strings. The command and route
    // names cannot tell a consumer whether the renderer has to parse prose or
    // whether narrow reflow preserves semantic roles and exact values.
    Behaviour {
        name: "tui.inspectors_use_semantic_information_hierarchy",
        what: "task inspector sections carry typed identity, groups, labelled values, badges, metrics, callouts and links; wide panes align them and narrow panes stack the same exact values with non-colour meaning and stable selection",
        proof: &[
            "inspector_summary_is_typed_grouped_exact_and_link_complete",
            "semantic_inspector_hierarchy_survives_appearance_width_and_reflow",
            "semantic_inspector_terminal_state_retains_textual_meaning",
            "tui_journey::semantic_task_summary_is_scannable_and_navigable_in_a_real_terminal",
        ],
    },
    // `A1-671`. The responsive shell and every destination already existed,
    // but older builds spent ten rows on bordered chrome at the supported
    // minimum and used one global width fallback. The command name cannot tell
    // a staged consumer whether short layouts retain useful route work,
    // orientation, discoverability and in-progress state.
    Behaviour {
        name: "tui.shell_adapts_by_route_and_available_rows",
        what: "the shell allocates chrome independently from pane width, retains route-specific useful work and explicit primary/detail orientation through short and narrow layouts, keeps hidden commands discoverable, and reflows without changing semantic or editable state",
        proof: &[
            "adaptive_shell_allocates_chrome_by_width_height_and_route",
            "responsive_resize_preserves_semantic_and_editable_state",
            "short_form_viewport_follows_cursor_and_manual_scroll_starts_from_the_painted_rows",
            "tui_journey::adaptive_shell_preserves_orientation_and_work_across_real_resizes",
        ],
    },
    // `A1-453`. The `tui`, `init` and `facts` command names all predate this
    // flow. Older builds fail before entering the terminal when setup inputs
    // are absent and carry no in-application view of the exact running build,
    // so none of those surface names distinguishes the widened behaviour.
    Behaviour {
        name: "tui.onboarding_and_capabilities_are_typed",
        what: "the TUI keeps missing, unreadable or invalid launch inputs in a read-only guided state, can retry into one complete snapshot, names root and configuration trust choices, and renders About from the same typed facts as `aplan facts`",
        proof: &[
            "tui_launch_classifies_every_onboarding_cause_without_writing",
            "tui_configuration_loading_is_bound_to_the_classified_selection",
            "tui_loaded_launch_names_root_explicit_config_and_no_config_semantics",
            "onboarding_retry_is_typed_inert_and_transitions_in_place",
            "retry_executor_confirms_one_typed_launch_snapshot",
            "onboarding_causes_and_complete_about_projection_are_rendered",
            "capability_projection_disposes_every_typed_fact_collection",
        ],
    },
    // `A1-422`, found while checking the headless release shape. TUI behaviour
    // names originally came from the unconditional core registry, so a build
    // that compiled only the enablement stub still claimed cursor and event-loop
    // guarantees. `build.tui` said false, but `--require tui.*` said true.
    Behaviour {
        name: "facts.omits_disabled_feature_behaviours",
        what: "`aplan facts` omits `tui.*` behaviour names when `build.tui` is disabled, so `--require` cannot accept a promise made only by code this build did not compile",
        proof: &["a_disabled_feature_omits_its_behaviours_as_well_as_its_capability"],
    },
    // `A1-263`. A widening with no new rule and no new command: `why` answers
    // the same question with a different traversal, and an older binary answers
    // the *complete* one just as confidently. A consumer whose script counts
    // blockers, or an agent budgeting context on the answer, cannot tell the two
    // builds apart by any name that already existed -- which is exactly what
    // `p00020`'s item 23 predicted would keep happening. The flag is not the
    // signal either: `--all` is accepted by neither older binary nor named in
    // any surface they publish.
    // `A1-286`. A widening with no new command, rule, config key, format or
    // evidence kind: a *global flag*, which is the one part of the surface
    // `facts` does not enumerate -- `commands` carries a name, an about and the
    // formats it accepts, and no flags at all. So a consumer wiring CI has no
    // way to ask whether the build it has can be told to colour, and an older
    // one fails with `unexpected argument` rather than anything legible. That is
    // `A1-282`'s argument, on the surface that raised it.
    //
    // The name is about the *explicit* half only. Following the terminal, and
    // honouring `NO_COLOR`, `CLICOLOR_FORCE` and `CLICOLOR`, is what `anstream`
    // has always done here, so a name promising that would be true of builds
    // predating this one and would say nothing.
    // `A1-289`. A widening with no new rule, command, config key, format or
    // evidence kind: what changed is what an *existing* surface does with the
    // text a plan gave it. An older binary prints the escape and runs it, and a
    // consumer reviewing an untrusted plan cannot tell the two builds apart by
    // any name that already existed -- which is the case this list is for.
    Behaviour {
        name: "output.plan_text_cannot_steer_a_terminal",
        what: "a control character in any field a plan supplies is replaced by a visible marker \
               before it reaches a human rendering, at every `--color` setting, while the machine \
               formats keep the bytes the plan holds",
        proof: &[
            "color::no_human_surface_lets_a_plan_write_to_the_terminal",
            "color::a_machine_format_keeps_the_bytes_the_plan_holds",
            "two_titles_differing_only_in_an_escape_still_differ",
        ],
    },
    // `A1-294`, from pre-public adoption evidence. A widening with no new command, no
    // new flag and no new rule: `export-agents-md` and `APLAN041` accept a marker
    // they used to count as absent. An older binary reports a file with a short
    // marker as having *zero* opening markers -- which is a refusal, so it is
    // loud, but a consumer cannot tell the two builds apart by any name that
    // already existed.
    Behaviour {
        name: "contract.begin_marker_matched_by_prefix",
        what: "the generated block's opening marker is recognised by its start: a line \
               opening `<!-- aplan:begin`, then a space or a dash, and closing `-->`, which \
               writing the block replaces with the full marker",
        proof: &[
            "agents_md::a_short_begin_marker_is_recognised_and_written_out_in_full",
            "a_marker_named_in_prose_is_not_a_marker",
        ],
    },
    Behaviour {
        name: "color.explicitly_selectable",
        what: "`--color=auto|always|never` overrides the terminal detection in both directions, \
               reaching everything the tool prints -- its findings, its refusals, and `clap`'s own \
               usage errors -- while the machine formats stay byte-identical at every setting",
        proof: &[
            "color::colour_can_be_forced_into_a_pipe",
            "color::colour_can_be_refused_at_a_terminal",
            "color::an_explicit_choice_beats_the_environment",
            "color::the_refusal_is_on_the_same_stream_as_a_finding",
            "color::claps_own_error_obeys_the_choice",
            "color::the_machine_formats_carry_no_colour_when_it_is_forced",
        ],
    },
    // `A1-665`. Verification already reported runner starts and finishes, but
    // neither those values nor the plan loader identified an operation
    // generation. No existing fact distinguishes a build that can reject a
    // late event from one that applies it to whichever operation is visible.
    Behaviour {
        name: "progress.events_are_generation_safe",
        what: "plan loading and configured-provider work emit one ordered typed lifecycle whose \
               operation generation, child identities and monotonic measures let a consumer reject \
               late, duplicate, regressing or cross-operation events",
        proof: &[
            "progress_events_are_generation_safe",
            "plan_store_emits_a_valid_bounded_source_lifecycle",
            "verification_context_is_identified_and_uses_the_typed_engine",
        ],
    },
    // `A1-665`. `show --refresh-evidence` used to write runner-specific lines
    // to stderr unconditionally. The new flag is assertable, but its default
    // silence in automation and strict stream split are behavior of existing
    // commands and therefore need a name of their own.
    Behaviour {
        name: "progress.stream_output_is_automation_safe",
        what: "operation progress is delayed and stderr-only; auto requires an attended human \
               mode outside CI, always is explicit, never emits no progress bytes, machine stdout \
               is unchanged and the TUI retains in-frame ownership",
        proof: &[
            "progress::explicit_progress_is_stderr_only_and_never_changes_structured_stdout",
            "progress::auto_is_hidden_in_pipes_and_machine_modes_but_visible_for_an_attended_human_mode",
        ],
    },
    // `A1-667`. The progress flag and typed events already existed, but older
    // builds ended presentation with each child load/provider operation and
    // did not consume the identities they advertised. No new command, format
    // or rule distinguishes the complete command-lifetime renderer.
    Behaviour {
        name: "progress.cli_renderer_is_identity_safe_and_terminal_safe",
        what: "one command-lifetime CLI renderer validates operation and child identities, \
               aggregates bounded concurrent work, fits the terminal, serializes ordinary \
               output and restores its transient row on success, failure, cancellation or \
               catchable process interruption",
        proof: &[
            "progress_renderer_validates_identities_and_aggregates_children",
            "progress::auto_is_hidden_in_pipes_and_machine_modes_but_visible_for_an_attended_human_mode",
            "progress::concurrent_runners_are_aggregated_without_identity_or_escape_leaks",
            "progress::a_slow_command_error_emits_one_failed_terminal_before_the_diagnostic",
            "cli_termination::noninteractive_signals_stop_configured_groups_and_preserve_the_signal",
            "tui_portability::portable_cli_progress_interrupt_cleans_its_terminal_row",
        ],
    },
    // `A1-191`, ADR-0035. Same shape as the entry below and as `traces.table_rows`
    // above: a rule accepting more than it did, with nothing new in `facts` to
    // see it by.
    Behaviour {
        name: "traces.structured_documents",
        what: "a `traces` anchor may address a node of a YAML or JSON document, as a JSON \
               Pointer (`#/states/provisioning`) or a JSONPath (`#$.states.provisioning`)",
        proof: &[
            "both_spellings_address_the_same_node",
            "every_construct_that_could_match_more_than_one_node_is_refused_by_name",
            "layout_is_not_drift_and_a_value_is",
            "a_yaml_node_is_traceable_and_drifts_when_its_value_changes",
        ],
    },
    // `A1-297`. The widening has no new rule code, command, config key, format or
    // evidence kind, so `facts` reported the same surface before and after it and
    // an adopter had no way to ask which behaviour their binary has.
    Behaviour {
        name: "traces.unstamped_is_reported",
        what: "a `done` task that stamps none of its anchored `traces` is reported by \
               `APLAN028`, rather than exempt as a task closed before drift was enabled",
        proof: &["a_closed_task_that_stamps_nothing_is_reported_but_an_unstampable_trace_is_not"],
    },
    // `A1-325`, ADR-0037. No new rule code, command, config key, format or
    // evidence kind: the same artifacts, one line longer, and the same
    // `APLAN041` reporting a difference it could not previously name. A
    // consumer pinning a build in CI and regenerating locally is exactly the
    // reader who needs to ask which of the two writes the line, and before
    // this name there was nothing in `facts` that changed.
    Behaviour {
        name: "generated.names_the_build_that_wrote_it",
        what: "a generated `ROADMAP.status.md` and `AGENTS.md` block carry a comment naming \
               the version and surface that wrote them, and a drift report names the build \
               doing the checking",
        proof: &[
            "banner::the_committed_roadmap_says_which_build_wrote_it",
            "agents_md::the_written_block_names_the_build_that_wrote_it",
            "a_block_written_by_another_build_is_reported_as_a_build_difference",
        ],
    },
    Behaviour {
        name: "why.cut_bounded_by_default",
        what: "`aplan why` reports an unfinished cut as itself rather than walking into its \
               members, and names the traversal that produced every answer; `--all` asks for \
               the complete closure",
        proof: &[
            "why_reach::a_late_task_names_its_own_blockers_and_the_cut_in_front_of_it",
            "why_reach::a_chain_of_ordinary_tasks_is_walked_whole",
            "why_reach::the_json_names_the_traversal_and_what_was_folded",
        ],
    },
    // `A1-581`. The `why` command, JSON shape and MCP tool already existed and
    // `--all` already claimed a complete closure. None of those surface names
    // distinguishes an older build that silently drops an undefined blocker
    // below loaded work.
    Behaviour {
        name: "why.complete_retains_nested_undefined_blockers",
        what: "a complete why walk retains undefined blockers reached below loaded tasks on CLI, \
               JSON and MCP, while a cut-bounded walk hides an undefined id behind its cut",
        proof: &[
            "undefined_blockers_follow_the_requested_why_boundary",
            "why_reach::a_nested_undefined_blocker_respects_the_walk_boundary_on_every_why_surface",
            "mcp_why_retains_nested_undefined_blockers_at_the_requested_boundary",
        ],
    },
    // `A1-230`. No new command, flag, rule or config key: the same `add`,
    // returning the same shape, and a build predating it refuses most of a
    // fleet's writes *confidently* -- `already exists` is a true sentence about
    // a file, so there is nothing to notice and nothing in the surface that
    // says which answer a binary gives. Measured at three of eight agents
    // landing before and eight of eight after. A consumer running agents
    // against a shared checkout is the reader who has to ask, and the id
    // strategy they would otherwise reach for costs a readable id in every
    // `blocked_by` and every filename.
    // `A1-364`. The shape this list exists for, by its own test at the top of
    // the file: no new command, no new flag, no new rule, no new config key in
    // `.aplan.yml` -- and a build without it resolves a claimant's identity from
    // a different set of sources, confidently, with nothing in the surface to
    // say which. An adopter who writes the file and finds it ignored has no way
    // to tell an old binary from a misplaced file.
    Behaviour {
        name: "identity.reads_a_per_user_file",
        what: "a claimant's name may come from a per-user file outside the repository, read \
               between the environment variable and git's `user.name`, and holding nothing a \
               rule reads",
        proof: &[
            "the_user_file_loses_to_the_environment_and_beats_git",
            "every_field_is_about_the_actor_and_not_the_plan",
            "a_relative_or_empty_variable_names_no_file_rather_than_one_in_the_repository",
        ],
    },
    Behaviour {
        name: "ids.sequential_survives_concurrent_writers",
        what: "a command that mints an id re-reads the plan and mints again when another \
               writer takes it first, so concurrent writers do not serialise on the id",
        proof: &[
            "id_race::concurrent_adds_are_unique_complete_or_actionably_refused",
            "a_lost_id_retries_with_controlled_time_and_backoff",
            "a_command_that_runs_out_of_deadline_says_what_to_do",
            "id_race::a_lost_id_leaves_nothing_behind_in_the_cut",
            // The other writer. `add` reports a collision as
            // `EmitError::Exists` and these two as `SaveError::Exists`, down a
            // separate branch -- which this list covered for one of them and
            // not the other two until the audit asked (`A1-230`).
            "id_race::concurrent_supersedes_are_complete_or_actionably_refused",
            "id_race::concurrent_manifests_are_whole_exact_once_or_actionably_refused",
            "id_race::a_supplied_id_that_is_taken_is_not_reported_as_a_race",
            "a_supplied_id_does_not_wait_out_the_deadline",
            "a_failed_draw_waits_the_whole_window",
        ],
    },
    // `A1-628`. The bounded sequential-id retry already had a behaviour name,
    // but it covered only a collision at publication. An older build can still
    // refuse before that point while the store tries to capture a changing
    // candidate set, so changing the old entry would make `--require` claim a
    // boundary those builds do not have.
    Behaviour {
        name: "ids.concurrent_snapshot_instability_is_retried",
        what: "an ID-creating mutation treats candidate-set instability during its pre-write plan \
               snapshot as bounded contention, distinct from a file changed after it was read",
        proof: &[
            "a_transient_plan_snapshot_retries_with_controlled_time_and_backoff",
            "snapshot_instability_at_the_deadline_is_an_actionable_no_write_refusal",
        ],
    },
    // `A1-606`. The add/import/rekey commands and the cut label already
    // existed. Older builds confidently created a structurally valid task whose
    // id took over a displayed cut label, so neither the command nor the rule
    // catalogue distinguishes the unsafe behavior.
    Behaviour {
        name: "authoring.task_ids_preserve_cut_labels",
        what: "in-plan task creation and rekey refuse an id that would shadow a displayed cut \
               label, before publishing any file",
        proof: &["add::a_new_task_id_cannot_shadow_a_cut_label"],
    },
    // `A1-410`, found by the adversarial audit of `A1-409`. `rekey` already
    // existed and a clean plan already accepted quoted ids containing spaces,
    // but the command falsely refused them as an internal rewrite bug; it also
    // rebuilt block relationship lists and silently deleted their comments.
    // The command list cannot distinguish the builds, so the widening needs a
    // name of its own.
    Behaviour {
        name: "rekey.surgical_yaml_scalars",
        what: "`rekey` treats a quoted task id or a block-list relationship member as one scalar \
               and preserves the comments and bytes around it",
        proof: &[
            "rekey::a_quoted_source_id_with_whitespace_rekeys_surgically",
            "rekey::the_collision_fixture_repairs_every_graph_reference_and_reports_prose",
        ],
    },
    // `A1-411`, also found by the adversarial audit of `A1-409`. The report
    // suppressed a Markdown link to `OLD.md` because its exact-token boundary
    // treated the dot as part of a longer id. No surface name changed when the
    // existing report began including the filename its own move made stale.
    Behaviour {
        name: "rekey.reports_moved_filename_links",
        what: "`rekey` includes repository text naming the old conventional `.md` filename in \
               its possible-reference review list without rewriting it",
        proof: &["rekey::the_collision_fixture_repairs_every_graph_reference_and_reports_prose"],
    },
    // `A1-412`, the policy-side finding from the same audit. A comment after a
    // block key is YAML trivia; the semantic pass saw the id while the text pass
    // falsely reported that it could rewrite none. The existing command and
    // config key do not say which build accepts this representation.
    Behaviour {
        name: "rekey.policy_cut_block_comments",
        what: "`rekey` preserves an inline comment on a `policy[].when.cut` block key while \
               changing its exact milestone-id members",
        proof: &[
            "policy_cut_rekey_is_surgical_and_confined_to_when_cut",
            "rekey::a_milestone_rekeys_contains_and_policy_but_not_its_unchanged_label",
        ],
    },
    // `A1-413`, the atomic-publication finding from the audit. The command
    // already rolled plan state back after a late filesystem failure, but the
    // sibling whose backup failed had been removed from its cleanup guard and
    // left a hidden staging file behind. No command or format name distinguishes
    // builds with the complete cleanup guarantee.
    Behaviour {
        name: "rekey.relocation_late_failure_cleans_staging",
        what: "a late sibling-backup failure during `rekey` restores every original file and \
               removes every unpublished staging file",
        proof: &["a_late_relocation_failure_restores_every_file_and_removes_staging"],
    },
    // `A1-678`. The graph command, TUI destination and semantic explorer all
    // predate this geometry. An older build can advertise those surfaces while
    // still offering only row lanes, so none of their existing names tells an
    // embedding that a bounded spatial cell scene is available.
    Behaviour {
        name: "tui.layered_cell_scene_is_bounded_and_deterministic",
        what: "the multiscale semantic graph can be laid out as deterministic terminal-cell \
               nodes, phase bands and exact routed edges under explicit work limits, with typed \
               degradation, stable prior geometry and generation-safe fallback",
        proof: &[
            "layered_cell_scene_is_bounded_deterministic_and_kernel_independent",
            "cell_scene_refuses_ten_thousand_nodes_before_coordinate_work",
        ],
    },
    // `A1-679`. The TUI Graph destination and layered cell scene both predate
    // the live painter. An older build can therefore advertise each while
    // still formatting the compatibility lane scene as Paragraph rows.
    Behaviour {
        name: "tui.layered_graph_is_a_semantic_ratatui_widget",
        what: "the TUI Graph destination paints the bounded layered scene as adaptive phase bands, \
               semantic cards, ports and routed edges with cell-accurate hit targets, viewport \
               accounting and an honest selectable fallback",
        proof: &[
            "layered_graph_widget_paints_complete_semantic_scene",
            "layered_graph_presentation_is_reversible_without_relayout",
        ],
    },
    // `A1-697`. The layered graph widget name predates this selected-surface
    // correction. An older build can advertise the widget while styling only
    // the characters on a selected card and leaving reset-background holes in
    // the remainder of its semantic rows.
    Behaviour {
        name: "tui.selected_graph_card_is_one_continuous_surface",
        what: "a selected layered-graph task or aggregate card paints every visible interior cell with one coherent theme-selected style, including unused row suffixes and blank rows, without changing graph identity or hit geometry",
        proof: &["selected_graph_card_paints_one_continuous_surface"],
    },
    // `A1-680`. The semantic projection, cell scene and Ratatui painter all
    // predate geometry-driven controls. Older builds can advertise all three
    // while Enter still opens a flat inspector and arrows still follow a
    // formatted row order, so the interaction widening needs its own name.
    Behaviour {
        name: "tui.layered_graph_navigation_is_semantic_and_reversible",
        what: "the layered Graph destination navigates actual dependency geometry and direct relations, drills and reverses plan/phase/milestone/task levels with exact session and contextual-return history, preserves explicit order, direction, density and scope policies, and keeps bounded off-view traversal typed",
        proof: &[
            "layered_graph_navigation_is_semantic_and_reversible",
            "bounded_relation_cycle_reaches_every_omitted_direct_neighbor",
            "contextual_graph_entry_restores_the_saved_session_and_scope",
            "graph_jump_commands_preserve_scope_and_use_semantic_candidates",
            "graph_peer_order_modes_are_complete_visible_and_reversible",
            "graph_refresh_preserves_semantic_session_and_explicit_pins",
            "tui_journey::graph_explorer_runs_end_to_end_in_a_real_terminal",
            "tui_journey::graph_manual_pan_and_resize_run_end_to_end_in_a_real_terminal",
        ],
    },
    // `A1-682`. The semantic projection, bounded cell scene and both painters
    // all predate the adversarial corpus. A build could advertise each while a
    // collapsed route remained inside a nominally complete scene, retained
    // geometry from a failed relayout was painted as current, or the static
    // adapter silently omitted the same edge.
    Behaviour {
        name: "graph.layered_scene_is_complete_or_fallback",
        what: "a layered scene is paintable only when every admitted edge, non-empty phase band and semantic identity has complete validated current geometry; any failure becomes an explained selectable fallback, and retained geometry is recovery input rather than current paint or hit state",
        proof: &[
            "complete_scene_contract_rejects_degraded_geometry",
            "cell_scene_reuses_status_geometry_recentres_orientation_and_retains_fallback",
            "static_layered_scene_refuses_an_edge_with_missing_geometry",
            "retained_geometry_is_recovery_input_not_current_paint_or_hits",
        ],
    },
    // `A1-682`. The earlier graph behavior names describe individual layers,
    // not the executable adversarial matrix which binds them together across
    // orientations, hostile topology, aggregation and exhaustive small DAGs.
    Behaviour {
        name: "graph.layered_hardening_corpus_is_executable",
        what: "the shared layered graph runs one discoverable hostile corpus and every four-node DAG through deterministic cell geometry, proving exact edge provenance, phase containment, valid ports and routes, no overlap, typed feedback and total dependency accounting in both orientations",
        proof: &[
            "hostile_graph_corpus_preserves_every_scene_invariant",
            "every_four_node_dag_has_complete_deterministic_cell_geometry",
        ],
    },
    // `A1-688`. Complete orthogonal routes and the hostile corpus predate the
    // matched default gate. A build could satisfy both while the layered
    // preset's facing-port preference added more bends than the compatibility
    // renderer on the same graph.
    Behaviour {
        name: "graph.layered_routes_choose_low_bend_ports_within_crossing_budget",
        what: "layered routes may choose any valid node-boundary port when that reduces bends; on the matched crossing-heavy corpus both orientations retain exact monotone routes while producing no more unrelated-edge crossings than Lanes and at most 125 percent of its bends",
        proof: &["hostile_graph_corpus_preserves_every_scene_invariant"],
    },
    // `A1-681`. The text format, layered projection and terminal-cell scene
    // all predate this adapter. An older build can advertise each while the
    // command still exposes only the compatibility lane document.
    Behaviour {
        name: "graph.layered_text_uses_the_shared_semantic_scene",
        what: "`graph --format text --layout layered` paints the same typed multiscale projection and validated terminal-cell scene as the TUI under explicit level, orientation, peer-order, detail, width and glyph policies; redirected bytes are deterministic and every omission is accounted while lanes remains an explicit compatibility layout",
        proof: &[
            "graph_shapes::layered_text_is_default_and_lanes_remains_exact_compatibility",
            "graph_shapes::layered_levels_orientations_orders_and_detail_are_real_controls",
            "graph_shapes::layered_controls_refuse_wrong_formats_and_missing_redirected_width_before_loading",
            "graph_shapes::layered_width_refusal_is_exact_and_emits_no_clipped_picture",
            "color::layered_graph_text_obeys_colour_without_changing_plain_bytes",
        ],
    },
    // `A1-687`. Auto originally committed to the intrinsically preferred
    // application scene before the static painter knew its exact document
    // width. It could therefore refuse a left-to-right scene even when the
    // already validated top-to-bottom candidate fit.
    Behaviour {
        name: "graph.auto_text_orientation_prefers_a_scene_which_fits",
        what: "static layered text Auto orientation retains both validated direction candidates until the exact document width is known, chooses the best-quality complete scene which fits, and reports the smallest required width only when neither fits; explicitly forced directions and viewport-independent application/TUI Auto semantics are unchanged",
        proof: &[
            "graph_shapes::layered_auto_orientation_chooses_a_complete_scene_which_fits",
            "graph_shapes::layered_width_refusal_is_exact_and_emits_no_clipped_picture",
        ],
    },
    // `A1-683`. Layered text and Lanes both predate the layout default move.
    // Without a separate name an automation gate cannot distinguish which
    // human text document an omitted `--layout` selects.
    Behaviour {
        name: "graph.text_defaults_to_layered_with_explicit_lane_compatibility",
        what: "`graph --format text` selects the shared Layered scene and therefore requires an exact width when redirected; `--layout lanes` retains the prior row-and-lane bytes, and JSON, Mermaid and DOT bytes and accepted controls do not change",
        proof: &[
            "graph_shapes::layered_text_is_default_and_lanes_remains_exact_compatibility",
            "graph_shapes::layered_controls_refuse_wrong_formats_and_missing_redirected_width_before_loading",
        ],
    },
    // `A1-696`. The graph command and every format predate the attended entry
    // policy, so their facts cannot distinguish a build which hides spatial
    // text behind an expert flag from one which preserves redirected scripts
    // while opening that view for a person.
    Behaviour {
        name: "graph.attended_default_is_layered_and_redirected_default_is_mermaid",
        what: "an attended bare `graph` selects width-aware layered text, redirected bare `graph` retains deterministic Mermaid, and an explicit format remains authoritative in either environment",
        proof: &[
            "graph_shapes::attended_bare_graph_is_layered_while_explicit_mermaid_stays_mermaid",
            "graph_shapes::redirected_bare_graph_remains_mermaid",
        ],
    },
    // `A1-696`. Complete floating-point geometry can share an unrelated box
    // border which becomes an occupied terminal cell after quantization. The
    // prior completeness behavior only proved that aplan refused the whole
    // scene; it did not distinguish a build that safely repairs the route.
    Behaviour {
        name: "graph.cell_routes_repair_terminal_border_collisions",
        what: "terminal-cell adaptation deterministically diverts an orthogonal route around unrelated painted card cells while preserving endpoints, monotone direction and complete edge identity; geometry which cannot be repaired still becomes an explained fallback",
        proof: &["release_candidate_milestone_repairs_cell_border_routes_in_both_orientations"],
    },
    // `A1-681`, found while exercising the shared A1-678 scene through a
    // second painter. Top-to-bottom phase labels were anchored down the left
    // edge even though both painters write horizontal text, so the label
    // overwrote the border and first card. The existing scene behavior cannot
    // distinguish a build with the corrected label contract.
    Behaviour {
        name: "graph.layered_phase_labels_are_horizontal_and_non_occluding",
        what: "layered phase labels in either orientation are horizontal top-border labels, so a top-to-bottom scene never overwrites a band edge or node card",
        proof: &["graph_shapes::layered_levels_orientations_orders_and_detail_are_real_controls"],
    },
    // `A1-686`. Plan and phase levels originally ignored the semantic window
    // budget used by task detail, so the 68-card aplan overview reached the
    // 64-node cell-scene refusal before either client could traverse it.
    Behaviour {
        name: "graph.aggregate_levels_are_bounded_and_continuable",
        what: "plan and phase aggregate levels apply a source-bound semantic node and edge window before cell layout, preserve exact raw-task and dependency accounting, and expose continuations which both the TUI and static text can traverse",
        proof: &[
            "aggregate_graph_levels_are_bounded_before_cell_layout",
            "oversized_aggregate_graph_opens_a_layered_window",
            "graph_shapes::layered_static_surface_explains_phase_boundaries_windows_and_layout_limits",
        ],
    },
    // `A1-682`, found by reproducing A1-681's intermittent coverage journey
    // timeout. A five-second identity probe could begin before quit and the
    // observer ignored the worker's cancellation token, so the restored
    // terminal still waited for an entire repository observation to finish.
    Behaviour {
        name: "tui.external_observation_is_shutdown_cancellable",
        what: "periodic and focus-gain repository observations receive the worker cancellation token and yield at identified load boundaries, so graceful quit cannot wait for a newly in-flight observation after terminal restoration",
        proof: &["graceful_quit_cancels_an_in_flight_external_observation"],
    },
    // `A1-682`, found by repeatedly exercising the longest graph journey. On
    // Unix, Crossterm's edge-triggered mio source could return a SIGWINCH
    // before consuming terminal input reported in the same readiness batch.
    // The queued byte then produced no second edge, leaving the TUI asleep.
    Behaviour {
        name: "tui.simultaneous_resize_and_input_are_not_lost",
        what: "the Unix TUI polls terminal readiness level-triggered and consumes terminal input before a simultaneous resize notification, so an already queued key cannot be stranded after SIGWINCH",
        proof: &["tui_journey::simultaneous_resize_and_quit_never_strand_terminal_input"],
    },
    // `A1-682`, found by composing the 10,000-task graph-search and Overview
    // resize performance cases. Density changes rebuilt a hidden Graph
    // destination from complete source, turning an ordinary reflow into two
    // unrelated graph projections.
    Behaviour {
        name: "tui.hidden_graph_resize_defers_reprojection",
        what: "terminal resize updates the future graph density without reprojecting or laying out a hidden Graph destination; the bounded scene is rebuilt from current size only when Graph becomes visible again",
        proof: &["resize_defers_hidden_graph_reprojection_until_graph_reentry"],
    },
    // `A1-682`, found by running the release graph-paint gate against a scene
    // that honestly fell back. The painter allocated text for every source
    // node and only then discarded all but the visible viewport.
    Behaviour {
        name: "tui.graph_fallback_paint_is_viewport_bounded",
        what: "the graph fallback caches complete-source extent when the projection changes and translates the visible row interval directly into a source-node slice, so steady-state paint work and allocation are bounded by the viewport rather than complete graph size",
        proof: &[
            "fallback_paint_visits_only_the_visible_node_range",
            "tui_performance_gate_render_interaction_and_graph_work",
        ],
    },
    // `A1-682`, found after the fallback paint fix let the release gate reach
    // graph focus under shared-host load. Selection had rebuilt an otherwise
    // unchanged semantic projection from the complete source graph.
    Behaviour {
        name: "tui.graph_selection_reuses_current_projection",
        what: "selecting a subject represented at a selection-independent plan or phase graph level updates selection and source-bound transition subjects in place, then refocuses retained cell geometry without repeating complete-source semantic projection; bounded detail levels still reproject around their new anchor",
        proof: &["represented_graph_selection_reuses_projection_and_geometry"],
    },
    // `A1-682`, found once the release gate reached a 10,000-task status-only
    // refresh. Refresh rebuilt Tasks, Graph and Reports even though only one
    // destination was visible and every destination already reconciled on
    // entry.
    Behaviour {
        name: "tui.refresh_rebuilds_only_the_visible_projection",
        what: "refresh adopts the coherent shared snapshot and reconciles the visible destination plus shared work and inspector state; hidden Tasks, Graph and Reports projections defer source-sized work until route entry, where they rebuild from the adopted snapshot",
        proof: &[
            "refresh_defers_hidden_destination_projection_until_route_entry",
            "tui_performance_gate_render_interaction_and_graph_work",
        ],
    },
];
