#!/usr/bin/env bash
set -euo pipefail

# Container entrypoint: register the runner (once), persist its durable identity
# to the config volume, and run the agent. Restores persisted credentials on
# restart so a reboot never needs a new registration token (the pre-public repository-boundary decision).

: "${GITHUB_REPO_URL:?GITHUB_REPO_URL is required}"
: "${GITHUB_TOKEN:?GITHUB_TOKEN is required}"

RUNNER_NAME="${RUNNER_NAME:-aplan-runner}"
RUNNER_LABELS="${RUNNER_LABELS:-self-hosted,linux,aplan}"
CONFIG_DIR="${RUNNER_CONFIG_DIR:-/home/runner/_config}"

cd /home/runner/actions-runner

CREDENTIAL_FILES=(.runner .credentials .credentials_rsaparams)

# Restore persisted credentials from the config volume.
if [[ -d "$CONFIG_DIR" && -f "$CONFIG_DIR/.runner" ]]; then
    echo "==> Restoring runner credentials from ${CONFIG_DIR}"
    for f in "${CREDENTIAL_FILES[@]}"; do
        [[ -f "$CONFIG_DIR/$f" ]] && cp "$CONFIG_DIR/$f" .
    done
fi

# Register only if not already configured.
if [[ ! -f .runner ]]; then
    MAX_ATTEMPTS=5
    for attempt in $(seq 1 "$MAX_ATTEMPTS"); do
        echo "==> Registering runner '${RUNNER_NAME}' for ${GITHUB_REPO_URL} (attempt ${attempt}/${MAX_ATTEMPTS})"
        if ./config.sh \
            --url "${GITHUB_REPO_URL}" \
            --token "${GITHUB_TOKEN}" \
            --name "${RUNNER_NAME}" \
            --labels "${RUNNER_LABELS}" \
            --unattended \
            --disableupdate \
            --replace; then
            break
        fi
        if [[ "$attempt" -eq "$MAX_ATTEMPTS" ]]; then
            echo "==> Registration failed after ${MAX_ATTEMPTS} attempts. Is GITHUB_TOKEN valid and unexpired?"
            exit 1
        fi
        delay=$(( 2 ** attempt ))
        echo "==> Registration failed, retrying in ${delay}s..."
        sleep "$delay"
    done

    # Persist the durable identity to the config volume.
    if [[ -d "$CONFIG_DIR" ]]; then
        echo "==> Persisting runner credentials to ${CONFIG_DIR}"
        for f in "${CREDENTIAL_FILES[@]}"; do
            [[ -f "$f" ]] && cp "$f" "$CONFIG_DIR/"
        done
    fi
fi

# Deregister on shutdown (uses stored credentials; the token is not needed).
cleanup() {
    echo "==> Caught signal, deregistering runner..."
    ./config.sh remove --token "${GITHUB_TOKEN}" 2>/dev/null || true
}
trap cleanup SIGTERM SIGINT

echo "==> Starting runner agent"
./run.sh &
wait $!
