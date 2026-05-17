#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import pathlib
from typing import Any


REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_SOURCE = REPO_ROOT / "codex-rs" / "models-manager" / "models.json"
DEFAULT_OUTPUT_DIR = pathlib.Path("/var/tmp/codex-model-instructions")

_PLAIN_STRING = set("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._-/")


def parse_extract_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Extract model instructions into human-readable YAML files.",
    )
    parser.add_argument(
        "--input",
        default=str(DEFAULT_SOURCE),
        help="Source models.json file.",
    )
    parser.add_argument(
        "--output-dir",
        default=str(DEFAULT_OUTPUT_DIR),
        help="Directory to write per-model YAML files.",
    )
    return parser.parse_args()


def load_models_json(path: pathlib.Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def is_plain_string(value: str) -> bool:
    return bool(value) and all(character in _PLAIN_STRING for character in value)


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


def sanitized_slug(slug: str) -> str:
    return slug.replace("/", "__")


def extract_model_document(model: dict[str, Any], index: int) -> dict[str, Any]:
    if "base_instructions" not in model:
        raise ValueError(f"model {model.get('slug', index)} is missing base_instructions")
    return model


def dump_yaml(value: Any) -> str:
    return "\n".join(render_yaml(value)) + "\n"


def extract_models(input_path: pathlib.Path, output_dir: pathlib.Path) -> int:
    payload = load_models_json(input_path)
    models = payload.get("models")
    if not isinstance(models, list):
        raise ValueError(f"{input_path} does not contain a models array")

    output_dir.mkdir(parents=True, exist_ok=True)
    for index, model in enumerate(models):
        if not isinstance(model, dict):
            raise ValueError(f"model entry {index} is not an object")
        document = extract_model_document(model, index)
        slug = document["slug"]
        out_path = output_dir / f"{index:04d}-{sanitized_slug(slug)}.yaml"
        out_path.write_text(dump_yaml(document), encoding="utf-8")

    return len(models)


def main() -> int:
    args = parse_extract_args()
    input_path = pathlib.Path(args.input).expanduser()
    output_dir = pathlib.Path(args.output_dir).expanduser()
    count = extract_models(input_path, output_dir)
    print(f"Wrote {count} model instruction files to {output_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
