# Contributing to aplan

`aplan` validates a plan the way a linter validates code. The one rule that governs
everything here: **the tool must never quietly lie**, so a change that makes a doc, a
count, or a status disagree with reality is a bug, not a detail.

## Build and check

```sh
bash ci/scripts/preflight.sh        # selected development contract for the current changes
bash ci/scripts/preflight.sh --full # exact complete integration/release contract
ci/scripts/test.sh              # pinned nextest tests, Cargo doctests and CLI audit
cargo fmt --all --check         # formatting (CI-enforced)
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps  # intra-doc links resolve
cargo xtask gen-schema --check  # the schema is derived from the types, not hand-written
cargo xtask gen-roadmap --check # ROADMAP.status.md is generated from the plan (ADR-0009)
cargo xtask check-docs          # ROADMAP.md and private pre-public adoption records agree with the plan
cargo xtask lint                # house style: no typographic tells (see below)
cargo deny --locked check advisories bans licenses sources
cargo xtask dependency-audit   # admitted dependencies and default/headless budgets
cargo xtask coverage --check    # per-crate coverage floors (slow: instrumented rebuild)
cargo run -p aplan -- check     # structural half of the dogfood
cargo run -p aplan -- verify    # resolves the plan's evidence (the authoritative half)
```

Ordinary handoff runs the development profile; release and explicit integration checkpoints require
full. The dogfood is not optional: a plan tool whose own plan does not validate has no business
merging. The `development` profile is selected feedback,
not a full or verified claim; its summary names selected packages plus selected, widened and
deferred gates. The assurance vocabulary and executable inventory are defined once in
[`ci/README.md`](ci/README.md#run-the-gates-locally).

Rustup selects the exact development/release compiler and required rustfmt/Clippy components from
`rust-toolchain.toml`. Do not add a compiler version to a workflow, hosted setup or runner image.
`Cargo.toml`'s `rust-version` is the separate consumer floor. The scheduled/manual
`latest-stable.yml` workflow is advance compatibility feedback, not permission to release under a
moving compiler; the reviewed update procedure is in
[`ci/README.md`](ci/README.md#updating-rust-deliberately).

Install the exact test runner once with `ci/scripts/install-nextest.sh install`; the shared test
gate refuses an absent or merely compatible build before compilation. Direct
`cargo test --workspace` remains a supported diagnostic fallback when isolating runner behavior,
but it is not the repository's test-gate verdict: it omits the pinned runner contract, inventory
and mutex census, run-owned cache/ledger, and CLI-surface audit. A bare `cargo test` is weaker still:
`default-members = ["crates/aplan"]`, so it skips every other workspace crate and has already
caused a false pass in review.

## Tests

The contract is in [`AGENTS.md`](AGENTS.md) and applies to humans equally. In short: CI
rejects a rule with no failing case, a rule with no unit test in the crate that raises it,
and any crate below its floor in `coverage.json`. `cargo xtask coverage` prints every
crate's measured percentage and banks a gain; it refuses to lower a floor without
`--allow-regression`.

None of those gates prove the tests are good. Coverage records which lines ran, not which
claims were checked: `APLAN023` shipped with the required scenario pair, full line coverage,
and three defects in the inputs beside the one it was written for (`A1-043`). When you add a
check, vary one input at a time across the neighbours (empty, blank, whitespace, zero-width,
zero/one/many, each terminal status, milestones), assert on the message and the
`agent_instruction` rather than only the code, and write the failing case first.

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
4. Add **unit tests in the crate that raises it**, covering the inputs next to the one the
   rule was written for and asserting on the message and `agent_instruction`, not only the
   code. `crates/aplan-e2e/tests/rule_tests.rs` fails the build if a rule has no test naming
   it in its emitting crate; the registry guard in `diagnostic.rs` does not count, because it
   lists every code by construction. This step exists because `APLAN023` satisfied step 3
   and was still wrong three ways (`A1-043`, `A1-046`).
5. Update the rule table in `README.md` and the catalog in `HLD.md §6`.
6. Regenerate `facts.json` (`cargo run -p aplan -- facts > facts.json`): it lists every rule
   with its severity, derived from the code, and `tests/facts.rs` fails CI if the committed
   file is stale.
7. Note it in `CHANGELOG.md`.

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
