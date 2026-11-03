#!/usr/bin/env bash
set -euo pipefail

# Create the GitHub release after crates.io succeeds. A rerun may add a missing
# expected asset, but it never replaces bytes already published under the tag.

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

: "${TAG:?TAG is required}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
: "${GH_TOKEN:?GH_TOKEN is required}"

release_source="$(ci/scripts/release-source.sh "$REPO_ROOT")"
export APLAN_RELEASE_SHA="$release_source"
version="$(ci/scripts/current-version.sh)"
if [[ "$TAG" != "v${version}" || "$version" == *-* ]]; then
    echo "ERROR: GitHub publication requires TAG=v${version} and a release version" >&2
    exit 1
fi

shopt -s nullglob
assets=(dist/*.tar.gz dist/*.sha256)
if [[ "${#assets[@]}" -ne 2 ]]; then
    echo "ERROR: expected one release archive and one checksum, found ${#assets[@]}" >&2
    exit 1
fi

if ! gh release view "$TAG" --repo "$GITHUB_REPOSITORY" >/dev/null 2>&1; then
    gh release create "$TAG" \
        --repo "$GITHUB_REPOSITORY" \
        --title "$TAG" \
        --generate-notes \
        "${assets[@]}"
    exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mapfile -t published < <(
    gh release view "$TAG" --repo "$GITHUB_REPOSITORY" --json assets --jq '.assets[].name'
)

for name in "${published[@]}"; do
    expected=false
    for asset in "${assets[@]}"; do
        if [[ "$(basename "$asset")" == "$name" ]]; then
            expected=true
            break
        fi
    done
    if [[ "$expected" != true ]]; then
        echo "ERROR: release ${TAG} has unexpected asset ${name}" >&2
        exit 1
    fi
done

missing=()
for asset in "${assets[@]}"; do
    name="$(basename "$asset")"
    present=false
    for published_name in "${published[@]}"; do
        if [[ "$published_name" == "$name" ]]; then
            present=true
            break
        fi
    done
    if [[ "$present" == true ]]; then
        gh release download "$TAG" \
            --repo "$GITHUB_REPOSITORY" \
            --dir "$work" \
            --pattern "$name"
        if ! cmp --silent "$asset" "$work/$name"; then
            echo "ERROR: release ${TAG} asset ${name} differs from the validated artifact" >&2
            exit 1
        fi
        echo "==> Existing ${name} matches the validated artifact"
    else
        missing+=("$asset")
    fi
done

if [[ "${#missing[@]}" -gt 0 ]]; then
    gh release upload "$TAG" --repo "$GITHUB_REPOSITORY" "${missing[@]}"
else
    echo "==> GitHub release ${TAG} already has the exact expected assets"
fi
