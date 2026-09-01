#!/usr/bin/env bash
set -euo pipefail

# Install the small set of tools used by the repository's development recipes.
# Run this script from a network-enabled shell when a package is missing.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
CHECK_ONLY=false
ANDROID_PHANTOM_PROCESS_MINIMUM=128

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

is_android_host() {
  [[ "$(uname -o 2>/dev/null || true)" == Android ]]
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
  if is_android_host && ! command -v adb >/dev/null 2>&1; then
    missing+=(android-tools)
  fi
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

android_adb_serial() {
  local devices=()
  if [[ -n "${ANDROID_SERIAL:-}" ]]; then
    [[ "$(adb -s "${ANDROID_SERIAL}" get-state 2>/dev/null || true)" == device ]] \
      || die "ANDROID_SERIAL device is not connected: ${ANDROID_SERIAL}"
    printf '%s\n' "${ANDROID_SERIAL}"
    return
  fi

  mapfile -t devices < <(adb devices | awk '$2 == "device" { print $1 }')
  ((${#devices[@]} > 0)) \
    || die "no ADB device is connected; enable wireless debugging and connect this Android device"
  ((${#devices[@]} == 1)) \
    || die "multiple ADB devices are connected; set ANDROID_SERIAL"
  printf '%s\n' "${devices[0]}"
}

configure_android_phantom_process_limit() {
  is_android_host || return

  local serial current effective desired override_supported=false override_value=''
  require_command adb
  serial="$(android_adb_serial)"
  current="$(adb -s "${serial}" shell device_config get activity_manager max_phantom_processes | tr -d '\r')"
  effective="$(adb -s "${serial}" shell dumpsys activity settings \
    | sed -n 's/^[[:space:]]*max_phantom_processes=//p' \
    | head -n 1 \
    | tr -d '\r')"
  if adb -s "${serial}" shell device_config help | grep -q '^  override '; then
    override_supported=true
    override_value="$(adb -s "${serial}" shell device_config list_local_overrides \
      | sed -n 's#^activity_manager/max_phantom_processes=##p' \
      | head -n 1 \
      | tr -d '\r')"
  fi
  if [[ "${effective}" =~ ^[0-9]+$ ]] \
    && ((effective >= ANDROID_PHANTOM_PROCESS_MINIMUM)) \
    && { [[ "${override_supported}" == false ]] \
      || [[ "${override_value}" == "${effective}" ]]; }; then
    printf '  Android max phantom processes: %s (device %s)\n' "${effective}" "${serial}"
    return
  fi

  [[ "${CHECK_ONLY}" == false ]] \
    || die "Android max_phantom_processes is ${effective}; run setup without --check to apply a durable minimum of ${ANDROID_PHANTOM_PROCESS_MINIMUM}"
  desired="${ANDROID_PHANTOM_PROCESS_MINIMUM}"
  if [[ "${current}" =~ ^[0-9]+$ ]] && ((current > desired)); then
    desired="${current}"
  fi
  if [[ "${override_supported}" == true ]]; then
    adb -s "${serial}" shell device_config override activity_manager max_phantom_processes \
      "${desired}"
  fi
  adb -s "${serial}" shell device_config put activity_manager max_phantom_processes \
    "${desired}"
  effective="$(adb -s "${serial}" shell dumpsys activity settings \
    | sed -n 's/^[[:space:]]*max_phantom_processes=//p' \
    | head -n 1 \
    | tr -d '\r')"
  [[ "${effective}" =~ ^[0-9]+$ ]] \
    && ((effective >= ANDROID_PHANTOM_PROCESS_MINIMUM)) \
    || die "failed to raise Android max_phantom_processes; effective value is ${effective}"
  printf '  Android max phantom processes: %s (device %s)\n' "${effective}" "${serial}"
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

cd "${REPO_ROOT}"
CARGO_SHEAR_VERSION="$(sed -n 's/^[[:space:]]*tool: cargo-shear@\([^[:space:]]*\).*$/\1/p' .github/workflows/rust-ci.yml | head -n 1)"
[[ -n "${CARGO_SHEAR_VERSION}" ]] || die "could not read cargo-shear version from .github/workflows/rust-ci.yml"
require_command cargo
require_command rustc
require_command python3
require_command git
require_command protoc

CARGO_BIN_DIR="${CARGO_HOME:-${HOME}/.cargo}/bin"
export PATH="${CARGO_BIN_DIR}:${PATH}"

install_system_tools
configure_android_phantom_process_limit

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
  local package="$1" executable="$2" version="${3:-}" installed_path="" package_spec
  local root_args=()
  package_spec="${package}"
  if [[ -n "${version}" ]]; then
    package_spec="${package}@${version}"
  fi
  if [[ -n "${PREFIX:-}" && -x "${PREFIX}/bin/${executable}" ]]; then
    installed_path="${PREFIX}/bin/${executable}"
  elif [[ -z "${PREFIX:-}" ]] && command -v "${executable}" >/dev/null 2>&1; then
    installed_path="$(command -v "${executable}")"
  fi
  if [[ -n "${installed_path}" ]]; then
    if [[ -z "${version}" ]]; then
      return
    fi
    if [[ -n "${PREFIX:-}" && -f "${PREFIX}/.crates2.json" ]]; then
      if grep -Fq "\"${package} ${version} (" "${PREFIX}/.crates2.json"; then
        return
      fi
    elif "${installed_path}" --version 2>/dev/null | grep -Fq "${version}"; then
      return
    fi
  fi
  [[ "${CHECK_ONLY}" == true ]] && die "missing ${executable}"
  if [[ -n "${PREFIX:-}" && -d "${PREFIX}/bin" ]]; then
    root_args=(--root "${PREFIX}")
  fi
  cargo install --locked "${root_args[@]}" "${package_spec}"
}

install_cargo_tool cargo-nextest cargo-nextest
install_cargo_tool cargo-insta cargo-insta
install_cargo_tool cargo-shear cargo-shear "${CARGO_SHEAR_VERSION}"

echo "Development environment prerequisites are available."
printf '  just:         %s\n' "$(command -v just)"
printf '  ripgrep:      %s\n' "$(command -v rg)"
printf '  cargo-nextest: %s\n' "$(command -v cargo-nextest)"
printf '  cargo-insta:   %s\n' "$(command -v cargo-insta)"
printf '  cargo-shear:   %s\n' "$(command -v cargo-shear)"
