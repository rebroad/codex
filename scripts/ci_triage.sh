#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

slugify() {
  tr '[:upper:]' '[:lower:]' <<<"$1" | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//'
}

BRANCH="$(git -C "${REPO_DIR}" rev-parse --abbrev-ref HEAD)"
WORKFLOW=""
RUN_ID=""
LIMIT=20
OUTPUT_ROOT="${REPO_DIR}/tmp/ci-failures"
TRIAGE_STATE_FILE="${REPO_DIR}/tmp/ci-triaged-tags.json"
MARK_TAG=""
UNMARK_TAG=""
LIST_TRIAGED_TAGS="false"

for arg in "$@"; do
  case "${arg}" in
    --branch=*)
      BRANCH="${arg#*=}"
      ;;
    --workflow=*)
      WORKFLOW="${arg#*=}"
      ;;
    --run-id=*)
      RUN_ID="${arg#*=}"
      ;;
    --limit=*)
      LIMIT="${arg#*=}"
      ;;
    --output=*)
      OUTPUT_ROOT="${arg#*=}"
      ;;
    --triage-state-file=*)
      TRIAGE_STATE_FILE="${arg#*=}"
      ;;
    --mark-tag=*)
      MARK_TAG="${arg#*=}"
      ;;
    --unmark-tag=*)
      UNMARK_TAG="${arg#*=}"
      ;;
    --list-triaged-tags)
      LIST_TRIAGED_TAGS="true"
      ;;
    -h|--help)
      cat <<'EOF'
Usage: ci_triage.sh [--branch=<name>] [--workflow=<name>] [--run-id=<id>] [--limit=<n>] [--output=<dir>] [--mark-tag=<tag>]

Downloads failed-job logs from a GitHub Actions run and writes a summary file.

Defaults:
- branch: current git branch
- run: latest completed run on that branch
- output: tmp/ci-failures
- triage-state-file: tmp/ci-triaged-tags.json

Examples:
  scripts/ci_triage.sh
  scripts/ci_triage.sh --branch=main
  scripts/ci_triage.sh --workflow=rust-ci
  scripts/ci_triage.sh --run-id=23687639837
  scripts/ci_triage.sh --run-id=23687639837 --mark-tag=codex-v0.118.0-alpha.3
  scripts/ci_triage.sh --list-triaged-tags
  scripts/ci_triage.sh --unmark-tag=codex-v0.118.0-alpha.3
EOF
      exit 0
      ;;
    *)
      echo "Unknown argument: ${arg}" >&2
      echo "Run with --help for usage." >&2
      exit 1
      ;;
  esac
done

require_cmd gh
require_cmd jq

ensure_triage_state_file() {
  mkdir -p "$(dirname "${TRIAGE_STATE_FILE}")"
  if [[ ! -f "${TRIAGE_STATE_FILE}" ]]; then
    cat > "${TRIAGE_STATE_FILE}" <<'EOF'
{"version":1,"tags":{}}
EOF
  fi
}

mark_triaged_tag() {
  local tag="$1"
  local run_id="$2"
  local head_sha="$3"
  local summary_path="$4"
  local now tmp
  now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  tmp="$(mktemp)"
  jq \
    --arg tag "${tag}" \
    --arg now "${now}" \
    --arg run_id "${run_id}" \
    --arg head_sha "${head_sha}" \
    --arg summary_path "${summary_path}" \
    '
      .version = 1
      | .tags = (.tags // {})
      | .tags[$tag] = {
          markedAt: $now,
          runId: $run_id,
          headSha: $head_sha,
          summaryPath: $summary_path
        }
    ' "${TRIAGE_STATE_FILE}" > "${tmp}"
  mv "${tmp}" "${TRIAGE_STATE_FILE}"
}

unmark_triaged_tag() {
  local tag="$1"
  local tmp
  tmp="$(mktemp)"
  jq --arg tag "${tag}" '.tags = (.tags // {}) | del(.tags[$tag])' "${TRIAGE_STATE_FILE}" > "${tmp}"
  mv "${tmp}" "${TRIAGE_STATE_FILE}"
}

list_triaged_tags() {
  jq -r '
    .tags
    | to_entries
    | sort_by(.key)
    | .[]
    | "\(.key)\t\(.value.markedAt // "unknown")\t\(.value.runId // "unknown")\t\(.value.headSha // "unknown")"
  ' "${TRIAGE_STATE_FILE}"
}

if [[ "${LIST_TRIAGED_TAGS}" == "true" || -n "${UNMARK_TAG}" ]]; then
  ensure_triage_state_file
  if [[ "${LIST_TRIAGED_TAGS}" == "true" ]]; then
    list_triaged_tags
  fi
  if [[ -n "${UNMARK_TAG}" ]]; then
    unmark_triaged_tag "${UNMARK_TAG}"
    echo "Removed triaged tag: ${UNMARK_TAG}"
  fi
  if [[ "${LIST_TRIAGED_TAGS}" == "true" || -n "${UNMARK_TAG}" ]]; then
    exit 0
  fi
fi

mkdir -p "${OUTPUT_ROOT}"

if [[ -z "${RUN_ID}" ]]; then
  run_list_args=(
    run list
    --branch "${BRANCH}"
    --limit "${LIMIT}"
    --json databaseId,status,conclusion,workflowName,headBranch,headSha,event,createdAt,updatedAt,url,displayTitle
  )
  if [[ -n "${WORKFLOW}" ]]; then
    run_list_args+=(--workflow "${WORKFLOW}")
  fi

  run_list_json="$(gh "${run_list_args[@]}")"
  RUN_ID="$(
    jq -r '
      map(select(.status == "completed"))
      | .[0].databaseId // empty
    ' <<<"${run_list_json}"
  )"
fi

if [[ -z "${RUN_ID}" ]]; then
  echo "No completed run found for branch '${BRANCH}'." >&2
  exit 1
fi

run_json="$(gh run view "${RUN_ID}" --json databaseId,name,displayTitle,status,conclusion,event,headBranch,headSha,createdAt,updatedAt,url,jobs)"
head_sha="$(jq -r '.headSha // "unknown"' <<<"${run_json}")"
sha_short="${head_sha:0:12}"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
run_dir="${OUTPUT_ROOT}/${timestamp}-run${RUN_ID}-${sha_short}"
logs_dir="${run_dir}/logs"
summary_file="${run_dir}/summary.md"
mkdir -p "${logs_dir}"

jq -r '
  .jobs[]
  | select(.conclusion == "failure")
  | @base64
' <<<"${run_json}" | while IFS= read -r row; do
  [[ -z "${row}" ]] && continue
  job_json="$(base64 -d <<<"${row}")"
  job_id="$(jq -r '.databaseId' <<<"${job_json}")"
  job_name="$(jq -r '.name' <<<"${job_json}")"
  job_slug="$(slugify "${job_name}")"
  log_file="${logs_dir}/${job_id}-${job_slug}.log"

  if gh run view "${RUN_ID}" --job "${job_id}" --log-failed >"${log_file}" 2>"${log_file}.stderr"; then
    rm -f "${log_file}.stderr"
  else
    {
      echo "Failed to fetch logs for job ${job_id} (${job_name})."
      echo
      cat "${log_file}.stderr"
    } > "${log_file}"
    rm -f "${log_file}.stderr"
  fi
done

{
  echo "# CI Triage Summary"
  echo
  echo "- run_id: ${RUN_ID}"
  echo "- title: $(jq -r '.displayTitle // .name' <<<"${run_json}")"
  echo "- workflow: $(jq -r '.name' <<<"${run_json}")"
  echo "- branch: $(jq -r '.headBranch' <<<"${run_json}")"
  echo "- head_sha: ${head_sha}"
  echo "- status: $(jq -r '.status' <<<"${run_json}")"
  echo "- conclusion: $(jq -r '.conclusion // "unknown"' <<<"${run_json}")"
  echo "- event: $(jq -r '.event' <<<"${run_json}")"
  echo "- created_at: $(jq -r '.createdAt' <<<"${run_json}")"
  echo "- updated_at: $(jq -r '.updatedAt' <<<"${run_json}")"
  echo "- url: $(jq -r '.url' <<<"${run_json}")"
  echo
  echo "## Failed Jobs"
  failed_count="$(jq '[.jobs[] | select(.conclusion == "failure")] | length' <<<"${run_json}")"
  if [[ "${failed_count}" == "0" ]]; then
    echo
    echo "No failed jobs in this run."
  else
    echo
    jq -r '
      .jobs[]
      | select(.conclusion == "failure")
      | "- id: \(.databaseId) | name: \(.name) | url: \(.url)"
    ' <<<"${run_json}"
  fi
  echo
  echo "## All Jobs"
  echo
  jq -r '
    .jobs[]
    | "- [\(.conclusion // "unknown")] \(.name) (id=\(.databaseId))"
  ' <<<"${run_json}"
} > "${summary_file}"

echo "Wrote CI summary: ${summary_file}"
echo "Wrote failed logs dir: ${logs_dir}"

if [[ -n "${MARK_TAG}" ]]; then
  ensure_triage_state_file
  mark_triaged_tag "${MARK_TAG}" "${RUN_ID}" "${head_sha}" "${summary_file}"
  failed_count="$(jq '[.jobs[] | select(.conclusion == "failure")] | length' <<<"${run_json}")"
  if [[ "${failed_count}" != "0" ]]; then
    echo "Marked triaged tag despite failed jobs: ${MARK_TAG}"
  else
    echo "Marked triaged tag: ${MARK_TAG}"
  fi
  echo "Triage registry: ${TRIAGE_STATE_FILE}"
fi
