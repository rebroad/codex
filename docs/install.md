## Installing & building

### System requirements

### Requirements

| Requirement | Details |
| --- | --- |
| Android | Android 10+ / API 29+ (release target) |
| CPU | ARM64 |
| Shell | Termux |
| Node.js | 18+ |

### One-command install from a clone

The repository includes an installer for Termux. After cloning this repository,
run:

```bash
sh scripts/install-termux.sh
```

The script installs Node.js if necessary, then installs the published Android
ARM64 package globally with npm and verifies `codex --version`. To select an
exact published version:

```bash
CODEX_TERMUX_PACKAGE='@mmmbuto/codex-cli-termux@0.146.0' sh scripts/install-termux.sh
```

### One-command build and install from source

To compile the checkout on the Android device and make the npm wrapper use
that newly compiled binary, run:

```bash
sh scripts/build-and-install-termux.sh
```

This installs missing Termux build tools, fetches the pinned and
checksum-verified Android V8 artifacts, builds the CLI, stages the native
binary plus `libc++_shared.so`, installs the local npm package globally, and
verifies the installed `codex` wrapper. It uses a faster debug profile with
debug information disabled. For a release build:

```bash
CODEX_BUILD_PROFILE=release sh scripts/build-and-install-termux.sh
```

### Install directly from npm

```bash
pkg update && pkg upgrade -y
pkg install nodejs-lts -y
npm install -g @mmmbuto/codex-cli-termux@latest
codex --version
codex login
```

The npm package includes one native Android ARM64 `codex` binary, `codex` and
`codex-exec` launcher scripts, and the bundled `libc++_shared.so` runtime
library. The npm `codex` command points to `bin/codex.js`; that wrapper sets up
the native runtime and launches `bin/codex.bin`. The package also includes the
shell launcher at `bin/codex` for direct native invocation. The `codex-exec`
launcher dispatches the native binary's `exec` subcommand instead of
duplicating the V8-linked ELF.

### What is in the repository?

The wrapper and package metadata are tracked here:

```text
npm-package/bin/codex.js       npm entry-point wrapper
npm-package/bin/codex          direct shell launcher
npm-package/bin/codex-exec     direct exec launcher
npm-package/package.json       npm command mapping
```

The large native files `npm-package/bin/codex.bin` and
`npm-package/bin/libc++_shared.so` are generated release artifacts and are
ignored in Git. The Android build workflow copies them into the package before
creating the npm tarball. Therefore `npm install` is the normal end-user path;
`npm install -g ./npm-package` is only valid after a native binary has been
built and copied there as described in [BUILDING.md](../BUILDING.md).

### Install from a published GitHub release

Download the `mmmbuto-codex-cli-termux-<version>.tgz` asset from the matching
GitHub release, then install it with npm:

```bash
npm install -g ./mmmbuto-codex-cli-termux-<published-version>.tgz
codex --version
```

Each release also publishes a `.sha256` checksum file for the npm tarball.

### Build from source

For the same device build command plus maintainer cross-build notes, see
[BUILDING.md](../BUILDING.md).

## Logging

Codex honors the `RUST_LOG` environment variable. The TUI writes logs under the
Codex log directory by default, and `codex exec` prints error-level messages
inline for non-interactive runs.
