#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
RUST_WORKSPACE_DIR="${REPO_DIR}/codex-rs"
TOOLCHAIN_FILE="${RUST_WORKSPACE_DIR}/rust-toolchain.toml"
INSTALL_BIN="${HOME}/.cargo/bin/codex"

MODE="debug"
PUBLISH="auto"
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
echo "[1/4] Regenerating app-server schema..."
just write-app-server-schema

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
  echo "[2/4] Installing release codex into ${INSTALL_BIN}..."
  RUSTUP_DISABLE_SELF_UPDATE=1 cargo +"${TOOLCHAIN}" install --path cli --force
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
    awk '
      $0 ~ /^\[workspace\.package\]/ { in=1; next }
      in && $0 ~ /^\[/ { exit }
      in && match($0, /^version[[:space:]]*=[[:space:]]*"([^"]+)"/, m) { print m[1]; exit }
    ' "${RUST_WORKSPACE_DIR}/Cargo.toml"
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
    echo "Tag ${TAG} already exists locally."
    exit 1
  fi

  if git ls-remote --tags origin "refs/tags/${TAG}" | grep -q "${TAG}"; then
    echo "Tag ${TAG} already exists on origin."
    exit 1
  fi

  echo "Tagging and pushing ${TAG}..."
  git tag -a "${TAG}" -m "Release ${VERSION}"
  git push origin "${TAG}"

  origin_url="$(git config --get remote.origin.url || true)"
  if [[ "${origin_url}" =~ github\.com[:/]+([^/]+)/([^/]+)(\.git)?$ ]]; then
    owner="${BASH_REMATCH[1]}"
    repo="${BASH_REMATCH[2]}"
    workflow_url="https://github.com/${owner}/${repo}/actions/workflows/custom-codex-release.yml"
    echo "GitHub Actions workflow: ${workflow_url}"
    echo "Look for the run triggered by tag ${TAG}."
  fi
fi
