#!/usr/bin/env bash
# Common Android Rust guard/timing environment for both build_codex.sh and
# just/cargo recipes. codex_cargo_env.sh sources this in its --emit output.
# Only the managed build needs the optional per-Cargo-stage ledger.

codex_rust_guard_setup() {
  [[ "${CODEX_ANDROID_RUSTC_MEMORY_GUARD:-0}" == 1 ]] || return 0
  local source_repo="${1:?source repository required}"

  if ! command -v flock >/dev/null 2>&1; then
    echo 'Android Rust guard: flock is required for shared compiler timing' >&2
    return 1
  fi
  if [[ -z "${RUSTC_WRAPPER:-}" || ! -f "$RUSTC_WRAPPER" ]]; then
    printf 'Android Rust guard: missing RUSTC_WRAPPER: %s\n' "${RUSTC_WRAPPER:-unset}" >&2
    return 1
  fi

  # One *new* ledger per top-level Cargo shell: just test and build_codex.sh
  # both enter here through codex_cargo_env.sh. A second Cargo stage from the
  # same shell reuses its ledger; an independent invocation gets fresh totals.
  # Unique directories avoid destroying the totals of a concurrent run.
  # Bash $$ is stable across command substitutions (cargo_build uses these)
  # and distinct for new top-level Bash invocations.
  if [[ "${CODEX_ANDROID_RUSTC_TIMING_RUN_OWNER:-}" != "$$" \
    || -z "${CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR:-}" \
    || ! -d "$CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR" ]]; then
    local run_root="${source_repo}/build/codex-rust-timing-runs"
    mkdir -p -- "$run_root" || return 1
    CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR="$(mktemp -d "${run_root%/}/run.XXXXXX")" || return 1
    CODEX_ANDROID_RUSTC_TIMING_RUN_OWNER="$$"
    export CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR CODEX_ANDROID_RUSTC_TIMING_RUN_OWNER
  fi
  if [[ ! -w "$CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR" ]]; then
    printf 'Android Rust guard: cannot use run timing directory: %s\n' \
      "$CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR" >&2
    return 1
  fi
}

codex_rust_guard_start_stage() {
  [[ "${CODEX_ANDROID_RUSTC_MEMORY_GUARD:-0}" == 1 ]] || return 0
  local temp_root="${1:-${TMPDIR:-${PREFIX:-/data/data/com.termux/files/usr}/tmp}}"
  mkdir -p -- "$temp_root" || return 1
  CODEX_ANDROID_RUSTC_TIMING_DIR="$(mktemp -d "${temp_root%/}/codex-rust-timing.XXXXXX")" || return 1
  export CODEX_ANDROID_RUSTC_TIMING_DIR
}
