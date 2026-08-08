#!/usr/bin/env bash
set -euo pipefail

: "${TARGET:?TARGET is required}"
: "${VERSION:?VERSION is required}"
: "${OUTPUT_DIR:?OUTPUT_DIR is required}"

case "${TARGET}" in
  x86_64-unknown-linux-musl) platform=linux-x64 ;;
  aarch64-unknown-linux-musl) platform=linux-arm64 ;;
  x86_64-apple-darwin) platform=darwin-x64 ;;
  aarch64-apple-darwin) platform=darwin-arm64 ;;
  x86_64-pc-windows-msvc) platform=win32-x64 ;;
  aarch64-pc-windows-msvc) platform=win32-arm64 ;;
  *)
    echo "Unsupported source npm target: ${TARGET}" >&2
    exit 1
    ;;
esac

target_dir="${CARGO_TARGET_DIR:-${RUNNER_TEMP:-/var/tmp}/codex-cargo-target}"
export CARGO_TARGET_DIR="${target_dir}"

build_args=(
  cargo build --release --target "${TARGET}"
  -p codex-cli
  -p codex-code-mode-host
)
case "${TARGET}" in
  *-unknown-linux-musl) build_args+=(-p codex-bwrap) ;;
  x86_64-pc-windows-msvc) export LIBSQLITE3_FLAGS=SQLITE_DISABLE_INTRINSIC ;;
esac

if ! (cd "${GITHUB_WORKSPACE}/codex-rs" && "${build_args[@]}" --locked); then
  echo "Locked source build failed; retrying offline." >&2
  (cd "${GITHUB_WORKSPACE}/codex-rs" && "${build_args[@]}" --offline)
fi

release_dir="${CARGO_TARGET_DIR}/${TARGET}/release"
vendor_dir="${RUNNER_TEMP:-/var/tmp}/codex-npm-vendor-${TARGET}"
rm -rf "${vendor_dir}"
mkdir -p "${vendor_dir}/${TARGET}/bin"

binary_suffix=""
[[ "${TARGET}" == *-pc-windows-msvc ]] && binary_suffix=.exe
cp "${release_dir}/codex${binary_suffix}" "${vendor_dir}/${TARGET}/bin/codex${binary_suffix}"
cp "${release_dir}/codex-code-mode-host${binary_suffix}" \
  "${vendor_dir}/${TARGET}/bin/codex-code-mode-host${binary_suffix}"
if [[ "${TARGET}" == *-unknown-linux-musl ]]; then
  mkdir -p "${vendor_dir}/${TARGET}/codex-resources"
  cp "${release_dir}/bwrap" "${vendor_dir}/${TARGET}/codex-resources/bwrap"
fi

mkdir -p "${OUTPUT_DIR}"
python3 "${GITHUB_WORKSPACE}/codex-cli/scripts/build_npm_package.py" \
  --release-version "${VERSION}" \
  --package "codex-${platform}" \
  --vendor-src "${vendor_dir}" \
  --pack-output "${OUTPUT_DIR}/codex-npm-${platform}-${VERSION}.tgz"
