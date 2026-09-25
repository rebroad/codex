#!/usr/bin/env bash
set -euo pipefail

: "${CODEX_CARGO_LINK_LOCK_FILE:?CODEX_CARGO_LINK_LOCK_FILE is required}"
: "${CODEX_CARGO_REAL_LINKER:?CODEX_CARGO_REAL_LINKER is required}"

if ! command -v flock >/dev/null 2>&1; then
  echo "flock is required to serialize Codex Cargo linker invocations" >&2
  exit 1
fi

linker_tmpdir="${CODEX_CARGO_LINKER_TMPDIR:-/var/tmp}"
exec flock --exclusive "${CODEX_CARGO_LINK_LOCK_FILE}" \
  env TMPDIR="${linker_tmpdir}" "${CODEX_CARGO_REAL_LINKER}" "$@"
