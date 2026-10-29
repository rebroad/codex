#!/usr/bin/env bash
# Managed Cargo owns a shared timing ledger. Preparatory/nested Cargo commands
# must not inherit the guarded wrapper without that directory.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
build_script="${script_dir}/../build_codex.sh"
temp="$(mktemp -d)"
trap 'rm -rf -- "$temp"' EXIT

# Exercise the production scoping helper without executing the full build.
awk '
  /^codex_scope_managed_rustc_wrapper\(\) \{/ { in_function=1 }
  in_function {
    print
    if ($0 == "}") {
      complete=1
      exit
    }
  }
  END { if (!complete) exit 1 }
' "$build_script" >"${temp}/scope.sh"
source "${temp}/scope.sh"

export CODEX_ANDROID_RUSTC_MEMORY_GUARD=1
export RUSTC_WRAPPER="/tmp/test-managed-rustc-wrapper"
codex_scope_managed_rustc_wrapper
[[ "$CODEX_MANAGED_RUSTC_WRAPPER" == "/tmp/test-managed-rustc-wrapper" ]]
[[ "$RUSTC_WRAPPER" == "/tmp/test-managed-rustc-wrapper" ]]

# The external Cargo environment no longer receives an orphaned wrapper.
if env | grep -q '^RUSTC_WRAPPER='; then
  echo "RUSTC_WRAPPER leaked into a preparatory Cargo invocation" >&2
  exit 1
fi

# The main Cargo invocation explicitly receives the wrapper with the ledger.
env RUSTC_WRAPPER="$CODEX_MANAGED_RUSTC_WRAPPER" \
  CODEX_ANDROID_RUSTC_TIMING_DIR="$temp" \
  bash -c '[[ "$RUSTC_WRAPPER" == /tmp/test-managed-rustc-wrapper ]] && [[ -d "$CODEX_ANDROID_RUSTC_TIMING_DIR" ]] && command -v flock >/dev/null'

# No guard: leave the ordinary environment helper's wrapper untouched.
export CODEX_ANDROID_RUSTC_MEMORY_GUARD=0
export RUSTC_WRAPPER="/tmp/test-standard-rustc-wrapper"
codex_scope_managed_rustc_wrapper
[[ -z "$CODEX_MANAGED_RUSTC_WRAPPER" ]]
[[ "$(env | grep '^RUSTC_WRAPPER=')" == 'RUSTC_WRAPPER=/tmp/test-standard-rustc-wrapper' ]]

# Verify that build_codex.sh actually passes the scoped wrapper into its
# managed Cargo environment, and checks that both variables reach the stage.
grep -Fq 'env_args+=(RUSTC_WRAPPER="${CODEX_MANAGED_RUSTC_WRAPPER}")' "$build_script"
grep -Fq 'ballast_env_args+=(CODEX_ANDROID_RUSTC_TIMING_DIR="${ANDROID_RUST_TIMING_DIR}")' "$build_script"
echo 'rustc stage scope: wrapper and shared ledger travel together'
