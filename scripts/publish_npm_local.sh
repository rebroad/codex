#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

VERSION=""
VENDOR_ROOT=""
DIST_DIR="${REPO_DIR}/dist/npm-local"
DO_PUBLISH="false"
DRY_RUN="false"

NPM_X64_TARGET="x86_64-unknown-linux-musl"
NPM_ARMV7_TARGET="armv7-unknown-linux-gnueabihf"

usage() {
  cat <<'EOF'
Usage: publish_npm_local.sh [--version <semver>] [--vendor-root <path>] [--publish] [--dry-run] [--dist-dir <path>]

Build local npm tarballs for:
- @reb.ai/codex
- @reb.ai/codex-linux-x64
- @reb.ai/codex-linux-armv7

Defaults (no guessing):
- --version defaults to codex-rs workspace version
- --vendor-root defaults to dist/npm-vendor/<version>
- If required vendor artifacts are missing, the script stops and prints the exact rebuild command.

Then optionally publish with the same dist-tag logic as CI:
- stable versions: latest, linux-x64, linux-armv7
- prerelease versions: alpha, alpha-linux-x64, alpha-linux-armv7

Options:
  --version <semver>     Release version (default: workspace version)
  --vendor-root <path>   Vendor root containing target subdirectories (default: dist/npm-vendor/<version>)
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

read_workspace_version() {
  REPO_DIR="${REPO_DIR}" python3 - <<'PY'
import os
import tomllib
from pathlib import Path

toml_path = Path(os.environ["REPO_DIR"]) / "codex-rs" / "Cargo.toml"
data = tomllib.loads(toml_path.read_text(encoding="utf-8"))
version = data.get("workspace", {}).get("package", {}).get("version")
if version:
    print(version)
PY
}

print_missing_vendor_help_and_exit() {
  local vendor_root="$1"
  shift
  echo "Missing required vendor artifacts under: ${vendor_root}" >&2
  for rel in "$@"; do
    echo "- ${vendor_root}/${rel}" >&2
  done
  echo >&2
  echo "Build/stage npm vendor artifacts with:" >&2
  echo "  ./scripts/rebuild_codex.sh --release --build-npm-vendor" >&2
  echo >&2
  echo "Then rerun:" >&2
  if [[ "${DO_PUBLISH}" == "true" ]]; then
    if [[ "${DRY_RUN}" == "true" ]]; then
      echo "  ./scripts/publish_npm_local.sh --publish --dry-run" >&2
    else
      echo "  ./scripts/publish_npm_local.sh --publish" >&2
    fi
  else
    echo "  ./scripts/publish_npm_local.sh" >&2
  fi
  exit 1
}

while (($# > 0)); do
  case "$1" in
    --version)
      VERSION="${2:-}"
      shift 2
      ;;
    --vendor-root)
      VENDOR_ROOT="${2:-}"
      shift 2
      ;;
    --dist-dir)
      DIST_DIR="${2:-}"
      shift 2
      ;;
    --publish)
      DO_PUBLISH="true"
      shift
      ;;
    --dry-run)
      DRY_RUN="true"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

require_cmd python3
require_cmd npm

if [[ -z "${VERSION}" ]]; then
  VERSION="$(read_workspace_version)"
fi
if [[ -z "${VERSION}" ]]; then
  echo "Unable to resolve workspace version. Provide --version <semver>." >&2
  exit 1
fi

if [[ -z "${VENDOR_ROOT}" ]]; then
  VENDOR_ROOT="${REPO_DIR}/dist/npm-vendor/${VERSION}"
elif [[ "${VENDOR_ROOT}" != /* ]]; then
  VENDOR_ROOT="${REPO_DIR}/${VENDOR_ROOT}"
fi

if [[ "${DIST_DIR}" != /* ]]; then
  DIST_DIR="${REPO_DIR}/${DIST_DIR}"
fi
DIST_DIR="$(mkdir -p "${DIST_DIR}" && cd "${DIST_DIR}" && pwd)"

required_vendor_paths=(
  "${NPM_X64_TARGET}/codex/codex"
  "${NPM_X64_TARGET}/path/rg"
  "${NPM_ARMV7_TARGET}/codex/codex"
)
missing_paths=()
for rel in "${required_vendor_paths[@]}"; do
  if [[ ! -f "${VENDOR_ROOT}/${rel}" ]]; then
    missing_paths+=("${rel}")
  fi
done
if ((${#missing_paths[@]} > 0)); then
  print_missing_vendor_help_and_exit "${VENDOR_ROOT}" "${missing_paths[@]}"
fi

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
