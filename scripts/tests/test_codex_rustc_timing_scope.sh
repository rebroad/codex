#!/usr/bin/env bash
# One Rust guard/timing initialization for just test and build_codex.sh.
# No Android required: use a fake Rust wrapper with the actual setup helper.
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
scripts_dir="${test_dir}/.."
build_script="${scripts_dir}/build_codex.sh"
shared_cargo_env="${scripts_dir}/codex_cargo_env.sh"
justfile="${scripts_dir}/../justfile"
temp="$(mktemp -d)"
trap 'rm -rf -- "$temp"' EXIT

# Both entry points call the exact same Cargo --emit setup; it in turn loads
# the same reusable Rust guard helper (without changing the user's justfile).
grep -Fq 'eval "${cargo_env_output}"' "$build_script"
grep -Fq 'eval \"$codex_cargo_env_output\"' "$justfile"
grep -Fq 'codex_rust_guard_setup' "$shared_cargo_env"
grep -Fq 'codex_rust_guard_start_stage' "$build_script"
if grep -Fq 'codex_scope_managed_rustc_wrapper' "$build_script"; then
  echo "build_codex.sh must not privately override the shared RUSTC_WRAPPER" >&2
  exit 1
fi

source "${scripts_dir}/codex_rust_guard_env.sh"
mkdir -p "$temp/source" "$temp/stages"
printf '#!/bin/sh\nexit 0\n' >"$temp/rustc-wrapper"
chmod +x "$temp/rustc-wrapper"
export CODEX_ANDROID_RUSTC_MEMORY_GUARD=1 RUSTC_WRAPPER="$temp/rustc-wrapper"

codex_rust_guard_setup "$temp/source"
global="${CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR}"
[[ "$global" == "$temp/source/build/codex-rust-timing-runs/run."* ]]
[[ "${CODEX_ANDROID_RUSTC_TIMING_RUN_OWNER:-}" == "$" ]]
[[ -d "$global" && -w "$global" ]]
[[ "$(env | grep '^CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR=')" == "CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR=$global" ]]
[[ "$(env | grep '^RUSTC_WRAPPER=')" == "RUSTC_WRAPPER=$temp/rustc-wrapper" ]]

# Another setup call in this same top-level shell must preserve all progress.
printf '5000000 0 0\n' >"$global/totals"
codex_rust_guard_setup "$temp/source"
[[ "$CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR" == "$global" ]]
[[ "$(cat "$global/totals")" == '5000000 0 0' ]]

# A NEW top-level Bash command inherits the old environment but must start
# from zero in a DIFFERENT directory. Do not truncate the old run's ledger:
# it may still have live compiler processes updating it.
next_run="$(bash -c '
  source "$1"
  codex_rust_guard_setup "$2"
  printf "%s" "$CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR"
' bash "$scripts_dir/codex_rust_guard_env.sh" "$temp/source")"
[[ "$next_run" != "$global" && -d "$next_run" && ! -e "$next_run/totals" ]]
[[ "$(cat "$global/totals")" == '5000000 0 0' ]]

# just test: no temporary stage, but wrapper and global ledger are present.
(
  unset CODEX_ANDROID_RUSTC_TIMING_DIR
  bash -c '[[ -f "$RUSTC_WRAPPER" && -d "$CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR" ]]'
  source "${scripts_dir}/codex_rustc_timing.sh"
  codex_timing_init
  [[ "$codex_timing_file" == "$global/totals" ]]
  [[ -z "$codex_timing_stage_file" ]]
)

# build_codex.sh: same wrapper and global ledger; only the stage is extra.
codex_rust_guard_start_stage "$temp/stages"
[[ -d "$CODEX_ANDROID_RUSTC_TIMING_DIR" ]]
(
  bash -c '[[ -f "$RUSTC_WRAPPER" && -d "$CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR" && -d "$CODEX_ANDROID_RUSTC_TIMING_DIR" ]]'
  source "${scripts_dir}/codex_rustc_timing.sh"
  codex_timing_init
  [[ "$codex_timing_file" == "$global/totals" ]]
  [[ "$codex_timing_stage_file" == "$CODEX_ANDROID_RUSTC_TIMING_DIR/totals" ]]
)

# A nonexistent wrapper must be rejected before a long Cargo operation.
(
  export RUSTC_WRAPPER="$temp/nonexistent-wrapper"
  if codex_rust_guard_setup "$temp/source" 2>/dev/null; then
    echo "failed to reject invalid guarded compiler wrapper" >&2
    exit 1
  fi
)

# Guard disabled: helper has no effects on non-Android builds.
(
  export CODEX_ANDROID_RUSTC_MEMORY_GUARD=0
  unset CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR
  codex_rust_guard_setup "$temp/source"
  [[ -z "${CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR:-}" ]]
)

echo 'rustc timing environment: just and build use one shared setup and global ledger OK'
