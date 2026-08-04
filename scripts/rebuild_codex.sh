#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
BUILD_TREE=""
SCRIPT_REPO="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
case "${SCRIPT_REPO##*/}" in
  *.build|*.make)
    BUILD_TREE="${SCRIPT_REPO}"
    SOURCE_REPO="${SCRIPT_REPO%.*}"
    ;;
  *)
    SOURCE_REPO="${SCRIPT_REPO}"
    for candidate in "${SOURCE_REPO}.build" "${SOURCE_REPO}.make"; do
      if [[ -d "${candidate}" ]]; then BUILD_TREE="${candidate}"; break; fi
    done
    ;;
esac
if [[ -z "${BUILD_TREE}" ]]; then
  echo "No sibling build tree found; expected ${SOURCE_REPO}.build or ${SOURCE_REPO}.make" >&2
  exit 1
fi
if [[ ! -d "${SOURCE_REPO}/codex-rs" ]]; then
  echo "Source checkout not found at ${SOURCE_REPO}" >&2
  exit 1
fi
BUILD_REPO="${BUILD_TREE}"
BUILD_WORKSPACE="${BUILD_REPO}/codex-rs"
INSTALL_BIN_DIR="${INSTALL_BIN_DIR:-${HOME}/.cargo/bin}"
VERSION=""
MODE="debug"
TARGET_MODE="native"
PUBLISH="false"
PUBLISH_NPM="false"
PACKAGE_NPM="false"
PREFLIGHT_ONLY="false"
DRY_RUN="false"
SYNCED="false"
TIMESTAMP="$(date +%Y%m%d%H%M)"
COMMIT_SHORT=""
TOOLCHAIN=""

usage() {
  cat <<'EOF'
Usage: scripts/rebuild_codex.sh [options]

Builds from the source checkout into the first existing sibling build tree
(<repo>.build or <repo>.make), retaining Cargo's incremental cache.

Options:
  --debug                  Build debug (default)
  --release                Build optimized release
  --target <name>          native, musl, armv7, or android
  --armv7                  Alias for --target armv7
  --build-npm-vendor       Build Linux musl/ARMv7 payloads for npm packaging
  --package-npm            Build local @reb.ai/codex npm archives
  --publish-npm            Publish npm archives (implies --package-npm)
  --dry-run                Use npm/GitHub dry-run checks where supported
  --publish                Create/push codex-v<version> and wait for GitHub release
  --preflight-only         Run syntax/tooling checks without compiling
  --no-sync                Reuse the already-synced sibling source tree
  --jobs N                 Set CARGO_BUILD_JOBS
  --install-dir PATH       Install versioned binary and codex symlink there
  -h, --help               Show this help
EOF
}

die() { echo "rebuild_codex.sh: $*" >&2; exit 1; }
require_cmd() { command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"; }

sync_sources() {
  [[ "${SYNCED}" == true ]] && return
  if [[ "${NO_SYNC:-0}" == 1 ]]; then SYNCED=true; return; fi
  if command -v cpto >/dev/null 2>&1; then
    cpto --no-lngit --nogit "${SOURCE_REPO}" "${BUILD_REPO}"
  else
    echo "cpto not found; syncing source without deleting build artifacts" >&2
    tar --exclude='./.git' -cf - -C "${SOURCE_REPO}" . | tar -xf - -C "${BUILD_REPO}"
  fi
  # Cargo.lock is source-controlled and must match Cargo.toml. Keep generated
  # build-tree changes from making the next --locked build fail.
  if ! cmp -s "${SOURCE_REPO}/codex-rs/Cargo.lock" "${BUILD_WORKSPACE}/Cargo.lock"; then
    cp -p "${SOURCE_REPO}/codex-rs/Cargo.lock" "${BUILD_WORKSPACE}/Cargo.lock"
  fi
  SYNCED=true
}

workspace_version() {
  sed -n 's/^version = "\([^"]*\)"/\1/p' "${SOURCE_REPO}/codex-rs/Cargo.toml" | head -n 1
}

read_toolchain() {
  TOOLCHAIN="${RUST_TOOLCHAIN:-}"
  if [[ -z "${TOOLCHAIN}" && -f "${BUILD_WORKSPACE}/rust-toolchain.toml" ]]; then
    TOOLCHAIN="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "${BUILD_WORKSPACE}/rust-toolchain.toml" | head -n 1)"
  fi
  TOOLCHAIN="${TOOLCHAIN:-stable}"
}

cargo_target_dir() {
  case "${1}" in
    native) echo "${BUILD_REPO}/cargo-target-linux" ;;
    musl) echo "${BUILD_REPO}/cargo-target-musl" ;;
    armv7) echo "${BUILD_REPO}/cargo-target-armv7" ;;
    android) echo "${BUILD_REPO}/cargo-target-android" ;;
    *) die "unknown target mode: ${1}" ;;
  esac
}

target_triple() {
  case "${1}" in
    native) echo "" ;;
    musl) echo "x86_64-unknown-linux-musl" ;;
    armv7) echo "${ARMV7_TARGET:-armv7-unknown-linux-gnueabihf}" ;;
    android) echo "aarch64-linux-android" ;;
  esac
}

ensure_target() {
  local triple="${1}"
  if ! rustup target list --toolchain "${TOOLCHAIN}" --installed | grep -Fxq "${triple}"; then
    echo "Installing Rust target ${triple} for ${TOOLCHAIN}..." >&2
    rustup target add --toolchain "${TOOLCHAIN}" "${triple}"
  fi
}

cargo_build() {
  local mode="${1}" target_mode="${2}" profile_args=() target triple target_dir
  [[ "${mode}" == release ]] && profile_args+=(--release)
  triple="$(target_triple "${target_mode}")"
  target_dir="$(cargo_target_dir "${target_mode}")"
  mkdir -p "${target_dir}"
  if [[ -n "${triple}" ]]; then
    target="${triple}"
    ensure_target "${triple}"
  else
    target=""
  fi

  local -a env_args=(CARGO_TARGET_DIR="${target_dir}" RUSTUP_DISABLE_SELF_UPDATE=1)
  [[ -n "${CARGO_BUILD_JOBS:-}" ]] && env_args+=(CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS}")
  if [[ "${target_mode}" == armv7 ]]; then
    local armv7_cc="${ARMV7_LINKER:-arm-linux-gnueabihf-gcc}"
    require_cmd "${armv7_cc}"
    env_args+=(
      CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER="${armv7_cc}"
      CC_armv7_unknown_linux_gnueabihf="${armv7_cc}"
    )
  fi
  if [[ "${target_mode}" == musl ]]; then
    local musl_cc="${MUSL_CC:-}"
    if [[ -z "${musl_cc}" ]]; then
      musl_cc="$(command -v musl-gcc || true)"
    fi
    [[ -n "${musl_cc}" ]] || die "musl-gcc is required for the musl target (install musl-tools or set MUSL_CC)"
    env_args+=(
      CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="${MUSL_LINKER:-${musl_cc}}"
      CC_x86_64_unknown_linux_musl="${musl_cc}"
    )
  fi

  local -a cmd=(cargo +"${TOOLCHAIN}" build -p codex-cli -p codex-code-mode-host)
  [[ -n "${target}" ]] && cmd+=(--target "${target}")
  cmd+=( "${profile_args[@]}" --locked )
  echo "Building ${mode} ${target_mode} in ${target_dir} (incremental cache retained)..." >&2
  if ! (cd "${BUILD_WORKSPACE}" && env "${env_args[@]}" "${cmd[@]}") >&2; then
    echo "Locked Cargo build failed; retrying offline in the build tree without --locked." >&2
    unset 'cmd[-1]'
    cmd+=(--offline)
    if ! (cd "${BUILD_WORKSPACE}" && env "${env_args[@]}" "${cmd[@]}") >&2; then
      return 1
    fi
  fi
  if [[ -n "${target}" ]]; then
    echo "${target_dir}/${target}/${mode}/codex"
  else
    echo "${target_dir}/${mode}/codex"
  fi
}

patch_timestamp() {
  local binary="${1}" version="${2}" suffix="${3}-${TIMESTAMP}"
  python3 - "${binary}" "${version}" "${suffix}" <<'PY'
import mmap, sys
from pathlib import Path
path = Path(sys.argv[1])
needle = (sys.argv[2] + "-000000000000-000000000000").encode()
replacement = (sys.argv[2] + "-" + sys.argv[3]).encode()
if len(needle) != len(replacement):
    raise SystemExit("timestamp placeholder width mismatch")
with path.open("r+b") as handle:
    mm = mmap.mmap(handle.fileno(), 0)
    count = 0
    start = 0
    while True:
        index = mm.find(needle, start)
        if index < 0: break
        mm[index:index + len(needle)] = replacement
        count += 1
        start = index + len(needle)
    mm.flush()
    mm.close()
if count == 0:
    raise SystemExit(f"no timestamp placeholder found in {path}")
print(f"Patched {count} embedded version string(s) in {path}")
PY
}

install_binary() {
  local binary="${1}" version="${2}" short
  [[ -x "${binary}" ]] || die "built binary not found: ${binary}"
  mkdir -p "${INSTALL_BIN_DIR}"
  short="$(git -C "${SOURCE_REPO}" rev-parse --short=12 HEAD)"
  patch_timestamp "${binary}" "${version}" "${short}"
  local name="codex-${version}-${short}-${TIMESTAMP}"
  install -m 0755 "${binary}" "${INSTALL_BIN_DIR}/${name}"
  ln -sfn "${name}" "${INSTALL_BIN_DIR}/codex"
  echo "Installed ${INSTALL_BIN_DIR}/${name}"
  echo "Linked ${INSTALL_BIN_DIR}/codex"
}

install_code_mode_host() {
  local binary="${1}"
  [[ -x "${binary}" ]] || die "built code-mode host not found: ${binary}"
  install -m 0755 "${binary}" "${INSTALL_BIN_DIR}/codex-code-mode-host"
  echo "Installed ${INSTALL_BIN_DIR}/codex-code-mode-host"
}

build_android() {
  local ndk="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
  [[ -d "${ndk}" ]] || die "set ANDROID_NDK_HOME to the Android NDK directory"
  local llvm="${ndk}/toolchains/llvm/prebuilt/linux-x86_64"
  [[ -x "${llvm}/bin/aarch64-linux-android29-clang" ]] || die "Android NDK Clang not found under ${llvm}"
  local builtins
  builtins="$(find "${llvm}" -name libclang_rt.builtins-aarch64-android.a -print -quit)"
  [[ -n "${builtins}" ]] || die "Android compiler builtins archive not found"
  export PATH="${llvm}/bin:${PATH}"
  export ANDROID_NDK_HOME="${ndk}" LIBLZMA_NO_PKG_CONFIG=1 BZIP2_NO_PKG_CONFIG=1 BZIP2_STATIC=1
  export PKG_CONFIG_ALLOW_CROSS=1 CODEX_SKIP_VENDORED_BWRAP=1
  export CC_aarch64_linux_android="${llvm}/bin/aarch64-linux-android29-clang"
  export CXX_aarch64_linux_android="${llvm}/bin/aarch64-linux-android29-clang++"
  export AR_aarch64_linux_android="${llvm}/bin/llvm-ar"
  export RANLIB_aarch64_linux_android="${llvm}/bin/llvm-ranlib"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${llvm}/bin/aarch64-linux-android29-clang"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-Clink-arg=-lc++_shared -Clink-arg=-Wl,-rpath,\$ORIGIN -Clink-arg=${builtins}"
  local binary
  binary="$(cargo_build "${MODE}" android)"
  local stage="${BUILD_REPO}/android-artifact"
  mkdir -p "${stage}"
  install -m 0755 "${binary}" "${stage}/codex.bin"
  install -m 0644 "${llvm}/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so" "${stage}/libc++_shared.so"
  "${llvm}/bin/llvm-strip" --strip-all "${stage}/codex.bin"
  patch_timestamp "${stage}/codex.bin" "${VERSION}" "${COMMIT_SHORT}"
  echo "Android artifacts staged in ${stage}"
}

run_preflight() {
  (cd "${SOURCE_REPO}" && bash -n scripts/rebuild_codex.sh scripts/build.sh scripts/package-npm.sh)
  if [[ -f "${SOURCE_REPO}/scripts/publish_npm_local.sh" ]]; then
    bash -n "${SOURCE_REPO}/scripts/publish_npm_local.sh"
  fi
  require_cmd rustup
  require_cmd cargo
  require_cmd npm
  echo "Shell and required-tool preflight passed."
}

while (($#)); do
  case "${1}" in
    --debug) MODE=debug; shift ;;
    --release) MODE=release; shift ;;
    --target) TARGET_MODE="${2:-}"; shift 2 ;;
    --target=*) TARGET_MODE="${1#*=}"; shift ;;
    --armv7) TARGET_MODE=armv7; shift ;;
    --build-npm-vendor) TARGET_MODE=musl; PACKAGE_NPM=true; shift ;;
    --package-npm) PACKAGE_NPM=true; shift ;;
    --publish-npm) PUBLISH_NPM=true; PACKAGE_NPM=true; MODE=release; shift ;;
    --dry-run) DRY_RUN=true; shift ;;
    --publish) PUBLISH=true; MODE=release; shift ;;
    --preflight-only) PREFLIGHT_ONLY=true; shift ;;
    --no-sync) NO_SYNC=1; shift ;;
    --jobs) CARGO_BUILD_JOBS="${2:-}"; shift 2 ;;
    --jobs=*) CARGO_BUILD_JOBS="${1#*=}"; shift ;;
    --install-dir) INSTALL_BIN_DIR="${2:-}"; shift 2 ;;
    --install-dir=*) INSTALL_BIN_DIR="${1#*=}"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option ${1} (use --help)" ;;
  esac
done

require_cmd git
require_cmd python3
read_toolchain
if [[ -n "${CARGO_BUILD_JOBS:-}" ]]; then
  export CARGO_BUILD_JOBS
fi
sync_sources
if [[ "${PREFLIGHT_ONLY:-false}" == true ]]; then
  run_preflight
  exit 0
fi

VERSION="$(workspace_version)"
[[ -n "${VERSION}" ]] || die "could not determine workspace version"
COMMIT_SHORT="$(git -C "${SOURCE_REPO}" rev-parse --short=12 HEAD)"

if [[ "${TARGET_MODE}" == android ]]; then
  build_android
else
  binary="$(cargo_build "${MODE}" "${TARGET_MODE}")"
  if [[ "${TARGET_MODE}" == native ]]; then
    install_binary "${binary}" "${VERSION}"
    install_code_mode_host "$(dirname "${binary}")/codex-code-mode-host"
  else
    echo "Built target binary: ${binary}"
  fi
fi

if [[ "${PACKAGE_NPM:-false}" == true ]]; then
  if [[ "${TARGET_MODE}" != musl ]]; then
    cargo_build "${MODE}" musl >/dev/null
  fi
  cargo_build "${MODE}" armv7 >/dev/null
  "${SOURCE_REPO}/scripts/package-npm.sh" "${VERSION}" "${MODE}"
fi

if [[ "${PUBLISH_NPM}" == true ]]; then
  publish_args=(--version "${VERSION}" --publish)
  [[ "${DRY_RUN}" == true ]] && publish_args+=(--dry-run)
  "${SOURCE_REPO}/scripts/publish_npm_local.sh" "${publish_args[@]}"
fi

if [[ "${PUBLISH}" == true ]]; then
  require_cmd gh
  require_cmd jq
  tag="codex-v${VERSION}"
  if [[ "${DRY_RUN}" == true ]]; then
    echo "Would create and push GitHub tag ${tag}"
  else
    git -C "${SOURCE_REPO}" tag -a "${tag}" -m "Release ${VERSION}"
    git -C "${SOURCE_REPO}" push origin "${tag}"
    run_id="$(gh run list --workflow custom-codex-release.yml --limit 20 --json databaseId,headBranch \
      | jq -r --arg tag "${tag}" '.[] | select(.headBranch == $tag) | .databaseId' | head -n 1)"
    if [[ -n "${run_id}" ]]; then
      gh run watch "${run_id}" --exit-status
      gh release view "${tag}" >/dev/null
    else
      echo "Tag pushed; custom-codex-release workflow has not appeared yet. Check GitHub Actions for ${tag}."
    fi
  fi
fi
