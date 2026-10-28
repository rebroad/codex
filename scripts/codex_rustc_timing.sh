#!/usr/bin/env bash
# Rust compiler accounting. Cumulative kept, lost, and suspended
# are mutually exclusive, never-decreasing compiler-process durations.
# Active attempts remain "in progress" until successfully finished or lost.
# With concurrent rustc processes, totals are process-seconds, not wall-time.

codex_timing_now() {
  # Bash's built-in EPOCHREALTIME works in Termux without /proc/uptime or a
  # Python subprocess. Clamp backward wall-clock adjustments to zero elapsed.
  local now="${EPOCHREALTIME}" seconds fraction
  seconds="${now%%.*}"
  fraction="${now#*.}"
  fraction="${fraction}000000"
  fraction="${fraction:0:6}"
  CODEX_TIMING_NOW_US=$((10#${seconds} * 1000000 + 10#${fraction}))
}

codex_timing_init() {
  # The build script supplies one unique directory to ALL rustc wrappers.
  # Android restricts /proc/<pid>/stat, so never require it to share counters.
  codex_timing_file=""
  if [[ -n "${CODEX_ANDROID_RUSTC_TIMING_DIR:-}" && -d "${CODEX_ANDROID_RUSTC_TIMING_DIR}" ]] \
    && command -v flock >/dev/null 2>&1; then
    codex_timing_file="${CODEX_ANDROID_RUSTC_TIMING_DIR}/totals"
  else
    echo "warning: shared Rust time ledger unavailable (missing timing directory or flock)" >&2
  fi
  codex_timing_mode=idle
  codex_timing_attempt_us=0
  codex_timing_pending_suspended_us=0
  codex_timing_local_running_us=0
  codex_timing_local_suspended_us=0
  codex_timing_local_lost_us=0
  codex_timing_now
  codex_timing_previous_us="${CODEX_TIMING_NOW_US}"
  codex_timing_last_periodic_us="${CODEX_TIMING_NOW_US}"
}
codex_timing_format() {
  local seconds=$(( $1 / 1000000 ))
  printf -v "$2" '%02d:%02d:%02d' \
    "$((seconds / 3600))" "$(((seconds / 60) % 60))" "$((seconds % 60))"
}

codex_timing_emit() {
  local running_duration paused_duration lost_duration pending_duration
  codex_timing_format "$1" running_duration
  codex_timing_format "$2" paused_duration
  codex_timing_format "$3" lost_duration
  codex_timing_format "$4" pending_duration
  printf 'Rust time: kept %s; suspended %s; lost %s; in progress (this rustc) %s\n' \
    "${running_duration}" "${paused_duration}" "${lost_duration}" "${pending_duration}" >&2
  # Ballast shares the timer report cadence instead of producing independent
  # status messages every time it allocates or releases pages.
  if declare -F codex_report_ballast_telemetry >/dev/null 2>&1; then
    codex_report_ballast_telemetry
  fi
}

# Events account all time up to the boundary, then switch mode. Successful
# output credits pending work to kept, discarded output credits it to lost.
# Suspended time accrues independently of the compiler attempt's outcome.
codex_timing_event() {
  local event="${1:-tick}" report="${2:-false}"
  local now_us elapsed_us=0 running_delta=0 lost_delta=0 suspended_delta
  local running=0 suspended=0 lost=0 lock_fd
  codex_timing_now
  now_us="${CODEX_TIMING_NOW_US}"
  if ((now_us > codex_timing_previous_us)); then
    elapsed_us=$((now_us - codex_timing_previous_us))
  fi
  codex_timing_previous_us="${now_us}"

  case "${codex_timing_mode}" in
    active) codex_timing_attempt_us=$((codex_timing_attempt_us + elapsed_us)) ;;
    paused) codex_timing_pending_suspended_us=$((codex_timing_pending_suspended_us + elapsed_us)) ;;
  esac
  case "${event}" in
    launch) codex_timing_mode=active ;;
    pause) codex_timing_mode=paused ;;
    resume)
      if [[ -n "${codex_compiler_pid:-}" ]]; then
        codex_timing_mode=active
      else
        codex_timing_mode=idle
      fi ;;
    success)
      running_delta="${codex_timing_attempt_us}"
      codex_timing_attempt_us=0
      codex_timing_mode="${codex_guard_compile_state:-running}"
      [[ "${codex_timing_mode}" == paused ]] || codex_timing_mode=idle ;;
    lost)
      lost_delta="${codex_timing_attempt_us}"
      codex_timing_attempt_us=0
      codex_timing_mode="${codex_guard_compile_state:-running}"
      [[ "${codex_timing_mode}" == paused ]] || codex_timing_mode=idle ;;
    tick) ;;
    *) echo "invalid Rust timing event: ${event}" >&2; return 2 ;;
  esac
  suspended_delta="${codex_timing_pending_suspended_us}"
  codex_timing_pending_suspended_us=0
  if [[ -z "${codex_timing_file}" ]]; then
    codex_timing_local_running_us=$((codex_timing_local_running_us + running_delta))
    codex_timing_local_suspended_us=$((codex_timing_local_suspended_us + suspended_delta))
    codex_timing_local_lost_us=$((codex_timing_local_lost_us + lost_delta))
    if [[ "${report}" == true ]]; then
      codex_timing_emit "${codex_timing_local_running_us}" \
        "${codex_timing_local_suspended_us}" "${codex_timing_local_lost_us}" \
        "${codex_timing_attempt_us}"
    fi
    return 0
  fi
  if ((running_delta == 0 && lost_delta == 0 && suspended_delta == 0)) \
    && [[ "${report}" != true ]]; then
    return 0
  fi
  exec {lock_fd}>"${codex_timing_file}.lock"
  flock -x "${lock_fd}"
  if [[ -r "${codex_timing_file}" ]]; then
    if ! read -r running suspended lost <"${codex_timing_file}" \
      || ! [[ "${running}" =~ ^[0-9]+$ && "${suspended}" =~ ^[0-9]+$ && "${lost}" =~ ^[0-9]+$ ]]; then
      echo "warning: Rust timing ledger invalid; restarting counters" >&2
      running=0
      suspended=0
      lost=0
    fi
  fi
  running=$((running + running_delta))
  suspended=$((suspended + suspended_delta))
  lost=$((lost + lost_delta))
  if ((running_delta > 0 || suspended_delta > 0 || lost_delta > 0)); then
    printf '%s %s %s\n' "${running}" "${suspended}" "${lost}" \
      >"${codex_timing_file}.tmp.$$"
    mv -f "${codex_timing_file}.tmp.$$" "${codex_timing_file}"
  fi
  if [[ "${report}" == true ]]; then
    codex_timing_emit "${running}" "${suspended}" "${lost}" "${codex_timing_attempt_us}"
  fi
  flock -u "${lock_fd}"
  exec {lock_fd}>&-
}

codex_timing_pause() { codex_timing_event pause; }
codex_timing_resume() { codex_timing_event resume; }
codex_timing_launch() { codex_timing_event launch; }
codex_timing_success() { codex_timing_event success; }
codex_timing_lost() { codex_timing_event lost; }
codex_report_timing_counters() { codex_timing_event tick true; }

# Periodic snapshots only during active work. Suspended/waiting is announced
# once at the transition, never reiterated every poll.
codex_timing_periodic_report() {
  [[ "${codex_timing_mode}" == active ]] || return 0
  codex_timing_now
  if ((CODEX_TIMING_NOW_US - codex_timing_last_periodic_us >= 60000000)); then
    codex_timing_last_periodic_us="${CODEX_TIMING_NOW_US}"
    codex_report_timing_counters
  fi
}

