#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACT_DIR="${1:-${ROOT}.build/build/npm-artifact}"
DRY_RUN=false

if [[ "${ARTIFACT_DIR}" == --help || "${ARTIFACT_DIR}" == -h ]]; then
  cat <<'EOF'
Usage: scripts/publish_npm_local.sh [ARTIFACT_DIR] [--dry-run]

Validates and publishes the complete @reb.ai/codex npm artifact set locally.
The registry is checked before publishing because npm versions are immutable.
EOF
  exit 0
fi

if [[ "${2:-}" == --dry-run || "${1:-}" == --dry-run ]]; then
  DRY_RUN=true
  [[ "${1:-}" == --dry-run ]] && ARTIFACT_DIR="${ROOT}.build/build/npm-artifact"
fi

ARTIFACT_DIR="$(cd -- "${ARTIFACT_DIR}" && pwd)"
require_cmd() { command -v "$1" >/dev/null 2>&1 || { echo "missing required command: $1" >&2; exit 1; }; }
require_cmd jq
require_cmd npm
require_cmd sha256sum
require_cmd tar

mapfile -t archives < <(find "${ARTIFACT_DIR}" -maxdepth 1 -type f -name 'codex-npm-*.tgz' -printf '%f\n' | sort)
(( ${#archives[@]} == 9 )) || {
  echo "Expected 9 Codex npm archives in ${ARTIFACT_DIR}; found ${#archives[@]}." >&2
  exit 1
}

platforms=(linux-x64 linux-arm64 darwin-x64 darwin-arm64 win32-x64 win32-arm64 linux-armv7 android-arm64)
declare -A platform_archives
root_archive=""
version=""
for filename in "${archives[@]}"; do
  archive="${ARTIFACT_DIR}/${filename}"
  checksum="${archive}.sha256"
  [[ -f "${checksum}" ]] || { echo "Missing checksum: ${checksum}" >&2; exit 1; }
  (cd "${ARTIFACT_DIR}" && sha256sum --check "$(basename -- "${checksum}")") >/dev/null

  metadata="$(tar -xOf "${archive}" package/package.json)"
  name="$(jq -r '.name' <<<"${metadata}")"
  package_version="$(jq -r '.version' <<<"${metadata}")"
  [[ "${name}" == @reb.ai/codex ]] || { echo "Unexpected package name: ${name}" >&2; exit 1; }
  [[ "${package_version}" != null && "${package_version}" != "" ]] || { echo "Missing package version: ${filename}" >&2; exit 1; }
  matched_platform=""
  for platform in "${platforms[@]}"; do
    if [[ "${package_version}" == *"-${platform}" ]]; then
      matched_platform="${platform}"
      break
    fi
  done
  if [[ -n "${matched_platform}" ]]; then
    base_version="${package_version%-${matched_platform}}"
    [[ -z "${version}" || "${version}" == "${base_version}" ]] || {
      echo "Version mismatch: ${filename} has ${base_version}, expected ${version}." >&2
      exit 1
    }
    version="${base_version}"
    [[ -z "${platform_archives[${matched_platform}]:-}" ]] || {
      echo "Duplicate platform package: ${matched_platform}" >&2
      exit 1
    }
    platform_archives["${matched_platform}"]="${archive}"
  else
    [[ -z "${root_archive}" ]] || { echo "Duplicate root package archive." >&2; exit 1; }
    root_archive="${archive}"
    [[ -z "${version}" || "${version}" == "${package_version}" ]] || {
      echo "Version mismatch: ${filename} has ${package_version}, expected ${version}." >&2
      exit 1
    }
    version="${package_version}"
  fi
done

[[ -n "${root_archive}" ]] || { echo "Missing root package archive." >&2; exit 1; }
for platform in "${platforms[@]}"; do
  [[ -n "${platform_archives[${platform}]:-}" ]] || {
    echo "Missing platform package: ${platform}" >&2
    exit 1
  }
done

root_metadata="$(tar -xOf "${root_archive}" package/package.json)"
for platform in "${platforms[@]}"; do
  name="@reb.ai/codex-${platform}"
  dependency="$(jq -r --arg name "${name}" '.optionalDependencies[$name] // empty' <<<"${root_metadata}")"
  expected_dependency="npm:@reb.ai/codex@${version}-${platform}"
  [[ "${dependency}" == "${expected_dependency}" ]] || {
    echo "Root optional dependency ${name} is ${dependency}, expected ${expected_dependency}." >&2
    exit 1
  }
done

if npm view "@reb.ai/codex@${version}" version >/dev/null 2>&1; then
  echo "@reb.ai/codex@${version} already exists on npm; refusing immutable republish." >&2
  exit 2
fi
for platform in "${platforms[@]}"; do
  if npm view "@reb.ai/codex@${version}-${platform}" version >/dev/null 2>&1; then
    echo "@reb.ai/codex@${version}-${platform} already exists on npm; refusing immutable republish." >&2
    exit 2
  fi
done

if [[ "${DRY_RUN}" != true ]]; then
  npm whoami >/dev/null
fi
for platform in "${platforms[@]}"; do
  archive="${platform_archives[${platform}]}"
  tag="${platform}"
  if [[ "${version}" == *-* ]]; then
    tag="alpha-${platform}"
  fi
  command=(npm publish "${archive}" --access public --tag "${tag}")
  [[ "${DRY_RUN}" == true ]] && command+=(--dry-run)
  echo "+ ${command[*]}"
  "${command[@]}"
done

root_tag=latest
[[ "${version}" == *-* ]] && root_tag=alpha
command=(npm publish "${root_archive}" --access public --tag "${root_tag}")
[[ "${DRY_RUN}" == true ]] && command+=(--dry-run)
echo "+ ${command[*]}"
"${command[@]}"
