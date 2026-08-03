<p align="center"><strong>Codex CLI</strong> is a coding agent from OpenAI that runs locally on your computer.
<p align="center">
  <img src="https://github.com/openai/codex/blob/main/.github/codex-cli-splash.png" alt="Codex CLI splash" width="80%" />
</p>

## Install

### Termux (Android ARM64)

```bash
pkg update && pkg upgrade -y
pkg install nodejs-lts -y
npm install -g @rebroad/codex
codex --version
codex login
```

From a clone of this repository, the same setup can be run with one command:

```bash
sh scripts/install-termux.sh
```

The installer installs the published package, including the tracked wrapper
and the release native binary.

Requirements:

- Android 10+ / API 29+ (the release binary is built with the API 29 NDK target)
- ARM64 device
- Node.js >= 18

## Scope

What this fork does:

- tracks upstream OpenAI Codex closely
- builds native Android ARM64 binaries for Termux
- applies only the compatibility patches upstream does not ship
- publishes GitHub release assets and an npm package for Termux users

What this fork does not do:

- maintain a broad feature fork
- replace upstream Codex
- carry fork-only product features unrelated to Termux compatibility

## Current Termux Delta

- browser login uses `termux-open-url`
- self-update points to `rebroad/codex` and `@rebroad/codex`
- packaged wrappers set `CODEX_SELF_EXE` to the native ELF, sanitize `LD_LIBRARY_PATH`, and bundle `libc++_shared.so`
- Android binaries are linked with `RUNPATH=$ORIGIN`
- `exec`/code-mode now runs for real on Android via the in-process V8 runtime (no longer a stub) — the meaningful capability gain on Termux
- realtime voice/audio is no longer part of this build: upstream removed the TUI realtime voice feature (openai/codex#27801), so the fork's Android cpal/oboe enablement toggle (never usable from the Termux CLI anyway, as the backend needs an Android JavaVM/Activity) was dropped with it. Termux-native audio remains tracked on the Codex VL roadmap.
- Android PTY and lock-handling compatibility patches remain enabled where upstream behavior still breaks on Bionic/Termux

## Releases and Updates

- Latest GitHub release: [releases/latest](https://github.com/rebroad/codex/releases/latest)
- Upstream base: OpenAI Codex `rust-v0.146.0`. The `0.146.0` Android package
  follows the exact-artifact release flow; npm dist-tags and GitHub Releases
  are authoritative for current publication status.
- npm package: [`@rebroad/codex`](https://www.npmjs.com/package/@rebroad/codex)

Maintainer publish flow:

- build the exact sanitized candidate with GitHub Actions and audit its
  immutable npm tarball
- land the validated full tree on `develop`
- publish that unchanged tarball to `next`
- promote the tested sanitized commit to clean GitHub `main`
- point `latest` and `next` at the stable version, then publish the annotated
  tag and GitHub release from `main`
- publish a post-release Termux device-validation summary

## Documentation

- [Changelog](./CHANGELOG.md)
- [Patch inventory](./patches/README.md)
- [Building from source](./BUILDING.md)
- [Build and install locally on Termux](./docs/install.md#one-command-build-and-install-from-source)
- The 0.146.0 on-device validation summary is pending the release artifact and
  post-build device testing.
- [Install docs](./docs/install.md)
- [Authentication](./docs/authentication.md)
- [Configuration](./docs/config.md)

## Security

This is a community fork of OpenAI Codex. Security-relevant properties of this build:

- **Network**: agents bind to loopback by default; nothing is exposed externally unless you opt in.
- **Supply chain**: builds and releases come only from fork-owned CI and the `@rebroad/...` npm
  scope. This package does not silently fetch or run the upstream installer; updates flow through
  the fork's own channel.
- **Termux**: TLS trust uses bundled webpki roots (no Android platform-verifier dependency).
  The daemon, history, PATH-alias, installation-id, and policy lock sites that are safe to run
  without advisory exclusion degrade only when the filesystem reports locking as unsupported;
  credential and certificate transactions remain fail-closed.

For sensitive work, prefer the official Codex CLI on Linux/macOS over SSH.

To report a vulnerability, see [SECURITY.md](./SECURITY.md).

## Community guides

- [OpenAI Codex CLI on Android via Termux](https://timharbakon.com/openai-codex-cli-android-termux/)
  — independent third-party walkthrough covering Termux setup, the Bionic-libc incompatibility
  this package solves, and a small Hono test project.

## License

This project remains under the Apache 2.0 license inherited from OpenAI Codex.

- Original work: OpenAI
- Termux port: minimal Android compatibility patches

See [LICENSE](./LICENSE).
