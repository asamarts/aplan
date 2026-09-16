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
            "a_release_is_refused_as_a_cut_on_every_command_that_selects",
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
            "an_edit_to_a_traced_row_re_flags_exactly_its_owner",
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
            "a_denied_warning_does_not_summarise_as_ok",
            "the_summary_agrees_with_the_exit_code_and_names_what_was_downgraded",
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
            "the_explain_reasons_are_fields_not_a_sentence",
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
            "a_dead_sibling_glob_does_not_refuse_a_bound_citation",
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
            "commands_other_than_add_refuse_a_post_state_check_would_reject",
            "add_refuses_before_writing_what_check_would_reject_after",
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
            "a_tasks_bad_glob_does_not_hide_another_tasks_finding",
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
            "a_preference_orders_the_offer_and_blocks_nothing",
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
            "a_cut_emptied_of_work_is_reported_rather_than_refused",
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
            "every_cut_taking_command_accepts_the_label",
            "a_cut_is_named_by_its_id_or_by_its_label",
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
            "a_named_config_is_the_only_one_consulted",
            "no_config_runs_nothing_and_says_so",
            "a_named_config_that_is_missing_is_an_error_not_a_fallback",
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
            "one_unresolvable_citation_refuses_the_whole_closure_on_both_surfaces",
            "no_resolve_records_it_and_says_that_it_did",
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
            "a_repository_with_no_history_reports_no_measurement",
            "a_path_that_does_not_compile_is_not_reported_as_zero",
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
            "the_summary_says_how_many_citations_nothing_opened",
            "the_json_report_carries_the_count_in_both_directions",
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
            "a_later_entry_sees_what_an_earlier_one_created",
            "a_batch_that_cannot_finish_writes_none_of_it",
        ],
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
            "a_path_spelled_any_way_resolves_in_both_fields",
            "an_oddly_spelled_citation_is_judged_against_paths_the_same_way",
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
            "the_identity_needs_no_plan_and_no_repository",
            "a_dirty_tree_is_answered_here_and_refused_by_attest",
            "the_binary_digest_is_this_binarys_own",
        ],
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
            "no_human_surface_lets_a_plan_write_to_the_terminal",
            "a_machine_format_keeps_the_bytes_the_plan_holds",
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
            "a_short_begin_marker_is_recognised_and_written_out_in_full",
            "a_marker_named_in_prose_is_not_a_marker",
        ],
    },
    Behaviour {
        name: "color.explicitly_selectable",
        what: "`--color=auto|always|never` overrides the terminal detection in both directions, \
               reaching everything the tool prints -- its findings, its refusals, and `clap`'s own \
               usage errors -- while the machine formats stay byte-identical at every setting",
        proof: &[
            "colour_can_be_forced_into_a_pipe",
            "colour_can_be_refused_at_a_terminal",
            "an_explicit_choice_beats_the_environment",
            "the_refusal_is_on_the_same_stream_as_a_finding",
            "claps_own_error_obeys_the_choice",
            "the_machine_formats_carry_no_colour_when_it_is_forced",
        ],
    },
    Behaviour {
        name: "why.cut_bounded_by_default",
        what: "`aplan why` reports an unfinished cut as itself rather than walking into its \
               members, and names the traversal that produced every answer; `--all` asks for \
               the complete closure",
        proof: &[
            "a_late_task_names_its_own_blockers_and_the_cut_in_front_of_it",
            "a_chain_of_ordinary_tasks_is_walked_whole",
            "the_json_names_the_traversal_and_what_was_folded",
        ],
    },
];
