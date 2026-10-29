#!/usr/bin/env bash
# Deterministic tests for shared Rust timing, no Android required.
set -euo pipefail
test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
temp="$(mktemp -d)"
trap 'rm -rf -- "$temp"' EXIT
export TMPDIR="$temp" CODEX_ANDROID_RUSTC_TIMING_DIR="$temp"
export CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR="$temp/global"
source "${test_dir}/../codex_rustc_timing.sh"

# Missing stage environment no longer causes isolated per-rustc counters:
# all wrappers join the project-wide ledger, regardless of Cargo process.
(
  unset CODEX_ANDROID_RUSTC_TIMING_DIR
  codex_timing_init 2>"$temp/diag"
  [[ ! -s "$temp/diag" ]]
  [[ "$codex_timing_file" == "$temp/global/totals" ]]
  [[ -z "$codex_timing_stage_file" ]]
)
(
  export CODEX_ANDROID_RUSTC_TIMING_DIR="$temp/does-not-exist"
  codex_timing_init 2>"$temp/diag"
  IFS= read -r output <"$temp/diag"
  [[ "$output" == *'stage Rust timing directory missing'* ]]
  [[ "$codex_timing_file" == "$temp/global/totals" ]]
  [[ -z "$codex_timing_stage_file" ]]
)
(
  export PATH="/a-nonexistent-path"
  codex_timing_init 2>"$temp/diag"
  IFS= read -r output <"$temp/diag"
  [[ "$output" == *'flock not in compiler PATH:'* ]]
  [[ -z "$codex_timing_file" ]]
)
(
  codex_timing_init 2>"$temp/diag"
  [[ ! -s "$temp/diag" ]]
  [[ "$codex_timing_file" == "$temp/global/totals" ]]
  [[ "$codex_timing_stage_file" == "$temp/totals" ]]
)

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

# An independent Cargo invocation (no managed timing directory) must keep
# adding to the exact same displayed aggregate, not restart from zero.
(
  unset CODEX_ANDROID_RUSTC_TIMING_DIR
  clock=57000000; codex_timing_init
  [[ "$codex_timing_file" == "$temp/global/totals" ]]
  [[ -z "$codex_timing_stage_file" ]]
  codex_compiler_pid=105
  clock=58000000; codex_timing_launch
  clock=60000000; codex_timing_success
  report="$(codex_report_timing_counters 2>&1)"
  [[ "$report" == *"Rust time: kept 00:00:21; suspended 00:00:13; lost 00:00:14"* ]]
)
assert_ledger 21000000 13000000 14000000
read -r stage_kept stage_suspended stage_lost <"$temp/totals"
[[ "$stage_kept" == 19000000 && "$stage_suspended" == 13000000 && "$stage_lost" == 14000000 ]]

# Competing auxiliary rustc processes must serialize global increments.
parallel_pids=()
for worker in 1 2 3 4; do
  (
    unset CODEX_ANDROID_RUSTC_TIMING_DIR
    clock=80000000; codex_timing_init
    codex_compiler_pid="$worker"
    clock=81000000; codex_timing_launch
    clock=83000000; codex_timing_success
  ) &
  parallel_pids+=("$!")
done
for pid in "${parallel_pids[@]}"; do
  wait "$pid"
done
assert_ledger 29000000 13000000 14000000
read -r stage_kept stage_suspended stage_lost <"$temp/totals"
[[ "$stage_kept" == 19000000 && "$stage_suspended" == 13000000 && "$stage_lost" == 14000000 ]]

# A fresh managed Cargo stage has a new stage ledger, but the visible
# project-wide Kept count still starts from all previously completed work.
mkdir -p "$temp/second-stage"
(
  export CODEX_ANDROID_RUSTC_TIMING_DIR="$temp/second-stage"
  clock=90000000; codex_timing_init
  [[ "$codex_timing_stage_file" == "$temp/second-stage/totals" ]]
  codex_compiler_pid=106
  clock=91000000; codex_timing_launch
  clock=93000000; codex_timing_success
)
assert_ledger 31000000 13000000 14000000
read -r fresh_kept fresh_suspended fresh_lost <"$temp/second-stage/totals"
[[ "$fresh_kept" == 2000000 && "$fresh_suspended" == 0 && "$fresh_lost" == 0 ]]

# If a running attempt is suspended then killed, only active time is lost.
clock=60000000; codex_timing_init
codex_compiler_pid=104
clock=61000000; codex_timing_launch
clock=67000000; codex_timing_pause
clock=71000000; codex_timing_lost
codex_compiler_pid=""
assert_ledger 31000000 17000000 20000000
read -r stage_kept stage_suspended stage_lost <"$temp/totals"
[[ "$stage_kept" == 19000000 && "$stage_suspended" == 17000000 && "$stage_lost" == 20000000 ]]

echo 'rustc timing: project-wide monotonic totals, isolated stage deltas, and disjoint counters OK'
