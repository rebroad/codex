#!/usr/bin/env bash
# Deterministic tests for shared Rust timing, no Android required.
set -euo pipefail
test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
temp="$(mktemp -d)"
trap 'rm -rf -- "$temp"' EXIT
export TMPDIR="$temp" CODEX_ANDROID_RUSTC_TIMING_DIR="$temp"
source "${test_dir}/../codex_rustc_timing.sh"

# The wrapper supplies this hook. Ballast must be printed with timing, not
# independently on every controller allocation/release tick.
codex_report_ballast_telemetry() {
  printf '🧠 Ballast: 96/256 MiB held | released 160 MiB | refilled 0 MiB\n' >&2
}

clock=1000000
codex_timing_now() { CODEX_TIMING_NOW_US="$clock"; }

assert_ledger() {
  local got_kept got_suspended got_lost
  read -r got_kept got_suspended got_lost <"$codex_timing_file"
  [[ "$got_kept" == "$1" && "$got_suspended" == "$2" && "$got_lost" == "$3" ]] || {
    printf 'expected kept/suspended/lost: %s/%s/%s; got %s/%s/%s\n' \
      "$1" "$2" "$3" "$got_kept" "$got_suspended" "$got_lost" >&2
    return 1
  }
}

codex_timing_init
# First attempt: 10s active + 4s paused + 4s active, then discarded.
codex_compiler_pid=101
clock=2000000; codex_timing_launch
clock=12000000; codex_timing_pause
clock=16000000; codex_timing_resume
clock=20000000; codex_timing_lost
codex_compiler_pid=""
assert_ledger 0 4000000 14000000

# Second attempt: 10s active + 3s paused + 4s active, then retained.
codex_compiler_pid=102
clock=21000000; codex_timing_launch
clock=31000000; codex_timing_pause
clock=34000000; codex_timing_resume
clock=38000000; codex_timing_success
codex_compiler_pid=""
assert_ledger 14000000 7000000 14000000

# Waiting for memory is suspended, not useful or lost.
clock=39000000; codex_timing_pause
clock=45000000; codex_timing_resume
assert_ledger 14000000 13000000 14000000

# Reports do not mutate past classifications or decrease counters.
clock=46000000
report="$(codex_report_timing_counters 2>&1)"
[[ "$report" == *"Rust time: kept 00:00:14; suspended 00:00:13; lost 00:00:14"* ]] || {
  echo "unexpected timing report: $report" >&2
  exit 1
}
[[ "$report" == *"🧠 Ballast: 96/256 MiB held"* ]] || {
  echo "ballast must accompany timing report: $report" >&2
  exit 1
}
assert_ledger 14000000 13000000 14000000

# A second wrapper sharing CODEX_BUILD_PID adds to the same ledger.
(
  clock=50000000; codex_timing_init
  codex_compiler_pid=103
  clock=51000000; codex_timing_launch
  clock=56000000; codex_timing_success
)
assert_ledger 19000000 13000000 14000000

# If a running attempt is suspended then killed, only active time is lost.
clock=60000000; codex_timing_init
codex_compiler_pid=104
clock=61000000; codex_timing_launch
clock=67000000; codex_timing_pause
clock=71000000; codex_timing_lost
codex_compiler_pid=""
assert_ledger 19000000 17000000 20000000

echo 'rustc timing: disjoint kept/suspended/lost counters and shared ledger OK'
