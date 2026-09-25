#!/usr/bin/env bash
set -euo pipefail

openssl_version_from_lock() {
  local lock_file="${1}"
  awk '
    $0 == "name = \"openssl-src\"" { found = 1; next }
    found && /^version = / { split($0, fields, "\""); print fields[2]; exit }
    found && /^\[\[package\]\]/ { exit }
  ' "${lock_file}"
}

openssl_cache_dir() {
  local build_repo="${1}" target="${2}" version="${3}"
  printf '%s/build/openssl-artifacts/%s/%s\n' "${build_repo}" "${version}" "${target}"
}

openssl_cache_is_valid() {
  local cache_dir="${1}"
  [[ -s "${cache_dir}/lib/libssl.a" \
    && -s "${cache_dir}/lib/libcrypto.a" \
    && -s "${cache_dir}/include/openssl/opensslv.h" \
    && -s "${cache_dir}/.metadata" ]]
}

openssl_cache_metadata() {
  local cache_dir="${1}" version="${2}" target="${3}" host="${4}"
  printf 'openssl_src_version=%s\ntarget=%s\nhost=%s\n' \
    "${version}" "${target}" "${host}" >"${cache_dir}/.metadata"
}

openssl_cache_from_target() {
  local target_dir="${1}" build_repo="${2}" target="${3}" version="${4}" host="${5}"
  local cache_dir install_dir
  cache_dir="$(openssl_cache_dir "${build_repo}" "${target}" "${version}")"
  openssl_cache_is_valid "${cache_dir}" && return 0

  for install_dir in "${target_dir}"/debug/build/openssl-sys-*/out/openssl-build/install \
    "${target_dir}"/release/build/openssl-sys-*/out/openssl-build/install; do
    [[ -d "${install_dir}" ]] || continue
    [[ -s "${install_dir}/lib/libssl.a" \
      && -s "${install_dir}/lib/libcrypto.a" \
      && -s "${install_dir}/include/openssl/opensslv.h" ]] || continue
    mkdir -p "${cache_dir}"
    rm -rf "${cache_dir}/include" "${cache_dir}/lib"
    cp -a "${install_dir}/include" "${cache_dir}/include"
    cp -a "${install_dir}/lib" "${cache_dir}/lib"
    openssl_cache_metadata "${cache_dir}" "${version}" "${target}" "${host}"
    printf 'Cached OpenSSL %s for %s at %s\n' "${version}" "${target}" "${cache_dir}" >&2
    return 0
  done
  return 1
}

openssl_cache_env() {
  local cache_dir="${1}"
  openssl_cache_is_valid "${cache_dir}" || return 1
  printf 'OPENSSL_DIR=%q OPENSSL_NO_VENDOR=1 OPENSSL_STATIC=1\n' "${cache_dir}"
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  case "${1:-}" in
    version)
      [[ $# -eq 2 ]] || { echo "usage: $0 version CARGO_LOCK" >&2; exit 2; }
      openssl_version_from_lock "$2"
      ;;
    env)
      [[ $# -eq 4 ]] || { echo "usage: $0 env BUILD_REPO TARGET VERSION" >&2; exit 2; }
      openssl_cache_env "$(openssl_cache_dir "$2" "$3" "$4")"
      ;;
    cache)
      [[ $# -eq 6 ]] || { echo "usage: $0 cache TARGET_DIR BUILD_REPO TARGET VERSION HOST" >&2; exit 2; }
      openssl_cache_from_target "$2" "$3" "$4" "$5" "$6"
      ;;
    *)
      echo "usage: $0 version CARGO_LOCK | env BUILD_REPO TARGET VERSION | cache TARGET_DIR BUILD_REPO TARGET VERSION HOST" >&2
      exit 2
      ;;
  esac
fi
