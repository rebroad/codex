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
for `aarch64-linux-android`, and the Android linker configuration is kept in
`codex-rs/.cargo/config.toml` and `scripts/codex_cargo_env.sh`. Native Termux
builds link the retained input in `scripts/android_tls_alignment.S` into every
Cargo executable, including test harnesses, so ARM64 Bionic receives the
required 64-byte `PT_TLS` alignment.

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

For the complete repeatable workflow, use `scripts/build_codex.sh`. It
supports debug/release builds, the sibling-tree cache, Linux musl x64,
Linux ARMv7, Android ARM64, timestamped installs in `~/.cargo/bin`, npm
packaging, and opt-in npm/GitHub publishing:

```bash
scripts/build_codex.sh --release
scripts/build_codex.sh --release --package-local-npm
scripts/build_codex.sh --release --target armv7
scripts/build_codex.sh --release --target android --package-npm
scripts/build_codex.sh --release --target all --package-local-npm
# Full local assembly, audit, and npm publication.
scripts/build_codex.sh --release --publish-local-npm
# Start the GitHub build/release workflow; this does not compile locally.
scripts/build_codex.sh --release --start-github-release
```

ARMv7 requires an installed `arm-linux-gnueabihf-gcc` (or set
`ARMV7_LINKER`). The musl and ARMv7 binaries are staged as platform variants
of `@reb.ai/codex`. `build_codex.sh` requests sudo only when it actually
needs to install missing musl build tools; cached/reused artifact and Android
runs do not prompt for sudo.

To build and install sequentially to one or more SSH targets, pass their SSH
aliases as a comma-separated list. The script identifies each target over SSH
before selecting native, ARMv7, or Android compilation; `--install-dir` can
override the target's default user executable directory:

```bash
scripts/build_codex.sh --release --install target-a,target-b
```

For a Termux/Android operational install, use the existing user executable
directory so both interactive shells and Termux:Boot resolve the same binary:

```bash
scripts/build_codex.sh --release --install-dir "$HOME/bin"
install -Dm755 scripts/remote-control/codex-remote-start "$HOME/bin/codex-remote-start"
install -Dm755 scripts/remote-control/codex-pairing-code "$HOME/bin/codex-pairing-code"
install -Dm755 scripts/remote-control/codex-remote-healthcheck "$HOME/bin/codex-remote-healthcheck"
codex-remote-healthcheck
```

The build script deliberately does not edit shell startup files. If a
service uses a different executable, set `CODEX_BIN` explicitly and verify the
running daemon with the health check.

Outputs are:

```text
../codex.build/build/linux-release/release/codex
../codex.build/build/android-release/aarch64-linux-android/release/codex
../codex.build/build/android-artifact/codex.bin
../codex.build/build/android-artifact/libc++_shared.so
```

If `cpto` is available in `PATH`, the build script uses it to preserve the
build cache. Git-worktree metadata is synchronized when possible; if the
destination's `.git` metadata is unsuitable, `cpto` warns and continues with
the file synchronization. It is optional; when absent, the script uses a
tar-based source sync that leaves generated build artifacts in place. Build
outputs are also listed in `.gitignore`.

## Native Termux build process limit

Android tracks native commands started by Termux as phantom processes and
normally permits at most 32 across the device. A large Rust workspace can
cross that limit even with one Cargo job, causing `rustc` or `sccache` to exit
without a compiler diagnostic. For native builds on Android, connect the
device to its own wireless-debugging endpoint and run:

```bash
scripts/setup-dev-environment.sh
scripts/setup-dev-environment.sh --check
```

On Android, the setup script installs `android-tools` when needed, raises
`activity_manager.max_phantom_processes` to 128, and disables Android's child
process restrictions through the connected ADB device. Where Android supports
per-flag overrides, the script makes the numeric limit sticky so DeviceConfig
server sync cannot restore the default of 32. It leaves an existing value
higher than 128 unchanged. Set `ANDROID_SERIAL` when more than one device is
connected. These are persistent, device-wide resource-policy changes; the
child-process setting disables both phantom-process count trimming and
excessive-CPU enforcement for native processes started by Termux.

Android 14 and newer also expose **Disable child process restrictions** in
Developer options. The setup script applies the equivalent
`persist.sys.fflag.override.settings_enable_monitor_phantom_procs=false`
property through ADB, so the developer-option state is reproducible for
scripted Android builds.

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
# Stage locally built fork targets selected by the target list.
VERSION="$(scripts/npm_candidate_version.sh)"
scripts/package_npm.sh "$VERSION" release musl,armv7,android
```

The script derives the version from the workspace `codex-rs/Cargo.toml`. Pass a
version explicitly only when packaging a deliberate override.

This writes the packages to:

```text
../codex.build/build/npm-artifact/
```

Target selection is deliberately split from the host build matrix:

- `native` is the default executable build. With npm packaging it means the
  local Linux musl x64 package only.
- `musl`, `armv7`, and `android` select the locally buildable fork targets.
- `all` means the complete npm candidate: all eight supported architectures
  (`linux-x64`, `linux-arm64`, both macOS targets, both Windows targets,
  `linux-armv7`, and `android-arm64`). The local phase reuses/downloads the
  desktop payloads and builds the locally supported fork targets as needed.

For a complete nine-package candidate, use:

```bash
scripts/build_codex.sh --release --target all --package-local-npm
```

`--publish-local-npm` implies `--package-local-npm` and selects `all` by
default. If a target is explicitly supplied, it must be `all`. It downloads
the newest completed fork release when one exists, reuses or builds missing
local targets, assembles and audits all nine
packages, checks npm authentication and immutable-version collisions, then
publishes the eight platform packages before the root alias package. It fails
before publishing if any platform package is missing or invalid.

The GitHub workflow has two stages: one matrix job builds and packages one
immutable npm payload per architecture, then a single assembly job combines,
audits, and publishes the complete package set. The final npm job publishes
through npm Trusted Publishing. Rusty V8 is supplied by the upstream artifact
workflow for all targets except the three fork-specific targets: x64 musl,
ARMv7, and Android arm64. Those three are downloaded from `rebroad/rusty_v8`;
x64 musl and Android use the sandbox profile, while ARMv7 uses the ordinary
release profile. Local npm publication uses the same target mapping:

The ARMv7 package intentionally does not contain `codex-code-mode-host`,
because that host depends on V8's 64-bit sandbox feature. Code Mode therefore
fails closed on ARMv7; the other package targets include the host executable.

Release tags and binary/npm candidate versions use the workspace version
followed by the first 10 hexadecimal commit characters and a UTC timestamp in
`YYYYMMDDHHMM` form, for example:

```text
codex-v0.148.0-alpha.5.9084226b62.202608082151
```

```bash
# Create/push the source release tag, wait for CI to appear, and print its run
# and job URLs plus the command to watch it. The script does not watch CI.
# No local cargo compilation or npm packaging is performed by this command.
scripts/build_codex.sh --release --start-github-release

# Download and audit the exact completed GitHub candidate locally instead of
# using the workflow's npm publish job.
TAG="codex-npm-v<candidate-version>"
scripts/download_npm_release.sh "$TAG" ../codex.build/build/npm-artifact-github
python3 scripts/audit_npm_packages.py \
  --artifact-dir ../codex.build/build/npm-artifact-github \
  --expected-version "<candidate-version>"
scripts/publish_npm_local.sh ../codex.build/build/npm-artifact-github
```

`--start-github-release` starts the GitHub build/release workflow. On a
successful tag-triggered run, its `publish-npm` job publishes through the
configured npm Trusted Publisher after the `npm-production` environment gate.
Manual workflow dispatches build and create the GitHub release but do not
publish to npm. `--publish-local-npm` publishes locally and does not start a
new GitHub run; when it reuses a completed fork release, it prints that release
URL.
`--publish` and `--publish-npm` remain compatibility aliases for the two
descriptive options. `npm whoami` (or an equivalent authenticated npm session)
must succeed before the final publication step.

The complete output contains one root archive and eight platform archives.
Publish or install them together when using the unified package:

```bash
npm install --global @reb.ai/codex
```

The generated archives are named `codex-npm-*.tgz`. For local testing,
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
