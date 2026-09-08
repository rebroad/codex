#!/usr/bin/env bash
set -euo pipefail

source_repo="${1:?source checkout path is required}"
build_repo="${2:?build tree path is required}"

if [[ ! -d "${source_repo}/codex-rs" ]]; then
  echo "source checkout not found at ${source_repo}" >&2
  exit 1
fi
if [[ ! -d "${build_repo}/codex-rs" ]]; then
  echo "build tree not found at ${build_repo}" >&2
  exit 1
fi
if [[ "$(cd -- "${source_repo}" && pwd -P)" == "$(cd -- "${build_repo}" && pwd -P)" ]]; then
  echo "source and build trees must be different" >&2
  exit 1
fi

if command -v cpto >/dev/null 2>&1; then
  exec cpto "${source_repo}" "${build_repo}"
fi

echo "cpto not found; syncing source without deleting build artifacts" >&2
tar --exclude='./.git' -cf - -C "${source_repo}" . | tar -xf - -C "${build_repo}"
