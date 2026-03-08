#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
RUST_WORKSPACE_DIR="${REPO_DIR}/codex-rs"
TOOLCHAIN_FILE="${RUST_WORKSPACE_DIR}/rust-toolchain.toml"
INSTALL_BIN="${HOME}/.cargo/bin/codex"

MODE="debug"
for arg in "$@"; do
  case "${arg}" in
    --debug)
      MODE="debug"
      ;;
    --release)
      MODE="release"
      ;;
    -h|--help)
      cat <<'EOF'
Usage: rebuild_codex_and_show_versions.sh [--release]

Default behavior:
- Regenerate app-server schema (just write-app-server-schema)
- Build debug codex, copy it to ~/.cargo/bin/codex
- Remove workspace codex build artifacts under target/

Options:
  --release   Build/install release codex into ~/.cargo/bin/codex, then clean target binaries
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
rm -f "${RUST_WORKSPACE_DIR}/target/release/codex"

echo "Final version:"
echo "- Installed: $("${INSTALL_BIN}" --version)"
