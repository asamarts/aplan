# YAML parser migration

Status: the bridge is shipped; `A2-009` owns a new public qualification and the safe Rust cutover.

## Decision

aplan now has one YAML ownership boundary: the root manifest's logical `yaml_compat` dependency. The immediate backend is exact
`yaml_serde` 0.10.7 over `libyaml-rs` 0.3.0. This removes the archived `serde_yaml_ng` /
`unsafe-libyaml` stack and `RUSTSEC-2024-0449` waiver while preserving the mature `serde_yaml`
value and error contract.

This is deliberately a bridge. `libyaml-rs` is maintained by the YAML organization, but it is
still libyaml translated from C to unsafe Rust. `A2-009` owns the public safe Rust cutover to
`d0026`. No caller names the selected backend;
changing it again is one root dependency change whose compiler errors enumerate the compatibility
work, rather than another backend-name sweep. The private `aplan-core` contract module pins the
behavior without adding a public facade or reversing the workspace's dependency-sink architecture.

## What aplan actually needs

The pre-migration workspace had 236 qualified uses across product crates, release tooling, tests
and the opt-in TUI evaluator: 147 `from_str`, 80 `Value`, eight `to_string`, four `Error` and three
`Mapping` uses. The syntactic count understates the contract. aplan also depends on:

- typed Serde decoding with useful nested errors and one-based source locations;
- a mutable ordered generic mapping with string indexing and borrowed-key lookup;
- `Value` acting as a Serde deserializer for configuration inspection;
- aliases and anchors materializing compatibly, while custom tags remain represented;
- duplicate fields and generic duplicate keys being rejected rather than silently replaced;
- merge syntax remaining a literal `<<` key unless a caller explicitly applies merge behavior;
- YAML 1.2-like scalar resolution: `yes`, dates and leading-zero decimals remain strings;
- exactly one document in convenience APIs and invalid UTF-8 refusal at byte inputs;
- deterministic serialization and semantic parse/mutate/serialize/parse checks; and
- Rust 1.88, dual MIT/Apache-compatible licensing and acceptable parse, memory and binary costs.

Task frontmatter, `.aplan.yml`, user configuration, import manifests, KEP import, release and CI
workflow audits, surgical configuration mutation and the TUI evaluation harness all use that same
boundary. A parser that serves only typed configuration is not enough.

## Options evaluated

| option | strengths | blocking gaps | disposition |
|---|---|---|---|
| `yaml_serde` 0.10.7 / `libyaml-rs` 0.3.0 | YAML-organization maintenance; Rust 1.82/1.60 minimums; MIT/Apache and MIT; source compatible with all aplan calls; mature value, error and emitter behavior | parser remains an unsafe C2Rust translation | selected as the bounded bridge |
| local `d0026` at clean immutable commit `5a9db923` | aplan-compatible safe parser goal; own scanner, events, values, Serde and emitter; native limits; Rust 1.88; 405/405 YAML Test Suite classification, 100,000 generic and 50,000 typed differential cases, and 938 mutually accepted external YAML files with no normalized differences; committed parse-throughput and peak-heap gates pass | private and `publish = false`; explicitly no current license grant; first product integration has not started; API gaps below; deep emission remains 2.40 times the bridge; the P6 status page has not caught up with the newer D8 result | target for `A2-009`, not a dependency yet |
| `saphyr-parser` / `saphyr` | active pure-Rust parser code, events and good throughput | parser dependency uses unsafe `arraydeque`; no Serde at parser layer; character rather than byte spans in relevant paths; no native policy limits; high-level deep load recurses; incompatible value model | reject as runtime; useful only as a test oracle |
| `serde-saphyr` | typed Serde, budgets and serializer | no compatible `Value`, `Mapping` or `from_value` surface; tested versions either had a depth-64 default or exceeded one of the relevant MSRV baselines; measured prototype slower | reject |
| `noyalib` 0.0.57 | broad `serde_yaml` compatibility, values, spans and limits; own safe parser code | pre-1.0 churn; unsafe general-purpose dependencies; default alias policy rejected an accepted shape; measured prototype about 2.2 times the bridge | reject for now |
| `yaml-rt-serde` 0.3.3 | broad values, conversion, limits and merge adapter | Rust 1.96 minimum exceeds aplan's 1.88 promise; young surface and no aplan compatibility evidence | reject |
| aplan-specific YAML subset | smallest apparent implementation | existing plans and importers already use tags, aliases, flow/block forms and generic mutations; calling a subset YAML would silently narrow the format | reject |
| a second in-house parser | full control | duplicates the active `d0026` effort and splits security/conformance ownership | reject; contribute requirements to `d0026` |

The alternative measurements above come from the reproducible evaluations in the sibling
`d0026` repository. aplan independently compiled and tested both serious candidates rather than
accepting the table on trust.

## Empirical integration results

### Maintained bridge

Renaming the workspace dependency to `yaml_serde` 0.10.7 required no Rust source adaptation. The
entire aplan test inventory passed: 3,375 tests passed and eight were deliberately skipped. That
result includes task/config/import parsing, mutation byte preservation, source diagnostics,
generated artifacts, CLI/TUI behavior and release tooling. The `yaml_compat` contract adds direct
coverage for aliases, anchors, custom tags, literal merge keys, duplicate keys, multi-document
refusal, scalar resolution, invalid UTF-8, depth 128, 1,000 aliases and stable serialization.

### Direct d0026 spike

A clean aplan worktree replaced the old dependency with the local `d0026` crate. Product crates
nearly compiled after these small adaptations:

1. pass an additional reference to `to_string` because `d0026` currently requires a sized type;
2. replace three `T::deserialize(value)` calls with `d0026::from_value(value)`; and
3. borrow two owned `Mapping` lookup keys.

The complete workspace did not compile. Release tooling and contract tests additionally require
`Value::get` / `get_mut`, lookup by `&str`, `Value == &str` / integer comparisons and ergonomic
borrowed mapping keys. These are normal compatibility conveniences, not evidence of a parser
semantic defect, but they make the current crate non-drop-in. They should be solved at the d0026
API or behind the logical `yaml_compat` boundary, never by scattering backend conditionals through aplan.

Those scanner and line-index optimizations are now committed at `5a9db923`, and the checkout is
clean. aplan re-ran the immutable revision on 2026-10-29: all 223 workspace tests including its
doctest pass, as does all-target Clippy with warnings denied. The direct-spike API result above is
unchanged because the newer commit touches only position/scanner implementation and D8
documentation. Publication, licensing, the remaining compatibility conveniences and real product
integration are the substantive blockers.

### Current product qualification

Upstream commit `73ebb6e` adds a private `yaml_serde` facade and closes the earlier compile-time
convenience gaps. Its immutable Git archive hashes to
`13edc5c469e6d37660a9a4354b601067ac87d832e2ee8d16ee8f26475d31000d`. In a detached aplan
worktree, replacing the bridge with that facade and removing the resolver-only engine pin made
`mbx check --workspace --all-targets` pass. The normal graph contained `d0026` and no
`libyaml-rs`, and all 59 standalone TUI-evaluator tests passed.

That compile result is not product compatibility. The complete nextest inventory ran 3,383 tests:
3,201 passed, 182 failed and eight were skipped. The direct YAML contract passed four of six tests.
The failures have a small number of repeated causes with broad product effects:

- Serde unit enums serialize as tags (`status: !todo`, `kind: !test`) rather than aplan's scalar
  task representation. Generated tasks then fail their own schema and mutation checks.
- generic `from_str::<Value>` drops custom tags even though `parse_value` retains them;
- empty or comment-only configuration becomes `null` rather than the default configuration map;
- at least one scalar-resolution difference changes a real configuration field's type; and
- duplicate-key source locations disagree with the compatibility contract.

Those differences break CLI authoring and mutation, import/export, configuration, MCP and TUI
mutation parity, release tooling and real-terminal journeys. They are upstream compatibility
work, not assertions to relax. The qualification stopped before performance and binary acceptance
because the semantic product gate had already failed.

The licensing gate independently blocks consumption. At `73ebb6e`, both workspace and package
metadata still set `publish = false`, the repository explicitly grants no license while private,
and it contains no license files. aplan neither copied nor vendored that source. `A2-009` can resume
only from a licensed immutable release whose compatibility revision passes the complete product
inventory; it must then run the differential, hostile-input, performance, binary and release gates
below.

## d0026 readiness gates

`A2-009` may replace the bridge only when all of these are true:

1. **Immutable consumption.** A published crate or immutable vendored source has an explicit
   `MIT OR Apache-2.0` grant, provenance, checksum and Rust 1.88 support. A sibling path or moving
   Git branch is not a releasable dependency.
2. **API fit.** The direct-spike gaps above are closed behind `yaml_compat`; error locations,
   values, mapping mutation and deterministic emission remain usable without backend knowledge in
   callers.
3. **Semantic differential.** A test-only bridge oracle compares every repository YAML fixture,
   generated document and mutation round trip for acceptance, normalized typed/generic value,
   essential error category/location and emitted semantics. Tags and deliberate representation
   differences need explicit dispositions, not blanket exclusion.
4. **Hostile-input limits.** Input bytes, collection depth, document count, aliases, expanded
   nodes/bytes, collection entries, scalar bytes, events and output bytes have explicit defaults.
   Each boundary and its first refused neighbor runs through a real aplan entry point. No public
   convenience path bypasses the limits or recurses to native-stack exhaustion.
5. **Product acceptance.** The full aplan workspace, TUI evaluator, schema/roadmap generation,
   documentation, supply-chain, MSRV and exact release gates pass. Normal dependency trees contain
   neither `yaml_serde` nor `libyaml-rs`; an oracle may retain them only as test dependencies.
6. **Performance.** On the controlled d0026 benchmark host, two runs meet the existing parse gates:
   representative at most 1.10 times the bridge, depth and aliases at most 1.25 times. Commit
   `5a9db923` passes at 0.899, 0.971 and 1.054 respectively, and every committed peak-heap ratio is
   at most 1.25. Depth-128 emission remains a measured 2.40 times the bridge; it is report-only
   upstream, but the integration must either remove the gap or show that aplan's product-level
   latency gates make it immaterial. In aplan, representative read-only commands retain their
   sub-100 ms warm objective, parser-heavy full validation regresses no more than 10 percent, and
   stripped default/headless binaries stay within the committed dependency budgets.
7. **No split lifetime.** Dual parsing is a test/qualification mode, not a permanent runtime mode.
   Cutover removes the bridge from normal graphs in one change and retains the narrow facade.

## Bridge removal and rollback

Both bridge layers are exact: `yaml_serde` is pinned to 0.10.7 and a private resolver-only
`yaml_engine_pin` holds `libyaml-rs` at 0.3.0. The executable dependency audit rejects either
requirement becoming compatible/floating, so a lock refresh cannot silently change plan semantics.
Rollback before the safe cutover is one root dependency revision; callers
do not move. The d0026 cutover keeps a short-lived test oracle and a last-known-good bridge commit,
but release binaries contain one parser. A semantic, resource or performance gate failure rolls
back the adapter change rather than weakening the gate.
