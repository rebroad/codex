#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/render_prompt_captures.sh [options]

Options:
  --dir <path>           Capture directory (default: latest /var/tmp or /tmp codex-backend-capture.*)
  --out <path>           Output directory (default: tmp/prompt-captures-md)
  --query-id <id>        Render only one query id
  --all                  Render all queries (default: compaction-only)
  --watch                Poll every 2s and render new matches continuously
  --help                 Show this help

Examples:
  scripts/render_prompt_captures.sh
  scripts/render_prompt_captures.sh --all
  scripts/render_prompt_captures.sh --query-id 127
  scripts/render_prompt_captures.sh --watch
EOF
}

capture_dir=""
out_dir="/home/rebroad/src/codex/tmp/prompt-captures-md"
query_id=""
only_compaction=1
watch_mode=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dir)
      capture_dir="$2"
      shift 2
      ;;
    --out)
      out_dir="$2"
      shift 2
      ;;
    --query-id)
      query_id="$2"
      shift 2
      ;;
    --all)
      only_compaction=0
      shift
      ;;
    --watch)
      watch_mode=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

pick_capture_dir() {
  if [[ -n "$capture_dir" ]]; then
    printf '%s\n' "$capture_dir"
    return 0
  fi
  ls -1dt \
    /var/tmp/codex-backend-capture.* \
    /tmp/codex-backend-capture.* \
    /var/tmp/codex-prompt-debug.* \
    /tmp/codex-prompt-debug.* 2>/dev/null | head -n 1
}

is_compaction_capture() {
  local input_file="$1"
  local kind=""
  kind="$(jq -r 'select(.kind? != null) | .kind' "$input_file" 2>/dev/null | head -n 1 || true)"
  if [[ "$kind" == "responses_compact" ]]; then
    return 0
  fi

  # Local compaction runs through normal responses requests; detect the
  # synthesized compaction prompt as the outgoing user message.
  jq -e '
    .payload
    | strings
    | fromjson?
    | .input? // []
    | any(
        .[]?;
        .type == "message"
        and .role == "user"
        and any(
          .content[]?;
          .type == "input_text"
          and (.text | startswith("You are performing a CONTEXT CHECKPOINT COMPACTION."))
        )
      )
  ' "$input_file" >/dev/null 2>&1
}

pretty_or_raw_json() {
  local raw="$1"
  if jq -e . >/dev/null 2>&1 <<<"$raw"; then
    jq . <<<"$raw"
  else
    printf '%s\n' "$raw"
  fi
}

render_one() {
  local dir="$1"
  local id="$2"
  local input_file="$dir/${id}_input.ndjson"
  local output_file="$dir/${id}_output.ndjson"
  local reasoning_file="$dir/${id}_reasoning.ndjson"
  local dest="$out_dir/${id}.md"

  [[ -f "$input_file" ]] || return 0
  if [[ "$only_compaction" -eq 1 ]] && ! is_compaction_capture "$input_file"; then
    return 0
  fi

  mkdir -p "$out_dir"

  local request_payload=""
  request_payload="$(jq -r '.payload // empty' "$input_file" 2>/dev/null | paste -sd '\n' - || true)"

  local response_payload=""
  local summary_encrypted=""
  local summary_item_count="0"
  local local_plain_summary=""
  if [[ -f "$output_file" ]]; then
    response_payload="$(jq -r 'select(.label? == "Compaction response") | .payload' "$output_file" 2>/dev/null | paste -sd '\n' - || true)"
    if [[ -z "$response_payload" ]]; then
      response_payload="$(jq -r '.payload // empty' "$output_file" 2>/dev/null | paste -sd '\n' - || true)"
    fi
    summary_item_count="$(jq -r '[select(.label? == "Compaction response") | .payload.output[]? | select(.type=="compaction_summary")] | length' "$output_file" 2>/dev/null | tail -n 1 || true)"
    summary_encrypted="$(jq -r 'select(.label? == "Compaction response") | .payload.output[]? | select(.type=="compaction_summary") | has("encrypted_content")' "$output_file" 2>/dev/null | head -n 1 || true)"
  fi

  if [[ -f "$output_file" ]]; then
    local_plain_summary="$(
      jq -r '
        .payload
        | strings
        | fromjson?
        | .response.output[]?
        | select(.type == "message")
        | .content[]?
        | select(.type == "output_text")
        | .text
      ' "$output_file" 2>/dev/null | paste -sd '\n' - || true
    )"
  fi

  {
    echo "# Prompt Capture $id"
    echo
    echo "- Capture dir: \`$dir\`"
    echo "- Input file: \`$input_file\`"
    [[ -f "$output_file" ]] && echo "- Output file: \`$output_file\`"
    [[ -f "$reasoning_file" ]] && echo "- Reasoning file: \`$reasoning_file\`"
    echo "- Compaction summary items: \`${summary_item_count:-0}\`"
    echo "- Summary encrypted: \`${summary_encrypted:-unknown}\`"
    echo
    echo "## Request"
    echo
    echo '```json'
    pretty_or_raw_json "$request_payload"
    echo '```'
    echo
    echo "## Response"
    echo
    echo '```json'
    pretty_or_raw_json "$response_payload"
    echo '```'
    if [[ -n "${local_plain_summary:-}" ]]; then
      echo
      echo "## Extracted Local Summary Text"
      echo
      echo '```text'
      printf '%s\n' "$local_plain_summary"
      echo '```'
    fi
    if [[ "${summary_encrypted:-}" == "true" ]]; then
      echo
      echo '> Note: backend compaction summary content is encrypted in captures (`compaction_summary.encrypted_content`), so plain summary text is not available here.'
    fi
  } >"$dest"

  echo "Rendered: $dest"
}

render_pass() {
  local dir="$1"
  local input
  shopt -s nullglob
  for input in "$dir"/*_input.ndjson; do
    local id
    id="$(basename "$input" _input.ndjson)"
    if [[ -n "$query_id" ]] && [[ "$id" != "$query_id" ]]; then
      continue
    fi
    render_one "$dir" "$id"
  done
}

main() {
  local dir
  dir="$(pick_capture_dir)"
  if [[ -z "$dir" ]] || [[ ! -d "$dir" ]]; then
    echo "No capture directory found." >&2
    exit 1
  fi

  if [[ "$watch_mode" -eq 0 ]]; then
    render_pass "$dir"
    exit 0
  fi

  echo "Watching capture dir: $dir"
  while true; do
    render_pass "$dir"
    sleep 2
  done
}

main "$@"
