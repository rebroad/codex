#!/usr/bin/env bash
# Another Android app's LMKD reclaim must be advisory, not a reason to throw
# away a Rust compiler attempt. Exercise real wrapper parsing and RAM checks.
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
wrapper="${test_dir}/../codex_rustc_oom_wrapper.sh"
temp="$(mktemp -d)"
trap 'rm -rf -- "$temp"' EXIT

for function_name in codex_read_psi codex_memory_guard_reason; do
  awk -v fn="$function_name" '
    $0 == fn "() {" { in_function=1 }
    in_function {
      print
      if ($0 == "}") {
        complete=1
        exit
      }
    }
    END { if (!complete) exit 1 }
  ' "$wrapper" >>"${temp}/guard-functions.sh"
done
source "${temp}/guard-functions.sh"

codex_uint() { [[ "$1" =~ ^[0-9]+$ ]]; }
codex_ballast_relief_count=0
codex_ballast_request_relief() {
  codex_ballast_relief_count=$((codex_ballast_relief_count+1))
}
codex_psi_available=false
codex_mem_total_kb=10000000
codex_mem_available_kb=6000000
codex_swap_total_kb=8000000
codex_swap_free_kb=7000000
codex_memavailable_critical_percent=5
codex_swap_headroom_percent=50
codex_guard_reason=""

cat >"${temp}/psi" <<'EOF'
L 1761722218.001 Reclaim 'com.samsung.mfcontents' while freeing background memory
L 1761722218.050 Reclaim 'com.google.android.tts' while freeing background memory
P 100 200
EOF
exec {psi_fd}<"${temp}/psi"
codex_psi_stream_fd="$psi_fd"
codex_read_psi 2>"${temp}/notices"
exec {psi_fd}<&-
[[ "$codex_psi_some_total_us" == 100 ]]
[[ "$codex_psi_full_total_us" == 200 ]]
[[ "$codex_ballast_relief_count" == 2 ]] || {
  echo "expected ballast release for both LMKD events" >&2
  exit 1
}
grep -q 'LMKD reclaimed com.samsung.mfcontents' "${temp}/notices"
grep -q 'LMKD reclaimed com.google.android.tts' "${temp}/notices"
grep -q 'advisory only' "${temp}/notices"

# Neither unrelated LMKD event changes the hard memory decision.
if codex_memory_guard_reason; then
  echo "unrelated LMKD reclaim incorrectly classified as memory emergency" >&2
  exit 1
fi
# The actual critical headroom guard still takes precedence.
codex_mem_available_kb=200000
codex_memory_guard_reason
[[ "$codex_guard_reason" == MemAvailable* ]] || {
  echo "hard RAM memory guard failed to detect critical low headroom" >&2
  exit 1
}

# In particular, no stale sticky LMKD state can re-trigger on each PSI read.
if grep -q 'codex_lmkd_reclaim_active' "$wrapper"; then
  echo "sticky LMKD hard-stop state still present in wrapper" >&2
  exit 1
fi
echo 'Rust LMKD advisory: both unrelated reclaims preserved rustc; critical RAM guard intact'
