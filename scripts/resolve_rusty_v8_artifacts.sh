#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd -- "${SCRIPT_DIR}/../codex-rs" && pwd)"
TARGET=""
OUTPUT_DIR=""
CARGO_BUILD_DIR=""
LOCAL_REPO=""
RELEASE_REPO="auto"
RELEASE_TAG=""
PROFILE=""
V8_VERSION=""
PREFER_LATEST="true"

usage() {
  cat <<'EOF'
Usage: resolve_rusty_v8_artifacts.sh --target=TRIPLE --output-dir=DIR [options]

Downloads and verifies the Rusty V8 archive and generated binding for a target.
Options:
  --target=TRIPLE       Rust target triple (required)
  --output-dir=DIR      Cache/output directory (required)
  --cargo-build-dir=DIR Existing Cargo build directory to inspect
  --local-repo=DIR      Local Rusty V8 artifact directory to inspect
  --release-repo=REPO   GitHub repository or auto (default: auto)
  --release-tag=TAG     Rusty V8 release tag (default: rusty-v8-v<VERSION>)
  --profile=PROFILE     Rusty V8 profile (default: release for ARMv7,
                        ptrcomp_sandbox_release otherwise)
  --v8-version=VERSION  Pinned V8 crate version (auto-detected if omitted)
  --prefer-latest       Use the latest suitable release when available (default)
  --no-prefer-latest    Use the version-pinned release tag
EOF
}

for arg in "$@"; do
  case "${arg}" in
    --target=*) TARGET="${arg#*=}" ;;
    --output-dir=*) OUTPUT_DIR="${arg#*=}" ;;
    --cargo-build-dir=*) CARGO_BUILD_DIR="${arg#*=}" ;;
    --local-repo=*) LOCAL_REPO="${arg#*=}" ;;
    --release-repo=*) RELEASE_REPO="${arg#*=}" ;;
    --release-tag=*) RELEASE_TAG="${arg#*=}" ;;
    --profile=*) PROFILE="${arg#*=}" ;;
    --v8-version=*) V8_VERSION="${arg#*=}" ;;
    --prefer-latest) PREFER_LATEST="true" ;;
    --no-prefer-latest) PREFER_LATEST="false" ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: ${arg}" >&2; usage >&2; exit 1 ;;
  esac
done

[[ -n "${TARGET}" ]] || { echo "--target is required" >&2; exit 1; }
[[ -n "${OUTPUT_DIR}" ]] || { echo "--output-dir is required" >&2; exit 1; }

if [[ -z "${V8_VERSION}" ]]; then
  V8_VERSION="$(python3 "${SCRIPT_DIR}/rusty_v8_version.py" "${WORKSPACE_DIR}/Cargo.lock")"
fi
[[ -n "${V8_VERSION}" ]] || { echo "Unable to determine the pinned v8 version" >&2; exit 1; }

if [[ "${RELEASE_REPO}" == auto ]]; then
  case "${TARGET}" in
    armv7-unknown-linux-gnueabihf|armv7-unknown-linux-musleabihf|aarch64-linux-android)
      RELEASE_REPO="rebroad/rusty_v8" ;;
    *) RELEASE_REPO="openai/codex" ;;
  esac
fi
V8_TARGET="${TARGET}"
if [[ -z "${PROFILE}" ]]; then
  case "${TARGET}" in
    armv7-unknown-linux-gnueabihf|armv7-unknown-linux-musleabihf) PROFILE=release ;;
    *) PROFILE=ptrcomp_sandbox_release ;;
  esac
fi

if [[ -n "${RUSTY_V8_ARCHIVE:-}" || -n "${RUSTY_V8_SRC_BINDING_PATH:-}" ]]; then
  [[ -s "${RUSTY_V8_ARCHIVE:-}" && -s "${RUSTY_V8_SRC_BINDING_PATH:-}" ]] || {
    echo "RUSTY_V8_ARCHIVE and RUSTY_V8_SRC_BINDING_PATH must point to existing files" >&2
    exit 1
  }
  printf 'RUSTY_V8_ARCHIVE=%q\n' "${RUSTY_V8_ARCHIVE}"
  printf 'RUSTY_V8_SRC_BINDING_PATH=%q\n' "${RUSTY_V8_SRC_BINDING_PATH}"
  printf 'RUSTY_V8_RELEASE_REPO=%q\n' "${RELEASE_REPO}"
  exit 0
fi

if [[ "${V8_TARGET}" == *-pc-windows-msvc ]]; then
  ARCHIVE_NAME="rusty_v8_${PROFILE}_${V8_TARGET}.lib.gz"
else
  ARCHIVE_NAME="librusty_v8_${PROFILE}_${V8_TARGET}.a.gz"
fi
BINDING_NAME="src_binding_${PROFILE}_${V8_TARGET}.rs"
CHECKSUMS_NAME="rusty_v8_${PROFILE}_${V8_TARGET}.sha256"
ARCHIVE_PATH="${OUTPUT_DIR}/${ARCHIVE_NAME}"
BINDING_PATH="${OUTPUT_DIR}/${BINDING_NAME}"
CHECKSUMS_PATH="${OUTPUT_DIR}/${CHECKSUMS_NAME}"

mkdir -p "${OUTPUT_DIR}"

if [[ -n "${CARGO_BUILD_DIR}" ]]; then
  for output in "${CARGO_BUILD_DIR}"/build/v8-*/output; do
    [[ -f "${output}" ]] || continue
    cached_archive="$(sed -n 's/^static lib URL: //p' "${output}" | tail -n 1)"
    cached_binding="$(sed -n 's/^cargo:rustc-env=RUSTY_V8_SRC_BINDING_PATH=//p' "${output}" | tail -n 1)"
    if [[ "${cached_archive}" == *"/rusty-v8-artifacts/${V8_VERSION}/${TARGET}/${ARCHIVE_NAME}" \
      && "${cached_binding}" == *"/rusty-v8-artifacts/${V8_VERSION}/${TARGET}/${BINDING_NAME}" \
      && -s "${cached_archive}" && -s "${cached_binding}" ]]; then
      printf 'RUSTY_V8_ARCHIVE=%q\n' "${cached_archive}"
      printf 'RUSTY_V8_SRC_BINDING_PATH=%q\n' "${cached_binding}"
      printf 'RUSTY_V8_RELEASE_REPO=%q\n' "${RELEASE_REPO}"
      exit 0
    fi
  done
fi

if [[ -n "${LOCAL_REPO}" && -s "${LOCAL_REPO}/${ARCHIVE_NAME}" \
  && -s "${LOCAL_REPO}/${BINDING_NAME}" ]]; then
  printf 'RUSTY_V8_ARCHIVE=%q\n' "${LOCAL_REPO}/${ARCHIVE_NAME}"
  printf 'RUSTY_V8_SRC_BINDING_PATH=%q\n' "${LOCAL_REPO}/${BINDING_NAME}"
  printf 'RUSTY_V8_RELEASE_REPO=%q\n' "${RELEASE_REPO}"
  exit 0
fi

if [[ -s "${ARCHIVE_PATH}" && -s "${BINDING_PATH}" && -s "${CHECKSUMS_PATH}" ]] \
  && (cd "${OUTPUT_DIR}" && sha256sum -c "${CHECKSUMS_NAME}" >/dev/null 2>&1); then
  printf 'RUSTY_V8_ARCHIVE=%q\n' "${ARCHIVE_PATH}"
  printf 'RUSTY_V8_SRC_BINDING_PATH=%q\n' "${BINDING_PATH}"
  printf 'RUSTY_V8_RELEASE_REPO=%q\n' "${RELEASE_REPO}"
  exit 0
fi

release_has_assets() {
  local tag="$1" metadata
  metadata="$(curl --fail --silent --show-error --location \
    --connect-timeout 20 --max-time 60 \
    "https://api.github.com/repos/${RELEASE_REPO}/releases/tags/${tag}")" || return 1
  python3 -c '
import json
import sys

required = set(sys.argv[1:])
assets = {asset["name"] for asset in json.load(sys.stdin).get("assets", [])}
raise SystemExit(0 if required <= assets else 1)
' "${ARCHIVE_NAME}" "${BINDING_NAME}" "${CHECKSUMS_NAME}" <<<"${metadata}"
}

if [[ -z "${RELEASE_TAG}" && "${PREFER_LATEST}" == true ]]; then
  latest_tag="$(curl --fail --silent --show-error --location \
    --connect-timeout 20 --max-time 60 \
    "https://api.github.com/repos/${RELEASE_REPO}/releases/latest" \
    | python3 -c 'import json, sys; print(json.load(sys.stdin).get("tag_name", ""))' \
    || true)"
  if [[ "${latest_tag}" == "rusty-v8-v${V8_VERSION}" ]] \
    && release_has_assets "${latest_tag}"; then
    RELEASE_TAG="${latest_tag}"
    echo "Using latest suitable Rusty V8 release: ${RELEASE_TAG}" >&2
  else
    RELEASE_TAG="rusty-v8-v${V8_VERSION}"
    echo "Latest Rusty V8 release is unsuitable; using pinned release: ${RELEASE_TAG}" >&2
  fi
else
  RELEASE_TAG="${RELEASE_TAG:-rusty-v8-v${V8_VERSION}}"
fi

BASE_URL="https://github.com/${RELEASE_REPO}/releases/download/${RELEASE_TAG}"
download_release_asset() {
  local name="$1"
  local path="$2"
  local partial_path="${path}.part"

  rm -f "${partial_path}"
  if curl --fail --silent --show-error --location \
    --retry 8 --retry-all-errors --retry-delay 5 --retry-max-time 180 \
    --connect-timeout 20 --max-time 300 \
    "${BASE_URL}/${name}" -o "${partial_path}"; then
    mv -- "${partial_path}" "${path}"
    return 0
  fi
  rm -f "${partial_path}"

  # Release downloads occasionally fail at GitHub's CDN while the release API
  # remains available. Use the API as a transport fallback, but retain the
  # checksum verification below as the trust boundary for every artifact.
  if command -v gh >/dev/null 2>&1 && [[ -n "${GH_TOKEN:-}" ]]; then
    local asset_id
    asset_id="$(gh api "repos/${RELEASE_REPO}/releases/tags/${RELEASE_TAG}" \
      --jq ".assets[] | select(.name == \"${name}\") | .id")" || asset_id=""
    if [[ -n "${asset_id}" ]] && gh api \
      --header 'Accept: application/octet-stream' \
      "repos/${RELEASE_REPO}/releases/assets/${asset_id}" > "${partial_path}"; then
      mv -- "${partial_path}" "${path}"
      return 0
    fi
    rm -f "${partial_path}"
  fi

  echo "Unable to download Rusty V8 release asset ${name} from ${RELEASE_REPO}:${RELEASE_TAG}" >&2
  return 1
}

[[ -s "${CHECKSUMS_PATH}" ]] || download_release_asset "${CHECKSUMS_NAME}" "${CHECKSUMS_PATH}"

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
  download_release_asset "${name}" "${path}"
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
