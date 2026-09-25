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

# cpto can leave a nested codex-rs/.git worktree pointer behind after its
# temporary worktree metadata has been pruned. If the build root itself is a
# valid checkout, remove only that broken nested marker so Git resolves through
# the build root instead of failing in codex-rs.
nested_git_pointer="${build_repo}/codex-rs/.git"
if [[ -f "${nested_git_pointer}" ]] \
  && ! git -C "${build_repo}/codex-rs" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  build_git_root="$(git -C "${build_repo}" rev-parse --show-toplevel 2>/dev/null || true)"
  if [[ -n "${build_git_root}" ]] \
    && [[ "$(cd -- "${build_git_root}" && pwd -P)" == "${build_repo_real}" ]]; then
    echo "Removing stale nested Git worktree pointer: ${nested_git_pointer}" >&2
    rm -- "${nested_git_pointer}"
  fi
fi

if command -v cpto >/dev/null 2>&1; then
  exec cpto "${source_repo}" "${build_repo}"
fi

echo "cpto not found; syncing source without deleting build artifacts" >&2
tar --exclude='./.git' -cf - -C "${source_repo}" . | tar -xf - -C "${build_repo}"
