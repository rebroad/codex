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

  # One persistent ledger for every compiler, regardless of whether it was
  # launched by just test or build_codex.sh. Both use the same source checkout.
  CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR="${CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR:-${source_repo}/build/codex-rust-timing-global}"
  if ! mkdir -p -- "$CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR" \
    || [[ ! -w "$CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR" ]]; then
    printf 'Android Rust guard: cannot use project-wide timing directory: %s\n' \
      "$CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR" >&2
    return 1
  fi
  export CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR
}

codex_rust_guard_start_stage() {
  [[ "${CODEX_ANDROID_RUSTC_MEMORY_GUARD:-0}" == 1 ]] || return 0
  local temp_root="${1:-${TMPDIR:-${PREFIX:-/data/data/com.termux/files/usr}/tmp}}"
  mkdir -p -- "$temp_root" || return 1
  CODEX_ANDROID_RUSTC_TIMING_DIR="$(mktemp -d "${temp_root%/}/codex-rust-timing.XXXXXX")" || return 1
  export CODEX_ANDROID_RUSTC_TIMING_DIR
}
