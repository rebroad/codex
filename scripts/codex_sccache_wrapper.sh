#!/usr/bin/env bash
set -euo pipefail

# Cargo holds file descriptor 9 for the shared target lock. Close it before
# starting sccache so a daemonized sccache server cannot retain that lock.
exec 9>&- 2>/dev/null || true
exec "${CODEX_SCCACHE_BIN:?CODEX_SCCACHE_BIN must be set}" "$@"
