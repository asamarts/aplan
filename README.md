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

Then initialize a repository and add work:

```sh
aplan init
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
Use the built-in command help and `aplan facts` for the complete current surface.

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
- uses: asamarts/aplan@v0.6.0
  with:
    command: verify
```

Set `deny_warnings: true` in `.aplan.yml` when warnings should fail CI. Pin actions to full commit
SHAs in production repositories. The Action verifies releases from v0.6.0 onward against the
publishing repository, `release.yml` and the exact release tag before executing the binary.

## Other interfaces

`aplan tui` opens the terminal interface. `aplan mcp` exposes the same guarded plan operations to
MCP clients. Machine-readable JSON, SARIF, schemas, facts, and generated agent guidance are also
available.

## Documentation

- [Quickstart](docs/quickstart.md)
- [Rule catalogue](docs/rule-catalogue.md)
- [Design and CLI reference](HLD.md)
- [Upgrading](docs/upgrading.md)
- [TUI guide](docs/tui.md)
- [Contributing](CONTRIBUTING.md)

Licensed under Apache-2.0 or MIT.
