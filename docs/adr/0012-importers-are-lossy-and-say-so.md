# ADR-0012: An importer is lossy, and says so

- Status: Accepted
- Date: 2026-07-29
- Deciders: kaminsod

## Context

`aplan import` (HLD section 13) reads external planning artifacts, including RFC and proposal
metadata such as a Kubernetes `kep.yaml` (`A1-019`). These formats commonly distinguish an
accepted design from an implemented result. That distinction maps to the existing task model:
a `todo` task can represent accepted but unfinished work, while a tag can preserve the source
artifact's lifecycle stage.

Three existing commitments constrain how an importer may behave, and they have never been
written down together:

- **HLD section 13** already requires that import be honest about loss: "a lossy import that
  silently drops a field is precisely the drift this tool exists to detect. Emit a diagnostic
  per dropped field."
- **ADR-0008** names `Importer` as one of `aplan-core`'s three sanctioned extension points and
  requires new concepts to use the existing task model unless that model cannot express them.
- **ADR-0001** is the thesis itself: a status is a falsifiable claim, not a field. Evidence
  strength is enforced (`APLAN017`, `evidence.require_min_strength`), and this repository's own
  policy excludes `note`, so a `done` carrying only a note is a hard `check` error here.

Without a recorded policy, the first importer sets precedent by accident for every importer
after it. The specific hazard is the third point: a source that says "implemented" is making a
claim, and an importer that transcribes that claim into `status: done` would be laundering an
unverified assertion into `aplan`'s strongest one, on import, at scale. That is the failure this
tool exists to refuse.

## Decision

1. **No new `Status` variant for pipeline stages.** A `todo` task already expresses
   accepted-but-not-implemented. Adding `accepted`, `published`, or `implementable` to the
   status enum would be the over-modelling ADR-0008 refuses, and would fragment every existing
   rule that reasons about status.

2. **A proposal's stage is a tag, never a status.** A KEP's `implementable` becomes the tag
   `kep:implementable` on a `todo` task; the maturity stage (alpha, beta, stable) becomes a
   second tag. Tags are free-form and carry no verdict, which is the honest place for a
   vocabulary `aplan` does not validate. The importing repository may then group imported tasks
   by tag or by cut, which is a planning decision, not an import one.

3. **Every field the task model does not hold produces a diagnostic.** Silent omission is
   forbidden (HLD section 13). The KEP template carries seventeen top-level fields, of which
   this mapping holds six (`title`, `kep-number`, `authors`, `owning-sig`, `status`, `stage`);
   the other eleven (`participating-sigs`, `creation-date`, `reviewers`, `approvers`,
   `see-also`, `replaces`, `latest-milestone`, `milestone`, `feature-gates`,
   `disable-supported`, `metrics`) are dropped, and each is reported, per source file, so the
   user sees precisely what did not survive. Import diagnostics are a report, not `APLAN0xx`
   rule codes: they describe a conversion, not a violated invariant in a plan.

4. **An importer never asserts a `done` it cannot prove.** A source's "implemented" is imported
   as `todo` plus the stage tag, and the report says so and names the fix (`aplan close <id>
   --evidence commit:<sha>`). Emitting `done` is opt-in
   (`--implemented-as done`), and then carries only a dated `note`, the weakest kind, with a
   warning that the provenance is note-level. The default therefore produces a plan that passes
   `check` under *any* evidence policy, including a strict `require_min_strength` that excludes
   `note`. The opt-in path is weaker on purpose: its dated note satisfies `APLAN005` under a
   default configuration, but fails `APLAN017` wherever `require_min_strength` excludes `note`
   (as this repository's own policy does). That is the intended asymmetry, because a claim
   imported from elsewhere should not survive a strict gate until someone proves it.

5. **Import writes new task files and nothing else, unless asked.** It refuses to overwrite an
   existing task id. The one sanctioned mutation of existing state is `--cut <milestone>`, which
   appends the imported ids to that milestone's `blocked_by` so the tasks land in a real release
   cut. It verifies first that the target exists, is tagged `milestone`, and is **not already
   `done` or `dropped`**: appending an unfinished task to a settled cut would make that milestone
   depend on an unfinished one (`APLAN007`, an error), so the flag refuses rather than invalidate
   a closed claim. It then edits that one line surgically (the approach `close` uses, not a
   whole-file reserialize) and names the file it touched in the report. Without the flag,
   imported tasks belong to no cut, and the report says that a repository setting
   `require_cut_membership` will see `APLAN022` until they are placed.

## Consequences

- **The source lifecycle needs an import path, not a schema change.** The RFC and proposal
  pipeline is expressible today; `A1-019` ships the reader and the documented tag convention,
  and `aplan-core` is untouched.
- **Import output satisfies any evidence policy by default.** Because "implemented" does not
  become `done`, a fresh import clears `APLAN005` and `APLAN017` at any configured strength. A
  user who wants the stronger claim must ask for it and accept the warning, or close the task
  with real evidence. Evidence is not the only gate, though, and this is deliberately not a
  claim that import output is unconditionally clean: where `require_cut_membership` is set,
  imported tasks in no cut still fail `APLAN022` until `--cut` or a `cross-cutting` tag places
  them, which is why clause 5 obliges the report to say so.
- **The importers live in `aplan-interop`**, the crate the HLD architecture table and ROADMAP
  already plan for exactly this role, not a new crate per flavor. Each flavor is a module behind
  one seam, so this policy has a single enforcement point and a second flavor is additive.
- **The report is a new output surface, deliberately outside the rule codes.** It is not a
  `Diagnostic` in the `APLAN0xx` sense, so it adds no rule code and never competes with the
  validator's vocabulary. The *command* is a different matter: `facts.json` enumerates the
  command tree and `crates/aplan/tests/facts.rs` pins it, so `aplan import` must land in that
  generated file in the same change or CI fails (HLD section 16's generate-the-counts gate).
- **This policy binds every future importer**, not just the KEP flavor: Backlog.md, task-master,
  spec-kit, beads, and any RFD or tracking-issue reader added later. A future importer that
  needs to break one of these clauses supersedes this ADR rather than quietly diverging.
- **The stage-tag vocabulary is unvalidated by construction**, which is the accepted cost of
  keeping it out of the status enum. A mistyped `kep:implemantable` is not caught, the same
  trade `APLAN021` accepts for the `milestone` tag. If a tag vocabulary later needs enforcement,
  it is a rule, not a status.
