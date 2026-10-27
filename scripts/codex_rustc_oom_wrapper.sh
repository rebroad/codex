#!/usr/bin/env bash
set -euo pipefail

codex_psi_notice_probe=false
if [[ "${1:-}" == --codex-psi-notice-probe ]]; then
  codex_psi_notice_probe=true
  shift
fi

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
  codex_sccache_wrapper="${CODEX_SCCACHE_WRAPPER:?CODEX_SCCACHE_WRAPPER must be set}"
  if [[ -x /system/bin/setsid ]]; then
    # Android's system `setsid` bypasses Termux's exec shim, so scripts with an
    # /usr/bin/env shebang cannot be launched directly from this process group.
    codex_compiler_command=("${PREFIX:-/data/data/com.termux/files/usr}/bin/bash" "$codex_sccache_wrapper" "$@")
  else
    codex_compiler_command=("$codex_sccache_wrapper" "$@")
  fi
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
codex_psi_some_resume_guard_us_per_second=$((codex_psi_some_guard_ms * 750))
codex_psi_resume_clear_windows_required=3

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

codex_adb_bin="$(command -v adb || true)"
codex_adb_serial="${ANDROID_SERIAL:-}"
codex_adb_server_socket="${ADB_SERVER_SOCKET:-tcp:127.0.0.1:5037}"
if [[ -n "$codex_adb_bin" && -z "$codex_adb_serial" ]]; then
  codex_adb_devices="$(ADB_SERVER_SOCKET="$codex_adb_server_socket" \
    "$codex_adb_bin" devices 2>/dev/null || true)"
  codex_adb_serials=()
  codex_adb_5555_serial=""
  while read -r codex_adb_device codex_adb_state _; do
    if [[ "$codex_adb_state" == device ]]; then
      codex_adb_serials+=("$codex_adb_device")
      if [[ "$codex_adb_device" == *:5555 ]]; then
        codex_adb_5555_serial="$codex_adb_device"
      fi
    fi
  done <<<"$codex_adb_devices"
  if [[ -n "$codex_adb_5555_serial" ]]; then
    codex_adb_serial="$codex_adb_5555_serial"
  elif ((${#codex_adb_serials[@]} == 1)); then
    codex_adb_serial="${codex_adb_serials[0]}"
  fi
fi

codex_psi_stream_fd=""
codex_psi_stream_pid=""
codex_start_psi_stream() {
  [[ -n "$codex_adb_bin" && -n "$codex_adb_serial" ]] || return 1
  coproc CODEX_ADB_PSI_STREAM {
    # The target lock is held by the build shell and inherited by rustc.
    # This long-running monitor must not retain it, and exec makes the coproc
    # PID the adb process so codex_stop_psi_stream can terminate and reap it.
    if [[ -n "${CODEX_CARGO_HELD_TARGET_LOCK_FILE:-}" ]]; then
      exec 9>&-
    fi
    ADB_SERVER_SOCKET="$codex_adb_server_socket" \
      exec "$codex_adb_bin" -s "$codex_adb_serial" exec-out sh -c '
      while :; do
        some=0
        full=0
        while read -r kind fields; do
          for field in $fields; do
            case "$field" in
              total=*)
                case "$kind" in
                  some) some=${field#total=} ;;
                  full) full=${field#total=} ;;
                esac
                ;;
            esac
          done
        done </proc/pressure/memory
        printf "%s %s\n" "$some" "$full"
        sleep 0.25
      done
    ' 2>/dev/null
  }
  codex_psi_stream_fd="${CODEX_ADB_PSI_STREAM[0]}"
  codex_psi_stream_pid="$CODEX_ADB_PSI_STREAM_PID"
}

codex_stop_psi_stream() {
  if [[ -n "$codex_psi_stream_fd" ]]; then
    eval "exec ${codex_psi_stream_fd}<&-"
    codex_psi_stream_fd=""
  fi
  if [[ -n "$codex_psi_stream_pid" ]]; then
    kill "$codex_psi_stream_pid" 2>/dev/null || true
    wait "$codex_psi_stream_pid" 2>/dev/null || true
    codex_psi_stream_pid=""
  fi
}

codex_start_psi_stream || true

codex_read_psi() {
  codex_psi_some_total_us=0
  codex_psi_full_total_us=0
  [[ -n "$codex_psi_stream_fd" ]] || return 1
  IFS=' ' read -r -t 1 codex_psi_some_total_us codex_psi_full_total_us \
    <&"$codex_psi_stream_fd" || return 1
  codex_uint "$codex_psi_some_total_us" && codex_uint "$codex_psi_full_total_us"
}

if ! codex_read_memory; then
  echo 'Android Rust memory guard cannot read required /proc/meminfo fields' >&2
  exit 1
fi
codex_psi_available=false
codex_recent_some_us_per_second=0
codex_recent_full_us_per_second=0
if codex_read_psi; then
  codex_psi_available=true
  codex_previous_some_total_us="$codex_psi_some_total_us"
  codex_previous_full_total_us="$codex_psi_full_total_us"
  codex_some_deltas=()
  codex_full_deltas=()
fi
if [[ "$codex_psi_notice_probe" == true ]]; then
  codex_stop_psi_stream
  if [[ "$codex_psi_available" == true ]]; then
    exit 0
  fi
  exit 10
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
      codex_recent_some_us_per_second=0
      codex_recent_full_us_per_second=0
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
      # Five 250ms samples cover 1.25s; normalize their sum to a per-second rate.
      codex_recent_some_us_per_second=$((codex_recent_some_us * 4 / 5))
      codex_recent_full_us_per_second=$((codex_recent_full_us * 4 / 5))
      if ((codex_recent_full_us_per_second >= codex_psi_full_guard_ms * 1000)); then
        codex_guard_reason="memory PSI full stall ${codex_recent_full_us_per_second}us per second >= ${codex_psi_full_guard_ms}ms"
        return 0
      fi
      if ((codex_recent_some_us_per_second >= codex_psi_some_guard_ms * 1000)); then
        codex_guard_reason="memory PSI some stall ${codex_recent_some_us_per_second}us per second >= ${codex_psi_some_guard_ms}ms"
        return 0
      fi
    fi
  fi
  return 1
}

codex_compiler_pid=""
codex_compiler_suspended_at=""
codex_compiler_suspended_at_epoch_us=""
codex_guard_report_window_started_us="${EPOCHREALTIME/./}"
codex_guard_pending_transition_count=0
codex_guard_pending_suspend_count=0
codex_guard_pending_resume_count=0
codex_guard_pending_stop_count=0
codex_guard_pending_completed_pause_us=0
codex_guard_pending_first_at=""
codex_guard_pending_last_at=""
codex_guard_pending_last_reason=""
codex_guard_pending_last_pause_duration=""
codex_guard_pending_last_pause_started_at=""
codex_guard_compiler_state=running
codex_resume_clear_windows=0
declare -A codex_guard_log_events=()
codex_log_guard_event() {
  local event="$1" message="$2"
  if [[ "${CODEX_ANDROID_RUSTC_MEMORY_GUARD_VERBOSE:-0}" == 1 \
    || -z "${codex_guard_log_events[$event]:-}" ]]; then
    echo "$message" >&2
    codex_guard_log_events["$event"]=1
  fi
}

codex_report_guard_transitions() {
  local codex_now_epoch_us codex_pause_duration_us codex_pause_duration
  local codex_current_pause_us codex_current_pause_duration
  codex_now_epoch_us="${EPOCHREALTIME/./}"
  ((codex_now_epoch_us - codex_guard_report_window_started_us >= 1000000)) || return 0
  if ((codex_guard_pending_transition_count == 1)); then
    if ((codex_guard_pending_suspend_count == 1)); then
      codex_pause_duration_us=$((codex_now_epoch_us - codex_compiler_suspended_at_epoch_us))
      printf -v codex_pause_duration '%d.%06ds' \
        "$((codex_pause_duration_us / 1000000))" "$((codex_pause_duration_us % 1000000))"
      echo "suspending Rust compile during Android memory pressure at $codex_guard_pending_last_at: $codex_guard_pending_last_reason (paused $codex_pause_duration so far)" >&2
    else
      echo "resuming Rust compile after Android memory pressure eased at $codex_guard_pending_last_at (paused $codex_guard_pending_last_pause_duration since $codex_guard_pending_last_pause_started_at)" >&2
    fi
  elif ((codex_guard_pending_transition_count > 1)); then
    codex_current_pause_duration=""
    if [[ "$codex_compiler_suspended" == true ]]; then
      codex_current_pause_us=$((codex_now_epoch_us - codex_compiler_suspended_at_epoch_us))
      printf -v codex_current_pause_duration '%d.%06ds' \
        "$((codex_current_pause_us / 1000000))" "$((codex_current_pause_us % 1000000))"
    fi
    printf -v codex_pause_duration '%d.%06ds' \
      "$((codex_guard_pending_completed_pause_us / 1000000))" \
      "$((codex_guard_pending_completed_pause_us % 1000000))"
    echo "Android Rust compile state flapped ${codex_guard_pending_transition_count} times in the last second (${codex_guard_pending_suspend_count} pauses, ${codex_guard_pending_resume_count} resumes, ${codex_guard_pending_stop_count} stops; completed pauses totaled $codex_pause_duration); now $codex_guard_compiler_state${codex_current_pause_duration:+ for $codex_current_pause_duration} (from $codex_guard_pending_first_at to $codex_guard_pending_last_at)" >&2
  fi
  codex_guard_report_window_started_us="$codex_now_epoch_us"
  codex_guard_pending_transition_count=0
  codex_guard_pending_suspend_count=0
  codex_guard_pending_resume_count=0
  codex_guard_pending_stop_count=0
  codex_guard_pending_completed_pause_us=0
  codex_guard_pending_first_at=""
  codex_guard_pending_last_at=""
  codex_guard_pending_last_reason=""
  codex_guard_pending_last_pause_duration=""
  codex_guard_pending_last_pause_started_at=""
}

codex_stop_compiler() {
  [[ -n "$codex_compiler_pid" ]] || return 0
  kill -CONT -- "-$codex_compiler_pid" 2>/dev/null || true
  kill -TERM -- "-$codex_compiler_pid" 2>/dev/null || true
  for codex_wait_attempt in {1..8}; do
    kill -0 -- "-$codex_compiler_pid" 2>/dev/null || break
    sleep 0.25
  done
  kill -KILL -- "-$codex_compiler_pid" 2>/dev/null || true
}
trap 'codex_stop_compiler; codex_stop_psi_stream; [[ -z "$codex_compiler_pid" ]] || wait "$codex_compiler_pid" 2>/dev/null || true; exit 130' INT
trap 'codex_stop_compiler; codex_stop_psi_stream; [[ -z "$codex_compiler_pid" ]] || wait "$codex_compiler_pid" 2>/dev/null || true; exit 143' TERM
trap 'codex_stop_compiler; codex_stop_psi_stream; [[ -z "$codex_compiler_pid" ]] || wait "$codex_compiler_pid" 2>/dev/null || true; exit 129' HUP

codex_wait_for_memory() {
  while true; do
    codex_read_memory || {
      echo 'Android Rust memory guard cannot read required /proc/meminfo fields' >&2
      return 1
    }
    if [[ "$codex_psi_available" == true ]]; then
      if ! codex_read_psi; then
        if [[ "${CODEX_ANDROID_RUSTC_MEMORY_GUARD_VERBOSE:-0}" == 1 ]]; then
          echo 'Android Rust memory guard lost its ADB PSI stream; continuing with RAM and swap guards' >&2
        fi
        codex_psi_available=false
        codex_stop_psi_stream
      fi
    fi
    if ! codex_memory_guard_reason; then
      return 0
    fi
    codex_log_guard_event waiting \
      "waiting for Android memory pressure to ease before (re)starting Rust compile: $codex_guard_reason"
    sleep 1
  done
}

while true; do
  if ! codex_wait_for_memory; then
    codex_stop_psi_stream
    exit 1
  fi
  /system/bin/setsid "${codex_compiler_command[@]}" &
  codex_compiler_pid=$!
  codex_restart_compiler=false
  codex_compiler_suspended=false
  codex_guard_compiler_state=running
  codex_resume_clear_windows=0
  while kill -0 "$codex_compiler_pid" 2>/dev/null; do
    codex_read_memory || {
      echo 'Android Rust memory guard lost access to /proc/meminfo; stopping the compiler' >&2
      codex_stop_compiler
      codex_stop_psi_stream
      wait "$codex_compiler_pid" 2>/dev/null || true
      exit 1
    }
    if [[ "$codex_psi_available" == true ]]; then
      if ! codex_read_psi; then
        if [[ "${CODEX_ANDROID_RUSTC_MEMORY_GUARD_VERBOSE:-0}" == 1 ]]; then
          echo 'Android Rust memory guard lost its ADB PSI stream; continuing with RAM and swap guards' >&2
        fi
        codex_psi_available=false
        codex_stop_psi_stream
      fi
    fi
    if codex_memory_guard_reason; then
      if [[ "$codex_guard_reason" == memory\ PSI\ some\ stall* ]]; then
        codex_resume_clear_windows=0
        if [[ "$codex_compiler_suspended" == false ]]; then
          codex_compiler_suspended_at="$(date '+%Y-%m-%dT%H:%M:%S%z')"
          codex_compiler_suspended_at_epoch_us="${EPOCHREALTIME/./}"
          ((codex_guard_pending_transition_count += 1))
          ((codex_guard_pending_suspend_count += 1))
          [[ -n "$codex_guard_pending_first_at" ]] || codex_guard_pending_first_at="$codex_compiler_suspended_at"
          codex_guard_pending_last_at="$codex_compiler_suspended_at"
          codex_guard_pending_last_reason="$codex_guard_reason"
          codex_guard_compiler_state=paused
          kill -STOP -- "-$codex_compiler_pid" 2>/dev/null || true
          codex_compiler_suspended=true
        fi
      else
        codex_log_guard_event stopping \
          "stopping Rust compile before Android low-memory kill: $codex_guard_reason; waiting to retry"
        if [[ "$codex_compiler_suspended" == true ]]; then
          codex_stopped_at="$(date '+%Y-%m-%dT%H:%M:%S%z')"
          codex_stopped_at_epoch_us="${EPOCHREALTIME/./}"
          codex_pause_duration_us=$((codex_stopped_at_epoch_us - codex_compiler_suspended_at_epoch_us))
          ((codex_guard_pending_transition_count += 1))
          ((codex_guard_pending_stop_count += 1))
          ((codex_guard_pending_completed_pause_us += codex_pause_duration_us))
          [[ -n "$codex_guard_pending_first_at" ]] || codex_guard_pending_first_at="$codex_stopped_at"
          codex_guard_pending_last_at="$codex_stopped_at"
          codex_guard_compiler_state=retrying
          codex_compiler_suspended=false
        fi
        codex_stop_compiler
        wait "$codex_compiler_pid" 2>/dev/null || true
        codex_compiler_pid=""
        codex_restart_compiler=true
        break
      fi
    # Require sustained relief below the pause threshold before resuming, to avoid PSI flapping.
    elif [[ "$codex_compiler_suspended" == true ]]; then
      if [[ "$codex_psi_available" == false ]] \
        || ((codex_recent_some_us_per_second < codex_psi_some_resume_guard_us_per_second)); then
        ((codex_resume_clear_windows += 1))
      else
        codex_resume_clear_windows=0
      fi
      if [[ "$codex_psi_available" == false ]] || ((codex_resume_clear_windows >= codex_psi_resume_clear_windows_required)); then
        codex_resumed_at="$(date '+%Y-%m-%dT%H:%M:%S%z')"
        codex_resumed_at_epoch_us="${EPOCHREALTIME/./}"
        codex_pause_duration_us=$((codex_resumed_at_epoch_us - codex_compiler_suspended_at_epoch_us))
        printf -v codex_pause_duration '%d.%06ds' \
          "$((codex_pause_duration_us / 1000000))" "$((codex_pause_duration_us % 1000000))"
        ((codex_guard_pending_transition_count += 1))
        ((codex_guard_pending_resume_count += 1))
        ((codex_guard_pending_completed_pause_us += codex_pause_duration_us))
        [[ -n "$codex_guard_pending_first_at" ]] || codex_guard_pending_first_at="$codex_resumed_at"
        codex_guard_pending_last_at="$codex_resumed_at"
        codex_guard_pending_last_pause_duration="$codex_pause_duration"
        codex_guard_pending_last_pause_started_at="$codex_compiler_suspended_at"
        codex_guard_compiler_state=running
        kill -CONT -- "-$codex_compiler_pid" 2>/dev/null || true
        codex_compiler_suspended=false
        codex_resume_clear_windows=0
      fi
    fi
    codex_report_guard_transitions
    sleep 0.25
  done
  while ((codex_guard_pending_transition_count > 0)); do
    codex_report_guard_transitions
    ((codex_guard_pending_transition_count == 0)) || sleep 0.25
  done
  if [[ "$codex_restart_compiler" == true ]]; then
    continue
  fi
  codex_stop_psi_stream
  if wait "$codex_compiler_pid"; then
    exit 0
  else
    exit "$?"
  fi
done
