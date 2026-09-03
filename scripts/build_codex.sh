#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
BUILD_TREE=""
SCRIPT_REPO="$(cd -- "${SCRIPT_DIR}/.." && pwd -P)"
case "${SCRIPT_REPO##*/}" in
  *.build|*.make)
    BUILD_TREE="${SCRIPT_REPO}"
    SOURCE_REPO="$(cd -- "${SCRIPT_REPO%.*}" && pwd -P)"
    ;;
  *)
    SOURCE_REPO="${SCRIPT_REPO}"
    for candidate in "${SOURCE_REPO}.build" "${SOURCE_REPO}.make"; do
      if [[ -d "${candidate}" ]]; then
        BUILD_TREE="$(cd -- "${candidate}" && pwd -P)"
        break
      fi
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
[[ "${BUILD_REPO}" != "${SOURCE_REPO}" ]] \
  || { echo "build_codex.sh: build tree must be a sibling of the source checkout, not the source checkout itself" >&2; exit 1; }
BUILD_WORKSPACE="${BUILD_REPO}/codex-rs"
INSTALL_BIN_DIR="${INSTALL_BIN_DIR:-${HOME}/.cargo/bin}"
VERSION=""
PACKAGE_VERSION=""
MODE="debug"
TARGET_MODE="native"
ARMV7_BUILD_ENV="${CODEX_ARMV7_BUILD_ENV:-host}"
PUBLISH="false"
SKIP_BUILD="false"
PACKAGE_NPM="false"
PUBLISH_NPM="false"
PREFLIGHT_ONLY="false"
DRY_RUN="false"
INSTALL_TARGETS=""
SYNCED="false"
RUSTY_V8_ARMV7_PREPARED="false"
RUSTY_V8_BUILD_REPO=""
LOCKFILE_REGENERATION_REQUIRED="false"
ALLOW_CONCURRENT_BUILD="false"
TIMESTAMP="$(date -u +%Y%m%d%H%M)"
COMMIT_SHORT=""
BUILD_TIMESTAMP_SEPARATOR="-"
TOOLCHAIN=""
CARGO_CMD=(cargo)
RUSTC_CMD=(rustc)
FORK_RELEASE_REPO="${CODEX_FORK_RELEASE_REPO:-rebroad/codex}"
SUDO_AUTHENTICATED="false"
SSH_OPTS=(-o ConnectTimeout="${CODEX_SSH_CONNECT_TIMEOUT:-10}" -o ServerAliveInterval=5 -o ServerAliveCountMax=2)

# Rebuild jobs have their own purpose link so its timestamp identifies the
# last rebuild invocation, even when Cargo reuses the same shared target.
export CODEX_CARGO_PURPOSE="${CODEX_CARGO_PURPOSE:-rebuild}"

# Keep the native OpenSSL cache implementation shared with the Just recipes.
# The script is deliberately sourced from the source checkout before cpto
# synchronizes it into the build tree.
source "${SOURCE_REPO}/scripts/openssl_artifacts.sh"

usage() {
  cat <<'EOF'
Usage: scripts/build_codex.sh [options]

Builds from the source checkout into the first existing sibling build tree
(<repo>.build or <repo>.make), retaining Cargo's incremental cache.

Options:
  --debug                  Build debug (default)
  --release                Build optimized release
  --target <names>         native, musl, armv7, android, all, or comma-separated targets
                           (all means the complete eight-architecture npm candidate)
  --armv7                  Alias for --target armv7
  --armv7-build-env MODE   ARMv7 environment: host (default); ARMv7 builds are musl
  --build-npm-vendor       Build the Linux musl payload for npm packaging
  --package-local-npm      Build/reuse local @reb.ai/codex npm archives
  --publish-local-npm      Assemble, audit, and publish npm locally
  --start-github-release   Push a release tag and start GitHub CI; print URLs
  --skip-build             With --start-github-release, package a completed CI build
  --package-npm            Alias for --package-local-npm
  --publish-npm            Alias for --publish-local-npm
  --publish                Alias for --start-github-release
  --package-version V      Override only the npm package release version
  --dry-run                Use supported dry-run checks
  --preflight-only         Run syntax/tooling checks without compiling
  --no-sync                Reuse the already-synced sibling source tree
  --allow-concurrent-build  Bypass the shared Cargo target lock and share the target directory
  --jobs N                 Set CARGO_BUILD_JOBS
  --install TARGETS        Build and install to comma-separated SSH targets
  -h, --help               Show this help
EOF
}

die() { echo "build_codex.sh: $*" >&2; exit 1; }
require_cmd() { command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"; }

BUILD_PROCESS_REGISTRY="${BUILD_REPO}/build/codex-build-processes"
BUILD_PROCESS_RECORD="${BUILD_PROCESS_REGISTRY}/$$"
register_build_process() {
  mkdir -p "${BUILD_PROCESS_REGISTRY}"
  {
    printf 'pid=%s\n' "$$"
    printf 'started=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'command=%s\n' "$(ps -p $$ -o args= 2>/dev/null || printf '%s' "${BASH_SOURCE[0]}")"
  } >"${BUILD_PROCESS_RECORD}"
}
cleanup_build_process() {
  rm -f "${BUILD_PROCESS_RECORD}"
}

authenticate_sudo() {
  [[ "${SUDO_AUTHENTICATED}" == true ]] && return
  require_cmd sudo
  echo "Authenticating sudo before installing required build tools..." >&2
  sudo -v
  SUDO_AUTHENTICATED="true"
}

set_v8_path_patch() {
  local manifest="${1}" build_repo="${2}"
  sed -i '/^v8 = { path = /d' "${manifest}"
  sed -i "/^\[patch\.crates-io\]$/a v8 = { path = \"${build_repo}\" }" "${manifest}"
}

download_latest_fork_npm_release() {
  local output_dir="${BUILD_REPO}/build/npm-artifact"
  local tag
  if ! command -v gh >/dev/null 2>&1; then
    echo "gh is unavailable; using local npm artifacts." >&2
    return 0
  fi
  mkdir -p "${output_dir}"
  tag="$(gh release list --repo "${FORK_RELEASE_REPO}" --limit 50 \
    --json tagName,publishedAt \
    --jq '[.[] | select(.tagName | startswith("codex-npm-v"))] | sort_by(.publishedAt) | last.tagName' \
    2>/dev/null || true)"
  if [[ -z "${tag}" ]]; then
    echo "No completed fork npm release is available from ${FORK_RELEASE_REPO}; using local artifacts." >&2
    return 0
  fi
  echo "Downloading latest fork npm release ${FORK_RELEASE_REPO}:${tag}..." >&2
  if ! gh release download "${tag}" --repo "${FORK_RELEASE_REPO}" \
    --pattern 'codex-npm-*.tgz' --dir "${output_dir}" --clobber; then
    echo "Fork npm release download failed; using local artifacts." >&2
  else
    echo "Fork npm release: https://github.com/${FORK_RELEASE_REPO}/releases/tag/${tag}" >&2
  fi
}

platform_for_package_target() {
  case "${1}" in
    musl) echo linux-x64 ;;
    armv7) echo linux-armv7 ;;
    android) echo android-arm64 ;;
  esac
}

has_local_npm_platform_archive() {
  local target="${1}" platform archive
  platform="$(platform_for_package_target "${target}")"
  shopt -s nullglob
  for archive in "${BUILD_REPO}/build/npm-artifact/codex-npm-${platform}-"*.tgz; do
    if tar -tzf "${archive}" package/package.json >/dev/null 2>&1; then
      return 0
    fi
  done
  return 1
}

find_reusable_github_run() {
  local tag="${1}" run_id
  while read -r run_id; do
    [[ -n "${run_id}" ]] || continue
    echo "${run_id}"
    return 0
  done < <(gh run list --repo "${FORK_RELEASE_REPO}" \
    --workflow rust-release.yml --status completed --limit 50 \
    --json databaseId,headBranch,event,conclusion \
    | jq -r --arg tag "${tag}" '.[] | select(.event == "push" and .headBranch == $tag and .conclusion != "cancelled") | .databaseId')
  return 1
}

show_github_run() {
  local run_id="${1}" run_url="${2}" job_info
  echo "GitHub CI run: ${run_url}" >&2
  job_info="$(gh run view "${run_id}" --repo "${FORK_RELEASE_REPO}" \
    --json jobs --jq '.jobs[] | [.databaseId, .name, .url] | @tsv' 2>/dev/null || true)"
  while IFS=$'\t' read -r job_id job_name job_url; do
    [[ -n "${job_id}" ]] || continue
    [[ -n "${job_url}" && "${job_url}" != null ]] \
      || job_url="https://github.com/${FORK_RELEASE_REPO}/actions/runs/${run_id}/job/${job_id}"
    echo "GitHub CI job (${job_name}): ${job_url}" >&2
  done <<<"${job_info}"
  echo "Watch command: gh run watch ${run_id} --repo ${FORK_RELEASE_REPO} --exit-status" >&2
}

offer_cancel_superseded_runs() {
  local active_runs answer
  active_runs="$(gh run list --repo "${FORK_RELEASE_REPO}" \
    --workflow rust-release.yml --limit 50 \
    --json databaseId,headBranch,status,url \
    | jq -r '.[] | select(.status != "completed") | [.databaseId, .headBranch, .url] | @tsv')"
  [[ -n "${active_runs}" ]] || return 0

  echo "Active rust-release runs that would be superseded:" >&2
  while IFS=$'\t' read -r run_id run_branch run_url; do
    printf '  %s  %s  %s\n' "${run_id}" "${run_branch}" "${run_url}" >&2
  done <<<"${active_runs}"
  if [[ ! -t 0 || ! -t 1 ]]; then
    echo "Not cancelling active runs because this release is non-interactive." >&2
    return 0
  fi
  read -r -p "Cancel these superseded runs before releasing? [y/N] " answer
  if [[ "${answer}" =~ ^[Yy]$ ]]; then
    while IFS=$'\t' read -r run_id _ _; do
      gh run cancel "${run_id}" --repo "${FORK_RELEASE_REPO}"
    done <<<"${active_runs}"
  fi
}

start_github_release() {
  require_cmd gh
  require_cmd jq
  local release_version tag run_info run_id run_url
  if [[ "${VERSION}" == *-alpha* ]]; then
    release_version="$(${SOURCE_REPO}/scripts/npm_candidate_version.sh)"
  else
    release_version="${VERSION}"
  fi
  tag="rust-v${release_version}"
  echo "GitHub Actions workflow: https://github.com/${FORK_RELEASE_REPO}/actions/workflows/rust-release.yml" >&2
  if [[ "${SKIP_BUILD}" == true ]]; then
    if [[ "${DRY_RUN}" == true ]]; then
      echo "Would dispatch packaging-only release for the latest completed rust-release run for ${tag}." >&2
      return 0
    fi
    git -C "${SOURCE_REPO}" ls-remote --exit-code origin "refs/tags/${tag}" >/dev/null \
      || die "release tag ${tag} does not exist; --skip-build can only retry an existing tagged build"
    run_id="$(find_reusable_github_run "${tag}")" \
      || die "no successful rust-release run found for ${tag}"
    source_ref="$(git -C "${SOURCE_REPO}" symbolic-ref --short HEAD)"
    gh workflow run rust-release.yml --repo "${FORK_RELEASE_REPO}" \
      --ref "${source_ref}" \
      -f "source_run_id=${run_id}" \
      -f "source_tag=${tag}" \
      -f "source_ref=${source_ref}"
    echo "Started packaging-only release from source run ${run_id}." >&2
    echo "Source run: https://github.com/${FORK_RELEASE_REPO}/actions/runs/${run_id}" >&2
    echo "Waiting for packaging workflow to appear..." >&2
    run_info=""
    for _ in {1..30}; do
      run_info="$(gh run list --repo "${FORK_RELEASE_REPO}" \
        --workflow rust-release.yml --event workflow_dispatch --limit 20 \
        --json databaseId,headBranch,url \
        | jq -r --arg branch "${source_ref}" '[.[] | select(.headBranch == $branch)] | .[0] | [.databaseId, .url] | @tsv')"
      read -r run_id run_url <<<"${run_info}"
      [[ -n "${run_id}" ]] && break
      sleep 2
    done
    if [[ -n "${run_id}" ]]; then
      show_github_run "${run_id}" "${run_url}"
    else
      echo "Packaging workflow dispatched; open the Actions page above." >&2
    fi
    return 0
  fi
  if [[ "${DRY_RUN}" == true ]]; then
    echo "Would create and push GitHub tag ${tag}" >&2
    return 0
  fi
  offer_cancel_superseded_runs
  git -C "${SOURCE_REPO}" tag -a "${tag}" -m "Release ${VERSION}"
  git -C "${SOURCE_REPO}" push origin "${tag}"
  echo "Waiting for GitHub to create the workflow run..." >&2
  run_info=""
  for _ in {1..30}; do
    run_info="$(gh run list --repo "${FORK_RELEASE_REPO}" \
      --workflow rust-release.yml --limit 20 --json databaseId,headBranch,url \
      | jq -r --arg tag "${tag}" '[.[] | select(.headBranch == $tag)] | .[0] | [.databaseId, .url] | @tsv')"
    read -r run_id run_url <<<"${run_info}"
    [[ -n "${run_id}" ]] && break
    sleep 2
  done
  if [[ -n "${run_id}" ]]; then
    show_github_run "${run_id}" "${run_url}"
  else
    echo "Tag pushed; the workflow has not appeared yet. Open the workflow URL above." >&2
  fi
}

sync_sources() {
  [[ "${SYNCED}" == true ]] && return
  if [[ "${NO_SYNC:-0}" == 1 ]]; then SYNCED=true; return; fi
  if command -v cpto >/dev/null 2>&1; then
    cpto "${SOURCE_REPO}" "${BUILD_REPO}"
  else
    echo "cpto not found; syncing source without deleting build artifacts" >&2
    tar --exclude='./.git' -cf - -C "${SOURCE_REPO}" . | tar -xf - -C "${BUILD_REPO}"
  fi
  SYNCED=true
}

prepare_armv7_rusty_v8_source() {
  [[ "${RUSTY_V8_ARMV7_PREPARED}" == true ]] && return
  local target_mode="${1:-armv7}"
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
      if ! cpto "${source_repo}" "${build_repo}"; then
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
      echo "cpto not found; syncing Rusty V8 source without deleting build artifacts" >&2
      tar --exclude='./.git' -cf - -C "${source_repo}" . | tar -xf - -C "${build_repo}"
    fi
  fi
  local rust_toolchain="${build_repo}/third_party/rust-toolchain"
  if [[ -f "${rust_toolchain}/lib/rustlib/src/rust/library/core/src/intrinsics/simd.rs" \
    && -f "${rust_toolchain}/lib/rustlib/src/rust/library/core/src/intrinsics/simd/mod.rs" ]]; then
    echo "Removing stale mixed Rusty V8 Rust toolchain from ${rust_toolchain}." >&2
    rm -rf "${rust_toolchain}"
  fi
  local manifest="${BUILD_WORKSPACE}/Cargo.toml"
  if [[ "${target_mode}" != armv7 ]]; then
    set_v8_path_patch "${manifest}" "${build_repo}"
  fi
  RUSTY_V8_ARMV7_PREPARED="true"
}

refresh_build_lockfile() {
  local source_lock="${SOURCE_REPO}/codex-rs/Cargo.lock" target_dir lock_target_mode
  local build_lock="${BUILD_WORKSPACE}/Cargo.lock"
  local fingerprint_file="${BUILD_REPO}/build/.codex-source-lock-fingerprint"
  # A comma-separated npm target selection has no single Cargo target
  # directory. Use the first selected package target for metadata/lockfile
  # work; each actual build still selects its own target directory below.
  lock_target_mode="${TARGET_MODE}"
  if [[ "${PACKAGE_NPM}" == true && "${#PACKAGE_TARGETS[@]}" -gt 0 ]]; then
    lock_target_mode="${PACKAGE_TARGETS[0]}"
  fi
  target_dir="$(target_dir_for "${MODE}" "${lock_target_mode}")"
  local source_fingerprint stored_fingerprint=""
  local armv7_lock_cache="${target_dir}/cache/cargo-resolution-lock"
  source_fingerprint="$(sed '/^version = /d' "${source_lock}" | sha256sum | awk '{print $1}')"
  [[ -f "${fingerprint_file}" ]] && read -r stored_fingerprint <"${fingerprint_file}"
  echo "Refreshing generated build-tree Cargo.lock..." >&2
  if [[ "${lock_target_mode}" == armv7 && "${source_fingerprint}" == "${stored_fingerprint}" \
    && -f "${armv7_lock_cache}" ]]; then
    cp "${armv7_lock_cache}" "${build_lock}"
    echo "Restored cached ARMv7 Cargo resolution." >&2
  elif [[ ! -f "${build_lock}" || "${source_fingerprint}" != "${stored_fingerprint}" ]]; then
    cp "${source_lock}" "${build_lock}"
    mkdir -p "$(dirname "${fingerprint_file}")"
    printf '%s\n' "${source_fingerprint}" >"${fingerprint_file}"
  else
    echo "Keeping generated build-tree Cargo.lock." >&2
  fi
  if [[ "${RUSTY_V8_ARMV7_PREPARED}" == true ]]; then
    local lock_update_fingerprint_file="${BUILD_REPO}/build/.codex-armv7-lock-update-fingerprint"
    local lock_update_fingerprint stored_lock_update_fingerprint=""
    lock_update_fingerprint="$({
      sha256sum "${BUILD_WORKSPACE}/Cargo.toml" "${build_lock}"
    } | sha256sum | awk '{print $1}')"
    [[ -f "${lock_update_fingerprint_file}" ]] \
      && read -r stored_lock_update_fingerprint <"${lock_update_fingerprint_file}"
    if [[ "${lock_update_fingerprint}" != "${stored_lock_update_fingerprint}" ]]; then
      (cd "${BUILD_WORKSPACE}" && env CARGO_TARGET_DIR="${target_dir}" "${CARGO_CMD[@]}" update -p v8 --offline)
      lock_update_fingerprint="$({
        sha256sum "${BUILD_WORKSPACE}/Cargo.toml" "${build_lock}"
      } | sha256sum | awk '{print $1}')"
      printf '%s\n' "${lock_update_fingerprint}" >"${lock_update_fingerprint_file}"
    else
      echo "Keeping cached ARMv7 V8 Cargo resolution." >&2
    fi
  fi
  echo "Cargo will try locked online dependency resolution before an offline retry." >&2
}

workspace_version() {
  sed -n 's/^version = "\([^"]*\)"/\1/p' "${SOURCE_REPO}/codex-rs/Cargo.toml" | head -n 1
}

verify_rusty_v8_artifacts() {
  local target_mode="${1}" expected_archive expected_binding actual
  expected_archive="${RUSTY_V8_ARCHIVE_SHA256:-}"
  expected_binding="${RUSTY_V8_BINDING_SHA256:-}"
  case "${target_mode}" in
    musl)
      expected_archive="${RUSTY_V8_MUSL_ARCHIVE_SHA256:-${expected_archive}}"
      expected_binding="${RUSTY_V8_MUSL_BINDING_SHA256:-${expected_binding}}"
      ;;
    armv7)
      expected_archive="${RUSTY_V8_ARMV7_ARCHIVE_SHA256:-${expected_archive}}"
      expected_binding="${RUSTY_V8_ARMV7_BINDING_SHA256:-${expected_binding}}"
      ;;
    android)
      expected_archive="${RUSTY_V8_ANDROID_ARCHIVE_SHA256:-${expected_archive}}"
      expected_binding="${RUSTY_V8_ANDROID_BINDING_SHA256:-${expected_binding}}"
      ;;
  esac
  if [[ -n "${expected_archive}" ]]; then
    actual="$(sha256sum "${RUSTY_V8_ARCHIVE_PATH}" | awk '{print $1}')"
    [[ "${actual}" == "${expected_archive}" ]] || die "Rusty V8 archive checksum mismatch for ${target_mode}: expected ${expected_archive}, got ${actual}"
  fi
  if [[ -n "${expected_binding}" ]]; then
    actual="$(sha256sum "${RUSTY_V8_BINDING_PATH}" | awk '{print $1}')"
    [[ "${actual}" == "${expected_binding}" ]] || die "Rusty V8 binding checksum mismatch for ${target_mode}: expected ${expected_binding}, got ${actual}"
  fi
}

configure_openssl_artifacts() {
  local target_mode="${1}" target version cache_dir
  case "${target_mode}" in
    native) target="$(${RUSTC_CMD[@]} -vV | sed -n 's/^host: //p')" ;;
    musl) target="x86_64-unknown-linux-musl" ;;
    armv7) target="${ARMV7_TARGET:-armv7-unknown-linux-musleabihf}" ;;
    android) target="aarch64-linux-android" ;;
    *) return 1 ;;
  esac
  version="$(openssl_version_from_lock "${BUILD_WORKSPACE}/Cargo.lock")"
  [[ -n "${version}" ]] || return 1
  cache_dir="$(openssl_cache_dir "${BUILD_REPO}" "${target}" "${version}")"
  if openssl_cache_is_valid "${cache_dir}"; then
    OPENSSL_DIR_PATH="${cache_dir}"
    echo "Using cached OpenSSL ${version} for ${target}." >&2
    return 0
  fi
  return 1
}

cache_openssl_artifacts() {
  local target_mode="${1}" target version host
  case "${target_mode}" in
    native) target="$(${RUSTC_CMD[@]} -vV | sed -n 's/^host: //p')" ;;
    musl) target="x86_64-unknown-linux-musl" ;;
    armv7) target="${ARMV7_TARGET:-armv7-unknown-linux-musleabihf}" ;;
    android) target="aarch64-linux-android" ;;
    *) return 0 ;;
  esac
  version="$(openssl_version_from_lock "${BUILD_WORKSPACE}/Cargo.lock")"
  host="$(${RUSTC_CMD[@]} -vV | sed -n 's/^host: //p')"
  openssl_cache_from_target "$(target_dir_for "${MODE}" "${target_mode}")" \
    "${BUILD_REPO}" "${target}" "${version}" "${host}" || true
}

configure_rusty_v8_artifacts() {
  local target_mode="${1}" target archive binding local_repo cache_dir release_tag
  local -a resolver_args
  case "${target_mode}" in
    native)
      target="$("${RUSTC_CMD[@]}" -vV | sed -n 's/^host: //p')"
      [[ -n "${target}" ]] || die "could not determine the Rust host target"
      ;;
    musl)
      target="x86_64-unknown-linux-musl"
      ;;
    armv7)
      target="${ARMV7_TARGET:-armv7-unknown-linux-musleabihf}"
      ;;
    android)
      target="aarch64-linux-android"
      ;;
    *)
      return
      ;;
  esac

  local crate_version="${V8_CRATE_VERSION:-$(python3 "${SOURCE_REPO}/scripts/rusty_v8_version.py" "${SOURCE_REPO}/codex-rs/Cargo.lock")}";
  [[ -n "${crate_version}" ]] || die "could not determine the pinned v8 crate version"
  local default_profile="ptrcomp_sandbox_release"
  local profile="${RUSTY_V8_PROFILE:-${default_profile}}"
  local target_dir build_profile build_root
  target_dir="$(target_dir_for "${MODE}" "${target_mode}")"
  build_profile="${MODE}"
  [[ "${build_profile}" == release ]] || build_profile="debug"
  build_root="${target_dir}/${build_profile}"
  if [[ "${target_mode}" != native ]]; then
    build_root="${target_dir}/${target}/${build_profile}"
  fi
  local_repo=""
  if [[ "${target_mode}" == armv7 || "${target_mode}" == android || "${target_mode}" == musl ]]; then
    local_repo="${RUSTY_V8_REPO_DIR:-${SOURCE_REPO%/codex}/rusty_v8}"
  elif [[ "${target_mode}" == native && -d "${BUILD_REPO}/build/rusty-v8-artifacts/native" && -z "${RUSTY_V8_REPO_DIR:-}" ]]; then
    local_repo="${BUILD_REPO}/build/rusty-v8-artifacts/native"
  fi
  cache_dir="${BUILD_REPO}/build/rusty-v8-artifacts/${crate_version}/${target}"
  mkdir -p "${cache_dir}"
  release_tag="${RUSTY_V8_RELEASE_TAG:-rusty-v8-v${crate_version}}"
  resolver_args=(
    "--target=${target}"
    "--output-dir=${cache_dir}"
    "--cargo-build-dir=${build_root}"
    "--local-repo=${local_repo}"
    "--release-repo=${RUSTY_V8_RELEASE_REPO:-auto}"
    "--release-tag=${release_tag}"
    "--profile=${profile}"
    "--v8-version=${crate_version}"
  )
  local resolved_artifacts
  if ! resolved_artifacts="$(bash "${SOURCE_REPO}/scripts/resolve_rusty_v8_artifacts.sh" "${resolver_args[@]}")"; then
    echo "Rusty V8 artifact resolution failed for ${target}." >&2
    return 1
  fi
  eval "${resolved_artifacts}"
  RUSTY_V8_ARCHIVE_PATH="${RUSTY_V8_ARCHIVE}"
  RUSTY_V8_BINDING_PATH="${RUSTY_V8_SRC_BINDING_PATH}"

  [[ -s "${RUSTY_V8_ARCHIVE_PATH}" && -s "${RUSTY_V8_BINDING_PATH}" ]] || return 1
  verify_rusty_v8_artifacts "${target_mode}"
}

read_toolchain() {
  TOOLCHAIN="${RUST_TOOLCHAIN:-}"
  if [[ -z "${TOOLCHAIN}" && -f "${BUILD_WORKSPACE}/rust-toolchain.toml" ]]; then
    TOOLCHAIN="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "${BUILD_WORKSPACE}/rust-toolchain.toml" | head -n 1)"
  fi
  TOOLCHAIN="${TOOLCHAIN:-stable}"
  if command -v rustup >/dev/null 2>&1; then
    CARGO_CMD=(cargo +"${TOOLCHAIN}")
    RUSTC_CMD=(rustc +"${TOOLCHAIN}")
  else
    CARGO_CMD=(cargo)
    RUSTC_CMD=(rustc)
    echo "rustup unavailable; using standalone Cargo and Rustc." >&2
  fi
}

target_dir_for() {
  local mode="${1}" target_mode="${2}" purpose="${CODEX_CARGO_PURPOSE:-build}"
  local shared_env_script="${BUILD_REPO}/scripts/codex_cargo_env.sh"
  [[ -x "${shared_env_script}" ]] || die "shared Cargo environment helper not found: ${shared_env_script}"
  bash "${shared_env_script}" \
    --source-repo "${SOURCE_REPO}" \
    --build-repo "${BUILD_REPO}" \
    --mode "${mode}" \
    --target-mode "${target_mode}" \
    --purpose "${purpose}" \
    --print-target
}

target_triple() {
  case "${1}" in
    native) echo "" ;;
    musl) echo "x86_64-unknown-linux-musl" ;;
    armv7) echo "${ARMV7_TARGET:-armv7-unknown-linux-musleabihf}" ;;
    android) echo "aarch64-linux-android" ;;
  esac
}

ensure_target() {
  local triple="${1}"
  if ! command -v rustup >/dev/null 2>&1; then
    [[ "$("${RUSTC_CMD[@]}" -vV | sed -n 's/^host: //p')" == "${triple}" ]] \
      || die "rustup is required to install the Rust target ${triple}"
    return
  fi
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
    authenticate_sudo
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
  local mode="${1}" target_mode="${2}" purpose="${CODEX_CARGO_PURPOSE:-build}" profile_args=() target triple target_dir
  [[ "${mode}" == release ]] && profile_args+=(--release)
  triple="$(target_triple "${target_mode}")"

  local shared_env_script="${BUILD_REPO}/scripts/codex_cargo_env.sh"
  [[ -x "${shared_env_script}" ]] || die "shared Cargo environment helper not found: ${shared_env_script}"
  local cargo_env_output
  cargo_env_output="$(bash "${shared_env_script}" \
    --source-repo "${SOURCE_REPO}" \
    --build-repo "${BUILD_REPO}" \
    --mode "${mode}" \
    --target-mode "${target_mode}" \
    --purpose "${purpose}" \
    --emit)" || return $?
  eval "${cargo_env_output}" || return $?
  target_dir="${CARGO_TARGET_DIR}"

  if [[ "${target_mode}" == armv7 ]]; then
    local armv7_builder="${BUILD_REPO}/scripts/build_armv7.sh"
    [[ -x "${armv7_builder}" ]] || die "ARMv7 builder not found or not executable: ${armv7_builder}"
    local -a armv7_args=("--${mode}" "--target=${triple}" --no-deploy-remote --no-publish-github --binary-only)
    [[ -n "${CARGO_BUILD_JOBS:-}" ]] && armv7_args+=("--jobs=${CARGO_BUILD_JOBS}")
    armv7_args+=("--build-env=${ARMV7_BUILD_ENV}")
    if ! CARGO_TARGET_DIR="${target_dir}" "${armv7_builder}" "${armv7_args[@]}" >&2; then
      die "ARMv7 build failed; refusing to use a possibly stale binary at ${target_dir}/${triple}/${mode}/codex"
    fi
    printf '%s\n' "${target_dir}/${triple}/${mode}/codex"
    return 0
  fi

  if [[ -n "${triple}" ]]; then
    target="${triple}"
    ensure_target "${triple}"
  else
    target=""
  fi
  local -a env_args=()
  if [[ "${target_mode}" == native ]]; then
    env_args+=(
      -u CARGO_TARGET_DIR
      -u CC
      -u CXX
      -u AR
      -u RANLIB
      -u CFLAGS
      -u CXXFLAGS
      -u TARGET_CC
      -u TARGET_CXX
      -u TARGET_AR
      -u TARGET_RANLIB
      -u PKG_CONFIG_ALLOW_CROSS
      -u PKG_CONFIG_ALL_STATIC
      -u PKG_CONFIG_PATH
      -u PKG_CONFIG_LIBDIR
      -u PKG_CONFIG_SYSROOT_DIR
      -u CMAKE_C_COMPILER
      -u CMAKE_CXX_COMPILER
      -u CMAKE_ARGS
    )
  fi
  if configure_openssl_artifacts "${target_mode}"; then
    env_args+=(
      OPENSSL_DIR="${OPENSSL_DIR_PATH}"
      OPENSSL_NO_VENDOR=1
      OPENSSL_STATIC=1
    )
  fi
  env_args+=(
    CARGO_TARGET_DIR="${target_dir}"
    RUSTUP_DISABLE_SELF_UPDATE=1
    # Keep Cargo's compile input stable across commits and rebuilds. The
    # installed binary receives the real commit and wall-clock suffix later
    # via patch_timestamp.
    CODEX_BUILD_TIMESTAMP="0000000000-000000000000"
  )
  [[ -n "${CARGO_BUILD_JOBS:-}" ]] && env_args+=(CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS}")
  if [[ "${target_mode}" == native ]]; then
    configure_rusty_v8_artifacts "${target_mode}" || die "OpenAI Rusty V8 artifacts are unavailable for the native target"
    env_args+=(
      RUSTY_V8_ARCHIVE="${RUSTY_V8_ARCHIVE_PATH}"
      RUSTY_V8_SRC_BINDING_PATH="${RUSTY_V8_BINDING_PATH}"
    )
  else
    if configure_rusty_v8_artifacts "${target_mode}"; then
      env_args+=(
        RUSTY_V8_ARCHIVE="${RUSTY_V8_ARCHIVE_PATH}"
        RUSTY_V8_SRC_BINDING_PATH="${RUSTY_V8_BINDING_PATH}"
      )
    else
      echo "Rusty V8 artifacts are unavailable for ${target_mode}; building V8 from the prepared source checkout." >&2
      env_args+=(V8_FROM_SOURCE=1)
    fi
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
    configure_musl_build_tools "${triple}" >&2
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

  local -a cmd=("${CARGO_CMD[@]}" build -p codex-cli -p codex-rmcp-client)
  if [[ "${target_mode}" != armv7 ]]; then
    cmd+=(-p codex-code-mode-host)
  fi
  [[ "${target_mode}" == musl ]] && cmd+=(-p codex-bwrap)
  [[ -n "${target}" ]] && cmd+=(--target "${target}")
  cmd+=( "${profile_args[@]}" )
  if [[ "${LOCKFILE_REGENERATION_REQUIRED}" == true ]]; then
    cmd+=(--offline)
  else
    cmd+=(--locked)
  fi
  echo "Building ${mode} ${target_mode} in ${target_dir} (incremental cache retained)..." >&2
  if ! (cd "${BUILD_WORKSPACE}" && env "${env_args[@]}" "${cmd[@]}") >&2; then
    if [[ "${LOCKFILE_REGENERATION_REQUIRED}" != true ]]; then
      echo "Locked Cargo build failed; retrying online in the build tree without --locked." >&2
      unset 'cmd[-1]'
      if (cd "${BUILD_WORKSPACE}" && env "${env_args[@]}" "${cmd[@]}") >&2; then
        :
      else
        echo "Online Cargo build without --locked failed; retrying offline." >&2
        cmd+=(--offline)
        if ! (cd "${BUILD_WORKSPACE}" && env "${env_args[@]}" "${cmd[@]}") >&2; then
          return 1
        fi
      fi
    else
      return 1
    fi
  fi
  if [[ "${target_mode}" == native ]]; then
    local -a test_cmd=("${CARGO_CMD[@]}" build -p codex-rmcp-client --bin test_stdio_server)
    test_cmd+=( "${profile_args[@]}" )
    if [[ "${LOCKFILE_REGENERATION_REQUIRED}" == true ]]; then
      test_cmd+=(--offline)
    else
      test_cmd+=(--locked)
    fi
    if ! (cd "${BUILD_WORKSPACE}" && env "${env_args[@]}" "${test_cmd[@]}") >&2; then
      if [[ "${LOCKFILE_REGENERATION_REQUIRED}" != true ]]; then
        echo "Locked test_stdio_server build failed; retrying online in the build tree without --locked." >&2
        unset 'test_cmd[-1]'
        if (cd "${BUILD_WORKSPACE}" && env "${env_args[@]}" "${test_cmd[@]}") >&2; then
          :
        else
          echo "Online test_stdio_server build without --locked failed; retrying offline." >&2
          test_cmd+=(--offline)
          if ! (cd "${BUILD_WORKSPACE}" && env "${env_args[@]}" "${test_cmd[@]}") >&2; then
            return 1
          fi
        fi
      else
        return 1
      fi
    fi
  fi
  cache_openssl_artifacts "${target_mode}"
  if [[ -n "${target}" ]]; then
    echo "${target_dir}/${target}/${mode}/codex"
  else
    echo "${target_dir}/${mode}/codex"
  fi
}

patch_timestamp() {
  local binary="${1}" version="${2}" suffix="${3}${BUILD_TIMESTAMP_SEPARATOR}${TIMESTAMP}"
python3 - "${binary}" "${version}" "${suffix}" <<'PY'
import mmap, sys
import re
from pathlib import Path
path = Path(sys.argv[1])
version = sys.argv[2].encode()
replacement = (sys.argv[2] + "-" + sys.argv[3]).encode()
pattern = re.compile(re.escape(version) + rb"-[0-9a-f]{10}[-+][0-9]{12}")
if len(replacement) != len(version) + 1 + 10 + 1 + 12:
    raise SystemExit("version placeholder width mismatch")
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
  short="$(git -C "${SOURCE_REPO}" rev-parse --short=10 HEAD)"
  local name="codex-${version}-${short}${BUILD_TIMESTAMP_SEPARATOR}${TIMESTAMP}"
  install -m 0755 "${binary}" "${INSTALL_BIN_DIR}/${name}"
  if ! patch_timestamp "${INSTALL_BIN_DIR}/${name}" "${version}" "${short}"; then
    rm -f "${INSTALL_BIN_DIR}/${name}"
    return 1
  fi
  ln -sfn "${name}" "${INSTALL_BIN_DIR}/codex"
  cleanup_adjacent_same_size_binaries "${INSTALL_BIN_DIR}/${name}"
  echo "Installed ${INSTALL_BIN_DIR}/${name}"
  echo "Linked ${INSTALL_BIN_DIR}/codex"
}

cleanup_adjacent_same_size_binaries() {
  python3 - "${INSTALL_BIN_DIR}" "${1}" <<'PY'
import os
import re
import shutil
import subprocess
import sys

directory = sys.argv[1]
current_path = os.path.abspath(sys.argv[2])
filename_pattern = re.compile(r"^codex-.*-[0-9a-f]{10,12}[-+][0-9]{12}$")
fuser = shutil.which("fuser")
try:
    with open("/proc/net/unix", "rb"):
        pass
except OSError:
    print("Skipping adjacent binary usage checks: /proc/net/unix is not readable.", file=sys.stderr)
    fuser = None

files = []
for entry in os.scandir(directory):
    if not entry.is_file(follow_symlinks=False) or not filename_pattern.fullmatch(entry.name):
        continue
    try:
        stat = entry.stat(follow_symlinks=False)
    except OSError as error:
        print(f"Skipping {entry.path}: could not stat file: {error}", file=sys.stderr)
        continue
    files.append((stat.st_mtime_ns, stat.st_size, entry.path))

files.sort()
start = 0
while start < len(files):
    end = start + 1
    while end < len(files) and files[end][1] == files[start][1]:
        end += 1
    for _, _, path in files[start : end - 1]:
        if os.path.abspath(path) == current_path:
            continue
        if fuser is not None:
            usage = subprocess.run([fuser, "-s", path], check=False).returncode
            if usage != 1:
                print(f"Keeping adjacent binary in use or uncheckable: {path}", file=sys.stderr)
                continue
        try:
            os.unlink(path)
        except OSError as error:
            print(f"Could not remove adjacent binary {path}: {error}", file=sys.stderr)
        else:
            print(f"Removed older adjacent binary {path}")
    start = end
PY
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

remote_install_dir() {
  case "${1}" in
    native|musl|armv7|android) ssh "${SSH_OPTS[@]}" "${2}" 'printf "%s/.cargo/bin" "$HOME"' ;;
    *) die "unsupported remote build mode: ${1}" ;;
  esac
}

remote_glibc_version() {
  ssh "${SSH_OPTS[@]}" "${1}" 'getconf GNU_LIBC_VERSION 2>/dev/null | sed -n "s/^glibc //p"'
}

glibc_version_is_older() {
  local older_major older_minor newer_major newer_minor
  [[ "${1}" =~ ^([0-9]+)\.([0-9]+)$ ]] || return 1
  older_major="${BASH_REMATCH[1]}"
  older_minor="${BASH_REMATCH[2]}"
  [[ "${2}" =~ ^([0-9]+)\.([0-9]+)$ ]] || return 1
  newer_major="${BASH_REMATCH[1]}"
  newer_minor="${BASH_REMATCH[2]}"
  ((10#${older_major} < 10#${newer_major} \
    || (10#${older_major} == 10#${newer_major} \
      && 10#${older_minor} < 10#${newer_minor})))
}

native_glibc_is_compatible() {
  local target="${1}" local_glibc target_glibc
  local_glibc="$(getconf GNU_LIBC_VERSION 2>/dev/null | sed -n 's/^glibc //p')"
  target_glibc="$(remote_glibc_version "${target}")" || return 0
  [[ -n "${local_glibc}" && -n "${target_glibc}" ]] || return 0
  ! glibc_version_is_older "${target_glibc}" "${local_glibc}"
}

remote_target_architecture() {
  local target="${1}" architecture
  architecture="$(ssh "${SSH_OPTS[@]}" "${target}" 'rustc -vV 2>/dev/null | sed -n "s/^host: //p"; uname -m' \
    | head -n 1)" || {
      echo "Unable to reach install target ${target}; deferring it for retry." >&2
      return 75
    }
  [[ -n "${architecture}" ]] || {
    echo "Could not determine architecture for install target ${target}; deferring it for retry." >&2
    return 75
  }
  echo "${architecture}"
}

install_remote_binary() {
  local target="${1}" binary="${2}" version="${3}" install_dir="${4}" already_stamped="${5:-false}"
  local short name transfer_key staging remote_tmp
  [[ -x "${binary}" ]] || die "built binary not found: ${binary}"
  short="$(git -C "${SOURCE_REPO}" rev-parse --short=10 HEAD)"
  name="codex-${version}-${short}${BUILD_TIMESTAMP_SEPARATOR}${TIMESTAMP}"
  transfer_key="${version}-${short}"
  staging="${BUILD_REPO}/build/remote-install/${target}-${name}"
  if ! remote_tmp="$(ssh "${SSH_OPTS[@]}" "${target}" \
    'printf "%s/.codex-install-%s.tmp" "${TMPDIR:-/var/tmp}" "$1"' _ "${transfer_key}")"; then
    echo "Unable to reach install target ${target}; deferring it for retry." >&2
    rm -f "${staging}"
    return 75
  fi
  mkdir -p "$(dirname "${staging}")"
  install -m 0755 "${binary}" "${staging}"
  if [[ "${already_stamped}" != true ]]; then
    patch_timestamp "${staging}" "${version}" "${short}"
  fi
  if ! rsync --compress --info=progress2 --timeout="${CODEX_RSYNC_TIMEOUT:-60}" \
    --partial --inplace --append-verify -e "ssh ${SSH_OPTS[*]}" \
    -- "${staging}" "${target}:${remote_tmp}"; then
    echo "Unable to upload to install target ${target}; deferring it for retry." >&2
    rm -f "${staging}"
    return 75
  fi
  if ! ssh "${SSH_OPTS[@]}" "${target}" bash -s -- "${install_dir}" "${name}" "${remote_tmp}" <<'REMOTE_INSTALL'
set -euo pipefail
install_dir="$1"
name="$2"
remote_tmp="$3"
mkdir -p "$install_dir"
install -m 0755 "$remote_tmp" "$install_dir/$name"
ln -sfn "$name" "$install_dir/codex"
current="$install_dir/$name"
for candidate in "$install_dir"/codex-*; do
  [[ -f "$candidate" && ! -L "$candidate" && "$candidate" != "$current" ]] || continue
  [[ "$(stat -c '%s' "$candidate")" == "$(stat -c '%s' "$current")" ]] || continue
  [[ "$candidate" -nt "$current" ]] && continue
  if command -v fuser >/dev/null 2>&1 && fuser -s "$candidate"; then
    printf 'Keeping adjacent binary in use: %s\n' "$candidate" >&2
    continue
  fi
  rm -f "$candidate"
  printf 'Removed older adjacent binary %s\n' "$candidate"
done
case ":${PATH}:" in
  *:"${install_dir}":*) ;;
  *)
    mkdir -p "$HOME/bin"
    ln -sfn "$install_dir/codex" "$HOME/bin/codex"
    printf 'Linked %s/bin/codex -> %s/codex\n' "$HOME" "$install_dir"
    ;;
esac
rm -f "$remote_tmp"
printf 'Installed %s/%s\nLinked %s/codex\n' "$install_dir" "$name" "$install_dir"
REMOTE_INSTALL
  then
    echo "Unable to finish installation on ${target}; deferring it for retry." >&2
    ssh "${SSH_OPTS[@]}" "${target}" rm -f "${remote_tmp}" || true
    rm -f "${staging}"
    return 75
  fi
  cleanup_remote_install_artifacts "${target}" "${staging}"
  rm -f "${staging}"
}

cleanup_remote_install_artifacts() {
  local target="${1}" current="${2}" artifact
  for artifact in "${BUILD_REPO}/build/remote-install/${target}-codex-"*; do
    [[ -f "${artifact}" && "${artifact}" != "${current}" ]] || continue
    rm -f "${artifact}"
    echo "Removed older remote-install artifact ${artifact}"
  done
}

install_target() {
  local target="${1}" architecture target_mode binary install_dir already_stamped=false
  if ! architecture="$(remote_target_architecture "${target}")"; then
    return 75
  fi
  case "${architecture}" in
    armv7*|armv6*) target_mode=armv7 ;;
    aarch64-linux-android) target_mode=android ;;
    x86_64*)
      [[ "$(native_target_host)" == x86_64-* ]] \
        || die "install target ${target} has ${architecture}; local host is $(native_target_host)"
      if native_glibc_is_compatible "${target}"; then
        target_mode=native
      else
        echo "Target ${target} has older glibc; selecting the static musl build." >&2
        target_mode=musl
      fi
      ;;
    *) die "unsupported architecture ${architecture} for install target ${target}" ;;
  esac
  if ! install_dir="$(remote_install_dir "${target_mode}" "${target}")"; then
    echo "Unable to determine install directory on ${target}; deferring it for retry." >&2
    return 75
  fi
  echo "Installing to ${target} (${architecture}, ${target_mode})..." >&2
  TARGET_MODE="${target_mode}"
  case "${target_mode}" in
    native)
      [[ "${V8_FROM_SOURCE:-}" =~ ^(1|true|yes)$ ]] \
        && die "native V8 source builds are disabled; use the upstream Rusty V8 artifact"
      configure_rusty_v8_artifacts native \
        || die "OpenAI Rusty V8 artifacts are unavailable for the native target"
      ;;
    armv7)
      configure_rusty_v8_artifacts armv7 || prepare_armv7_rusty_v8_source armv7
      ;;
    android)
      configure_rusty_v8_artifacts android || prepare_armv7_rusty_v8_source android
      ;;
  esac
  refresh_build_lockfile
  if [[ "${target_mode}" == android ]]; then
    build_android
    binary="${BUILD_REPO}/build/android-artifact/codex.bin"
    already_stamped=true
  else
    binary="$(cargo_build "${MODE}" "${target_mode}")"
  fi
  install_remote_binary "${target}" "${binary}" "${VERSION}" "${install_dir}" "${already_stamped}"
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
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="${CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS:-} -Clink-arg=${builtins}"
  export CC_aarch64_linux_android="${llvm}/bin/aarch64-linux-android29-clang"
  export CXX_aarch64_linux_android="${llvm}/bin/aarch64-linux-android29-clang++"
  export AR_aarch64_linux_android="${llvm}/bin/llvm-ar"
  export RANLIB_aarch64_linux_android="${llvm}/bin/llvm-ranlib"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${llvm}/bin/aarch64-linux-android29-clang"
  # Source builds need Rusty V8's bundled NDK metadata; prebuilt artifact
  # builds do not, and their checkout may intentionally omit this source-only
  # directory.
  if [[ "${V8_FROM_SOURCE:-}" =~ ^(1|true|yes)$ ]] || ! configure_rusty_v8_artifacts android; then
    local ndk_properties="${RUSTY_V8_BUILD_REPO}/third_party/android_ndk/source.properties"
    local rusty_v8_ndk_version
    rusty_v8_ndk_version="$(sed -n 's/^Pkg.Revision = //p' "${ndk_properties}" | head -n 1)"
    [[ -n "${rusty_v8_ndk_version}" ]] || die "Rusty V8 bundled Android NDK version not found in ${ndk_properties}"
    local gclient_args="${RUSTY_V8_BUILD_REPO}/build/config/gclient_args.gni"
    if ! grep -Fq 'android_ndk_version' "${gclient_args}"; then
      printf 'declare_args() {\n  android_ndk_version = "%s"\n}\n' \
        "${rusty_v8_ndk_version}" >"${gclient_args}"
    fi
  fi
  local binary
  binary="$(cargo_build "${MODE}" android)"
  local stage="${BUILD_REPO}/build/android-artifact"
  mkdir -p "${stage}"
  install -m 0755 "${binary}" "${stage}/codex.bin"
  install -m 0755 "$(dirname "${binary}")/codex-code-mode-host" "${stage}/codex-code-mode-host"
  install -m 0644 "${llvm}/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so" "${stage}/libc++_shared.so"
  "${llvm}/bin/llvm-strip" --strip-all "${stage}/codex.bin"
  patch_timestamp "${stage}/codex.bin" "${VERSION}" "${COMMIT_SHORT}"
  echo "Android artifacts staged in ${stage}"
}

run_preflight() {
  (
    cd "${SOURCE_REPO}"
    bash -n \
      scripts/build_codex.sh \
      scripts/build_armv7.sh \
      scripts/resolve_rusty_v8_artifacts.sh \
      scripts/package_npm.sh \
      scripts/publish_npm_local.sh
    python3 -m py_compile \
      scripts/audit_npm_packages.py \
      scripts/assemble_npm_packages.py \
      scripts/stage_npm_packages.py
  )
  require_cmd cargo
  if command -v rustup >/dev/null 2>&1; then
    echo "rustup is available; using the configured ${TOOLCHAIN} toolchain."
  else
    echo "rustup is unavailable; using standalone Cargo and Rustc."
  fi
  require_cmd npm
  echo "Shell and required-tool preflight passed."
}

validate_release_checkout() {
  [[ "${SCRIPT_REPO}" == "${SOURCE_REPO}" ]] \
    || die "--start-github-release must be run from the source checkout, not a build tree"

  local branch
  branch="$(git -C "${SOURCE_REPO}" symbolic-ref --quiet --short HEAD)" \
    || die "--start-github-release requires a checked-out stable or alpha branch"
  [[ "${branch}" == stable || "${branch}" == alpha ]] \
    || die "--start-github-release requires the stable or alpha branch (got ${branch})"

}

while (($#)); do
  case "${1}" in
    --debug) MODE=debug; shift ;;
    --release) MODE=release; shift ;;
    --target) TARGET_MODE="${2:-}"; shift 2 ;;
    --target=*) TARGET_MODE="${1#*=}"; shift ;;
    --armv7) TARGET_MODE=armv7; shift ;;
    --armv7-build-env) ARMV7_BUILD_ENV="${2:-}"; shift 2 ;;
    --armv7-build-env=*) ARMV7_BUILD_ENV="${1#*=}"; shift ;;
    --build-npm-vendor) TARGET_MODE=musl; PACKAGE_NPM=true; shift ;;
    --package-local-npm|--package-npm) PACKAGE_NPM=true; shift ;;
    --publish-local-npm|--publish-npm) PACKAGE_NPM=true; PUBLISH_NPM=true; shift ;;
    --package-version) PACKAGE_VERSION="${2:-}"; shift 2 ;;
    --package-version=*) PACKAGE_VERSION="${1#*=}"; shift ;;
    --dry-run) DRY_RUN=true; shift ;;
    --start-github-release|--publish) PUBLISH=true; MODE=release; shift ;;
    --skip-build) SKIP_BUILD=true; shift ;;
    --preflight-only) PREFLIGHT_ONLY=true; shift ;;
    --no-sync) NO_SYNC=1; shift ;;
    --allow-concurrent-build) ALLOW_CONCURRENT_BUILD=true; shift ;;
    --jobs) CARGO_BUILD_JOBS="${2:-}"; shift 2 ;;
    --jobs=*) CARGO_BUILD_JOBS="${1#*=}"; shift ;;
    --install) INSTALL_TARGETS="${2:-}"; shift 2 ;;
    --install=*) INSTALL_TARGETS="${1#*=}"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option ${1} (use --help)" ;;
  esac
done

export CODEX_ALLOW_CONCURRENT_BUILD="${ALLOW_CONCURRENT_BUILD}"
export CODEX_BUILD_PID="$$"
register_build_process
trap cleanup_build_process EXIT

read_toolchain
native_target_host() {
  "${RUSTC_CMD[@]}" -vV | sed -n 's/^host: //p'
}

if [[ "${TARGET_MODE}" == android && "$(native_target_host)" == *-linux-android ]]; then
  echo "Android Rust host detected; treating --target android as native." >&2
  TARGET_MODE=native
fi

if [[ "${PUBLISH}" == true && ( "${PACKAGE_NPM}" == true || "${PUBLISH_NPM}" == true ) ]]; then
  die "GitHub release startup and local npm publication are separate operations"
fi
[[ "${SKIP_BUILD}" != true || "${PUBLISH}" == true ]] \
  || die "--skip-build requires --start-github-release"

if [[ "${PUBLISH_NPM}" == true ]]; then
  case "${TARGET_MODE}" in
    native) TARGET_MODE=all ;;
    all) ;;
    *) die "--publish-npm requires --target all when a target is specified" ;;
  esac
fi

IFS=',' read -r -a REQUESTED_TARGETS <<<"${TARGET_MODE}"
[[ "${#REQUESTED_TARGETS[@]}" -gt 0 ]] || die "target must not be empty"
PACKAGE_TARGETS=()
for requested_target in "${REQUESTED_TARGETS[@]}"; do
  case "${requested_target}" in
    native)
      if [[ "${PACKAGE_NPM}" == true ]]; then
        # The Linux npm payload is the portable musl build. Keep
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

require_cmd git
require_cmd python3
if [[ "${PUBLISH}" == true ]]; then
  validate_release_checkout
fi
VERSION="$(workspace_version)"
[[ -n "${VERSION}" ]] || die "could not determine workspace version"
if [[ "${PUBLISH}" == true ]]; then
  start_github_release
  exit 0
fi
if [[ -z "${PACKAGE_VERSION}" && "${PACKAGE_NPM}" == true ]]; then
  PACKAGE_VERSION="$(${SOURCE_REPO}/scripts/npm_candidate_version.sh)"
else
  PACKAGE_VERSION="${PACKAGE_VERSION:-${VERSION}}"
fi
[[ "${PACKAGE_VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+[-+.0-9A-Za-z-]+$ ]] \
  || die "invalid npm package version: ${PACKAGE_VERSION}"
COMMIT_SHORT="$(git -C "${SOURCE_REPO}" rev-parse --short=10 HEAD)"
if [[ -n "$(git -C "${SOURCE_REPO}" status --porcelain --untracked-files=all)" ]]; then
  BUILD_TIMESTAMP_SEPARATOR="+"
fi
if [[ -n "${CARGO_BUILD_JOBS:-}" ]]; then
  export CARGO_BUILD_JOBS
fi
sync_sources
if [[ -n "${INSTALL_TARGETS}" ]]; then
  require_cmd rsync
  IFS=',' read -r -a INSTALL_TARGET_LIST <<<"${INSTALL_TARGETS}"
  [[ "${#INSTALL_TARGET_LIST[@]}" -gt 0 ]] || die "install target list must not be empty"
  for install_target_name in "${INSTALL_TARGET_LIST[@]}"; do
    [[ -n "${install_target_name}" ]] || die "install target list contains an empty target"
  done
  FAILED_INSTALL_TARGETS=()
  for install_target_name in "${INSTALL_TARGET_LIST[@]}"; do
    if install_target "${install_target_name}"; then
      :
    else
      status=$?
      if [[ "${status}" -eq 75 ]]; then
        FAILED_INSTALL_TARGETS+=("${install_target_name}")
      else
        exit "${status}"
      fi
    fi
  done
  if [[ "${#FAILED_INSTALL_TARGETS[@]}" -gt 0 ]]; then
    echo "Retrying deferred target installation after the remaining builds..." >&2
    RETRY_FAILED_INSTALL_TARGETS=()
    for install_target_name in "${FAILED_INSTALL_TARGETS[@]}"; do
      if install_target "${install_target_name}"; then
        :
      else
        status=$?
        if [[ "${status}" -eq 75 ]]; then
          RETRY_FAILED_INSTALL_TARGETS+=("${install_target_name}")
        else
          exit "${status}"
        fi
      fi
    done
    if [[ "${#RETRY_FAILED_INSTALL_TARGETS[@]}" -gt 0 ]]; then
      printf 'Install still unavailable for: %s\n' \
        "$(IFS=,; echo "${RETRY_FAILED_INSTALL_TARGETS[*]}")" >&2
      exit 75
    fi
  fi
  exit 0
fi
if [[ "${PACKAGE_NPM}" == true ]]; then
  if [[ "${CODEX_BUILD_FROM_SOURCE:-false}" != true ]]; then
    download_latest_fork_npm_release
  fi
  BUILD_PACKAGE_TARGETS=()
  if [[ "${CODEX_BUILD_FROM_SOURCE:-false}" == true ]]; then
    BUILD_PACKAGE_TARGETS=("${PACKAGE_TARGETS[@]}")
  else
    for package_target in "${PACKAGE_TARGETS[@]}"; do
      if has_local_npm_platform_archive "${package_target}"; then
        echo "Reusing existing npm payload for ${package_target}." >&2
      else
        BUILD_PACKAGE_TARGETS+=("${package_target}")
      fi
    done
  fi
else
  BUILD_PACKAGE_TARGETS=()
fi
if [[ "${PACKAGE_NPM}" == true ]]; then
  for package_target in "${PACKAGE_TARGETS[@]}"; do
    case "${package_target}" in
      linux-armv7)
        : # build_armv7.sh owns ARMv7 toolchain and Rusty V8 setup.
        ;;
      android-arm64)
        configure_rusty_v8_artifacts android || prepare_armv7_rusty_v8_source android
        ;;
    esac
  done
elif [[ "${TARGET_MODE}" == armv7 || "${TARGET_MODE}" == android ]]; then
  if [[ "${TARGET_MODE}" == android ]]; then
    configure_rusty_v8_artifacts android || prepare_armv7_rusty_v8_source android
  fi
elif [[ "${TARGET_MODE}" == native ]]; then
  [[ "${V8_FROM_SOURCE:-}" =~ ^(1|true|yes)$ ]] && die "native V8 source builds are disabled; use the upstream Rusty V8 artifact"
  configure_rusty_v8_artifacts native || die "OpenAI Rusty V8 artifacts are unavailable for the native target"
fi
refresh_build_lockfile
if [[ "${PREFLIGHT_ONLY:-false}" == true ]]; then
  run_preflight
  exit 0
fi

if [[ "${PACKAGE_NPM}" == true ]]; then
  if [[ "${#BUILD_PACKAGE_TARGETS[@]}" -gt 0 ]]; then
    echo "Building npm target(s): $(IFS=,; echo "${BUILD_PACKAGE_TARGETS[*]}")" >&2
  fi
  for package_target in "${BUILD_PACKAGE_TARGETS[@]}"; do
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
  if [[ "${#BUILD_PACKAGE_TARGETS[@]}" -gt 0 ]]; then
    package_target_csv="$(IFS=,; echo "${BUILD_PACKAGE_TARGETS[*]}")"
    "${SOURCE_REPO}/scripts/package_npm.sh" "${PACKAGE_VERSION}" "${MODE}" "${package_target_csv}"
  fi
  if [[ "${TARGET_MODE}" == all || "${PUBLISH_NPM}" == true ]]; then
    complete_output="${BUILD_REPO}/build/npm-artifact-complete"
    rm -rf "${complete_output}"
    mkdir -p "${complete_output}"
    python3 "${SOURCE_REPO}/scripts/assemble_npm_packages.py" \
      --release-version "${PACKAGE_VERSION}" \
      --fork-artifact-dir "${BUILD_REPO}/build/npm-artifact" \
      --no-upstream \
      --output-dir "${complete_output}"
    for archive in "${complete_output}"/codex-npm-*.tgz; do
      sha256sum "${archive}" >"${archive}.sha256"
    done
    python3 "${SOURCE_REPO}/scripts/audit_npm_packages.py" \
      --artifact-dir "${complete_output}" \
      --expected-version "${PACKAGE_VERSION}"
    echo "Complete npm artifact set: ${complete_output}" >&2
    if [[ "${PUBLISH_NPM}" == true ]]; then
      "${SOURCE_REPO}/scripts/publish_npm_local.sh" "${complete_output}"
    fi
  fi
fi
