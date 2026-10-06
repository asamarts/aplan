#!/usr/bin/env bash
set -euo pipefail

# Build the release artifact: one statically linked Linux x86_64 binary, plus the
# checksum a consumer needs to pin it (A1-028).
#
# Deliberately not `dist` (HLD section 17). That pipeline's value is
# multi-platform binaries, and macOS and Windows cannot be built on the Linux
# self-hosted runner: they would need GitHub-hosted runners, reopening the Actions
# billing the pre-public repository-boundary decision eliminated. This is the slice that runs where every other job
# already runs.
#
# Usage:
#   ci/scripts/release.sh              # build for the workspace version
#   TAG=v0.1.0 ci/scripts/release.sh   # build, and refuse if the tag disagrees
#
# Output lands in `dist/`.

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

TARGET="x86_64-unknown-linux-musl"
OUT_DIR="${REPO_ROOT}/dist"

# The single source of truth for the version is `[workspace.package].version`
# (HLD section 17). Read it from there rather than from the tag, so the tag is
# checked against the code instead of the code being described by the tag.
#
# Through the same one-line script `.aplan.yml` declares as `version.current`,
# so the number this script refuses a tag against and the number `aplan version`
# does arithmetic on cannot come apart.
version="$(ci/scripts/current-version.sh)"

if [[ -z "$version" ]]; then
    echo "ERROR: could not read [workspace.package].version from Cargo.toml" >&2
    exit 1
fi

# A tag that disagrees with the code is the drift this repository exists to
# reject: publishing `v0.2.0` built from a `0.1.0` tree would make the release
# name a claim nothing backs.
if [[ -n "${TAG:-}" ]]; then
    # `main` carries a pre-release version so a staged build is never mistakable
    # for a published one (A1-060). Releasing is therefore an explicit bump, and
    # saying so beats a mismatch error the reader has to decode.
    if [[ "$version" == *-* ]]; then
        echo "ERROR: [workspace.package].version is '${version}', a pre-release." >&2
        echo "       main carries one on purpose, so a staged build cannot claim to be" >&2
        echo "       a release. Set the release version in Cargo.toml, commit, then tag." >&2
        exit 1
    fi
    if [[ "$TAG" != "v${version}" ]]; then
        echo "ERROR: tag '${TAG}' does not match [workspace.package].version '${version}'." >&2
        echo "       Expected tag 'v${version}'. Bump the version, or fix the tag." >&2
        exit 1
    fi
    echo "==> Tag ${TAG} matches the workspace version"

    # A matching tag and manifest still do not tell a reader which changes the
    # release carries. Refuse before the expensive build, and while the author
    # can still run this script before creating or pushing the tag (`A1-321`).
    ci/scripts/check-release-changelog.sh "$version"
fi

echo "==> Building aplan ${version} for ${TARGET}"
cargo build --release --locked -p aplan --target "${TARGET}"

# Not `target/`: the CI runner image sets `CARGO_TARGET_DIR` to a persistent
# volume outside the checkout (so builds stay warm across jobs), and hardcoding
# `target/` finds nothing there.
binary="${CARGO_TARGET_DIR:-target}/${TARGET}/release/aplan"
if [[ ! -x "$binary" ]]; then
    echo "ERROR: ${binary} was not produced" >&2
    exit 1
fi

# Prove the artifact is what it claims: a static binary is the whole reason a
# consumer's runner can execute it regardless of that image's glibc.
#
# Asserted positively, and accepting both phrasings, because `ldd` describes a
# static binary as "statically linked" on some toolchains and "not a dynamic
# executable" on others (and exits non-zero while doing it). A check that only
# knew one wording would fail a perfectly good release.
if command -v ldd >/dev/null 2>&1; then
    ldd_out="$(ldd "$binary" 2>&1 || true)"
    if ! grep -qE "statically linked|not a dynamic executable" <<<"$ldd_out"; then
        echo "ERROR: ${binary} is not statically linked; it would not be portable." >&2
        echo "$ldd_out" >&2
        exit 1
    fi
fi

echo "==> Verifying the built binary reports the version it was built for"
built_version="$("$binary" --version | awk '{print $2}')"
if [[ "$built_version" != "$version" ]]; then
    echo "ERROR: binary reports '${built_version}', expected '${version}'" >&2
    exit 1
fi

# And the plan's own arithmetic (A1-313, ADR-0034). The tag check above asks
# whether the tag agrees with the manifest, which is two files saying the same
# number; this asks whether that number is *large enough* for what has closed
# since the last release. A breaking change that shipped in a patch release is
# not discovered by reading the tag -- it is discovered by a consumer, and the
# tag is the last moment it is still preventable.
#
# Asked of the artifact that was just built, rather than of a second build from
# source: it is the binary being shipped, so the gate and the release cannot be
# two different programs, and it costs nothing here where the build has already
# happened.
#
# Only under TAG, for the same reason the check above is: an untagged build is a
# staged one, and `main` carries a pre-release version it is *supposed* to be
# short of.
if [[ -n "${TAG:-}" ]]; then
    echo "==> Asking the plan what this release must be"
    # stderr is kept rather than discarded: on the refusing path it is the
    # reason, and on the disagreeing path it names the window and the task that
    # demanded the larger number -- the whole difference between a refusal
    # somebody can act on and one they argue with.
    why="$(mktemp)"
    trap 'rm -f "$why"' EXIT
    if ! required="$("$binary" version 2>"$why")"; then
        echo "ERROR: the plan could not say what the next version must be:" >&2
        sed 's/^/       /' "$why" >&2
        echo "       It refuses rather than guessing, because a version number is a" >&2
        echo "       compatibility promise to somebody who is not in the room." >&2
        exit 1
    fi
    if [[ "$required" != "$version" ]]; then
        echo "ERROR: the plan requires version '${required}', and this tree declares" >&2
        echo "       '${version}':" >&2
        sed 's/^/       /' "$why" >&2
        echo "       Set [workspace.package].version to '${required}' and re-tag, or" >&2
        echo "       correct the version_impact of the work that demands it." >&2
        exit 1
    fi
    echo "==> The plan agrees: ${required}"
fi

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"
archive="aplan-v${version}-${TARGET}.tar.gz"

echo "==> Packaging ${archive}"
# Archived from its own directory so the entry is a bare `aplan`, with no leading
# path for tar to strip and warn about (`CARGO_TARGET_DIR` is absolute on the CI
# runner). The fixed mtime and owner keep the tarball byte-identical across runs
# of the same commit, so a rebuild can be compared against what was published.
tar --create --gzip \
    --file "${OUT_DIR}/${archive}" \
    --directory "$(dirname "$binary")" \
    --owner=0 --group=0 --numeric-owner \
    --mtime='UTC 2020-01-01' \
    "$(basename "$binary")"

(cd "$OUT_DIR" && sha256sum "$archive" > "${archive}.sha256")

echo "==> Built:"
ls -1 "$OUT_DIR" | sed 's/^/      /'
echo "==> sha256: $(cut -d' ' -f1 "${OUT_DIR}/${archive}.sha256")"
