#!/usr/bin/env bash
set -u -o pipefail

if ! git rev-parse --git-dir >/dev/null 2>&1; then
  echo "error: must be run from inside a git repository" >&2
  exit 2
fi

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"
results_dir="${repo_root}/.bisect-test-results"
mkdir -p "${results_dir}"
summary_file="${results_dir}/summary.tsv"
if [ ! -f "${summary_file}" ]; then
  printf "step\tcommit\ttest_exit\tbisect_decision\tlog_file\n" >"${summary_file}"
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

LAST_AUTOSTASH_REF=""
autostash_before_bisect_transition() {
  local log_file="$1"
  LAST_AUTOSTASH_REF=""
  if [ -n "$(git status --porcelain --untracked-files=normal)" ]; then
    local stash_message="test-bisect-autostash-step-${step}-$(date -u +'%Y%m%dT%H%M%SZ')"
    echo "working tree dirty; creating transient stash before bisect transition: ${stash_message}" | tee -a "$log_file"
    git stash push --include-untracked -m "$stash_message" >/dev/null
    LAST_AUTOSTASH_REF="$(git stash list -n1 --pretty=%gd 2>/dev/null || true)"
    if [ -n "${LAST_AUTOSTASH_REF}" ]; then
      echo "created transient stash (kept): ${LAST_AUTOSTASH_REF}" | tee -a "$log_file"
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
  local log_file="$2"
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

  autostash_before_bisect_transition "$log_file"
  echo "$message" | tee -a "$log_file"
  last_bisect_decision="$bisect_cmd"
  bisect_output="$(git bisect "$bisect_cmd" 2>&1)"
  last_bisect_output="$bisect_output"
  bisect_status=$?
  printf '%s\n' "$bisect_output" | tee -a "$log_file"
  if [ "$bisect_status" -ne 0 ]; then
    return "$bisect_status"
  fi

  if printf '%s' "$bisect_output" | grep -Eiq \
    "is the first bad commit|first bad commit could be any of|only skipped commits left to test"; then
    return 200
  fi

  return 0
}

step=0
while [ -f "${bisect_start_file}" ]; do
  step=$((step + 1))
  commit_hash="$(git rev-parse --verify HEAD)"
  echo "=== bisect step ${step} @ ${commit_hash} ==="
  log_file="${results_dir}/$(printf '%04d' "$step")_${commit_hash}.log"
  {
    echo "step: ${step}"
    echo "commit: ${commit_hash}"
    echo "time_utc: $(date -u +'%Y-%m-%dT%H:%M:%SZ')"
    echo "cargo_cmd: ${cargo_test_cmd[*]} ${cargo_args[*]}"
    echo
  } >"$log_file"

  run_once 2>&1 | tee -a "$log_file"
  test_status=${PIPESTATUS[0]}
  apply_bisect_mark "$test_status" "$log_file"
  bisect_apply_status=$?
  next_commit_hash="$(git rev-parse --verify HEAD)"
  printf "%s\t%s\t%s\t%s\t%s\n" \
    "$step" "$commit_hash" "$test_status" "${last_bisect_decision}" "$log_file" >>"$summary_file"
  if [ "$bisect_apply_status" -eq 200 ]; then
    echo "bisect complete. summary: ${summary_file}"
    if bad_hash="$(printf '%s\n' "$last_bisect_output" | awk '/is the first bad commit/{print $1; exit}')"; then
      if [ -n "${bad_hash:-}" ]; then
        echo "FIRST_BAD_COMMIT=${bad_hash}"
      fi
      if [ -n "${bad_hash:-}" ] && [ -f "${results_dir}/$(printf '%04d' "$step")_${bad_hash}.log" ]; then
        echo "first bad commit log: ${results_dir}/$(printf '%04d' "$step")_${bad_hash}.log"
      else
        bad_log="$(ls "${results_dir}"/*_"${bad_hash}".log 2>/dev/null | tail -n1 || true)"
        if [ -n "$bad_log" ]; then
          echo "first bad commit log: ${bad_log}"
        else
          final_log="${results_dir}/final_${bad_hash}.log"
          {
            echo "final_commit: ${bad_hash}"
            echo "time_utc: $(date -u +'%Y-%m-%dT%H:%M:%SZ')"
            echo "cargo_cmd: ${cargo_test_cmd[*]} ${cargo_args[*]}"
            echo
          } >"$final_log"
          echo "capturing final bad commit output: ${bad_hash}" | tee -a "$final_log"
          run_once 2>&1 | tee -a "$final_log"
          final_status=${PIPESTATUS[0]}
          printf "%s\t%s\t%s\t%s\t%s\n" \
            "$((step + 1))" "$bad_hash" "$final_status" "final_bad_observation" "$final_log" >>"$summary_file"
          echo "first bad commit log: ${final_log}"
        fi
      fi
    fi
    exit 0
  fi
  if [ "$bisect_apply_status" -ne 0 ]; then
    echo "bisect halted. summary: ${summary_file}"
    if printf '%s' "$last_bisect_output" | grep -Fq "would be overwritten by checkout"; then
      echo "bisect halted because checkout failed due to dirty tracked files."
    fi
    exit "$bisect_apply_status"
  fi
  if [ "$next_commit_hash" = "$commit_hash" ]; then
    echo "bisect did not advance (still at ${commit_hash}); aborting to avoid an infinite loop."
    echo "last decision: ${last_bisect_decision}"
    echo "summary: ${summary_file}"
    exit 3
  fi
done

echo "git bisect is no longer active. summary: ${summary_file}"
