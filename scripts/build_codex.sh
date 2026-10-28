#!/usr/bin/env bash
set -euo pipefail

# Keep the logical path here: the designated build sibling may itself be a
# symlink into a separate build filesystem, and source discovery depends on
# its logical .build/.make name.
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -L)"
BUILD_TREE=""
SCRIPT_REPO="$(cd -- "${SCRIPT_DIR}/.." && pwd -L)"
BUILD_REPO_OVERRIDE="${CODEX_BUILD_REPO:-}"
case "${SCRIPT_REPO##*/}" in
  *.build|*.make)
    BUILD_TREE="${SCRIPT_REPO}"
    SOURCE_REPO=""
    if git_common_dir="$(git -C "${SCRIPT_REPO}" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" \
      && [[ "$(basename "${git_common_dir}")" == .git ]] \
      && [[ -d "${git_common_dir}" ]]; then
      source_candidate="$(dirname "${git_common_dir}")"
      if [[ -d "${source_candidate}/codex-rs" ]]; then
        SOURCE_REPO="${source_candidate}"
      fi
    fi
    if [[ -z "${SOURCE_REPO}" ]]; then
      source_candidate="${SCRIPT_REPO%.*}"
      if [[ -d "${source_candidate}/codex-rs" ]]; then
        SOURCE_REPO="$(cd -- "${source_candidate}" && pwd -P)"
      fi
    fi
    ;;
  *)
    SOURCE_REPO="${SCRIPT_REPO}"
    if [[ -n "${BUILD_REPO_OVERRIDE}" ]]; then
      BUILD_TREE="${BUILD_REPO_OVERRIDE}"
    else
      BUILD_TREE="${SOURCE_REPO}.build"
    fi
    ;;
esac
if [[ -z "${BUILD_TREE}" ]]; then
  echo "No build tree found. Set CODEX_BUILD_REPO to an external .build/.make path, or create ${SOURCE_REPO}.build" >&2
  exit 1
fi
if [[ -z "${SOURCE_REPO}" || ! -d "${SOURCE_REPO}/codex-rs" ]]; then
  echo "Source checkout could not be discovered for build tree ${BUILD_TREE}" >&2
  exit 1
fi
BUILD_REPO="${BUILD_TREE}"
BUILD_WORKSPACE="${BUILD_REPO}/codex-rs"
INSTALL_BIN_DIR="${INSTALL_BIN_DIR:-${HOME}/.cargo/bin}"
VERSION=""
PACKAGE_VERSION=""
MODE="debug"
CARGO_PROFILE="debug"
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
RESTART_APP_SERVER="false"
PROFILE_OPTION_SET="false"
SOURCE_LOCK_BACKUP=""
ANDROID_PSI_NOTICE_DIR=""
ANDROID_BALLAST_PID=""
ANDROID_BALLAST_DIR=""
ANDROID_RUST_TIMING_DIR=""
TIMESTAMP="$(date -u +%Y%m%d%H%M)"
COMMIT_SHORT=""
SOURCE_START_COMMIT=""
SOURCE_START_STATUS=""
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
# The script is deliberately sourced from the source checkout before it
# synchronizes that checkout into the build tree.
source "${SOURCE_REPO}/scripts/openssl_artifacts.sh"
source "${SOURCE_REPO}/scripts/codex_sccache_server.sh"

usage() {
  cat <<'EOF'
Usage: scripts/build_codex.sh [options]

Builds from the source checkout into CODEX_BUILD_REPO or the designated
external build tree, retaining Cargo's target artifacts.

Options:
  --debug                  Build debug (default)
  --dev-small              Build the unoptimized, stripped dev-small profile
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
  --no-sync                Reuse the already-synced external source tree
  --allow-concurrent-build  Bypass the shared Cargo target lock and share the target directory
  --restart-if-idle        Restart local/remote app-servers after changed installs when idle
  --jobs N                 Set CARGO_BUILD_JOBS
  --install TARGETS        Build and install to targets (SSH hosts or localhost), optionally
                           with per-target profiles: localhost:dev,pi3:dev-small
                           Defaults: localhost=dev, SSH targets=dev-small
  -h, --help               Show this help
EOF
}

die() { echo "build_codex.sh: $*" >&2; exit 1; }
require_cmd() { command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"; }

run_timed_build() {
  local stage="${1}" target_mode="${2}" profile="${3}" metrics_file status=0
  shift 3
  mkdir -p "${BUILD_REPO}/build"
  local timing_tmpdir
  if [[ -n "${TERMUX_VERSION:-}" || "${PREFIX:-}" == /data/data/*/files/usr ]]; then
    timing_tmpdir="${PREFIX:-/data/data/com.termux/files/usr}/tmp"
  else
    timing_tmpdir="${TMPDIR:-/var/tmp}"
  fi
  metrics_file="$(mktemp "${timing_tmpdir}/codex-build-timing.XXXXXX")"
  if (
    exec 3>&2
    exec 2>"${metrics_file}"
    export LC_NUMERIC=C
    TIMEFORMAT='%3U %3S %3R'
    time "$@" 2>&3
  ); then
    status=0
  else
    status=$?
  fi
  local user_cpu system_cpu elapsed
  read -r user_cpu system_cpu elapsed < <(tail -n 1 "${metrics_file}")
  sed '$d' "${metrics_file}" >&2
  rm -f "${metrics_file}"
  python3 - "${BUILD_REPO}/build/codex-build-timings.jsonl" \
    "${BUILD_TIMING_TARGET:-${target_mode}}" "${target_mode}" "${profile}" \
    "${stage}" "${user_cpu:-0}" "${system_cpu:-0}" "${elapsed:-0}" "${status}" <<'PY'
import fcntl
import json
import sys
from datetime import datetime, timezone

path, install_target, target_mode, profile, stage, user, system, elapsed, status = sys.argv[1:]
record = {
    "timestamp_utc": datetime.now(timezone.utc).isoformat(),
    "install_target": install_target,
    "target_mode": target_mode,
    "profile": profile,
    "stage": stage,
    "user_cpu_seconds": float(user),
    "system_cpu_seconds": float(system),
    "cpu_seconds": float(user) + float(system),
    "elapsed_seconds": float(elapsed),
    "exit_code": int(status),
}
with open(path, "a", encoding="utf-8") as output:
    fcntl.flock(output, fcntl.LOCK_EX)
    output.write(json.dumps(record, sort_keys=True) + "\n")
    output.flush()
PY
  return "${status}"
}

refresh_build_provenance() {
  local current_commit current_status
  current_commit="$(git -C "${SOURCE_REPO}" rev-parse HEAD)"
  current_status="$(git -C "${SOURCE_REPO}" status --porcelain --untracked-files=all \
    | sed -E '/(^|[[:space:]])codex-rs\/Cargo\.(lock|toml)$/d')"
  if [[ "${current_commit}" != "${SOURCE_START_COMMIT}" || "${current_status}" != "${SOURCE_START_STATUS}" ]]; then
    BUILD_TIMESTAMP_SEPARATOR="+"
    echo "Source checkout changed during the build; using '+' in the installed version." >&2
    if [[ "${current_commit}" != "${SOURCE_START_COMMIT}" ]]; then
      printf '  commit: %s -> %s\n' "${SOURCE_START_COMMIT}" "${current_commit}" >&2
    fi
    if [[ "${current_status}" != "${SOURCE_START_STATUS}" ]]; then
      echo '  source status delta (Cargo.lock excluded):' >&2
      diff -u \
        --label 'source status at start' \
        --label 'source status at end' \
        <(printf '%s\n' "${SOURCE_START_STATUS}") \
        <(printf '%s\n' "${current_status}") >&2 || true
    fi
  fi
}

capture_build_provenance() {
  SOURCE_START_COMMIT="$(git -C "${SOURCE_REPO}" rev-parse HEAD)"
  SOURCE_START_STATUS="$(git -C "${SOURCE_REPO}" status --porcelain --untracked-files=all \
    | sed -E '/(^|[[:space:]])codex-rs\/Cargo\.(lock|toml)$/d')"
  COMMIT_SHORT="${SOURCE_START_COMMIT:0:10}"
  if [[ -n "${SOURCE_START_STATUS}" ]]; then
    BUILD_TIMESTAMP_SEPARATOR="+"
    echo "Source checkout is dirty; using '+' in the installed version." >&2
  fi
}

set_termux_build_oom_score() {
  [[ -n "${TERMUX_VERSION:-}" || "${PREFIX:-}" == /data/data/com.termux/files/usr ]] || return 0

  local oom_score_adj="${CODEX_BUILD_OOM_SCORE_ADJ:-500}"
  [[ "${oom_score_adj}" =~ ^[0-9]+$ ]] || die "CODEX_BUILD_OOM_SCORE_ADJ must be a non-negative integer"
  [[ -w /proc/self/oom_score_adj ]] || die "Termux build cannot set /proc/self/oom_score_adj"
  printf '%s\n' "${oom_score_adj}" > /proc/self/oom_score_adj \
    || die "failed to set /proc/self/oom_score_adj"
  echo "Termux build OOM score preference: ${oom_score_adj}" >&2
}

set_termux_build_oom_score

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
release_cargo_target_lock() {
  [[ -n "${CODEX_CARGO_HELD_TARGET_LOCK_FILE:-}" \
    && "${CODEX_CARGO_HELD_TARGET_LOCK_FILE}" == "${TARGET_LOCK_FILE:-}" ]] || return 0
  flock -u 9
  exec 9>&-
  unset CODEX_CARGO_HELD_TARGET_LOCK_FILE CODEX_CARGO_HELD_TARGET_LOCK_PID
}
# One ballast per Cargo stage (not one per concurrently running rustc).
# The controller is disposable; the OOM wrapper remains the final safety guard.
stop_android_ballast() {
  if [[ -n "${ANDROID_BALLAST_PID}" ]]; then
    kill -TERM "${ANDROID_BALLAST_PID}" 2>/dev/null || true
    wait "${ANDROID_BALLAST_PID}" 2>/dev/null || true
    ANDROID_BALLAST_PID=""
  fi
  if [[ -n "${ANDROID_BALLAST_DIR}" ]]; then
    rm -rf -- "${ANDROID_BALLAST_DIR}"
    ANDROID_BALLAST_DIR=""
  fi
}
start_android_ballast() {
  local max_mb="${CODEX_ANDROID_RUSTC_BALLAST_MB:-256}"
  local temp_root="${TMPDIR:-${PREFIX:-/data/data/com.termux/files/usr}/tmp}"
  local attempt held_mb
  local -a tune_args=()
  [[ "${max_mb}" =~ ^[0-9]+$ ]] || die "CODEX_ANDROID_RUSTC_BALLAST_MB must be a non-negative integer"
  ((max_mb > 0)) || return 1
  if ! mkdir -p -- "${temp_root}" \
    || ! ANDROID_BALLAST_DIR="$(mktemp -d "${temp_root%/}/codex-android-ballast.XXXXXX")"; then
    echo "Unable to create Android RAM ballast runtime directory; continuing without ballast." >&2
    return 1
  fi
  # An explicit ballast size is a manual override and disables learning.
  # By default, clean Cargo runs can tune the next reserve in 32 MiB steps.
  if [[ "${CODEX_ANDROID_RUSTC_BALLAST_AUTOTUNE:-1}" == 1 \
        && ! -v CODEX_ANDROID_RUSTC_BALLAST_MB ]]; then
    mkdir -p -- "${BUILD_REPO}/build"
    tune_args=(
      --history-file "${BUILD_REPO}/build/codex-android-ballast-autotune.json"
      --build-result-file "${ANDROID_BALLAST_DIR}/build-result"
      --autotune-cap-mb "${CODEX_ANDROID_RUSTC_BALLAST_AUTOTUNE_CAP_MB:-384}"
    )
  fi
  python3 -u "${SOURCE_REPO}/scripts/codex_android_ballast.py" \
    --max-mb "${max_mb}" \
    --chunk-mb "${CODEX_ANDROID_RUSTC_BALLAST_CHUNK_MB:-8}" \
    --ramp-ms "${CODEX_ANDROID_RUSTC_BALLAST_RAMP_MS:-250}" \
    --floor-percent "${CODEX_ANDROID_RUSTC_BALLAST_FLOOR_PERCENT:-12}" \
    --ready-file "${ANDROID_BALLAST_DIR}/ready" \
    --status-file "${ANDROID_BALLAST_DIR}/status" \
    --telemetry-file "${ANDROID_BALLAST_DIR}/telemetry" \
    --psi-status-file "${ANDROID_BALLAST_DIR}/psi-sample" \
    "${tune_args[@]}" 9>&- &
  ANDROID_BALLAST_PID=$!
  for ((attempt = 0; attempt < 160; attempt++)); do
    [[ -f "${ANDROID_BALLAST_DIR}/ready" ]] && break
    kill -0 "${ANDROID_BALLAST_PID}" 2>/dev/null || break
    sleep 0.25
  done
  if [[ ! -f "${ANDROID_BALLAST_DIR}/ready" ]]; then
    echo "Android RAM ballast did not initialize; continuing with the existing memory guard." >&2
    stop_android_ballast
    return 1
  fi
  held_mb=0
  read -r held_mb <"${ANDROID_BALLAST_DIR}/status" || true
  if [[ ! "${held_mb}" =~ ^[0-9]+$ ]] || ((held_mb == 0)); then
    echo "Android RAM ballast has no safe headroom; continuing without it." >&2
    stop_android_ballast
    return 1
  fi
  return 0
}
cleanup_build_process() {
  stop_android_ballast
  if [[ -n "${ANDROID_RUST_TIMING_DIR}" ]]; then
    rm -rf -- "${ANDROID_RUST_TIMING_DIR}"
    ANDROID_RUST_TIMING_DIR=""
  fi
  release_cargo_target_lock
  codex_sccache_server_stop
  if [[ -n "${ANDROID_PSI_NOTICE_DIR}" ]]; then
    rm -rf "${ANDROID_PSI_NOTICE_DIR}"
  fi
  if [[ -n "${SOURCE_LOCK_BACKUP}" && -f "${SOURCE_LOCK_BACKUP}" ]]; then
    if cp -p "${SOURCE_LOCK_BACKUP}" "${SOURCE_REPO}/codex-rs/Cargo.lock"; then
      rm -f "${SOURCE_LOCK_BACKUP}"
    else
      echo "Unable to restore source Cargo.lock from ${SOURCE_LOCK_BACKUP}" >&2
    fi
  fi
  rm -f "${BUILD_PROCESS_RECORD}" "${BUILD_REPO}/build/.native-mold-result-$$"
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
  bash "${SOURCE_REPO}/scripts/sync_build_tree.sh" "${SOURCE_REPO}" "${BUILD_REPO}"
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
    python3 "${SOURCE_REPO}/scripts/rsync_git_sync.py" \
      "${source_repo}" "${build_repo}" --exclude=/third_party/rust-toolchain
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
  if [[ "$(cd -- "${SOURCE_REPO}" && pwd -P)" == "$(cd -- "${BUILD_REPO}" && pwd -P)" ]]; then
    python3 "${SOURCE_REPO}/scripts/normalize_cargo_lock.py" \
      --manifest "${BUILD_WORKSPACE}/Cargo.toml" \
      --source-lock "${source_lock}" \
      --build-lock "${source_lock}"
    echo "Temporarily normalized the source Cargo.lock for the source-tree build." >&2
    return
  fi
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
  # Git stores workspace package versions as 0.0.0 to avoid version churn.
  # Cargo resolves them to the actual workspace version in the build tree;
  # materialize those versions after source syncs the Git lockfile, before any
  # Cargo operation can observe the generated lockfile.
  python3 "${SOURCE_REPO}/scripts/normalize_cargo_lock.py" \
    --manifest "${BUILD_WORKSPACE}/Cargo.toml" \
    --source-lock "${source_lock}" \
    --build-lock "${build_lock}"
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
  # V8's pointer compression and sandbox require a 64-bit target.
  [[ "${target_mode}" == armv7 ]] && default_profile="release"
  local rusty_v8_profile="${RUSTY_V8_PROFILE:-${default_profile}}"
  local target_dir build_profile build_root
  target_dir="$(target_dir_for "${MODE}" "${target_mode}")"
  build_profile="${CARGO_PROFILE}"
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
    "--profile=${rusty_v8_profile}"
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

build_armv7_bwrap() {
  local profile="${1}" target_dir="${2}"
  local target="armv7-unknown-linux-musleabihf"
  local cache_dir="${target_dir}/cache"
  local env_file="${cache_dir}/musl-env"
  local runner_temp="${cache_dir}/runner"
  local setup_script="${SOURCE_REPO}/.github/scripts/install-musl-build-tools.sh"
  local bwrap_log="${cache_dir}/bwrap-build.log"
  local -a profile_args=()

  case "${profile}" in
    release) profile_args+=(--release) ;;
    dev-small) profile_args+=(--profile dev-small) ;;
  esac
  mkdir -p "${runner_temp}"
  if [[ ! -s "${env_file}" ]]; then
    [[ -f "${setup_script}" ]] || die "ARMv7 musl build-tool setup script not found: ${setup_script}"
    : >"${env_file}"
    TARGET="${target}" \
      GITHUB_ENV="${env_file}" \
      RUNNER_TEMP="${runner_temp}" \
      CODEX_BUILD_TOOLS_DIR="${BUILD_REPO}/build/tools" \
      bash "${setup_script}" >&2
  fi
  while IFS= read -r assignment; do
    [[ -n "${assignment}" ]] && export "${assignment}"
  done <"${env_file}"

  echo "Building bundled bubblewrap for ${target}..." >&2
  if ! (cd "${BUILD_WORKSPACE}" && run_timed_build bubblewrap "armv7" "${profile}" env \
    CARGO_TARGET_DIR="${target_dir}" \
    RUSTUP_DISABLE_SELF_UPDATE=1 \
    CODEX_BUILD_TIMESTAMP="0000000000-000000000000" \
    "${CARGO_CMD[@]}" build -p codex-bwrap --target "${target}" \
    "${profile_args[@]}" --locked) >"${bwrap_log}" 2>&1; then
    echo "ARMv7 bubblewrap build failed; last 40 lines:" >&2
    tail -n 40 "${bwrap_log}" >&2
    return 1
  fi
}

cargo_build() {
  local mode="${1}" target_mode="${2}" profile="${3:-${CARGO_PROFILE}}" purpose="${CODEX_CARGO_PURPOSE:-build}" profile_args=() target triple target_dir psi_probe_status
  case "${profile}" in
    release) profile_args+=(--release) ;;
    dev-small) profile_args+=(--profile dev-small) ;;
    debug) ;;
    *) die "unsupported Cargo profile: ${profile}" ;;
  esac
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
  if [[ "${CODEX_ANDROID_RUSTC_MEMORY_GUARD:-0}" == 1 ]]; then
    python3 - "${target_dir}" <<'PY'
from pathlib import Path
import sys

target_dir = Path(sys.argv[1])
notice = b"Android Rust memory guard: PSI unavailable; using RAM and swap guards only"
fingerprint_dirs = list(target_dir.glob("*/.fingerprint"))
fingerprint_dirs.extend(target_dir.glob("*/*/.fingerprint"))
removed = 0
for fingerprint_dir in fingerprint_dirs:
    for output in fingerprint_dir.glob("*/output-*"):
        try:
            original = output.read_bytes()
        except OSError:
            continue
        lines = original.splitlines(keepends=True)
        kept = [line for line in lines if line.rstrip(b"\r\n") != notice]
        if len(kept) != len(lines):
            output.write_bytes(b"".join(kept))
            removed += len(lines) - len(kept)
if removed:
    print(f"Removed {removed} stale PSI fallback notice(s) from Cargo fingerprints.", file=sys.stderr)
PY
    if [[ -n "${ANDROID_PSI_NOTICE_DIR}" ]] \
      && mkdir "${ANDROID_PSI_NOTICE_DIR}/probed" 2>/dev/null; then
      psi_probe_status=0
      "${RUSTC_WRAPPER}" --codex-psi-notice-probe /bin/true || psi_probe_status=$?
      if [[ "${psi_probe_status}" == 10 ]]; then
        echo 'Android Rust memory guard: PSI unavailable; using RAM and swap guards only' >&2
      elif [[ "${psi_probe_status}" != 0 ]]; then
        echo "Android Rust memory guard PSI notice probe failed; compiler memory guards remain enabled" >&2
      fi
    fi
  fi
  codex_sccache_server_start || return $?

  if [[ "${target_mode}" == armv7 ]]; then
    local armv7_builder="${BUILD_REPO}/scripts/build_armv7.sh"
    [[ -x "${armv7_builder}" ]] || die "ARMv7 builder not found or not executable: ${armv7_builder}"
    local -a armv7_args=("--${profile}" "--target=${triple}" --no-deploy-remote --no-publish-github --binary-only)
    local -a armv7_v8_env=()
    if [[ -z "${RUSTY_V8_ARCHIVE_PATH:-}" || -z "${RUSTY_V8_BINDING_PATH:-}" ]]; then
      configure_rusty_v8_artifacts armv7 \
        || die "Rusty V8 artifacts are unavailable for the ARMv7 build"
    fi
    armv7_v8_env=(
      "RUSTY_V8_ARCHIVE=${RUSTY_V8_ARCHIVE_PATH}"
      "RUSTY_V8_SRC_BINDING_PATH=${RUSTY_V8_BINDING_PATH}"
    )
    [[ -n "${CARGO_BUILD_JOBS:-}" ]] && armv7_args+=("--jobs=${CARGO_BUILD_JOBS}")
    armv7_args+=("--build-env=${ARMV7_BUILD_ENV}")
    if ! run_timed_build armv7 "${target_mode}" "${profile}" env CARGO_TARGET_DIR="${target_dir}" "${armv7_v8_env[@]}" \
      "${armv7_builder}" "${armv7_args[@]}" >&2; then
      die "ARMv7 build failed; refusing to use a possibly stale binary at ${target_dir}/${triple}/${profile}/codex"
    fi
    build_armv7_bwrap "${profile}" "${target_dir}"
    release_cargo_target_lock
    printf '%s\n' "${target_dir}/${triple}/${profile}/codex"
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
  if [[ "${target_mode}" == native ]]; then
    # Build the auxiliary native executable in the same Cargo invocation as
    # the main binaries. Separate invocations resolve features independently
    # and can rebuild a large portion of the dependency graph.
    cmd+=(--bin codex --bin codex-code-mode-host --bin test_stdio_server)
  fi
  [[ -n "${target}" ]] && cmd+=(--target "${target}")
  cmd+=( "${profile_args[@]}" )
  if [[ "${LOCKFILE_REGENERATION_REQUIRED}" == true ]]; then
    cmd+=(--offline)
  else
    cmd+=(--locked)
  fi
  run_cargo_build() {
    local stage="${1}" stderr_log status=0
    shift
    if [[ "${CODEX_ANDROID_RUSTC_MEMORY_GUARD:-0}" != 1 ]]; then
      (cd "${BUILD_WORKSPACE}" && run_timed_build "${stage}" "${target_mode}" "${profile}" env "${env_args[@]}" "${cmd[@]}") >&2
      return $?
    fi

    # Clear any inherited controller reference if ballast is disabled or
    # cannot initialize; never signal a stale PID from another build.
    local -a ballast_env_args=(
      CODEX_ANDROID_RUSTC_BALLAST_PID=
      CODEX_ANDROID_RUSTC_BALLAST_STATUS_FILE=
      CODEX_ANDROID_RUSTC_BALLAST_TELEMETRY_FILE=
      CODEX_ANDROID_RUSTC_BALLAST_PSI_FILE=
    )
    local temp_root="${TMPDIR:-${PREFIX:-/data/data/com.termux/files/usr}/tmp}"
    mkdir -p -- "${temp_root}"
    # A new shared ledger for each Cargo stage, including concurrent rustc
    # processes. Avoid /proc/<pid>/stat, which Android may not permit reading.
    ANDROID_RUST_TIMING_DIR="$(mktemp -d "${temp_root%/}/codex-rust-timing.XXXXXX")" || return 1
    ballast_env_args+=(CODEX_ANDROID_RUSTC_TIMING_DIR="${ANDROID_RUST_TIMING_DIR}")
    if start_android_ballast; then
      ballast_env_args+=(
        CODEX_ANDROID_RUSTC_BALLAST_PID="${ANDROID_BALLAST_PID}"
        CODEX_ANDROID_RUSTC_BALLAST_STATUS_FILE="${ANDROID_BALLAST_DIR}/status"
        CODEX_ANDROID_RUSTC_BALLAST_TELEMETRY_FILE="${ANDROID_BALLAST_DIR}/telemetry"
        CODEX_ANDROID_RUSTC_BALLAST_PSI_FILE="${ANDROID_BALLAST_DIR}/psi-sample"
      )
      echo "🧠 Ballast enabled for ${stage}: controller ${ANDROID_BALLAST_PID}, base reserve ${CODEX_ANDROID_RUSTC_BALLAST_MB:-256} MiB; live reports at most once/minute" >&2
    else
      echo "🧠 Ballast inactive for ${stage}; Rust PSI/RAM safeguards remain enabled" >&2
    fi
    [[ -f "${RUSTC_WRAPPER}" ]] || die "Android rustc wrapper missing: ${RUSTC_WRAPPER}"
    [[ -f "${RUSTC_WRAPPER%/*}/codex_rustc_timing.sh" ]] || die "Android rustc timing helper missing next to ${RUSTC_WRAPPER}"
    echo "Android Rust wrapper: ${RUSTC_WRAPPER} (sha256 $(sha256sum "${RUSTC_WRAPPER}" | cut -c1-12)); timing ledger ${ANDROID_RUST_TIMING_DIR}" >&2
    stderr_log="$(mktemp "${temp_root%/}/codex-cargo-stderr.XXXXXX")"
    if (cd "${BUILD_WORKSPACE}" && run_timed_build "${stage}" "${target_mode}" "${profile}" env "${env_args[@]}" "${ballast_env_args[@]}" "${cmd[@]}") 2>&1 \
      | tee "${stderr_log}" \
      | awk '
        {
          # A single line per minute for the whole Cargo stage, even when
          # Cargo runs many independent rustc wrapper processes.
          if (index($0, "🧠 Ballast:") == 1) {
            now = systime()
            if (last_ballast && now - last_ballast < 60) next
            last_ballast = now
            printf "[%s] %s\n", strftime("%H:%M:%S"), $0
          } else if (index($0, "Rust time: kept ") == 1) {
            # Frequent short rustc invocations should not flood the log.
            now = systime()
            if (last_timing && now - last_timing < 60) next
            last_timing = now
            print
          } else {
            print
          }
          fflush()
        }
      ' >&2; then
      status=0
    else
      status=$?
    fi
    rm -f "${stderr_log}"
    # Always print the final shared ledger before deleting the stage runtime,
    # including short stages whose progress snapshots were throttled.
    local kept_us=0 suspended_us=0 lost_us
    if [[ -r "${ANDROID_RUST_TIMING_DIR}/totals" ]] \
      && read -r kept_us suspended_us lost_us <"${ANDROID_RUST_TIMING_DIR}/totals" \
      && [[ "${kept_us}" =~ ^[0-9]+$ && "${suspended_us}" =~ ^[0-9]+$ \
         && "${lost_us}" =~ ^[0-9]+$ ]]; then
      local k=$((kept_us / 1000000)) p=$((suspended_us / 1000000)) l=$((lost_us / 1000000))
      printf '⏱ Rust totals (%s): kept %02d:%02d:%02d; suspended %02d:%02d:%02d; lost %02d:%02d:%02d\n' \
        "${stage}" "$((k / 3600))" "$(((k / 60) % 60))" "$((k % 60))" \
        "$((p / 3600))" "$(((p / 60) % 60))" "$((p % 60))" \
        "$((l / 3600))" "$(((l / 60) % 60))" "$((l % 60))" >&2
    fi
    if [[ -n "${ANDROID_BALLAST_DIR}" && -d "${ANDROID_BALLAST_DIR}" ]]; then
      printf '%s %s\n' "${status}" "${lost_us}" >"${ANDROID_BALLAST_DIR}/build-result"
    fi
    stop_android_ballast
    if [[ -n "${ANDROID_RUST_TIMING_DIR}" ]]; then
      rm -rf -- "${ANDROID_RUST_TIMING_DIR}"
      ANDROID_RUST_TIMING_DIR=""
    fi
    return "${status}"
  }
  echo "Building ${profile} ${target_mode} in ${target_dir} (target artifacts retained)..." >&2
  if ! run_cargo_build cargo; then
    if [[ "${LOCKFILE_REGENERATION_REQUIRED}" != true ]]; then
      echo "Locked Cargo build failed; retrying online in the build tree without --locked." >&2
      unset 'cmd[-1]'
      if run_cargo_build cargo-retry-online; then
        :
      else
        echo "Online Cargo build without --locked failed; retrying offline." >&2
        cmd+=(--offline)
        if ! run_cargo_build cargo-retry-offline; then
          return 1
        fi
      fi
    else
      return 1
    fi
  fi
  # The lock protects shared Cargo target writes. Release it before artifact
  # bookkeeping or installation commands can spawn long-lived helpers that
  # would inherit fd 9 and keep the lock held after this build finishes.
  release_cargo_target_lock
  cache_openssl_artifacts "${target_mode}"
  if [[ -n "${target}" ]]; then
    echo "${target_dir}/${target}/${profile}/codex"
  else
    echo "${target_dir}/${profile}/codex"
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

strip_installed_binary() {
  local binary="${1}" host strip_cmd
  host="$(native_target_host)"
  case "${host}" in
    *apple-darwin)
      if ! strip -S -x "${binary}"; then
        echo "Unable to strip debug symbols from ${binary}." >&2
        return 1
      fi
      ;;
    *)
      for strip_cmd in "${STRIP:-}" llvm-strip strip; do
        [[ -n "${strip_cmd}" ]] || continue
        command -v "${strip_cmd}" >/dev/null 2>&1 || continue
        if "${strip_cmd}" --strip-debug --strip-unneeded "${binary}"; then
          echo "Stripped debug symbols from ${binary} with ${strip_cmd}."
          return 0
        fi
      done
      echo "Unable to strip debug symbols from ${binary}: no compatible strip tool succeeded." >&2
      return 1
      ;;
  esac
  echo "Stripped debug symbols from ${binary}."
}

install_binary() {
  local binary="${1}" version="${2}" short
  [[ -x "${binary}" ]] || die "built binary not found: ${binary}"
  mkdir -p "${INSTALL_BIN_DIR}"
  short="$(git -C "${SOURCE_REPO}" rev-parse --short=10 HEAD)"
  local name="codex-${version}-${short}${BUILD_TIMESTAMP_SEPARATOR}${TIMESTAMP}"
  install -m 0755 "${binary}" "${INSTALL_BIN_DIR}/${name}"
  if ! strip_installed_binary "${INSTALL_BIN_DIR}/${name}"; then
    rm -f "${INSTALL_BIN_DIR}/${name}"
    return 1
  fi
  if ! patch_timestamp "${INSTALL_BIN_DIR}/${name}" "${version}" "${short}"; then
    rm -f "${INSTALL_BIN_DIR}/${name}"
    return 1
  fi
  ln -sfn "${name}" "${INSTALL_BIN_DIR}/codex"
  cleanup_adjacent_same_size_binaries "${INSTALL_BIN_DIR}/${name}"
  echo "Installed ${INSTALL_BIN_DIR}/${name}"
  echo "Linked ${INSTALL_BIN_DIR}/codex"
  local running_version="" running_commit="" daemon_version_output=""
  if daemon_version_output="$("${INSTALL_BIN_DIR}/codex" app-server daemon version 2>/dev/null)"; then
    running_version="$(python3 -c 'import json,sys; value=json.load(sys.stdin).get("appServerVersion"); print(value or "")' <<<"${daemon_version_output}")"
  fi
  if [[ "${running_version}" =~ -([0-9a-f]{10,12})[-+][0-9]{12}$ ]]; then
    running_commit="${BASH_REMATCH[1]}"
  fi
  local installed_version="${version}-${short}${BUILD_TIMESTAMP_SEPARATOR}${TIMESTAMP}"
  if [[ "${RESTART_APP_SERVER}" == true \
    && ( "${running_commit}" != "${short}" || "${running_version}" != "${installed_version}" ) ]]; then
    echo "Requesting a graceful app-server restart after active turns finish."
    nohup "${INSTALL_BIN_DIR}/codex" app-server daemon restart-if-idle \
      </dev/null >/dev/null 2>&1 &
  fi
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
try:
    current_size = os.stat(current_path).st_size
except OSError as error:
    print(
        f"Skipping adjacent binary cleanup: could not stat current binary: {error}",
        file=sys.stderr,
    )
    raise SystemExit(0)
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
    if stat.st_size != current_size:
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
  if ! strip_installed_binary "${INSTALL_BIN_DIR}/codex-code-mode-host"; then
    rm -f "${INSTALL_BIN_DIR}/codex-code-mode-host"
    return 1
  fi
  echo "Installed ${INSTALL_BIN_DIR}/codex-code-mode-host"
}

install_test_stdio_server() {
  local binary="${1}"
  [[ -x "${binary}" ]] || die "built test stdio server not found: ${binary}"
  install -m 0755 "${binary}" "${INSTALL_BIN_DIR}/test_stdio_server"
  if ! strip_installed_binary "${INSTALL_BIN_DIR}/test_stdio_server"; then
    rm -f "${INSTALL_BIN_DIR}/test_stdio_server"
    return 1
  fi
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
  architecture="$(ssh "${SSH_OPTS[@]}" "${target}" 'rustc -vV 2>/dev/null | sed -n "s/^host: //p"; if command -v rustc >/dev/null 2>&1; then :; elif [ "$(uname -m)" = aarch64 ] && [ -n "${PREFIX:-}" ] && [ -d "${PREFIX:-}" ]; then printf "%s\\n" aarch64-linux-android; else uname -m; fi' \
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
  local name transfer_key staging remote_tmp
  [[ -x "${binary}" ]] || die "built binary not found: ${binary}"
  refresh_build_provenance
  name="codex-${version}-${COMMIT_SHORT}${BUILD_TIMESTAMP_SEPARATOR}${TIMESTAMP}"
  transfer_key="${version}-${COMMIT_SHORT}"
  staging="${BUILD_REPO}/build/remote-install/${target}-${name}"
  if ! remote_tmp="$(ssh "${SSH_OPTS[@]}" "${target}" \
    "printf '%s/.codex-install-${transfer_key}.tmp' \"\${TMPDIR:-/var/tmp}\"")"; then
    echo "Unable to reach install target ${target}; deferring it for retry." >&2
    rm -f "${staging}"
    return 75
  fi
  mkdir -p "$(dirname "${staging}")"
  install -m 0755 "${binary}" "${staging}"
  if [[ "${already_stamped}" != true ]]; then
    patch_timestamp "${staging}" "${version}" "${COMMIT_SHORT}"
  fi
  if ! rsync --compress --info=progress2 --timeout="${CODEX_RSYNC_TIMEOUT:-60}" \
    --partial --inplace --append-verify -e "ssh ${SSH_OPTS[*]}" \
    -- "${staging}" "${target}:${remote_tmp}"; then
    echo "Unable to upload to install target ${target}; deferring it for retry." >&2
    rm -f "${staging}"
    return 75
  fi
  if ! ssh "${SSH_OPTS[@]}" "${target}" bash -s -- \
    "${install_dir}" "${name}" "${remote_tmp}" "${RESTART_APP_SERVER}" <<'REMOTE_INSTALL'
set -euo pipefail
install_dir="$1"
name="$2"
remote_tmp="$3"
restart_if_idle="$4"
mkdir -p "$install_dir"
remote_size="$(stat -c '%s' "$remote_tmp")"
previous_commit=""
previous_size=""
if [[ -L "$install_dir/codex" ]]; then
  previous_target="$(readlink "$install_dir/codex")"
  if [[ "$previous_target" =~ -([0-9a-f]{10,12})[-+][0-9]{12}$ ]]; then
    previous_commit="${BASH_REMATCH[1]}"
  fi
  if [[ -f "$install_dir/$previous_target" ]]; then
    previous_size="$(wc -c <"$install_dir/$previous_target")"
  fi
fi
for candidate in "$install_dir"/codex-*; do
  [[ -f "$candidate" && ! -L "$candidate" ]] || continue
  [[ "$(stat -c '%s' "$candidate")" == "$remote_size" ]] || continue
  [[ "$candidate" -nt "$remote_tmp" ]] && continue
  if command -v fuser >/dev/null 2>&1 && fuser -s "$candidate"; then
    printf 'Keeping adjacent binary in use: %s\n' "$candidate" >&2
    continue
  fi
  rm -f "$candidate"
  printf 'Removed older adjacent binary %s\n' "$candidate"
done
install -m 0755 "$remote_tmp" "$install_dir/$name"
ln -sfn "$name" "$install_dir/codex"
new_commit=""
if [[ "$name" =~ -([0-9a-f]{10,12})[-+][0-9]{12}$ ]]; then
  new_commit="${BASH_REMATCH[1]}"
fi
new_size="$(wc -c <"$install_dir/$name")"
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
if [[ "$restart_if_idle" == true \
  && ( "$previous_commit" != "$new_commit" || "$previous_size" != "$new_size" ) ]]; then
  printf 'Requesting an idle app-server restart on the remote target.\n'
  nohup "$install_dir/codex" app-server daemon restart-if-idle \
    </dev/null >/dev/null 2>&1 &
fi
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

install_remote_auxiliary_binary() {
  local target="${1}" binary="${2}" install_dir="${3}" name="${4}"
  local staging remote_tmp
  [[ -x "${binary}" ]] || die "built auxiliary binary not found: ${binary}"
  staging="${BUILD_REPO}/build/remote-install/${target}-${name}"
  if ! remote_tmp="$(ssh "${SSH_OPTS[@]}" "${target}" \
    "printf '%s/.codex-${name}.tmp' \"\${TMPDIR:-/var/tmp}\"")"; then
    echo "Unable to reach install target ${target}; deferring it for retry." >&2
    rm -f "${staging}"
    return 75
  fi
  mkdir -p "$(dirname "${staging}")"
  install -m 0755 "${binary}" "${staging}"
  if ! rsync --compress --info=progress2 --timeout="${CODEX_RSYNC_TIMEOUT:-60}" \
    --partial --inplace --append-verify -e "ssh ${SSH_OPTS[*]}" \
    -- "${staging}" "${target}:${remote_tmp}"; then
    echo "Unable to upload auxiliary binary to install target ${target}; deferring it for retry." >&2
    rm -f "${staging}"
    return 75
  fi
  if ! ssh "${SSH_OPTS[@]}" "${target}" bash -s -- "${install_dir}" "${name}" "${remote_tmp}" <<'REMOTE_AUXILIARY_INSTALL'
set -euo pipefail
install_dir="$1"
name="$2"
remote_tmp="$3"
mkdir -p "$install_dir"
install -m 0755 "$remote_tmp" "$install_dir/$name"
rm -f "$remote_tmp"
printf 'Installed %s/%s\n' "$install_dir" "$name"
REMOTE_AUXILIARY_INSTALL
  then
    echo "Unable to finish auxiliary binary installation on ${target}; deferring it for retry." >&2
    ssh "${SSH_OPTS[@]}" "${target}" rm -f "${remote_tmp}" || true
    rm -f "${staging}"
    return 75
  fi
  rm -f "${staging}"
}

install_remote_helper_script() {
  local target="${1}" name="${2}" staging remote_tmp remote_tmp_dir source
  source="${SOURCE_REPO}/scripts/remote-control/${name}"
  [[ -f "${source}" ]] || die "remote-control helper not found: ${source}"
  staging="${BUILD_REPO}/build/remote-install/${target}-${name}"
  if ! remote_tmp_dir="$(ssh "${SSH_OPTS[@]}" "${target}" '
    tmp_dir="${TMPDIR:-}"
    if [ -z "$tmp_dir" ] && [ -n "${PREFIX:-}" ]; then
      tmp_dir="$PREFIX/tmp"
    fi
    printf "%s" "${tmp_dir:-/var/tmp}"
  ')"; then
    echo "Unable to reach install target ${target}; deferring helper installation." >&2
    rm -f "${staging}"
    return 75
  fi
  remote_tmp="${remote_tmp_dir}/.codex-${name}.tmp"
  mkdir -p "$(dirname "${staging}")"
  install -m 0755 "${source}" "${staging}"
  if ! rsync --compress --info=progress2 --timeout="${CODEX_RSYNC_TIMEOUT:-60}" \
    --partial --inplace --append-verify -e "ssh ${SSH_OPTS[*]}" \
    -- "${staging}" "${target}:${remote_tmp}"; then
    echo "Unable to upload remote-control helper ${name} to ${target}; deferring it for retry." >&2
    rm -f "${staging}"
    return 75
  fi
  if ! ssh "${SSH_OPTS[@]}" "${target}" bash -s -- "${name}" "${remote_tmp}" <<'REMOTE_HELPER_INSTALL'
set -euo pipefail
name="$1"
remote_tmp="$2"
mkdir -p "$HOME/bin"
rm -f "$HOME/bin/$name"
install -m 0755 "$remote_tmp" "$HOME/bin/$name"
rm -f "$remote_tmp"
printf 'Installed %s/bin/%s\n' "$HOME" "$name"
REMOTE_HELPER_INSTALL
  then
    echo "Unable to finish remote-control helper installation on ${target}; deferring it for retry." >&2
    ssh "${SSH_OPTS[@]}" "${target}" rm -f "${remote_tmp}" || true
    rm -f "${staging}"
    return 75
  fi
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
  local requested_mode="${MODE}" requested_profile="${CARGO_PROFILE}" target_spec="${1}"
  local MODE CARGO_PROFILE target architecture target_mode binary install_dir already_stamped=false profile_override=""
  MODE="${requested_mode}"
  CARGO_PROFILE="${requested_profile}"
  target="${target_spec%%:*}"
  if [[ "${target_spec}" == *:* ]]; then
    profile_override="${target_spec#*:}"
    case "${profile_override}" in
      dev|debug) MODE=debug; CARGO_PROFILE=debug ;;
      dev-small) MODE=debug; CARGO_PROFILE=dev-small ;;
      release) MODE=release; CARGO_PROFILE=release ;;
      *) die "unsupported install profile in ${target_spec}; use dev, dev-small, or release" ;;
    esac
  elif [[ "${PROFILE_OPTION_SET}" != true ]]; then
    if [[ "${target}" == localhost ]]; then
      MODE=debug
      CARGO_PROFILE=debug
    else
      MODE=debug
      CARGO_PROFILE=dev-small
    fi
  fi
  BUILD_TIMING_TARGET="${target}"
  if [[ "${target}" == localhost ]]; then
    target_mode=native
    echo "Installing to localhost ($(native_target_host), native)..." >&2
    binary="$(cargo_build "${MODE}" "${target_mode}" "${CARGO_PROFILE}")" || return $?
    install_binary "${binary}" "${VERSION}" || return $?
    install_code_mode_host "$(dirname "${binary}")/codex-code-mode-host" || return $?
    install_test_stdio_server "$(dirname "${binary}")/test_stdio_server" || return $?
    return 0
  fi
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
  capture_build_provenance
  if [[ "${target_mode}" == android ]]; then
    build_android || return $?
    binary="${BUILD_REPO}/build/android-artifact/codex.bin"
    already_stamped=true
  else
    binary="$(cargo_build "${MODE}" "${target_mode}" "${CARGO_PROFILE}")" || return $?
  fi
  install_remote_binary "${target}" "${binary}" "${VERSION}" "${install_dir}" "${already_stamped}" || return $?
  if [[ "${target_mode}" == armv7 || "${target_mode}" == musl ]]; then
    local bwrap_binary="$(dirname "${binary}")/bwrap"
    if [[ "${target_mode}" == armv7 ]]; then
      local armv7_triple="${ARMV7_TARGET:-armv7-unknown-linux-musleabihf}"
      bwrap_binary="${binary%/${armv7_triple}/${CARGO_PROFILE}/codex}/${armv7_triple}/${CARGO_PROFILE}/bwrap"
    fi
    install_remote_auxiliary_binary \
      "${target}" "${bwrap_binary}" \
      "${install_dir}/codex-resources" bwrap || return $?
  fi
  if [[ "${target_mode}" == native || "${target_mode}" == musl || "${target_mode}" == armv7 ]]; then
    install_remote_auxiliary_binary \
      "${target}" "$(dirname "${binary}")/codex-code-mode-host" \
      "${install_dir}" codex-code-mode-host || return $?
  fi
  for helper in codex-remote-start codex-pairing-code codex-remote-healthcheck; do
    install_remote_helper_script "${target}" "${helper}" || return $?
  done
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
  binary="$(cargo_build "${MODE}" android "${CARGO_PROFILE}")" || return $?
  local stage="${BUILD_REPO}/build/android-artifact"
  mkdir -p "${stage}"
  install -m 0755 "${binary}" "${stage}/codex.bin"
  install -m 0755 "$(dirname "${binary}")/codex-code-mode-host" "${stage}/codex-code-mode-host"
  install -m 0644 "${llvm}/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so" "${stage}/libc++_shared.so"
  for staged_binary in "${stage}/codex.bin" "${stage}/codex-code-mode-host"; do
    "${llvm}/bin/llvm-strip" --strip-all "${staged_binary}"
  done
  refresh_build_provenance
  patch_timestamp "${stage}/codex.bin" "${VERSION}" "${COMMIT_SHORT}"
  echo "Android artifacts staged in ${stage}"
}

run_preflight() {
  (
    cd "${SOURCE_REPO}"
    bash -n \
      scripts/build_codex.sh \
      scripts/codex_rustc_oom_wrapper.sh \
      scripts/codex_rustc_timing.sh \
      scripts/tests/test_codex_rustc_timing.sh \
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
    --debug) MODE=debug; CARGO_PROFILE=debug; PROFILE_OPTION_SET=true; shift ;;
    --dev-small) MODE=debug; CARGO_PROFILE=dev-small; PROFILE_OPTION_SET=true; shift ;;
    --release) MODE=release; CARGO_PROFILE=release; PROFILE_OPTION_SET=true; shift ;;
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
    --start-github-release|--publish) PUBLISH=true; MODE=release; CARGO_PROFILE=release; PROFILE_OPTION_SET=true; shift ;;
    --skip-build) SKIP_BUILD=true; shift ;;
    --preflight-only) PREFLIGHT_ONLY=true; shift ;;
    --no-sync) NO_SYNC=1; shift ;;
    --allow-concurrent-build) ALLOW_CONCURRENT_BUILD=true; shift ;;
    --restart-if-idle) RESTART_APP_SERVER=true; shift ;;
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
trap cleanup_build_process EXIT
if [[ -n "${PREFIX:-}" && -d "${PREFIX}/tmp" ]]; then
  ANDROID_PSI_NOTICE_DIR="$(mktemp -d "${PREFIX}/tmp/codex-android-psi-notice.XXXXXX")"
fi
if [[ "$(cd -- "${SOURCE_REPO}" && pwd -P)" == "$(cd -- "${BUILD_REPO}" && pwd -P)" ]]; then
  source_lock_backup_root="${PREFIX:+${PREFIX}/tmp}"
  [[ -d "${source_lock_backup_root}" ]] || source_lock_backup_root=/var/tmp
  source_lock_backup_candidate="$(mktemp "${source_lock_backup_root%/}/codex-source-Cargo.lock.XXXXXX")"
  if cp -p "${SOURCE_REPO}/codex-rs/Cargo.lock" "${source_lock_backup_candidate}"; then
    SOURCE_LOCK_BACKUP="${source_lock_backup_candidate}"
  else
    rm -f "${source_lock_backup_candidate}"
    die "unable to preserve source Cargo.lock before source-tree build"
  fi
fi

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

[[ "${PACKAGE_NPM}" != true || "${CARGO_PROFILE}" != dev-small ]] \
  || die "--dev-small cannot be used with npm packaging; select --debug or --release"

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
if [[ -n "${CARGO_BUILD_JOBS:-}" ]]; then
  export CARGO_BUILD_JOBS
fi
sync_sources
register_build_process
if [[ -n "${INSTALL_TARGETS}" ]]; then
  IFS=',' read -r -a INSTALL_TARGET_LIST <<<"${INSTALL_TARGETS}"
  [[ "${#INSTALL_TARGET_LIST[@]}" -gt 0 ]] || die "install target list must not be empty"
  for install_target_name in "${INSTALL_TARGET_LIST[@]}"; do
    install_target_host="${install_target_name%%:*}"
    [[ -n "${install_target_host}" ]] || die "install target list contains an empty target"
    if [[ "${install_target_name}" == *:* ]]; then
      install_target_profile="${install_target_name#*:}"
      [[ "${install_target_profile}" != *:* && -n "${install_target_profile}" ]] \
        || die "invalid install target specification: ${install_target_name}"
      case "${install_target_profile}" in
        dev|debug|dev-small|release) ;;
        *) die "unsupported install profile in ${install_target_name}; use dev, dev-small, or release" ;;
      esac
    fi
    [[ "${install_target_host}" == localhost ]] || require_cmd rsync
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
capture_build_provenance
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
      cargo_build "${MODE}" "${package_target}" "${CARGO_PROFILE}" >/dev/null
    fi
  done
elif [[ "${TARGET_MODE}" == android ]]; then
  build_android
else
  binary="$(cargo_build "${MODE}" "${TARGET_MODE}" "${CARGO_PROFILE}")"
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
