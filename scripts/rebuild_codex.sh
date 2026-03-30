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
REGEN_SCHEMA="auto"
CI_PREFLIGHT="auto"
FORCE_TAG="true"
AUTO_COMMIT_PREFLIGHT_FIXES="true"
PUBLISH_TIMEOUT_MINUTES="${PUBLISH_TIMEOUT_MINUTES_DEFAULT}"
SHEAR_FIX_MODE="on_error"
BUILD_JOBS=""
FAST_RELEASE_BUILD="false"
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
    --ci-preflight)
      CI_PREFLIGHT="true"
      ;;
    --no-ci-preflight)
      CI_PREFLIGHT="false"
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
  --no-publish
             Skip tag/push even in release mode
  --ci-preflight
             Run CI-like preflight checks (pnpm format, cargo fmt, argument-comment-lint, cargo shear)
  --no-ci-preflight
             Skip preflight checks even when publishing
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

cd "${REPO_DIR}"
schema_should_run="false"
if [[ "${REGEN_SCHEMA}" == "true" ]]; then
  schema_should_run="true"
elif [[ "${REGEN_SCHEMA}" != "false" ]]; then
  schema_should_run="true"
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
  rustup toolchain install "${TOOLCHAIN}" --component clippy rustfmt rust-src
fi

should_publish="false"
if [[ "${MODE}" == "release" && "${PUBLISH}" != "false" ]]; then
  should_publish="true"
elif [[ "${PUBLISH}" == "true" ]]; then
  should_publish="true"
fi

if [[ "${should_publish}" == "true" ]]; then
  require_cmd gh
  require_cmd jq
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
  TAG="codex-v${VERSION}"
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

echo "[4/4] Cleaning workspace codex binaries from target/..."
rm -f "${RUST_WORKSPACE_DIR}/target/release/codex"

echo "Final version:"
echo "- Installed: $("${INSTALL_BIN}" --version)"

if [[ "${should_publish}" == "true" ]]; then

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
