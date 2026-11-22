# ADR-0069: A trace anchor retargeting to another element kind is an error

Date: 2026-11-22

Status: Accepted

Tasks: `A2-050`, `A2-051`

Extends `ADR-0015` (table-row anchors and heading-first precedence) and `ADR-0035`
(structured-document node anchors). Supersedes nothing.

## Context

`ADR-0015` allowed a Markdown `traces` anchor to name either a heading or a table row (addressed
by its first cell), with headings winning when both mint the same slug. That precedence preserved
byte-for-byte compatibility for existing plans whose documents contained both a `## Decision`
heading and a `| Decision | ... |` table row.

Issue #30 exposed the failure mode of that fallback when a document evolves:

1. A document contains both `## Beta` and a table row `| Beta | ... |`.
2. A task declares `traces: [DESIGN.md#beta]`, which resolves to the heading `## Beta`.
3. A later commit renames `## Beta` to `## Beta Prime`, leaving `| Beta | ... |` intact.
4. `aplan check` passes because `#beta` still resolves to the table row (`APLAN008` is silent).
5. `aplan verify` reports at most an ordinary `APLAN045` warning (or `APLAN018` on a closed task
   with `detect_drift: true`) saying the section changed -- which a repository may run in
   `advisory`, or which disappears if `detect_drift` is off.
6. Deleting the table row as well immediately produces the `APLAN008` error that renaming the
   heading should have produced. The same-slug table row silently hid a broken trace and retargeted
   the task to a different element kind.

Three options were considered:

1. Track and compare the resolved **element kind** (`heading` vs `table row`) and report
   retargeting explicitly.
2. Lint or reject any trace anchor that matches both a heading and a table row when written.
3. Document heading-first precedence and accept silent retargeting.

`APLAN045` (open tasks) and `APLAN018` (closed tasks) have different storage models and therefore
require their consequences to be specified separately.

## Decision

**Choose option 1: track the resolved element kind and report a kind change as a blocking error,
while preserving `ADR-0015`'s heading-first precedence and backward compatibility for legacy bare
stamps.**

### 1. Element-kind classification

Every resolved trace anchor carries a typed element kind alongside its content hash:

- Markdown heading: `heading`
- Markdown table row: `row`
- Structured YAML/JSON node (`ADR-0035`): `node`

`ADR-0015`'s precedence stays unchanged: when a Markdown document contains both a heading and a
table row with the same slug, the anchor resolves to `heading`.

### 2. Consequence for `APLAN045` (open tasks, `aplan verify`)

Open tasks carry no stored stamp by default; `aplan verify` already reads the traced document at
the task's review baseline commit (as defined by `ADR-0070`) and at `HEAD`.

- Resolve `(kind_then, hash_then)` at the baseline commit and `(kind_now, hash_now)` at `HEAD`
  (or compare against the live task's explicit `trace_fingerprints` stamp when present per
  `ADR-0070`).
- If `kind_then != kind_now` (for example `heading` -> `row`, or `row` -> `heading`), emit a
  **blocking error** (a dedicated error rule code, distinct from advisory-capable `APLAN045`)
  naming the trace, the prior element kind, and the new element kind, and directing the author to
  update `traces` to the renamed target or explicitly confirm the new target (via editing `traces`
  or `aplan review`).
- Because retargeting is an error rather than an `APLAN045` warning, placing `APLAN045` in
  `advisory` can no longer hide a renamed heading behind a same-slug table row.
- If `kind_then == kind_now` and `hash_then != hash_now`, ordinary `APLAN045` content-drift
  warning semantics remain unchanged.

### 3. Consequence for `APLAN018` and `trace_fingerprints` (closed tasks, `aplan check`)

Today `trace_fingerprints` is a `BTreeMap<String, String>` storing bare 16-hex-digit FNV-1a
hashes (`"DESIGN.md#beta": "0123456789abcdef"`).

- **Additive kind-tagged stamp format (no schema break).** Newly written stamps produced by
  `aplan close`, `aplan close --restamp`, `aplan supersede`, and `ADR-0070`'s `aplan review`
  encode the resolved element kind as an explicit prefix inside the existing string value:
  `heading:<16-hex>`, `row:<16-hex>`, or `node:<16-hex>`. `schemas/v1/task.json` stays
  `BTreeMap<String, String>` with no structural schema change.
- **Zero migration churn for existing bare stamps.** Existing tasks carrying bare `<16-hex>`
  stamps continue to validate via `holds_in` against content hashes only (including the pre-`A1-122`
  trailing-blank compatibility path). An untagged legacy stamp did not record its element kind, so
  `aplan check` does not invent a kind-mismatch error or flag existing plans on upgrade. Plans
  converge naturally as tasks are closed or restamped.
- **Hermetic retargeting error on kind-tagged stamps.** When `detect_drift` is enabled on a closed
  task and a kind-tagged stamp (`heading:<hex>` or `row:<hex>`) now resolves to a different
  element kind, `aplan check` emits the same **blocking error** (distinct from `APLAN018`, so
  `advisory: [APLAN018]` does not hide retargeting!). If the element kind matches and only the
  content hash differs, ordinary `APLAN018` warning semantics remain unchanged.
- **Forward-upgrade compatibility note.** Because older binaries predating `A2-051` compare
  `trace_fingerprints` strings verbatim against bare `<16-hex>`, they will report `APLAN018` on
  tasks stamped by the newer format. `A2-051` must document this in `docs/upgrading.md` and gate it
  behind the staged upgrade procedure (`ADR-0068`), following the same precedent as `A1-122`.

## Consequences

- Renaming a traced heading when a same-slug table row exists is no longer silent or downgraded to
  an advisory warning on either open tasks (`verify`) or closed kind-stamped tasks (`check`).
- Existing plans with valid `## Decision` + `| Decision |` overlap continue to pass with zero
  false positives, and existing bare `trace_fingerprints` require no bulk migration.
- Retargeting detection is symmetrical: a table-row trace silently captured by a newly added
  same-slug heading (`row` -> `heading`) is caught identically to `heading` -> `row`.
- `A2-051` implements this decision, registers the dedicated retargeting error rule and stamp
  format behaviour, updates `docs/rule-catalogue.md` and `docs/upgrading.md`, and closes #30.

## Alternatives considered

| Alternative | Disposition |
|---|---|
| Option 2: lint or reject any trace whose slug matches both a heading and a table row at write time | Rejected: produces false positives on valid existing documents (`## Decision` plus a `| Decision |` summary row, the exact motivating case of `ADR-0015`), and fails to catch a table row added in the same commit that renames the heading. |
| Option 3: document precedence and leave retargeting silent | Rejected: violates the evidence-integrity premise by letting an unrelated table row suppress the `APLAN008` error a heading rename should have caused. |
| Keep retargeting inside `APLAN045` / `APLAN018` as a warning | Rejected: issue #30 specifically noted that repositories running `APLAN045` or `APLAN018` as `advisory` would still lose the error signal that `APLAN008` would have given without the colliding row. |
| Add a separate `trace_kinds` frontmatter field | Rejected: creates a second map keyed by trace string that can drift out of sync with `trace_fingerprints` and requires a new task schema field when `heading:<16-hex>` fits cleanly in one atomic string value. |

## References

- Issue #30, `A2-050`, `A2-051`.
- `ADR-0015` (table-row anchors), `ADR-0035` (structured-document anchors), `ADR-0068` (staged
  upgrade boundary), `ADR-0070` (open-task review baseline).
