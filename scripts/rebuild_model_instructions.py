#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import pathlib
from collections.abc import Iterable
from typing import Any

import yaml


DEFAULT_OUTPUT = pathlib.Path(__file__).resolve().parents[1] / "codex-rs" / "models-manager" / "models.json"
DEFAULT_INPUT_DIR = pathlib.Path("/var/tmp/codex-model-instructions")


def parse_rebuild_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Rebuild models.json from extracted model instruction YAML files.",
    )
    parser.add_argument(
        "--input-dir",
        default=str(DEFAULT_INPUT_DIR),
        help="Directory containing per-model YAML files.",
    )
    parser.add_argument(
        "--output",
        default=str(DEFAULT_OUTPUT),
        help="Destination models.json file.",
    )
    return parser.parse_args()


def rebuild_model_document(document: dict[str, Any]) -> dict[str, Any]:
    slug = document.get("slug")
    if not isinstance(slug, str) or not slug:
        raise ValueError("document missing slug")
    if "base_instructions" not in document:
        raise ValueError(f"document {slug} missing base_instructions")
    return document


def iter_yaml_documents(input_dir: pathlib.Path) -> Iterable[pathlib.Path]:
    yield from sorted(input_dir.glob("*.yaml"))


def rebuild_models(input_dir: pathlib.Path, output_path: pathlib.Path) -> int:
    models: list[dict[str, Any]] = []
    for yaml_path in iter_yaml_documents(input_dir):
        document = yaml.safe_load(yaml_path.read_text(encoding="utf-8"))
        if not isinstance(document, dict):
            raise ValueError(f"{yaml_path} did not parse to a mapping")
        models.append(rebuild_model_document(document))

    output = {"models": models}
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(
        json.dumps(output, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    return len(models)


def main() -> int:
    args = parse_rebuild_args()
    input_dir = pathlib.Path(args.input_dir).expanduser()
    output_path = pathlib.Path(args.output).expanduser()
    count = rebuild_models(input_dir, output_path)
    print(f"Wrote {count} models to {output_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
