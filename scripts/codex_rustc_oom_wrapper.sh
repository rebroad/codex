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

codex_swap_headroom_percent="${CODEX_ANDROID_RUSTC_SWAP_HEADROOM_PERCENT:-50}"
codex_memavailable_critical_percent="${CODEX_ANDROID_RUSTC_MEMAVAILABLE_CRITICAL_PERCENT:-5}"
codex_psi_some_guard_ms="${CODEX_ANDROID_RUSTC_PSI_SOME_GUARD_MS:-35}"
codex_psi_full_guard_ms="${CODEX_ANDROID_RUSTC_PSI_FULL_GUARD_MS:-60}"
for codex_threshold in \
  "$codex_swap_headroom_percent" \
  "$codex_memavailable_critical_percent" \
  "$codex_psi_some_guard_ms" \
  "$codex_psi_full_guard_ms"; do
  if ! codex_uint "$codex_threshold"; then
    echo "invalid Android Rust memory guard threshold: $codex_threshold" >&2
    exit 2
  fi
done
codex_psi_some_resume_guard_us_per_second=$((codex_psi_some_guard_ms * 750))
codex_psi_resume_clear_windows_required=3
# PSI describes memory stalls, not necessarily imminent OOM. A rustc that
# stays suspended cannot make progress, so soft-pressure pauses are bounded.
# Hard RAM/swap exhaustion and actual LMKD reclaim still stop the compiler.
codex_psi_max_pause_seconds="${CODEX_ANDROID_RUSTC_PSI_MAX_PAUSE_SECONDS:-5}"
codex_psi_resume_grace_seconds="${CODEX_ANDROID_RUSTC_PSI_RESUME_GRACE_SECONDS:-30}"
for codex_threshold in "$codex_psi_max_pause_seconds" "$codex_psi_resume_grace_seconds"; do
  codex_uint "$codex_threshold" && ((codex_threshold > 0)) || {
    echo "invalid positive Rust PSI pause/grace duration: $codex_threshold" >&2
    exit 2
  }
done
codex_psi_resume_grace_until_epoch_us=0

# Keep the pause/kill distinction explicit: neither some nor full PSI alone
# is evidence that discarding an expensive rustc attempt will help.
codex_guard_is_psi_only() {
  [[ "${codex_guard_reason:-}" == memory\ PSI\ some\ stall* \
      || "${codex_guard_reason:-}" == memory\ PSI\ full\ stall* ]]
}
codex_psi_pause_limit_reached() {
  local now_us="$1" paused_us="${codex_compiler_suspended_at_epoch_us:-}"
  [[ "$paused_us" =~ ^[0-9]+$ ]] || return 1
  ((now_us - paused_us >= codex_psi_max_pause_seconds * 1000000))
}


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
      since="$(date +%s.%3N)"
      logcat -T "$since" -v epoch -s lmkd:I |
        while IFS= read -r line; do
          printf "L %s\n" "$line"
        done &
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
        printf "P %s %s\n" "$some" "$full"
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

# Other applications reclaimed by Android's LMKD are advisory information:
# they do not prove that this rustc needs to be killed or stopped.
codex_start_psi_stream || true

codex_read_psi() {
  local codex_record_kind codex_record_value codex_record_rest codex_lmkd_line
  local codex_lmkd_reclaim_process
  codex_psi_some_total_us=0
  codex_psi_full_total_us=0
  [[ -n "$codex_psi_stream_fd" ]] || return 1
  while IFS=' ' read -r -t 1 codex_record_kind codex_record_value codex_record_rest \
    <&"$codex_psi_stream_fd"; do
    case "$codex_record_kind" in
      P)
        codex_psi_some_total_us="$codex_record_value"
        codex_psi_full_total_us="$codex_record_rest"
        codex_uint "$codex_psi_some_total_us" && codex_uint "$codex_psi_full_total_us"
        return
        ;;
      L)
        codex_lmkd_line="$codex_record_value $codex_record_rest"
        if [[ "$codex_lmkd_line" == *"Reclaim '"* ]]; then
          codex_lmkd_reclaim_process=${codex_lmkd_line#*Reclaim \'}
          codex_lmkd_reclaim_process=${codex_lmkd_reclaim_process%%\'*}
          # Send diagnostics to stderr inside the rustc wrapper so Cargo's
          # compiler stdout stays protocol-clean. The build-wide log filter
          # forwards these notices to the build's actual stdout.
          if [[ "$codex_lmkd_reclaim_process" =~ ^[[:alnum:]_.:-]+$ ]]; then
            printf 'ℹ️ Android LMKD reclaimed %s; advisory only (no rustc stop from this event)\n' \
              "$codex_lmkd_reclaim_process" >&2
            # A reclaim event still warrants releasing disposable ballast,
            # but never proactively discarding this compiler's progress.
            if declare -F codex_ballast_request_relief >/dev/null 2>&1; then
              codex_ballast_request_relief
            fi
          fi
        fi
        ;;
    esac
  done
  return 1
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

# The ballast is shared across concurrent rustc wrappers. Its controller
# throttles signals so a burst of concurrent pressure reports cannot release
# many batches at once. This is advisory; the existing guard never waits for it.
codex_ballast_request_relief() {
  local ballast_pid="${CODEX_ANDROID_RUSTC_BALLAST_PID:-}"
  local ballast_status="${CODEX_ANDROID_RUSTC_BALLAST_STATUS_FILE:-}"
  local held_mb=0
  [[ "${ballast_pid}" =~ ^[0-9]+$ && -r "${ballast_status}" ]] || return 0
  read -r held_mb <"${ballast_status}" || return 0
  if codex_uint "${held_mb}" && ((held_mb > 0)); then
    kill -USR1 "${ballast_pid}" 2>/dev/null || true
  fi
}

# This file belongs to the build-wide controller. Every compiler shares the
# same counters; the build's awk output deduplicates concurrent status reports.
# Called only alongside the shared Rust timing snapshot. The build-wide
# output filter controls frequency; this does not print on every 8 MiB change.
codex_report_ballast_telemetry() {
  local telemetry_file="${CODEX_ANDROID_RUSTC_BALLAST_TELEMETRY_FILE:-}"
  local held_mb max_mb released_mb refilled_mb psi_some psi_full
  [[ -n "${telemetry_file}" && -r "${telemetry_file}" ]] || return 0
  if ! read -r held_mb max_mb released_mb refilled_mb <"${telemetry_file}"; then
    return 0
  fi
  codex_uint "${held_mb}" && codex_uint "${max_mb}" \
    && codex_uint "${released_mb}" && codex_uint "${refilled_mb}" || return 0
  psi_some=n/a
  psi_full=n/a
  if [[ "${codex_psi_available}" == true ]] && (("${#codex_some_deltas[@]}" >= 5)); then
    printf -v psi_some '%d.%01d ms/s' \
      "$((codex_recent_some_us_per_second / 1000))" \
      "$(((codex_recent_some_us_per_second % 1000) / 100))"
    printf -v psi_full '%d.%01d ms/s' \
      "$((codex_recent_full_us_per_second / 1000))" \
      "$(((codex_recent_full_us_per_second % 1000) / 100))"
  fi
  printf '🧠 Ballast: %s/%s MiB held | released %s MiB | refilled %s MiB | MemAvailable %s MiB | PSI some %s, full %s\n' \
    "${held_mb}" "${max_mb}" "${released_mb}" "${refilled_mb}" \
    "$((codex_mem_available_kb / 1024))" "${psi_some}" "${psi_full}" >&2
}

# Share a recent, system-wide PSI rate with the ballast controller. Android
# restricts direct /proc/pressure reads in Termux; this uses the PSI already
# obtained through the existing adb stream, with one atomic write per second
# per rustc wrapper. Stale samples are rejected by the controller.
codex_ballast_last_psi_sample_us=0
codex_publish_ballast_psi() {
  local path="${CODEX_ANDROID_RUSTC_BALLAST_PSI_FILE:-}" now_us tmp
  [[ -n "$path" && "$codex_psi_available" == true ]] || return 0
  now_us="${EPOCHREALTIME/./}"
  if ((codex_ballast_last_psi_sample_us > 0 \
    && now_us - codex_ballast_last_psi_sample_us < 1000000)); then
    return 0
  fi
  tmp="$path.$"
  if printf '%s %s %s\n' "$now_us" \
      "$codex_recent_some_us_per_second" "$codex_recent_full_us_per_second" >"$tmp"; then
    mv -f -- "$tmp" "$path" 2>/dev/null || rm -f -- "$tmp"
    codex_ballast_last_psi_sample_us="$now_us"
  fi
}

codex_memory_guard_reason() {
  if ((codex_mem_available_kb * 100 <= codex_mem_total_kb * codex_memavailable_critical_percent)); then
    codex_guard_reason="MemAvailable ${codex_mem_available_kb}kB <= ${codex_memavailable_critical_percent}% critical RAM floor"
    codex_ballast_request_relief
    return 0
  fi
  if ((codex_swap_total_kb > 0 \
    && (codex_mem_available_kb + codex_swap_free_kb) * 100 \
      <= codex_swap_total_kb * codex_swap_headroom_percent)); then
    codex_guard_reason="combined RAM and swap headroom $((codex_mem_available_kb + codex_swap_free_kb))kB <= ${codex_swap_headroom_percent}% of configured swap"
    codex_ballast_request_relief
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
      codex_publish_ballast_psi
      # Relieve pressure before reaching the existing suspend/stop threshold.
      # A release cannot guarantee lower PSI; keep all guard limits unchanged.
      if ((codex_recent_some_us_per_second >= codex_psi_some_guard_ms * 500 \
        || codex_recent_full_us_per_second >= codex_psi_full_guard_ms * 500)); then
        codex_ballast_request_relief
      fi
      # High full PSI gets a reversible, bounded pause just like some PSI.
      # Do not confuse an observed stall with proof of insufficient headroom.
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
codex_compiler_ended_epoch_us=""
codex_compiler_started_epoch_us=""
codex_compiler_suspended_total_us=0
# Accounting is shared across rustc wrappers and never retroactively changes
# an already reported counter. An attempt is only credited to Kept or Lost
# when we know whether its output was retained.
source "${BASH_SOURCE[0]%/*}/codex_rustc_timing.sh"
codex_timing_init
# Link each accounting transition to the rustc crate and (for generic test
# targets) the source path, regardless of interleaved Cargo progress output.
codex_timing_identify_compiler "$@"
# Confirm a compiler actually loaded this implementation, once per Cargo
# stage. The build's wrapper-path banner alone cannot prove execution.
if [[ -n "${CODEX_ANDROID_RUSTC_TIMING_DIR:-}" ]] \
  && mkdir "${CODEX_ANDROID_RUSTC_TIMING_DIR}/wrapper-started" 2>/dev/null; then
  echo "Android Rust guard active: Kept/Suspended/Lost timing; event-only pause/resume" >&2
fi
codex_compiler_stdout=""
codex_compiler_stderr=""
# One observable pause/resume notification per actual state transition. The
# watchdog can poll frequently without generating repeated status messages.
codex_guard_compile_state=running
codex_guard_pause_mode=running
codex_guard_pause_started_at=""
codex_guard_pause_started_epoch_us=""
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

codex_guard_mark_paused() {
  local reason="$1" mode="${2:-waiting}"
  if [[ "$codex_guard_compile_state" == paused ]]; then
    # A killed suspended process is a new event. Continuing to wait isn't.
    if [[ "$mode" == stopped && "$codex_guard_pause_mode" != stopped ]]; then
      echo "⏹ Rust attempt stopped: $reason; waiting to retry" >&2
    fi
    codex_guard_pause_mode="$mode"
    return 0
  fi
  codex_timing_pause
  codex_guard_compile_state=paused
  codex_guard_pause_mode="$mode"
  codex_guard_pause_started_at="$(date '+%Y-%m-%dT%H:%M:%S%z')"
  codex_guard_pause_started_epoch_us="${EPOCHREALTIME/./}"
  case "$mode" in
    stopped)
      echo "⏹ Rust attempt stopped at $codex_guard_pause_started_at: $reason; waiting to retry" >&2 ;;
    waiting)
      echo "⏸ Rust waiting at $codex_guard_pause_started_at: $reason" >&2 ;;
    *)
      echo "⏸ Rust suspended at $codex_guard_pause_started_at: $reason" >&2 ;;
  esac
}

codex_guard_mark_running() {
  [[ "$codex_guard_compile_state" == paused ]] || return 0
  local now elapsed_us duration
  now="${EPOCHREALTIME/./}"
  elapsed_us=$((now - codex_guard_pause_started_epoch_us))
  ((elapsed_us >= 0)) || elapsed_us=0
  printf -v duration '%d.%03ds' \
    "$((elapsed_us / 1000000))" "$(((elapsed_us % 1000000) / 1000))"
  codex_timing_resume
  codex_guard_compile_state=running
  codex_guard_pause_mode=running
  # Allow a window of forward progress after PSI-only pauses, while always
  # letting actual RAM/swap/LMKD emergencies terminate a compiler.
  codex_psi_resume_grace_until_epoch_us=$((now + ${codex_psi_resume_grace_seconds:-30} * 1000000))
  # Re-arm the PSI suspension path after each successful resume. Otherwise
  # the next pressure spike sees a stale 'true' and never sends SIGSTOP.
  codex_compiler_suspended=false
  codex_compiler_suspended_at=""
  codex_compiler_suspended_at_epoch_us=""
  echo "▶️ Rust resumed at $(date '+%Y-%m-%dT%H:%M:%S%z') (paused $duration${codex_guard_resume_note:+; $codex_guard_resume_note})" >&2
  codex_guard_resume_note=""
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
  codex_compiler_ended_epoch_us="${EPOCHREALTIME/./}"
}
codex_cleanup_compiler_output() {
  [[ -z "$codex_compiler_stdout" ]] || rm -f "$codex_compiler_stdout"
  [[ -z "$codex_compiler_stderr" ]] || rm -f "$codex_compiler_stderr"
  codex_compiler_stdout=""
  codex_compiler_stderr=""
}
codex_abort_compiler() {
  local code="$1"
  if [[ -n "$codex_compiler_pid" ]]; then
    codex_timing_pause
    codex_stop_compiler
    wait "$codex_compiler_pid" 2>/dev/null || true
    codex_timing_lost
    codex_compiler_pid=""
  fi
  # Also commit any outstanding time spent waiting for memory, when an
  # interrupted wrapper has no active compiler.
  codex_report_timing_counters
  codex_stop_psi_stream
  codex_cleanup_compiler_output
  exit "$code"
}
trap 'codex_abort_compiler 130' INT
trap 'codex_abort_compiler 143' TERM
trap 'codex_abort_compiler 129' HUP

codex_wait_for_memory() {
  local psi_wait_started_epoch_us=0 now_epoch_us
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
    if codex_memory_guard_reason; then
      if codex_guard_is_psi_only; then
        now_epoch_us="${EPOCHREALTIME/./}"
        if ((psi_wait_started_epoch_us == 0)); then
          psi_wait_started_epoch_us="$now_epoch_us"
        fi
        # PSI stalls alone must not block rustc from ever starting. Critical
        # RAM/swap checks are still enforced on every iteration.
        if ((now_epoch_us - psi_wait_started_epoch_us >=
              codex_psi_max_pause_seconds * 1000000)); then
          return 0
        fi
      else
        psi_wait_started_epoch_us=0
      fi
    else
      return 0
    fi
    codex_guard_mark_paused "$codex_guard_reason" waiting
    sleep 1
  done
}

while true; do
  if ! codex_wait_for_memory; then
    codex_report_timing_counters
    codex_stop_psi_stream
    exit 1
  fi
  codex_temp_root="${TMPDIR:-${PREFIX:-/data/data/com.termux/files/usr}/tmp}"
  codex_compiler_stdout="$(mktemp "${codex_temp_root%/}/codex-rustc-stdout.XXXXXX")"
  codex_compiler_stderr="$(mktemp "${codex_temp_root%/}/codex-rustc-stderr.XXXXXX")"
  /system/bin/setsid "${codex_compiler_command[@]}" \
    >"$codex_compiler_stdout" 2>"$codex_compiler_stderr" &
  codex_compiler_pid=$!
  codex_timing_launch
  codex_timing_emit_start
  codex_guard_mark_running
  codex_compiler_started_epoch_us="${EPOCHREALTIME/./}"
  codex_compiler_ended_epoch_us=""
  codex_restart_compiler=false
  codex_compiler_suspended=false
  codex_guard_compiler_state=running
  codex_resume_clear_windows=0
  codex_compiler_waited=false
  while kill -0 "$codex_compiler_pid" 2>/dev/null; do
    codex_read_memory || {
      echo 'Android Rust memory guard lost access to /proc/meminfo; stopping the compiler' >&2
      codex_timing_pause
      codex_stop_compiler
      codex_stop_psi_stream
      wait "$codex_compiler_pid" 2>/dev/null || true
      codex_timing_lost
      codex_compiler_pid=""
      codex_report_timing_counters
      codex_cleanup_compiler_output
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
      if codex_guard_is_psi_only; then
        # Both PSI some and full indicate stalls; neither alone warrants
        # discarding rustc's work. Pause reversibly, but ensure a suspended
        # compiler gets another chance to make progress after 5 seconds.
        codex_resume_clear_windows=0
        codex_now_epoch_us="${EPOCHREALTIME/./}"
        if [[ "$codex_compiler_suspended" == true ]]; then
          if codex_psi_pause_limit_reached "$codex_now_epoch_us"; then
            kill -CONT -- "-$codex_compiler_pid" 2>/dev/null || true
            codex_guard_resume_note="PSI-only backoff limit; ${codex_psi_resume_grace_seconds}s progress grace"
            codex_guard_mark_running
          fi
        elif ((codex_now_epoch_us >= codex_psi_resume_grace_until_epoch_us)); then
          codex_compiler_suspended_at="$(date '+%Y-%m-%dT%H:%M:%S%z')"
          codex_compiler_suspended_at_epoch_us="$codex_now_epoch_us"
          codex_guard_mark_paused "$codex_guard_reason" suspended
          kill -STOP -- "-$codex_compiler_pid" 2>/dev/null || true
          codex_compiler_suspended=true
        fi
      else
        # Transition straight to stopped, not briefly through running.
        # Once pressure forces termination, time spent delivering signals
        # and waiting for a retry is suspended, not useful or lost work.
        codex_guard_mark_paused "$codex_guard_reason" stopped
        codex_stop_compiler
        codex_compiler_status=0
        wait "$codex_compiler_pid" 2>/dev/null || codex_compiler_status=$?
        codex_compiler_waited=true
        # The compiler may have finished just before we sent TERM. Classify
        # only after wait reveals whether Cargo can retain its output.
        if ((codex_compiler_status == 0)); then
          codex_timing_success
        else
          codex_timing_lost
        fi
        codex_compiler_pid=""
        codex_compiler_started_epoch_us=""
        codex_report_timing_counters
        if ((codex_compiler_status == 0)); then
          codex_compiler_suspended=false
          codex_compiler_suspended_at_epoch_us=""
          codex_guard_compile_state=running
          break
        fi
        # A killed rustc may have emitted only part of Cargo's JSON artifact
        # stream. Discard that attempt so a retry cannot report duplicate or
        # incomplete artifacts to Cargo's dependency queue.
        codex_cleanup_compiler_output
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
        kill -CONT -- "-$codex_compiler_pid" 2>/dev/null || true
        codex_guard_mark_running
        codex_resume_clear_windows=0
      fi
    fi
    codex_timing_periodic_report
    sleep 0.25
  done
  if [[ "$codex_restart_compiler" == true ]]; then
    continue
  fi
  codex_stop_psi_stream
  codex_compiler_status=0
  if [[ "$codex_compiler_waited" != true ]]; then
    wait "$codex_compiler_pid" || codex_compiler_status=$?
    if ((codex_compiler_status == 0)); then
      codex_timing_success
    else
      codex_timing_lost
    fi
  fi
  codex_compiler_pid=""
  codex_compiler_started_epoch_us=""
  codex_compiler_suspended=false
  codex_compiler_suspended_at_epoch_us=""
  codex_report_timing_counters
  cat "$codex_compiler_stdout"
  cat "$codex_compiler_stderr" >&2
  codex_cleanup_compiler_output
  exit "$codex_compiler_status"
done
