codex_sccache_server_pid=""

codex_sccache_server_start() {
  [[ -n "${CODEX_SCCACHE_BIN:-}" ]] || return 0
  if (exec 8<>/dev/tcp/127.0.0.1/${SCCACHE_SERVER_PORT:-4226}) 2>/dev/null; then
    return 0
  fi

  (SCCACHE_START_SERVER=1 SCCACHE_NO_DAEMON=1 SCCACHE_IDLE_TIMEOUT=0 "$CODEX_SCCACHE_BIN") >/dev/null 2>&1 &
  codex_sccache_server_pid=$!
  for attempt in {1..50}; do
    if (exec 8<>/dev/tcp/127.0.0.1/${SCCACHE_SERVER_PORT:-4226}) 2>/dev/null; then
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
