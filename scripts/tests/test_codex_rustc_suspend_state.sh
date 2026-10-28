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

resumes=0
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
  grep -q 'Rust resumed at ' "${temp}/out"
done
[[ "$resumes" == 2 ]]
echo "rustc suspension state: two pause/resume cycles re-arm the PSI guard OK"
