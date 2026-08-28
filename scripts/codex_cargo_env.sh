#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: codex_cargo_env.sh --source-repo DIR --build-repo DIR
  --mode debug|release --target-mode native|musl|armv7|android
  [--purpose NAME]
  [--emit|--print-target]
EOF
}

SOURCE_REPO=''
BUILD_REPO=''
MODE=debug
TARGET_MODE=native
PURPOSE="${CODEX_CARGO_PURPOSE:-default}"
OUTPUT=emit

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source-repo) SOURCE_REPO="${2:?missing source repository}"; shift 2 ;;
    --build-repo) BUILD_REPO="${2:?missing build repository}"; shift 2 ;;
    --mode) MODE="${2:?missing build mode}"; shift 2 ;;
    --target-mode) TARGET_MODE="${2:?missing target mode}"; shift 2 ;;
    --purpose) PURPOSE="${2:?missing target purpose}"; shift 2 ;;
    --emit) OUTPUT=emit; shift ;;
    --print-target) OUTPUT=target; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done

[[ -d "${SOURCE_REPO}/codex-rs" ]] || { echo "source repository not found: ${SOURCE_REPO}" >&2; exit 1; }
[[ -d "${BUILD_REPO}/codex-rs" ]] || { echo "build repository not found: ${BUILD_REPO}" >&2; exit 1; }
[[ "${MODE}" == debug || "${MODE}" == release ]] || { echo "invalid mode: ${MODE}" >&2; exit 2; }
[[ "${PURPOSE}" =~ ^[[:alnum:]_.-]+$ ]] || { echo "invalid target purpose: ${PURPOSE}" >&2; exit 2; }

source_lock_fingerprint() {
  python3 "${SOURCE_REPO}/scripts/normalize_cargo_lock.py" \
    --manifest "${BUILD_REPO}/codex-rs/Cargo.toml" \
    --source-lock "${SOURCE_REPO}/codex-rs/Cargo.lock" \
    --fingerprint
}

ensure_build_lockfile() {
  local source_fingerprint marker stored_fingerprint build_lock
  marker="${BUILD_REPO}/build/.cargo-source-lock-fingerprint"
  build_lock="${BUILD_REPO}/codex-rs/Cargo.lock"
  source_fingerprint="$(source_lock_fingerprint)"
  stored_fingerprint=''
  [[ -f "${marker}" ]] && read -r stored_fingerprint <"${marker}"
  if [[ -f "${build_lock}" ]] \
    && [[ "${source_fingerprint}" == "${stored_fingerprint}" ]] \
    && python3 "${SOURCE_REPO}/scripts/normalize_cargo_lock.py" \
      --manifest "${BUILD_REPO}/codex-rs/Cargo.toml" \
      --source-lock "${SOURCE_REPO}/codex-rs/Cargo.lock" \
      --build-lock "${build_lock}" \
    && cargo metadata --locked --offline --no-deps \
      --manifest-path "${BUILD_REPO}/codex-rs/Cargo.toml" >/dev/null 2>&1; then
    return
  fi

  mkdir -p "$(dirname "${marker}")"
  cp "${SOURCE_REPO}/codex-rs/Cargo.lock" "${build_lock}"
  python3 "${SOURCE_REPO}/scripts/normalize_cargo_lock.py" \
    --manifest "${BUILD_REPO}/codex-rs/Cargo.toml" \
    --source-lock "${SOURCE_REPO}/codex-rs/Cargo.lock" \
    --build-lock "${build_lock}"
  echo "Synchronizing workspace package versions in the build-tree Cargo.lock." >&2
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
  native) TARGET="${HOST_TARGET}"; TARGET_ROOT="${BUILD_REPO}/codex-rs/target" ;;
  musl) TARGET=x86_64-unknown-linux-musl; BASE_TARGET_DIR="${BUILD_REPO}/build/musl-${MODE}" ;;
  armv7) TARGET="${ARMV7_TARGET:-armv7-unknown-linux-musleabihf}"; BASE_TARGET_DIR="${BUILD_REPO}/build/armv7-${MODE}" ;;
  android) TARGET=aarch64-linux-android; BASE_TARGET_DIR="${BUILD_REPO}/build/android-${MODE}" ;;
  *) echo "invalid target mode: ${TARGET_MODE}" >&2; exit 2 ;;
esac
TARGET_ROOT="${TARGET_ROOT:-${BASE_TARGET_DIR}}"

LOCK_FILE="${BUILD_REPO}/codex-rs/Cargo.lock"
RUSTC_VERSION="$(${RUSTC_BIN} -vV)"
LINKER_VALUE="${CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER:-}"
RUSTFLAGS_VALUE="${CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS:-}"
FINGERPRINT_RUSTFLAGS_VALUE="${RUSTFLAGS_VALUE}"
MOLD_BIN="$(command -v mold || true)"
LINKER_IDENTITY=unavailable

if [[ "${TARGET}" == aarch64-linux-android && "${HOST_TARGET}" == aarch64-linux-android ]]; then
  ANDROID_CLANG="$(command -v aarch64-linux-android-clang || true)"
  [[ -x "${ANDROID_CLANG}" ]] || { echo "Android linker not found" >&2; exit 1; }
  ANDROID_BUILTINS="$(${ANDROID_CLANG} -print-file-name=libclang_rt.builtins-aarch64-android.a)"
  [[ -s "${ANDROID_BUILTINS}" ]] || { echo "Android compiler builtins archive not found" >&2; exit 1; }
  LINKER_VALUE="${ANDROID_CLANG}"
  FINGERPRINT_RUSTFLAGS_VALUE="-Clink-arg=-lc++_shared -Clink-arg=-Wl,-rpath,\$ORIGIN -Clink-arg=${ANDROID_BUILTINS}"
  RUSTFLAGS_VALUE="-Clink-arg=${ANDROID_BUILTINS}"
fi

if [[ "${TARGET}" == aarch64-linux-android && "${HOST_TARGET}" == aarch64-linux-android ]]; then
  if [[ -n "${MOLD_BIN}" ]]; then
    mold_identity="$("${MOLD_BIN}" --version | head -n 1):$(sha256sum "${MOLD_BIN}" | awk '{print $1}')"
    mold_probe_dir="${TMPDIR:-${RUNNER_TEMP:-/var/tmp}}/codex-mold-probe"
    mkdir -p "${mold_probe_dir}"
    mold_probe_src="${mold_probe_dir}/probe.c"
    mold_probe_bin="${mold_probe_dir}/probe"
    printf '%s\n' 'int main(void) { return 0; }' >"${mold_probe_src}"
    if "${ANDROID_CLANG}" -fuse-ld=mold "${mold_probe_src}" -o "${mold_probe_bin}" >/dev/null 2>&1; then
      RUSTFLAGS_VALUE+=" -Clink-arg=-fuse-ld=mold"
      FINGERPRINT_RUSTFLAGS_VALUE+=" -Clink-arg=-fuse-ld=mold"
      LINKER_IDENTITY="mold:${mold_identity}"
    fi
    rm -f "${mold_probe_src}" "${mold_probe_bin}"
  fi
  if [[ "${LINKER_IDENTITY}" == unavailable ]]; then
    lld_bin="$(command -v ld.lld || true)"
    [[ -n "${lld_bin}" ]] || { echo "Neither Mold nor LLD is available for Android linking" >&2; exit 1; }
    lld_identity="$("${lld_bin}" --version | head -n 1):$(sha256sum "${lld_bin}" | awk '{print $1}')"
    mold_probe_dir="${TMPDIR:-${RUNNER_TEMP:-/var/tmp}}/codex-lld-probe"
    mkdir -p "${mold_probe_dir}"
    mold_probe_src="${mold_probe_dir}/probe.c"
    mold_probe_bin="${mold_probe_dir}/probe"
    printf '%s\n' 'int main(void) { return 0; }' >"${mold_probe_src}"
    if ! "${ANDROID_CLANG}" -fuse-ld=lld "${mold_probe_src}" -o "${mold_probe_bin}" >/dev/null 2>&1; then
      rm -f "${mold_probe_src}" "${mold_probe_bin}"
      echo "Neither Mold nor LLD can link the native Android target" >&2
      exit 1
    fi
    rm -f "${mold_probe_src}" "${mold_probe_bin}"
    RUSTFLAGS_VALUE+=" -Clink-arg=-fuse-ld=lld"
    FINGERPRINT_RUSTFLAGS_VALUE+=" -Clink-arg=-fuse-ld=lld"
    LINKER_IDENTITY="lld:${lld_identity}"
  fi
fi
if [[ "${LINKER_IDENTITY}" == unavailable && -n "${LINKER_VALUE}" && -x "${LINKER_VALUE}" ]]; then
  LINKER_IDENTITY="driver:${LINKER_VALUE}:$(sha256sum "${LINKER_VALUE}" | awk '{print $1}')"
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
LOCK_GRAPH_FINGERPRINT="$(python3 "${SOURCE_REPO}/scripts/normalize_cargo_lock.py" \
  --manifest "${BUILD_REPO}/codex-rs/Cargo.toml" \
  --source-lock "${LOCK_FILE}" --fingerprint)"

FINGERPRINT_INPUT="$(printf '%s\n' \
  "mode=${MODE}" "target_mode=${TARGET_MODE}" "target=${TARGET}" \
  "host=${HOST_TARGET}" "rustc=${RUSTC_VERSION}" \
  "lockfile=${LOCK_GRAPH_FINGERPRINT}" \
  "linker=${LINKER_VALUE}" "linker_identity=${LINKER_IDENTITY}" "rustflags=${FINGERPRINT_RUSTFLAGS_VALUE}" \
  "openssl=${OPENSSL_VERSION}:${OPENSSL_IDENTITY}" \
  "rusty_v8=${RUSTY_V8_VERSION}:${RUSTY_V8_IDENTITY}" \
  'CODEX_BUILD_TIMESTAMP=0000000000-000000000000')"
FINGERPRINT="$(printf '%s' "${FINGERPRINT_INPUT}" | sha256sum | awk '{print substr($1, 1, 16)}')"
TARGET_DIR="${TARGET_ROOT}.${FINGERPRINT}"
MARKER="${TARGET_DIR}/.codex-cargo-fingerprint"
if [[ -e "${TARGET_DIR}" ]]; then
  if [[ ! -f "${MARKER}" ]] || ! grep -Fxq "${FINGERPRINT}" "${MARKER}"; then
    echo "target directory exists with an incompatible fingerprint: ${TARGET_DIR}" >&2
    exit 1
  fi
fi
mkdir -p "${TARGET_DIR}"
printf '%s\n' "${FINGERPRINT}" >"${TARGET_DIR}/.codex-cargo-fingerprint"

TARGET_LINK="${BUILD_REPO}/codex-rs/target-${PURPOSE}"
if [[ -e "${TARGET_LINK}" && ! -L "${TARGET_LINK}" ]]; then
  echo "target purpose link is not a symbolic link: ${TARGET_LINK}" >&2
  exit 1
fi
ln -sfn "${TARGET_DIR}" "${TARGET_LINK}"

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
if [[ "${TARGET}" == aarch64-linux-android && "${HOST_TARGET}" == aarch64-linux-android ]]; then
  printf 'export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-1} CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=%q CC_aarch64_linux_android=%q CXX_aarch64_linux_android=%q AR_aarch64_linux_android=llvm-ar RANLIB_aarch64_linux_android=llvm-ranlib CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS=%q PROTOC=%q\n' \
    "${ANDROID_CLANG}" "${ANDROID_CLANG}" "${ANDROID_CLANG}++" "${RUSTFLAGS_VALUE}" "$(command -v protoc)"
fi
