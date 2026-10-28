#!/usr/bin/env bash
# The rustc wrapper must be able to suspend the SAME compiler again after
# resuming it. This test does not require Android or launch a compiler.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
wrapper="${script_dir}/../codex_rustc_oom_wrapper.sh"
temp="$(mktemp -d)"
trap 'rm -rf -- "$temp"' EXIT

# Isolate the production state-transition function; sourcing the entire
# wrapper would start its Android PSI guard and try to launch rustc.
awk '
  /^codex_guard_mark_running\(\) \{/ { in_function = 1 }
  in_function {
    print
    if ($0 == "}") {
      complete = 1
      exit
    }
  }
  END { if (!complete) exit 1 }
' "$wrapper" >"${temp}/resume.sh"
source "${temp}/resume.sh"

# Exercise the production policy helpers, rather than a duplicate model.
for policy_function in codex_guard_is_psi_only codex_psi_pause_limit_reached; do
  awk -v function_name="$policy_function" '
    $0 == function_name "() {" { in_function = 1 }
    in_function {
      print
      if ($0 == "}") {
        complete = 1
        exit
      }
    }
    END { if (!complete) exit 1 }
  ' "$wrapper" >>"${temp}/psi_policy.sh"
done
source "${temp}/psi_policy.sh"

codex_guard_reason="memory PSI some stall 40000us per second >= 35ms"
codex_guard_is_psi_only
codex_guard_reason="memory PSI full stall 105939us per second >= 60ms"
codex_guard_is_psi_only
for codex_guard_reason in "MemAvailable 200000kB <= 5% critical RAM floor" \
    "combined RAM and swap headroom 200000kB <= 50% of configured swap" \
    "LMKD reclaimed another app"; do
  if codex_guard_is_psi_only; then
    echo "hard memory condition incorrectly classified as PSI-only: $codex_guard_reason" >&2
    exit 1
  fi
done

codex_psi_max_pause_seconds=5
codex_compiler_suspended_at_epoch_us=1000000
if codex_psi_pause_limit_reached 5999999; then
  echo 'PSI pause returned to rustc before its five-second limit' >&2
  exit 1
fi
codex_psi_pause_limit_reached 6000000
codex_compiler_suspended_at_epoch_us=""
if codex_psi_pause_limit_reached 7000000; then
  echo 'empty pause timestamp counted as expired' >&2
  exit 1
fi

resumes=0
codex_psi_resume_grace_seconds=30
codex_timing_resume() { resumes=$((resumes + 1)); }

for cycle in 1 2; do
  codex_guard_compile_state=paused
  codex_guard_pause_mode=suspended
  codex_compiler_suspended=true
  codex_compiler_suspended_at="old-pause"
  codex_compiler_suspended_at_epoch_us="${EPOCHREALTIME/./}"
  codex_guard_pause_started_epoch_us="${EPOCHREALTIME/./}"
  codex_guard_mark_running 2>"${temp}/out"

  [[ "$codex_guard_compile_state" == running ]]
  [[ "$codex_guard_pause_mode" == running ]]
  [[ "$codex_compiler_suspended" == false ]] || {
    echo "cycle $cycle: the compiler cannot suspend again (stale flag)" >&2
    exit 1
  }
  [[ -z "$codex_compiler_suspended_at" ]]
  [[ -z "$codex_compiler_suspended_at_epoch_us" ]]
  [[ "${codex_psi_resume_grace_until_epoch_us:-0}" -gt "${EPOCHREALTIME/./}" ]]
  grep -q 'Rust resumed at ' "${temp}/out"
done
[[ "$resumes" == 2 ]]
echo "rustc suspension state: repeated resume and bounded PSI pause policy OK"
