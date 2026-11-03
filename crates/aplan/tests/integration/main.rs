//! The single integration-test harness for this crate (`A1-630`).
//!
//! Keep the source modules in `tests/`: task paths and trace references name
//! those files directly. The integration-target census checks that this list
//! and the files on disk remain equal.

// The historical suite is Unix-only. Keep each existing declaration at the
// crate root (and therefore preserve every registered evidence citation), but
// platform-gate it. `common` itself is portable so the shared three-OS module
// below still crosses the same recorded, immutable process boundary.
#[path = "../common/mod.rs"]
mod common;

#[cfg(unix)]
#[path = "../add.rs"]
mod add;
#[cfg(unix)]
#[path = "../add_from.rs"]
mod add_from;
#[cfg(unix)]
#[path = "../advisory.rs"]
mod advisory;
#[cfg(unix)]
#[path = "../agent_contract_drift.rs"]
mod agent_contract_drift;
#[cfg(unix)]
#[path = "../agents_md.rs"]
mod agents_md;
#[cfg(unix)]
#[path = "../amend.rs"]
mod amend;
#[cfg(unix)]
#[path = "../as_of.rs"]
mod as_of;
#[cfg(unix)]
#[path = "../atomic_writes.rs"]
mod atomic_writes;
#[cfg(unix)]
#[path = "../attest.rs"]
mod attest;
#[cfg(unix)]
#[path = "../banner.rs"]
mod banner;
#[cfg(unix)]
#[path = "../behaviour_proof.rs"]
mod behaviour_proof;
#[cfg(unix)]
#[path = "../blueprint_application.rs"]
mod blueprint_application;
#[cfg(unix)]
#[path = "../blueprint_binding.rs"]
mod blueprint_binding;
#[cfg(unix)]
#[path = "../blueprint_examples.rs"]
mod blueprint_examples;
#[cfg(unix)]
#[path = "../blueprint_schema.rs"]
mod blueprint_schema;
#[cfg(unix)]
#[path = "../build_identity.rs"]
mod build_identity;
#[cfg(unix)]
#[path = "../bulk_body.rs"]
mod bulk_body;
#[cfg(unix)]
#[path = "../cache.rs"]
mod cache;
#[cfg(unix)]
#[path = "../capture_inbox.rs"]
mod capture_inbox;
#[cfg(unix)]
#[path = "../cascade.rs"]
mod cascade;
#[cfg(unix)]
#[path = "../check_docs_config.rs"]
mod check_docs_config;
#[cfg(unix)]
#[path = "../ci_runner_census.rs"]
mod ci_runner_census;
#[cfg(unix)]
#[path = "../ci_test_gate.rs"]
mod ci_test_gate;
#[cfg(unix)]
#[path = "../claim.rs"]
mod claim;
#[cfg(unix)]
#[path = "../cli.rs"]
mod cli;
#[cfg(unix)]
#[path = "../cli_termination.rs"]
mod cli_termination;
#[cfg(unix)]
#[path = "../close_advice.rs"]
mod close_advice;
#[cfg(unix)]
#[path = "../close_gate.rs"]
mod close_gate;
#[cfg(unix)]
#[path = "../color.rs"]
mod color;
#[cfg(unix)]
#[path = "../concurrent.rs"]
mod concurrent;
#[cfg(unix)]
#[path = "../config_error.rs"]
mod config_error;
#[cfg(unix)]
#[path = "../consumer_facing.rs"]
mod consumer_facing;
#[cfg(unix)]
#[path = "../criteria.rs"]
mod criteria;
#[cfg(unix)]
#[path = "../critical_path_to.rs"]
mod critical_path_to;
#[cfg(unix)]
#[path = "../dep.rs"]
mod dep;
#[cfg(unix)]
#[path = "../deprecations.rs"]
mod deprecations;
#[cfg(unix)]
#[path = "../docs.rs"]
mod docs;
#[cfg(unix)]
#[path = "../drift.rs"]
mod drift;
#[cfg(unix)]
#[path = "../due.rs"]
mod due;
#[cfg(unix)]
#[path = "../edge_kinds.rs"]
mod edge_kinds;
#[cfg(unix)]
#[path = "../empty_plan.rs"]
mod empty_plan;
#[cfg(unix)]
#[path = "../evidence_kinds.rs"]
mod evidence_kinds;
#[cfg(unix)]
#[path = "../evidence_observation.rs"]
mod evidence_observation;
#[cfg(unix)]
#[path = "../exit_codes.rs"]
mod exit_codes;
#[cfg(unix)]
#[path = "../explain.rs"]
mod explain;
#[cfg(unix)]
#[path = "../export.rs"]
mod export;
#[cfg(unix)]
#[path = "../export_roadmap_json.rs"]
mod export_roadmap_json;
#[cfg(unix)]
#[path = "../external_evidence.rs"]
mod external_evidence;
#[cfg(unix)]
#[path = "../facts.rs"]
mod facts;
#[cfg(unix)]
#[path = "../failed_test_output.rs"]
mod failed_test_output;
#[cfg(unix)]
#[path = "../finding_locations.rs"]
mod finding_locations;
#[cfg(unix)]
#[path = "../fixture_root.rs"]
mod fixture_root;
#[cfg(unix)]
#[path = "../format_surfaces.rs"]
mod format_surfaces;
#[cfg(unix)]
#[path = "../graph_shapes.rs"]
mod graph_shapes;
#[cfg(unix)]
#[path = "../greenfield.rs"]
mod greenfield;
#[cfg(unix)]
#[path = "../guarantees.rs"]
mod guarantees;
#[cfg(unix)]
#[path = "../help_wraps.rs"]
mod help_wraps;
#[cfg(unix)]
#[path = "../human_output_width.rs"]
mod human_output_width;
#[cfg(unix)]
#[path = "../id_race.rs"]
mod id_race;
#[cfg(unix)]
#[path = "../import_out.rs"]
mod import_out;
#[cfg(unix)]
#[path = "../import_tables.rs"]
mod import_tables;
#[cfg(unix)]
#[path = "../import_vocabulary.rs"]
mod import_vocabulary;
#[cfg(unix)]
#[path = "../inert_flags.rs"]
mod inert_flags;
#[cfg(unix)]
#[path = "../init.rs"]
mod init;
#[cfg(unix)]
#[path = "../ladder.rs"]
mod ladder;
#[cfg(unix)]
#[path = "../locality.rs"]
mod locality;
#[cfg(unix)]
#[path = "../machine_formats_unfolded.rs"]
mod machine_formats_unfolded;
#[cfg(unix)]
#[path = "../machine_readable.rs"]
mod machine_readable;
#[cfg(unix)]
#[path = "../mcp.rs"]
mod mcp;
#[cfg(unix)]
#[path = "../mcp_close_claims.rs"]
mod mcp_close_claims;
#[cfg(unix)]
#[path = "../mcp_parity.rs"]
mod mcp_parity;
#[cfg(unix)]
#[path = "../mcp_policy.rs"]
mod mcp_policy;
#[cfg(unix)]
#[path = "../mutate.rs"]
mod mutate;
#[cfg(unix)]
#[path = "../mutation_gate.rs"]
mod mutation_gate;
#[cfg(unix)]
#[path = "../mutation_policy.rs"]
mod mutation_policy;
#[cfg(unix)]
#[path = "../new_cut.rs"]
mod new_cut;
#[cfg(unix)]
#[path = "../no_effect_outside_the_plan.rs"]
mod no_effect_outside_the_plan;
#[cfg(unix)]
#[path = "../owner_vocabulary.rs"]
mod owner_vocabulary;
#[cfg(unix)]
#[path = "../path_spelling.rs"]
mod path_spelling;
#[cfg(unix)]
#[path = "../paths.rs"]
mod paths;
#[cfg(unix)]
#[path = "../phase_contract.rs"]
mod phase_contract;
#[cfg(unix)]
#[path = "../plan_validation_split.rs"]
mod plan_validation_split;
#[cfg(unix)]
#[path = "../policy.rs"]
mod policy;
#[cfg(unix)]
#[path = "../progress.rs"]
mod progress;
#[cfg(unix)]
#[path = "../publication_boundary.rs"]
mod publication_boundary;
#[cfg(unix)]
#[path = "../quoted_ids.rs"]
mod quoted_ids;
#[cfg(unix)]
#[path = "../reachability.rs"]
mod reachability;
#[cfg(unix)]
#[path = "../rekey.rs"]
mod rekey;
#[cfg(unix)]
#[path = "../repo_flag.rs"]
mod repo_flag;
#[cfg(unix)]
#[path = "../report_hub.rs"]
mod report_hub;
#[cfg(unix)]
#[path = "../revision.rs"]
mod revision;
#[cfg(unix)]
#[path = "../roadmap_header.rs"]
mod roadmap_header;
#[cfg(unix)]
#[path = "../run_cost.rs"]
mod run_cost;
#[cfg(unix)]
#[path = "../runners.rs"]
mod runners;
#[cfg(unix)]
#[path = "../scoped_policy.rs"]
mod scoped_policy;
#[cfg(unix)]
#[path = "../selection.rs"]
mod selection;
#[cfg(unix)]
#[path = "../show.rs"]
mod show;
#[cfg(unix)]
#[path = "../spawn_helper.rs"]
mod spawn_helper;
#[cfg(unix)]
#[path = "../spawn_recording.rs"]
mod spawn_recording;
#[cfg(unix)]
#[path = "../spec_kit_import.rs"]
mod spec_kit_import;
#[cfg(unix)]
#[path = "../stale.rs"]
mod stale;
#[cfg(unix)]
#[path = "../summary_group.rs"]
mod summary_group;
#[cfg(unix)]
#[path = "../supersede.rs"]
mod supersede;
#[cfg(unix)]
#[path = "../tag_cmd.rs"]
mod tag_cmd;
#[cfg(unix)]
#[path = "../tag_families.rs"]
mod tag_families;
#[cfg(unix)]
#[path = "../tasks_surface.rs"]
mod tasks_surface;
#[cfg(unix)]
#[path = "../terminal_integrity.rs"]
mod terminal_integrity;
#[cfg(unix)]
#[path = "../test_catalogue.rs"]
mod test_catalogue;
#[cfg(unix)]
#[path = "../timings.rs"]
mod timings;
#[cfg(unix)]
#[path = "../toolchain_contract.rs"]
mod toolchain_contract;
#[cfg(unix)]
#[path = "../traces.rs"]
mod traces;
#[cfg(unix)]
#[path = "../tree_status.rs"]
mod tree_status;
#[cfg(unix)]
#[path = "../trusted_config.rs"]
mod trusted_config;
#[cfg(unix)]
#[path = "../tui.rs"]
mod tui;
#[cfg(unix)]
#[path = "../tui_authoring_parity.rs"]
mod tui_authoring_parity;
#[cfg(unix)]
#[path = "../tui_capture_parity.rs"]
mod tui_capture_parity;
#[cfg(unix)]
#[path = "../tui_evidence_parity.rs"]
mod tui_evidence_parity;
#[cfg(unix)]
#[path = "../tui_journey.rs"]
mod tui_journey;
#[cfg(unix)]
#[path = "../tui_mutation_surface.rs"]
mod tui_mutation_surface;
#[cfg(unix)]
#[path = "../tui_performance.rs"]
mod tui_performance;
#[cfg(unix)]
#[path = "../tui_planning_parity.rs"]
mod tui_planning_parity;
#[cfg(unix)]
#[path = "../tui_read_surface.rs"]
mod tui_read_surface;
#[cfg(unix)]
#[path = "../tui_work_action_parity.rs"]
mod tui_work_action_parity;
#[cfg(unix)]
#[path = "../unopened.rs"]
mod unopened;
#[cfg(unix)]
#[path = "../vacuity.rs"]
mod vacuity;
#[cfg(unix)]
#[path = "../verify_policy.rs"]
mod verify_policy;
#[cfg(unix)]
#[path = "../version_next.rs"]
mod version_next;
#[cfg(unix)]
#[path = "../version_record.rs"]
mod version_record;
#[cfg(unix)]
#[path = "../warning_tier.rs"]
mod warning_tier;
#[cfg(unix)]
#[path = "../why_reach.rs"]
mod why_reach;

#[path = "../tui_portability.rs"]
mod tui_portability;
