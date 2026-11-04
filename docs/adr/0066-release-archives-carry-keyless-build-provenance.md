# ADR-0066: Release archives carry keyless build provenance

Date: 2026-11-04

Status: Accepted

Supersedes nothing. Extends ADR-0065 and the release transaction delivered by `A2-011` and
`A2-012`.

## Context

The public release transaction already has four distinct controls. A protected environment admits
one exact source, the complete profile validates it, a digest-pinned builder makes the Linux
archive reproducible, and a SHA-256 sidecar detects changed bytes. The archive and sidecar are both
ordinary release assets, however. A consumer who obtains both from the same compromised location
has no separate proof that their digest came from this repository's release workflow.

The crates.io transaction uses OIDC to authenticate publication, but that credential is an
authorization to upload packages. It is not a consumer-verifiable statement about the GitHub
archive. A checksum, reproducible build, release approval, registry credential, and build
provenance answer different questions and none substitutes for another.

GitHub artifact attestations bind an artifact name and digest to an in-toto SLSA provenance
predicate. A public repository receives a short-lived certificate from Sigstore's public-good
instance through the job's GitHub OIDC identity. No durable signing key is created or stored.

## Decision

**1. The subject is the one compressed release archive.** The `dist/*.tar.gz` archive is what a
consumer executes after extraction, and its digest already commits to the binary and archive
metadata. The adjacent checksum remains a convenient digest transport, not a second attested
subject. Attesting a broad `dist/**` glob could silently add logs or transient files to the signed
claim, so the workflow and its executable audit require the narrow archive pattern.

**2. Attestation is part of the existing same-job transaction.** It runs after complete validation
and reproducible artifact construction, then its newly created bundle is verified before crates.io
authentication, package publication, or GitHub release publication. A failed, skipped, or ignored
attestation or verification therefore cannot publish anything. The same bundle and archive are
verified again after crates.io publication, immediately before GitHub asset publication, so an
intervening action or script cannot substitute different archive bytes.

**3. Verification binds the whole identity, not only a valid signature.** The verifier consumes
the bundle output from the immediately preceding action and requires:

- repository `asamarts/aplan`
- signer workflow `asamarts/aplan/.github/workflows/release.yml`
- signer and source digest equal to the exact release source
- source ref equal to the current `main` staging ref or `v*` release tag
- a GitHub-hosted runner
- the explicitly named SLSA provenance v1 predicate and GitHub Actions OIDC issuer

This prevents an older matching attestation, another workflow, another repository, another commit,
or a self-hosted signer from satisfying the publication gate.

**4. Manual staging attests too.** `workflow_dispatch` from `main` enters the protected environment,
runs the complete release profile, builds the `-dev` archive, attests it, and verifies the result.
It still cannot authenticate to crates.io or publish any asset because those steps require a tag.
The first tag is consequently not the first execution of the signing path.

**5. Use GitHub's current generic attestation action at one reviewed commit.** New implementations
use `actions/attest`, not the older `attest-build-provenance` wrapper. The action is pinned to a full
commit and included in the repository's closed external-action census. The workflow adds only
`attestations: write`; its existing `id-token: write` serves both Sigstore and crates.io OIDC.

**6. Do not add the generated bundle as a release asset.** A keyless certificate and transparency
log timestamp make a fresh bundle intentionally non-reproducible. Publishing it beside immutable
assets would make an otherwise safe rerun produce different bytes under the same tag. The
attestation API stores every valid bundle, online verification fetches it by artifact digest, and
GitHub documents how to download a bundle plus trusted roots for offline verification.

**7. The claim starts with the next release.** The v0.5.2 builder did not create an attestation.
Running a newer workflow later would prove that later workflow invocation, not retroactively prove
the historical build. Documentation names that boundary rather than implying all old assets gained
provenance.

## Consequences

- A consumer can authenticate an archive digest to the repository, exact release workflow, tag,
  source commit, hosted builder identity, and public transparency log without trusting a stored key.
- The repository does not claim a SLSA level. The predicate uses the SLSA provenance v1 format, but
  a level also depends on broader build isolation and policy not decided here.
- Compromise of the release workflow at the attested commit remains inside the trust boundary. The
  full-SHA action pin, protected branch, protected environment, independent review, full profile,
  reproducible builder, and post-generation verification remain separate defenses.
- GitHub and the Sigstore public-good service become online dependencies of a publishing run. A
  failure is closed: publication stops before crates or assets are created.

## References

- [GitHub artifact attestations](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations)
- [GitHub offline attestation verification](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/verify-attestations-offline)
- [actions/attest](https://github.com/actions/attest)
- [SLSA provenance v1](https://slsa.dev/provenance/v1)
