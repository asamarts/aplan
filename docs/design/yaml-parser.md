# YAML parser migration

Status: cut over. Since `A2-009` the product parses YAML with the safe Rust `ayaml` 0.1.3 behind
`yaml_compat`; the `yaml_serde` bridge survives only as a test oracle. Earlier sections call the
parser `d0026`, the name it was evaluated under before it was published as `ayaml`.

## Decision

aplan has one YAML ownership boundary: the root manifest's logical `yaml_compat` dependency. Its
backend is exact `ayaml` 0.1.3 with the `libyaml-compat` feature (see "Cutover to ayaml 0.1.3"
below). Before that cutover it was exact `yaml_serde` 0.10.7 over `libyaml-rs` 0.3.0. This removes the archived `serde_yaml_ng` /
`unsafe-libyaml` stack and `RUSTSEC-2024-0449` waiver while preserving the mature `serde_yaml`
value and error contract.

That was deliberately a bridge. `libyaml-rs` is maintained by the YAML organization, but it is
still libyaml translated from C to unsafe Rust. `A2-009` owned the public safe Rust cutover to
`d0026`, published as `ayaml`. No caller names the selected backend;
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

## Cutover to ayaml 0.1.3

`ayaml` was published on crates.io under `MIT OR Apache-2.0` with `rust-version = "1.88"`, which
met readiness gate 1. Version 0.1.2 added compatibility entry points that take caller limits and
bounds `%TAG` expansion. 0.1.3 reads a local tag after a flow `?` the way libyaml does in
compatibility mode. `A2-009` consumes 0.1.3 directly, through the same `yaml_compat` name:

- **Callers.** Most callers read through `yaml_compat::compat::from_str`, which follows libyaml's
  resolution: task front matter, configuration, imports and cuts, the release and CI audits, and
  the TUI evaluation harness. The readers of formats aplan defines itself (captures, blueprints, dependency
  manifests) and a few tests use `ayaml`'s default reader. `add --from` manifests are read with the
  compatibility reader, although a blueprint that embeds the same entries uses the default one;
  `A2-166` records the split rather than changing what either accepts. Native limits are built
  from `ayaml`'s own options rather than a wrapper.
- **Oracle.** `aplan-core` and `aplan-config` keep `yaml_serde` 0.10.7 as a dev-dependency named
  `yaml_serde_baseline`, used only by the differential tests. The dependency audit pins `ayaml`
  to `=0.1.3` and refuses any product graph that resolves `yaml_serde`, `libyaml-rs` or the
  parsers retired before them.
- **Generic differential.** Every YAML file in the repository and every task's front matter, 201
  documents when this ran, parse to the same normalized value under both parsers, through
  `ayaml`'s compatibility reader and its default reader alike. `ayaml`'s emission reparses under
  the bridge to the same value, and no document is accepted by one and refused by the other.
- **Typed differential.** Every task's front matter decodes to the same `serde_json::Value` and
  the same typed `Task` under both parsers, the two reads the loader makes. Every configuration
  document `aplan-config` owns (the one naming every accepted key, each scaffold example and this
  repository's `.aplan.yml`) decodes to the same `Config`.
- **Hostile input.** 24 hostile or degenerate documents are read by both parsers four ways: as a
  generic value, as JSON and as a task through the compatibility reader, and as JSON through the
  default reader. They cover empty and comment-only documents, duplicate keys, extra documents,
  broken flow collections, tabs, undefined and self-referencing aliases, tags, a byte order mark,
  a control character, out-of-range integers, non-string keys, merge keys, redefined anchors, a
  64 KiB scalar, 10,000 levels of nesting and an alias bomb. 89 of the 96 outcomes agree. The
  test pins the other seven exactly, so a new divergence fails it. None lets aplan accept a value
  the bridge refused or read one differently, but the tags below make it refuse values the bridge
  read:
  - The default reader refuses a duplicate key, at the top level or nested, where the bridge's
    JSON read kept the last value. It also refuses a flow collection whose continuation line is
    not indented, which YAML forbids and libyaml allowed; the compatibility reader still accepts
    it. Captures, blueprints and dependency manifests are read this stricter way.
  - `ayaml` keeps a tag outside YAML's core schema on a generic value, where the bridge dropped
    it and read the scalar as a plain string. That covers a `%TAG` handle and also tags that need
    no directive: `!!timestamp`, `!!binary` and a verbatim tag naming another vocabulary, such as
    `!<tag:example.com,2000:x>`. Core tags (`!!str`, `!!int`, `!!float`, `!!bool`, and their
    verbatim forms) resolve the same under both parsers. Every product read
    that goes through a generic or JSON value therefore refuses such a document where the bridge
    read the string: the loader's front matter read, the fields the KEP importer does not model,
    and the capture reader. It reaches task files as well as captures, since those tags need no
    directive, and since the front matter ends only at a line that is exactly `---`, so a
    `--- # note` line keeps a directive inside it. A local tag such as `!custom` is refused by
    both, and typed string fields, such as a KEP's `title`, ignore the tag under both parsers. The test pins only the `%TAG` case; `A2-169`
    owns restoring the bridge's reading and pinning the rest.
  - `ayaml`'s generic value holds an integer past the 64-bit range, which the bridge refused.
    Read as JSON or as a task, both refuse it, and the KEP importer, the one product read that
    keeps a generic value, refuses it itself (`A2-166`).
- **Contract and limits.** The contract tests pin aliases, anchors, tags, literal merge keys,
  duplicate keys and extra documents with their locations, scalar typing, invalid UTF-8, depth
  and alias stress, mapping serialization, and empty documents. An empty document reads as an
  empty collection or a struct's defaults; an explicit `~` is refused for both. The limit tests
  set each of `ayaml`'s native ceilings, the tag-expansion one since `A2-166`, and prove its
  boundary and first refused neighbour, and the product task loader still refuses nesting and an
  alias bomb past the defaults.
- **Errors.** Parser errors keep their line but may name a different column and cause. Two
  messages in the rule catalogue changed; for a flow sequence whose continuation is not
  indented, the bridge reported column 6 and "did not find expected ',' or ']'", and `ayaml`
  reports column 1 and "flow collection entries must be indented". The catalogue and the
  command-output tests were regenerated rather than asserted away.
- **Capture guard.** A capture's duplicate key used to pass the schema check, because the bridge's
  JSON read kept the last value, and was refused only by the typed read after it. The default
  reader now refuses it at the JSON read. The typed read is still the only guard for a value the schema admits and the types
  cannot hold, such as a `promotion_defaults.phase` past `u32::MAX`, and a test now covers that.
- **Package budgets.** The lock gains `ayaml` and keeps the oracle: 291 packages. Both product
  graphs replace `yaml_serde` and `libyaml-rs` with `ayaml`, and the headless graph also loses
  `ryu`, which only the bridge used there. Default goes from 201 to 200 packages and headless from
  171 to 169.
- **Binary size.** Stripped release binaries, built the same way from the same revision with
  Rust 1.99.1, grow by about 588 KB with `ayaml`: default 21,799,912 to 22,388,096 bytes (+2.7%),
  headless 19,400,736 to 19,989,704 bytes (+3.0%). The default-over-headless increment that the
  dependency policy budgets stays at 2,398,392 bytes (12.0%), inside its 2,500,000-byte and 15%
  limits.
- **Speed.** On the quiet benchmark host, through its one-slot queue, both revisions were built
  there with Rust 1.97.0 and timed with hyperfine (5 warm-up runs, 40 timed). Two corpora were
  used: this plan (171 tasks) and the larger pre-public ledger (758 tasks). `--cache off` forces
  a full parse; `--cache auto` measures the warm path a repeated read takes.

  | Read | Plan | Bridge | `ayaml` | Change |
  |---|---|---|---|---|
  | `check`, cold | 171 tasks | 33.9 ms | 32.7 ms | -3.4% |
  | `check`, cold | 758 tasks | 170.6 ms | 165.1 ms | -3.2% |
  | `ready`, cold | 171 tasks | 26.6 ms | 25.8 ms | -2.9% |
  | `ready`, cold | 758 tasks | 135.4 ms | 131.6 ms | -2.8% |
  | `ready`, warm | 171 tasks | 12.4 ms | 12.5 ms | +1.2% |
  | `ready`, warm | 758 tasks | 46.7 ms | 47.2 ms | +1.0% |

  Cold reads retire 4.5 to 6.6% fewer instructions. Warm reads retire the same number, because
  the cache answers them without parsing, so their 1% is noise. Peak resident memory rises by
  about 0.4 MiB. Both binaries print byte-identical output with the same exit status for every
  row. Each corpus was the plan directory alone, so `check` reports the documents its tasks
  trace as missing and exits 1 under both parsers alike. The qualification limits were a
  regression of at most 10% and warm reads under 100 ms; both hold.

## d0026 readiness gates

`A2-009` could replace the bridge only when all of these were true. How the cutover met each:

- **1.** `ayaml` 0.1.3 is published on crates.io with the grant, Rust 1.88 support and a lockfile
  checksum. Its changelog records Trusted Publishing with a build provenance attestation for every
  release from 0.1.2 on.
- **2.** Callers name only `yaml_compat`. The spike's compile gaps are closed in `ayaml`'s API, and
  each product failure recorded above is now covered. The contract tests pin kept custom tags,
  empty documents as defaults, scalar typing and duplicate-key locations. The task emitter's
  byte-level tests pin enums written as plain scalars (`status: todo`). The complete
  workspace suite passes.
- **3.** The generic, typed and hostile-input differentials above, with every tag and
  representation difference pinned with a reason, except the tags that need no directive
  (`A2-169`). The command-output corpus the bridge build produced passes unchanged apart from the
  two parser messages above, but it prints no front matter, so it does not pin emitted files byte
  for byte. `ayaml` quotes the strings that need it with double quotes where the bridge used
  single quotes (41 of 68 awkward strings in one sweep), and each reads back as the same value.
- **4.** Partly met. Every ceiling has an explicit default, and aplan overrides none, so every
  product read applies them. Each boundary and its first refused neighbour is proven against
  `yaml_compat` with explicit values, which proves `ayaml`'s mechanism rather than aplan's reads.
  Through a product entry point, the task loader proves the depth boundary and its first refused
  neighbour and refuses an alias bomb, and `parse_config` and the task emitter refuse input past
  the defaults for anchors, alias dereferences, events, documents, input bytes and output bytes
  without proving the accepted boundary. Scalar bytes, collection entries and tag expansion never
  reach a product read in a test. `A2-173` owns the rest.
- **5.** The complete local profile passes on the change. `A2-009` closes only after the hosted
  complete profile passes on `main`, and the dependency audit refuses a product graph holding
  either bridge crate. The TUI evaluator, a separate workspace that no gate built, was missed: its
  manifest still named the bridge under `yaml_compat`, so it did not build until `A2-166` moved it
  to `ayaml` and made the dependency audit check its lock and parser.
- **6.** The product-level gates hold, as measured above. aplan emits only its own records (task
  front matter, evidence, captures and configuration), each a few levels deep, so the 2.40 times
  depth-128 emission ratio reaches no product path.
- **7.** One change removes the bridge from the product graphs, and the oracle is test-only.

The gates as they were set:

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

Before the cutover both bridge layers were exact: `yaml_serde` was pinned to 0.10.7 and a private
resolver-only `yaml_engine_pin` held `libyaml-rs` at 0.3.0. After it, the executable dependency
audit pins `ayaml` to `=0.1.3` and rejects a compatible or floating requirement, so a lock refresh
still cannot silently change plan semantics. Rolling back is reverting the `A2-009` change: the
bridge comes back as one root dependency revision, and the callers that moved to
`yaml_compat::compat` move back with it. The d0026 cutover keeps a short-lived test oracle and a last-known-good bridge commit,
but release binaries contain one parser. A semantic, resource or performance gate failure rolls
back the adapter change rather than weakening the gate.
