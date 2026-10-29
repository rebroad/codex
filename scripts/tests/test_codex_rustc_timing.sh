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
# Extract crate identity from real Cargo/rustc flags, not interleaved Cargo
# "Compiling" messages. Generic test crate names get their input path.
codex_timing_identify_compiler /usr/bin/rustc --crate-name=all --test app-server/tests/all.rs
[[ "$codex_timing_crate" == all ]]
[[ "$codex_timing_source" == 'app-server/tests/all.rs' ]]
codex_compiler_pid=515
codex_timing_emit_start 2>"$temp/started"
grep -Eq '^[0-9]{2}:[0-9]{2}:[0-9]{2} ▶ start crate=all pid=515 src=app-server/tests/all.rs$' "$temp/started"
codex_timing_identify_compiler /usr/bin/rustc --crate-name codex_core core/src/lib.rs
[[ "$codex_timing_crate" == codex_core ]]
[[ -z "$codex_timing_source" ]]

# First attempt: 10s active + 4s paused + 4s active, then discarded.
codex_compiler_pid=101
clock=2000000; codex_timing_launch
clock=12000000; codex_timing_pause
clock=16000000; codex_timing_resume
clock=20000000; codex_timing_lost 2>"$temp/lost-transition"
codex_compiler_pid=""
grep -Eq '^[0-9]{2}:[0-9]{2}:[0-9]{2} ✗ lost\+14s crate=codex_core pid=101$' "$temp/lost-transition"
assert_ledger 0 4000000 14000000

# Second attempt: 10s active + 3s paused + 4s active, then retained.
codex_compiler_pid=102
codex_timing_identify_compiler /usr/bin/rustc --crate-name codex_app_server app-server/src/lib.rs
clock=21000000; codex_timing_launch
clock=31000000; codex_timing_pause
clock=34000000; codex_timing_resume
clock=38000000; codex_timing_success 2>"$temp/kept-transition"
codex_compiler_pid=""
grep -Eq '^[0-9]{2}:[0-9]{2}:[0-9]{2} ✓ kept\+14s crate=codex_app_server pid=102$' "$temp/kept-transition"
assert_ledger 14000000 7000000 14000000

# Waiting for memory is suspended, not useful or lost.
clock=39000000; codex_timing_pause
clock=45000000; codex_timing_resume
assert_ledger 14000000 13000000 14000000

# Formatting is compact and omits zero-valued trailing units.
codex_timing_format 7590000000 time_2h6m30s
[[ "$time_2h6m30s" == '2h6m30s' ]]
codex_timing_format 7560000000 time_2h6m
[[ "$time_2h6m" == '2h6m' ]]
codex_timing_format 232000000 time_3m52s
[[ "$time_3m52s" == '3m52s' ]]
codex_timing_format 0 time_zero
[[ "$time_zero" == '0s' ]]

# Reports do not mutate past classifications or decrease counters.
clock=46000000
report="$(codex_report_timing_counters 2>&1)"
[[ "$report" =~ [0-9]{2}:[0-9]{2}:[0-9]{2}\ kept=14s\ slept=13s\ lost=14s\ this=0s ]] || {
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
  [[ "$report" =~ [0-9]{2}:[0-9]{2}:[0-9]{2}\ kept=21s\ slept=13s\ lost=14s\ this=0s ]]
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
