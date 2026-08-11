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
  V8_VERSION="$(python3 - "${WORKSPACE_DIR}/Cargo.lock" <<'PY'
import sys
import tomllib

with open(sys.argv[1], "rb") as lockfile:
    versions = sorted(
        {
            package["version"]
            for package in tomllib.load(lockfile)["package"]
            if package["name"] == "v8"
        }
    )

if len(versions) != 1:
    raise SystemExit(f"Expected exactly one resolved v8 version, found: {versions}")
print(versions[0])
PY
  )"
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
[[ -s "${CHECKSUMS_PATH}" ]] || curl -fsSL "${BASE_URL}/${CHECKSUMS_NAME}" -o "${CHECKSUMS_PATH}"

checksum_lines="$(tr -d '\r' < "${CHECKSUMS_PATH}")"
checksum_count=0
archive_checksum_seen=false
binding_checksum_seen=false
archive_digest=""
binding_digest=""
while read -r digest name extra; do
  [[ -n "${digest:-}" ]] || continue
  [[ -z "${extra:-}" ]] || { echo "Invalid checksum line in ${CHECKSUMS_PATH}" >&2; exit 1; }
  name="${name#\*}"
  [[ "${digest}" =~ ^[0-9a-fA-F]{64}$ ]] || {
    echo "Invalid checksum digest in ${CHECKSUMS_PATH}: ${digest}" >&2
    exit 1
  }
  case "${name}" in
    "${ARCHIVE_NAME}") archive_checksum_seen=true; archive_digest="${digest}" ;;
    "${BINDING_NAME}") binding_checksum_seen=true; binding_digest="${digest}" ;;
    *) echo "Unexpected checksum artifact in ${CHECKSUMS_PATH}: ${name}" >&2; exit 1 ;;
  esac
  checksum_count=$((checksum_count + 1))
done <<< "${checksum_lines}"
[[ "${checksum_count}" -eq 2 && "${archive_checksum_seen}" == true && "${binding_checksum_seen}" == true ]] || {
  echo "Expected exactly ${ARCHIVE_NAME} and ${BINDING_NAME} in ${CHECKSUMS_PATH}" >&2
  exit 1
}

verify_checksum() {
  local digest="$1"
  local path="$2"
  if command -v sha256sum >/dev/null 2>&1; then
    printf '%s  %s\n' "${digest}" "${path}" | sha256sum -c - >/dev/null
  else
    printf '%s  %s\n' "${digest}" "${path}" | shasum -a 256 -c - >/dev/null
  fi
}

ensure_artifact() {
  local digest="$1"
  local name="$2"
  local path="$3"
  if [[ -s "${path}" ]] && verify_checksum "${digest}" "${path}"; then
    return 0
  fi
  rm -f "${path}"
  curl -fsSL "${BASE_URL}/${name}" -o "${path}"
  verify_checksum "${digest}" "${path}" || {
    echo "Checksum verification failed for ${name}" >&2
    return 1
  }
}

ensure_artifact "${archive_digest}" "${ARCHIVE_NAME}" "${ARCHIVE_PATH}"
ensure_artifact "${binding_digest}" "${BINDING_NAME}" "${BINDING_PATH}"

printf 'RUSTY_V8_ARCHIVE=%q\n' "${ARCHIVE_PATH}"
printf 'RUSTY_V8_SRC_BINDING_PATH=%q\n' "${BINDING_PATH}"
printf 'RUSTY_V8_RELEASE_REPO=%q\n' "${RELEASE_REPO}"
printf 'RUSTY_V8_RELEASE_TAG=%q\n' "${RELEASE_TAG}"
