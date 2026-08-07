#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_TREE=""
for candidate in "${ROOT}.build" "${ROOT}.make"; do
  if [[ -d "${candidate}" ]]; then BUILD_TREE="${candidate}"; break; fi
done
[[ -n "${BUILD_TREE}" ]] || { echo "No sibling build tree found" >&2; exit 1; }

VERSION="${1:-}"
PROFILE="${2:-release}"
if [[ "${VERSION}" == --help || "${VERSION}" == -h ]]; then
  cat <<'EOF'
Usage: scripts/package-npm.sh [VERSION] [debug|release]
Stages locally built native targets using scripts/stage_npm_packages.py.
EOF
  exit 0
fi
[[ -n "${VERSION}" ]] || VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${ROOT}/codex-rs/Cargo.toml" | head -n 1)"
[[ "${PROFILE}" == debug || "${PROFILE}" == release ]] || { echo "profile must be debug or release" >&2; exit 2; }

OUTPUT_DIR="${OUTPUT_DIR:-${BUILD_TREE}/npm-artifact}"
VENDOR_ROOT="$(mktemp -d "${BUILD_TREE}/npm-vendor.XXXXXX")"
trap 'rm -rf "${VENDOR_ROOT}"' EXIT
mkdir -p "${OUTPUT_DIR}"
rm -f "${OUTPUT_DIR}"/*.tgz

profile_path() { [[ "${1}" == release ]] && echo release || echo debug; }
require_binary() {
  [[ -x "${1}" ]] || {
    echo "Missing executable: ${1}" >&2
    echo "Build it with scripts/rebuild_codex.sh --release --build-npm-vendor" >&2
    exit 1
  }
}
stage_binary() {
  local target="${1}" binary="${2}" host="${3:-}" bwrap="${4:-}"
  require_binary "${binary}"
  mkdir -p "${VENDOR_ROOT}/${target}/bin"
  install -m 0755 "${binary}" "${VENDOR_ROOT}/${target}/bin/codex"
  if [[ -n "${host}" ]]; then
    require_binary "${host}"
    install -m 0755 "${host}" "${VENDOR_ROOT}/${target}/bin/codex-code-mode-host"
  fi
  if [[ -n "${bwrap}" ]]; then
    require_binary "${bwrap}"
    mkdir -p "${VENDOR_ROOT}/${target}/codex-resources"
    install -m 0755 "${bwrap}" "${VENDOR_ROOT}/${target}/codex-resources/bwrap"
  fi
}

MUSL_DIR="${BUILD_TREE}/build/musl-${PROFILE}/x86_64-unknown-linux-musl/$(profile_path "${PROFILE}")"
ARMV7_DIR="${BUILD_TREE}/build/armv7-${PROFILE}/${ARMV7_TARGET:-armv7-unknown-linux-gnueabihf}/$(profile_path "${PROFILE}")"
stage_binary x86_64-unknown-linux-musl "${MUSL_DIR}/codex" "${MUSL_DIR}/codex-code-mode-host" "${MUSL_DIR}/bwrap"
stage_binary armv7-unknown-linux-gnueabihf "${ARMV7_DIR}/codex" "${ARMV7_DIR}/codex-code-mode-host"

ANDROID_STAGE="${BUILD_TREE}/build/android-artifact"
if [[ -x "${ANDROID_STAGE}/codex.bin" && -f "${ANDROID_STAGE}/libc++_shared.so" ]]; then
  mkdir -p "${VENDOR_ROOT}/aarch64-linux-android/bin"
  install -m 0755 "${ANDROID_STAGE}/codex.bin" "${VENDOR_ROOT}/aarch64-linux-android/bin/codex"
  install -m 0644 "${ANDROID_STAGE}/libc++_shared.so" "${VENDOR_ROOT}/aarch64-linux-android/bin/libc++_shared.so"
fi

python3 "${ROOT}/scripts/stage_npm_packages.py" \
  --release-version "${VERSION}" \
  --package codex \
  --vendor-src "${VENDOR_ROOT}" \
  --output-dir "${OUTPUT_DIR}"
