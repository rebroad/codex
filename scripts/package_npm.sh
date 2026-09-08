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
TARGETS="${3:-musl,armv7,android}"
if [[ "${VERSION}" == --help || "${VERSION}" == -h ]]; then
  cat <<'EOF'
Usage: scripts/package_npm.sh [VERSION] [debug|release] [TARGETS] [options]
Stages selected locally built targets using scripts/stage_npm_packages.py.
TARGETS is a comma-separated list of musl, armv7, and android (default: musl,armv7,android).

Complete local mode:
  --vendor-root PATH  merge the staged targets with a local upstream-style
                      vendor tree, then assemble and audit all nine packages.
  --fork-artifact-dir PATH
                      reuse existing local fork archives instead of staging
  --output-dir PATH   write package archives to PATH
EOF
  exit 0
fi
UPSTREAM_VENDOR_ROOT=""
REUSE_FORK_ARTIFACT_DIR=""
OUTPUT_DIR_OVERRIDE=""
shift $(( $# >= 3 ? 3 : $# ))
while (($#)); do
  case "$1" in
    --vendor-root) UPSTREAM_VENDOR_ROOT="${2:-}"; shift 2 ;;
    --vendor-root=*) UPSTREAM_VENDOR_ROOT="${1#*=}"; shift ;;
    --fork-artifact-dir) REUSE_FORK_ARTIFACT_DIR="${2:-}"; shift 2 ;;
    --fork-artifact-dir=*) REUSE_FORK_ARTIFACT_DIR="${1#*=}"; shift ;;
    --output-dir) OUTPUT_DIR_OVERRIDE="${2:-}"; shift 2 ;;
    --output-dir=*) OUTPUT_DIR_OVERRIDE="${1#*=}"; shift ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done
[[ -z "${REUSE_FORK_ARTIFACT_DIR}" || -n "${UPSTREAM_VENDOR_ROOT}" ]] || {
  echo "--fork-artifact-dir requires --vendor-root" >&2
  exit 2
}
[[ -n "${VERSION}" ]] || VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${ROOT}/codex-rs/Cargo.toml" | head -n 1)"
[[ "${PROFILE}" == debug || "${PROFILE}" == release ]] || { echo "profile must be debug or release" >&2; exit 2; }
IFS=',' read -r -a SELECTED_TARGETS <<<"${TARGETS}"
[[ "${#SELECTED_TARGETS[@]}" -gt 0 ]] || { echo "targets must not be empty" >&2; exit 2; }
for target in "${SELECTED_TARGETS[@]}"; do
  case "${target}" in
    musl|armv7|android) ;;
    *) echo "unsupported npm target: ${target}" >&2; exit 2 ;;
  esac
done

OUTPUT_DIR="${OUTPUT_DIR_OVERRIDE:-${OUTPUT_DIR:-${BUILD_TREE}/build/npm-artifact}}"
mkdir -p "${OUTPUT_DIR}"
rm -f "${OUTPUT_DIR}/npm-artifact-sources.json"
# Target-scoped invocations must preserve previously staged architectures so
# local builds can be accumulated before the complete assembly/audit step.
shopt -s nullglob
for archive in "${OUTPUT_DIR}"/codex-npm-*.tgz; do
  case "$(basename "${archive}")" in
    codex-npm-linux-x64-*|codex-npm-linux-arm64-*|codex-npm-darwin-x64-*|\
    codex-npm-darwin-arm64-*|codex-npm-win32-x64-*|codex-npm-win32-arm64-*|\
    codex-npm-linux-armv7-*|codex-npm-android-arm64-*) ;;
    *) rm -f "${archive}" "${archive}.sha256" ;;
  esac
done
for target in "${SELECTED_TARGETS[@]}"; do
  case "${target}" in
    musl) platform="linux-x64" ;;
    armv7) platform="linux-armv7" ;;
    android) platform="android-arm64" ;;
  esac
  rm -f "${OUTPUT_DIR}/codex-npm-${platform}-"*.tgz \
    "${OUTPUT_DIR}/codex-npm-${platform}-"*.tgz.sha256
done
VENDOR_ROOT="$(mktemp -d "${BUILD_TREE}/npm-vendor.XXXXXX")"
FORK_ARTIFACT_DIR=""
if [[ -n "${UPSTREAM_VENDOR_ROOT}" ]]; then
  UPSTREAM_VENDOR_ROOT="$(cd -- "${UPSTREAM_VENDOR_ROOT}" && pwd)"
  if [[ -n "${REUSE_FORK_ARTIFACT_DIR}" ]]; then
    FORK_ARTIFACT_DIR="$(cd -- "${REUSE_FORK_ARTIFACT_DIR}" && pwd)"
    trap 'rm -rf "${VENDOR_ROOT}"' EXIT
    STAGE_OUTPUT_DIR=""
  else
    FORK_ARTIFACT_DIR="$(mktemp -d "${BUILD_TREE}/npm-local-fork.XXXXXX")"
    trap 'rm -rf "${VENDOR_ROOT}" "${FORK_ARTIFACT_DIR}"' EXIT
    STAGE_OUTPUT_DIR="${FORK_ARTIFACT_DIR}"
  fi
else
  trap 'rm -rf "${VENDOR_ROOT}"' EXIT
  STAGE_OUTPUT_DIR="${OUTPUT_DIR}"
fi

profile_path() { [[ "${1}" == release ]] && echo release || echo debug; }
require_binary() {
  [[ -x "${1}" ]] || {
    echo "Missing executable: ${1}" >&2
    echo "Build it with scripts/build_codex.sh --release --build-npm-vendor" >&2
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

if [[ -z "${REUSE_FORK_ARTIFACT_DIR}" ]]; then
  for target in "${SELECTED_TARGETS[@]}"; do
    case "${target}" in
      musl)
        MUSL_DIR="${BUILD_TREE}/build/musl-${PROFILE}/x86_64-unknown-linux-musl/$(profile_path "${PROFILE}")"
        stage_binary x86_64-unknown-linux-musl "${MUSL_DIR}/codex" "${MUSL_DIR}/codex-code-mode-host" "${MUSL_DIR}/bwrap"
        ;;
      armv7)
        ARMV7_TARGET_TRIPLE="${ARMV7_TARGET:-armv7-unknown-linux-musleabihf}"
        ARMV7_DIR="${BUILD_TREE}/build/armv7-${PROFILE}/${ARMV7_TARGET_TRIPLE}/$(profile_path "${PROFILE}")"
        stage_binary "${ARMV7_TARGET_TRIPLE}" "${ARMV7_DIR}/codex" "${ARMV7_DIR}/codex-code-mode-host"
        ;;
      android)
        ANDROID_STAGE="${BUILD_TREE}/build/android-artifact"
        [[ -x "${ANDROID_STAGE}/codex.bin" && -x "${ANDROID_STAGE}/codex-code-mode-host" && -f "${ANDROID_STAGE}/libc++_shared.so" ]] || {
          echo "Missing Android npm artifacts under ${ANDROID_STAGE}" >&2
          exit 1
        }
        mkdir -p "${VENDOR_ROOT}/aarch64-linux-android/bin"
        install -m 0755 "${ANDROID_STAGE}/codex.bin" "${VENDOR_ROOT}/aarch64-linux-android/bin/codex"
        install -m 0755 "${ANDROID_STAGE}/codex-code-mode-host" "${VENDOR_ROOT}/aarch64-linux-android/bin/codex-code-mode-host"
        install -m 0644 "${ANDROID_STAGE}/libc++_shared.so" "${VENDOR_ROOT}/aarch64-linux-android/bin/libc++_shared.so"
        ;;
    esac
  done

  python3 "${ROOT}/scripts/stage_npm_packages.py" \
    --release-version "${VERSION}" \
    --package codex \
    --vendor-src "${VENDOR_ROOT}" \
    --output-dir "${STAGE_OUTPUT_DIR}"
fi

if [[ -n "${UPSTREAM_VENDOR_ROOT}" ]]; then
  BASE_VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${ROOT}/codex-rs/Cargo.toml" | head -n 1)"
  python3 "${ROOT}/scripts/assemble_npm_packages.py" \
    --release-version "${VERSION}" \
    --output-dir "${OUTPUT_DIR}" \
    --fork-artifact-dir "${FORK_ARTIFACT_DIR}" \
    --upstream-version "${BASE_VERSION}" \
    --upstream-vendor-src "${UPSTREAM_VENDOR_ROOT}"
fi

for archive in "${OUTPUT_DIR}"/codex-npm-*.tgz; do
  sha256sum "${archive}" >"${archive}.sha256"
done

if [[ -n "${UPSTREAM_VENDOR_ROOT}" ]]; then
  python3 "${ROOT}/scripts/audit_npm_packages.py" \
    --artifact-dir "${OUTPUT_DIR}" \
    --expected-version "${VERSION}"
fi
