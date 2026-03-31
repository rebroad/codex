#!/usr/bin/env bash
set -u -o pipefail

if ! git rev-parse --git-dir >/dev/null 2>&1; then
  echo "error: must be run from inside a git repository" >&2
  exit 2
fi

bisect_start_file="$(git rev-parse --git-path BISECT_START 2>/dev/null || true)"
if [ -z "${bisect_start_file}" ] || [ ! -f "${bisect_start_file}" ]; then
  echo "error: git bisect does not appear to be active" >&2
  echo "start bisect first (e.g. git bisect start ...), then rerun this script" >&2
  exit 2
fi

if [ -f Cargo.toml ]; then
  cargo_test_cmd=(cargo test)
  lockfile_path="Cargo.lock"
elif [ -f codex-rs/Cargo.toml ]; then
  cargo_test_cmd=(cargo test --manifest-path codex-rs/Cargo.toml)
  lockfile_path="codex-rs/Cargo.lock"
else
  echo "error: could not find Cargo.toml in current directory or ./codex-rs/" >&2
  exit 2
fi

cargo_args=()
exact_tests=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --exact-test)
      shift
      if [ "$#" -eq 0 ]; then
        echo "error: --exact-test requires a test name" >&2
        exit 2
      fi
      exact_tests+=("$1")
      ;;
    *)
      cargo_args+=("$1")
      ;;
  esac
  shift
done

maybe_restore_lockfile() {
  if [ -n "${lockfile_path:-}" ] && [ -f "${lockfile_path}" ]; then
    if ! git diff --quiet -- "${lockfile_path}"; then
      echo "restoring modified lockfile: ${lockfile_path}"
      git restore --worktree -- "${lockfile_path}"
    fi
  fi
}

run_once() {
  if [ "${#exact_tests[@]}" -eq 0 ]; then
    echo "running: ${cargo_test_cmd[*]} ${cargo_args[*]}"
    "${cargo_test_cmd[@]}" "${cargo_args[@]}"
    return $?
  fi

  for test_name in "${exact_tests[@]}"; do
    echo "checking test exists: ${test_name}"
    # Check test existence first so old commits that predate a test are skipped.
    list_output_file="$(mktemp)"
    "${cargo_test_cmd[@]}" "${cargo_args[@]}" -- --exact "$test_name" --list 2>&1 | tee "$list_output_file"
    list_status=${PIPESTATUS[0]}
    if [ "$list_status" -ne 0 ]; then
      rm -f "$list_output_file"
      return "$list_status"
    fi
    if ! grep -Fq -- "$test_name" "$list_output_file"; then
      rm -f "$list_output_file"
      echo "test '${test_name}' was not found on this commit"
      return 125
    fi
    rm -f "$list_output_file"

    echo "running exact test: ${test_name}"
    "${cargo_test_cmd[@]}" "${cargo_args[@]}" -- --exact "$test_name"
    status=$?
    if [ "$status" -ne 0 ]; then
      return "$status"
    fi
  done

  return 0
}

apply_bisect_mark() {
  local test_status="$1"
  local bisect_cmd
  local message

  if [ "$test_status" -eq 0 ]; then
    bisect_cmd="good"
    message="cargo test passed -> git bisect good"
  elif [ "$test_status" -eq 125 ]; then
    bisect_cmd="skip"
    message="tests missing on this commit -> git bisect skip"
  else
    bisect_cmd="bad"
    message="cargo test failed with exit ${test_status} -> git bisect bad"
  fi

  maybe_restore_lockfile
  echo "$message"
  bisect_output="$(git bisect "$bisect_cmd" 2>&1)"
  bisect_status=$?
  printf '%s\n' "$bisect_output"
  if [ "$bisect_status" -ne 0 ]; then
    return "$bisect_status"
  fi

  if printf '%s' "$bisect_output" | grep -Eiq \
    "is the first bad commit|first bad commit could be any of|only skipped commits left to test"; then
    return 200
  fi

  return 0
}

while [ -f "${bisect_start_file}" ]; do
  run_once
  test_status=$?
  apply_bisect_mark "$test_status"
  bisect_apply_status=$?
  if [ "$bisect_apply_status" -eq 200 ]; then
    exit 0
  fi
  if [ "$bisect_apply_status" -ne 0 ]; then
    exit "$bisect_apply_status"
  fi
done

echo "git bisect is no longer active"
