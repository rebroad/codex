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

MODE=${1:-}
if [[ "$MODE" != android && "$MODE" != linux ]]; then
  echo "Usage: $0 android|linux" >&2
  exit 2
fi

# Keep generated Cargo output in the sibling build tree. If cpto is installed,
# use its cache-preserving mode; otherwise copy source files without replacing
# the build tree's metadata or deleting its generated artifacts.
if command -v cpto >/dev/null 2>&1; then
  cpto "$ROOT" "$BUILD_TREE"
else
  echo "cpto not found; syncing sources with tar" >&2
  tar --exclude='./.git' -cf - -C "$ROOT" . | tar -xf - -C "$BUILD_TREE"
fi

RUST_TOOLCHAIN=${RUST_TOOLCHAIN:-1.95.0}
export RUSTC_WRAPPER=
export CARGO_NET_OFFLINE=true

if [[ "$MODE" == linux ]]; then
  export CARGO_TARGET_DIR="$BUILD_TREE/build/linux-release"
  (cd "$BUILD_TREE/codex-rs" && \
    rustup run "$RUST_TOOLCHAIN" cargo build --offline --release -p codex-cli)
  file "$CARGO_TARGET_DIR/release/codex"
  sha256sum "$CARGO_TARGET_DIR/release/codex"
  exit 0
fi

NDK=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}
if [[ -z "$NDK" || ! -d "$NDK" ]]; then
  echo "Set ANDROID_NDK_HOME to the Android NDK directory" >&2
  exit 1
fi
LLVM="$NDK/toolchains/llvm/prebuilt/linux-x86_64"
if [[ ! -x "$LLVM/bin/aarch64-linux-android29-clang" ]]; then
  echo "Android NDK Clang not found under $LLVM" >&2
  exit 1
fi
builtins=$(find "$LLVM" -name libclang_rt.builtins-aarch64-android.a -print -quit)
if [[ -z "$builtins" ]]; then
  echo "Android compiler builtins archive not found" >&2
  exit 1
fi

export PATH="$LLVM/bin:$PATH"
export ANDROID_NDK_HOME="$NDK"
export LIBLZMA_NO_PKG_CONFIG=1
export BZIP2_NO_PKG_CONFIG=1
export BZIP2_STATIC=1
export PKG_CONFIG_ALLOW_CROSS=1
export CODEX_SKIP_VENDORED_BWRAP=1
export CC_aarch64_linux_android="$LLVM/bin/aarch64-linux-android29-clang"
export CXX_aarch64_linux_android="$LLVM/bin/aarch64-linux-android29-clang++"
export AR_aarch64_linux_android="$LLVM/bin/llvm-ar"
export RANLIB_aarch64_linux_android="$LLVM/bin/llvm-ranlib"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$LLVM/bin/aarch64-linux-android29-clang"
export CARGO_TARGET_DIR="$BUILD_TREE/build/android-release"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-Clink-arg=-lc++_shared -Clink-arg=-Wl,-rpath,\$ORIGIN -Clink-arg=$builtins"

(cd "$BUILD_TREE/codex-rs" && \
  rustup run "$RUST_TOOLCHAIN" cargo build --offline --target aarch64-linux-android --release -p codex-cli)

STAGE="$BUILD_TREE/build/android-artifact"
mkdir -p "$STAGE"
cp "$CARGO_TARGET_DIR/aarch64-linux-android/release/codex" "$STAGE/codex.bin"
cp "$LLVM/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so" "$STAGE/libc++_shared.so"
llvm-strip --strip-all "$STAGE/codex.bin"
chmod 0755 "$STAGE/codex.bin"
chmod 0644 "$STAGE/libc++_shared.so"
file "$STAGE/codex.bin"
sha256sum "$STAGE/codex.bin" "$STAGE/libc++_shared.so"
