# Building Codex CLI for Linux and Android

This repository keeps source edits in the primary checkout and builds in the
sibling build tree (`codex.build`, or `codex.make`). The Android target is
`aarch64-linux-android`; the native host target is Linux x86-64.

## Prerequisites

- Rust toolchain `1.95.0`, including the `aarch64-linux-android` target
- Android NDK `28.2.13676358` (or a compatible NDK)
- `cargo`, `rustup`, `file`, `sha256sum`, `tar`, and `llvm-strip`
- An offline Cargo cache containing the workspace dependencies

The Android compatibility delta is intentionally small: OpenSSL is vendored
for `aarch64-linux-android`, and the Android TLS/linker configuration is kept
in `codex-rs/.cargo/config.toml` and `codex-rs/cli/src/android_tls_alignment.rs`.

## Recommended builds

Run from the source checkout. The script synchronizes the source into the
sibling build tree and never compiles in the source tree:

```bash
# Native Linux release binary.
scripts/build.sh linux

# ARM64 Android/Termux release binary and stripped staging directory.
export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/28.2.13676358"
scripts/build.sh android
```

For the complete repeatable workflow, use `scripts/rebuild_codex.sh`. It
supports debug/release builds, the sibling-tree cache, Linux musl x64,
Linux ARMv7, Android ARM64, timestamped installs in `~/.cargo/bin`, npm
packaging, and opt-in npm/GitHub publishing:

```bash
scripts/rebuild_codex.sh --release
scripts/rebuild_codex.sh --release --package-npm
scripts/rebuild_codex.sh --release --target armv7
```

ARMv7 requires an installed `arm-linux-gnueabihf-gcc` (or set
`ARMV7_LINKER`). The musl and ARMv7 binaries are staged as platform variants
of `@reb.ai/codex`.

Outputs are:

```text
../codex.build/build/linux-release/release/codex
../codex.build/build/android-release/aarch64-linux-android/release/codex
../codex.build/build/android-artifact/codex.bin
../codex.build/build/android-artifact/libc++_shared.so
```

If `cpto` is available in `PATH`, the build script uses
`cpto --no-lngit --nogit` to preserve the build cache. It is optional; when it
is absent, the script uses a tar-based source sync that leaves generated build
artifacts in place. Build outputs are also listed in `.gitignore`.

## Exact Android build environment

The script expands to the following essential Cargo configuration:

```bash
NDK="$HOME/Android/Sdk/ndk/28.2.13676358"
LLVM="$NDK/toolchains/llvm/prebuilt/linux-x86_64"
builtins="$(find "$LLVM" -name libclang_rt.builtins-aarch64-android.a -print -quit)"

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
export CARGO_TARGET_DIR="$PWD/../codex.build/build/android-release"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-Clink-arg=-lc++_shared -Clink-arg=-Wl,-rpath,\$ORIGIN -Clink-arg=$builtins"

cd "$CARGO_TARGET_DIR/../codex-rs"
rustup run 1.95.0 cargo build --offline \
  --target aarch64-linux-android --release -p codex-cli
```

After building, strip only the deployment copy. Keep the unstripped Cargo
artifact for debugging:

```bash
mkdir -p $CARGO_TARGET_DIR/../android-artifact
cp target/aarch64-linux-android/release/codex $CARGO_TARGET_DIR/../android-artifact/codex.bin
cp "$LLVM/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so" \
  $CARGO_TARGET_DIR/../android-artifact/libc++_shared.so
llvm-strip --strip-all $CARGO_TARGET_DIR/../android-artifact/codex.bin
```

## Direct-binary npm packages

The build mirrors OpenAI's npm layout: `@reb.ai/codex` is a small
platform selector, while the native payloads are separate optional packages.
The selector is only needed for one cross-platform package name; each native
package still exposes its ELF directly.

```bash
scripts/package-npm.sh
```

The script derives the version from the workspace `codex-rs/Cargo.toml`. Pass a
version explicitly only when packaging a deliberate override.

This writes the packages to:

```text
../codex.build/build/npm-artifact/
```

Publish or install the three archives together when using the unified package:

```bash
npm install --global @reb.ai/codex
```

The generated archives are named `reb.ai-codex-*.tgz`. For local testing,
install the platform archive directly; it creates npm's normal global `codex`
link and points it directly at the native ELF. The unified package's selector
adds only platform detection and signal forwarding.

Ensure `$(npm prefix --global)/bin` is in `PATH`. Alternatively, copy a
platform package's `bin/codex` to `$HOME/.cargo/bin/codex`, which is already in
the standard Rust-user PATH on this machine. Keep `libc++_shared.so` beside the
Android executable when installing it manually.

## Debug versus release

Use release for the final Android binary: it is faster, smaller after
stripping, and lower-memory. Use debug for a diagnostic iteration when symbols
and fast incremental compilation matter more than runtime performance.
Preserve the unstripped release Cargo artifact when investigating startup
failures.
