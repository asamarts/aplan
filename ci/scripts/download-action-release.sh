#!/usr/bin/env bash
set -euo pipefail

# Download a pinned release archive and its SHA-256 sidecar for the composite
# Action, retrying only transient 5xx or network errors a small fixed number of
# times with short backoff (`A2-027`, #35). Permanent errors such as HTTP 404
# fail immediately, and checksum/provenance verification runs separately after
# this script returns and is never retried.

if [[ "$#" -ne 4 ]]; then
    echo "usage: download-action-release.sh TAG REPOSITORY ASSET DEST_DIR" >&2
    exit 2
fi

tag="$1"
repository="$2"
asset="$3"
dest_dir="$4"

# Three retries (four total attempts) with short exponential backoff by default.
# Tests may override APLAN_DOWNLOAD_BACKOFF_SECONDS (e.g. "0 0 0" to exercise
# retries without sleeping, or "" for a single attempt with no retries).
backoff_spec="${APLAN_DOWNLOAD_BACKOFF_SECONDS-2 4 8}"
read -r -a delays <<< "$backoff_spec"
if (( ${#delays[@]} > 0 )); then
    for delay in "${delays[@]}"; do
        if [[ ! "$delay" =~ ^[0-9]+$ ]]; then
            echo "ERROR: invalid backoff delay '${delay}'" >&2
            exit 2
        fi
    done
fi
max_attempts=$(( ${#delays[@]} + 1 ))

is_transient_download_error() {
    local lowered
    lowered="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')"

    # Never retry 4xx or missing-release / auth answers.
    if [[ "$lowered" =~ http[[:space:]]+4[0-9][0-9] \
        || "$lowered" == *"release not found"* \
        || "$lowered" == *"no assets match"* \
        || "$lowered" == *"bad credentials"* \
        || "$lowered" == *"unauthorized"* \
        || "$lowered" == *"forbidden"* ]]; then
        return 1
    fi

    # Retry only 5xx HTTP responses or transport/network failures.
    if [[ "$lowered" =~ http[[:space:]]+5[0-9][0-9] \
        || "$lowered" == *"500 internal server error"* \
        || "$lowered" == *"502 bad gateway"* \
        || "$lowered" == *"503 service unavailable"* \
        || "$lowered" == *"504 gateway timeout"* \
        || "$lowered" == *"connection reset"* \
        || "$lowered" == *"connection refused"* \
        || "$lowered" == *"connection timed out"* \
        || "$lowered" == *"error connecting to"* \
        || "$lowered" == *"failed to connect"* \
        || "$lowered" == *"i/o timeout"* \
        || "$lowered" == *"tls handshake timeout"* \
        || "$lowered" == *"timeout awaiting response headers"* \
        || "$lowered" == *"unexpected eof"* \
        || "$lowered" == *": eof"* \
        || "$lowered" == *"dial tcp"* \
        || "$lowered" == *"no such host"* \
        || "$lowered" == *"could not resolve host"* \
        || "$lowered" == *"temporary failure in name resolution"* \
        || "$lowered" == *"network is unreachable"* \
        || "$lowered" == *"stream error"* \
        || "$lowered" == *"goaway"* \
        || "$lowered" == *"context deadline exceeded"* \
        || "$lowered" == *"client.timeout"* \
        || "$lowered" == *"broken pipe"* \
        || "$lowered" == *"server closed idle connection"* \
        || "$lowered" == *"remote error: tls"* ]]; then
        return 0
    fi

    return 1
}

attempt=1
while true; do
    if output="$(
        gh release download "$tag" \
            --repo "$repository" \
            --pattern "$asset" \
            --pattern "${asset}.sha256" \
            --dir "$dest_dir" \
            --clobber 2>&1
    )"; then
        if [[ -n "$output" ]]; then
            printf '%s\n' "$output"
        fi
        exit 0
    else
        status=$?
    fi

    if [[ -n "$output" ]]; then
        printf '%s\n' "$output" >&2
    fi

    if (( attempt < max_attempts )) && is_transient_download_error "$output"; then
        delay="${delays[$((attempt - 1))]}"
        echo "==> Transient error downloading aplan ${tag} (attempt ${attempt}/${max_attempts}); retrying in ${delay}s" >&2
        if (( delay > 0 )); then
            sleep "$delay"
        fi
        attempt=$((attempt + 1))
        continue
    fi

    echo "::error::Install failed: could not download aplan ${tag} from ${repository} after ${attempt} attempt(s); no plan check ran." >&2
    exit "$status"
done
