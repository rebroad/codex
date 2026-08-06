#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
VERSION=""
DO_PUBLISH=false
DRY_RUN=false
DIST_DIR="${ROOT}/dist/npm-local"
usage() {
  cat <<'EOF'
Usage: scripts/publish_npm_local.sh [--version VERSION] [--publish] [--dry-run] [--dist-dir DIR]
Creates @reb.ai/codex archives with scripts/package-npm.sh. Publishing is opt-in.
EOF
}
while (($#)); do
  case "${1}" in
    --version) VERSION="${2:-}"; shift 2 ;;
    --publish) DO_PUBLISH=true; shift ;;
    --dry-run) DRY_RUN=true; shift ;;
    --dist-dir) DIST_DIR="${2:-}"; shift 2 ;;
    --dist-dir=*) DIST_DIR="${1#*=}"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: ${1}" >&2; exit 1 ;;
  esac
done
[[ -n "${VERSION}" ]] || VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${ROOT}/codex-rs/Cargo.toml" | head -n 1)"
[[ -n "${VERSION}" ]] || { echo "Unable to determine workspace version" >&2; exit 1; }
mkdir -p "${DIST_DIR}"
rm -f "${DIST_DIR}"/*.tgz
OUTPUT_DIR="${DIST_DIR}" "${ROOT}/scripts/package-npm.sh" "${VERSION}" release
if [[ "${DO_PUBLISH}" != true ]]; then
  echo "Skipping npm publish (use --publish)."
  exit 0
fi
publish_args=(
  --publish
  --publish-dir "${DIST_DIR}"
)
[[ "${DRY_RUN}" == true ]] && publish_args+=(--dry-run)
python3 "${ROOT}/codex-cli/scripts/build_npm_package.py" "${publish_args[@]}"
