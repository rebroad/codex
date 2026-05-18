#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/render_prompt_captures.sh [options]
  scripts/render_prompt_captures.sh <capture-file-or-dir> [more paths...]

Options:
  --dir <path>           Capture directory (default: latest /var/tmp or /tmp codex-backend-capture.*)
  --out <path>           Write output files to directory instead of stdout
  --query-id <id>        Render only one query id
  --query_id <id>        Alias for --query-id
  --all                  Render all queries (default: compaction-only)
  --watch                Poll every 2s and render new matches continuously
  --help                 Show this help

Examples:
  scripts/render_prompt_captures.sh
  scripts/render_prompt_captures.sh --all
  scripts/render_prompt_captures.sh --query-id 127
  scripts/render_prompt_captures.sh 311_backend_traffic.ndjson
  scripts/render_prompt_captures.sh --watch
EOF
}

capture_dir=""
out_dir=""
query_id=""
only_compaction=1
watch_mode=0
declare -a explicit_targets=()

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
      only_compaction=0
      shift 2
      ;;
    --query_id)
      query_id="$2"
      only_compaction=0
      shift 2
      ;;
    --query-id=*)
      query_id="${1#*=}"
      only_compaction=0
      shift
      ;;
    --query_id=*)
      query_id="${1#*=}"
      only_compaction=0
      shift
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
      explicit_targets+=("$1")
      only_compaction=0
      shift
      ;;
  esac
done

infer_capture_dir_from_path() {
  local path="$1"
  if [[ -d "$path" ]]; then
    printf '%s\n' "$path"
    return 0
  fi

  if [[ -f "$path" ]]; then
    printf '%s\n' "$(dirname "$path")"
    return 0
  fi

  return 1
}

infer_query_id_from_path() {
  local path="$1"
  local file_name
  file_name="$(basename "$path")"
  if [[ "$file_name" =~ ^([0-9]+)_(backend_traffic|input|output)\.ndjson$ ]]; then
    printf '%s\n' "${BASH_REMATCH[1]}"
    return 0
  fi
  if [[ "$file_name" =~ ^([0-9]+)\.ndjson$ ]]; then
    printf '%s\n' "${BASH_REMATCH[1]}"
    return 0
  fi
  return 1
}

pick_capture_dir() {
  if [[ -n "$capture_dir" ]]; then
    printf '%s\n' "$capture_dir"
    return 0
  fi

  local candidates=()
  local pattern
  local match
  for pattern in \
    /var/tmp/codex-backend-capture.* \
    /tmp/codex-backend-capture.* \
    /var/tmp/codex-prompt-debug.* \
    /tmp/codex-prompt-debug.*
  do
    for match in $pattern; do
      [[ -e "$match" ]] || continue
      candidates+=("$match")
    done
  done

  [[ "${#candidates[@]}" -gt 0 ]] || return 0
  ls -1dt -- "${candidates[@]}" 2>/dev/null | head -n 1
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
  python3 -c '
import json
import sys

PLAIN = set("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._-/:@+")


def is_plain_string(value: str) -> bool:
    return bool(value) and all(ch in PLAIN for ch in value)


def render_inline_scalar(value):
    if value is None:
        return "null"
    if value is True:
        return "true"
    if value is False:
        return "false"
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, str):
        if "\n" in value:
            return None
        if is_plain_string(value):
            return value
        return json.dumps(value, ensure_ascii=False)
    raise TypeError(f"unsupported scalar type: {type(value)!r}")


def render_block_string(value: str, indent: int) -> list[str]:
    prefix = " " * indent
    chomp = "|+" if value.endswith("\n\n") else "|" if value.endswith("\n") else "|-"
    lines = value.split("\n")
    if value.endswith("\n"):
        lines = lines[:-1]
    rendered = [f"{prefix}{chomp}"]
    rendered.extend(f"{prefix}  {line}" for line in lines)
    return rendered


def render_key(value):
    rendered = render_inline_scalar(value)
    if rendered is None:
        raise TypeError("mapping keys cannot contain newlines")
    return rendered


def render_yaml(value, indent=0):
    prefix = " " * indent
    if isinstance(value, dict):
        if not value:
            return [f"{prefix}{{}}"]
        lines = []
        for key, item in value.items():
            rendered_key = render_key(key)
            if isinstance(item, dict) and not item:
                lines.append(f"{prefix}{rendered_key}: {{}}")
            elif isinstance(item, list) and not item:
                lines.append(f"{prefix}{rendered_key}: []")
            elif isinstance(item, (dict, list)):
                lines.append(f"{prefix}{rendered_key}:")
                lines.extend(render_yaml(item, indent + 2))
            else:
                rendered_item = render_inline_scalar(item)
                if rendered_item is not None:
                    lines.append(f"{prefix}{rendered_key}: {rendered_item}")
                else:
                    block_lines = render_block_string(item, indent + 2)
                    lines.append(f"{prefix}{rendered_key}: {block_lines[0].lstrip()}")
                    lines.extend(block_lines[1:])
        return lines
    if isinstance(value, list):
        if not value:
            return [f"{prefix}[]"]
        lines = []
        for item in value:
            if isinstance(item, dict) and not item:
                lines.append(f"{prefix}- {{}}")
            elif isinstance(item, list) and not item:
                lines.append(f"{prefix}- []")
            elif isinstance(item, (dict, list)):
                lines.append(f"{prefix}-")
                lines.extend(render_yaml(item, indent + 2))
            else:
                rendered_item = render_inline_scalar(item)
                if rendered_item is not None:
                    lines.append(f"{prefix}- {rendered_item}")
                else:
                    block_lines = render_block_string(item, indent + 2)
                    lines.append(f"{prefix}- {block_lines[0].lstrip()}")
                    lines.extend(block_lines[1:])
        return lines
    if isinstance(value, str):
        rendered_item = render_inline_scalar(value)
        if rendered_item is not None:
            return [f"{prefix}{rendered_item}"]
        return render_block_string(value, indent)
    return [f"{prefix}{render_inline_scalar(value)}"]


def main() -> int:
    raw = sys.stdin.read()
    try:
        value = json.loads(raw)
    except json.JSONDecodeError:
        sys.stdout.write(raw)
        return 0
    sys.stdout.write("\n".join(render_yaml(value)) + "\n")
    return 0


raise SystemExit(main())
'
}

render_one() {
  local dir="$1"
  local id="$2"
  local force_render="${3:-0}"
  local input_file="$dir/${id}_input.ndjson"
  local output_file="$dir/${id}_output.ndjson"
  local dest=""

  [[ -f "$input_file" ]] || return 0
  if [[ "$force_render" -eq 0 ]] && [[ "$only_compaction" -eq 1 ]] && ! is_compaction_capture "$input_file"; then
    return 1
  fi

  local request_payload=""
  request_payload="$(jq -c '.payload | fromjson? // .payload' "$input_file" 2>/dev/null | head -n 1 || true)"

  local response_payload=""
  local summary_encrypted=""
  local summary_item_count="0"
  local local_plain_summary=""
  if [[ -f "$output_file" ]]; then
    response_payload="$(jq -c 'select(.label? == "Compaction response") | .payload | fromjson? // .payload' "$output_file" 2>/dev/null | head -n 1 || true)"
    if [[ -z "$response_payload" ]]; then
      response_payload="$(jq -c '.payload | fromjson? // .payload' "$output_file" 2>/dev/null | head -n 1 || true)"
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

  local rendered_output
  rendered_output="$(
  {
    echo "# Prompt Capture $id"
    echo
    echo "- Capture dir: \`$dir\`"
    echo "- Input file: \`$input_file\`"
    [[ -f "$output_file" ]] && echo "- Output file: \`$output_file\`"
    echo "- Compaction summary items: \`${summary_item_count:-0}\`"
    echo "- Summary encrypted: \`${summary_encrypted:-unknown}\`"
    echo
    echo "## Request"
    echo
    echo '```yaml'
    printf '%s' "$request_payload" | pretty_or_raw_json
    echo '```'
    echo
    echo "## Response"
    echo
    echo '```yaml'
    printf '%s' "$response_payload" | pretty_or_raw_json
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
    }
  )"

  if [[ -n "$out_dir" ]]; then
    mkdir -p "$out_dir"
    dest="$out_dir/${id}.md"
    printf '%s\n' "$rendered_output" >"$dest"
    echo "Rendered: $dest" >&2
  else
    printf '%s\n' "$rendered_output"
  fi
  return 0
}

render_pass() {
  local dir="$1"
  local force_render="${2:-0}"
  local input
  shopt -s nullglob
  for input in "$dir"/*_input.ndjson; do
    local id
    id="$(basename "$input" _input.ndjson)"
    if [[ -n "$query_id" ]] && [[ "$id" != "$query_id" ]]; then
      continue
    fi
    render_one "$dir" "$id" "$force_render"
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
    local rendered=0
    if [[ "${#explicit_targets[@]}" -gt 0 ]]; then
      local target
      for target in "${explicit_targets[@]}"; do
        if [[ -d "$target" ]]; then
          local target_input
          local target_rendered=0
          shopt -s nullglob
          for target_input in "$target"/*_input.ndjson; do
            local target_id
            target_id="$(basename "$target_input" _input.ndjson)"
            if render_one "$target" "$target_id" 1; then
              target_rendered=1
              rendered=$((rendered + 1))
            fi
          done
          if [[ "$target_rendered" -eq 0 ]]; then
            echo "No captures rendered for dir: $target" >&2
          fi
          continue
        fi

        local target_dir=""
        local target_id=""
        target_dir="$(infer_capture_dir_from_path "$target" || true)"
        target_id="$(infer_query_id_from_path "$target" || true)"

        if [[ -z "$target_dir" ]] && [[ -n "$capture_dir" ]]; then
          target_dir="$capture_dir"
        fi
        if [[ -z "$target_dir" ]]; then
          target_dir="$dir"
        fi
        if [[ -z "$target_id" ]]; then
          echo "Could not infer query id from: $target" >&2
          continue
        fi

        if render_one "$target_dir" "$target_id" 1; then
          rendered=$((rendered + 1))
        fi
      done
    else
      local input
      shopt -s nullglob
      for input in "$dir"/*_input.ndjson; do
        local id
        id="$(basename "$input" _input.ndjson)"
        if [[ -n "$query_id" ]] && [[ "$id" != "$query_id" ]]; then
          continue
        fi
        if render_one "$dir" "$id" 0; then
          rendered=$((rendered + 1))
        fi
      done
    fi
    if [[ "$rendered" -eq 0 ]]; then
      echo "No captures rendered." >&2
      exit 1
    fi
    exit 0
  fi

  echo "Watching capture dir: $dir"
  while true; do
    render_pass "$dir" 0
    sleep 2
  done
}

main "$@"
