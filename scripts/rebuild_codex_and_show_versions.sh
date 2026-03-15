#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
RUST_WORKSPACE_DIR="${REPO_DIR}/codex-rs"
TOOLCHAIN_FILE="${RUST_WORKSPACE_DIR}/rust-toolchain.toml"
INSTALL_BIN="${HOME}/.cargo/bin/codex"

MODE="debug"
PUBLISH="auto"
REGEN_SCHEMA="auto"
FORCE_TAG="true"
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
    -h|--help)
      cat <<'EOF'
Usage: rebuild_codex_and_show_versions.sh [--release] [--publish|--no-publish]

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
rm -f "${RUST_WORKSPACE_DIR}/target/debug/codex" "${RUST_WORKSPACE_DIR}/target/release/codex"

echo "Final version:"
echo "- Installed: $("${INSTALL_BIN}" --version)"

should_publish="false"
if [[ "${MODE}" == "release" && "${PUBLISH}" != "false" ]]; then
  should_publish="true"
elif [[ "${PUBLISH}" == "true" ]]; then
  should_publish="true"
fi

if [[ "${should_publish}" == "true" ]]; then
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

  if [[ -n "$(git status --porcelain)" ]]; then
    echo "Working tree is not clean; refusing to tag ${TAG}."
    exit 1
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
    echo "Look for the run triggered by tag ${TAG}."
  fi
fi
