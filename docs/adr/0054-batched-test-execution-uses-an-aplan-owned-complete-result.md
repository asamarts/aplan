# ADR-0054: Batched test execution uses an aplan-owned complete result

Date: 2026-10-23
Status: Accepted; outcome vocabulary superseded in part by ADR-0056
Tasks: `A1-642`

## Context

`aplan verify --execute` historically started `test.run` once for every distinct cited test name.
Deduplicating repeated citations made the cost proportional to distinct names, but one measured
repository still had 199 names across eight runners and started 215 provider processes: eight
catalogues, 199 exact tests and eight no-match probes. The frameworks already amortize startup and
collection when asked for several tests, but their selectors and result formats are not one
portable contract.

Passing several names on a command line is not enough. Names can be hostile argv values, command
line length is bounded, a framework can silently select nothing, and a process-wide exit status
cannot say which of several tests failed or skipped. Native JSON, JUnit XML and TAP also report
what a framework observed; none binds a complete answer to aplan's exact request, runner and
invocation.

## Decision

**1. Batching is an optional versioned adapter beside the legacy command.** A runner may add:

```yaml
batch:
  protocol: 1
  run: ["./ci/aplan-test-batch", "{manifest}", "{result}"]
```

`run: ... {test}` remains required. It is the fallback for configurations without `batch` and
allows a new configuration to be rolled back to an older execution strategy without rewriting
task evidence. An older aplan build refuses the unknown `batch` key, so the binary is upgraded
before the configuration as for every other new key. There is no deprecation date for the legacy
path; adoption evidence, not the existence of a faster path, will decide one.

Both batch placeholders occur exactly once as whole, non-command argv elements. Test identities
never enter argv. The adapter is trusted repository configuration and may start framework tools;
task-authored files can select only identities that `test.list` already returned.

**2. The input is `aplan/test-batch-manifest/v1`.** It is a bounded JSON object with no unknown
fields:

```json
{
  "schema": "aplan/test-batch-manifest/v1",
  "nonce": "64 lowercase hex characters",
  "provider": "64 lowercase hex characters",
  "tests": ["exact listed identity", "another identity"]
}
```

The tests are sorted, deduplicated and exact. The provider digest binds the runner's position and
complete configuration. The unpredictable nonce binds one result to one invocation. A request is
refused before launch when it is empty, exceeds 100,000 identities, contains an identity over 16
KiB, or may exceed the 32 MiB encoded-document bound.

**3. The output is `aplan/test-batch-result/v1`.** The adapter publishes one JSON object at the
initially absent result path:

```json
{
  "schema": "aplan/test-batch-result/v1",
  "nonce": "the manifest nonce",
  "provider": "the manifest provider",
  "complete": true,
  "tests": [
    {"test": "exact listed identity", "outcome": "passed"},
    {"test": "another identity", "outcome": "failed", "diagnostic": "bounded context"}
  ]
}
```

Outcomes are `passed`, `failed`, `skipped`, `stopped` and `unavailable`. Every requested identity
appears exactly once and no other identity appears. A missing, partial, duplicate, extra, malformed
or unknown-version result proves nothing. `skipped`, `stopped` and `unavailable` are findings, never
passes. A nonzero adapter exit refuses every row even if the file claims passes. This complete
requested/result equality replaces the legacy output-comparison probe: an adapter may emit
`passed` only for an identity its native result attributes as executed and passing. Selecting
nothing therefore produces missing or non-passing rows, not evidence.

**4. The filesystem boundary fails closed.** Every invocation gets a private temporary directory,
a mode-0600 manifest on Unix and an absent result path. Aplan reopens without following a symlink
on Unix and requires the open file to match the path before and after reading. It rejects a
symlink, hard link, owner change, replacement, manifest mutation, wrong nonce, wrong provider and
documents over 32 MiB before JSON parsing. After that bounded parse, each decoded adapter diagnostic
is limited to 64 KiB before it reaches the resolver, then passes through the existing line bound,
visible-text sanitizer and credential redactor before any finding. The private path itself does not
enter a diagnostic.

Adapters should write a sibling temporary result, flush it, and rename it to the supplied absent
path only after every row is final. Aplan still checks completeness after a clean adapter exit, so
a crash, torn write or adapter that publishes early cannot become a clean verdict.

**5. One adapter process owns one runner's complete request.** Aplan lists every runner first so
cross-runner ambiguity is still refused before execution. It then starts at most one batch adapter
for each runner that owns at least one unambiguous cited name. The adapter may internally use
bounded chunks or framework workers, but it publishes one complete result. If it cannot attribute
a chunk failure, every affected request is `unavailable`; it does not infer passes from the other
chunks. Outcomes fan out to every task citation while launch and resource cost remain aggregated
under the runner.

The adapter uses the same repository-relative directory, wrapper, memory/process limits, bounded
stdout/stderr capture and process-group cancellation owner as `list` and legacy `run`. Test
execution remains deliberately unbounded by `test.timeout_seconds`; that key bounds discovery,
while a framework or adapter owns execution timeouts. The optional `test.skipped` discovery still
runs and can only make the result stricter.

**6. Framework translation stays outside the evidence core.** The adapter must translate exact
listed identities to the framework and consume a native structured result when one exists:

- pytest accepts exact node ids such as `module.py::class::test` and multiple selectors. The
  adapter supplies every manifest id, reads its structured/JUnit result and emits one row per id.
- Vitest's CLI file filter and `--testNamePattern` address different namespaces than `vitest list`
  output. The adapter groups by file, uses the full-name field from its native JSON reporter for
  attribution, and never treats a successful filter with no matching result as a pass.
- Playwright accepts several test files and title filters and can shard internally. The adapter
  maps manifest identities to file/title selections and merges its blob or JSON reports only when
  every requested identity has one terminal outcome.

These are adapter responsibilities rather than built-in parsers. Framework releases can change
their native formats without changing aplan's evidence protocol, and a repository can pin and test
its adapter with its framework. The primary selector references are pytest's specifying-tests
guide, Vitest's filtering and CLI guides, and Playwright's test CLI and sharding guides, all linked
from `A1-642`.

## Consequences

- The deterministic P0000021-shaped fixture performs eight listings and eight batches for 199
  distinct tests and 241 citations, rather than 215 provider starts. A runner without `batch`
  retains one legacy launch per distinct name and one no-match probe.
- Batching changes execution strategy, not evidence identity, ambiguity, per-task findings or the
  L3 definition. CLI, MCP and TUI consume the same resolver outcomes.
- A repository must own a small adapter. Aplan deliberately does not guess how a framework spells
  identities, retries, parameterization, skips or structured reports.
- The adapter is part of trusted top-level policy. Results from an earlier invocation, another
  runner or an incomplete execution cannot be replayed as current evidence.
- Portable cross-job receipts remain separate work. This protocol is their result vocabulary and
  direct-execution boundary, but it does not authenticate a CI producer or make an old execution
  current; `A1-644` through `A1-647` own those claims.

## Alternatives

| option | why not |
|---|---|
| Pass every test name directly on argv | command-length limits, hostile values and framework-specific selector namespaces remain, and one exit status cannot attribute outcomes |
| Parse framework console output in aplan | output is presentation, changes by version and locale, and would make the evidence core own every framework |
| Accept JUnit XML or TAP directly | neither binds an exact requested set, nonce and provider; dialects also lose or merge identities |
| Treat a zero batch exit as every test passing | a selector miss becomes a false green and one failure cannot be attributed honestly |
| Replace legacy `run` immediately | forces every adopter to write an adapter before upgrading and removes the compatible fallback before field evidence exists |
| Start one adapter per chunk | makes a configured "batch" silently return to process-count growth and complicates completeness across partial results |
