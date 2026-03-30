#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

VERSION=""
VENDOR_ROOT=""
DIST_DIR="${REPO_DIR}/dist/npm-local"
DO_PUBLISH="false"
DRY_RUN="false"

usage() {
  cat <<'EOF'
Usage: publish_npm_local.sh --version <semver> --vendor-root <path> [--publish] [--dry-run] [--dist-dir <path>]

Build local npm tarballs for:
- @reb.ai/codex
- @reb.ai/codex-linux-x64
- @reb.ai/codex-linux-armv7

Then optionally publish with the same dist-tag logic as CI:
- stable versions: latest, linux-x64, linux-armv7
- prerelease versions: alpha, alpha-linux-x64, alpha-linux-armv7

Options:
  --version <semver>     Required release version (example: 0.118.0-alpha.3)
  --vendor-root <path>   Required vendor root containing target subdirectories
  --dist-dir <path>      Output directory for generated tarballs (default: dist/npm-local)
  --publish              Publish tarballs to npm
  --dry-run              Use `npm publish --dry-run` (can be combined with --publish)
  -h, --help             Show this help
EOF
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

for arg in "$@"; do
  case "${arg}" in
    --version)
      VERSION="${2:-}"
      shift
      ;;
    --vendor-root)
      VENDOR_ROOT="${2:-}"
      shift
      ;;
    --dist-dir)
      DIST_DIR="${2:-}"
      shift
      ;;
    --publish)
      DO_PUBLISH="true"
      ;;
    --dry-run)
      DRY_RUN="true"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: ${arg}" >&2
      usage
      exit 1
      ;;
  esac
  shift
done

if [[ -z "${VERSION}" || -z "${VENDOR_ROOT}" ]]; then
  usage
  exit 1
fi

VENDOR_ROOT="$(cd "${VENDOR_ROOT}" && pwd)"
DIST_DIR="$(mkdir -p "${DIST_DIR}" && cd "${DIST_DIR}" && pwd)"

require_cmd python3
require_cmd npm

check_vendor_file() {
  local rel="$1"
  if [[ ! -f "${VENDOR_ROOT}/${rel}" ]]; then
    echo "Missing vendor file: ${VENDOR_ROOT}/${rel}" >&2
    exit 1
  fi
}

check_vendor_file "x86_64-unknown-linux-musl/codex/codex"
check_vendor_file "x86_64-unknown-linux-musl/path/rg"
check_vendor_file "armv7-unknown-linux-gnueabihf/codex/codex"

cd "${REPO_DIR}"

echo "Building npm tarballs into ${DIST_DIR}..."
python3 codex-cli/scripts/build_npm_package.py \
  --package codex \
  --release-version "${VERSION}" \
  --vendor-src "${VENDOR_ROOT}" \
  --pack-output "${DIST_DIR}/codex-npm-${VERSION}.tgz"

for platform in linux-x64 linux-armv7; do
  python3 codex-cli/scripts/build_npm_package.py \
    --package "codex-${platform}" \
    --release-version "${VERSION}" \
    --vendor-src "${VENDOR_ROOT}" \
    --pack-output "${DIST_DIR}/codex-npm-${platform}-${VERSION}.tgz"
done

echo "Built tarballs:"
ls -lh "${DIST_DIR}"/*.tgz

if [[ "${DO_PUBLISH}" != "true" ]]; then
  echo "Skipping npm publish (use --publish to publish)."
  exit 0
fi

npm_tag=""
if [[ "${VERSION}" == *-* ]]; then
  npm_tag="alpha"
fi
prefix=""
if [[ -n "${npm_tag}" ]]; then
  prefix="${npm_tag}-"
fi

echo "Validating npm auth..."
npm whoami >/dev/null

for tarball in "${DIST_DIR}"/*-"${VERSION}".tgz; do
  filename="$(basename "${tarball}")"
  tag=""

  case "${filename}" in
    codex-npm-linux-*-"${VERSION}".tgz)
      platform="${filename#codex-npm-}"
      platform="${platform%-${VERSION}.tgz}"
      tag="${prefix}${platform}"
      ;;
    codex-npm-"${VERSION}".tgz)
      tag="${npm_tag}"
      ;;
    *)
      echo "Unexpected npm tarball: ${filename}" >&2
      exit 1
      ;;
  esac

  publish_cmd=(npm publish "${tarball}" --access public)
  if [[ -n "${tag}" ]]; then
    publish_cmd+=(--tag "${tag}")
  fi
  if [[ "${DRY_RUN}" == "true" ]]; then
    publish_cmd+=(--dry-run)
  fi

  echo "+ ${publish_cmd[*]}"
  "${publish_cmd[@]}"
done

echo "npm publish flow completed."
