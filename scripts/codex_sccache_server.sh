codex_sccache_server_pid=""

codex_sccache_server_set_oom_score() {
  [[ -n "${CODEX_RUSTC_OOM_SCORE_ADJ:-}" ]] || return 0

  local server_pids=""
  if command -v pgrep >/dev/null 2>&1; then
    server_pids="$(
      {
        pgrep -x sccache 2>/dev/null || true
        # Android may expose a truncated executable path in /proc/<pid>/comm
        # instead of the process name "sccache".
        pgrep -f '[s]ccache$' 2>/dev/null || true
      } | sort -u
    )"
  elif command -v lsof >/dev/null 2>&1; then
    server_pids="$(lsof -nP -t -iTCP:"${SCCACHE_SERVER_PORT:-4226}" -sTCP:LISTEN 2>/dev/null | sort -u)"
  else
    printf '%s\n' 'cannot identify an existing sccache server to set its OOM score (need pgrep or lsof)' >&2
    return 1
  fi

  if [[ -z "${server_pids}" ]]; then
    printf '%s\n' 'unable to identify the running sccache server process' >&2
    return 1
  fi

  local server_adjusted=false
  local server_pid server_args
  while IFS= read -r server_pid; do
    [[ -n "${server_pid}" && -r "/proc/${server_pid}/cmdline" ]] || continue
    server_args="$(ps -ww -p "${server_pid}" -o args= 2>/dev/null || true)"
    [[ "${server_args}" == "${CODEX_SCCACHE_BIN:-sccache}" ]] || continue
    if ! printf '%s\n' "${CODEX_RUSTC_OOM_SCORE_ADJ}" \
      >"/proc/${server_pid}/oom_score_adj" 2>/dev/null; then
      printf 'unable to set sccache server %s oom_score_adj to %s\n' \
        "${server_pid}" "${CODEX_RUSTC_OOM_SCORE_ADJ}" >&2
      return 1
    fi
    server_adjusted=true
  done <<<"${server_pids}"

  if [[ "${server_adjusted}" != true ]]; then
    printf '%s\n' 'could not identify the running sccache process' >&2
    return 1
  fi
}

codex_sccache_server_start() {
  [[ -n "${CODEX_SCCACHE_BIN:-}" ]] || return 0
  if (exec 8<>/dev/tcp/127.0.0.1/${SCCACHE_SERVER_PORT:-4226}) 2>/dev/null; then
    codex_sccache_server_set_oom_score || return $?
    return 0
  fi

  (
    if [[ -n "${CODEX_RUSTC_OOM_SCORE_ADJ:-}" ]]; then
      printf '%s\n' "${CODEX_RUSTC_OOM_SCORE_ADJ}" \
        >"/proc/${BASHPID}/oom_score_adj" 2>/dev/null || {
          printf 'unable to set sccache server oom_score_adj to %s\n' \
            "${CODEX_RUSTC_OOM_SCORE_ADJ}" >&2
          exit 1
        }
    fi
    exec 9>&-
    SCCACHE_START_SERVER=1 SCCACHE_NO_DAEMON=1 SCCACHE_IDLE_TIMEOUT=0 \
      exec "$CODEX_SCCACHE_BIN"
  ) >/dev/null 2>&1 &
  codex_sccache_server_pid=$!
  for attempt in {1..50}; do
    if (exec 8<>/dev/tcp/127.0.0.1/${SCCACHE_SERVER_PORT:-4226}) 2>/dev/null; then
      if ! codex_sccache_server_set_oom_score; then
        codex_sccache_server_stop
        return 1
      fi
      return 0
    fi
    if ! kill -0 "$codex_sccache_server_pid" 2>/dev/null; then
      printf '%s\n' 'sccache server failed to start' >&2
      return 1
    fi
    sleep 0.1
  done

  printf '%s\n' 'sccache server did not become ready' >&2
  return 1
}

codex_sccache_server_stop() {
  [[ -n "$codex_sccache_server_pid" ]] || return 0
  kill "$codex_sccache_server_pid" 2>/dev/null || true
  wait "$codex_sccache_server_pid" 2>/dev/null || true
}
