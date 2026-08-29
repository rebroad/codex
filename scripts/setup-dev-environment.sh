#!/usr/bin/env bash
set -euo pipefail

# Install the small set of tools used by the repository's development recipes.
# Run this script from a network-enabled shell when a package is missing.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
CHECK_ONLY=false

usage() {
  cat <<'EOF'
Usage: scripts/setup-dev-environment.sh [--check]

Install or verify the tools used by the repository's development recipes.
--check  verify prerequisites without installing anything
EOF
}

die() {
  echo "setup-dev-environment.sh: $*" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) CHECK_ONLY=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

install_system_tools() {
  local missing=()
  command -v just >/dev/null 2>&1 || missing+=(just)
  command -v rg >/dev/null 2>&1 || missing+=(ripgrep)
  ((${#missing[@]} == 0)) && return
  [[ "${CHECK_ONLY}" == true ]] && die "missing ${missing[*]}"

  if command -v pkg >/dev/null 2>&1; then
    pkg install -y "${missing[@]}"
  elif command -v brew >/dev/null 2>&1; then
    brew install "${missing[@]}"
  elif command -v apt-get >/dev/null 2>&1; then
    if [[ "${EUID}" -eq 0 ]]; then
      apt-get update
      apt-get install -y "${missing[@]}"
    elif command -v sudo >/dev/null 2>&1; then
      sudo apt-get update
      sudo apt-get install -y "${missing[@]}"
    else
      die "missing ${missing[*]}; install them with apt-get"
    fi
  else
    die "missing ${missing[*]}; no supported package manager was found"
  fi
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

cd "${REPO_ROOT}"
require_command cargo
require_command rustc
require_command python3
require_command git
require_command protoc

CARGO_BIN_DIR="${CARGO_HOME:-${HOME}/.cargo}/bin"
export PATH="${CARGO_BIN_DIR}:${PATH}"

install_system_tools

if command -v rustup >/dev/null 2>&1; then
  if [[ "${CHECK_ONLY}" == true ]]; then
    require_command rustfmt
    require_command cargo-clippy
  else
    rustup component add rustfmt clippy
  fi
else
  require_command rustfmt
  require_command cargo-clippy
fi

install_cargo_tool() {
  local package="$1" executable="$2" root_args=()
  if [[ -n "${PREFIX:-}" && -x "${PREFIX}/bin/${executable}" ]]; then
    return
  fi
  if [[ -z "${PREFIX:-}" ]] && command -v "${executable}" >/dev/null 2>&1; then
    return
  fi
  [[ "${CHECK_ONLY}" == true ]] && die "missing ${executable}"
  if [[ -n "${PREFIX:-}" && -d "${PREFIX}/bin" ]]; then
    root_args=(--root "${PREFIX}")
  fi
  cargo install --locked "${root_args[@]}" "${package}"
}

install_cargo_tool cargo-nextest cargo-nextest
install_cargo_tool cargo-insta cargo-insta
install_cargo_tool cargo-shear cargo-shear

echo "Development environment prerequisites are available."
printf '  just:         %s\n' "$(command -v just)"
printf '  ripgrep:      %s\n' "$(command -v rg)"
printf '  cargo-nextest: %s\n' "$(command -v cargo-nextest)"
printf '  cargo-insta:   %s\n' "$(command -v cargo-insta)"
printf '  cargo-shear:   %s\n' "$(command -v cargo-shear)"
