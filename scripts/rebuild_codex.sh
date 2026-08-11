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
SKIP_BUILD="false"
PACKAGE_NPM="false"
PUBLISH_NPM="false"
PREFLIGHT_ONLY="false"
DRY_RUN="false"
SYNCED="false"
RUSTY_V8_ARMV7_PREPARED="false"
RUSTY_V8_BUILD_REPO=""
PAGABLE_ARMV7_PATCH="false"
LOCKFILE_REGENERATION_REQUIRED="false"
TIMESTAMP="$(date -u +%Y%m%d%H%M)"
COMMIT_SHORT=""
BUILD_TIMESTAMP_SEPARATOR="-"
TOOLCHAIN=""
FORK_RELEASE_REPO="${CODEX_FORK_RELEASE_REPO:-rebroad/codex}"
SUDO_AUTHENTICATED="false"

usage() {
  cat <<'EOF'
Usage: scripts/rebuild_codex.sh [options]

Builds from the source checkout into the first existing sibling build tree
(<repo>.build or <repo>.make), retaining Cargo's incremental cache.

Options:
  --debug                  Build debug (default)
  --release                Build optimized release
  --target <names>         native, musl, armv7, android, all, or comma-separated targets
                           (all means the complete eight-architecture npm candidate)
  --armv7                  Alias for --target armv7
  --build-npm-vendor       Build the Linux musl payload for npm packaging
  --package-local-npm      Build/reuse local @reb.ai/codex npm archives
  --publish-local-npm      Assemble, audit, and publish npm locally
  --start-github-release   Push a release tag and start GitHub CI; print URLs
  --skip-build             With --start-github-release, reuse a completed CI build
  --package-npm            Alias for --package-local-npm
  --publish-npm            Alias for --publish-local-npm
  --publish                Alias for --start-github-release
  --package-version V      Override only the npm package release version
  --dry-run                Use supported dry-run checks
  --preflight-only         Run syntax/tooling checks without compiling
  --no-sync                Reuse the already-synced sibling source tree
  --jobs N                 Set CARGO_BUILD_JOBS
  --install-dir PATH       Install versioned binary and codex symlink there
  -h, --help               Show this help
EOF
}

die() { echo "rebuild_codex.sh: $*" >&2; exit 1; }
require_cmd() { command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"; }

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

set_pagable_patch() {
  local manifest="${1}" enabled="${2}"
  sed -i '/^# ARMv7 support is not in the crates.io release yet\.$/,+1d' "${manifest}"
  sed -i "/^# Keep pagable's shared workspace dependencies on crates.io so they use the$/,/^strong_hash = { version = \"0.1.0\" }$/d" "${manifest}"
  if [[ "${enabled}" == true ]]; then
    sed -i '/^\[patch\.crates-io\]$/a # ARMv7 support is not in the crates.io release yet.\npagable = { git = "https://github.com/facebook/starlark-rust", rev = "4190cefd570e05858cbb51815a4de11a7b49f951" }' "${manifest}"
    cat >>"${manifest}" <<'EOF'

# Keep pagable's shared workspace dependencies on crates.io so they use the
# same Dupe/Allocative traits as the released Starlark crates.
[patch."https://github.com/facebook/starlark-rust"]
allocative = { version = "0.3.6" }
dupe = { version = "0.9.1" }
strong_hash = { version = "0.1.0" }
EOF
  fi
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
  local tag="${1}" run_id artifact_count
  while read -r run_id; do
    [[ -n "${run_id}" ]] || continue
    artifact_count="$(gh api "repos/${FORK_RELEASE_REPO}/actions/runs/${run_id}/artifacts?per_page=100" \
      --jq '[.artifacts[] | select(.expired == false and (.name | startswith("npm-source-"))) | .name] | unique | length')"
    if [[ "${artifact_count}" == 8 ]]; then
      echo "${run_id}"
      return 0
    fi
  done < <(gh run list --repo "${FORK_RELEASE_REPO}" \
    --workflow custom-codex-release.yml --status completed --limit 50 \
    --json databaseId,headBranch,event,conclusion \
    | jq -r --arg tag "${tag}" '.[] | select(.event == "push" and .conclusion == "success" and .headBranch == $tag) | .databaseId')
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

start_github_release() {
  require_cmd gh
  require_cmd jq
  local release_version tag run_info run_id run_url workflow_ref dispatch_started
  release_version="$(${SOURCE_REPO}/scripts/npm_candidate_version.sh)"
  tag="codex-v${release_version}"
  echo "GitHub Actions workflow: https://github.com/${FORK_RELEASE_REPO}/actions/workflows/custom-codex-release.yml" >&2
  if [[ "${SKIP_BUILD}" == true ]]; then
    workflow_ref="$(git -C "${SOURCE_REPO}" branch --show-current)"
    [[ -n "${workflow_ref}" ]] || die "--skip-build requires a checked-out branch"
    if [[ "${DRY_RUN}" == true ]]; then
      echo "Would dispatch GitHub CI for ${tag} using the latest completed artifact run." >&2
      return 0
    fi
    git -C "${SOURCE_REPO}" ls-remote --exit-code origin "refs/tags/${tag}" >/dev/null \
      || die "release tag ${tag} does not exist; --skip-build can only retry an existing tagged build"
    run_id="$(find_reusable_github_run "${tag}")" \
      || die "no completed ${tag} run has all eight reusable npm source artifacts"
    echo "Reusing artifacts from GitHub CI run: https://github.com/${FORK_RELEASE_REPO}/actions/runs/${run_id}" >&2
    dispatch_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    gh workflow run custom-codex-release.yml --repo "${FORK_RELEASE_REPO}" --ref "${workflow_ref}" \
      -f "tag=${tag}" -f "reuse_artifacts_run_id=${run_id}" -f publish_npm=true
    echo "Waiting for the fast-path GitHub workflow run..." >&2
    run_info=""
    for _ in {1..30}; do
      run_info="$(gh run list --repo "${FORK_RELEASE_REPO}" \
        --workflow custom-codex-release.yml --event workflow_dispatch --limit 20 \
        --json databaseId,url,createdAt \
        | jq -r --arg started "${dispatch_started}" '[.[] | select(.createdAt >= $started)] | sort_by(.createdAt) | .[0] | [.databaseId, .url] | @tsv')"
      read -r run_id run_url <<<"${run_info}"
      [[ -n "${run_id}" ]] && break
      sleep 2
    done
    [[ -n "${run_id}" ]] || die "fast-path workflow was dispatched but did not appear"
    show_github_run "${run_id}" "${run_url}"
    return 0
  fi
  if [[ "${DRY_RUN}" == true ]]; then
    echo "Would create and push GitHub tag ${tag}" >&2
    return 0
  fi
  git -C "${SOURCE_REPO}" tag -a "${tag}" -m "Release ${VERSION}"
  git -C "${SOURCE_REPO}" push origin "${tag}"
  echo "Waiting for GitHub to create the workflow run..." >&2
  run_info=""
  for _ in {1..30}; do
    run_info="$(gh run list --repo "${FORK_RELEASE_REPO}" \
      --workflow custom-codex-release.yml --limit 20 --json databaseId,headBranch,url \
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
  set_v8_path_patch "${manifest}" "${build_repo}"
  RUSTY_V8_ARMV7_PREPARED="true"
}

refresh_build_lockfile() {
  local source_lock="${SOURCE_REPO}/codex-rs/Cargo.lock"
  local build_lock="${BUILD_WORKSPACE}/Cargo.lock"
  local fingerprint_file="${BUILD_REPO}/build/.codex-source-lock-fingerprint"
  local source_fingerprint stored_fingerprint=""
  source_fingerprint="$(sed '/^version = /d' "${source_lock}" | sha256sum | awk '{print $1}')"
  [[ -f "${fingerprint_file}" ]] && read -r stored_fingerprint <"${fingerprint_file}"
  echo "Refreshing generated build-tree Cargo.lock..." >&2
  if [[ ! -f "${build_lock}" || "${source_fingerprint}" != "${stored_fingerprint}" ]]; then
    cp "${source_lock}" "${build_lock}"
    mkdir -p "$(dirname "${fingerprint_file}")"
    printf '%s\n' "${source_fingerprint}" >"${fingerprint_file}"
  else
    echo "Keeping generated build-tree Cargo.lock." >&2
  fi
  if [[ "${PAGABLE_ARMV7_PATCH}" == true ]]; then
    (cd "${BUILD_WORKSPACE}" && cargo +"${TOOLCHAIN}" update -p pagable)
  fi
  if [[ "${RUSTY_V8_ARMV7_PREPARED}" == true ]]; then
    (cd "${BUILD_WORKSPACE}" && cargo +"${TOOLCHAIN}" update -p v8 --offline)
  fi
  if ! (cd "${BUILD_WORKSPACE}" && cargo +"${TOOLCHAIN}" metadata \
    --format-version 1 --locked --offline >/dev/null 2>&1); then
    LOCKFILE_REGENERATION_REQUIRED="true"
    echo "Generated build-tree Cargo.lock requires offline regeneration; skipping locked Cargo builds." >&2
  fi
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

configure_rusty_v8_artifacts() {
  local target_mode="${1}" target archive binding local_repo cache_dir release_tag base_url release_repo preferred_release_repo
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
    android)
      target="aarch64-linux-android"
      ;;
    *)
      return
      ;;
  esac

  local crate_version="${V8_CRATE_VERSION:-$(sed -n '/^name = "v8"$/,/^version = /s/^version = "\([^"]*\)"/\1/p' "${SOURCE_REPO}/codex-rs/Cargo.lock" | head -n 1)}"
  [[ -n "${crate_version}" ]] || die "could not determine the pinned v8 crate version"
  local default_profile="ptrcomp_sandbox_release"
  local profile="${RUSTY_V8_PROFILE:-${default_profile}}"
  archive="librusty_v8_${profile}_${target}.a.gz"
  binding="src_binding_${profile}_${target}.rs"

  if [[ -n "${RUSTY_V8_ARCHIVE:-}" || -n "${RUSTY_V8_SRC_BINDING_PATH:-}" ]]; then
    [[ -s "${RUSTY_V8_ARCHIVE:-}" && -s "${RUSTY_V8_SRC_BINDING_PATH:-}" ]] \
      || die "RUSTY_V8_ARCHIVE and RUSTY_V8_SRC_BINDING_PATH must point to existing files"
    RUSTY_V8_ARCHIVE_PATH="${RUSTY_V8_ARCHIVE}"
    RUSTY_V8_BINDING_PATH="${RUSTY_V8_SRC_BINDING_PATH}"
    verify_rusty_v8_artifacts "${target_mode}"
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

  local_repo=""
  if [[ "${target_mode}" == armv7 || "${target_mode}" == android || "${target_mode}" == musl ]]; then
    local_repo="${RUSTY_V8_REPO_DIR:-${SOURCE_REPO%/codex}/rusty_v8}"
  elif [[ "${target_mode}" == native && -d "${BUILD_REPO}/build/rusty-v8-artifacts/native" && -z "${RUSTY_V8_REPO_DIR:-}" ]]; then
    local_repo="${BUILD_REPO}/build/rusty-v8-artifacts/native"
  fi
  cache_dir="${BUILD_REPO}/build/rusty-v8-artifacts/${VERSION}/${target}"
  mkdir -p "${cache_dir}"

  if [[ -n "${local_repo}" && -f "${local_repo}/${archive}" && -f "${local_repo}/${binding}" ]]; then
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
      preferred_release_repo="${RUSTY_V8_RELEASE_REPO:-}"
      if [[ -z "${preferred_release_repo}" ]]; then
        case "${target_mode}" in
          musl|armv7|android) preferred_release_repo="rebroad/rusty_v8" ;;
          *) preferred_release_repo="openai/codex" ;;
        esac
      fi
      local release_repos=("${preferred_release_repo}")
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

  [[ -s "${RUSTY_V8_ARCHIVE_PATH}" && -s "${RUSTY_V8_BINDING_PATH}" ]] || return 1
  verify_rusty_v8_artifacts "${target_mode}"
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
  case "${target_mode}" in
    native) echo "${BUILD_WORKSPACE}/target" ;;
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

  local -a env_args=(
    CARGO_TARGET_DIR="${target_dir}"
    RUSTUP_DISABLE_SELF_UPDATE=1
    CODEX_BUILD_TIMESTAMP="${COMMIT_SHORT}${BUILD_TIMESTAMP_SEPARATOR}${TIMESTAMP}"
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

  local -a cmd=(cargo +"${TOOLCHAIN}" build -p codex-cli -p codex-rmcp-client)
  [[ "${target_mode}" != armv7 ]] && cmd+=(-p codex-code-mode-host)
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
      echo "Locked Cargo build failed; retrying offline in the build tree without --locked." >&2
      unset 'cmd[-1]'
      cmd+=(--offline)
      if ! (cd "${BUILD_WORKSPACE}" && env "${env_args[@]}" "${cmd[@]}") >&2; then
        return 1
      fi
    else
      return 1
    fi
  fi
  if [[ "${target_mode}" == native ]]; then
    local -a test_cmd=(cargo +"${TOOLCHAIN}" build -p codex-rmcp-client --bin test_stdio_server)
    test_cmd+=( "${profile_args[@]}" )
    if [[ "${LOCKFILE_REGENERATION_REQUIRED}" == true ]]; then
      test_cmd+=(--offline)
    else
      test_cmd+=(--locked)
    fi
    if ! (cd "${BUILD_WORKSPACE}" && env "${env_args[@]}" "${test_cmd[@]}") >&2; then
      if [[ "${LOCKFILE_REGENERATION_REQUIRED}" != true ]]; then
        echo "Locked test_stdio_server build failed; retrying offline in the build tree without --locked." >&2
        unset 'test_cmd[-1]'
        test_cmd+=(--offline)
        if ! (cd "${BUILD_WORKSPACE}" && env "${env_args[@]}" "${test_cmd[@]}") >&2; then
          return 1
        fi
      else
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
if fuser is None:
    print("Skipping adjacent binary cleanup: fuser is unavailable.", file=sys.stderr)
    raise SystemExit

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
  install -m 0644 "${llvm}/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so" "${stage}/libc++_shared.so"
  "${llvm}/bin/llvm-strip" --strip-all "${stage}/codex.bin"
  patch_timestamp "${stage}/codex.bin" "${VERSION}" "${COMMIT_SHORT}"
  echo "Android artifacts staged in ${stage}"
}

run_preflight() {
  (cd "${SOURCE_REPO}" && bash -n scripts/rebuild_codex.sh scripts/build.sh scripts/package_npm.sh)
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
    --package-local-npm|--package-npm) PACKAGE_NPM=true; shift ;;
    --publish-local-npm|--publish-npm) PACKAGE_NPM=true; PUBLISH_NPM=true; shift ;;
    --package-version) PACKAGE_VERSION="${2:-}"; shift 2 ;;
    --package-version=*) PACKAGE_VERSION="${1#*=}"; shift ;;
    --dry-run) DRY_RUN=true; shift ;;
    --start-github-release|--publish) PUBLISH=true; MODE=release; shift ;;
    --skip-build) SKIP_BUILD=true; shift ;;
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
VERSION="$(workspace_version)"
[[ -n "${VERSION}" ]] || die "could not determine workspace version"
if [[ "${PUBLISH}" == true ]]; then
  start_github_release
  exit 0
fi
read_toolchain
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
        configure_rusty_v8_artifacts armv7 || prepare_armv7_rusty_v8_source
        ;;
      android-arm64)
        configure_rusty_v8_artifacts android || prepare_armv7_rusty_v8_source
        ;;
    esac
  done
elif [[ "${TARGET_MODE}" == armv7 || "${TARGET_MODE}" == android ]]; then
  configure_rusty_v8_artifacts "${TARGET_MODE}" || prepare_armv7_rusty_v8_source
elif [[ "${TARGET_MODE}" == native ]]; then
  [[ "${V8_FROM_SOURCE:-}" =~ ^(1|true|yes)$ ]] && die "native V8 source builds are disabled; use the upstream Rusty V8 artifact"
  configure_rusty_v8_artifacts native || die "OpenAI Rusty V8 artifacts are unavailable for the native target"
fi
if [[ "${PACKAGE_NPM}" == true ]]; then
  for package_target in "${PACKAGE_TARGETS[@]}"; do
    if [[ "${package_target}" == linux-armv7 ]]; then
      PAGABLE_ARMV7_PATCH="true"
      break
    fi
  done
elif [[ "${TARGET_MODE}" == armv7 ]]; then
  PAGABLE_ARMV7_PATCH="true"
fi
set_pagable_patch "${BUILD_WORKSPACE}/Cargo.toml" "${PAGABLE_ARMV7_PATCH}"
refresh_build_lockfile
if [[ "${PREFLIGHT_ONLY:-false}" == true ]]; then
  run_preflight
  exit 0
fi

COMMIT_SHORT="$(git -C "${SOURCE_REPO}" rev-parse --short=10 HEAD)"
if [[ -n "$(git -C "${SOURCE_REPO}" status --porcelain --untracked-files=all)" ]]; then
  BUILD_TIMESTAMP_SEPARATOR="+"
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
