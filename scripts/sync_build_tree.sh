#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source_repo="${1:-$(cd -- "${script_dir}/.." && pwd -P)}"
build_repo="${2:-${source_repo}/../codex.build}"

if [[ ! -d "${source_repo}/codex-rs" ]]; then
  echo "source checkout not found at ${source_repo}" >&2
  exit 1
fi
if [[ ! -d "${build_repo}/codex-rs" ]]; then
  if [[ -e "${build_repo}" ]] && [[ -n "$(find "${build_repo}" -mindepth 1 -maxdepth 1 -print -quit 2>/dev/null)" ]]; then
    echo "build tree exists but is not a Codex checkout: ${build_repo}" >&2
    exit 1
  fi
  if ! mkdir -p "${build_repo}"; then
    echo "cannot create build tree at ${build_repo}; check that its parent is writable, or pass a build tree path as the second argument" >&2
    exit 1
  fi
fi
source_repo_real="$(cd -- "${source_repo}" && pwd -P)"
build_repo_real="$(cd -- "${build_repo}" && pwd -P)"
if [[ "${source_repo_real}" == "${build_repo_real}" && -L "${build_repo}" ]]; then
  echo "Using source checkout as the build tree via symlink: ${build_repo}; no sync needed" >&2
  exit 0
fi
case "${build_repo_real}/" in
  "${source_repo_real}/"*)
    echo "build tree must be outside the source checkout: ${build_repo_real}" >&2
    exit 1
    ;;
esac
if [[ "${source_repo_real}" == "${build_repo_real}" ]]; then
  echo "source and build trees must be different" >&2
  exit 1
fi

if ! command -v cpto >/dev/null 2>&1; then
  echo "cpto is required to synchronize source and build trees" >&2
  exit 1
fi
exec cpto --no-lngit "${source_repo}" "${build_repo}"
