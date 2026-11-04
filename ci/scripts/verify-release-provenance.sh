#!/usr/bin/env bash
set -euo pipefail

# Verify the keyless build-provenance bundle created for this run before any
# registry package or GitHub release asset can be published.

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

: "${APLAN_ATTESTATION_BUNDLE:?APLAN_ATTESTATION_BUNDLE is required}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
: "${GITHUB_REF:?GITHUB_REF is required}"
: "${GH_TOKEN:?GH_TOKEN is required}"

expected_repository="asamarts/aplan"
if [[ "$GITHUB_REPOSITORY" != "$expected_repository" ]]; then
    echo "ERROR: expected GITHUB_REPOSITORY=${expected_repository}, found ${GITHUB_REPOSITORY}" >&2
    exit 1
fi
case "$GITHUB_REF" in
    refs/heads/main|refs/tags/v*) ;;
    *)
        echo "ERROR: release provenance requires main or a v* tag, found ${GITHUB_REF}" >&2
        exit 1
        ;;
esac
if [[ ! -f "$APLAN_ATTESTATION_BUNDLE" || -L "$APLAN_ATTESTATION_BUNDLE" ]]; then
    echo "ERROR: attestation bundle is not one regular file: ${APLAN_ATTESTATION_BUNDLE}" >&2
    exit 1
fi

release_source="$(ci/scripts/release-source.sh "$REPO_ROOT")"
shopt -s nullglob
archives=(dist/*.tar.gz)
checksums=(dist/*.sha256)
if [[ "${#archives[@]}" -ne 1 ]]; then
    echo "ERROR: expected one release archive, found ${#archives[@]}" >&2
    exit 1
fi
if [[ "${#checksums[@]}" -ne 1 ]]; then
    echo "ERROR: expected one release checksum, found ${#checksums[@]}" >&2
    exit 1
fi
(cd dist && sha256sum --check --status "$(basename "${checksums[0]}")") || {
    echo "ERROR: release archive does not match ${checksums[0]}" >&2
    exit 1
}

signer_workflow="${expected_repository}/.github/workflows/release.yml"
gh attestation verify "${archives[0]}" \
    --repo "$expected_repository" \
    --bundle "$APLAN_ATTESTATION_BUNDLE" \
    --predicate-type "https://slsa.dev/provenance/v1" \
    --cert-oidc-issuer "https://token.actions.githubusercontent.com" \
    --signer-workflow "$signer_workflow" \
    --signer-digest "$release_source" \
    --source-digest "$release_source" \
    --source-ref "$GITHUB_REF" \
    --deny-self-hosted-runners

source_after="$(ci/scripts/release-source.sh "$REPO_ROOT")"
if [[ "$source_after" != "$release_source" ]]; then
    echo "ERROR: release source changed during provenance verification" >&2
    exit 1
fi
echo "==> Verified keyless release provenance for ${archives[0]}"
