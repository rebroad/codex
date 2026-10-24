#!/usr/bin/env bash
set -euo pipefail

# On Android, mark this compiler invocation as an early memory-pressure kill
# candidate. The score survives exec and is inherited by rustc's children.
if [[ -n "${CODEX_RUSTC_OOM_SCORE_ADJ:-}" ]]; then
  oom_score_file="/proc/$$/oom_score_adj"
  if ! printf '%s\n' "${CODEX_RUSTC_OOM_SCORE_ADJ}" >"${oom_score_file}" 2>/dev/null; then
    printf 'unable to set rustc oom_score_adj to %s\n' \
      "${CODEX_RUSTC_OOM_SCORE_ADJ}" >&2
    exit 1
  fi
fi

if [[ -n "${CODEX_SCCACHE_BIN:-}" ]]; then
  exec "${CODEX_SCCACHE_WRAPPER:?CODEX_SCCACHE_WRAPPER must be set}" "$@"
fi

exec "$@"
