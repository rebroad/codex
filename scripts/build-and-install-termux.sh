#!/usr/bin/env sh

set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
PROFILE="${CODEX_BUILD_PROFILE:-debug}"
JOBS="${CARGO_BUILD_JOBS:-1}"

if [ "$(uname -m)" != "aarch64" ]; then
  echo "Codex for Termux requires an ARM64 device (uname -m must be aarch64)." >&2
  exit 1
fi

if ! command -v pkg >/dev/null 2>&1; then
  echo "Run this script inside Termux." >&2
  exit 1
fi

missing=0
for command_name in cargo aarch64-linux-android-clang llvm-strip python3 npm; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    missing=1
  fi
done

if [ "$missing" -ne 0 ]; then
  echo "Installing Termux build dependencies..."
  pkg install -y clang lld llvm make nodejs-lts openssl perl pkg-config python ripgrep rust
fi

for command_name in cargo aarch64-linux-android-clang llvm-strip python3 npm; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "Required command is still missing: $command_name" >&2
    exit 1
  fi
done

echo "Fetching the pinned Android rusty_v8 artifacts..."
v8_environment=$(python3 "$ROOT/scripts/fetch_rusty_v8_android.py")
eval "$(printf '%s\n' "$v8_environment" | sed -n '/^export /p')"

builtins=$(find "$PREFIX/lib/clang" -name 'libclang_rt.builtins-aarch64-android.a' -print -quit 2>/dev/null || true)
if [ -z "$builtins" ]; then
  echo "Could not find libclang_rt.builtins-aarch64-android.a under $PREFIX/lib/clang." >&2
  exit 1
fi

export CARGO_BUILD_JOBS="$JOBS"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=aarch64-linux-android-clang
export CC_aarch64_linux_android=aarch64-linux-android-clang
export CXX_aarch64_linux_android=aarch64-linux-android-clang++
export RUSTFLAGS="-Clink-arg=-lc++_shared -Clink-arg=-Wl,-rpath,\$ORIGIN -Clink-arg=$builtins"

case "$PROFILE" in
  debug)
    export CARGO_PROFILE_DEV_DEBUG=0
    export CARGO_PROFILE_DEV_INCREMENTAL=false
    echo "Building Codex (debug profile, $JOBS job(s))..."
    (cd "$ROOT/codex-rs" && cargo build -p codex-cli)
    binary_path="$ROOT/codex-rs/target/debug/codex"
    ;;
  release)
    echo "Building Codex (release profile, $JOBS job(s))..."
    (cd "$ROOT/codex-rs" && cargo build --release -p codex-cli)
    binary_path="$ROOT/codex-rs/target/release/codex"
    ;;
  *)
    echo "CODEX_BUILD_PROFILE must be debug or release." >&2
    exit 1
    ;;
esac

package_bin="$ROOT/npm-package/bin"
mkdir -p "$package_bin"
cp "$binary_path" "$package_bin/codex.bin"
cp "$PREFIX/lib/libc++_shared.so" "$package_bin/libc++_shared.so"
llvm-strip --strip-all "$package_bin/codex.bin" "$package_bin/libc++_shared.so"
chmod 0755 "$package_bin/codex.bin"
chmod 0644 "$package_bin/libc++_shared.so"

echo "Installing the local npm package and its Codex wrapper..."
npm install --global "$ROOT/npm-package"

global_package_root="$(npm root --global)/@mmmbuto/codex-cli-termux"
installed_binary="$global_package_root/bin/codex.bin"
if [ ! -x "$installed_binary" ]; then
  echo "Installed package is missing its native binary: $installed_binary" >&2
  exit 1
fi

echo "Verifying the installed wrapper..."
codex --version
codex --help >/dev/null
echo "Installed native binary: $installed_binary"
