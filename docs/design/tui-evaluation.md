# TUI human evaluation protocol

Status: protocol frozen by `A1-682`; final renderer boundary recorded by `A1-683`; execution and
disposition belong to `A1-449`.

Minimum final renderer implementation: `1c813efe924f3453dc3ed91c3b1b1c1dcfd4d4bb`. Each human
session still records the exact later source and binary it actually uses.

This is a protocol, not a result. It does not claim that the interface is intuitive, accessible or
release-ready. Automated actor runs may rehearse these tasks under
[`tui-ai-evaluation.md`](tui-ai-evaluation.md), but they remain formative synthetic observations
and never count as a participant or a human result.

## Questions

The evaluation asks whether a person who has not learned aplan's implementation can:

1. identify what the plan needs next and why;
2. move from plan to phase, milestone and task without losing orientation;
3. distinguish phase order, visual peer order and dependency direction;
4. trace exact blocker, dependent, critical-route and aggregate-edge meaning;
5. recognize omitted work and use continuation or recenter controls to recover it;
6. find task identity, readiness, responsibility, placement and next action without reading an
   undifferentiated block; and
7. recover from a narrow display, delayed operation and unavailable or stale state without
   mistaking presentation for a plan verdict.

Visual preference is a separate question. A preferred picture cannot compensate for a wrong graph
answer, more wrong turns or worse task completion.

## Evidence boundary and cohort

Recruit at least five consenting adults. The cohort must contain at least two people unfamiliar
with aplan and at least two experienced with terminal development tools; one person may satisfy
both descriptions only if the report says so. Record relevant experience as a broad self-report,
not a demographic proxy. Do not claim accessibility from a simulated persona.

Before recording, explain the purpose, what interaction and speech are captured, retention and
withdrawal. Use participant labels such as P1, remove names and account identifiers, and use only
synthetic public fixtures. Do not place a private repository, participant secret or production
credential in a session. The report records refusals and protocol deviations rather than silently
replacing a participant or task.

Synthetic actor and critic records remain in their own evidence class. A facilitator may use an
automated run to find a broken fixture before recruitment, but may not show its route, expected
answer, critique or terminology to a participant.

## Candidate identity and setup

Each session record names:

- the exact source commit and binary digest;
- `aplan --version`, `aplan facts` digest and terminal emulator;
- operating system, terminal dimensions, color mode and glyph mode;
- fixture revision and SHA-256 of every plan/configuration input;
- renderer and order assignment for each graph task;
- start/end monotonic times and any interruption or facilitator intervention.

Build the binary once from clean tracked bytes. Do not change labels, key bindings, fixture data or
expected answers within a round. A material candidate change starts a new revision and repeats the
affected tasks; it is not pooled with the old candidate as though participants saw one product.

Use these fixed environments:

| environment | size | purpose |
|---|---:|---|
| wide | 120x30 | full hierarchy and inspector comprehension |
| normal | 80x24 | ordinary graph and work discovery |
| narrow | 48x14 | responsive list/detail, overlays and recovery |
| monochrome ASCII | 80x24 | non-color and glyph-independent meaning |

The graph fixture is generated from the A1-682 named corpus as corrected through A1-688 at the
candidate commit. Preserve the same titles, phases, milestone membership, dependency ordinals,
aggregate provenance, disconnected component and bounded-window omissions for every participant.
It must include a view whose source is too large for one interactive scene, so the correct answer
requires typed continuation rather than horizontal clipping. The canonical answer sheet is derived
from the typed JSON projection, not from either visual renderer.

The non-graph fixture includes missing-plan onboarding, a plan with one in-progress and at least
two ready tasks with a deterministic tie, one blocked task, one finding, one claimed task, a
deliberately delayed startup and one slow operation whose progress can be disabled. No task id or
title may reveal an answer such as `NEXT`, `BLOCKER` or `EXPECTED`.

## Blinding and counterbalancing

Call the graph presentations A and B in participant materials. Do not mention `layered`, `lanes`,
Graphlay, task ids, implementation status or which presentation is proposed as the default. The
facilitator has a separate assignment sheet and does not describe one as newer or better.

Use a balanced crossover:

- odd participant labels see A then B; even labels see B then A;
- within each order, alternate the canonical and reversed task sequence;
- swap whether A means Lanes or Layered for successive participant pairs;
- use isomorphic fixture copies with different neutral labels, then verify their typed answer
  sheets are identical before the round;
- reset process, viewport, selection, search, filters and history between presentations.

Do not show expected routes, command hints beyond what the product itself displays, source code,
this protocol, design documents or earlier transcripts. Ask goal questions, not leading action
questions. If a participant asks for help, first ask what they would expect to do, then provide the
smallest neutral prompt and record it verbatim as help use.

## Session sequence

Begin with consent and a neutral think-aloud practice unrelated to aplan. Then run the following
tasks. Do not teach the keymap before the first attempt.

### First-use and work discovery

1. Start in a directory without a plan. Ask what happened and what they would do next.
2. Launch the delayed fixture. Ask what the application is doing, whether it is stuck and how they
   would disable progress for a script.
3. In Overview, ask what needs attention next and what is evidence versus a status claim.
4. Find the named task and answer: what is selected, can it start, who owns it, who currently holds
   it, where does it belong, and which detail or action should be opened next?
5. Diagnose the seeded finding and identify the exact repair instruction without performing a
   write.
6. Explain why the blocked task cannot start, then return to the original work list.

### Graph comprehension and navigation

Run these once with each blinded presentation where the surface supports the same question. CLI
text is captured as a distinct surface; it is not timed as if printing a document and navigating a
TUI were the same motor task.

1. From the plan overview, identify the phase and milestone containing the next work.
2. Drill to one named task and return to the prior overview without losing the original context.
3. Name one direct blocker and one direct dependent, then follow each relationship.
4. Identify the critical route and explain what makes it critical.
5. Explain one aggregate milestone edge by naming the underlying task relationship it represents.
6. Change peer ordering and explain whether dependency order changed.
7. Move one dependency layer, then one ordered phase; explain the difference.
8. Find a task omitted from the current bounded scene, use a continuation or search/recenter path,
   and return.
9. Recover after manual pan and after the terminal narrows to 48x14.
10. In monochrome ASCII, trace one dependency and identify selection, feedback and continuation
    without relying on color or Unicode shape.

After each presentation, ask an open question about what was clear or confusing. Ask visual
preference only after all answer and task measurements are complete.

### Recovery and write boundary

Open a representative mutation form, reach review, cancel, and explain whether anything was
written. Then present a stale or unavailable result and ask how to recover. The participant must
not be asked to weaken a gate or create fake evidence.

## Observation record

For each task record separately:

- correct, incorrect, incomplete or abandoned answer against the typed answer sheet;
- elapsed time to first correct answer and to task completion;
- primitive actions and route transitions;
- wrong turns, backtracks, repeated actions and dead ends;
- help request, exact facilitator prompt and result;
- whether the participant read the inspector sequentially before answering;
- visible text cited by the participant and their explanation in their own words;
- rendering, focus, pointer, resize or restoration defect;
- confidence and visual preference only after the behavioral fields.

Do not merge these into a universal usability score. Report per-task counts and distributions with
the small cohort visible. Distinguish a wrong domain answer from a motor error, a missing control,
an unreadable presentation and a fixture/protocol fault.

## Predeclared interpretation

A candidate fails the comprehension gate if any of these are confirmed and not fixed and repeated:

- blocker and dependent direction are reversed;
- phase or peer order is described as causality after the participant has used the relevant
  control;
- an aggregate edge is interpreted as one invented dependency and its provenance cannot be found;
- omitted work is mistaken for absent work, or the only recovery path is undiscoverable help;
- a stale, failed or preview state is described as committed;
- narrow or monochrome presentation removes the only usable action or semantic distinction;
- Layered has a lower correct-answer count than Lanes for a shared graph question without an
  evidence-backed disposition and a new blocking task.

Time is descriptive for five participants. Treat a consistent material slowdown, repeated
backtracking or repeated help use as a finding rather than averaging it away. Preference never
overrides a comprehension failure. No clean synthetic run can waive one of these conditions.

## Disposition and report

`A1-449` owns the report. Every observation receives exactly one disposition:

1. fixed in a named commit and, when applicable, protected by a deterministic test;
2. filed as a prioritized task with the affected surface and release consequence; or
3. declined with a concrete reason and the evidence that makes acceptance safe.

The report keeps measured behavior, participant language, facilitator interpretation, preference
and synthetic preflight in separate sections. It includes adverse and incomplete sessions, names
the exact tested build and fixture digests, and does not claim universal usability from this
cohort. A protocol fault triggers only the affected repeat; it is never silently edited out.

## Method sources

The task-centered moderation and consent structure follows the sources already reviewed for
[`tui-structure-review.md`](tui-structure-review.md): GOV.UK's moderated-testing and research-round
guidance. The synthetic-evidence limits and judge-bias controls come from
[`tui-ai-evaluation.md`](tui-ai-evaluation.md). Those sources support study preparation and
augmentation, not replacing observed human behavior with a model verdict.
