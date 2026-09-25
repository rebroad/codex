#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
REPO="${CODEX_RELEASE_REPO:-rebroad/codex}"
TAG="${1:-}"
OUTPUT_DIR="${2:-${ROOT}.build/build/npm-artifact}"

if [[ "${TAG}" == --help || "${TAG}" == -h || -z "${TAG}" ]]; then
  cat <<'EOF'
Usage: scripts/download_npm_release.sh <codex-npm-vTAG> [OUTPUT_DIR]

Downloads the immutable npm archives, checksums, and source manifest from a
GitHub release. Set CODEX_RELEASE_REPO to override the default repository.
EOF
  [[ -z "${TAG}" ]] && exit 2 || exit 0
fi

command -v gh >/dev/null 2>&1 || { echo "missing required command: gh" >&2; exit 1; }
mkdir -p "${OUTPUT_DIR}"
gh release download "${TAG}" \
  --repo "${REPO}" \
  --pattern 'codex-npm-*.tgz' \
  --pattern 'codex-npm-*.tgz.sha256' \
  --pattern 'npm-artifact-sources.json' \
  --dir "${OUTPUT_DIR}" \
  --clobber

echo "Downloaded npm release ${TAG} from ${REPO} to ${OUTPUT_DIR}"
