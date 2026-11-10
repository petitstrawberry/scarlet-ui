#!/usr/bin/env bash
# A fresh nix invocation can recover when a resumed Cachix download returns 416.
# Keep the toolchain pinned and fail the job if all attempts fail.
set -euo pipefail

: "${SCARLET_TOOLCHAIN:?SCARLET_TOOLCHAIN must name the pinned toolchain}"
: "${GITHUB_PATH:?GITHUB_PATH must name the runner path file}"

for attempt in 1 2 3; do
    if toolchain_path="$(nix build "${SCARLET_TOOLCHAIN}^out" --no-link --print-out-paths)"; then
        printf '%s/bin\n' "$toolchain_path" >> "$GITHUB_PATH"
        exit 0
    else
        status=$?
    fi
    if (( attempt == 3 )); then
        echo "Scarlet toolchain acquisition failed after $attempt attempts." >&2
        exit "$status"
    fi
    echo "Scarlet toolchain acquisition failed; retrying ($attempt/3)." >&2
    sleep "$((attempt * 10))"
done
