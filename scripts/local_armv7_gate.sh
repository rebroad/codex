#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
RUST_WORKSPACE_DIR="${REPO_DIR}/codex-rs"
TOOLCHAIN_FILE="${RUST_WORKSPACE_DIR}/rust-toolchain.toml"

TARGET_DEFAULT="armv7-unknown-linux-gnueabihf"
PROFILE="release"
ALLOW_NON_ARM_HOST="false"
TARGET="${TARGET_DEFAULT}"
RUSTY_V8_RELEASE_REPO="${RUSTY_V8_RELEASE_REPO:-openai/codex}"
RUSTY_V8_RELEASE_TAG="${RUSTY_V8_RELEASE_TAG:-}"

usage() {
  cat <<'EOF'
Usage: local_armv7_gate.sh [--release|--debug] [--target=<triple>] [--allow-non-arm-host] [--rusty-v8-release-repo=<owner/repo>] [--rusty-v8-release-tag=<tag>]

Build codex-cli with the same core armv7 environment as release CI:
- prebuilt rusty_v8 archive + binding
- target defaults to armv7-unknown-linux-gnueabihf

Options:
  --release             Build release profile (default)
  --debug               Build debug profile
  --target=<triple>     Override target triple (default: armv7-unknown-linux-gnueabihf)
  --allow-non-arm-host  Allow running on non-arm hosts (cross setup is your responsibility)
  --rusty-v8-release-repo=<owner/repo>
                        GitHub repo hosting rusty_v8 release assets (default: openai/codex)
  --rusty-v8-release-tag=<tag>
                        Release tag containing armv7 assets (default: rusty-v8-v<resolved v8 crate version>)
  -h, --help            Show this help
EOF
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

download_file() {
  local url="$1"
  local output="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "${url}" -o "${output}"
    return 0
  fi
  if command -v wget >/dev/null 2>&1; then
    wget -q -O "${output}" "${url}"
    return 0
  fi
  echo "curl or wget is required to download rusty_v8 artifacts." >&2
  exit 1
}

for arg in "$@"; do
  case "${arg}" in
    --release)
      PROFILE="release"
      ;;
    --debug)
      PROFILE="debug"
      ;;
    --target=*)
      TARGET="${arg#*=}"
      ;;
    --allow-non-arm-host)
      ALLOW_NON_ARM_HOST="true"
      ;;
    --rusty-v8-release-repo=*)
      RUSTY_V8_RELEASE_REPO="${arg#*=}"
      ;;
    --rusty-v8-release-tag=*)
      RUSTY_V8_RELEASE_TAG="${arg#*=}"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: ${arg}" >&2
      usage
      exit 1
      ;;
  esac
done

host_arch="$(uname -m)"
if [[ "${ALLOW_NON_ARM_HOST}" != "true" ]]; then
  case "${host_arch}" in
    armv7*|armv6*|armhf|arm)
      ;;
    *)
      echo "Host architecture is ${host_arch}." >&2
      echo "This script is intended for native Pi/armv7 builds." >&2
      echo "Use --allow-non-arm-host only if your cross toolchain is already configured." >&2
      exit 1
      ;;
  esac
fi

require_cmd cargo
require_cmd rustup
require_cmd python3
require_cmd file
require_cmd mktemp

if [[ -f "${HOME}/.cargo/env" ]]; then
  # shellcheck disable=SC1090
  source "${HOME}/.cargo/env"
fi

TOOLCHAIN="$(sed -n 's/^channel = "\(.*\)"/\1/p' "${TOOLCHAIN_FILE}")"
if [[ -z "${TOOLCHAIN}" ]]; then
  echo "Unable to read pinned Rust toolchain from ${TOOLCHAIN_FILE}" >&2
  exit 1
fi

if ! rustup toolchain list | grep -q "^${TOOLCHAIN}-"; then
  echo "Installing pinned Rust toolchain ${TOOLCHAIN}..."
  rustup toolchain install "${TOOLCHAIN}" --component clippy rustfmt rust-src
fi

if ! rustup target list --toolchain "${TOOLCHAIN}" --installed | grep -Fxq "${TARGET}"; then
  echo "Installing Rust target ${TARGET} for toolchain ${TOOLCHAIN}..."
  rustup target add --toolchain "${TOOLCHAIN}" "${TARGET}"
fi

cd "${RUST_WORKSPACE_DIR}"

export RUSTUP_DISABLE_SELF_UPDATE=1

resolved_v8_version="$(python3 "${REPO_DIR}/.github/scripts/rusty_v8_bazel.py" resolved-v8-crate-version)"
release_tag="${RUSTY_V8_RELEASE_TAG}"
if [[ -z "${release_tag}" ]]; then
  release_tag="rusty-v8-v${resolved_v8_version}"
fi
base_url="https://github.com/${RUSTY_V8_RELEASE_REPO}/releases/download/${release_tag}"
archive_url="${base_url}/librusty_v8_release_${TARGET}.a.gz"

tmp_dir="$(mktemp -d)"
cleanup() {
  rm -rf "${tmp_dir}"
}
trap cleanup EXIT INT TERM

binding_path="${tmp_dir}/src_binding_release_${TARGET}.rs"
echo "Fetching rusty_v8 binding: ${base_url}/src_binding_release_${TARGET}.rs"
download_file "${base_url}/src_binding_release_${TARGET}.rs" "${binding_path}"

export RUSTY_V8_ARCHIVE="${archive_url}"
export RUSTY_V8_SRC_BINDING_PATH="${binding_path}"

cargo_args=(+"${TOOLCHAIN}" build -p codex-cli --locked --target "${TARGET}")
if [[ "${PROFILE}" == "release" ]]; then
  cargo_args+=(--release)
fi

echo "Building codex-cli (${PROFILE}) for ${TARGET}..."
echo "Using prebuilt rusty_v8 from ${RUSTY_V8_RELEASE_REPO} (${release_tag})"
cargo "${cargo_args[@]}"

bin_path="${RUST_WORKSPACE_DIR}/target/${TARGET}/${PROFILE}/codex"
if [[ ! -x "${bin_path}" ]]; then
  echo "Build completed but binary was not found at ${bin_path}" >&2
  exit 1
fi

bin_desc="$(file -b "${bin_path}" || true)"
if [[ "${TARGET}" == "${TARGET_DEFAULT}" ]] && ! grep -Eq 'ARM|arm' <<<"${bin_desc}"; then
  echo "Built binary does not appear to be ARM (${bin_desc})" >&2
  exit 1
fi

echo "Local armv7 gate passed."
echo "Binary: ${bin_path}"
echo "Description: ${bin_desc}"
