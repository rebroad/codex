#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
base_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${ROOT}/codex-rs/Cargo.toml" | head -n 1)"
commit="${CODEX_PACKAGE_COMMIT:-$(git -C "${ROOT}" rev-parse --short=10 HEAD)}"
timestamp="${CODEX_PACKAGE_TIMESTAMP:-$(date -u +%Y%m%d%H%M)}"

[[ -n "${base_version}" ]] || { echo "could not determine workspace version" >&2; exit 1; }
[[ "${commit}" =~ ^[0-9a-fA-F]{10}$ ]] || { echo "invalid commit hash: ${commit}" >&2; exit 1; }
[[ "${timestamp}" =~ ^[0-9]{12}$ ]] || { echo "invalid UTC timestamp: ${timestamp}" >&2; exit 1; }

printf '%s.%s.%s\n' "${base_version}" "${commit}" "${timestamp}"
