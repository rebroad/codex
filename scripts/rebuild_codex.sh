#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
RUST_WORKSPACE_DIR="${REPO_DIR}/codex-rs"
TOOLCHAIN_FILE="${RUST_WORKSPACE_DIR}/rust-toolchain.toml"
INSTALL_BIN="${HOME}/.cargo/bin/codex"
CARGO_LOCK_REL="codex-rs/Cargo.lock"
PUBLISH_TIMEOUT_MINUTES_DEFAULT=45

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
  status_lines="$(git -C "${REPO_DIR}" status --porcelain)"
  if [[ -z "${status_lines}" ]]; then
    return 0
  fi
  filtered_status="$(grep -Ev "^[ MARCUD?]{2} ${CARGO_LOCK_REL}$" <<<"${status_lines}" || true)"
  [[ -z "${filtered_status}" ]]
}

assert_publish_worktree_state() {
  if is_only_cargo_lock_dirty; then
    return 0
  fi
  echo "Working tree has changes beyond ${CARGO_LOCK_REL}; refusing to publish." >&2
  git -C "${REPO_DIR}" status --short >&2
  exit 1
}

find_release_run_id() {
  local tag_name="$1"
  gh run list \
    --workflow custom-codex-release.yml \
    --limit 50 \
    --json databaseId,displayTitle,event \
    --jq ".[] | select(.event == \"push\" and .displayTitle == \"${tag_name}\") | .databaseId" \
    | head -n 1
}

wait_for_run_to_appear() {
  local tag_name="$1"
  local timeout_secs="$2"
  local start_secs now elapsed run_id
  start_secs="$(date +%s)"
  while true; do
    run_id="$(find_release_run_id "${tag_name}" || true)"
    if [[ -n "${run_id}" ]]; then
      echo "${run_id}"
      return 0
    fi
    now="$(date +%s)"
    elapsed="$((now - start_secs))"
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
  local start_secs now elapsed status conclusion url
  start_secs="$(date +%s)"
  while true; do
    status="$(gh run view "${run_id}" --json status --jq '.status')"
    conclusion="$(gh run view "${run_id}" --json conclusion --jq '.conclusion // ""')"
    url="$(gh run view "${run_id}" --json url --jq '.url')"
    echo "run=${run_id} status=${status} conclusion=${conclusion} url=${url}"

    if [[ "${status}" == "completed" ]]; then
      if [[ "${conclusion}" != "success" ]]; then
        echo "Release workflow failed: ${url}" >&2
        return 1
      fi
      return 0
    fi

    now="$(date +%s)"
    elapsed="$((now - start_secs))"
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

MODE="debug"
PUBLISH="auto"
REGEN_SCHEMA="auto"
FORCE_TAG="true"
PUBLISH_TIMEOUT_MINUTES="${PUBLISH_TIMEOUT_MINUTES_DEFAULT}"
SCHEMA_HASH_FILE="${REPO_DIR}/codex-rs/target/app-server-schema.hash"
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
    --regen-schema)
      REGEN_SCHEMA="true"
      ;;
    --no-regen-schema)
      REGEN_SCHEMA="false"
      ;;
    --publish-timeout-minutes=*)
      PUBLISH_TIMEOUT_MINUTES="${arg#*=}"
      ;;
    -h|--help)
      cat <<'EOF'
Usage: rebuild_codex.sh [--release] [--publish|--no-publish] [--publish-timeout-minutes=N]

Default behavior:
- Regenerate app-server schema (just write-app-server-schema)
- Build debug codex, copy it to ~/.cargo/bin/codex
- Remove workspace codex build artifacts under target/

Options:
  --release   Build/install release codex into ~/.cargo/bin/codex, then clean target binaries
  --publish   Create + push a git tag for the workspace version (codex-vX.Y.Z[-...])
  --no-publish
             Skip tag/push even in release mode
  --no-force-tag
             Do not replace existing tags (default is to replace)
  --regen-schema
             Force schema regeneration
  --no-regen-schema
             Skip schema regeneration
  --publish-timeout-minutes=N
             Timeout in minutes for release workflow/npm verification (default: 45)
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

if [[ "${MODE}" == "release" ]]; then
  echo "[2/4] Building release codex..."
  RUSTUP_DISABLE_SELF_UPDATE=1 CARGO_INCREMENTAL=1 cargo +"${TOOLCHAIN}" build -p codex-cli --release --locked
  echo "[3/4] Copying release codex to ${INSTALL_BIN}..."
  install -D -m 755 "${RUST_WORKSPACE_DIR}/target/release/codex" "${INSTALL_BIN}"
else
  echo "[2/4] Building debug codex..."
  # Force rebuild of codex-build-info so its build script reruns and embeds the current timestamp.
  cargo +"${TOOLCHAIN}" clean -p codex-build-info
  RUSTUP_DISABLE_SELF_UPDATE=1 cargo +"${TOOLCHAIN}" build -p codex-cli
  echo "[3/4] Copying debug codex to ${INSTALL_BIN}..."
  install -D -m 755 "${RUST_WORKSPACE_DIR}/target/debug/codex" "${INSTALL_BIN}"
fi

echo "[4/4] Cleaning workspace codex binaries from target/..."
rm -f "${RUST_WORKSPACE_DIR}/target/release/codex"

echo "Final version:"
echo "- Installed: $("${INSTALL_BIN}" --version)"

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

  timeout_secs="$((PUBLISH_TIMEOUT_MINUTES * 60))"
  VERSION="$(
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
  )"
  if [[ -z "${VERSION}" ]]; then
    version_from_codex="$("${INSTALL_BIN}" --version | awk '{print $2}')"
    if [[ "${version_from_codex}" =~ ^(.+)-([0-9]{8,})$ ]]; then
      VERSION="${BASH_REMATCH[1]}"
    else
      VERSION="${version_from_codex}"
    fi
  fi

  if [[ -z "${VERSION}" ]]; then
    echo "Unable to read workspace version from ${RUST_WORKSPACE_DIR}/Cargo.toml or ${INSTALL_BIN} --version"
    exit 1
  fi

  TAG="codex-v${VERSION}"

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
