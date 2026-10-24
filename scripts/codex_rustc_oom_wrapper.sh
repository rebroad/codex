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

codex_compiler_command=("$@")
if [[ -n "${CODEX_SCCACHE_BIN:-}" ]]; then
  codex_compiler_command=("${CODEX_SCCACHE_WRAPPER:?CODEX_SCCACHE_WRAPPER must be set}" "$@")
fi

if [[ "${CODEX_ANDROID_RUSTC_MEMORY_GUARD:-0}" != 1 ]]; then
  exec "${codex_compiler_command[@]}"
fi

# Keep the compiler and any linker it starts in a process group that can be
# stopped together when Android's observed memory-pressure guard is reached.
if [[ ! -x /system/bin/setsid ]]; then
  echo 'Android Rust memory guard requires /system/bin/setsid' >&2
  exit 1
fi

codex_uint() {
  [[ "$1" =~ ^[0-9]+$ ]]
}

codex_swap_guard_percent="${CODEX_ANDROID_RUSTC_SWAP_FREE_GUARD_PERCENT:-50}"
codex_memavailable_guard_percent="${CODEX_ANDROID_RUSTC_MEMAVAILABLE_GUARD_PERCENT:-20}"
codex_psi_some_guard_ms="${CODEX_ANDROID_RUSTC_PSI_SOME_GUARD_MS:-35}"
codex_psi_full_guard_ms="${CODEX_ANDROID_RUSTC_PSI_FULL_GUARD_MS:-60}"
for codex_threshold in \
  "$codex_swap_guard_percent" \
  "$codex_memavailable_guard_percent" \
  "$codex_psi_some_guard_ms" \
  "$codex_psi_full_guard_ms"; do
  if ! codex_uint "$codex_threshold"; then
    echo "invalid Android Rust memory guard threshold: $codex_threshold" >&2
    exit 2
  fi
done

codex_read_memory() {
  codex_mem_total_kb=0
  codex_mem_available_kb=0
  codex_swap_total_kb=0
  codex_swap_free_kb=0
  while read -r codex_key codex_value _; do
    case "$codex_key" in
      MemTotal:) codex_mem_total_kb="$codex_value" ;;
      MemAvailable:) codex_mem_available_kb="$codex_value" ;;
      SwapTotal:) codex_swap_total_kb="$codex_value" ;;
      SwapFree:) codex_swap_free_kb="$codex_value" ;;
    esac
  done </proc/meminfo
  codex_uint "$codex_mem_total_kb" \
    && codex_uint "$codex_mem_available_kb" \
    && codex_uint "$codex_swap_total_kb" \
    && codex_uint "$codex_swap_free_kb" \
    && ((codex_mem_total_kb > 0))
}

codex_read_psi() {
  codex_psi_some_total_us=0
  codex_psi_full_total_us=0
  [[ -r /proc/pressure/memory ]] || return 1
  while read -r codex_kind codex_fields; do
    for codex_field in $codex_fields; do
      case "$codex_field" in
        total=*)
          case "$codex_kind" in
            some) codex_psi_some_total_us="${codex_field#total=}" ;;
            full) codex_psi_full_total_us="${codex_field#total=}" ;;
          esac
          ;;
      esac
    done
  done </proc/pressure/memory
  codex_uint "$codex_psi_some_total_us" && codex_uint "$codex_psi_full_total_us"
}

if ! codex_read_memory; then
  echo 'Android Rust memory guard cannot read required /proc/meminfo fields' >&2
  exit 1
fi
codex_psi_available=false
if codex_read_psi; then
  codex_psi_available=true
  codex_previous_some_total_us="$codex_psi_some_total_us"
  codex_previous_full_total_us="$codex_psi_full_total_us"
  codex_some_deltas=()
  codex_full_deltas=()
else
  echo 'Android Rust memory guard: PSI unavailable; using RAM and swap guards only' >&2
fi

codex_memory_guard_reason() {
  if ((codex_mem_available_kb * 100 <= codex_mem_total_kb * codex_memavailable_guard_percent)); then
    codex_guard_reason="MemAvailable ${codex_mem_available_kb}kB <= ${codex_memavailable_guard_percent}% of RAM"
    return 0
  fi
  if ((codex_swap_total_kb > 0 && codex_swap_free_kb * 100 <= codex_swap_total_kb * codex_swap_guard_percent)); then
    codex_guard_reason="free swap ${codex_swap_free_kb}kB <= ${codex_swap_guard_percent}% of total swap"
    return 0
  fi
  if [[ "${1:-}" == memory-only ]]; then
    return 1
  fi
  if [[ "$codex_psi_available" == true ]]; then
    codex_some_delta_us=$((codex_psi_some_total_us - codex_previous_some_total_us))
    codex_full_delta_us=$((codex_psi_full_total_us - codex_previous_full_total_us))
    codex_previous_some_total_us="$codex_psi_some_total_us"
    codex_previous_full_total_us="$codex_psi_full_total_us"
    ((codex_some_delta_us >= 0 && codex_full_delta_us >= 0)) || {
      codex_some_deltas=()
      codex_full_deltas=()
      codex_some_delta_us=0
      codex_full_delta_us=0
    }
    codex_some_deltas+=("$codex_some_delta_us")
    codex_full_deltas+=("$codex_full_delta_us")
    if ((${#codex_some_deltas[@]} > 5)); then
      codex_some_deltas=("${codex_some_deltas[@]:1}")
      codex_full_deltas=("${codex_full_deltas[@]:1}")
    fi
    if ((${#codex_some_deltas[@]} == 5)); then
      codex_recent_some_us=0
      codex_recent_full_us=0
      for codex_delta in "${codex_some_deltas[@]}"; do
        codex_recent_some_us=$((codex_recent_some_us + codex_delta))
      done
      for codex_delta in "${codex_full_deltas[@]}"; do
        codex_recent_full_us=$((codex_recent_full_us + codex_delta))
      done
      if ((codex_recent_some_us >= codex_psi_some_guard_ms * 1000)); then
        codex_guard_reason="memory PSI some stall ${codex_recent_some_us}us per second >= ${codex_psi_some_guard_ms}ms"
        return 0
      fi
      if ((codex_recent_full_us >= codex_psi_full_guard_ms * 1000)); then
        codex_guard_reason="memory PSI full stall ${codex_recent_full_us}us per second >= ${codex_psi_full_guard_ms}ms"
        return 0
      fi
    fi
  fi
  return 1
}

if codex_memory_guard_reason memory-only; then
  echo "not starting Rust compile because Android memory guard is active: $codex_guard_reason" >&2
  exit 75
fi

/system/bin/setsid "${codex_compiler_command[@]}" &
codex_compiler_pid=$!
codex_stop_compiler() {
  kill -TERM -- "-$codex_compiler_pid" 2>/dev/null || true
  for codex_wait_attempt in {1..8}; do
    kill -0 -- "-$codex_compiler_pid" 2>/dev/null || break
    sleep 0.25
  done
  kill -KILL -- "-$codex_compiler_pid" 2>/dev/null || true
}
trap 'codex_stop_compiler; wait "$codex_compiler_pid" 2>/dev/null || true; exit 130' INT
trap 'codex_stop_compiler; wait "$codex_compiler_pid" 2>/dev/null || true; exit 143' TERM
trap 'codex_stop_compiler; wait "$codex_compiler_pid" 2>/dev/null || true; exit 129' HUP

while kill -0 "$codex_compiler_pid" 2>/dev/null; do
  if codex_memory_guard_reason; then
    echo "stopping Rust compile before Android low-memory kill: $codex_guard_reason" >&2
    codex_stop_compiler
    wait "$codex_compiler_pid" 2>/dev/null || true
    exit 75
  fi
  codex_read_memory || {
    echo 'Android Rust memory guard lost access to /proc/meminfo; stopping the compiler' >&2
    codex_stop_compiler
    wait "$codex_compiler_pid" 2>/dev/null || true
    exit 1
  }
  if [[ "$codex_psi_available" == true ]]; then
    if codex_read_psi; then
      :
    else
      echo 'Android Rust memory guard lost access to PSI; continuing with RAM and swap guards' >&2
      codex_psi_available=false
    fi
  fi
  sleep 0.25
done

if wait "$codex_compiler_pid"; then
  exit 0
else
  exit "$?"
fi
