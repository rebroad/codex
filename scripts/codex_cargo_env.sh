#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: codex_cargo_env.sh --source-repo DIR --build-repo DIR
  --mode debug|release --target-mode native|musl|armv7|android
  [--emit|--print-target]
EOF
}

SOURCE_REPO=''
BUILD_REPO=''
MODE=debug
TARGET_MODE=native
OUTPUT=emit

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source-repo) SOURCE_REPO="${2:?missing source repository}"; shift 2 ;;
    --build-repo) BUILD_REPO="${2:?missing build repository}"; shift 2 ;;
    --mode) MODE="${2:?missing build mode}"; shift 2 ;;
    --target-mode) TARGET_MODE="${2:?missing target mode}"; shift 2 ;;
    --emit) OUTPUT=emit; shift ;;
    --print-target) OUTPUT=target; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done

[[ -d "${SOURCE_REPO}/codex-rs" ]] || { echo "source repository not found: ${SOURCE_REPO}" >&2; exit 1; }
[[ -d "${BUILD_REPO}/codex-rs" ]] || { echo "build repository not found: ${BUILD_REPO}" >&2; exit 1; }
[[ "${MODE}" == debug || "${MODE}" == release ]] || { echo "invalid mode: ${MODE}" >&2; exit 2; }

source_lock_fingerprint() {
  sed '/^version = /d' "${SOURCE_REPO}/codex-rs/Cargo.lock" | sha256sum | awk '{print $1}'
}

ensure_build_lockfile() {
  local source_fingerprint marker stored_fingerprint
  marker="${BUILD_REPO}/build/.cargo-source-lock-fingerprint"
  source_fingerprint="$(source_lock_fingerprint)"
  stored_fingerprint=''
  [[ -f "${marker}" ]] && read -r stored_fingerprint <"${marker}"
  if [[ -f "${BUILD_REPO}/codex-rs/Cargo.lock" ]] \
    && [[ "${source_fingerprint}" == "${stored_fingerprint}" ]] \
    && cargo metadata --locked --offline --no-deps \
      --manifest-path "${BUILD_REPO}/codex-rs/Cargo.toml" >/dev/null 2>&1; then
    return
  fi

  mkdir -p "$(dirname "${marker}")"
  cp "${SOURCE_REPO}/codex-rs/Cargo.lock" "${BUILD_REPO}/codex-rs/Cargo.lock"
  echo "Synchronizing the build-tree Cargo.lock from the source lockfile." >&2
  if ! cargo metadata --locked --offline --no-deps \
    --manifest-path "${BUILD_REPO}/codex-rs/Cargo.toml" >/dev/null 2>&1; then
    echo "The source Cargo.lock is not valid for this workspace." >&2
    return 1
  fi
  printf '%s\n' "${source_fingerprint}" >"${marker}"
}

ensure_build_lockfile

RUSTC_BIN="${RUSTC:-rustc}"
HOST_TARGET="$(${RUSTC_BIN} -vV | sed -n 's/^host: //p')"
SCCACHE_BIN="$(command -v sccache || true)"
case "${TARGET_MODE}" in
  native) TARGET="${HOST_TARGET}"; BASE_TARGET_DIR="${BUILD_REPO}/codex-rs/target" ;;
  musl) TARGET=x86_64-unknown-linux-musl; BASE_TARGET_DIR="${BUILD_REPO}/build/musl-${MODE}" ;;
  armv7) TARGET="${ARMV7_TARGET:-armv7-unknown-linux-musleabihf}"; BASE_TARGET_DIR="${BUILD_REPO}/build/armv7-${MODE}" ;;
  android) TARGET=aarch64-linux-android; BASE_TARGET_DIR="${BUILD_REPO}/build/android-${MODE}" ;;
  *) echo "invalid target mode: ${TARGET_MODE}" >&2; exit 2 ;;
esac

LOCK_FILE="${BUILD_REPO}/codex-rs/Cargo.lock"
RUSTC_VERSION="$(${RUSTC_BIN} -vV)"
LINKER_VALUE="${CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER:-}"
RUSTFLAGS_VALUE="${CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS:-}"

if [[ "${HOST_TARGET}" == aarch64-linux-android ]]; then
  ANDROID_CLANG="$(command -v aarch64-linux-android-clang || true)"
  [[ -x "${ANDROID_CLANG}" ]] || { echo "Android linker not found" >&2; exit 1; }
  ANDROID_BUILTINS="$(${ANDROID_CLANG} -print-file-name=libclang_rt.builtins-aarch64-android.a)"
  [[ -s "${ANDROID_BUILTINS}" ]] || { echo "Android compiler builtins archive not found" >&2; exit 1; }
  LINKER_VALUE="${ANDROID_CLANG}"
  RUSTFLAGS_VALUE="-Clink-arg=-lc++_shared -Clink-arg=-Wl,-rpath,\$ORIGIN -Clink-arg=${ANDROID_BUILTINS}"
fi

OPENSSL_VERSION="$(bash "${SOURCE_REPO}/scripts/openssl_artifacts.sh" version "${LOCK_FILE}")"
OPENSSL_DIR_VALUE=''
OPENSSL_IDENTITY=vendored
if OPENSSL_ENV="$(bash "${SOURCE_REPO}/scripts/openssl_artifacts.sh" env "${BUILD_REPO}" "${TARGET}" "${OPENSSL_VERSION}" 2>/dev/null)"; then
  eval "${OPENSSL_ENV}"
  OPENSSL_DIR_VALUE="${OPENSSL_DIR}"
  OPENSSL_IDENTITY="${OPENSSL_DIR}:$(sha256sum "${OPENSSL_DIR}/.metadata" | awk '{print $1}')"
fi

RUSTY_V8_VERSION="$(python3 "${SOURCE_REPO}/scripts/rusty_v8_version.py" "${LOCK_FILE}")"
RUSTY_V8_DIR="${BUILD_REPO}/build/rusty-v8-artifacts/${RUSTY_V8_VERSION}/${TARGET}"
RUSTY_V8_ENV="$(bash "${SOURCE_REPO}/scripts/resolve_rusty_v8_artifacts.sh" \
  --target="${TARGET}" --output-dir="${RUSTY_V8_DIR}" \
  --cargo-build-dir="${BUILD_REPO}/codex-rs/target" 2>/dev/null)"
eval "${RUSTY_V8_ENV}"
RUSTY_V8_IDENTITY="$(sha256sum "${RUSTY_V8_ARCHIVE}" "${RUSTY_V8_SRC_BINDING_PATH}" | awk '{print $1}' | tr '\n' ':')"

FINGERPRINT_INPUT="$(printf '%s\n' \
  "mode=${MODE}" "target_mode=${TARGET_MODE}" "target=${TARGET}" \
  "host=${HOST_TARGET}" "rustc=${RUSTC_VERSION}" \
  "linker=${LINKER_VALUE}" "rustflags=${RUSTFLAGS_VALUE}" \
  "openssl=${OPENSSL_VERSION}:${OPENSSL_IDENTITY}" \
  "rusty_v8=${RUSTY_V8_VERSION}:${RUSTY_V8_IDENTITY}" \
  'CODEX_BUILD_TIMESTAMP=0000000000-000000000000')"
FINGERPRINT="$(printf '%s' "${FINGERPRINT_INPUT}" | sha256sum | awk '{print substr($1, 1, 16)}')"
MARKER="${BASE_TARGET_DIR}/.codex-cargo-fingerprint"
TARGET_DIR="${BASE_TARGET_DIR}"
if [[ -e "${BASE_TARGET_DIR}" && -f "${MARKER}" ]] \
  && ! grep -Fxq "${FINGERPRINT}" "${MARKER}"; then
  TARGET_DIR="${BASE_TARGET_DIR}.${FINGERPRINT}"
fi
mkdir -p "${TARGET_DIR}"
printf '%s\n' "${FINGERPRINT}" >"${TARGET_DIR}/.codex-cargo-fingerprint"

if [[ "${OUTPUT}" == target ]]; then
  printf '%s\n' "${TARGET_DIR}"
  exit 0
fi

printf '%s\n' 'unset CC CXX AR RANLIB CFLAGS CXXFLAGS TARGET_CC TARGET_CXX TARGET_AR TARGET_RANLIB PKG_CONFIG_ALLOW_CROSS PKG_CONFIG_ALL_STATIC PKG_CONFIG_PATH PKG_CONFIG_LIBDIR PKG_CONFIG_SYSROOT_DIR CMAKE_C_COMPILER CMAKE_CXX_COMPILER CMAKE_ARGS'
printf 'export RUSTUP_DISABLE_SELF_UPDATE=1 CARGO_TARGET_DIR=%q CODEX_BUILD_TIMESTAMP=0000000000-000000000000 CODEX_BUILD_FINGERPRINT=%q\n' "${TARGET_DIR}" "${FINGERPRINT}"
if [[ -n "${SCCACHE_BIN}" ]]; then
  printf 'export RUSTC_WRAPPER=%q SCCACHE_DIR=%q\n' "${SCCACHE_BIN}" "${SCCACHE_DIR:-${HOME}/.cache/sccache}"
fi
[[ -n "${CARGO_BUILD_JOBS:-}" ]] && printf 'export CARGO_BUILD_JOBS=%q\n' "${CARGO_BUILD_JOBS}"
if [[ -n "${OPENSSL_DIR_VALUE}" ]]; then
  printf 'export OPENSSL_DIR=%q OPENSSL_NO_VENDOR=1 OPENSSL_STATIC=1\n' "${OPENSSL_DIR_VALUE}"
fi
printf 'export RUSTY_V8_ARCHIVE=%q RUSTY_V8_SRC_BINDING_PATH=%q\n' "${RUSTY_V8_ARCHIVE}" "${RUSTY_V8_SRC_BINDING_PATH}"
if [[ "${HOST_TARGET}" == aarch64-linux-android ]]; then
  printf 'export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-1} CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=%q CC_aarch64_linux_android=%q CXX_aarch64_linux_android=%q AR_aarch64_linux_android=llvm-ar RANLIB_aarch64_linux_android=llvm-ranlib CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS=%q PROTOC=%q\n' \
    "${ANDROID_CLANG}" "${ANDROID_CLANG}" "${ANDROID_CLANG}++" "${RUSTFLAGS_VALUE}" "$(command -v protoc)"
fi
