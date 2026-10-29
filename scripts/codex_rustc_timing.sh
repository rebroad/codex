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
  # Per-stage ledger: short-lived Cargo accounting, used by ballast autotune.
  # Global ledger: durable across ALL wrappers, including ad-hoc Cargo commands
  # with no CODEX_ANDROID_RUSTC_TIMING_DIR, so displayed totals never reset.
  codex_timing_file=""
  codex_timing_stage_file=""
  local stage_dir="${CODEX_ANDROID_RUSTC_TIMING_DIR:-}"
  local global_dir="${CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR:-${BASH_SOURCE[0]%/*}/../build/codex-rust-timing-global}"

  if ! command -v flock >/dev/null 2>&1; then
    printf 'warning: shared Rust time ledger unavailable: flock not in compiler PATH: %s\n' "${PATH:-}" >&2
  else
    if mkdir -p -- "$global_dir" 2>/dev/null && [[ -w "$global_dir" ]]; then
      codex_timing_file="$global_dir/totals"
    else
      printf 'warning: project-wide Rust timing directory unavailable: %s\n' "$global_dir" >&2
    fi
    if [[ -n "$stage_dir" ]]; then
      if [[ -d "$stage_dir" && -w "$stage_dir" ]]; then
        codex_timing_stage_file="$stage_dir/totals"
      else
        printf 'warning: Cargo stage Rust timing directory missing or unwritable: %s\n' "$stage_dir" >&2
      fi
    fi
    # Prefer the global aggregate for user-visible figures. If unavailable,
    # the per-stage ledger is still useful for accounting and rate-limits.
    [[ -n "$codex_timing_file" ]] || codex_timing_file="$codex_timing_stage_file"
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
  local total_seconds=$(( $1 / 1000000 ))
  local hours=$((total_seconds / 3600))
  local minutes=$(((total_seconds / 60) % 60))
  local seconds=$((total_seconds % 60))
  local duration=""
  ((hours == 0)) || duration+="${hours}h"
  ((minutes == 0)) || duration+="${minutes}m"
  if ((seconds > 0)) || [[ -z "$duration" ]]; then
    duration+="${seconds}s"
  fi
  printf -v "$2" '%s' "$duration"
}

# The wrapper receives Rustc's actual argv (compiler path followed by flags).
# Cargo's "Compiling ..." lines can interleave across multiple compiler
# processes; keep this immutable label and each attempt PID on transitions.
codex_timing_identify_compiler() {
  codex_timing_crate=unknown
  codex_timing_source=""
  local arg
  while (($#)); do
    arg="$1"
    case "$arg" in
      --crate-name)
        shift
        codex_timing_crate="${1:-unknown}"
        ;;
      --crate-name=*)
        codex_timing_crate="${arg#--crate-name=}"
        ;;
      *.rs)
        if [[ -z "$codex_timing_source" && "$arg" != -* ]]; then
          codex_timing_source="$arg"
        fi
        ;;
    esac
    shift
  done
  # Rust test targets often have generic names like "all"; the input path
  # disambiguates these in otherwise identical-looking timing updates.
  case "$codex_timing_crate" in
    all|lib|main|unknown)
      if [[ -n "$codex_timing_source" ]]; then
        codex_timing_source="${codex_timing_source#*/codex-rs/}"
      fi
      ;;
    *) codex_timing_source="" ;;
  esac
}

codex_timing_emit_transition() {
  local kind="$1" micros="$2" duration context
  [[ "$kind" == kept || "$kind" == lost ]] || return 0
  # The aggregate is updated first. Each unthrottled transition then explains
  # exactly which compiler attempt changed Kept or Lost.
  codex_timing_format "$micros" duration
  if ((micros > 0 && micros < 1000000)); then
    duration="<1s"
  fi
  context="crate=${codex_timing_crate:-unknown} pid=${codex_compiler_pid:-unknown}"
  if [[ -n "${codex_timing_source:-}" ]]; then
    context+=" src=$codex_timing_source"
  fi
  case "$kind" in
    kept) printf '%s ✓ kept+%s %s\n' "$(date '+%H:%M:%S')" "$duration" "$context" >&2 ;;
    lost) printf '%s ✗ lost+%s %s\n' "$(date '+%H:%M:%S')" "$duration" "$context" >&2 ;;
  esac
}

codex_timing_emit_start() {
  local context="crate=${codex_timing_crate:-unknown} pid=${codex_compiler_pid:-unknown}"
  if [[ -n "${codex_timing_source:-}" ]]; then
    context+=" src=$codex_timing_source"
  fi
  printf '%s ▶ start %s\n' "$(date '+%H:%M:%S')" "$context" >&2
}

codex_timing_emit() {
  local running_duration paused_duration lost_duration pending_duration
  codex_timing_format "$1" running_duration
  codex_timing_format "$2" paused_duration
  codex_timing_format "$3" lost_duration
  codex_timing_format "$4" pending_duration
  printf '%s kept=%s slept=%s lost=%s this=%s\n' \
    "$(date '+%H:%M:%S')" \
    "$running_duration" "$paused_duration" "$lost_duration" "$pending_duration" >&2
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
  local running=0 suspended=0 lost=0 lock_fd ledger_file
  local transition_kind="" transition_us=0
  local report_running=0 report_suspended=0 report_lost=0
  local -a ledgers=()
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
      transition_kind=kept
      transition_us="$running_delta"
      codex_timing_attempt_us=0
      codex_timing_mode="${codex_guard_compile_state:-running}"
      [[ "${codex_timing_mode}" == paused ]] || codex_timing_mode=idle ;;
    lost)
      lost_delta="${codex_timing_attempt_us}"
      transition_kind=lost
      transition_us="$lost_delta"
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
    codex_timing_emit_transition "$transition_kind" "$transition_us"
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
  ledgers=("$codex_timing_file")
  if [[ -n "$codex_timing_stage_file" \
    && "$codex_timing_stage_file" != "$codex_timing_file" ]]; then
    ledgers+=("$codex_timing_stage_file")
  fi
  for ledger_file in "${ledgers[@]}"; do
    running=0
    suspended=0
    lost=0
    exec {lock_fd}>"${ledger_file}.lock"
    flock -x "${lock_fd}"
    if [[ -r "$ledger_file" ]]; then
      if ! read -r running suspended lost <"$ledger_file" \
        || ! [[ "$running" =~ ^[0-9]+$ && "$suspended" =~ ^[0-9]+$ && "$lost" =~ ^[0-9]+$ ]]; then
        echo "warning: Rust timing ledger invalid: $ledger_file; restarting its counters" >&2
        running=0
        suspended=0
        lost=0
      fi
    fi
    running=$((running + running_delta))
    suspended=$((suspended + suspended_delta))
    lost=$((lost + lost_delta))
    if ((running_delta > 0 || suspended_delta > 0 || lost_delta > 0)); then
      printf '%s %s %s\n' "$running" "$suspended" "$lost" \
        >"${ledger_file}.tmp.$$"
      mv -f "${ledger_file}.tmp.$$" "$ledger_file"
    fi
    if [[ "$ledger_file" == "$codex_timing_file" ]]; then
      report_running="$running"
      report_suspended="$suspended"
      report_lost="$lost"
    fi
    flock -u "${lock_fd}"
    exec {lock_fd}>&-
  done
  codex_timing_emit_transition "$transition_kind" "$transition_us"
  if [[ "$report" == true ]]; then
    codex_timing_emit "$report_running" "$report_suspended" "$report_lost" \
      "${codex_timing_attempt_us}"
  fi
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

