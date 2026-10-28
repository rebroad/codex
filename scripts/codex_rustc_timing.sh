#!/usr/bin/env bash
# Shared, monotonic Rust compiler accounting for the Android OOM wrapper.
#
# All wrappers in one build write deltas to a flock-protected ledger. Running
# includes time in attempts that may later fail. Lost is a SUBSET of running,
# not a bucket from which we subtract previously reported running time.
# With multiple concurrent rustc processes, these are aggregate process
# seconds, not wall-clock seconds.

codex_timing_get_now() {
  # /proc/uptime uses the kernel's monotonic boot clock. Use its decimal
  # digits (usually centiseconds) without date/time command subprocesses.
  local uptime_value uptime_seconds uptime_fraction
  read -r uptime_value _ </proc/uptime
  uptime_seconds="${uptime_value%%.*}"
  if [[ "${uptime_value}" == *.* ]]; then
    uptime_fraction="${uptime_value#*.}000000"
    uptime_fraction="${uptime_fraction:0:6}"
  else
    uptime_fraction=000000
  fi
  CODEX_TIMING_NOW_US=$((10#${uptime_seconds} * 1000000 + 10#${uptime_fraction}))
}

codex_timing_init() {
  local owner_pid="${PPID}" owner_start
  if [[ "${CODEX_BUILD_PID:-}" =~ ^[0-9]+$ && -r "/proc/${CODEX_BUILD_PID}/stat" ]]; then
    owner_pid="${CODEX_BUILD_PID}"
  fi
  owner_start="$(sed 's/^.*) //' "/proc/${owner_pid}/stat" 2>/dev/null | awk '{print $20}')"
  codex_timing_state_file=""
  codex_timing_local_running_us=0
  codex_timing_local_suspended_us=0
  codex_timing_local_lost_us=0
  # Tracks non-suspended work on the current invocation alone. On failure,
  # classify exactly this amount as discarded; never subtract from running.
  codex_timing_attempt_running_us=0
  codex_timing_state_lock_fd=""
  if [[ "${owner_start}" =~ ^[0-9]+$ ]] && command -v flock >/dev/null 2>&1; then
    local state_dir="${TMPDIR:-${PREFIX:-/data/data/com.termux/files/usr}/tmp}"
    mkdir -p "${state_dir}"
    codex_timing_state_file="${state_dir}/codex-rustc-time-v2-${owner_pid}-${owner_start}"
  else
    echo 'warning: shared rustc timing ledger unavailable; reporting only this wrapper' >&2
  fi
  codex_timing_get_now
  codex_timing_last_epoch_us="${CODEX_TIMING_NOW_US}"
}

codex_timing_format_duration() {
  local whole_seconds=$(( $1 / 1000000 ))
  printf -v "$2" '%02d:%02d:%02d' \
    "$((whole_seconds / 3600))" "$(((whole_seconds / 60) % 60))" "$((whole_seconds % 60))"
}

codex_timing_emit() {
  local running_duration suspended_duration lost_duration
  codex_timing_format_duration "$1" running_duration
  codex_timing_format_duration "$2" suspended_duration
  codex_timing_format_duration "$3" lost_duration
  printf 'Rust time: running %s; suspended %s; lost %s\n' \
    "${running_duration}" "${suspended_duration}" "${lost_duration}" >&2
}

# Add elapsed work since our previous checkpoint, optionally mark an attempt
# as lost and/or emit a report. Emit under the same ledger lock to guarantee
# consistent ordering between concurrently running compiler wrappers.
codex_timing_update() {
  local completion="${1:-none}" report="${2:-false}" lost_delta_us=0
  local now_us elapsed_us=0 running_delta_us=0 suspended_delta_us=0
  local running_total suspended_total lost_total
  codex_timing_get_now
  now_us="${CODEX_TIMING_NOW_US}"
  if ((now_us > codex_timing_last_epoch_us)); then
    elapsed_us=$((now_us - codex_timing_last_epoch_us))
  fi
  codex_timing_last_epoch_us="${now_us}"

  if [[ "${codex_guard_compile_state}" == paused ]]; then
    suspended_delta_us="${elapsed_us}"
  elif [[ -n "${codex_compiler_pid:-}" ]]; then
    running_delta_us="${elapsed_us}"
  fi
  codex_timing_attempt_running_us=$((codex_timing_attempt_running_us + running_delta_us))
  if [[ "${completion}" == lost ]]; then
    lost_delta_us="${codex_timing_attempt_running_us}"
    codex_timing_attempt_running_us=0
  elif [[ "${completion}" == complete ]]; then
    codex_timing_attempt_running_us=0
  fi
  if [[ -z "${codex_timing_state_file}" ]]; then
    codex_timing_local_running_us=$((codex_timing_local_running_us + running_delta_us))
    codex_timing_local_suspended_us=$((codex_timing_local_suspended_us + suspended_delta_us))
    codex_timing_local_lost_us=$((codex_timing_local_lost_us + lost_delta_us))
    if [[ "${report}" == true ]]; then
      codex_timing_emit "${codex_timing_local_running_us}" \
        "${codex_timing_local_suspended_us}" "${codex_timing_local_lost_us}"
    fi
    return 0
  fi
  # Avoid a lock acquisition/write on no-change ticks. Reports still read and
  # emit while holding the lock so they cannot appear out of order.
  if ((running_delta_us == 0 && suspended_delta_us == 0 && lost_delta_us == 0)) \
    && [[ "${report}" != true ]]; then
    return 0
  fi
  exec {codex_timing_state_lock_fd}>"${codex_timing_state_file}.lock"
  flock -x "${codex_timing_state_lock_fd}"
  running_total=0
  suspended_total=0
  lost_total=0
  if [[ -r "${codex_timing_state_file}" ]]; then
    if ! read -r running_total suspended_total lost_total <"${codex_timing_state_file}" \
      || ! [[ "${running_total}" =~ ^[0-9]+$ && "${suspended_total}" =~ ^[0-9]+$ \
        && "${lost_total}" =~ ^[0-9]+$ ]]; then
      running_total=0
      suspended_total=0
      lost_total=0
    fi
  fi
  running_total=$((running_total + running_delta_us))
  suspended_total=$((suspended_total + suspended_delta_us))
  lost_total=$((lost_total + lost_delta_us))
  if ((running_delta_us > 0 || suspended_delta_us > 0 || lost_delta_us > 0)); then
    printf '%s %s %s\n' "${running_total}" "${suspended_total}" "${lost_total}" \
      >"${codex_timing_state_file}.tmp.$$"
    mv -f "${codex_timing_state_file}.tmp.$$" "${codex_timing_state_file}"
  fi
  if [[ "${report}" == true ]]; then
    codex_timing_emit "${running_total}" "${suspended_total}" "${lost_total}"
  fi
  flock -u "${codex_timing_state_lock_fd}"
  exec {codex_timing_state_lock_fd}>&-
}

codex_timing_tick() {
  codex_timing_get_now
  if ((CODEX_TIMING_NOW_US - codex_timing_last_epoch_us >= 1000000)); then
    codex_timing_update
  fi
}

codex_report_timing_counters() {
  codex_timing_update none true
}

codex_commit_timing_counters() {
  codex_timing_update complete
}

codex_timing_mark_lost() {
  codex_timing_update lost
}
