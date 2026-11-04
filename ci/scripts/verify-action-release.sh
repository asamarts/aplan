#!/usr/bin/env bash
set -euo pipefail

# Verify the archive downloaded by the composite Action before it is extracted.
# v0.6.0 is the first release built by the keyless provenance transaction in
# ADR-0066. The finite older set remains usable under its historical checksum
# contract, but no later release may silently fall back to that weaker path.

if [[ "$#" -ne 4 ]]; then
    echo "usage: verify-action-release.sh TAG REPOSITORY ARCHIVE CHECKSUM" >&2
    exit 2
fi

tag="$1"
repository="$2"
archive="$3"
checksum="$4"

if [[ ! "$tag" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
    echo "ERROR: release tag must be an exact stable version, found '${tag}'" >&2
    exit 1
fi
if [[ ! "$repository" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
    echo "ERROR: release repository must be OWNER/REPO, found '${repository}'" >&2
    exit 1
fi
if [[ ! -f "$archive" || -L "$archive" ]]; then
    echo "ERROR: release archive is not one regular file: ${archive}" >&2
    exit 1
fi
if [[ ! -f "$checksum" || -L "$checksum" ]]; then
    echo "ERROR: release checksum is not one regular file: ${checksum}" >&2
    exit 1
fi

# Do not let a syntactically valid sidecar name some other file. The release
# builder writes exactly one lowercase SHA-256 and the archive basename.
read -r expected_digest checksum_subject checksum_extra < "$checksum" || {
    echo "ERROR: release checksum is empty: ${checksum}" >&2
    exit 1
}
if [[ ! "$expected_digest" =~ ^[0-9a-f]{64}$ \
    || "$checksum_subject" != "$(basename "$archive")" \
    || -n "${checksum_extra:-}" \
    || "$(wc -l < "$checksum")" -ne 1 ]]; then
    echo "ERROR: release checksum does not name exactly $(basename "$archive")" >&2
    exit 1
fi
actual_digest="$(sha256sum "$archive" | awk '{print $1}')"
if [[ "$actual_digest" != "$expected_digest" ]]; then
    echo "ERROR: release archive does not match ${checksum}" >&2
    exit 1
fi

case "$tag" in
    v0.1.0|v0.2.0|v0.3.0|v0.4.0|v0.5.0|v0.5.1|v0.5.2)
        echo "==> ${tag} predates keyless release provenance; checksum verified"
        exit 0
        ;;
esac

: "${GH_TOKEN:?GH_TOKEN is required to verify release provenance}"
gh attestation verify "$archive" \
    --repo "$repository" \
    --predicate-type "https://slsa.dev/provenance/v1" \
    --cert-oidc-issuer "https://token.actions.githubusercontent.com" \
    --signer-workflow "${repository}/.github/workflows/release.yml" \
    --source-ref "refs/tags/${tag}" \
    --deny-self-hosted-runners

echo "==> Verified ${tag} checksum and keyless release provenance"
