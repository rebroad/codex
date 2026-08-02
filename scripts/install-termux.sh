#!/usr/bin/env sh

set -eu

PACKAGE_SPEC="${CODEX_TERMUX_PACKAGE:-@mmmbuto/codex-cli-termux@latest}"

if [ "$(uname -m)" != "aarch64" ]; then
  echo "Codex for Termux requires an ARM64 device (uname -m must be aarch64)." >&2
  exit 1
fi

if ! command -v npm >/dev/null 2>&1; then
  if command -v pkg >/dev/null 2>&1; then
    echo "npm is missing; installing Node.js for Termux..."
    pkg install -y nodejs-lts
  else
    echo "npm is required. Run this script inside Termux, then install nodejs-lts." >&2
    exit 1
  fi
fi

echo "Installing ${PACKAGE_SPEC}..."
npm install --global "$PACKAGE_SPEC"

if ! command -v codex >/dev/null 2>&1; then
  echo "npm installation succeeded, but codex is not on PATH." >&2
  echo "Add npm's global bin directory to PATH, then run: codex --version" >&2
  exit 1
fi

codex --version
echo "Codex is installed. Run 'codex login' to authenticate."
