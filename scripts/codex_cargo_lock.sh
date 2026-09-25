#!/usr/bin/env bash

codex_cargo_lock_backup=""

codex_cargo_lock_backup_source() {
  [[ -n "${CODEX_CARGO_SOURCE_LOCK:-}" ]] || return 0
  [[ -f "${CODEX_CARGO_SOURCE_LOCK}" ]] || {
    printf 'source Cargo.lock not found: %s\n' "${CODEX_CARGO_SOURCE_LOCK}" >&2
    return 1
  }

  local temp_dir
  if [[ -n "${PREFIX:-}" && "${PREFIX}" == /data/data/*/files/usr ]]; then
    temp_dir="${PREFIX}/tmp"
  else
    temp_dir=/var/tmp
  fi
  mkdir -p "${temp_dir}"
  codex_cargo_lock_backup="$(mktemp "${temp_dir%/}/codex-Cargo.lock.XXXXXX")" || return $?
  if ! cp -p "${CODEX_CARGO_SOURCE_LOCK}" "${codex_cargo_lock_backup}"; then
    rm -f "${codex_cargo_lock_backup}"
    codex_cargo_lock_backup=""
    return 1
  fi
}

codex_cargo_lock_restore_source() {
  [[ -n "${codex_cargo_lock_backup}" ]] || return 0

  local original_fingerprint current_fingerprint
  if [[ ! -f "${CODEX_CARGO_SOURCE_LOCK}" ]]; then
    if ! cp -p "${codex_cargo_lock_backup}" "${CODEX_CARGO_SOURCE_LOCK}"; then
      printf 'Unable to restore source Cargo.lock from %s\n' "${codex_cargo_lock_backup}" >&2
      return 1
    fi
    rm -f "${codex_cargo_lock_backup}"
    codex_cargo_lock_backup=""
    return
  fi

  original_fingerprint="$(python3 "${CODEX_CARGO_LOCK_NORMALIZER}" \
    --manifest "${CODEX_CARGO_LOCK_MANIFEST}" \
    --source-lock "${codex_cargo_lock_backup}" --fingerprint)" || {
      printf '%s\n' 'Unable to verify Cargo.lock normalization; leaving the lockfile unchanged.' >&2
      rm -f "${codex_cargo_lock_backup}"
      codex_cargo_lock_backup=""
      return 0
    }
  current_fingerprint="$(python3 "${CODEX_CARGO_LOCK_NORMALIZER}" \
    --manifest "${CODEX_CARGO_LOCK_MANIFEST}" \
    --source-lock "${CODEX_CARGO_SOURCE_LOCK}" --fingerprint)" || {
      printf '%s\n' 'Unable to verify Cargo.lock normalization; leaving the lockfile unchanged.' >&2
      rm -f "${codex_cargo_lock_backup}"
      codex_cargo_lock_backup=""
      return 0
    }

  if [[ "${original_fingerprint}" == "${current_fingerprint}" ]]; then
    if ! cmp -s "${codex_cargo_lock_backup}" "${CODEX_CARGO_SOURCE_LOCK}"; then
      cp -p "${codex_cargo_lock_backup}" "${CODEX_CARGO_SOURCE_LOCK}" || return $?
    fi
  else
    printf '%s\n' 'Cargo.lock contains dependency changes; keeping the updated lockfile.' >&2
  fi
  rm -f "${codex_cargo_lock_backup}"
  codex_cargo_lock_backup=""
}
