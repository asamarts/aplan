# Contributing to aplan

`aplan` validates a plan the way a linter validates code. The one rule that governs
everything here: **the tool must never quietly lie**, so a change that makes a doc, a
count, or a status disagree with reality is a bug, not a detail.

## Build and check

```sh
cargo test --workspace          # all unit tests
cargo fmt --all --check         # formatting (CI-enforced)
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask gen-schema --check  # the schema is derived from the types, not hand-written
cargo xtask lint                # house style: no typographic tells (see below)
cargo run -p aplan -- check     # aplan lints its own plan (the dogfood)
```

A PR is expected to leave all of these green. The dogfood is not optional: a plan tool
whose own plan does not validate has no business merging.

## Adding or changing an invariant

Rules are a **closed, numbered set** (`APLAN001`...), deliberately: the codes are a
consumer contract, so adding one is a breaking change for anyone parsing our output.

1. Add one row to the `define_rules!` table in `crates/aplan-core/src/diagnostic.rs`.
   That single row generates the `Rule` variant, its `code::` constant, and its entry in
   `Rule::ALL`; the three can't drift because they come from the same line.
2. Implement the check in `validate.rs` (per-task) or `validate()` (graph-wide).
3. Add **both** a scenario where it fires *and* one where it stays silent, in
   `crates/aplan-e2e/tests/coverage.rs`, `corpus()` for a hermetic `check` rule, or
   `verify_corpus()` (which builds a real temp git repo) for an `aplan verify` rule. This
   is **not optional advice**: because the coverage audit walks `Rule::ALL`, a rule with no
   fires-and-silent pair fails CI. A rule that ships with only its passing case lets its
   failure path silently regress, so the build refuses it.
4. Update the rule table in `README.md` and the catalog in `HLD.md §6`.
5. Regenerate `facts.json` (`cargo run -p aplan -- facts > facts.json`): it lists every rule
   with its severity, derived from the code, and `tests/facts.rs` fails CI if the committed
   file is stale.
6. Note it in `CHANGELOG.md`.

An **opt-in** rule (off unless `.aplan.yml` turns it on, like `APLAN017`/`APLAN018`/`APLAN022`)
also needs a field on `Config` (`crates/aplan-config`), a matching `Options` field wired
through the CLI, and `cargo xtask gen-schema` to regenerate `schemas/v1/config.json`; gate the
scenario with the toggle rather than firing it unconditionally.

The same shape applies to a new `--format`: add the variant, its `as_str`/`from_str`
arms (the compiler forces the render arm), and list it in the format audit, or CI fails.

## The source of truth

- The **Rust types** are the spec; `schemas/v1/task.json` (from `Task`) and
  `schemas/v1/config.json` (from `aplan_config::Config`) are *derived* from them and gated
  by `gen-schema --check`. Never hand-edit a schema; adding a config key is a field on
  `Config`, then `cargo xtask gen-schema`.
- **Design** lives in `HLD.md`; **sequencing** in `ROADMAP.md`; **decisions** in
  `docs/adr/` (immutable; supersede, don't rewrite); **competitive rationale** in
  `pre-public product research`. If code and a doc disagree, the code is right and the doc is stale; fix
  the doc.

## Style

Match the surrounding code: `unsafe` is forbidden, clippy pedantic is on, output is
deterministic (BTreeMap, totally-ordered sorts, no `HashMap` iteration leaking into
diagnostics). Comments state constraints the code can't, not narration.

### House style: plain-ASCII typography

`cargo xtask lint` (a CI gate) forbids a small set of typographic characters that read as
machine-generated: em and en dashes, curly/smart quotes, the ellipsis character, and the
non-breaking space. Use their ASCII equivalents: a comma, colon, semicolon, parentheses,
or a fresh sentence instead of an em dash; a hyphen or "to" for an en dash; straight
quotes; three dots for an ellipsis. This keeps the repo's prose from carrying the
tells of a generated draft. Functional Unicode is deliberately allowed and untouched: the
arrow in cycle output, the section sign in cross-references, and the accented letters and
emoji in the slug tests are all fine. When a rule needs a literal forbidden character (the
lint's own tests do), spell it with a `\u{...}` escape so the source stays clean.

## Licensing

By contributing you agree your work is dual-licensed under Apache-2.0 OR MIT, matching the
project.
