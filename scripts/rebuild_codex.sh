#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
RUST_WORKSPACE_DIR="${REPO_DIR}/codex-rs"
TOOLCHAIN_FILE="${RUST_WORKSPACE_DIR}/rust-toolchain.toml"
INSTALL_BIN="${HOME}/.cargo/bin/codex"
CARGO_LOCK_REL="codex-rs/Cargo.lock"
PUBLISH_TIMEOUT_MINUTES_DEFAULT=45
TRIAGE_STATE_FILE_DEFAULT="tmp/ci-triaged-tags.json"
PNPM_VERSION_DEFAULT="10.29.3"
NPM_VENDOR_DIR_DEFAULT="dist/npm-vendor"
NPM_X64_TARGET="x86_64-unknown-linux-musl"
NPM_ARMV7_TARGET="armv7-unknown-linux-gnueabihf"

RED=$'\033[31m'
BOLD=$'\033[1m'
RESET=$'\033[0m'

log_error() {
  echo "${BOLD}${RED}ERROR:${RESET} $*" >&2
}

restore_cargo_lock_if_needed() {
  git -C "${REPO_DIR}" checkout -- "./${CARGO_LOCK_REL}" >/dev/null 2>&1 || true
}
trap restore_cargo_lock_if_needed EXIT

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

is_only_cargo_lock_dirty() {
  local status_lines filtered_status
  # Ignore untracked files for publish gating. We only enforce tracked-tree
  # cleanliness here.
  status_lines="$(
    git -C "${REPO_DIR}" status --porcelain \
      | grep -Ev '^\?\?' || true
  )"
  if [[ -z "${status_lines}" ]]; then
    return 0
  fi
  filtered_status="$(
    grep -Ev \
      "^[ MARCUD?]{2} (${CARGO_LOCK_REL}|tmp(/.*)?|scripts/rebuild_codex\\.sh|scripts/ci_triage\\.sh)$" \
      <<<"${status_lines}" || true
  )"
  [[ -z "${filtered_status}" ]]
}

assert_publish_worktree_state() {
  if is_only_cargo_lock_dirty; then
    return 0
  fi
  echo "Working tree has tracked changes beyond allowed local-only files (${CARGO_LOCK_REL}, tmp/, scripts/rebuild_codex.sh, scripts/ci_triage.sh); refusing to publish." >&2
  git -C "${REPO_DIR}" status --short >&2
  exit 1
}

find_release_run_id() {
  local tag_name="$1"
  local min_created_at="$2"
  gh run list \
    --workflow custom-codex-release.yml \
    --branch "${tag_name}" \
    --event push \
    --limit 50 \
    --json databaseId,createdAt \
    | jq -r --arg min_created_at "${min_created_at}" '
        map(select(.createdAt >= $min_created_at))
        | sort_by(.createdAt)
        | reverse
        | .[0].databaseId // empty
      ' \
    | head -n 1
}

wait_for_run_to_appear() {
  local tag_name="$1"
  local timeout_secs="$2"
  local start_secs now elapsed run_id started_at last_log_secs
  start_secs="$(date +%s)"
  started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  last_log_secs=0
  echo "Wait start (UTC): ${started_at}. Polling for custom-codex-release run on tag branch ${tag_name}." >&2
  while true; do
    run_id="$(find_release_run_id "${tag_name}" "${started_at}" || true)"
    if [[ -n "${run_id}" ]]; then
      now="$(date +%s)"
      elapsed="$((now - start_secs))"
      echo "Matched workflow run ${run_id} after ${elapsed}s." >&2
      echo "${run_id}"
      return 0
    fi
    now="$(date +%s)"
    elapsed="$((now - start_secs))"
    if (( elapsed - last_log_secs >= 15 )); then
      echo "Still waiting for run on ${tag_name}... elapsed=${elapsed}s" >&2
      last_log_secs="${elapsed}"
    fi
    if (( elapsed > timeout_secs )); then
      echo "Timed out waiting for custom-codex-release run for ${tag_name}" >&2
      return 1
    fi
    sleep 5
  done
}

wait_for_run_completion() {
  local run_id="$1"
  local timeout_secs="$2"
  local start_secs now elapsed status conclusion url started_at last_log_secs
  local run_json activity active_jobs prev_activity
  start_secs="$(date +%s)"
  started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  last_log_secs=0
  prev_activity=""
  echo "Completion wait start (UTC): ${started_at} for run=${run_id}"
  while true; do
    run_json="$(gh run view "${run_id}" --json status,conclusion,url,jobs)"
    status="$(jq -r '.status' <<<"${run_json}")"
    conclusion="$(jq -r '.conclusion // ""' <<<"${run_json}")"
    url="$(jq -r '.url' <<<"${run_json}")"
    now="$(date +%s)"
    elapsed="$((now - start_secs))"
    activity="$(jq -r '
      .jobs as $jobs
      | ([$jobs[] | select(.status == "completed" and .completedAt != null)
          | {t: .completedAt, msg: (.name + " -> " + (.conclusion // ""))}]
         | sort_by(.t) | last // {t: "", msg: ""}) as $done
      | ([$jobs[] | select(.status != "completed" and .startedAt != null)
          | {t: .startedAt, msg: (.name + " -> " + .status)}]
         | sort_by(.t) | last // {t: "", msg: ""}) as $active
      | if $done.t != "" and ($active.t == "" or $done.t > $active.t) then
          $done.msg
        elif $active.t != "" then
          $active.msg
        else
          "queued"
        end
    ' <<<"${run_json}")"
    active_jobs="$(jq -r '
      [.jobs[] | select(.status != "completed") | .name] | join(", ")
    ' <<<"${run_json}")"
    if [[ -z "${active_jobs}" ]]; then
      active_jobs="none"
    fi
    if (( elapsed - last_log_secs >= 20 || elapsed == 0 )); then
      if [[ "${activity}" != "${prev_activity}" ]]; then
        echo "run=${run_id} elapsed=${elapsed}s status=${status} conclusion=${conclusion} active=${active_jobs} last_activity=${activity} url=${url}"
        prev_activity="${activity}"
      else
        echo "run=${run_id} elapsed=${elapsed}s status=${status} conclusion=${conclusion} active=${active_jobs} url=${url}"
      fi
      last_log_secs="${elapsed}"
    fi
    if [[ "${status}" == "completed" ]]; then
      if [[ "${conclusion}" != "success" ]]; then
        echo "Release workflow failed: ${url}" >&2
        return 1
      fi
      return 0
    fi

    if (( elapsed > timeout_secs )); then
      echo "Timed out waiting for run ${run_id} to complete (${url})" >&2
      return 1
    fi
    sleep 10
  done
}

assert_release_assets() {
  local tag_name="$1"
  local version="$2"
  local assets missing
  assets="$(gh release view "${tag_name}" --json assets --jq '.assets[].name')"
  missing=0
  for asset in \
    "codex-npm-${version}.tgz" \
    "codex-npm-linux-x64-${version}.tgz" \
    "codex-npm-linux-armv7-${version}.tgz"; do
    if ! grep -Fxq "${asset}" <<<"${assets}"; then
      echo "Missing GitHub release asset: ${asset}" >&2
      missing=1
    fi
  done
  if (( missing != 0 )); then
    echo "Current assets for ${tag_name}:" >&2
    echo "${assets}" >&2
    return 1
  fi
}

assert_npm_linux_tags() {
  local timeout_secs="$1"
  local start_secs now elapsed tags_json has_linux_x64 has_linux_armv7
  start_secs="$(date +%s)"
  while true; do
    if tags_json="$(npm view @reb.ai/codex dist-tags --json 2>/dev/null)"; then
      has_linux_x64="$(jq -r 'has("linux-x64")' <<<"${tags_json}")"
      has_linux_armv7="$(jq -r 'has("linux-armv7")' <<<"${tags_json}")"
      if [[ "${has_linux_x64}" == "true" && "${has_linux_armv7}" == "true" ]]; then
        echo "npm dist-tags: ${tags_json}"
        return 0
      fi
      echo "Waiting for npm linux tags. Current dist-tags: ${tags_json}"
    else
      echo "Waiting for @reb.ai/codex visibility on npm..."
    fi

    now="$(date +%s)"
    elapsed="$((now - start_secs))"
    if (( elapsed > timeout_secs )); then
      echo "Timed out waiting for npm linux-x64/linux-armv7 dist-tags on @reb.ai/codex" >&2
      return 1
    fi
    sleep 15
  done
}

run_release_build_with_locked_fallback() {
  local build_log
  local -a cargo_env
  cargo_env=(RUSTUP_DISABLE_SELF_UPDATE=1 CARGO_INCREMENTAL=1)
  if [[ -n "${BUILD_JOBS}" ]]; then
    cargo_env+=(CARGO_BUILD_JOBS="${BUILD_JOBS}")
  fi
  if [[ "${FAST_RELEASE_BUILD}" == "true" ]]; then
    cargo_env+=(
      CARGO_PROFILE_RELEASE_LTO=thin
      CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
    )
  fi
  build_log="$(mktemp)"
  set +e
  env "${cargo_env[@]}" cargo +"${TOOLCHAIN}" build -p codex-cli --release --locked 2>&1 | tee "${build_log}"
  local status=${PIPESTATUS[0]}
  set -e

  if (( status == 0 )); then
    rm -f "${build_log}"
    return 0
  fi

  if grep -q "cannot update the lock file .*Cargo.lock because --locked was passed" "${build_log}"; then
    echo "Locked build needs lockfile regeneration; retrying release build without --locked."
    rm -f "${build_log}"
    env "${cargo_env[@]}" cargo +"${TOOLCHAIN}" build -p codex-cli --release
    return 0
  fi

  rm -f "${build_log}"
  return "${status}"
}

run_target_build_with_locked_fallback() {
  local target="$1"
  local profile="$2"
  local build_log
  local -a cargo_env
  cargo_env=(RUSTUP_DISABLE_SELF_UPDATE=1 CARGO_INCREMENTAL=1)
  if [[ -n "${BUILD_JOBS}" ]]; then
    cargo_env+=(CARGO_BUILD_JOBS="${BUILD_JOBS}")
  fi
  if [[ "${FAST_RELEASE_BUILD}" == "true" ]]; then
    cargo_env+=(
      CARGO_PROFILE_RELEASE_LTO=thin
      CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
    )
  fi
  build_log="$(mktemp)"
  set +e
  local -a cargo_cmd
  cargo_cmd=(cargo +"${TOOLCHAIN}" build -p codex-cli --target "${target}" --locked)
  if [[ "${profile}" == "release" ]]; then
    cargo_cmd+=(--release)
  fi
  env "${cargo_env[@]}" "${cargo_cmd[@]}" 2>&1 | tee "${build_log}"
  local status=${PIPESTATUS[0]}
  set -e

  if (( status == 0 )); then
    rm -f "${build_log}"
    return 0
  fi

  if grep -q "cannot update the lock file .*Cargo.lock because --locked was passed" "${build_log}"; then
    echo "Locked build for ${target} (${profile}) needs lockfile regeneration; retrying without --locked."
    rm -f "${build_log}"
    cargo_cmd=(cargo +"${TOOLCHAIN}" build -p codex-cli --target "${target}")
    if [[ "${profile}" == "release" ]]; then
      cargo_cmd+=(--release)
    fi
    env "${cargo_env[@]}" "${cargo_cmd[@]}"
    return 0
  fi

  rm -f "${build_log}"
  return "${status}"
}

run_armv7_build() {
  local mode="$1"
  local -a armv7_cmd
  armv7_cmd=(
    "${REPO_DIR}/scripts/build_armv7.sh"
    "--${mode}"
    "--build-env=${ARMV7_BUILD_ENV}"
    "--rusty-v8-release-repo=${RUSTY_V8_RELEASE_REPO}"
  )
  if [[ -n "${RUSTY_V8_RELEASE_TAG}" ]]; then
    armv7_cmd+=("--rusty-v8-release-tag=${RUSTY_V8_RELEASE_TAG}")
  fi
  if [[ "${ARMV7_EPHEMERAL}" == "true" ]]; then
    armv7_cmd+=(--ephemeral)
  fi
  if [[ -n "${ARMV7_TARGET}" ]]; then
    armv7_cmd+=("--target=${ARMV7_TARGET}")
  fi
  if [[ "${ARMV7_PUBLISH_GITHUB}" == "true" ]]; then
    armv7_cmd+=(--publish-github)
  else
    armv7_cmd+=(--no-publish-github)
  fi
  if [[ -n "${ARMV7_GITHUB_RELEASE_REPO}" ]]; then
    armv7_cmd+=("--github-release-repo=${ARMV7_GITHUB_RELEASE_REPO}")
  fi
  if [[ -n "${ARMV7_GITHUB_RELEASE_TAG}" ]]; then
    armv7_cmd+=("--github-release-tag=${ARMV7_GITHUB_RELEASE_TAG}")
  fi
  "${armv7_cmd[@]}"
}

prepare_npm_vendor_rg_binary() {
  local vendor_root="$1"
  local target="$2"
  local platform_key="$3"
  REPO_DIR="${REPO_DIR}" RG_VENDOR_ROOT="${vendor_root}" RG_TARGET="${target}" RG_PLATFORM_KEY="${platform_key}" python3 - <<'PY'
import json
import os
import shutil
import tarfile
import zipfile
from pathlib import Path
import urllib.request

repo_root = Path(os.environ["REPO_DIR"])
platform_key = os.environ["RG_PLATFORM_KEY"]
vendor_root = Path(os.environ["RG_VENDOR_ROOT"])
target = os.environ["RG_TARGET"]

manifest_text = (repo_root / "codex-cli" / "bin" / "rg").read_text(encoding="utf-8")
manifest = json.loads(manifest_text[manifest_text.index("{"):])
platform = manifest["platforms"].get(platform_key)
if platform is None:
    raise RuntimeError(f"rg manifest missing platform key: {platform_key}")

url = platform["providers"][0]["url"]
member_path = platform["path"]
archive_format = platform.get("format", "tar.gz")

archive_path = vendor_root / "rg-archive"
with urllib.request.urlopen(url) as response, open(archive_path, "wb") as out:
    shutil.copyfileobj(response, out)

dest_dir = vendor_root / target / "path"
dest_dir.mkdir(parents=True, exist_ok=True)
dest = dest_dir / "rg"

if archive_format == "tar.gz":
    with tarfile.open(archive_path, "r:gz") as tar:
        member = tar.getmember(member_path)
        with tar.extractfile(member) as src, open(dest, "wb") as out:
            shutil.copyfileobj(src, out)
elif archive_format == "zip":
    with zipfile.ZipFile(archive_path) as zf:
        with zf.open(member_path) as src, open(dest, "wb") as out:
            shutil.copyfileobj(src, out)
else:
    raise RuntimeError(f"Unsupported rg archive format: {archive_format}")

dest.chmod(0o755)
PY
}

stage_npm_vendor_from_binaries() {
  local version="$1"
  local vendor_root="$2"
  local profile="$3"
  local x64_bin armv7_bin
  x64_bin="${RUST_WORKSPACE_DIR}/target/${NPM_X64_TARGET}/${profile}/codex"
  armv7_bin="${RUST_WORKSPACE_DIR}/target/${NPM_ARMV7_TARGET}/${profile}/codex"

  if [[ ! -x "${x64_bin}" || ! -x "${armv7_bin}" ]]; then
    echo "Missing required release binaries for npm vendor staging." >&2
    if [[ ! -x "${x64_bin}" ]]; then
      echo "- Missing ${x64_bin}" >&2
    fi
    if [[ ! -x "${armv7_bin}" ]]; then
      echo "- Missing ${armv7_bin}" >&2
    fi
    echo "Build them with:" >&2
    echo "  ./scripts/rebuild_codex.sh --release --build-npm-vendor" >&2
    return 1
  fi

  mkdir -p "${vendor_root}/${NPM_X64_TARGET}/codex"
  mkdir -p "${vendor_root}/${NPM_X64_TARGET}/path"
  mkdir -p "${vendor_root}/${NPM_ARMV7_TARGET}/codex"

  install -m 0755 "${x64_bin}" "${vendor_root}/${NPM_X64_TARGET}/codex/codex"
  install -m 0755 "${armv7_bin}" "${vendor_root}/${NPM_ARMV7_TARGET}/codex/codex"

  echo "Fetching rg payload for ${NPM_X64_TARGET}..."
  prepare_npm_vendor_rg_binary "${vendor_root}" "${NPM_X64_TARGET}" "linux-x64"

  echo "Prepared npm vendor root for ${version} (${profile}): ${vendor_root}"
}

run_ci_preflight_checks() {
  local preflight_ts preflight_dir shear_log shear_errors
  preflight_ts="$(date -u +%Y%m%dT%H%M%SZ)"
  preflight_dir="${REPO_DIR}/tmp/preflight/${preflight_ts}"
  mkdir -p "${preflight_dir}"
  echo "[preflight] Artifacts directory: ${preflight_dir}"

  require_cmd just
  init_pnpm

  if [[ ! -d "${REPO_DIR}/node_modules" ]]; then
    echo "[preflight] Installing JS dependencies with pnpm..."
    (
      cd "${REPO_DIR}"
      run_pnpm install --frozen-lockfile
    )
  fi

  echo "[preflight] Checking docs/JS formatting (pnpm run format)..."
  if ! (
    cd "${REPO_DIR}"
    run_pnpm run format
  ); then
    echo "[preflight] Formatting drift detected; applying pnpm run format:fix..."
    (
      cd "${REPO_DIR}"
      run_pnpm run format:fix
    )
  fi

  echo "[preflight] Checking Rust formatting (cargo fmt --check)..."
  if ! (
    cd "${RUST_WORKSPACE_DIR}"
    cargo +"${TOOLCHAIN}" fmt -- --config imports_granularity=Item --check
  ); then
    echo "[preflight] Rust formatting drift detected; applying cargo fmt..."
    (
      cd "${RUST_WORKSPACE_DIR}"
      cargo +"${TOOLCHAIN}" fmt -- --config imports_granularity=Item
    )
  fi

  echo "[preflight] Running argument comment lint..."
  (
    cd "${REPO_DIR}"
    just argument-comment-lint
  )

  if ! command -v cargo-shear >/dev/null 2>&1; then
    echo "[preflight] Installing cargo-shear..."
    cargo +"${TOOLCHAIN}" install --locked cargo-shear
  fi

  echo "[preflight] Running cargo shear..."
  shear_log="${preflight_dir}/cargo-shear.log"
  shear_errors="${preflight_dir}/cargo-shear-errors.txt"
  shear_fix_log="${preflight_dir}/cargo-shear-fix.log"
  set +e
  (
    cd "${RUST_WORKSPACE_DIR}"
    cargo +"${TOOLCHAIN}" shear 2>&1
  ) | tee "${shear_log}"
  local shear_status=${PIPESTATUS[0]}
  set -e

  awk '
    /^shear\/[a-z0-9_]+/ {section=$0; mode=0}
    /^[[:space:]]*× / {
      print section
      print $0
      mode=1
      next
    }
    mode==1 {
      if ($0 ~ /^[[:space:]]*$/) {
        print ""
        mode=0
      } else {
        print $0
      }
    }
  ' "${shear_log}" > "${shear_errors}"

  if [[ -s "${shear_errors}" ]]; then
    echo "[preflight] cargo-shear actionable errors: ${shear_errors}"
  else
    echo "[preflight] cargo-shear found no actionable '×' errors."
  fi
  echo "[preflight] cargo-shear full log: ${shear_log}"

  if (( shear_status != 0 )) || [[ "${SHEAR_FIX_MODE}" == "always" ]]; then
    if (( shear_status != 0 )); then
      echo "[preflight] cargo-shear reported errors; attempting automatic fix with --fix..."
    else
      echo "[preflight] --fix enabled; running cargo shear --fix..."
    fi
    set +e
    (
      cd "${RUST_WORKSPACE_DIR}"
      cargo +"${TOOLCHAIN}" shear --fix 2>&1
    ) | tee "${shear_fix_log}"
    local shear_fix_status=${PIPESTATUS[0]}
    set -e
    echo "[preflight] cargo-shear fix log: ${shear_fix_log}"
    if (( shear_fix_status != 0 )); then
      return "${shear_fix_status}"
    fi

    echo "[preflight] Re-running cargo shear after automatic fixes..."
    set +e
    (
      cd "${RUST_WORKSPACE_DIR}"
      cargo +"${TOOLCHAIN}" shear 2>&1
    ) | tee "${shear_log}"
    shear_status=${PIPESTATUS[0]}
    set -e

    awk '
      /^shear\/[a-z0-9_]+/ {section=$0; mode=0}
      /^[[:space:]]*× / {
        print section
        print $0
        mode=1
        next
      }
      mode==1 {
        if ($0 ~ /^[[:space:]]*$/) {
          print ""
          mode=0
        } else {
          print $0
        }
      }
    ' "${shear_log}" > "${shear_errors}"

    if [[ -s "${shear_errors}" ]]; then
      echo "[preflight] cargo-shear actionable errors remain: ${shear_errors}"
    else
      echo "[preflight] cargo-shear actionable errors resolved."
    fi
    echo "[preflight] cargo-shear full log: ${shear_log}"
    if (( shear_status != 0 )); then
      return "${shear_status}"
    fi
  fi
}

commit_preflight_changes_if_needed() {
  local status_lines commit_message
  status_lines="$(git -C "${REPO_DIR}" status --porcelain)"
  if [[ -z "${status_lines}" ]]; then
    return 0
  fi
  if is_only_cargo_lock_dirty; then
    return 0
  fi
  if [[ "${AUTO_COMMIT_PREFLIGHT_FIXES}" != "true" ]]; then
    return 1
  fi
  echo "Preflight introduced tracked changes; committing them before publish."
  git -C "${REPO_DIR}" add -u
  if git -C "${REPO_DIR}" diff --cached --quiet; then
    return 0
  fi
  commit_message="chore: apply rebuild_codex preflight fixes for ${VERSION}"
  git -C "${REPO_DIR}" commit -m "${commit_message}"
}

init_pnpm() {
  if command -v pnpm >/dev/null 2>&1; then
    PNPM_CMD=(pnpm)
    return 0
  fi

  if ! command -v corepack >/dev/null 2>&1; then
    log_error "pnpm is missing and corepack is not installed."
    log_error "Install pnpm ${PNPM_VERSION_DEFAULT}+ or install Node.js with corepack enabled."
    return 1
  fi

  echo "[preflight] pnpm not found; installing pnpm ${PNPM_VERSION_DEFAULT} via corepack..."
  mkdir -p "${HOME}/.cache/node/corepack/v1"
  if ! corepack prepare "pnpm@${PNPM_VERSION_DEFAULT}" --activate; then
    log_error "Failed to install pnpm via corepack."
    return 1
  fi

  if command -v pnpm >/dev/null 2>&1; then
    PNPM_CMD=(pnpm)
  else
    # Some environments cannot create a global pnpm shim; corepack pnpm still works.
    PNPM_CMD=(corepack pnpm)
  fi
}

run_pnpm() {
  "${PNPM_CMD[@]}" "$@"
}

read_workspace_version() {
  RUST_WORKSPACE_DIR="${RUST_WORKSPACE_DIR}" python3 - <<'PY'
import os
import tomllib
from pathlib import Path

toml_path = Path(os.environ["RUST_WORKSPACE_DIR"]) / "Cargo.toml"
data = tomllib.loads(toml_path.read_text(encoding="utf-8"))
version = data.get("workspace", {}).get("package", {}).get("version")
if version:
    print(version)
PY
}

resolve_publish_version() {
  local version version_from_codex
  version="$(read_workspace_version)"
  if [[ -z "${version}" ]]; then
    version_from_codex="$("${INSTALL_BIN}" --version | awk '{print $2}')"
    if [[ "${version_from_codex}" =~ ^(.+)-([0-9]{8,})$ ]]; then
      version="${BASH_REMATCH[1]}"
    else
      version="${version_from_codex}"
    fi
  fi
  echo "${version}"
}

is_commit_preflight_passed() {
  local commit_sha="$1"
  local version="$2"
  [[ -f "${TRIAGE_STATE_FILE}" ]] || return 1
  jq -e \
    --arg commit_sha "${commit_sha}" \
    --arg version "${version}" \
    '
      .commits[$commit_sha] as $record
      | $record != null
      | if . then
          (($record.version // "") == "" or $record.version == $version)
        else
          false
        end
    ' "${TRIAGE_STATE_FILE}" >/dev/null 2>&1
}

is_tree_preflight_passed() {
  local tree_hash="$1"
  local version="$2"
  [[ -f "${TRIAGE_STATE_FILE}" ]] || return 1
  jq -e \
    --arg tree_hash "${tree_hash}" \
    --arg version "${version}" \
    '
      .trees[$tree_hash] as $record
      | $record != null
      | if . then
          (($record.version // "") == "" or $record.version == $version)
        else
          false
        end
    ' "${TRIAGE_STATE_FILE}" >/dev/null 2>&1
}

ensure_triage_state_file() {
  mkdir -p "$(dirname "${TRIAGE_STATE_FILE}")"
  if [[ ! -f "${TRIAGE_STATE_FILE}" ]]; then
    cat > "${TRIAGE_STATE_FILE}" <<'EOF'
{"version":1,"commits":{},"trees":{}}
EOF
    return 0
  fi
  if jq -e '.commits == null or .trees == null' "${TRIAGE_STATE_FILE}" >/dev/null 2>&1; then
    local tmp
    tmp="$(mktemp)"
    jq '.version = 1 | .commits = (.commits // {}) | .trees = (.trees // {})' "${TRIAGE_STATE_FILE}" > "${tmp}"
    mv "${tmp}" "${TRIAGE_STATE_FILE}"
  fi
}

record_preflight_success_for_commit_and_tree() {
  local commit_sha="$1"
  local tree_hash="$2"
  local version="$3"
  local now tmp
  ensure_triage_state_file
  now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  tmp="$(mktemp)"
  jq \
    --arg commit_sha "${commit_sha}" \
    --arg tree_hash "${tree_hash}" \
    --arg now "${now}" \
    --arg version "${version}" \
    '
      .version = 1
      | .commits = (.commits // {})
      | .trees = (.trees // {})
      | .commits[$commit_sha] = {
          markedAt: $now,
          version: $version,
          treeHash: $tree_hash,
          source: "rebuild_codex_preflight"
        }
      | .trees[$tree_hash] = {
          markedAt: $now,
          version: $version,
          commitSha: $commit_sha,
          source: "rebuild_codex_preflight"
        }
    ' "${TRIAGE_STATE_FILE}" > "${tmp}"
  mv "${tmp}" "${TRIAGE_STATE_FILE}"
}

MODE="debug"
PUBLISH="auto"
PUBLISH_MODE="github"
PREFLIGHT_ONLY="false"
ARMV7_ONLY="false"
ARMV7_BUILD_ENV="auto"
ARMV7_EPHEMERAL="false"
ARMV7_TARGET=""
ARMV7_PUBLISH_GITHUB="false"
ARMV7_GITHUB_RELEASE_REPO=""
ARMV7_GITHUB_RELEASE_TAG=""
REGEN_SCHEMA="auto"
CI_PREFLIGHT="auto"
FORCE_TAG="true"
AUTO_COMMIT_PREFLIGHT_FIXES="true"
PUBLISH_TIMEOUT_MINUTES="${PUBLISH_TIMEOUT_MINUTES_DEFAULT}"
SHEAR_FIX_MODE="on_error"
BUILD_JOBS=""
FAST_RELEASE_BUILD="false"
BUILD_NPM_VENDOR="false"
NPM_VENDOR_DIR=""
RUSTY_V8_RELEASE_REPO="${RUSTY_V8_RELEASE_REPO:-rebroad/rusty_v8}"
RUSTY_V8_RELEASE_TAG="${RUSTY_V8_RELEASE_TAG:-}"
NPM_PUBLISH_DRY_RUN="false"
SCHEMA_HASH_FILE="${REPO_DIR}/codex-rs/target/app-server-schema.hash"
TRIAGE_STATE_FILE="${REPO_DIR}/${TRIAGE_STATE_FILE_DEFAULT}"
for arg in "$@"; do
  case "${arg}" in
    --debug)
      MODE="debug"
      ;;
    --release)
      MODE="release"
      ;;
    --publish)
      PUBLISH="true"
      ;;
    --publish-npm)
      PUBLISH="true"
      PUBLISH_MODE="npm"
      ;;
    --no-publish)
      PUBLISH="false"
      ;;
    --no-force-tag)
      FORCE_TAG="false"
      ;;
    --fix)
      SHEAR_FIX_MODE="always"
      CI_PREFLIGHT="true"
      ;;
    --auto-commit-fixes)
      AUTO_COMMIT_PREFLIGHT_FIXES="true"
      ;;
    --no-auto-commit-fixes)
      AUTO_COMMIT_PREFLIGHT_FIXES="false"
      ;;
    --regen-schema)
      REGEN_SCHEMA="true"
      ;;
    --no-regen-schema)
      REGEN_SCHEMA="false"
      ;;
    --preflight|--ci-preflight)
      CI_PREFLIGHT="true"
      ;;
    --preflight-only)
      PREFLIGHT_ONLY="true"
      CI_PREFLIGHT="true"
      ;;
    --no-ci-preflight)
      CI_PREFLIGHT="false"
      ;;
    --armv7)
      ARMV7_ONLY="true"
      ;;
    --armv7-build-env=*)
      ARMV7_BUILD_ENV="${arg#*=}"
      ;;
    --armv7-ephemeral)
      ARMV7_EPHEMERAL="true"
      ;;
    --armv7-target=*)
      ARMV7_TARGET="${arg#*=}"
      ;;
    --armv7-publish-github)
      ARMV7_PUBLISH_GITHUB="true"
      ;;
    --armv7-no-publish-github)
      ARMV7_PUBLISH_GITHUB="false"
      ;;
    --armv7-github-release-repo=*)
      ARMV7_GITHUB_RELEASE_REPO="${arg#*=}"
      ;;
    --armv7-github-release-tag=*)
      ARMV7_GITHUB_RELEASE_TAG="${arg#*=}"
      ;;
    --publish-timeout-minutes=*)
      PUBLISH_TIMEOUT_MINUTES="${arg#*=}"
      ;;
    --jobs=*)
      BUILD_JOBS="${arg#*=}"
      ;;
    --fast-release-build)
      FAST_RELEASE_BUILD="true"
      ;;
    --build-npm-vendor)
      BUILD_NPM_VENDOR="true"
      ;;
    --build-npm-targets)
      BUILD_NPM_VENDOR="true"
      ;;
    --prepare-npm-vendor)
      BUILD_NPM_VENDOR="true"
      ;;
    --npm-vendor-dir=*)
      NPM_VENDOR_DIR="${arg#*=}"
      ;;
    --rusty-v8-release-repo=*)
      RUSTY_V8_RELEASE_REPO="${arg#*=}"
      ;;
    --rusty-v8-release-tag=*)
      RUSTY_V8_RELEASE_TAG="${arg#*=}"
      ;;
    --npm-publish-dry-run)
      NPM_PUBLISH_DRY_RUN="true"
      ;;
    --triage-state-file=*)
      TRIAGE_STATE_FILE="${arg#*=}"
      ;;
    -h|--help)
      cat <<'EOF'
Usage: rebuild_codex.sh [--release] [--publish|--no-publish] [--publish-timeout-minutes=N]

Default behavior:
- Regenerate app-server schema (just write-app-server-schema)
- Build debug codex, copy it to ~/.cargo/bin/codex
- Remove workspace codex build artifacts under target/
- Skip CI preflight checks unless publishing
- Record successful preflight checks by content hash (git tree hash) for future publish skip decisions

Options:
  --release   Build/install release codex into ~/.cargo/bin/codex, then clean target binaries
  --publish   Create + push a git tag for the workspace version (codex-vX.Y.Z[-...])
  --publish-npm
             Publish directly to npm locally (no GitHub tag/workflow publish path)
  --no-publish
             Skip tag/push even in release mode
  --preflight, --ci-preflight
             Run CI-like preflight checks (pnpm format, cargo fmt, argument-comment-lint, cargo shear)
  --preflight-only
             Run CI preflight checks and exit before schema/build/install/publish steps
  --no-ci-preflight
             Skip preflight checks even when publishing
  --armv7
             Build armv7 codex via scripts/build_armv7.sh and exit (reuses armv7 script behavior)
  --armv7-build-env=<auto|host|docker-buster>
             Forward armv7 build environment mode (default: auto)
  --armv7-ephemeral
             Forward ephemeral Docker mode to armv7 build
  --armv7-target=<triple>
             Forward target override to armv7 build
  --armv7-publish-github
             Forward GitHub artifact publish to armv7 build
  --armv7-no-publish-github
             Disable GitHub artifact publish for armv7 build (default)
  --armv7-github-release-repo=<owner/repo>
             Forward armv7 GitHub release repo
  --armv7-github-release-tag=<tag>
             Forward armv7 GitHub release tag
  --no-force-tag
             Do not replace existing tags (default is to replace)
  --fix
             Force CI preflight and always run cargo shear --fix before publish checks
  --auto-commit-fixes
             Auto-commit tracked preflight-applied fixes before publishing (default)
  --no-auto-commit-fixes
             Do not auto-commit preflight fixes; fail if tracked changes remain
  --regen-schema
             Force schema regeneration
  --no-regen-schema
             Skip schema regeneration
  --publish-timeout-minutes=N
             Timeout in minutes for release workflow/npm verification (default: 45)
  --jobs=N
             Set Cargo build parallelism via CARGO_BUILD_JOBS
  --fast-release-build
             Faster release compile (LTO=thin, codegen-units=16) with potential runtime perf tradeoff
  --build-npm-vendor
             Build + stage npm vendor payload for linux x64 + armv7 using current mode (--debug or --release)
  --npm-vendor-dir=<path>
             Override npm vendor output directory when using --build-npm-vendor
  --rusty-v8-release-repo=<owner/repo>
             rusty_v8 release repo for armv7 build (default: rebroad/rusty_v8)
  --rusty-v8-release-tag=<tag>
             rusty_v8 release tag for armv7 build (default: resolved from crate version)
  --npm-publish-dry-run
             With --publish-npm, run npm publish in --dry-run mode
  --triage-state-file=<path>
             Local triage registry path keyed by content hash (default: tmp/ci-triaged-tags.json)
EOF
      exit 0
      ;;
    *)
      echo "Unknown argument: ${arg}"
      echo "Run with --help for usage."
      exit 1
      ;;
  esac
done

if [[ -n "${BUILD_JOBS}" ]] && ! [[ "${BUILD_JOBS}" =~ ^[1-9][0-9]*$ ]]; then
  echo "Invalid --jobs value: ${BUILD_JOBS} (must be a positive integer)." >&2
  exit 1
fi
if [[ "${PREFLIGHT_ONLY}" == "true" && "${CI_PREFLIGHT}" == "false" ]]; then
  echo "--preflight-only cannot be combined with --no-ci-preflight." >&2
  exit 1
fi

cd "${REPO_DIR}"
if [[ "${ARMV7_ONLY}" == "true" && "${PREFLIGHT_ONLY}" == "true" ]]; then
  echo "--armv7 cannot be combined with --preflight-only." >&2
  exit 1
fi

schema_should_run="false"
if [[ "${REGEN_SCHEMA}" == "true" ]]; then
  schema_should_run="true"
elif [[ "${REGEN_SCHEMA}" != "false" ]]; then
  schema_should_run="true"
fi
if [[ "${PREFLIGHT_ONLY}" == "true" ]]; then
  schema_should_run="false"
fi

if [[ "${schema_should_run}" == "true" ]]; then
  schema_hash="$(
    find "${REPO_DIR}/codex-rs/app-server" "${REPO_DIR}/codex-rs/app-server-protocol" \
      -type f -print0 \
      | LC_ALL=C sort -z \
      | xargs -0 sha256sum \
      | sha256sum \
      | awk '{print $1}'
  )"
  previous_hash=""
  if [[ -f "${SCHEMA_HASH_FILE}" ]]; then
    previous_hash="$(cat "${SCHEMA_HASH_FILE}")"
  fi
  if [[ -n "${schema_hash}" && "${schema_hash}" == "${previous_hash}" ]]; then
    echo "[1/4] Skipping schema regeneration (no source changes detected)..."
  else
    echo "[1/4] Regenerating app-server schema..."
    just write-app-server-schema
    mkdir -p "$(dirname "${SCHEMA_HASH_FILE}")"
    echo "${schema_hash}" > "${SCHEMA_HASH_FILE}"
  fi
else
  echo "[1/4] Skipping schema regeneration (forced off)..."
fi

cd "${RUST_WORKSPACE_DIR}"
if [[ -f "${HOME}/.cargo/env" ]]; then
  # Ensure rustup-managed cargo/rustc are available in this shell.
  # shellcheck disable=SC1090
  source "${HOME}/.cargo/env"
fi

TOOLCHAIN="$(sed -n 's/^channel = "\(.*\)"/\1/p' "${TOOLCHAIN_FILE}")"
if [[ -z "${TOOLCHAIN}" ]]; then
  echo "Unable to read pinned Rust toolchain from ${TOOLCHAIN_FILE}"
  exit 1
fi

if ! rustup toolchain list | grep -q "^${TOOLCHAIN}-"; then
  echo "Installing pinned Rust toolchain ${TOOLCHAIN}..."
  rustup toolchain install "${TOOLCHAIN}" \
    --component clippy \
    --component rustfmt \
    --component rust-src
fi

should_publish="false"
if [[ "${MODE}" == "release" && "${PUBLISH}" != "false" ]]; then
  should_publish="true"
elif [[ "${PUBLISH}" == "true" ]]; then
  should_publish="true"
fi

publish_to_github="false"
publish_to_npm="false"
if [[ "${should_publish}" == "true" ]]; then
  case "${PUBLISH_MODE}" in
    github)
      publish_to_github="true"
      ;;
    npm)
      publish_to_npm="true"
      BUILD_NPM_VENDOR="true"
      MODE="release"
      ;;
    *)
      echo "Unknown publish mode: ${PUBLISH_MODE}" >&2
      exit 1
      ;;
  esac
fi
if [[ "${PREFLIGHT_ONLY}" == "true" && "${should_publish}" == "true" ]]; then
  echo "--preflight-only cannot be combined with publish options." >&2
  exit 1
fi

if [[ "${publish_to_github}" == "true" ]]; then
  require_cmd gh
  require_cmd jq
  require_cmd npm
elif [[ "${publish_to_npm}" == "true" ]]; then
  require_cmd npm
fi

HEAD_COMMIT_SHA="$(git -C "${REPO_DIR}" rev-parse HEAD)"
HEAD_TREE_HASH="$(git -C "${REPO_DIR}" rev-parse HEAD^{tree})"
VERSION=""
TAG=""
if [[ "${should_publish}" == "true" ]]; then
  VERSION="$(resolve_publish_version)"
  if [[ -z "${VERSION}" ]]; then
    echo "Unable to read workspace version from ${RUST_WORKSPACE_DIR}/Cargo.toml or ${INSTALL_BIN} --version"
    exit 1
  fi
  if [[ "${publish_to_github}" == "true" ]]; then
    TAG="codex-v${VERSION}"
  fi
fi

if [[ "${BUILD_NPM_VENDOR}" == "true" && -z "${VERSION}" ]]; then
  VERSION="$(resolve_publish_version)"
  if [[ -z "${VERSION}" ]]; then
    echo "Unable to read workspace version from ${RUST_WORKSPACE_DIR}/Cargo.toml or ${INSTALL_BIN} --version"
    exit 1
  fi
fi

run_ci_preflight="false"
if [[ "${CI_PREFLIGHT}" == "true" ]]; then
  run_ci_preflight="true"
elif [[ "${CI_PREFLIGHT}" != "false" && "${should_publish}" == "true" ]]; then
  if [[ -n "${VERSION}" ]] && is_tree_preflight_passed "${HEAD_TREE_HASH}" "${VERSION}"; then
    echo "Skipping CI preflight checks for tree ${HEAD_TREE_HASH} (version ${VERSION}) from ${TRIAGE_STATE_FILE}."
  elif [[ -n "${VERSION}" ]] && is_commit_preflight_passed "${HEAD_COMMIT_SHA}" "${VERSION}"; then
    # Backward compatibility for previously recorded entries.
    echo "Skipping CI preflight checks for commit ${HEAD_COMMIT_SHA} (version ${VERSION}) from ${TRIAGE_STATE_FILE}."
  else
    run_ci_preflight="true"
  fi
fi

if [[ "${run_ci_preflight}" == "true" ]]; then
  echo
  echo "${BOLD}=== CI PREFLIGHT START ===${RESET}"
  if ! run_ci_preflight_checks; then
    echo "${BOLD}${RED}=== CI PREFLIGHT FAILED ===${RESET}" >&2
    log_error "Fix the preflight issue above, then re-run rebuild_codex.sh."
    exit 1
  fi
  echo "${BOLD}=== CI PREFLIGHT PASSED ===${RESET}"
  echo "CI preflight checks completed."
  if [[ "${should_publish}" == "true" ]]; then
    if ! commit_preflight_changes_if_needed; then
      echo "Working tree has tracked changes after preflight and auto-commit is disabled." >&2
    fi
    assert_publish_worktree_state

    # Preflight may have committed tracked fixes; build/publish from that tree.
    HEAD_COMMIT_SHA="$(git -C "${REPO_DIR}" rev-parse HEAD)"
    HEAD_TREE_HASH="$(git -C "${REPO_DIR}" rev-parse HEAD^{tree})"
    VERSION="$(resolve_publish_version)"
    TAG="codex-v${VERSION}"
  fi

  if [[ -n "${VERSION}" ]]; then
    record_preflight_success_for_commit_and_tree "${HEAD_COMMIT_SHA}" "${HEAD_TREE_HASH}" "${VERSION}"
    echo "Recorded successful preflight for commit ${HEAD_COMMIT_SHA} and tree ${HEAD_TREE_HASH} (version ${VERSION}) in ${TRIAGE_STATE_FILE}."
  else
    echo "Warning: unable to resolve version for preflight recording; skipping preflight record."
  fi
fi

if [[ "${PREFLIGHT_ONLY}" == "true" ]]; then
  echo "Preflight-only mode complete; skipping codex compile/install/publish."
  exit 0
fi

if [[ "${ARMV7_ONLY}" == "true" ]]; then
  echo "Running armv7 build via scripts/build_armv7.sh..."
  run_armv7_build "${MODE}"
  exit 0
fi

if [[ "${MODE}" == "release" ]]; then
  echo "[2/4] Building release codex..."
  if [[ -n "${BUILD_JOBS}" ]]; then
    echo "Using Cargo build jobs: ${BUILD_JOBS}"
  fi
  if [[ "${FAST_RELEASE_BUILD}" == "true" ]]; then
    echo "Using fast release profile overrides: LTO=thin, codegen-units=16"
  fi
  run_release_build_with_locked_fallback
  echo "[3/4] Copying release codex to ${INSTALL_BIN}..."
  install -D -m 755 "${RUST_WORKSPACE_DIR}/target/release/codex" "${INSTALL_BIN}"
else
  echo "[2/4] Building debug codex..."
  # Force rebuild of codex-build-info so its build script reruns and embeds the current timestamp.
  cargo +"${TOOLCHAIN}" clean -p codex-build-info
  if [[ -n "${BUILD_JOBS}" ]]; then
    echo "Using Cargo build jobs: ${BUILD_JOBS}"
    RUSTUP_DISABLE_SELF_UPDATE=1 CARGO_BUILD_JOBS="${BUILD_JOBS}" cargo +"${TOOLCHAIN}" build -p codex-cli
  else
    RUSTUP_DISABLE_SELF_UPDATE=1 cargo +"${TOOLCHAIN}" build -p codex-cli
  fi
  echo "[3/4] Copying debug codex to ${INSTALL_BIN}..."
  install -D -m 755 "${RUST_WORKSPACE_DIR}/target/debug/codex" "${INSTALL_BIN}"
fi

if [[ "${BUILD_NPM_VENDOR}" == "true" ]]; then
  require_cmd python3
  echo "Building npm vendor target ${NPM_X64_TARGET} (${MODE})..."
  if ! rustup target list --toolchain "${TOOLCHAIN}" --installed | grep -Fxq "${NPM_X64_TARGET}"; then
    rustup target add --toolchain "${TOOLCHAIN}" "${NPM_X64_TARGET}"
  fi
  run_target_build_with_locked_fallback "${NPM_X64_TARGET}" "${MODE}"

  echo "Building npm vendor target ${NPM_ARMV7_TARGET} (${MODE})..."
  run_armv7_build "${MODE}"
  vendor_root="${NPM_VENDOR_DIR}"
  if [[ -z "${vendor_root}" ]]; then
    vendor_root="${REPO_DIR}/${NPM_VENDOR_DIR_DEFAULT}/${VERSION}"
  elif [[ "${vendor_root}" != /* ]]; then
    vendor_root="${REPO_DIR}/${vendor_root}"
  fi
  stage_npm_vendor_from_binaries "${VERSION}" "${vendor_root}" "${MODE}"
fi

echo "[4/4] Cleaning workspace codex binaries from target/..."
rm -f "${RUST_WORKSPACE_DIR}/target/release/codex"

echo "Final version:"
echo "- Installed: $("${INSTALL_BIN}" --version)"

if [[ "${publish_to_github}" == "true" ]]; then

  timeout_secs="$((PUBLISH_TIMEOUT_MINUTES * 60))"

  assert_publish_worktree_state

  current_branch="$(git -C "${REPO_DIR}" rev-parse --abbrev-ref HEAD)"
  if [[ "${current_branch}" != "main" ]]; then
    echo "Publish requires main branch (current: ${current_branch})." >&2
    exit 1
  fi

  echo "Syncing with origin..."
  git -C "${REPO_DIR}" fetch origin --prune
  local_head="$(git -C "${REPO_DIR}" rev-parse HEAD)"
  remote_head="$(git -C "${REPO_DIR}" rev-parse origin/main)"
  if [[ "${local_head}" != "${remote_head}" ]]; then
    if git -C "${REPO_DIR}" merge-base --is-ancestor "${remote_head}" "${local_head}"; then
      echo "Local main is ahead of origin/main; pushing main..."
      git -C "${REPO_DIR}" push origin main
    else
      echo "Local main is behind/diverged from origin/main; sync manually first." >&2
      exit 1
    fi
  fi

  if git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null; then
    if [[ "${FORCE_TAG}" == "true" ]]; then
      echo "Tag ${TAG} already exists locally; replacing it."
      git tag -d "${TAG}"
    else
      echo "Tag ${TAG} already exists locally. Re-run without --no-force-tag to replace it."
      exit 1
    fi
  fi

  if git ls-remote --tags origin "refs/tags/${TAG}" | grep -q "${TAG}"; then
    if [[ "${FORCE_TAG}" == "true" ]]; then
      echo "Tag ${TAG} already exists on origin; deleting it."
      if ! git push origin ":refs/tags/${TAG}"; then
        echo "Failed to delete tag ${TAG} on origin. Fix the issue above, then re-run this script." >&2
        exit 1
      fi
    else
      echo "Tag ${TAG} already exists on origin. Re-run without --no-force-tag to replace it."
      exit 1
    fi
  fi

  echo "Tagging and pushing ${TAG}..."
  git tag -a "${TAG}" -m "Release ${VERSION}"
  if ! git push origin "${TAG}"; then
    echo "Failed to push ${TAG}. Fix the issue above, then re-run this script." >&2
    exit 1
  fi
  echo "Pushed ${TAG} to origin."

  origin_url="$(git config --get remote.origin.url || true)"
  if [[ "${origin_url}" =~ github\.com[:/]+([^/]+)/([^/]+)(\.git)?$ ]]; then
    owner="${BASH_REMATCH[1]}"
    repo="${BASH_REMATCH[2]%\.git}"
    workflow_url="https://github.com/${owner}/${repo}/actions/workflows/custom-codex-release.yml"
    echo "GitHub Actions workflow: ${workflow_url}"
    echo "Waiting for run triggered by ${TAG}..."
  fi

  run_id="$(wait_for_run_to_appear "${TAG}" "${timeout_secs}")"
  echo "Found run: ${run_id}"
  wait_for_run_completion "${run_id}" "${timeout_secs}"

  echo "Verifying GitHub release assets..."
  assert_release_assets "${TAG}" "${VERSION}"

  echo "Verifying npm linux tags..."
  assert_npm_linux_tags "${timeout_secs}"
  echo "Publish flow completed successfully for ${TAG}."
fi

if [[ "${publish_to_npm}" == "true" ]]; then
  npm_publish_cmd=(
    "${REPO_DIR}/scripts/publish_npm_local.sh"
    --version "${VERSION}"
    --publish
  )
  if [[ "${NPM_PUBLISH_DRY_RUN}" == "true" ]]; then
    npm_publish_cmd+=(--dry-run)
  fi
  echo "Publishing npm packages locally for version ${VERSION}..."
  "${npm_publish_cmd[@]}"
fi
