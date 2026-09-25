#!/usr/bin/env bash
set -euo pipefail

# Cargo holds file descriptor 9 for the shared target lock. Close it before
# starting sccache so a daemonized sccache server cannot retain that lock.
exec 9>&- || true
# Cargo emits `-C extra-filename=...` for dependency artifacts, but also emits
# `-o` for final binaries. Rustc ignores the former in that combination and
# warns; remove only that redundant pair so warning-deny builds stay quiet.
has_output=false
for arg in "$@"; do
  if [[ "${arg}" == "-o" ]]; then
    has_output=true
    break
  fi
done

if [[ "${has_output}" == true ]]; then
  original_args=("$@")
  filtered_args=()
  index=0
  while ((index < ${#original_args[@]})); do
    arg="${original_args[index]}"
    next_arg="${original_args[index+1]-}"
    if [[ "${arg}" == "-C" && "${next_arg}" == extra-filename=* ]]; then
      index=$((index + 2))
      continue
    fi
    if [[ "${arg}" == -Cextra-filename=* ]]; then
      index=$((index + 1))
      continue
    fi
    filtered_args+=("${arg}")
    index=$((index + 1))
  done
  set -- "${filtered_args[@]}"
fi

exec "${CODEX_SCCACHE_BIN:?CODEX_SCCACHE_BIN must be set}" "$@"
