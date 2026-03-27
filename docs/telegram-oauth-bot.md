# Telegram Login Bot (Device Auth)

This document describes the Telegram login integration using device authorization.

## Overview

The bot keeps Codex non-blocking:

1. `codex tlogin start --user-id <id>` requests a device code and stores pending login state.
2. Bot sends the verification URL and one-time user code to Telegram.
3. User completes approval in browser at the verification URL.
4. User sends any message to the bot.
5. `codex tlogin complete --user-id <id>` finalizes auth and writes credentials to `--auth-file`.

No localhost callback, public callback URL, or cloudflared tunnel is required.

## Components

- `codex tlogin ...`:
  - `start` and `complete` CLI subcommands in `codex-cli`
  - requires `--auth-file`
- `scripts/telegram_oauth_bot.py`:
  - Telegram polling worker
  - auto-starts login when auth is missing and cooldown allows
  - caches pending logins in-memory keyed by chat/user ID
- `scripts/run_telegram_oauth_bot.sh`:
  - one-command launcher
  - builds `codex-cli` (unless `--no-build`)
  - deletes webhook so polling mode works
  - runs the bot worker

## `--auth-root` Meaning

`--auth-root` is the directory where per-user auth files are stored.

Each Telegram user gets:

- `<auth-root>/<telegram_username>.auth.json` (sanitized for filesystem safety)
- fallback when username is unavailable: `<auth-root>/id-<telegram_numeric_id>.auth.json`

That file path is passed to `codex --auth-file` during `tlogin start/complete`.

The bot also keeps a per-user `CODEX_HOME` at:

- `<auth-root>/homes/<telegram_username>/`

This keeps Codex sessions isolated per Telegram user.

For each Telegram user invocation, if a `config.toml` is not present in that
user's `CODEX_HOME`, the bot searches parent directories for `config.toml`,
stopping at `~/.codex`, and uses the nearest file in place (no copy).

If `[prompt_debug_http].capture_dir` contains `$user`, the bot expands it at
runtime to `tg-<telegram_username>` (sanitized) via env override. Example:

- `capture_dir = "/tmp/codex-prompt-debug.$user"`
- becomes `capture_dir = "/tmp/codex-prompt-debug.tg-wibble"` for user `wibble`.

## Run It

```bash
cd /home/rebroad/src/codex
scripts/run_telegram_oauth_bot.sh
```

Defaults:

- token file: `~/.codex/telegram.token`
- auth root: `~/.codex/telegram-auth`
- codex binary: `codex-rs/target/debug/codex`

Token file format:

- one Telegram bot token per line
- blank lines and `#` comments are ignored
- the bot worker runs one polling loop per token

## Useful Options

```bash
scripts/run_telegram_oauth_bot.sh \
  --no-build \
  --codex-bin /home/rebroad/src/codex/codex-rs/target/debug/codex \
  --auth-root ~/.codex/telegram-auth \
  --token-file ~/.codex/telegram.token

# Optional Telegram-specific system prompt:
scripts/run_telegram_oauth_bot.sh -- \
  --system-prompt-file ~/.codex/telegram-system-prompt.txt
```

## User Flow in Telegram

1. User sends `/start` (or any message).
2. If auth is already valid, bot replies accordingly.
3. If auth is missing and no recent attempt exists, bot sends verification URL + user code.
4. User approves in browser.
5. User sends any message again; bot attempts `tlogin complete`.
6. On success, bot confirms login completion.
7. After login, user messages are sent to `codex exec --bare-prompt` and replies are sent back.

## Cooldown Behavior

- Bot starts a new login automatically when:
  - auth is invalid, and
  - no pending login is active, and
  - no login was started in the previous hour.
- If a login was started recently, bot asks user to complete that one or wait for cooldown.

## Troubleshooting

- Bot does not respond:
  - confirm token file exists and webhook deletion succeeded.
- Bot says login is not complete:
  - finish browser approval first, then send another Telegram message.
- Credentials not found:
  - check files under `--auth-root` and confirm `codex --auth-file <file> login status`.
