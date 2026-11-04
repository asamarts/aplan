#!/usr/bin/env bash
set -euo pipefail

# Build the release artifact: one statically linked Linux x86_64 binary, plus the
# checksum a consumer needs to pin it (A1-028).
#
# Deliberately not `dist` (HLD section 17). Changing from private self-hosted CI
# to public hosted CI does not itself decide a multi-platform distribution
# promise. This remains the one portable asset the release design specifies.
#
# Usage:
#   ci/scripts/release.sh              # build for the workspace version
#   TAG=v0.1.0 ci/scripts/release.sh   # build, and refuse if the tag disagrees
#
# Output lands in `dist/`.

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

run_in_pinned_builder() {
    local engine image contract scratch_root build_root release_source status git_common
    local -a git_mount=() user_args=()

    container_engine_usable() {
        command -v "$1" >/dev/null 2>&1 \
            && "$1" info >/dev/null 2>&1 \
            && "$1" build --help 2>&1 | grep -q -- '--platform'
    }

    release_source="$(ci/scripts/release-source.sh "$REPO_ROOT")"
    git_common="$(git rev-parse --path-format=absolute --git-common-dir)"
    if [[ "$git_common" != "$REPO_ROOT/.git" ]]; then
        git_mount=(--volume "$git_common:$git_common:ro")
    fi
    contract="$(sha256sum \
        ci/release.Containerfile rust-toolchain.toml ci/scripts/toolchain.sh \
        | sha256sum | awk '{print $1}')"
    image="aplan-release-builder:${contract}"

    if [[ -n "${APLAN_RELEASE_CONTAINER_ENGINE:-}" ]]; then
        case "$APLAN_RELEASE_CONTAINER_ENGINE" in
            docker|podman) engine="$APLAN_RELEASE_CONTAINER_ENGINE" ;;
            *)
                echo "ERROR: APLAN_RELEASE_CONTAINER_ENGINE must be docker or podman" >&2
                return 1
                ;;
        esac
        container_engine_usable "$engine" || {
            echo "ERROR: configured release container engine '$engine' cannot build images" >&2
            return 1
        }
    elif container_engine_usable docker; then
        engine=docker
    elif container_engine_usable podman; then
        engine=podman
    else
        echo "ERROR: release artifact construction requires Docker or Podman" >&2
        return 1
    fi
    if [[ "$engine" == podman ]]; then
        user_args=(--userns=keep-id)
    else
        user_args=(--user "$(id -u):$(id -g)")
    fi

    if [[ -e "$REPO_ROOT/dist" && ( ! -d "$REPO_ROOT/dist" || -L "$REPO_ROOT/dist" ) ]]; then
        echo "ERROR: release output path '$REPO_ROOT/dist' is not a real directory" >&2
        return 1
    fi
    mkdir -p "$REPO_ROOT/dist"
    find "$REPO_ROOT/dist" -mindepth 1 -maxdepth 1 -delete

    scratch_root="${APLAN_RELEASE_SCRATCH_ROOT:-${HOME:?}/.tmp}"
    mkdir -p "$scratch_root"
    build_root="$(mktemp -d "${scratch_root%/}/aplan-release-builder.XXXXXX")"
    mkdir -p "$build_root/cargo" "$build_root/home" "$build_root/target"
    cleanup_release_builder() {
        rm -rf "$build_root"
    }
    trap cleanup_release_builder RETURN

    echo "==> Building pinned release environment ${image}"
    "$engine" build \
        --platform linux/amd64 \
        --file ci/release.Containerfile \
        --tag "$image" \
        "$REPO_ROOT"

    echo "==> Building the artifact inside ${image}"
    set +e
    "$engine" run --rm \
        --platform linux/amd64 \
        "${user_args[@]}" \
        --volume "$REPO_ROOT:/aplan/source:ro" \
        "${git_mount[@]}" \
        --volume "$REPO_ROOT/dist:/aplan/dist:rw" \
        --volume "$build_root:/aplan/build:rw" \
        --env APLAN_RELEASE_IN_BUILDER=1 \
        --env APLAN_RELEASE_DIST_DIR=/aplan/dist \
        --env "APLAN_RELEASE_SHA=$release_source" \
        --env "GITHUB_SHA=${GITHUB_SHA:-}" \
        --env "TAG=${TAG:-}" \
        --env CARGO_HOME=/aplan/build/cargo \
        --env HOME=/aplan/build/home \
        --env CARGO_TARGET_DIR=/aplan/build/target \
        --env GIT_CONFIG_COUNT=1 \
        --env GIT_CONFIG_KEY_0=safe.directory \
        --env GIT_CONFIG_VALUE_0=/aplan/source \
        "$image" bash ci/scripts/release.sh
    status=$?
    set -e
    return "$status"
}

if [[ "${APLAN_RELEASE_IN_BUILDER:-}" != 1 ]]; then
    run_in_pinned_builder
    exit $?
fi

export CARGO_BUILD_WARNINGS=deny

# Release binaries retain line-table source paths, including paths belonging to
# dependencies in Cargo and Rustup homes. Bind the artifact to stable public
# prefixes before the first build so a release is byte-comparable across
# builders and cannot disclose a maintainer or runner home directory (A2-005).
# Refuse inherited flags instead of silently mixing an unreviewed artifact
# fingerprint into the release contract.
if [[ -n "${RUSTFLAGS:-}" || -n "${CARGO_ENCODED_RUSTFLAGS:-}" ]]; then
    echo "ERROR: release builds require an empty RUSTFLAGS and CARGO_ENCODED_RUSTFLAGS" >&2
    exit 1
fi
rustflags_separator=$'\x1f'
export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=${REPO_ROOT}=/aplan/source${rustflags_separator}--remap-path-prefix=${HOME:?}=/aplan/home"

release_started="$(date +%s)"
why=""
release_phase_on_exit() {
    local status=$?
    local outcome=failed
    local finished
    trap - EXIT
    [[ -z "$why" ]] || rm -f "$why"
    if [[ "$status" -eq 0 ]]; then
        outcome=passed
    fi
    finished="$(date +%s)"
    printf 'release: phase=artifact outcome=%s duration_seconds=%s\n' \
        "$outcome" "$((finished - release_started))"
    exit "$status"
}
trap release_phase_on_exit EXIT

# Bind direct invocations and the workflow to one full commit ID. The workflow
# supplies github.sha; a local staged build binds its current committed HEAD.
release_source="$(ci/scripts/release-source.sh "$REPO_ROOT")"
export APLAN_RELEASE_SHA="$release_source"
echo "==> Release source ${release_source} is committed, clean, and exact"

# Publication must never inherit a moving hosted default. A1-614 adds the full
# profile before packaging; this local check independently keeps direct and
# workflow release invocations on the repository's exact compiler.
ci/scripts/toolchain.sh verify

TARGET="x86_64-unknown-linux-musl"
OUT_DIR="${APLAN_RELEASE_DIST_DIR:-${REPO_ROOT}/dist}"

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

echo "==> Proving the supported headless source build"
cargo build --release --locked -p aplan --no-default-features --target "${TARGET}"

# Not `target/`: the CI runner image sets `CARGO_TARGET_DIR` to a persistent
# volume outside the checkout (so builds stay warm across jobs), and hardcoding
# `target/` finds nothing there.
binary="${CARGO_TARGET_DIR:-target}/${TARGET}/release/aplan"
if [[ ! -x "$binary" ]]; then
    echo "ERROR: ${binary} was not produced" >&2
    exit 1
fi

# A successful feature-free compile is not dependency isolation: an optional
# edge accidentally made unconditional compiles just as successfully. Inspect
# the selected normal/build graph and refuse any terminal-stack package.
headless_graph="$(cargo tree --locked -p aplan --no-default-features \
    --edges normal,build --prefix none)"
if grep -qE '^(aplan-tui|ratatui|ratatui-core|ratatui-crossterm|ratatui-widgets|crossterm) ' \
    <<<"$headless_graph"; then
    echo "ERROR: --no-default-features still resolves the terminal stack:" >&2
    grep -E '^(aplan-tui|ratatui|ratatui-core|ratatui-crossterm|ratatui-widgets|crossterm) ' \
        <<<"$headless_graph" >&2
    exit 1
fi
set +e
"$binary" facts --require build.tui >/dev/null 2>&1
headless_require_status=$?
set -e
if [[ "$headless_require_status" -ne 1 ]]; then
    echo "ERROR: the headless build's build.tui requirement exited ${headless_require_status}, expected 1" >&2
    exit 1
fi
set +e
headless_error="$("$binary" tui 2>&1)"
headless_tui_status=$?
set -e
if [[ "$headless_tui_status" -ne 2 ]]; then
    echo "ERROR: the headless aplan tui stub exited ${headless_tui_status}, expected 2" >&2
    exit 1
fi
if [[ "$headless_error" != *"does not include the terminal interface"* ]]; then
    echo "ERROR: the headless aplan tui stub did not give its enablement advice" >&2
    echo "$headless_error" >&2
    exit 1
fi

echo "==> Building aplan ${version} for ${TARGET} with the release TUI capability"
cargo build --release --locked -p aplan --features tui --target "${TARGET}"
if ! "$binary" facts --require build.tui >/dev/null; then
    echo "ERROR: the release artifact does not advertise build.tui" >&2
    exit 1
fi

for private_build_prefix in "${REPO_ROOT}/" "${HOME}/"; do
    if LC_ALL=C grep -aFq "$private_build_prefix" "$binary"; then
        echo "ERROR: the release artifact retained private build prefix ${private_build_prefix}" >&2
        exit 1
    fi
done

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
    rm -f "$why"
    why=""
fi

# Compilation and version probes must not be able to move or dirty the source
# whose full validation authorized this artifact. Ignored build output is fine;
# tracked or untracked source is not.
source_after="$(ci/scripts/release-source.sh "$REPO_ROOT")" || {
    echo "ERROR: release source changed during artifact construction" >&2
    exit 1
}
if [[ "$source_after" != "$release_source" ]]; then
    echo "ERROR: release source identity changed during artifact construction" >&2
    exit 1
fi

if [[ -e "$OUT_DIR" && ( ! -d "$OUT_DIR" || -L "$OUT_DIR" ) ]]; then
    echo "ERROR: release output path '$OUT_DIR' is not a real directory" >&2
    exit 1
fi
mkdir -p "$OUT_DIR"
find "$OUT_DIR" -mindepth 1 -maxdepth 1 -delete
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
find "$OUT_DIR" -mindepth 1 -maxdepth 1 -printf '%f\n' | LC_ALL=C sort | sed 's/^/      /'
echo "==> sha256: $(cut -d' ' -f1 "${OUT_DIR}/${archive}.sha256")"
