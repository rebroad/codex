#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
UPSTREAM_REMOTE="${UPSTREAM_REMOTE:-upstream}"
PUBLISH_REMOTE="${PUBLISH_REMOTE:-origin}"
FORK_POINT_BRANCH="${FORK_POINT_BRANCH:-upstream}"
UPSTREAM_ALPHA_BRANCH="${UPSTREAM_ALPHA_BRANCH:-upstream/latest-alpha-cli}"

die() {
  echo "rebase-alpha: $*" >&2
  exit 1
}

git -C "${ROOT}" rev-parse --show-toplevel >/dev/null || die "not a Git checkout"
cd -- "${ROOT}"

[[ "$(git branch --show-current)" == "alpha" ]] || die "must run on the alpha branch"
[[ -z "$(git status --porcelain)" ]] || die "working tree must be clean"

git_dir="$(git rev-parse --git-dir)"
[[ ! -d "${git_dir}/rebase-merge" && ! -d "${git_dir}/rebase-apply" ]] || \
  die "a rebase is already in progress"

git remote get-url "${UPSTREAM_REMOTE}" >/dev/null || \
  die "remote not found: ${UPSTREAM_REMOTE}"
git remote get-url "${PUBLISH_REMOTE}" >/dev/null || \
  die "remote not found: ${PUBLISH_REMOTE}"

echo "Fetching ${UPSTREAM_REMOTE}..."
git fetch "${UPSTREAM_REMOTE}"

git rev-parse --verify "${FORK_POINT_BRANCH}^{commit}" >/dev/null || \
  die "fork point not found: ${FORK_POINT_BRANCH}"
git rev-parse --verify "${UPSTREAM_ALPHA_BRANCH}^{commit}" >/dev/null || \
  die "upstream alpha branch not found: ${UPSTREAM_ALPHA_BRANCH}"

fork_point_ref="$(git rev-parse --symbolic-full-name "${FORK_POINT_BRANCH}")"
[[ "${fork_point_ref}" == refs/heads/* ]] || \
  die "fork point must be a local branch: ${FORK_POINT_BRANCH}"
fork_point_commit="$(git rev-parse "${FORK_POINT_BRANCH}^{commit}")"
upstream_alpha_commit="$(git rev-parse "${UPSTREAM_ALPHA_BRANCH}^{commit}")"

version_from_ref() {
  local ref="$1"
  git show "${ref}:codex-rs/Cargo.toml" |
    sed -n 's/^version = "\([^"]*\)"/\1/p' |
    head -n 1
}

fork_point_version="$(version_from_ref "${fork_point_commit}")"
upstream_alpha_version="$(version_from_ref "${upstream_alpha_commit}")"
[[ -n "${fork_point_version}" ]] || die "could not determine ${FORK_POINT_BRANCH} version"
[[ -n "${upstream_alpha_version}" ]] || \
  die "could not determine ${UPSTREAM_ALPHA_BRANCH} version"

if [[ "${fork_point_version}" == "${upstream_alpha_version}" ]] || \
   [[ "$(printf '%s\n' "${fork_point_version}" "${upstream_alpha_version}" | sort -V | tail -n 1)" != "${upstream_alpha_version}" ]]; then
  echo "No rebase needed: ${UPSTREAM_ALPHA_BRANCH} is ${upstream_alpha_version}; fork point is ${fork_point_version}."
  exit 0
fi

backup_branch="alpha.old.$(date -u +%Y%m%d%H%M%S)"
git show-ref --verify --quiet "refs/heads/${backup_branch}" && \
  die "backup branch already exists locally: ${backup_branch}"
git ls-remote --exit-code --quiet "${PUBLISH_REMOTE}" \
  "refs/heads/${backup_branch}" >/dev/null 2>&1 && \
  die "backup branch already exists on ${PUBLISH_REMOTE}: ${backup_branch}"

echo "Creating and pushing backup ${backup_branch}..."
git branch "${backup_branch}" HEAD
git branch -f alpha.old "${backup_branch}"
git push "${PUBLISH_REMOTE}" "${backup_branch}"
echo "Updating ${PUBLISH_REMOTE}/alpha.old..."
git push --force-with-lease "${PUBLISH_REMOTE}" \
  "${backup_branch}:refs/heads/alpha.old"

echo "Rebasing alpha onto ${UPSTREAM_ALPHA_BRANCH} (${upstream_alpha_version})..."
sequence_editor="sed -i '1i update-ref ${fork_point_ref}'"
GIT_SEQUENCE_EDITOR="${sequence_editor}" git rebase --interactive --no-update-refs \
  --onto "${upstream_alpha_commit}" "${fork_point_commit}" alpha
