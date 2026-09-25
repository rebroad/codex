#!/usr/bin/env bash
set -euo pipefail

source_repo="${1:?source checkout path is required}"
build_repo="${2:?build tree path is required}"

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
    echo "cannot create build tree at ${build_repo}; check that the external build volume is mounted and writable, or set CODEX_BUILD_REPO to another external path" >&2
    exit 1
  fi
fi
source_repo_real="$(cd -- "${source_repo}" && pwd -P)"
build_repo_real="$(cd -- "${build_repo}" && pwd -P)"
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

# An old copy can leave a nested codex-rs/.git worktree pointer behind after
# its temporary worktree metadata has been pruned. If the build root itself is
# a valid checkout, remove only that broken nested marker so Git resolves
# through the build root instead of failing in codex-rs.
nested_git_pointer="${build_repo}/codex-rs/.git"
source_nested_git_pointer="${source_repo}/codex-rs/.git"
if [[ -f "${nested_git_pointer}" ]] \
  && [[ ! -e "${source_nested_git_pointer}" ]]; then
  build_git_root="$(git -C "${build_repo}" rev-parse --show-toplevel 2>/dev/null || true)"
  nested_git_root="$(git -C "${build_repo}/codex-rs" rev-parse --show-toplevel 2>/dev/null || true)"
  if [[ -n "${build_git_root}" ]] \
    && [[ "$(cd -- "${build_git_root}" && pwd -P)" == "${build_repo_real}" ]] \
    && [[ -n "${nested_git_root}" ]] \
    && [[ "$(cd -- "${nested_git_root}" && pwd -P)" == "${build_repo_real}/codex-rs" ]]; then
    echo "Removing stale nested Git worktree pointer: ${nested_git_pointer}" >&2
    rm -- "${nested_git_pointer}"
  fi
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
if ! command -v cpto >/dev/null 2>&1; then
  echo "cpto is required to synchronize source and build trees" >&2
  exit 1
fi
exec cpto --lngit "${source_repo}" "${build_repo}"
