#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import pathlib
import sys
from collections.abc import Iterable
from typing import Any

SUPPORTED_SUFFIXES = {".jsonl", ".ndjson"}
_JSONISH_STRING_KEYS = {"payload", "input", "output", "body"}
_PLAIN_STRING = set("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._-/:@+")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Render Codex rollout or prompt-debug JSONL/NDJSON files as human-readable text.",
    )
    parser.add_argument(
        "paths",
        nargs="+",
        help="Input file(s) or directory(ies) containing .jsonl/.ndjson files.",
    )
    parser.add_argument(
        "--output-dir",
        help="Write one .txt file per input instead of printing to stdout.",
    )
    return parser.parse_args()


def is_plain_string(value: str) -> bool:
    return bool(value) and all(character in _PLAIN_STRING for character in value)


def maybe_parse_jsonish_string(value: str, key: str | None) -> Any:
    if key not in _JSONISH_STRING_KEYS:
        return value

    stripped = value.lstrip()
    if not stripped or stripped[0] not in "{[":
        return value

    try:
        parsed = json.loads(value)
    except json.JSONDecodeError:
        return value

    return parsed if isinstance(parsed, (dict, list)) else value


def normalize_value(value: Any, key: str | None = None) -> Any:
    if isinstance(value, str):
        return maybe_parse_jsonish_string(value, key)
    if isinstance(value, dict):
        return {child_key: normalize_value(child_value, child_key) for child_key, child_value in value.items()}
    if isinstance(value, list):
        return [normalize_value(item) for item in value]
    return value


def render_inline_scalar(value: Any) -> str | None:
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


def render_key(value: Any) -> str:
    rendered = render_inline_scalar(value)
    if rendered is None:
        raise TypeError("mapping keys cannot contain newlines")
    return rendered


def render_yaml(value: Any, indent: int = 0) -> list[str]:
    prefix = " " * indent
    if isinstance(value, dict):
        if not value:
            return [f"{prefix}{{}}"]
        lines: list[str] = []
        for key, item in value.items():
            rendered_key = render_key(key)
            if isinstance(item, dict) and not item:
                lines.append(f"{prefix}{rendered_key}: {{}}")
            elif isinstance(item, list) and not item:
                lines.append(f"{prefix}{rendered_key}: []")
            elif isinstance(item, (dict, list)):
                child_lines = render_yaml(item, indent + 2)
                lines.append(f"{prefix}{rendered_key}:")
                lines.extend(child_lines)
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
        lines: list[str] = []
        for item in value:
            if isinstance(item, dict) and not item:
                lines.append(f"{prefix}- {{}}")
            elif isinstance(item, list) and not item:
                lines.append(f"{prefix}- []")
            elif isinstance(item, (dict, list)):
                child_lines = render_yaml(item, indent + 2)
                lines.append(f"{prefix}-")
                lines.extend(child_lines)
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


def discover_inputs(paths: Iterable[str]) -> list[pathlib.Path]:
    discovered: list[pathlib.Path] = []
    for raw_path in paths:
        path = pathlib.Path(raw_path).expanduser()
        if path.is_dir():
            for child in sorted(path.iterdir()):
                if child.is_file() and child.suffix in SUPPORTED_SUFFIXES:
                    discovered.append(child)
        elif path.is_file():
            discovered.append(path)
        else:
            raise FileNotFoundError(path)
    return discovered


def render_file(path: pathlib.Path) -> str:
    lines: list[str] = [f"# File: {path}", ""]
    with path.open("r", encoding="utf-8") as handle:
        for line_number, raw_line in enumerate(handle, start=1):
            stripped = raw_line.rstrip("\n")
            if not stripped:
                continue

            try:
                parsed = json.loads(stripped)
            except json.JSONDecodeError:
                lines.append(f"--- line {line_number} ---")
                lines.append(stripped)
                lines.append("")
                continue

            normalized = normalize_value(parsed)
            lines.append(f"--- line {line_number} ---")
            lines.extend(render_yaml(normalized))
            lines.append("")

    if len(lines) == 2:
        lines.append("(empty file)")
    return "\n".join(lines).rstrip() + "\n"


def output_path_for(input_path: pathlib.Path, output_dir: pathlib.Path) -> pathlib.Path:
    return output_dir / f"{input_path.with_suffix('.txt').name}"


def main() -> int:
    args = parse_args()
    inputs = discover_inputs(args.paths)
    if not inputs:
        print("No .jsonl or .ndjson files found.", file=sys.stderr)
        return 1

    if args.output_dir:
        output_dir = pathlib.Path(args.output_dir).expanduser()
        output_dir.mkdir(parents=True, exist_ok=True)
        for input_path in inputs:
            rendered = render_file(input_path)
            out_path = output_path_for(input_path, output_dir)
            out_path.write_text(rendered, encoding="utf-8")
            print(f"Wrote {out_path}")
        return 0

    for index, input_path in enumerate(inputs):
        if index:
            sys.stdout.write("\n")
        sys.stdout.write(render_file(input_path))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
