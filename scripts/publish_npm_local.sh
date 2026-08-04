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
npm whoami >/dev/null
for archive in "${DIST_DIR}"/*.tgz; do
  [[ -f "${archive}" ]] || continue
  package_name="$(tar -xOf "${archive}" package/package.json | python3 -c 'import json,sys; print(json.load(sys.stdin)["name"])')"
  case "${package_name}" in
    @reb.ai/codex)
      if [[ "${VERSION}" == *-* ]]; then tag=alpha; else tag=latest; fi
      ;;
    @reb.ai/codex-linux-x64) tag="linux-x64" ;;
    @reb.ai/codex-linux-armv7) tag="linux-armv7" ;;
    @reb.ai/codex-android-arm64) tag="android-arm64" ;;
    *) echo "Unexpected package ${package_name}" >&2; exit 1 ;;
  esac
  [[ "${VERSION}" == *-* ]] && [[ "${package_name}" != @reb.ai/codex ]] && tag="alpha-${tag}"
  cmd=(npm publish "${archive}" --access public --tag "${tag}")
  [[ "${DRY_RUN}" == true ]] && cmd+=(--dry-run)
  echo "+ ${cmd[*]}"
  "${cmd[@]}"
done
