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
PACKAGE_VERSION=""
MODE="debug"
TARGET_MODE="native"
PUBLISH="false"
PACKAGE_NPM="false"
PREFLIGHT_ONLY="false"
DRY_RUN="false"
SYNCED="false"
RUSTY_V8_ARMV7_PREPARED="false"
RUSTY_V8_SOURCE_PREPARED="false"
RUSTY_V8_BUILD_REPO=""
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
  --target <names>         native, musl, armv7, android, all, or comma-separated targets
  --armv7                  Alias for --target armv7
  --build-npm-vendor       Build the Linux musl payload for npm packaging
  --package-npm            Build local @reb.ai/codex npm archives
  --package-version V      Override only the npm package release version
  --dry-run                Use supported dry-run checks
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
  SYNCED=true
}

prepare_armv7_rusty_v8_source() {
  [[ "${RUSTY_V8_ARMV7_PREPARED}" == true ]] && return
  local source_repo="${RUSTY_V8_REPO_DIR:-${SOURCE_REPO%/codex}/rusty_v8}"
  local build_repo=""
  for candidate in "${source_repo}.build" "${source_repo}.make"; do
    if [[ -d "${candidate}" ]]; then
      build_repo="${candidate}"
      break
    fi
  done
  [[ -n "${build_repo}" ]] || die "Rusty V8 build tree not found beside ${source_repo}"
  RUSTY_V8_BUILD_REPO="${build_repo}"
  if [[ "${NO_SYNC:-0}" != 1 ]]; then
    if command -v cpto >/dev/null 2>&1; then
      local rust_toolchain="${build_repo}/third_party/rust-toolchain"
      local preserved_rust_toolchain=""
      if [[ -d "${rust_toolchain}" ]] \
        && ! [[ -f "${rust_toolchain}/lib/rustlib/src/rust/library/core/src/intrinsics/simd.rs" \
          && -f "${rust_toolchain}/lib/rustlib/src/rust/library/core/src/intrinsics/simd/mod.rs" ]]; then
        preserved_rust_toolchain="${build_repo}/.codex-preserved-rust-toolchain"
        [[ ! -e "${preserved_rust_toolchain}" ]] \
          || die "temporary Rusty V8 toolchain path already exists: ${preserved_rust_toolchain}"
        mv "${rust_toolchain}" "${preserved_rust_toolchain}"
      fi
      local source_rust_toolchain="${source_repo}/third_party/rust-toolchain"
      local staged_source_rust_toolchain="${source_repo}.cpto-rust-toolchain"
      if [[ -d "${source_rust_toolchain}" ]]; then
        [[ ! -e "${staged_source_rust_toolchain}" ]] \
          || die "temporary Rusty V8 source path already exists: ${staged_source_rust_toolchain}"
        mv "${source_rust_toolchain}" "${staged_source_rust_toolchain}"
      fi
      if ! cpto --no-lngit --nogit "${source_repo}" "${build_repo}"; then
        [[ ! -e "${source_rust_toolchain}" && -e "${staged_source_rust_toolchain}" ]] \
          && mv "${staged_source_rust_toolchain}" "${source_rust_toolchain}"
        [[ ! -e "${rust_toolchain}" && -e "${preserved_rust_toolchain}" ]] \
          && mv "${preserved_rust_toolchain}" "${rust_toolchain}"
        die "Rusty V8 source sync failed"
      fi
      if [[ -n "${staged_source_rust_toolchain}" && -e "${staged_source_rust_toolchain}" ]]; then
        mv "${staged_source_rust_toolchain}" "${source_rust_toolchain}"
      fi
      if [[ -n "${preserved_rust_toolchain}" ]]; then
        rm -rf "${rust_toolchain}"
        mv "${preserved_rust_toolchain}" "${rust_toolchain}"
      fi
    else
      echo "cpto not found; Rusty V8 source sync requires cpto" >&2
      die "missing required command: cpto"
    fi
  fi
  local rust_toolchain="${build_repo}/third_party/rust-toolchain"
  if [[ -f "${rust_toolchain}/lib/rustlib/src/rust/library/core/src/intrinsics/simd.rs" \
    && -f "${rust_toolchain}/lib/rustlib/src/rust/library/core/src/intrinsics/simd/mod.rs" ]]; then
    echo "Removing stale mixed Rusty V8 Rust toolchain from ${rust_toolchain}." >&2
    rm -rf "${rust_toolchain}"
  fi
  local manifest="${BUILD_WORKSPACE}/Cargo.toml"
  if ! grep -Fq "path = \"${build_repo}\"" "${manifest}"; then
    sed -i "/^\[patch\.crates-io\]$/a v8 = { path = \"${build_repo}\" }" "${manifest}"
  fi
  RUSTY_V8_ARMV7_PREPARED="true"
}

prepare_native_rusty_v8_source() {
  [[ "${RUSTY_V8_SOURCE_PREPARED}" == true ]] && return
  local source_repo="${RUSTY_V8_SOURCE_DIR:-${SOURCE_REPO%/codex}/rusty_v8}"
  [[ -f "${source_repo}/Cargo.toml" ]] || return
  local build_repo="${BUILD_REPO}/rusty-v8-native"
  if command -v cpto >/dev/null 2>&1; then
    mkdir -p "${build_repo}"
    cpto --no-lngit --nogit "${source_repo}" "${build_repo}"
  else
    die "cpto is required to prepare the native Rusty V8 source"
  fi
  rm -rf "${build_repo}/third_party/rust-toolchain"
  source_repo="${build_repo}"
  local manifest="${BUILD_WORKSPACE}/Cargo.toml"
  if ! grep -Fq "path = \"${source_repo}\"" "${manifest}"; then
    sed -i "/^\[patch\.crates-io\]$/a v8 = { path = \"${source_repo}\" }" "${manifest}"
  fi
  RUSTY_V8_SOURCE_PREPARED="true"
}

refresh_build_lockfile() {
  local source_lock="${SOURCE_REPO}/codex-rs/Cargo.lock"
  local build_lock="${BUILD_WORKSPACE}/Cargo.lock"
  local fingerprint_file="${BUILD_WORKSPACE}/.codex-source-lock-fingerprint"
  local source_fingerprint stored_fingerprint=""
  source_fingerprint="$(sed '/^version = /d' "${source_lock}" | sha256sum | awk '{print $1}')"
  [[ -f "${fingerprint_file}" ]] && read -r stored_fingerprint <"${fingerprint_file}"
  echo "Refreshing generated build-tree Cargo.lock..." >&2
  if [[ ! -f "${build_lock}" || "${source_fingerprint}" != "${stored_fingerprint}" ]]; then
    cp "${source_lock}" "${build_lock}"
    printf '%s\n' "${source_fingerprint}" >"${fingerprint_file}"
  else
    echo "Keeping generated build-tree Cargo.lock." >&2
  fi
  # Keep the Starlark pagable override in every build profile. Removing it for
  # native builds leaves the generated tree on crates.io pagable, while the
  # ARMv7/npm path needs the patched Git revision and its unified Dupe graph.
  if grep -Fq 'pagable = { git =' "${BUILD_WORKSPACE}/Cargo.toml"; then
    (cd "${BUILD_WORKSPACE}" && cargo +"${TOOLCHAIN}" update -p pagable)
  fi
  if [[ "${RUSTY_V8_ARMV7_PREPARED}" == true ]]; then
    (cd "${BUILD_WORKSPACE}" && cargo +"${TOOLCHAIN}" update -p v8 --offline)
  fi
  if [[ "${RUSTY_V8_SOURCE_PREPARED}" == true ]]; then
    (cd "${BUILD_WORKSPACE}" && cargo +"${TOOLCHAIN}" update -p v8 --offline)
  else
    sed -i "\|v8 = { path = \"${BUILD_REPO}/rusty-v8-native\" }|d" "${BUILD_WORKSPACE}/Cargo.toml"
  fi
}

workspace_version() {
  sed -n 's/^version = "\([^"]*\)"/\1/p' "${SOURCE_REPO}/codex-rs/Cargo.toml" | head -n 1
}

configure_rusty_v8_artifacts() {
  local target_mode="${1}" target archive binding local_repo cache_dir release_tag base_url release_repo
  case "${target_mode}" in
    native)
      target="x86_64-unknown-linux-gnu"
      ;;
    musl)
      target="x86_64-unknown-linux-musl"
      ;;
    armv7)
      target="${ARMV7_TARGET:-armv7-unknown-linux-gnueabihf}"
      ;;
    *)
      return
      ;;
  esac

  local crate_version="${V8_CRATE_VERSION:-$(sed -n '/^name = "v8"$/,/^version = /s/^version = "\([^"]*\)"/\1/p' "${SOURCE_REPO}/codex-rs/Cargo.lock" | head -n 1)}"
  [[ -n "${crate_version}" ]] || die "could not determine the pinned v8 crate version"
  local default_profile="ptrcomp_sandbox_release"
  [[ "${target_mode}" == armv7 ]] && default_profile="release"
  local profile="${RUSTY_V8_PROFILE:-${default_profile}}"
  archive="librusty_v8_${profile}_${target}.a.gz"
  binding="src_binding_${profile}_${target}.rs"

  if [[ -n "${RUSTY_V8_ARCHIVE:-}" || -n "${RUSTY_V8_SRC_BINDING_PATH:-}" ]]; then
    [[ -s "${RUSTY_V8_ARCHIVE:-}" && -s "${RUSTY_V8_SRC_BINDING_PATH:-}" ]] \
      || die "RUSTY_V8_ARCHIVE and RUSTY_V8_SRC_BINDING_PATH must point to existing files"
    RUSTY_V8_ARCHIVE_PATH="${RUSTY_V8_ARCHIVE}"
    RUSTY_V8_BINDING_PATH="${RUSTY_V8_SRC_BINDING_PATH}"
    echo "Using Rusty V8 artifacts from the environment for ${target}." >&2
    return 0
  fi

  local target_dir build_profile build_root
  target_dir="$(cargo_target_dir "${MODE}" "${target_mode}")"
  build_profile="${MODE}"
  [[ "${build_profile}" == release ]] || build_profile="debug"
  build_root="${target_dir}/${build_profile}"
  if [[ "${target_mode}" != native ]]; then
    build_root="${target_dir}/${target}/${build_profile}"
  fi
  for output in "${build_root}"/build/v8-*/output; do
    [[ -f "${output}" ]] || continue
    local cached_archive cached_binding
    cached_archive="$(sed -n 's/^static lib URL: //p' "${output}" | tail -n 1)"
    cached_binding="$(sed -n 's/^cargo:rustc-env=RUSTY_V8_SRC_BINDING_PATH=//p' "${output}" | tail -n 1)"
    if [[ "$(basename "${cached_archive}")" == "${archive}" \
      && "$(basename "${cached_binding}")" == "${binding}" \
      && -s "${cached_archive}" && -s "${cached_binding}" ]]; then
      RUSTY_V8_ARCHIVE_PATH="${cached_archive}"
      RUSTY_V8_BINDING_PATH="${cached_binding}"
      echo "Using Rusty V8 artifacts recorded by Cargo for ${target}." >&2
      return 0
    fi
  done

  local_repo="${RUSTY_V8_REPO_DIR:-${SOURCE_REPO%/codex}/rusty_v8}"
  if [[ "${target_mode}" == native && -d "${BUILD_REPO}/rusty-v8-artifacts/native" && -z "${RUSTY_V8_REPO_DIR:-}" ]]; then
    local_repo="${BUILD_REPO}/rusty-v8-artifacts/native"
  fi
  cache_dir="${BUILD_REPO}/rusty-v8-artifacts/${VERSION}/${target}"
  mkdir -p "${cache_dir}"

  if [[ -f "${local_repo}/${archive}" && -f "${local_repo}/${binding}" ]]; then
    RUSTY_V8_ARCHIVE_PATH="${local_repo}/${archive}"
    RUSTY_V8_BINDING_PATH="${local_repo}/${binding}"
    echo "Using local Rusty V8 artifacts from ${local_repo} for ${target}." >&2
  else
    require_cmd curl
    release_tag="${RUSTY_V8_RELEASE_TAG:-rusty-v8-v${crate_version}}"
    RUSTY_V8_ARCHIVE_PATH="${cache_dir}/${archive}"
    RUSTY_V8_BINDING_PATH="${cache_dir}/${binding}"
    if [[ -s "${RUSTY_V8_ARCHIVE_PATH}" && -s "${RUSTY_V8_BINDING_PATH}" ]]; then
      echo "Using cached Rusty V8 ${release_tag} artifacts for ${target}." >&2
    else
      local release_repos=("${RUSTY_V8_RELEASE_REPO:-rebroad/codex}")
      [[ -n "${RUSTY_V8_RELEASE_REPO:-}" ]] || release_repos+=("openai/codex")
      for release_repo in "${release_repos[@]}"; do
        base_url="https://github.com/${release_repo}/releases/download/${release_tag}"
        echo "Downloading Rusty V8 ${release_tag} artifacts for ${target} from ${release_repo}." >&2
        if curl --fail --location --retry 3 --silent --show-error \
          "${base_url}/${archive}" --output "${RUSTY_V8_ARCHIVE_PATH}" \
          && curl --fail --location --retry 3 --silent --show-error \
            "${base_url}/${binding}" --output "${RUSTY_V8_BINDING_PATH}"; then
          break
        fi
        rm -f "${RUSTY_V8_ARCHIVE_PATH}" "${RUSTY_V8_BINDING_PATH}"
      done
    fi
  fi

  [[ -s "${RUSTY_V8_ARCHIVE_PATH}" && -s "${RUSTY_V8_BINDING_PATH}" ]]
}

read_toolchain() {
  TOOLCHAIN="${RUST_TOOLCHAIN:-}"
  if [[ -z "${TOOLCHAIN}" && -f "${BUILD_WORKSPACE}/rust-toolchain.toml" ]]; then
    TOOLCHAIN="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "${BUILD_WORKSPACE}/rust-toolchain.toml" | head -n 1)"
  fi
  TOOLCHAIN="${TOOLCHAIN:-stable}"
}

cargo_target_dir() {
  local mode="${1}" target_mode="${2}"
  if [[ "${mode}" == debug && "${target_mode}" == native ]]; then
    if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
      echo "${CARGO_TARGET_DIR}"
    else
      (cd "${BUILD_WORKSPACE}" && cargo metadata --no-deps --format-version 1 \
        | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
    fi
    return
  fi
  case "${target_mode}" in
    native) echo "${BUILD_REPO}/build/linux-${mode}" ;;
    musl) echo "${BUILD_REPO}/build/musl-${mode}" ;;
    armv7) echo "${BUILD_REPO}/build/armv7-${mode}" ;;
    android) echo "${BUILD_REPO}/build/android-${mode}" ;;
    *) die "unknown target mode: ${target_mode}" ;;
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

configure_musl_build_tools() {
  local target="${1}" env_file="${MUSL_TOOL_ENV_FILE:-${RUNNER_TEMP:-/var/tmp}/codex-musl-env-${1}}"
  local tool_root="${RUNNER_TEMP:-/var/tmp}/codex-musl-tools-${target}"
  local libcap_archive="${tool_root}/libcap-2.75/prefix/lib/libcap.a"
  local setup_script="${SOURCE_REPO}/.github/scripts/install-musl-build-tools.sh"

  if [[ ! -f "${libcap_archive}" || ! -f "${env_file}" ]]; then
    require_cmd sudo
    [[ -f "${setup_script}" ]] || die "musl build-tool setup script not found: ${setup_script}"
    echo "Preparing musl build tools and target-built libcap..." >&2
    TARGET="${target}" GITHUB_ENV="${env_file}" RUNNER_TEMP="${RUNNER_TEMP:-/var/tmp}" \
      bash "${setup_script}"
  fi

  while IFS= read -r assignment; do
    [[ -n "${assignment}" ]] && export "${assignment}"
  done <"${env_file}"
}

cargo_build() {
  local mode="${1}" target_mode="${2}" profile_args=() target triple target_dir
  [[ "${mode}" == release ]] && profile_args+=(--release)
  triple="$(target_triple "${target_mode}")"
  target_dir="$(cargo_target_dir "${mode}" "${target_mode}")"
  mkdir -p "${target_dir}"
  if [[ -n "${triple}" ]]; then
    target="${triple}"
    ensure_target "${triple}"
  else
    target=""
  fi

  local -a env_args=(CARGO_TARGET_DIR="${target_dir}" RUSTUP_DISABLE_SELF_UPDATE=1)
  [[ -n "${CARGO_BUILD_JOBS:-}" ]] && env_args+=(CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS}")
  if [[ "${target_mode}" == native ]]; then
    if [[ "${V8_FROM_SOURCE:-}" =~ ^(1|true|yes)$ || "${RUSTY_V8_SOURCE_PREPARED}" == true ]]; then
      env_args+=(V8_FROM_SOURCE=1)
    else
      if configure_rusty_v8_artifacts "${target_mode}"; then
        env_args+=(
          RUSTY_V8_ARCHIVE="${RUSTY_V8_ARCHIVE_PATH}"
          RUSTY_V8_SRC_BINDING_PATH="${RUSTY_V8_BINDING_PATH}"
        )
      else
        echo "Native Rusty V8 artifacts are unavailable; building V8 from source." >&2
        env_args+=(V8_FROM_SOURCE=1)
      fi
    fi
  elif [[ "${target_mode}" != android ]]; then
    configure_rusty_v8_artifacts "${target_mode}"
    env_args+=(
      RUSTY_V8_ARCHIVE="${RUSTY_V8_ARCHIVE_PATH}"
      RUSTY_V8_SRC_BINDING_PATH="${RUSTY_V8_BINDING_PATH}"
    )
  else
    # Upstream does not publish the sandboxed Android archive for every V8
    # release. Android must therefore use the patched Rusty V8 checkout that
    # was prepared above instead of falling back to the upstream downloader.
    env_args+=(V8_FROM_SOURCE=1)
  fi
  if [[ "${target_mode}" == armv7 ]]; then
    local armv7_cc="${ARMV7_LINKER:-arm-linux-gnueabihf-gcc}"
    local armv7_cxx="${ARMV7_CXX:-arm-linux-gnueabihf-g++}"
    local armv7_ar="${ARMV7_AR:-arm-linux-gnueabihf-ar}"
    local armv7_ranlib="${ARMV7_RANLIB:-arm-linux-gnueabihf-ranlib}"
    require_cmd "${armv7_cc}"
    require_cmd "${armv7_cxx}"
    require_cmd "${armv7_ar}"
    require_cmd "${armv7_ranlib}"
    # The musl tool setup exports TARGET_* for x86_64-musl. Those variables
    # take precedence over the target-specific CC_* values in some build
    # scripts, causing an x86 compiler to receive ARM flags.
    unset TARGET_CC TARGET_CXX TARGET_AR TARGET_RANLIB 2>/dev/null || true
    env_args+=(
      CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER="${armv7_cc}"
      CC_armv7_unknown_linux_gnueabihf="${armv7_cc}"
      CXX_armv7_unknown_linux_gnueabihf="${armv7_cxx}"
      AR_armv7_unknown_linux_gnueabihf="${armv7_ar}"
      RANLIB_armv7_unknown_linux_gnueabihf="${armv7_ranlib}"
    )
  fi
  if [[ "${target_mode}" == musl ]]; then
    configure_musl_build_tools "${triple}"
    local musl_cc="${MUSL_CC:-}"
    if [[ -z "${musl_cc}" ]]; then
      musl_cc="$(command -v musl-gcc || true)"
    fi
    [[ -n "${musl_cc}" ]] || die "musl-gcc is required for the musl target (install musl-tools or set MUSL_CC)"
    env_args+=(
      CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="${MUSL_LINKER:-${musl_cc}}"
      CC_x86_64_unknown_linux_musl="${musl_cc}"
      PKG_CONFIG_ALLOW_CROSS=1
      PKG_CONFIG_ALL_STATIC=1
    )
    if [[ -d /usr/include/x86_64-linux-gnu ]]; then
      env_args+=(CFLAGS_x86_64_unknown_linux_musl="-idirafter/usr/include -idirafter/usr/include/x86_64-linux-gnu")
    fi
  fi

  local -a cmd=(cargo +"${TOOLCHAIN}" build -p codex-cli -p codex-code-mode-host -p codex-rmcp-client)
  [[ "${target_mode}" == musl ]] && cmd+=(-p codex-bwrap)
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
  if [[ "${target_mode}" == native ]]; then
    local -a test_cmd=(cargo +"${TOOLCHAIN}" build -p codex-rmcp-client --bin test_stdio_server)
    test_cmd+=( "${profile_args[@]}" --locked )
    if ! (cd "${BUILD_WORKSPACE}" && env "${env_args[@]}" "${test_cmd[@]}") >&2; then
      echo "Locked test_stdio_server build failed; retrying offline in the build tree without --locked." >&2
      unset 'test_cmd[-1]'
      test_cmd+=(--offline)
      if ! (cd "${BUILD_WORKSPACE}" && env "${env_args[@]}" "${test_cmd[@]}") >&2; then
        return 1
      fi
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
import re
from pathlib import Path
path = Path(sys.argv[1])
version = sys.argv[2].encode()
replacement = (sys.argv[2] + "-" + sys.argv[3]).encode()
pattern = re.compile(re.escape(version) + rb"-[0-9a-f]{12}-[0-9]{12}")
if len(replacement) != len(version) + 1 + 12 + 1 + 12:
    raise SystemExit("timestamp placeholder width mismatch")
with path.open("r+b") as handle:
    mm = mmap.mmap(handle.fileno(), 0)
    count = 0
    for match in list(pattern.finditer(mm)):
        mm[match.start():match.end()] = replacement
        count += 1
    mm.flush()
    mm.close()
if count == 0:
    raise SystemExit(f"no version timestamp found in {path}")
print(f"Patched {count} embedded version string(s) in {path}")
PY
}

install_binary() {
  local binary="${1}" version="${2}" short
  [[ -x "${binary}" ]] || die "built binary not found: ${binary}"
  mkdir -p "${INSTALL_BIN_DIR}"
  short="$(git -C "${SOURCE_REPO}" rev-parse --short=12 HEAD)"
  local name="codex-${version}-${short}-${TIMESTAMP}"
  install -m 0755 "${binary}" "${INSTALL_BIN_DIR}/${name}"
  if ! patch_timestamp "${INSTALL_BIN_DIR}/${name}" "${version}" "${short}"; then
    rm -f "${INSTALL_BIN_DIR}/${name}"
    return 1
  fi
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

install_test_stdio_server() {
  local binary="${1}"
  [[ -x "${binary}" ]] || die "built test stdio server not found: ${binary}"
  install -m 0755 "${binary}" "${INSTALL_BIN_DIR}/test_stdio_server"
  echo "Installed ${INSTALL_BIN_DIR}/test_stdio_server"
}

build_android() {
  local ndk="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
  local user_home="${HOME:-$(getent passwd "$(id -u)" | cut -d: -f6)}"
  if [[ ! -d "${ndk}" ]]; then
    ndk=""
    local sdk_root
    for sdk_root in \
      "${ANDROID_HOME:-}" \
      "${ANDROID_SDK_ROOT:-}" \
      "${user_home}/Android/sdk" \
      "${user_home}/Android/Sdk"
    do
      if [[ -d "${sdk_root}/ndk" ]]; then
        ndk="$(find "${sdk_root}/ndk" -mindepth 1 -maxdepth 1 -type d -print | sort -V | tail -n 1)"
        [[ -n "${ndk}" ]] && break
      fi
    done
  fi
  [[ -d "${ndk}" ]] || die "Android NDK not found; set ANDROID_NDK_HOME or ANDROID_NDK_ROOT"
  export ANDROID_NDK_HOME="${ndk}" ANDROID_NDK_ROOT="${ndk}"
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
  local ndk_properties="${RUSTY_V8_BUILD_REPO}/third_party/android_ndk/source.properties"
  local rusty_v8_ndk_version
  rusty_v8_ndk_version="$(sed -n 's/^Pkg.Revision = //p' "${ndk_properties}" | head -n 1)"
  [[ -n "${rusty_v8_ndk_version}" ]] || die "Rusty V8 bundled Android NDK version not found in ${ndk_properties}"
  local gclient_args="${RUSTY_V8_BUILD_REPO}/build/config/gclient_args.gni"
  if ! grep -Fq 'android_ndk_version' "${gclient_args}"; then
    printf 'declare_args() {\n  android_ndk_version = "%s"\n}\n' \
      "${rusty_v8_ndk_version}" >"${gclient_args}"
  fi
  local binary
  binary="$(cargo_build "${MODE}" android)"
  local stage="${BUILD_REPO}/build/android-artifact"
  mkdir -p "${stage}"
  install -m 0755 "${binary}" "${stage}/codex.bin"
  install -m 0644 "${llvm}/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so" "${stage}/libc++_shared.so"
  "${llvm}/bin/llvm-strip" --strip-all "${stage}/codex.bin"
  patch_timestamp "${stage}/codex.bin" "${VERSION}" "${COMMIT_SHORT}"
  echo "Android artifacts staged in ${stage}"
}

run_preflight() {
  (cd "${SOURCE_REPO}" && bash -n scripts/rebuild_codex.sh scripts/build.sh scripts/package-npm.sh)
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
    --package-version) PACKAGE_VERSION="${2:-}"; shift 2 ;;
    --package-version=*) PACKAGE_VERSION="${1#*=}"; shift ;;
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

IFS=',' read -r -a REQUESTED_TARGETS <<<"${TARGET_MODE}"
[[ "${#REQUESTED_TARGETS[@]}" -gt 0 ]] || die "target must not be empty"
PACKAGE_TARGETS=()
for requested_target in "${REQUESTED_TARGETS[@]}"; do
  case "${requested_target}" in
    native)
      if [[ "${PACKAGE_NPM}" == true ]]; then
        # The upstream Linux npm payload is the portable musl build. Keep
        # native as the default host-build mode while making its npm meaning
        # explicit and target-scoped.
        PACKAGE_TARGETS+=(musl)
      else
        [[ "${#REQUESTED_TARGETS[@]}" -eq 1 ]] || die "multiple targets require --package-npm"
      fi
      ;;
    musl|armv7|android) PACKAGE_TARGETS+=("${requested_target}") ;;
    all)
      [[ "${PACKAGE_NPM}" == true ]] || die "target all requires --package-npm"
      PACKAGE_TARGETS+=(musl armv7 android)
      ;;
    *) die "unknown target: ${requested_target} (use native, musl, armv7, android, or all)" ;;
  esac
done

# Preserve order while removing duplicates from comma-separated selections.
UNIQUE_PACKAGE_TARGETS=()
for package_target in "${PACKAGE_TARGETS[@]}"; do
  already_selected=false
  for selected_target in "${UNIQUE_PACKAGE_TARGETS[@]}"; do
    [[ "${selected_target}" == "${package_target}" ]] && already_selected=true && break
  done
  [[ "${already_selected}" == true ]] || UNIQUE_PACKAGE_TARGETS+=("${package_target}")
done
PACKAGE_TARGETS=("${UNIQUE_PACKAGE_TARGETS[@]}")
if [[ "${PACKAGE_NPM}" == true && "${#PACKAGE_TARGETS[@]}" -eq 0 ]]; then
  die "npm packaging requires at least one target"
fi

if [[ "${PACKAGE_NPM}" == true || "${TARGET_MODE}" == musl ]]; then
  require_cmd sudo
  echo "Authenticating sudo before starting the build..." >&2
  sudo -v
fi

require_cmd git
require_cmd python3
read_toolchain
VERSION="$(workspace_version)"
[[ -n "${VERSION}" ]] || die "could not determine workspace version"
if [[ -z "${PACKAGE_VERSION}" && "${PACKAGE_NPM}" == true ]]; then
  PACKAGE_VERSION="$(${SOURCE_REPO}/scripts/npm_candidate_version.sh)"
else
  PACKAGE_VERSION="${PACKAGE_VERSION:-${VERSION}}"
fi
[[ "${PACKAGE_VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+[-+.0-9A-Za-z-]+$ ]] \
  || die "invalid npm package version: ${PACKAGE_VERSION}"
if [[ -n "${CARGO_BUILD_JOBS:-}" ]]; then
  export CARGO_BUILD_JOBS
fi
sync_sources
if [[ "${PACKAGE_NPM}" == true || "${TARGET_MODE}" == armv7 || "${TARGET_MODE}" == android ]]; then
  prepare_armv7_rusty_v8_source
elif [[ "${TARGET_MODE}" == native ]]; then
  if [[ "${V8_FROM_SOURCE:-}" =~ ^(1|true|yes)$ ]] || ! configure_rusty_v8_artifacts native; then
    prepare_native_rusty_v8_source || true
  fi
fi
refresh_build_lockfile
if [[ "${PREFLIGHT_ONLY:-false}" == true ]]; then
  run_preflight
  exit 0
fi

COMMIT_SHORT="$(git -C "${SOURCE_REPO}" rev-parse --short=12 HEAD)"

if [[ "${PACKAGE_NPM}" == true ]]; then
  echo "Building npm target(s): $(IFS=,; echo "${PACKAGE_TARGETS[*]}")" >&2
  for package_target in "${PACKAGE_TARGETS[@]}"; do
    if [[ "${package_target}" == android ]]; then
      build_android
    else
      cargo_build "${MODE}" "${package_target}" >/dev/null
    fi
  done
elif [[ "${TARGET_MODE}" == android ]]; then
  build_android
else
  binary="$(cargo_build "${MODE}" "${TARGET_MODE}")"
  if [[ "${TARGET_MODE}" == native ]]; then
    install_binary "${binary}" "${VERSION}"
    install_code_mode_host "$(dirname "${binary}")/codex-code-mode-host"
    install_test_stdio_server "$(dirname "${binary}")/test_stdio_server"
  else
    echo "Built target binary: ${binary}"
  fi
fi

if [[ "${PACKAGE_NPM:-false}" == true ]]; then
  package_target_csv="$(IFS=,; echo "${PACKAGE_TARGETS[*]}")"
  "${SOURCE_REPO}/scripts/package-npm.sh" "${PACKAGE_VERSION}" "${MODE}" "${package_target_csv}"
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
