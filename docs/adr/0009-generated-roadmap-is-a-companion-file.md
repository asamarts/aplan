# ADR-0009: The generated roadmap is a companion file, not spliced into prose

- Status: Accepted
- Date: 2026-07-25
- Deciders: kaminsod

## Context

The v0.5 self-hosting cutover (ROADMAP §6) makes the plan the source of truth for the
roadmap's *machine-checkable half*: which cut each task belongs to, its status,
dependencies, evidence, and the per-cut rollups and critical path. Today that half is
hand-maintained in ROADMAP.md and drifts from the plan (authoring the milestone tasks in
A1-014 already surfaced one drift: §10's table had dropped `A1-012` from v0.4).

Two questions must be answered together: **what content can actually be generated from the
plan**, and **where the generated content lives and how its drift is gated**. The second is
easy to answer in isolation and wrong to answer without the first.

Three facts constrain the answer.

- **The repo has one gate shape for generated artifacts.** `schemas/v1/*.json` (via `xtask
  gen-schema --check`) and `facts.json` (via `tests/facts.rs`) are each produced by a
  function and gated by *whole-file* equality (`assert_eq!(on_disk, generated)`), regenerated
  with a documented command. Both are standalone files, but note *why*: they are
  machine-consumed JSON with no host document they could ever be embedded in. So they
  establish a precedent for the **gate shape**, not for standalone-versus-embedded.
- **ADR-0001's constraint:** never make English prose a regenerable source of truth. Only
  machine-checkable facts may be generated; rationale stays hand-authored.
- **The plan models less than the current roadmap shows.** `Task` (ADR-0008 keeps the model
  minimal) carries no HLD-section field and no rule-code field, so parts of today's ROADMAP
  are *not derivable from the plan* and cannot be generated (see Decision §1).

ROADMAP §6, as first written, said "generate the machine-checkable *sections of this file*",
i.e. fenced regions spliced into ROADMAP.md, and several other places echo that wording
(the intro banner, §3-v0.5, §6, §10, `M-v0.5.md`, `plan/README.md`, and the
`A1-015`/`A1-016` task bodies, all enumerated in the Downstream-edits table). This ADR refines
that mechanism and enumerates the edits the refinement forces.

## Decision

### 1. What is generated, and what is not

Generated into the companion file, because it is derivable from the plan:

- the **cut list**, from the milestone meta-tasks (ADR-0008), ordered by each milestone's
  `v0.N` tag parsed *numerically* (so `v0.10` sorts after `v0.9`, not before);
- per cut, its **member tasks** (`id`, `title`, `status`, `blocked_by`, and the *cited*
  evidence ref) from the milestone's `blocked_by` set;
- per cut, a **rollup** (done/total, ready, blocked) and the **critical path**;
- a **cross-cutting** section for tasks that belong to no cut (e.g. `A1-006`).

*Not* generated, because the plan does not model it, so it stays hand-authored in ROADMAP.md:

- **which HLD sections a cut relates to** (§10's "HLD §" column). The nearest plan data is
  `traces`, but the milestones carry none, most tasks carry none, and traces are arbitrary
  doc anchors, not per-cut HLD labels. Not reconstructable.
- **which rule codes a cut introduced** (§10's "Rule codes" column). Tasks carry no rule
  codes (they appear only as prose inside task titles); rules are not tasks; `facts.json` has
  no cut linkage.

So "the §10 table migrates into the generated file" is true only for its `cut / theme / task
/ status` rows (theme is the milestone title). Its **design** and **rules** cross-reference
columns, the two axes that make §10 a *consolidation* map, have no plan source and remain
editorial prose in ROADMAP.md; so does its **Status** column's editorial text about
not-yet-tasks (e.g. "Kani proofs ... planned"), which is prose, not a per-task status, and
belongs in §3 rationale rather than the companion. The
generated file reproduces the derivable rows; it does not silently absorb the
cross-references, which would delete information the plan cannot regenerate.

### 2. Mechanism: a standalone companion file

The gated artifact is **`ROADMAP.status.md`**, a standalone file written by `aplan roadmap
--format markdown` and gated whole-file by `gen-roadmap --check`, not fenced regions inside
ROADMAP.md.

The honest justification is a **single-editability boundary**: ROADMAP.md is 100%
hand-authored and safe to edit anywhere; `ROADMAP.status.md` is 100% generated and never
hand-edited, so a reader never has to ask which lines survive regeneration. We reuse the
repo's whole-file **gate shape** (`gen-schema`/`facts`); we do *not* claim those JSON
precedents settle standalone-versus-embedded, they never faced that choice. Fenced-region
generation (doctoc, terraform-docs, cog) is a legitimate, widely-used technique and is not
"fragile"; it is declined because a half-generated document is a clarity hazard, not because
the technique is unsound.

### 3. `aplan roadmap`: advisory, hermetic, deterministic

- **Advisory:** it never fails the build (a long plan is not an error). Only *drift* between
  `ROADMAP.status.md` and fresh output fails, via `gen-roadmap --check`, as schema drift does.
- **Hermetic, and why that is safe:** like `check`, it opens no git and renders the *cited*
  evidence ref, not its resolution, so the artifact is byte-reproducible in any checkout. It
  does **not** verify `done`. It is trustworthy only because the *same plan* is gated by
  `aplan verify` in the same CI run (the "aplan lints its own plan" job runs `aplan verify`
  with `fetch-depth: 0` as a blocking step): `ROADMAP.status.md` is only ever committed green
  when `verify` passed, so every `done` it renders is one the CI `verify` gate accepted.
  ("Accepted", not "resolved": `verify` accepts a dated `note` at L0, ADR-0001, so the
  render shows a claim the gate let through, not necessarily a machine-resolved one.) It
  carries a footer documenting that its statuses are the ones the CI `verify` gate accepts;
  the footer is for the reader, not a per-render marker, by byte-reproducibility a local
  render, where `verify` never ran, emits the same footer. Resolving evidence *inside*
  `roadmap` is rejected: it would break byte-reproducibility, and `verify` already owns it.
- **Deterministic and reflow-proof (the gate's stability):** the markdown is emitted as
  **non-aligned** pipe tables (single-space padding), so a cell's width never depends on its
  content and one longer title cannot re-pad, hence re-diff, a whole column. Ordering is
  fixed (cuts by numeric version, tasks by id). The emitted form *is* the canonical form: no
  markdown formatter may rewrite `ROADMAP.status.md` (it goes in any formatter's ignore list;
  the house-style lint already accepts it as plain ASCII). This is a genuinely new constraint
  relative to `gen-schema`, whose JSON is canonicalised by `serde_json`; A1-015 owns meeting
  it, and the "no new concepts" reassurance applies only to the gate, not the rendering.
- **Implementation seam:** the per-cut *model* (grouping by cut, rollups, per-cut critical
  path) is built in `aplan-core`, mirroring `Summary`/`CriticalPath`, and `aplan-output`
  renders it to a `String` (today's `render_*` print to stdout; the roadmap renderer returns
  the markdown). So the `gen-roadmap` xtask calls those libraries directly, as `gen-schema`
  calls `derived_schema`, never the built binary, and the `spec` CI job stays
  build-order-clean. `xtask` gains a dependency on `aplan-output` (it already depends on
  `aplan-core`), and A1-015's `paths` include `aplan-core` accordingly.
- **Grouping seam:** `roadmap` is the one view whose *subject* is milestones, so unlike
  `ready`/`critical-path`/`summary` it must not apply the milestone-excluding filter to
  grouping; it still excludes each milestone from its own member list and reuses
  critical-path's zero-weight-passthrough handling. A1-015 notes this explicitly.

### 4. Narrative lives in one place per content type

Per-cut **rationale** (the *why*) is canonical in ROADMAP.md §3. A milestone **body is to
carry only its exit-criteria** (the *what-done-looks-like*); its `## Theme` paragraph
duplicates §3 and is removed in the cutover (A1-016, see Downstream edits), so the bodies
still carry it until then. The two are different content, so once split neither can drift
into the other. This corrects ADR-0008's "the body carries the cut's narrative": the body
carries the cut's *exit-criteria*; its *rationale* lives in ROADMAP.md.

### 5. ROADMAP.md after the cutover

ROADMAP.md stays hand-authored: north star, per-cut rationale (§3), the design and rule
cross-references (§1 above), formal-methods and testing strategy, risks. Its opening line
stops claiming "this file is the single source of truth" (the *plan* is); it states it is
narrative and points to `ROADMAP.status.md` for generated status. §10 keeps only what the
plan cannot generate (the HLD-§ and rule-code cross-references) and links to the companion
for the cut/task/status rows.

## Consequences

- **Consistency where it is real:** the gate is the repo's established whole-file shape; a
  contributor who understands `gen-schema` understands `gen-roadmap`'s gate. The *rendering*
  (reflow-proof markdown) is new, and is called out as such.
- **A single-editability boundary:** no document is half-generated, so the "may I edit this
  line?" hazard never arises. ADR-0001 is honored by a file boundary.
- **The status-strip is pervasive, and §3's residual is not gated.** §3 today fuses task
  references, status markers (15+ `**done**`/`**landed**` plus inline parentheticals), and
  rationale into single sentences, so the cutover (A1-016) must rewrite most §3 bullets to
  pure rationale, moving status to the companion. After the strip, §3 still carries
  hand-authored task-id, HLD-§, and rule-code references that the generated file does not
  cover and no gate binds to the plan, so §3 remains a residual drift surface. This is
  accepted in writing: the generated file is the *authoritative* cut/task/status mapping, and
  §3's references are illustrative narrative. A lint that every `A#-###` mentioned in
  ROADMAP.md exists in the plan is a cheap candidate follow-up (noted, not required for v0.5).
- **Two files, named to keep authority clear:** `ROADMAP.status.md` (generated status) sits
  beside `ROADMAP.md` (narrative). The name reads as "the status *of* the roadmap", a
  subordinate view, not a second roadmap. The README-to-`facts.json` analogy is dropped:
  unlike `facts.json`, this file is roadmap prose and could compete for the identity "the
  roadmap" if misnamed.
- **The self-hosting demonstration stands:** `aplan` generates its roadmap-of-record from its
  own plan and CI proves they never drift; `ROADMAP.status.md` is that record, and its
  `done`-claims are exactly the ones `verify` gates.

## Downstream edits this decision forces

The refinement in §2 falsifies every in-repo description of the old "sections of this file"
mechanism. The **wording** edits land now, with this ADR; the **structural** §3 status-strip,
the milestone-body Theme removal (§4), and the generator itself land in A1-015/A1-016.

| Location | Was | Becomes | When |
|---|---|---|---|
| `ROADMAP.md` intro banner | "single source of truth"; "the generated sections are regenerated" | narrative; the plan is the SSOT; status generated in `ROADMAP.status.md` | now (banner mechanism line); header-flip in A1-016 |
| `ROADMAP.md` §3 v0.5 | "regenerates the machine-checkable sections of this file" | "generates `ROADMAP.status.md`" | now |
| `ROADMAP.md` §6 Stage 3 + checklist | "equals this file's generated sections byte-for-byte"; "this file's header flips" | "`ROADMAP.status.md` equals `aplan roadmap` output byte-for-byte"; "ROADMAP.md becomes narrative-only and links to the companion" | now |
| `ROADMAP.md` §10 table | "at the cutover `gen-roadmap` generates it from them"; "this table is generated" | its cut/theme/task/status rows move to `ROADMAP.status.md`; §10 keeps the `HLD §`/`Rule codes` columns and a link, and its Status-column prose about not-yet-tasks moves to §3 | wording now; row-migration A1-016 |
| `plan/tasks/M-v0.5.md` exit criteria and Theme | "equals ROADMAP.md's generated sections byte for byte"; Theme's "sections of ROADMAP.md" | reference `ROADMAP.status.md` | now |
| `plan/tasks/A1-015.md`, `A1-016.md` bodies | "gate against ROADMAP.md"; "sections of ROADMAP.md"; "header flips" | reference `ROADMAP.status.md` and the companion mechanism | now |
| `plan/README.md` | "at the v0.5 cutover `gen-roadmap` generates it [§10] from them" | its cut/task/status rows generate into `ROADMAP.status.md` | now |
| `plan/tasks/M-v0.1.md` .. `M-v0.6.md` bodies | `## Theme` (duplicates §3) + `## Exit criteria` | `## Exit criteria` only | A1-016 |
| `ROADMAP.md` §3 per-cut bullets | task-ref + `**status**` + rationale fused | pure rationale; status moves to the companion | A1-016 |
| `HLD.md` §4, `plan/README.md`, `ROADMAP.md` (§3-v0.5 bullet, §6 Stage 2) | "body holding the narrative/theme (ADR-0008)" | "body holding the exit-criteria" (ADR-0009 §4; rationale in §3) | now |
| `HLD.md` §18 | "ROADMAP.md, the single source of truth for what `aplan` builds" | ROADMAP.md is narrative; the plan is the SSOT (same flip as the ROADMAP banner) | A1-016 |

## Alternatives rejected

- **Fenced regions spliced into ROADMAP.md** (§6's original wording). A common, robust
  technique (doctoc, terraform-docs), so *not* rejected for fragility, and it has real merits:
  one file, it satisfies §6 as first written, and each cut's status sits next to its
  rationale. Rejected because it produces a half-generated document whose per-line
  editability is not visually obvious; the single-editability boundary (§2) is the deciding
  advantage of the companion file.
- **Maximal scope: also generate each cut's narrative** from its milestone body. Rejected
  twice over: it regenerates English prose (ADR-0001), and after the §4 split the milestone
  body no longer holds rationale to generate.
- **Minimal scope: generate only the §10 cut-to-task table.** Rejected: the per-cut rollups,
  statuses, and critical path would stay hand-maintained and keep drifting; the
  machine-checkable *half*, not just the appendix rows, is the right unit.
- **A JSON-only artifact** (like `facts.json`), no committed markdown. Rejected: a roadmap is
  read by humans; `--format json` is an additional view for tools, not the artifact of record.
- **Resolve evidence in `roadmap`** (verify-style). Rejected: a committed, CI-gated artifact
  must be byte-reproducible without a git walk, and `verify` is already the command that
  resolves evidence.

## References

- ROADMAP.md §6 (the cutover, whose mechanism this refines and whose checklist this
  reconciles), §10 (the table whose derivable rows migrate and whose cross-references do not),
  §8 (`gen-roadmap` among the committed-and-gated artifacts).
- ADR-0001 (never make prose a regenerable source of truth): honored by a file boundary.
- ADR-0008 (milestones as cuts): the cuts this groups by; §4 corrects its "body carries the
  narrative" to "body carries the exit-criteria".
- ADR-0002 (schema derived from types): the `gen-schema` precedent reused for the *gate
  shape*; `xtask/src/main.rs` and `crates/aplan/tests/facts.rs` are the two instances.
- `.github/workflows/ci.yml` ("aplan lints its own plan" runs `aplan verify`): the sibling
  gate that makes `ROADMAP.status.md`'s hermetic `done`-claims trustworthy.
- HLD.md §8 (CLI surface), §17 (generated artifacts are committed and gated).
