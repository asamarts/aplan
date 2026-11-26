# aplan

`aplan` is an evidence-gated validator for plans stored in Git. It checks task structure,
dependencies, release cuts, traceability, and the evidence behind completed work. Its core rule is
simple: a status is a claim, not proof.

Published releases are available on [GitHub](https://github.com/asamarts/aplan/releases) and
[crates.io](https://crates.io/crates/aplan). Building from source requires Rust 1.88 or newer.
Release assets and the GitHub Action currently provide a Linux x86_64 binary.

Release archives after v0.5.2 carry keyless build provenance. Verify a downloaded archive with
`gh attestation verify ARCHIVE --repo asamarts/aplan --signer-workflow
asamarts/aplan/.github/workflows/release.yml --source-ref refs/tags/vX.Y.Z
--deny-self-hosted-runners`. v0.5.2 and earlier predate this provenance contract.

## Start

Install from crates.io:

```sh
cargo install --locked aplan
```

Then initialize a repository and add work. `aplan init` writes only `.aplan.yml`, so create the
plan directory before the first `add`:

```sh
aplan init
mkdir -p aplan/tasks
aplan add "Define the release gate" --id P1-001 --effort 1 --tag cross-cutting
aplan check
```

Plans that require every task to belong to a release cut should use the atomic first-plan recipe in
the [quickstart](docs/quickstart.md).

## Work loop

```sh
aplan ready
aplan start P1-001
# implement and test
aplan close P1-001 --evidence test:REGISTERED_TEST_NAME
aplan check
aplan verify
```

`check` validates the plan without external work. `verify` resolves evidence against the repository.
To preflight live task `paths` or wire a custom gate, run `aplan paths match <GLOB>...` or
`aplan paths match --task <ID>` (`--format json`, `--limit N` / `--all`) instead of reimplementing
glob semantics, and run `aplan paths` to inspect bounded commit history; neither proves semantic
relevance. `aplan show <id> --body` pretty-renders task Markdown for the terminal (`--width`, `--ascii`), while
`aplan show <id> --raw-body` emits the verbatim stored Markdown bytes for pipes and editors.
`aplan completed` lists finished work in deterministic chronological order (`recorded` timestamps
first, with optional bounded Git history inference for legacy `done` tasks). Use the built-in
command help and `aplan facts` for the complete current surface.

## GitHub Actions

Grant the workflow read access to both release assets and their provenance:

```yaml
permissions:
  contents: read
  attestations: read
```

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0
- uses: asamarts/aplan@v0.7.0
  with:
    command: verify
```

Set `deny_warnings: true` in `.aplan.yml` when warnings should fail CI. Pin actions to full commit
SHAs in production repositories. The Action retries transient HTTP 5xx and network download errors
with short backoff, never retries HTTP 404 or verification failures, and verifies releases from
v0.6.0 onward against the publishing repository, `release.yml` and the exact release tag before
executing the binary.

### Self-hosted runners

GitHub-hosted Linux runners ship with everything the Action's install step needs. On a self-hosted
runner, make sure the following are available on `PATH` before the first run:

- **Platform:** Linux x86_64 (the only platform the release assets provide).
- **Shell:** `bash` (both Action steps run with `shell: bash`, and the install helpers run under
  `env bash`).
- **GitHub CLI (`gh`):** version **2.68.0** or newer, with **2.102.0** or newer recommended. The
  install step downloads the archive with `gh release download` and verifies releases from v0.6.0
  onward with `gh attestation verify --signer-workflow ... --source-ref ...`; both flags first
  shipped together in `gh` 2.68.0, so older distro packages are not enough. `gh` 2.102.0 or newer
  is recommended because 2.97.0 and 2.102.0 fixed matcher weaknesses in those flags and 2.102.0
  fixed a symlink issue in `gh release download`.
- **Standard tools:** coreutils (`sha256sum`, `uname`, `mkdir`, `chmod`, `wc`, `basename`, `tr`,
  `sleep`), `tar` with `gzip`, and `awk`. `curl` is not required.

If a prerequisite is missing, the job fails in the install step before any plan check runs. An
unsupported platform or a missing `gh` produces an explicit install-step error; another missing
command fails with command-not-found, and a `gh` older than 2.68.0 fails during verification with
an unknown-flag error.

## Other interfaces

`aplan tui` opens the terminal interface. `aplan mcp` exposes the same guarded plan operations to
MCP clients under contract `aplan/mcp/v2` (generated input/output schemas, object-enveloped
`structuredContent` + concise text on `2025-06-18+` with legacy JSON text compatibility on
`2024-11-05`/`2025-03-26`, and bounded `aplan://` resources including plan summary, focused task
detail, ready tasks, and completed-task history with snapshot-bound pagination and private
zero-freshness cache metadata). Machine-readable JSON, SARIF, schemas, facts, and generated
agent guidance (`aplan export-agents-md`, with optional `agent_contract.invocation` in `.aplan.yml`
for repositories that pin `aplan` behind a checked-in launcher) are also available.

## Documentation

- [Quickstart](docs/quickstart.md)
- [Rule catalogue](docs/rule-catalogue.md)
- [Design and CLI reference](HLD.md)
- [Upgrading](docs/upgrading.md)
- [TUI guide](docs/tui.md)
- [Contributing](CONTRIBUTING.md)

Licensed under Apache-2.0 or MIT.
