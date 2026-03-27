#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Delete a single Codex conversation thread from the current CODEX_HOME.

Usage:
  scripts/delete_codex_thread.sh <thread_id>

Environment:
  CODEX_HOME   Codex home directory to target (default: ~/.codex)

Behavior:
  1. Finds state*.sqlite files under CODEX_HOME.
  2. Deletes matching row(s) from threads table by thread id.
  3. Removes referenced rollout file(s), if present.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -ne 1 ]]; then
  usage >&2
  exit 1
fi

if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "Missing required command: sqlite3" >&2
  exit 1
fi

THREAD_ID="$1"
CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"

if [[ ! -d "$CODEX_HOME" ]]; then
  echo "CODEX_HOME does not exist: $CODEX_HOME" >&2
  exit 1
fi

mapfile -t STATE_DBS < <(find "$CODEX_HOME" -maxdepth 1 -type f -name 'state*.sqlite' | sort)
if [[ ${#STATE_DBS[@]} -eq 0 ]]; then
  echo "No state*.sqlite files found under CODEX_HOME: $CODEX_HOME" >&2
  exit 1
fi

escape_sql() {
  local s="$1"
  printf '%s' "${s//\'/\'\'}"
}

THREAD_ID_ESCAPED="$(escape_sql "$THREAD_ID")"
DELETED=0

for db in "${STATE_DBS[@]}"; do
  mapfile -t ROLLOUT_PATHS < <(
    sqlite3 "$db" \
      "SELECT rollout_path FROM threads WHERE id = '$THREAD_ID_ESCAPED';"
  )

  if [[ ${#ROLLOUT_PATHS[@]} -eq 0 ]]; then
    continue
  fi

  sqlite3 "$db" "DELETE FROM threads WHERE id = '$THREAD_ID_ESCAPED';"
  DELETED=1

  for rollout_path in "${ROLLOUT_PATHS[@]}"; do
    [[ -z "$rollout_path" ]] && continue

    if [[ "$rollout_path" = /* ]]; then
      rm -f -- "$rollout_path"
      echo "Deleted rollout file: $rollout_path"
    else
      rm -f -- "$CODEX_HOME/$rollout_path"
      echo "Deleted rollout file: $CODEX_HOME/$rollout_path"
    fi
  done

  echo "Deleted thread $THREAD_ID from DB: $db"
done

if [[ "$DELETED" -eq 0 ]]; then
  echo "Thread not found in CODEX_HOME state DBs: $THREAD_ID" >&2
  exit 1
fi
