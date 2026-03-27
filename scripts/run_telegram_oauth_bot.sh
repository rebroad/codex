#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

TOKEN_FILE="${TOKEN_FILE:-$HOME/.codex/telegram.token}"
CODEX_BIN_DEFAULT="$REPO_ROOT/codex-rs/target/debug/codex"
CODEX_BIN="${CODEX_BIN:-$CODEX_BIN_DEFAULT}"
AUTH_ROOT="${AUTH_ROOT:-$HOME/.codex/telegram-auth}"
BUILD_CODEX=1

usage() {
  cat <<'EOF'
Usage: scripts/run_telegram_oauth_bot.sh [options] [-- bot_args...]

Options:
  --codex-bin PATH          Path to codex binary (default: codex-rs/target/debug/codex).
  --auth-root DIR           Directory for per-user auth.json files.
  --token-file FILE         Telegram bot token file (default: ~/.codex/telegram.token).
  --no-build                Skip cargo build step.
  -h, --help                Show this help.
EOF
}

extra_bot_args=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --codex-bin)
      CODEX_BIN="$2"
      shift 2
      ;;
    --auth-root)
      AUTH_ROOT="$2"
      shift 2
      ;;
    --token-file)
      TOKEN_FILE="$2"
      shift 2
      ;;
    --no-build)
      BUILD_CODEX=0
      shift
      ;;
    --)
      shift
      extra_bot_args+=("$@")
      break
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

require_cmd python3

if [[ "$BUILD_CODEX" -eq 1 ]]; then
  require_cmd cargo
  echo "Building codex-cli..."
  cargo build --manifest-path "$REPO_ROOT/codex-rs/Cargo.toml" -p codex-cli
fi

if [[ ! -x "$CODEX_BIN" ]]; then
  echo "Codex binary not found or not executable: $CODEX_BIN" >&2
  exit 1
fi

if [[ ! -f "$TOKEN_FILE" ]]; then
  echo "Telegram token file not found: $TOKEN_FILE" >&2
  exit 1
fi

echo "Per-user auth root: $AUTH_ROOT"

exec "$REPO_ROOT/scripts/telegram_oauth_bot.py" \
  --token-file "$TOKEN_FILE" \
  --codex-bin "$CODEX_BIN" \
  --auth-root "$AUTH_ROOT" \
  "${extra_bot_args[@]}"
