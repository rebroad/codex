#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
BUILD_TREE=""
for candidate in "${ROOT}.build" "${ROOT}.make"; do
  if [[ -d "$candidate" ]]; then
    BUILD_TREE="$candidate"
    break
  fi
done
if [[ -z "$BUILD_TREE" ]]; then
  echo "No sibling build tree found; expected ${ROOT}.build or ${ROOT}.make" >&2
  exit 1
fi

VERSION=${1:-}
if [[ -z "$VERSION" ]]; then
  VERSION=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/codex-rs/Cargo.toml" | head -n 1)
fi
if [[ -z "$VERSION" ]]; then
  echo "Could not determine the package version" >&2
  exit 1
fi

LINUX_BINARY="$BUILD_TREE/cargo-target-linux/release/codex"
ANDROID_STAGE="$BUILD_TREE/android-artifact-hybrid"
ANDROID_BINARY="$ANDROID_STAGE/codex.bin"
ANDROID_LIBCXX="$ANDROID_STAGE/libc++_shared.so"
OUTPUT_DIR="$BUILD_TREE/npm-artifact-hybrid"
STAGE_ROOT=$(mktemp -d "$BUILD_TREE/npm-stage-hybrid.XXXXXX")
trap 'rm -rf "$STAGE_ROOT"' EXIT

if [[ ! -x "$LINUX_BINARY" ]]; then
  echo "Missing Linux binary: $LINUX_BINARY" >&2
  echo "Run scripts/build-hybrid.sh linux first." >&2
  exit 1
fi
if [[ ! -x "$ANDROID_BINARY" || ! -f "$ANDROID_LIBCXX" ]]; then
  echo "Missing Android artifacts under $ANDROID_STAGE" >&2
  echo "Run scripts/build-hybrid.sh android first." >&2
  exit 1
fi
if ! command -v npm >/dev/null 2>&1; then
  echo "npm is required to create the package archives" >&2
  exit 1
fi
if [[ -n "${STRIP:-}" ]]; then
  if ! command -v "$STRIP" >/dev/null 2>&1 && [[ ! -x "$STRIP" ]]; then
    echo "Could not find strip tool: $STRIP" >&2
    exit 1
  fi
else
  for candidate in llvm-strip strip; do
    if command -v "$candidate" >/dev/null 2>&1; then
      STRIP=$candidate
      break
    fi
  done
  if [[ -z "${STRIP:-}" ]]; then
    echo "Could not find llvm-strip or strip" >&2
    exit 1
  fi
fi

mkdir -p "$OUTPUT_DIR"

write_package_json() {
  local package_dir=$1
  local package_name=$2
  local os_name=$3
  local cpu_name=$4
  mkdir -p "$package_dir"
  cat >"$package_dir/package.json" <<EOF
{
  "name": "$package_name",
  "version": "$VERSION",
  "description": "Codex CLI $os_name/$cpu_name package with a direct native binary",
  "license": "Apache-2.0",
  "os": ["$os_name"],
  "cpu": ["$cpu_name"],
  "bin": {
    "codex": "bin/codex"
  },
  "files": ["bin"]
}
EOF
  cat >"$package_dir/README.md" <<EOF
# $package_name

Codex CLI $VERSION for $os_name/$cpu_name.

This package exposes the native codex executable directly; it does not
require Node.js at runtime. Install it globally with npm, or use the bin
entry from the package in another npm installation.
EOF
}

package_linux() {
  local package_dir="$STAGE_ROOT/codex-cli-hybrid-linux-x64"
  write_package_json "$package_dir" "@reb.ai/codex-cli-hybrid-linux-x64" linux x64
  mkdir -p "$package_dir/bin"
  install -m 0755 "$LINUX_BINARY" "$package_dir/bin/codex"
  "$STRIP" --strip-all "$package_dir/bin/codex"
  npm pack --ignore-scripts --pack-destination "$OUTPUT_DIR" "$package_dir"
}

package_android() {
  local package_dir="$STAGE_ROOT/codex-cli-hybrid-android-arm64"
  write_package_json "$package_dir" "@reb.ai/codex-cli-hybrid-android-arm64" android arm64
  mkdir -p "$package_dir/bin"
  install -m 0755 "$ANDROID_BINARY" "$package_dir/bin/codex"
  install -m 0644 "$ANDROID_LIBCXX" "$package_dir/bin/libc++_shared.so"
  npm pack --ignore-scripts --pack-destination "$OUTPUT_DIR" "$package_dir"
}

package_linux
package_android

echo "Created npm packages in $OUTPUT_DIR"
ls -lh "$OUTPUT_DIR"/*.tgz
