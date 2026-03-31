#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
RUST_WORKSPACE_DIR="${REPO_DIR}/codex-rs"
TOOLCHAIN_FILE="${RUST_WORKSPACE_DIR}/rust-toolchain.toml"
CARGO_LOCK_PATH="${RUST_WORKSPACE_DIR}/Cargo.lock"

TARGET_DEFAULT="armv7-unknown-linux-gnueabihf"
PROFILE="release"
TARGET="${TARGET_DEFAULT}"
BUILD_ENV="${BUILD_ENV:-auto}"
RUSTY_V8_RELEASE_REPO="${RUSTY_V8_RELEASE_REPO:-rebroad/rusty_v8}"
RUSTY_V8_RELEASE_TAG="${RUSTY_V8_RELEASE_TAG:-}"
RUSTY_V8_LOCAL_PATH_DEFAULT="${HOME}/src/rusty_v8"
RUSTY_V8_LOCAL_PATH="${RUSTY_V8_LOCAL_PATH:-${RUSTY_V8_LOCAL_PATH_DEFAULT}}"
ARMV7_CACHE_DIR="${ARMV7_CACHE_DIR:-${REPO_DIR}/tmp/armv7-cache}"
PUBLISH_GITHUB="false"
GITHUB_RELEASE_REPO="${GITHUB_RELEASE_REPO:-}"
GITHUB_RELEASE_TAG="${GITHUB_RELEASE_TAG:-}"

usage() {
  cat <<'EOF'
Usage: build_armv7.sh [--release|--debug] [--target=<triple>] [--build-env=<auto|host|docker-buster>] [--allow-non-arm-host] [--rusty-v8-release-repo=<owner/repo>] [--rusty-v8-release-tag=<tag>] [--rusty-v8-local-path=<path>] [--publish-github|--no-publish-github] [--github-release-repo=<owner/repo>] [--github-release-tag=<tag>]

Build codex-cli with the same core armv7 environment as release CI:
- prebuilt rusty_v8 archive + binding
- target defaults to armv7-unknown-linux-gnueabihf

Options:
  --release             Build release profile (default)
  --debug               Build debug profile
  --target=<triple>     Override target triple (default: armv7-unknown-linux-gnueabihf)
  --build-env=<mode>    Build environment mode:
                        auto (default): use docker-buster for armv7 unless host is buster
                        host: always build on current host
                        docker-buster: force Debian buster container build
  --allow-non-arm-host  Deprecated no-op (non-arm hosts are now supported by default)
  --rusty-v8-release-repo=<owner/repo>
                        GitHub repo hosting rusty_v8 release assets (default: rebroad/rusty_v8)
  --rusty-v8-release-tag=<tag>
                        Release tag containing armv7 assets (default: rusty-v8-v<resolved v8 crate version>)
  --rusty-v8-local-path=<path>
                        Local rusty_v8 checkout used for crates.io patch (default: ~/src/rusty_v8 when present)
  --publish-github      Upload generated artifacts to a GitHub release (opt-in)
  --no-publish-github   Skip GitHub release upload
  --github-release-repo=<owner/repo>
                        Release repository (default: derived from git remote origin)
  --github-release-tag=<tag>
                        Release tag used for uploads (default: codex-armv7-local-v<version>)
  -h, --help            Show this help
EOF
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

prepare_patched_v8_source() {
  local source_repo="$1"
  local version="$2"
  local output_dir="$3"
  local ref expected_commit meta_file
  ref="v${version}"
  meta_file="${output_dir}/.codex-armv7-v8-patch.meta"

  require_cmd git
  require_cmd tar

  if [[ ! -d "${source_repo}/.git" ]]; then
    echo "rusty_v8 source repo not found: ${source_repo}" >&2
    exit 1
  fi
  if ! git -C "${source_repo}" rev-parse -q --verify "${ref}^{commit}" >/dev/null 2>&1; then
    echo "Missing ${ref} in ${source_repo}; cannot prepare v8 ${version} patch source." >&2
    exit 1
  fi
  expected_commit="$(git -C "${source_repo}" rev-parse "${ref}^{commit}")"

  if [[ -f "${meta_file}" ]] \
    && [[ -f "${output_dir}/Cargo.toml" ]] \
    && grep -Fxq "source_repo=${source_repo}" "${meta_file}" \
    && grep -Fxq "ref=${ref}" "${meta_file}" \
    && grep -Fxq "commit=${expected_commit}" "${meta_file}" \
    && grep -Fxq "patch=remove_typeid_alignment_assert_v1" "${meta_file}"; then
    return 0
  fi

  rm -rf "${output_dir}"
  mkdir -p "${output_dir}"
  git -C "${source_repo}" archive --format=tar "${ref}" | tar -xf - -C "${output_dir}"

  python3 - "${output_dir}/src/isolate.rs" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
needle = """  assert!(
    align_of::<TypeId>() == align_of::<u64>()
      || align_of::<TypeId>() == align_of::<u128>()
  );
"""
replacement = """  // armv7: keep size assertions, but avoid strict alignment assertions that
  // can fail on 32-bit targets.
"""
if needle not in text:
    print(f"Expected TypeId alignment assert block not found in {path}", file=sys.stderr)
    sys.exit(1)
path.write_text(text.replace(needle, replacement, 1), encoding="utf-8")
PY

  cat > "${meta_file}" <<EOF
source_repo=${source_repo}
ref=${ref}
commit=${expected_commit}
patch=remove_typeid_alignment_assert_v1
EOF
}

ensure_armv7_cross_packages() {
  local -a packages
  packages=(
    gcc-arm-linux-gnueabihf
    g++-arm-linux-gnueabihf
    libssl-dev:armhf
    libcap-dev:armhf
    zlib1g-dev:armhf
    libbz2-dev:armhf
    pkg-config
  )

  if ! command -v sudo >/dev/null 2>&1; then
    echo "Missing required command: sudo" >&2
    exit 1
  fi
  if ! command -v apt-get >/dev/null 2>&1; then
    echo "Automatic dependency install currently supports apt-get hosts only." >&2
    exit 1
  fi

  echo "Ensuring armv7 cross-build packages are installed..."
  sudo dpkg --add-architecture armhf
  sudo apt-get update -y
  sudo apt-get install -y "${packages[@]}"
}

setup_armv7_pkg_config_env() {
  local openssl_pc libcap_pc libbz2_so
  openssl_pc="/usr/lib/arm-linux-gnueabihf/pkgconfig/openssl.pc"
  libcap_pc="/usr/lib/arm-linux-gnueabihf/pkgconfig/libcap.pc"
  libbz2_so="/usr/lib/arm-linux-gnueabihf/libbz2.so"

  require_cmd pkg-config

  if [[ ! -f "${openssl_pc}" ]]; then
    ensure_armv7_cross_packages
  fi
  if [[ ! -f "${libcap_pc}" ]]; then
    ensure_armv7_cross_packages
  fi
  if [[ ! -f "${libbz2_so}" ]]; then
    ensure_armv7_cross_packages
  fi
  if [[ ! -f "${openssl_pc}" || ! -f "${libcap_pc}" ]]; then
    echo "Missing required armv7 pkg-config metadata after package install." >&2
    exit 1
  fi
  if [[ ! -f "${libbz2_so}" ]]; then
    echo "Missing required armv7 bzip2 library after package install: ${libbz2_so}" >&2
    exit 1
  fi

  # Force pkg-config to resolve target-armhf libraries while cross-compiling.
  export PKG_CONFIG_ALLOW_CROSS=1
  export PKG_CONFIG_PATH="/usr/lib/arm-linux-gnueabihf/pkgconfig:/usr/share/pkgconfig"
  export PKG_CONFIG_LIBDIR="${PKG_CONFIG_PATH}"
  export PKG_CONFIG_SYSROOT_DIR="/"

  export OPENSSL_LIB_DIR="/usr/lib/arm-linux-gnueabihf"
  export OPENSSL_INCLUDE_DIR="/usr/include"
}

resolve_codex_version() {
  awk '
    /^\[workspace\.package\]/ { in_ws=1; next }
    /^\[/ { if (in_ws) exit }
    in_ws && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' "${RUST_WORKSPACE_DIR}/Cargo.toml"
}

host_version_codename() {
  if [[ -f /etc/os-release ]]; then
    # shellcheck disable=SC1091
    source /etc/os-release
    echo "${VERSION_CODENAME:-}"
    return 0
  fi
  echo ""
}

should_use_docker_buster() {
  if [[ "${TARGET}" != "armv7-unknown-linux-gnueabihf" ]]; then
    return 1
  fi
  if [[ "${CODEX_ARMV7_IN_DOCKER:-}" == "1" ]]; then
    return 1
  fi
  case "${BUILD_ENV}" in
    host)
      return 1
      ;;
    docker-buster)
      return 0
      ;;
    auto)
      if [[ "$(host_version_codename)" == "buster" ]]; then
        return 1
      fi
      return 0
      ;;
    *)
      echo "Invalid --build-env value: ${BUILD_ENV} (expected auto|host|docker-buster)" >&2
      exit 1
      ;;
  esac
}

run_in_docker_buster() {
  local script_in_container="/work/codex/scripts/build_armv7.sh"
  local -a forwarded_args docker_cmd
  local container_rusty_v8_path=""
  local docker_home_dir="${ARMV7_CACHE_DIR}/docker-home"
  local maybe_mount=()

  require_cmd docker
  mkdir -p "${docker_home_dir}"

  forwarded_args=(
    "--${PROFILE}"
    "--target=${TARGET}"
    "--build-env=host"
    "--rusty-v8-release-repo=${RUSTY_V8_RELEASE_REPO}"
  )
  if [[ -n "${RUSTY_V8_RELEASE_TAG}" ]]; then
    forwarded_args+=("--rusty-v8-release-tag=${RUSTY_V8_RELEASE_TAG}")
  fi
  if [[ "${PUBLISH_GITHUB}" == "true" ]]; then
    forwarded_args+=("--publish-github")
  else
    forwarded_args+=("--no-publish-github")
  fi
  if [[ -n "${GITHUB_RELEASE_REPO}" ]]; then
    forwarded_args+=("--github-release-repo=${GITHUB_RELEASE_REPO}")
  fi
  if [[ -n "${GITHUB_RELEASE_TAG}" ]]; then
    forwarded_args+=("--github-release-tag=${GITHUB_RELEASE_TAG}")
  fi

  if [[ -d "${RUSTY_V8_LOCAL_PATH}/.git" ]]; then
    container_rusty_v8_path="/work/rusty_v8"
    maybe_mount=(-v "${RUSTY_V8_LOCAL_PATH}:${container_rusty_v8_path}:ro")
    forwarded_args+=("--rusty-v8-local-path=${container_rusty_v8_path}")
  else
    forwarded_args+=("--rusty-v8-local-path=${RUSTY_V8_LOCAL_PATH}")
  fi

  echo "Building in Debian buster container for Pi-compatible glibc/OpenSSL ABI..."
  docker_cmd=(
    docker run --rm -t
    --platform linux/amd64
    -e DEBIAN_FRONTEND=noninteractive
    -e CODEX_ARMV7_IN_DOCKER=1
    -v "${docker_home_dir}:/root"
    -v "${REPO_DIR}:/work/codex"
    "${maybe_mount[@]}"
    -w /work/codex
    debian:buster
    bash -lc
    "set -euo pipefail; \
      printf '%s\n' \
        'deb http://archive.debian.org/debian buster main contrib non-free' \
        'deb http://archive.debian.org/debian-security buster/updates main contrib non-free' \
        > /etc/apt/sources.list; \
      dpkg --add-architecture armhf; \
      apt-get -o Acquire::Check-Valid-Until=false update -y; \
      apt-get install -y ca-certificates curl git python3 file pkg-config gcc-arm-linux-gnueabihf g++-arm-linux-gnueabihf libssl-dev:armhf libcap-dev:armhf zlib1g-dev:armhf libbz2-dev:armhf; \
      if [[ ! -x /root/.cargo/bin/rustup ]]; then curl https://sh.rustup.rs -sSf | sh -s -- -y; fi; \
      source /root/.cargo/env; \
      ${script_in_container} ${forwarded_args[*]}"
  )
  "${docker_cmd[@]}"
}

validate_pi3_abi_compat() {
  local bin_path="$1"
  local glibc_versions max_glibc

  require_cmd strings
  glibc_versions="$(strings "${bin_path}" | grep -o 'GLIBC_[0-9]\+\.[0-9]\+' | sed 's/GLIBC_//' | sort -Vu || true)"
  if [[ -z "${glibc_versions}" ]]; then
    return 0
  fi
  max_glibc="$(tail -n1 <<<"${glibc_versions}")"
  if ! awk -v v="${max_glibc}" 'BEGIN { split(v, a, "."); exit !((a[1] < 2) || (a[1] == 2 && a[2] <= 28)) }'; then
    echo "Built binary is not Pi OS Buster compatible (requires GLIBC_${max_glibc})." >&2
    echo "Re-run with --build-env=docker-buster (or keep default --build-env=auto)." >&2
    exit 1
  fi
  if strings "${bin_path}" | grep -q 'libssl\.so\.3'; then
    echo "Built binary links against OpenSSL 3 (libssl.so.3), but Pi OS Buster provides OpenSSL 1.1." >&2
    echo "Re-run with --build-env=docker-buster (or keep default --build-env=auto)." >&2
    exit 1
  fi
}

publish_local_artifacts() {
  local bin_path="$1"
  local version="$2"
  local profile="$3"
  local target="$4"
  local release_tag="$5"
  local out_dir artifact_base tarball checksum_file metadata_file
  out_dir="${REPO_DIR}/dist/local-armv7/${version}"
  artifact_base="codex-${target}-${version}-${profile}"
  tarball="${out_dir}/${artifact_base}.tar.gz"
  checksum_file="${tarball}.sha256"
  metadata_file="${out_dir}/${artifact_base}.json"

  mkdir -p "${out_dir}"

  install -m 0755 "${bin_path}" "${out_dir}/${artifact_base}"
  tar -C "${out_dir}" -czf "${tarball}" "${artifact_base}"
  sha256sum "${tarball}" > "${checksum_file}"

  python3 - <<'PY' "${metadata_file}" "${artifact_base}" "${target}" "${profile}" "${version}" "${release_tag}" "${RUSTY_V8_RELEASE_REPO}" "${RUSTY_V8_LOCAL_PATH}" "${bin_path}"
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

metadata_path = Path(sys.argv[1])
artifact_base = sys.argv[2]
target = sys.argv[3]
profile = sys.argv[4]
version = sys.argv[5]
release_tag = sys.argv[6]
release_repo = sys.argv[7]
local_rusty_v8_path = sys.argv[8]
binary_path = sys.argv[9]

metadata = {
    "createdAtUtc": datetime.now(timezone.utc).isoformat(),
    "artifactBase": artifact_base,
    "target": target,
    "profile": profile,
    "version": version,
    "rustyV8ReleaseRepo": release_repo,
    "rustyV8ReleaseTag": release_tag,
    "localRustyV8Path": local_rusty_v8_path,
    "binaryPath": binary_path,
}
metadata_path.write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
PY

  echo "Published local artifacts:"
  echo "- Binary: ${out_dir}/${artifact_base}"
  echo "- Tarball: ${tarball}"
  echo "- SHA256: ${checksum_file}"
  echo "- Metadata: ${metadata_file}"
}

resolve_github_release_repo() {
  local origin_url
  origin_url="$(git -C "${REPO_DIR}" config --get remote.origin.url || true)"
  if [[ "${origin_url}" =~ github\.com[:/]+([^/]+)/([^/]+)(\.git)?$ ]]; then
    echo "${BASH_REMATCH[1]}/${BASH_REMATCH[2]%\.git}"
    return 0
  fi
  return 1
}

publish_github_artifacts() {
  local version="$1"
  local profile="$2"
  local target="$3"
  local out_dir="$4"
  local artifact_base="$5"
  local release_repo release_tag release_title tarball checksum_file metadata_file binary_file
  local -a upload_files

  require_cmd gh
  release_repo="${GITHUB_RELEASE_REPO}"
  if [[ -z "${release_repo}" ]]; then
    release_repo="$(resolve_github_release_repo)"
  fi
  if [[ -z "${release_repo}" ]]; then
    echo "Unable to resolve GitHub release repo. Set --github-release-repo=<owner/repo>." >&2
    exit 1
  fi

  release_tag="${GITHUB_RELEASE_TAG}"
  if [[ -z "${release_tag}" ]]; then
    release_tag="codex-armv7-local-v${version}"
  fi
  release_title="Codex armv7 local build ${version}"

  tarball="${out_dir}/${artifact_base}.tar.gz"
  checksum_file="${tarball}.sha256"
  metadata_file="${out_dir}/${artifact_base}.json"
  binary_file="${out_dir}/${artifact_base}"
  upload_files=("${binary_file}" "${tarball}" "${checksum_file}" "${metadata_file}")

  if ! gh auth status >/dev/null 2>&1; then
    echo "GitHub CLI is not authenticated. Run: gh auth login" >&2
    exit 1
  fi

  if ! gh release view "${release_tag}" --repo "${release_repo}" >/dev/null 2>&1; then
    echo "Creating GitHub release ${release_tag} in ${release_repo}..."
    gh release create "${release_tag}" \
      --repo "${release_repo}" \
      --title "${release_title}" \
      --notes "Automated armv7 local build artifacts for ${version} (${profile}, ${target})."
  fi

  echo "Uploading artifacts to GitHub release ${release_tag} (${release_repo})..."
  gh release upload "${release_tag}" "${upload_files[@]}" --repo "${release_repo}" --clobber
  echo "Published GitHub artifacts: https://github.com/${release_repo}/releases/tag/${release_tag}"
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
    --build-env=*)
      BUILD_ENV="${arg#*=}"
      ;;
    --allow-non-arm-host)
      # Backward-compatible no-op.
      ;;
    --rusty-v8-release-repo=*)
      RUSTY_V8_RELEASE_REPO="${arg#*=}"
      ;;
    --rusty-v8-release-tag=*)
      RUSTY_V8_RELEASE_TAG="${arg#*=}"
      ;;
    --rusty-v8-local-path=*)
      RUSTY_V8_LOCAL_PATH="${arg#*=}"
      ;;
    --publish-github)
      PUBLISH_GITHUB="true"
      ;;
    --no-publish-github)
      PUBLISH_GITHUB="false"
      ;;
    --github-release-repo=*)
      GITHUB_RELEASE_REPO="${arg#*=}"
      ;;
    --github-release-tag=*)
      GITHUB_RELEASE_TAG="${arg#*=}"
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
if [[ ! "${host_arch}" =~ ^(armv7|armv6|armhf|arm)$ ]]; then
  echo "Host architecture is ${host_arch}; using cross-compile mode for ${TARGET}."
fi

if should_use_docker_buster; then
  run_in_docker_buster
  exit 0
fi

require_cmd cargo
require_cmd rustup
require_cmd python3
require_cmd file
require_cmd mktemp

if [[ "${TARGET}" == "armv7-unknown-linux-gnueabihf" ]]; then
  if ! command -v arm-linux-gnueabihf-gcc >/dev/null 2>&1; then
    ensure_armv7_cross_packages
  fi
  if ! command -v arm-linux-gnueabihf-gcc >/dev/null 2>&1; then
    echo "Missing required cross linker: arm-linux-gnueabihf-gcc" >&2
    exit 1
  fi
  setup_armv7_pkg_config_env
fi

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
  if [[ -f "${tmp_lock_backup:-}" ]]; then
    cp "${tmp_lock_backup}" "${CARGO_LOCK_PATH}"
  fi
  rm -rf "${tmp_dir}"
}
trap cleanup EXIT INT TERM

tmp_lock_backup="$(mktemp)"
cp "${CARGO_LOCK_PATH}" "${tmp_lock_backup}"

mkdir -p "${ARMV7_CACHE_DIR}"

binding_path="${ARMV7_CACHE_DIR}/src_binding_release_${release_tag}_${TARGET}.rs"
if [[ ! -f "${binding_path}" ]]; then
  echo "Fetching rusty_v8 binding: ${base_url}/src_binding_release_${TARGET}.rs"
  download_file "${base_url}/src_binding_release_${TARGET}.rs" "${binding_path}"
fi

export RUSTY_V8_ARCHIVE="${archive_url}"
export RUSTY_V8_SRC_BINDING_PATH="${binding_path}"
if [[ "${TARGET}" == "armv7-unknown-linux-gnueabihf" ]]; then
  export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER="${CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER:-arm-linux-gnueabihf-gcc}"
fi

cargo_args=(+"${TOOLCHAIN}" build -p codex-cli --locked --target "${TARGET}")
if [[ "${PROFILE}" == "release" ]]; then
  cargo_args+=(--release)
fi
if [[ "${TARGET}" == "armv7-unknown-linux-gnueabihf" ]]; then
  # Codex pins v8 = <resolved_v8_version>; prepare a matching patched source.
  patched_v8_dir="${ARMV7_CACHE_DIR}/v8-${resolved_v8_version}-armv7-patched"
  echo "Preparing patched v8 source from ${RUSTY_V8_LOCAL_PATH} (${resolved_v8_version})..."
  prepare_patched_v8_source "${RUSTY_V8_LOCAL_PATH}" "${resolved_v8_version}" "${patched_v8_dir}"
  cargo_args+=(
    --config
    "patch.crates-io.v8.path=\"${patched_v8_dir}\""
  )
fi

echo "Building codex-cli (${PROFILE}) for ${TARGET}..."
echo "Using prebuilt rusty_v8 from ${RUSTY_V8_RELEASE_REPO} (${release_tag})"
build_log="$(mktemp)"
set +e
cargo "${cargo_args[@]}" 2>&1 | tee "${build_log}"
status=${PIPESTATUS[0]}
set -e
if (( status != 0 )); then
  if grep -q "cannot update the lock file .*Cargo.lock because --locked was passed" "${build_log}"; then
    echo "Locked build failed; retrying without --locked..."
    cargo_args=(+"${TOOLCHAIN}" build -p codex-cli --target "${TARGET}")
    if [[ "${PROFILE}" == "release" ]]; then
      cargo_args+=(--release)
    fi
    if [[ "${TARGET}" == "armv7-unknown-linux-gnueabihf" ]]; then
      cargo_args+=(
        --config
        "patch.crates-io.v8.path=\"${patched_v8_dir}\""
      )
    fi
    cargo "${cargo_args[@]}"
  else
    rm -f "${build_log}"
    exit "${status}"
  fi
fi
rm -f "${build_log}"

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
if [[ "${TARGET}" == "armv7-unknown-linux-gnueabihf" ]]; then
  validate_pi3_abi_compat "${bin_path}"
fi

version="$(resolve_codex_version)"
if [[ -n "${version}" ]]; then
  publish_local_artifacts "${bin_path}" "${version}" "${PROFILE}" "${TARGET}" "${release_tag}"
  if [[ "${PUBLISH_GITHUB}" == "true" ]]; then
    artifact_base="codex-${TARGET}-${version}-${PROFILE}"
    artifact_dir="${REPO_DIR}/dist/local-armv7/${version}"
    publish_github_artifacts "${version}" "${PROFILE}" "${TARGET}" "${artifact_dir}" "${artifact_base}"
  fi
fi
