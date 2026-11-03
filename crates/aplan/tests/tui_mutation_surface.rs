//! Closed-world parity audit for the native TUI mutation surface (`A1-443`).
//!
//! The workflow crates prove each typed request in depth. This file proves the
//! composition cannot drift: every native action drives one workflow, every
//! mutating CLI mode has an interactive disposition, and every action names the
//! state-contract tests which exercise acceptance and refusal boundaries.

#![cfg(feature = "tui")]

use std::collections::{BTreeMap, BTreeSet};

use aplan_tui::mutations::MutationAction;

#[derive(Debug, Clone, Copy)]
struct StateProof {
    accepted: &'static str,
    denied_or_refused: &'static str,
    stale: &'static str,
    noop_or_inapplicable: &'static str,
}

#[derive(Clone, Copy)]
struct Driver {
    action: MutationAction,
    drive: fn(MutationAction),
    states: StateProof,
}

fn drive_typed_workflow(action: MutationAction) {
    let families = [
        action.quick().is_some(),
        action.planning().is_some(),
        action.authoring().is_some(),
        action.evidence().is_some(),
        action.capture().is_some(),
    ];
    assert_eq!(
        families.into_iter().filter(|member| *member).count(),
        1,
        "{action:?} must drive exactly one typed workflow"
    );
    assert!(!action.title().trim().is_empty());
    assert!(!action.cli().trim().is_empty());
}

const CLAIM: StateProof = StateProof {
    accepted: "tui_work_action_parity::cli_and_tui_claim_start_stop_release_leave_the_same_state",
    denied_or_refused: "work_action::tests::malformed_requests_and_unavailable_coordination_fail_closed",
    stale: "work_action::tests::claim_and_release_preview_exact_records_and_detect_every_store_race",
    noop_or_inapplicable: "tui_work_action_parity::cli_and_typed_tui_claim_and_release_cover_every_status",
};

const RELEASE: StateProof = StateProof {
    accepted: CLAIM.accepted,
    denied_or_refused: "work_action::tests::release_recovers_an_orphan_but_refuses_an_unknown_id",
    stale: CLAIM.stale,
    noop_or_inapplicable: CLAIM.noop_or_inapplicable,
};

const STATUS: StateProof = StateProof {
    accepted: CLAIM.accepted,
    denied_or_refused: "work_action::tests::status_previews_match_cli_rules_and_publish_only_the_reviewed_bytes",
    stale: "work_action::tests::status_preview_retains_gate_findings_and_guarded_staleness",
    noop_or_inapplicable: "tui_work_action_parity::cli_and_typed_tui_start_and_stop_cover_every_status",
};

const EDGE: StateProof = StateProof {
    accepted: "tui_planning_parity::cli_and_typed_tui_planning_operations_publish_the_same_plan_bytes",
    denied_or_refused: "planning::tests::edge_previews_cover_readiness_preferences_and_every_structural_refusal",
    stale: "aplan_tui::form::tests::a_preview_for_another_input_is_inspectable_but_never_confirmable",
    noop_or_inapplicable: "planning::tests::no_op_and_inverse_edges_and_tags_remain_exact_and_reviewable",
};

const TAG: StateProof = StateProof {
    accepted: EDGE.accepted,
    denied_or_refused: "planning::tests::tag_previews_gate_families_and_review_broad_exact_changes",
    stale: EDGE.stale,
    noop_or_inapplicable: EDGE.noop_or_inapplicable,
};

const MOVE: StateProof = StateProof {
    accepted: EDGE.accepted,
    denied_or_refused: "planning::tests::phase_only_moves_and_direct_drops_cover_argument_and_terminal_boundaries",
    stale: EDGE.stale,
    noop_or_inapplicable: "inapplicable: a move must change cut placement or phase to be meaningful",
};

const DROP: StateProof = StateProof {
    accepted: EDGE.accepted,
    denied_or_refused: MOVE.denied_or_refused,
    stale: EDGE.stale,
    noop_or_inapplicable: "planning::tests::preference_removal_empty_cut_reclassification_and_bad_plans_fail_exactly",
};

const ADD: StateProof = StateProof {
    accepted: "tui_authoring_parity::every_authoring_action_matches_the_complete_cli_post_state",
    denied_or_refused: "authoring::tests::authoring_validation_refuses_each_malformed_scalar_and_relation",
    stale: "authoring::tests::an_id_race_is_stale_before_any_batch_member_is_published",
    noop_or_inapplicable: "inapplicable: structural creation has no idempotent no-op contract",
};

const MANIFEST: StateProof = StateProof {
    accepted: ADD.accepted,
    denied_or_refused: "authoring::tests::manifest_validation_is_entry_scoped_and_existing_cut_edits_are_previewed",
    stale: "aplan_tui::form::tests::a_preview_for_another_input_is_inspectable_but_never_confirmable",
    noop_or_inapplicable: ADD.noop_or_inapplicable,
};

const IMPORT: StateProof = StateProof {
    accepted: ADD.accepted,
    denied_or_refused: "authoring::tests::import_validation_covers_dispatch_provenance_prefix_and_cardinality",
    stale: MANIFEST.stale,
    noop_or_inapplicable: ADD.noop_or_inapplicable,
};

const SUPERSEDE: StateProof = StateProof {
    accepted: ADD.accepted,
    denied_or_refused: "authoring::tests::supersede_refuses_non_outcomes_and_supports_an_unplaced_explicit_successor",
    stale: MANIFEST.stale,
    noop_or_inapplicable: ADD.noop_or_inapplicable,
};

const REKEY: StateProof = StateProof {
    accepted: ADD.accepted,
    denied_or_refused: "authoring::tests::rekey_refuses_collisions_terminal_history_claims_and_stale_destinations",
    stale: MANIFEST.stale,
    noop_or_inapplicable: ADD.noop_or_inapplicable,
};

const CLOSE: StateProof = StateProof {
    accepted: "tui_evidence_parity::every_task_local_evidence_action_matches_the_complete_cli_post_state",
    denied_or_refused: "evidence_mutation::tests::close_refuses_ambiguous_tests_irrelevant_commits_and_terminal_tasks",
    stale: "evidence_mutation::tests::close_previews_policy_resolution_exact_bytes_and_stale_publication",
    noop_or_inapplicable: "inapplicable: closing an already terminal task is a refusal rather than an idempotent write",
};

const AMEND: StateProof = StateProof {
    accepted: "tui_evidence_parity::every_task_local_evidence_action_matches_the_complete_cli_post_state",
    denied_or_refused: "evidence_mutation::tests::amend_modes_keep_resolution_delta_and_minimum_strength_guards",
    stale: "evidence_mutation::tests::evidence_mutation_preview_refuses_repository_and_configuration_only_races",
    noop_or_inapplicable: "evidence_mutation::tests::amend_modes_keep_resolution_delta_and_minimum_strength_guards",
};

const RESTAMP: StateProof = StateProof {
    accepted: "tui_evidence_parity::every_task_local_evidence_action_matches_the_complete_cli_post_state",
    denied_or_refused: "evidence_mutation::tests::policy_projection_and_restamp_preconditions_are_exact",
    stale: "evidence_mutation::tests::evidence_mutation_preview_refuses_repository_and_configuration_only_races",
    noop_or_inapplicable: "evidence_mutation::tests::restamp_is_separate_and_a_current_baseline_is_a_no_op",
};

const VERIFY_FIX: StateProof = StateProof {
    accepted: "tui_evidence_parity::verification_fix_matches_the_complete_cli_post_state",
    denied_or_refused: "evidence_mutation::tests::verification_repair_operands_are_untrusted_at_the_application_boundary",
    stale: "evidence_mutation::tests::verification_fix_is_one_guarded_batch_with_no_repair_and_stale_controls",
    noop_or_inapplicable: "evidence_mutation::tests::verification_fix_is_one_guarded_batch_with_no_repair_and_stale_controls",
};

const CAPTURE_CREATE: StateProof = StateProof {
    accepted: "tui_capture_parity::native_capture_and_every_inbox_lifecycle_match_cli_post_state",
    denied_or_refused: "capture::tests::capture_input_refusals_name_the_policy_identity_and_configuration",
    stale: "capture::tests::source_uniqueness_is_rechecked_under_one_inbox_publication_lock",
    noop_or_inapplicable: "capture::tests::source_uniqueness_is_rechecked_under_one_inbox_publication_lock",
};

const CAPTURE_LIFECYCLE: StateProof = StateProof {
    accepted: "tui_capture_parity::native_capture_and_every_inbox_lifecycle_match_cli_post_state",
    denied_or_refused: "inbox::tests::lifecycle_forms_refuse_the_wrong_state_and_invalid_values",
    stale: "capture::tests::prefix_membership_changes_make_mutations_stale",
    noop_or_inapplicable: "inapplicable: lifecycle transitions refuse the wrong terminal state instead of publishing an idempotent write",
};

const CAPTURE_PROMOTE: StateProof = StateProof {
    accepted: "tui_capture_parity::native_capture_and_every_inbox_lifecycle_match_cli_post_state",
    denied_or_refused: "capture_inbox::capture_promotion_refusals_leave_task_and_capture_unchanged",
    stale: "capture::tests::concurrent_inbox_writer_makes_whole_promotion_stale",
    noop_or_inapplicable: "capture_inbox::capture_promotion_is_validated_atomic_and_idempotent",
};

const DRIVERS: &[Driver] = &[
    Driver {
        action: MutationAction::CaptureCreate,
        drive: drive_typed_workflow,
        states: CAPTURE_CREATE,
    },
    Driver {
        action: MutationAction::CaptureAmend,
        drive: drive_typed_workflow,
        states: CAPTURE_LIFECYCLE,
    },
    Driver {
        action: MutationAction::CaptureDiscard,
        drive: drive_typed_workflow,
        states: CAPTURE_LIFECYCLE,
    },
    Driver {
        action: MutationAction::CaptureRestore,
        drive: drive_typed_workflow,
        states: CAPTURE_LIFECYCLE,
    },
    Driver {
        action: MutationAction::CapturePromote,
        drive: drive_typed_workflow,
        states: CAPTURE_PROMOTE,
    },
    Driver {
        action: MutationAction::Claim,
        drive: drive_typed_workflow,
        states: CLAIM,
    },
    Driver {
        action: MutationAction::Release,
        drive: drive_typed_workflow,
        states: RELEASE,
    },
    Driver {
        action: MutationAction::Start,
        drive: drive_typed_workflow,
        states: STATUS,
    },
    Driver {
        action: MutationAction::Stop,
        drive: drive_typed_workflow,
        states: STATUS,
    },
    Driver {
        action: MutationAction::DependencyAdd,
        drive: drive_typed_workflow,
        states: EDGE,
    },
    Driver {
        action: MutationAction::DependencyRemove,
        drive: drive_typed_workflow,
        states: EDGE,
    },
    Driver {
        action: MutationAction::PreferenceAdd,
        drive: drive_typed_workflow,
        states: EDGE,
    },
    Driver {
        action: MutationAction::PreferenceRemove,
        drive: drive_typed_workflow,
        states: EDGE,
    },
    Driver {
        action: MutationAction::TagAdd,
        drive: drive_typed_workflow,
        states: TAG,
    },
    Driver {
        action: MutationAction::TagRemove,
        drive: drive_typed_workflow,
        states: TAG,
    },
    Driver {
        action: MutationAction::TagRename,
        drive: drive_typed_workflow,
        states: TAG,
    },
    Driver {
        action: MutationAction::Move,
        drive: drive_typed_workflow,
        states: MOVE,
    },
    Driver {
        action: MutationAction::DropTasks,
        drive: drive_typed_workflow,
        states: DROP,
    },
    Driver {
        action: MutationAction::DropCuts,
        drive: drive_typed_workflow,
        states: DROP,
    },
    Driver {
        action: MutationAction::Add,
        drive: drive_typed_workflow,
        states: ADD,
    },
    Driver {
        action: MutationAction::Manifest,
        drive: drive_typed_workflow,
        states: MANIFEST,
    },
    Driver {
        action: MutationAction::Import,
        drive: drive_typed_workflow,
        states: IMPORT,
    },
    Driver {
        action: MutationAction::Supersede,
        drive: drive_typed_workflow,
        states: SUPERSEDE,
    },
    Driver {
        action: MutationAction::Rekey,
        drive: drive_typed_workflow,
        states: REKEY,
    },
    Driver {
        action: MutationAction::Close,
        drive: drive_typed_workflow,
        states: CLOSE,
    },
    Driver {
        action: MutationAction::AmendAdd,
        drive: drive_typed_workflow,
        states: AMEND,
    },
    Driver {
        action: MutationAction::AmendReplace,
        drive: drive_typed_workflow,
        states: AMEND,
    },
    Driver {
        action: MutationAction::AmendRemove,
        drive: drive_typed_workflow,
        states: AMEND,
    },
    Driver {
        action: MutationAction::Restamp,
        drive: drive_typed_workflow,
        states: RESTAMP,
    },
    Driver {
        action: MutationAction::VerifyFix,
        drive: drive_typed_workflow,
        states: VERIFY_FIX,
    },
];

const PROOF_SOURCES: &[&str] = &[
    include_str!("../../aplan-tui/src/quick_actions.rs"),
    include_str!("../../aplan-tui/src/planning.rs"),
    include_str!("../../aplan-tui/src/authoring.rs"),
    include_str!("../../aplan-tui/src/evidence.rs"),
    include_str!("../../aplan-tui/src/inbox.rs"),
    include_str!("../../aplan-tui/src/form.rs"),
    include_str!("tui_work_action_parity.rs"),
    include_str!("tui_planning_parity.rs"),
    include_str!("tui_authoring_parity.rs"),
    include_str!("tui_evidence_parity.rs"),
    include_str!("tui_capture_parity.rs"),
    include_str!("capture_inbox.rs"),
    include_str!("../../aplan-app/src/capture.rs"),
    include_str!("../../aplan-app/src/authoring.rs"),
    include_str!("../../aplan-app/src/evidence_mutation.rs"),
    include_str!("../../aplan-app/src/planning.rs"),
    include_str!("../../aplan-app/src/work_action.rs"),
];

fn proof_exists(proof: &str) -> bool {
    if proof.starts_with("inapplicable:") {
        return true;
    }
    let Some(name) = proof.rsplit("::").next() else {
        return false;
    };
    let declaration = format!("fn {name}(");
    let matches = PROOF_SOURCES
        .iter()
        .filter_map(|source| source.find(&declaration).map(|offset| (*source, offset)))
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return false;
    }
    let (source, offset) = matches[0];
    source[..offset]
        .lines()
        .rev()
        .map(str::trim)
        .take_while(|line| line.is_empty() || line.starts_with('#') || line.starts_with("///"))
        .any(|line| line == "#[test]")
}

fn audit_drivers(drivers: &[Driver]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for driver in drivers {
        if !seen.insert(driver.action) {
            return Err(format!("duplicate parity driver for {:?}", driver.action));
        }
        (driver.drive)(driver.action);
        for (state, proof) in [
            ("accepted", driver.states.accepted),
            ("denied/refused", driver.states.denied_or_refused),
            ("stale", driver.states.stale),
            ("no-op/inapplicable", driver.states.noop_or_inapplicable),
        ] {
            if proof.trim().is_empty() {
                return Err(format!("{:?} has no {state} proof", driver.action));
            }
            if !proof_exists(proof) {
                return Err(format!(
                    "{:?} names missing {state} proof `{proof}`",
                    driver.action
                ));
            }
        }
    }
    let expected = MutationAction::ALL.into_iter().collect::<BTreeSet<_>>();
    if seen != expected {
        let missing = expected.difference(&seen).copied().collect::<Vec<_>>();
        let extra = seen.difference(&expected).copied().collect::<Vec<_>>();
        return Err(format!(
            "mutation parity drivers drifted; missing {missing:?}; extra {extra:?}"
        ));
    }
    Ok(())
}

#[test]
/// Proof for `tui.mutation_surface_has_cross_adapter_parity`: the table is the
/// exact typed action union and every row drives its workflow classification.
fn every_native_mutation_drives_parity_and_each_contract_state() {
    audit_drivers(DRIVERS).expect("complete mutation parity table");
}

#[test]
/// Proof for `tui.mutation_surface_has_cross_adapter_parity`: removing any one
/// row makes the otherwise complete closed-world audit name that omission.
fn deleting_one_driver_makes_the_closed_world_audit_fail() {
    for (omitted, omitted_driver) in DRIVERS.iter().enumerate() {
        let drivers = DRIVERS
            .iter()
            .enumerate()
            .filter_map(|(index, driver)| (index != omitted).then_some(*driver))
            .collect::<Vec<_>>();
        let error = audit_drivers(&drivers).expect_err("an omitted driver must be visible");
        assert!(
            error.contains(&format!("{:?}", omitted_driver.action)),
            "{error}"
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModeDisposition {
    Native,
    Session,
    Preview,
    OutputOnly,
    Unsupported,
}

#[derive(Debug)]
struct CliMapping {
    path: &'static str,
    actions: &'static [MutationAction],
    modes: &'static [(&'static str, ModeDisposition)],
}

const CLI: &[CliMapping] = &[
    CliMapping {
        path: "capture",
        actions: &[MutationAction::CaptureCreate],
        modes: &[
            ("title", ModeDisposition::Native),
            ("kind", ModeDisposition::Native),
            ("label", ModeDisposition::Native),
            ("profile", ModeDisposition::Native),
            ("source", ModeDisposition::Unsupported),
            ("body", ModeDisposition::Native),
            ("body_file", ModeDisposition::Unsupported),
            ("edit", ModeDisposition::Unsupported),
            ("revisit_at", ModeDisposition::Native),
            ("dry_run", ModeDisposition::Preview),
            ("format", ModeDisposition::OutputOnly),
            ("phase", ModeDisposition::Unsupported),
            ("cut", ModeDisposition::Unsupported),
            ("effort", ModeDisposition::Unsupported),
            ("owner", ModeDisposition::Unsupported),
            ("paths", ModeDisposition::Unsupported),
            ("tags", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "inbox amend",
        actions: &[MutationAction::CaptureAmend],
        modes: &[
            ("id", ModeDisposition::Native),
            ("title", ModeDisposition::Native),
            ("kind", ModeDisposition::Native),
            ("body", ModeDisposition::Native),
            ("body_file", ModeDisposition::Unsupported),
            ("edit", ModeDisposition::Unsupported),
            ("clear_body", ModeDisposition::Native),
            ("label", ModeDisposition::Native),
            ("clear_labels", ModeDisposition::Native),
            ("revisit_at", ModeDisposition::Native),
            ("clear_revisit", ModeDisposition::Native),
            ("clear_promotion", ModeDisposition::Native),
            ("dry_run", ModeDisposition::Preview),
            ("phase", ModeDisposition::Native),
            ("cut", ModeDisposition::Native),
            ("effort", ModeDisposition::Native),
            ("owner", ModeDisposition::Native),
            ("paths", ModeDisposition::Native),
            ("tags", ModeDisposition::Native),
        ],
    },
    CliMapping {
        path: "inbox discard",
        actions: &[MutationAction::CaptureDiscard],
        modes: &[
            ("id", ModeDisposition::Native),
            ("reason", ModeDisposition::Native),
            ("dry_run", ModeDisposition::Preview),
        ],
    },
    CliMapping {
        path: "inbox restore",
        actions: &[MutationAction::CaptureRestore],
        modes: &[
            ("id", ModeDisposition::Native),
            ("dry_run", ModeDisposition::Preview),
        ],
    },
    CliMapping {
        path: "inbox promote",
        actions: &[MutationAction::CapturePromote],
        modes: &[
            ("id", ModeDisposition::Native),
            ("task_id", ModeDisposition::Native),
            ("phase", ModeDisposition::Native),
            ("cut", ModeDisposition::Native),
            ("effort", ModeDisposition::Native),
            ("owner", ModeDisposition::Native),
            ("paths", ModeDisposition::Native),
            ("clear_paths", ModeDisposition::Native),
            ("tags", ModeDisposition::Native),
            ("clear_tags", ModeDisposition::Native),
            ("reason", ModeDisposition::Native),
            ("dry_run", ModeDisposition::Preview),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "claim",
        actions: &[MutationAction::Claim],
        modes: &[
            ("id", ModeDisposition::Native),
            ("as_name", ModeDisposition::Native),
            ("from", ModeDisposition::Native),
        ],
    },
    CliMapping {
        path: "release",
        actions: &[MutationAction::Release],
        modes: &[("id", ModeDisposition::Native)],
    },
    CliMapping {
        path: "start",
        actions: &[MutationAction::Start],
        modes: &[
            ("id", ModeDisposition::Native),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "stop",
        actions: &[MutationAction::Stop],
        modes: &[
            ("id", ModeDisposition::Native),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "dep add",
        actions: &[MutationAction::DependencyAdd, MutationAction::PreferenceAdd],
        modes: &[
            ("id", ModeDisposition::Native),
            ("after", ModeDisposition::Native),
            ("prefer", ModeDisposition::Native),
            ("dry_run", ModeDisposition::Preview),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "dep rm",
        actions: &[
            MutationAction::DependencyRemove,
            MutationAction::PreferenceRemove,
        ],
        modes: &[
            ("id", ModeDisposition::Native),
            ("after", ModeDisposition::Native),
            ("prefer", ModeDisposition::Native),
            ("dry_run", ModeDisposition::Preview),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "dep apply",
        actions: &[
            MutationAction::DependencyAdd,
            MutationAction::DependencyRemove,
        ],
        modes: &[
            ("from", ModeDisposition::Unsupported),
            ("dry_run", ModeDisposition::Preview),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "tag add",
        actions: &[MutationAction::TagAdd],
        modes: &[
            ("id", ModeDisposition::Native),
            ("tag", ModeDisposition::Native),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "tag rm",
        actions: &[MutationAction::TagRemove],
        modes: &[
            ("id", ModeDisposition::Native),
            ("tag", ModeDisposition::Native),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "tag rename",
        actions: &[MutationAction::TagRename],
        modes: &[
            ("from", ModeDisposition::Native),
            ("to", ModeDisposition::Native),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "mv",
        actions: &[MutationAction::Move],
        modes: &[
            ("ids", ModeDisposition::Native),
            ("cut", ModeDisposition::Native),
            ("phase", ModeDisposition::Native),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "drop",
        actions: &[MutationAction::DropTasks, MutationAction::DropCuts],
        modes: &[
            ("ids", ModeDisposition::Native),
            ("cut", ModeDisposition::Native),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "add",
        actions: &[MutationAction::Add, MutationAction::Manifest],
        modes: &[
            ("title", ModeDisposition::Native),
            ("from", ModeDisposition::Native),
            ("id", ModeDisposition::Native),
            ("phase", ModeDisposition::Native),
            ("blocked_by", ModeDisposition::Native),
            ("status", ModeDisposition::Native),
            ("prefer_after", ModeDisposition::Native),
            ("ratified_by", ModeDisposition::Native),
            ("contains", ModeDisposition::Native),
            ("members", ModeDisposition::Native),
            ("tags", ModeDisposition::Native),
            ("effort", ModeDisposition::Native),
            ("owner", ModeDisposition::Native),
            ("revisit_at", ModeDisposition::Native),
            ("version_impact", ModeDisposition::Native),
            ("version_impact_reason", ModeDisposition::Native),
            ("paths", ModeDisposition::Native),
            ("traces", ModeDisposition::Native),
            ("cut", ModeDisposition::Native),
            ("out", ModeDisposition::Unsupported),
            ("dry_run", ModeDisposition::Preview),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "import",
        actions: &[MutationAction::Import],
        modes: &[
            ("source", ModeDisposition::Native),
            ("format", ModeDisposition::Native),
            ("out", ModeDisposition::Unsupported),
            ("dry_run", ModeDisposition::Preview),
            ("phase", ModeDisposition::Native),
            ("implemented_as", ModeDisposition::Native),
            ("as_of", ModeDisposition::Native),
            ("cut", ModeDisposition::Native),
            ("id_prefix", ModeDisposition::Native),
            ("tags", ModeDisposition::Native),
        ],
    },
    CliMapping {
        path: "supersede",
        actions: &[MutationAction::Supersede],
        modes: &[
            ("id", ModeDisposition::Native),
            ("title", ModeDisposition::Native),
            ("cut", ModeDisposition::Native),
            ("effort", ModeDisposition::Native),
            ("owner", ModeDisposition::Native),
            ("tags", ModeDisposition::Native),
            ("paths", ModeDisposition::Native),
            ("clear_paths", ModeDisposition::Native),
            ("traces", ModeDisposition::Native),
            ("clear_traces", ModeDisposition::Native),
            ("body", ModeDisposition::Native),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "rekey",
        actions: &[MutationAction::Rekey],
        modes: &[
            ("old", ModeDisposition::Native),
            ("new", ModeDisposition::Native),
            ("allow_terminal", ModeDisposition::Native),
            ("format", ModeDisposition::OutputOnly),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "close",
        actions: &[MutationAction::Close, MutationAction::Restamp],
        modes: &[
            ("id", ModeDisposition::Native),
            ("evidence", ModeDisposition::Native),
            ("repo", ModeDisposition::Session),
            ("restamp", ModeDisposition::Native),
            ("no_resolve", ModeDisposition::Native),
            ("execute", ModeDisposition::Native),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "amend",
        actions: &[
            MutationAction::AmendAdd,
            MutationAction::AmendReplace,
            MutationAction::AmendRemove,
        ],
        modes: &[
            ("id", ModeDisposition::Native),
            ("replace_evidence", ModeDisposition::Native),
            ("add_evidence", ModeDisposition::Native),
            ("remove_evidence", ModeDisposition::Native),
            ("repo", ModeDisposition::Session),
            ("no_resolve", ModeDisposition::Native),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
    CliMapping {
        path: "verify",
        actions: &[MutationAction::VerifyFix],
        modes: &[
            ("cut", ModeDisposition::Unsupported),
            ("phase", ModeDisposition::Unsupported),
            ("tag", ModeDisposition::Unsupported),
            ("owner", ModeDisposition::Unsupported),
            ("unclaimed", ModeDisposition::Unsupported),
            ("status", ModeDisposition::Unsupported),
            ("format", ModeDisposition::OutputOnly),
            ("repo", ModeDisposition::Session),
            ("no_require_effort", ModeDisposition::Unsupported),
            ("traces_root", ModeDisposition::Unsupported),
            ("no_traces", ModeDisposition::Unsupported),
            ("execute", ModeDisposition::Native),
            ("test_receipts", ModeDisposition::Unsupported),
            ("refresh_test_receipts", ModeDisposition::Unsupported),
            ("refresh_base", ModeDisposition::Unsupported),
            ("refresh_head", ModeDisposition::Unsupported),
            ("refresh_working_tree", ModeDisposition::Unsupported),
            ("refresh_shadow", ModeDisposition::Unsupported),
            ("fix", ModeDisposition::Native),
            ("deny_warnings", ModeDisposition::Unsupported),
            ("no_deny_warnings", ModeDisposition::Unsupported),
        ],
    },
];

const GLOBAL: &[(&str, ModeDisposition)] = &[
    ("root", ModeDisposition::Session),
    ("color", ModeDisposition::OutputOnly),
    ("progress", ModeDisposition::OutputOnly),
    ("config", ModeDisposition::Session),
    ("no_config", ModeDisposition::Session),
    ("timings", ModeDisposition::OutputOnly),
    ("cache", ModeDisposition::Session),
];

/// CLI leaves which do not mutate task/graph state. Some write other explicit
/// artifacts (for example intake records, `init`, exports, or an attestation
/// note); they are outside the task-mutation catalogue rather than silently
/// mistaken for a native task action.
const NON_TASK_MUTATION_LEAVES: &[&str] = &[
    "attest",
    "check",
    "check-docs",
    "critical-path",
    "deprecations",
    "due",
    "explain",
    "export",
    "export-agents-md",
    "export-roadmap-json",
    "facts",
    "graph",
    "inbox show",
    "init",
    "mcp",
    "paths",
    "ready",
    "roadmap",
    "schema",
    "show",
    "stale",
    "summary",
    "tasks",
    "test list",
    "test receipt",
    "tree",
    "tui",
    "vacuity",
    "version",
    "why",
];

/// Task mutations deliberately kept at an explicit, user-visible CLI boundary.
/// These are separate from [`NON_TASK_MUTATION_LEAVES`]: classifying a plan
/// write as a read-only surface would make this audit pass for the wrong reason.
const CLI_ONLY_TASK_MUTATION_LEAVES: &[&str] = &["blueprint apply", "park", "unpark"];

fn leaf_paths(command: &clap::Command, prefix: &str, paths: &mut BTreeSet<String>) {
    let children = command
        .get_subcommands()
        .filter(|child| child.get_name() != "help")
        .collect::<Vec<_>>();
    if children.is_empty() {
        if !prefix.is_empty() {
            paths.insert(prefix.to_string());
        }
        return;
    }
    for child in children {
        let path = if prefix.is_empty() {
            child.get_name().to_string()
        } else {
            format!("{prefix} {}", child.get_name())
        };
        leaf_paths(child, &path, paths);
    }
}

fn command_at<'a>(root: &'a clap::Command, path: &str) -> &'a clap::Command {
    path.split_whitespace().fold(root, |command, name| {
        command
            .get_subcommands()
            .find(|candidate| candidate.get_name() == name)
            .unwrap_or_else(|| panic!("CLI command path `{path}` lost `{name}`"))
    })
}

#[test]
/// Proof for `tui.mutation_surface_has_cross_adapter_parity`: clap is the live
/// CLI inventory, so a new behavior-bearing argument needs a disposition.
fn every_mutating_cli_mode_is_native_or_has_an_explicit_boundary() {
    let root = aplan::cli_command();
    let mut actual_leaves = BTreeSet::new();
    leaf_paths(&root, "", &mut actual_leaves);
    let classified_leaves = CLI
        .iter()
        .map(|mapping| mapping.path.to_string())
        .chain(
            NON_TASK_MUTATION_LEAVES
                .iter()
                .map(|path| (*path).to_string()),
        )
        .chain(
            CLI_ONLY_TASK_MUTATION_LEAVES
                .iter()
                .map(|path| (*path).to_string()),
        )
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual_leaves, classified_leaves,
        "every CLI leaf must be classified as a task mutation or an explicit non-task surface"
    );
    for path in CLI_ONLY_TASK_MUTATION_LEAVES {
        let top_level = path
            .split_whitespace()
            .next()
            .expect("a CLI leaf has a name");
        let command = aplan_tui::discovery::command(top_level)
            .unwrap_or_else(|| panic!("`{path}` has no user-visible TUI boundary"));
        let Some((reason, example)) = command.disposition.boundary() else {
            panic!("`{path}` is classified CLI-only here but native in TUI discovery");
        };
        assert!(reason.len() > 80, "`{path}` has no useful boundary reason");
        assert!(
            example.starts_with(&format!("aplan {path}")),
            "`{path}` boundary does not show its command: {example}"
        );
        assert!(command.mutations.is_empty());
    }
    let global = GLOBAL
        .iter()
        .map(|(name, _)| *name)
        .collect::<BTreeSet<_>>();
    let mut mapped_actions = BTreeSet::new();
    for mapping in CLI {
        mapped_actions.extend(mapping.actions.iter().copied());
        let command = command_at(&root, mapping.path);
        let actual = command
            .get_arguments()
            .map(|argument| argument.get_id().as_str())
            .filter(|id| *id != "help" && !global.contains(id))
            .collect::<BTreeSet<_>>();
        let expected = mapping
            .modes
            .iter()
            .map(|(name, _)| *name)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            actual, expected,
            "unclassified CLI mode on `{}`",
            mapping.path
        );

        let mut unique = BTreeMap::new();
        for (name, disposition) in mapping.modes {
            assert!(
                unique.insert(*name, *disposition).is_none(),
                "duplicate `{name}`"
            );
            if *disposition == ModeDisposition::Unsupported {
                let spelling = command
                    .get_arguments()
                    .find(|argument| argument.get_id().as_str() == *name)
                    .and_then(clap::Arg::get_long)
                    .map_or_else(
                        || format!("--{}", name.replace('_', "-")),
                        |long| format!("--{long}"),
                    );
                assert!(
                    mapping.actions.iter().all(|action| action
                        .cli_only_help()
                        .iter()
                        .any(|guidance| guidance.contains(&spelling))),
                    "`{}` mode `{spelling}` needs exact user-visible boundary guidance for {:?}",
                    mapping.path,
                    mapping.actions,
                );
            }
        }
    }
    assert_eq!(mapped_actions, MutationAction::ALL.into_iter().collect());

    let actual_globals = root
        .get_arguments()
        .map(|argument| argument.get_id().as_str())
        .filter(|id| *id != "help" && *id != "version")
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual_globals, global,
        "global CLI modes need TUI dispositions"
    );
}
