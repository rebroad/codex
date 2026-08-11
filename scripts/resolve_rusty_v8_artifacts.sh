#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd -- "${SCRIPT_DIR}/../codex-rs" && pwd)"
TARGET=""
OUTPUT_DIR=""
RELEASE_REPO="auto"
RELEASE_TAG=""
PROFILE=""
V8_VERSION=""

usage() {
  cat <<'EOF'
Usage: resolve_rusty_v8_artifacts.sh --target=TRIPLE --output-dir=DIR [options]

Downloads and verifies the Rusty V8 archive and generated binding for a target.
Options:
  --target=TRIPLE       Rust target triple (required)
  --output-dir=DIR      Cache/output directory (required)
  --release-repo=REPO   GitHub repository or auto (default: auto)
  --release-tag=TAG     Rusty V8 release tag (default: rusty-v8-v<VERSION>)
  --profile=PROFILE     Rusty V8 profile (default: release for ARMv7,
                        ptrcomp_sandbox_release otherwise)
  --v8-version=VERSION  Pinned V8 crate version (auto-detected if omitted)
EOF
}

for arg in "$@"; do
  case "${arg}" in
    --target=*) TARGET="${arg#*=}" ;;
    --output-dir=*) OUTPUT_DIR="${arg#*=}" ;;
    --release-repo=*) RELEASE_REPO="${arg#*=}" ;;
    --release-tag=*) RELEASE_TAG="${arg#*=}" ;;
    --profile=*) PROFILE="${arg#*=}" ;;
    --v8-version=*) V8_VERSION="${arg#*=}" ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: ${arg}" >&2; usage >&2; exit 1 ;;
  esac
done

[[ -n "${TARGET}" ]] || { echo "--target is required" >&2; exit 1; }
[[ -n "${OUTPUT_DIR}" ]] || { echo "--output-dir is required" >&2; exit 1; }

if [[ -z "${V8_VERSION}" ]]; then
  V8_VERSION="$(awk '
    BEGIN { in_pkg=0; name=""; version="" }
    /^\[\[package\]\]/ {
      if (in_pkg && name == "v8") { print version; exit }
      in_pkg=1; name=""; version=""; next
    }
    in_pkg && /^name = / { gsub(/"/, "", $3); name=$3; next }
    in_pkg && /^version = / { gsub(/"/, "", $3); version=$3; next }
    END { if (in_pkg && name == "v8") print version }
  ' "${WORKSPACE_DIR}/Cargo.lock")"
fi
[[ -n "${V8_VERSION}" ]] || { echo "Unable to determine the pinned v8 version" >&2; exit 1; }

if [[ "${RELEASE_REPO}" == auto ]]; then
  case "${TARGET}" in
    x86_64-unknown-linux-musl|armv7-unknown-linux-gnueabihf|aarch64-linux-android)
      RELEASE_REPO="rebroad/rusty_v8" ;;
    *) RELEASE_REPO="openai/codex" ;;
  esac
fi
RELEASE_TAG="${RELEASE_TAG:-rusty-v8-v${V8_VERSION}}"
if [[ -z "${PROFILE}" ]]; then
  case "${TARGET}" in
    armv7-unknown-linux-gnueabihf) PROFILE=release ;;
    *) PROFILE=ptrcomp_sandbox_release ;;
  esac
fi

if [[ "${TARGET}" == *-pc-windows-msvc ]]; then
  ARCHIVE_NAME="rusty_v8_${PROFILE}_${TARGET}.lib.gz"
else
  ARCHIVE_NAME="librusty_v8_${PROFILE}_${TARGET}.a.gz"
fi
BINDING_NAME="src_binding_${PROFILE}_${TARGET}.rs"
CHECKSUMS_NAME="rusty_v8_${PROFILE}_${TARGET}.sha256"
BASE_URL="https://github.com/${RELEASE_REPO}/releases/download/${RELEASE_TAG}"
ARCHIVE_PATH="${OUTPUT_DIR}/${ARCHIVE_NAME}"
BINDING_PATH="${OUTPUT_DIR}/${BINDING_NAME}"
CHECKSUMS_PATH="${OUTPUT_DIR}/${CHECKSUMS_NAME}"

mkdir -p "${OUTPUT_DIR}"
[[ -s "${ARCHIVE_PATH}" ]] || curl -fsSL "${BASE_URL}/${ARCHIVE_NAME}" -o "${ARCHIVE_PATH}"
[[ -s "${BINDING_PATH}" ]] || curl -fsSL "${BASE_URL}/${BINDING_NAME}" -o "${BINDING_PATH}"
[[ -s "${CHECKSUMS_PATH}" ]] || curl -fsSL "${BASE_URL}/${CHECKSUMS_NAME}" -o "${CHECKSUMS_PATH}"

if command -v sha256sum >/dev/null 2>&1; then
  (cd "${OUTPUT_DIR}" && tr -d '\r' < "${CHECKSUMS_PATH}" | sha256sum -c - >&2)
else
  (cd "${OUTPUT_DIR}" && tr -d '\r' < "${CHECKSUMS_PATH}" | shasum -a 256 -c - >&2)
fi

printf 'RUSTY_V8_ARCHIVE=%q\n' "${ARCHIVE_PATH}"
printf 'RUSTY_V8_SRC_BINDING_PATH=%q\n' "${BINDING_PATH}"
printf 'RUSTY_V8_RELEASE_REPO=%q\n' "${RELEASE_REPO}"
printf 'RUSTY_V8_RELEASE_TAG=%q\n' "${RELEASE_TAG}"
